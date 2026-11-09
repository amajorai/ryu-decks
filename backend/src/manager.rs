//! The device loop: one OS thread owns the HID context and every open deck.
//!
//! hidapi calls block, and a `HidApi` must not be shared across threads, so all
//! device I/O lives on this thread. It rescans for decks every couple of seconds
//! (hot-plug without OS-specific notifications), paints a deck from its stored
//! layout when it appears, polls each open deck for input, and forwards presses to
//! the async dispatcher. The HTTP side talks to it through [`Manager`]: a command
//! channel in, a status snapshot out. On a Mac with a Touch Bar, the Touch Bar is
//! one more deck, always present, reached over a link to the main thread. QMK
//! boards on Vial firmware have no USB ids of their own: each is found by Vial's
//! serial-number marker on its raw-HID collection and asked for its layout once,
//! which becomes its model; HID LampArray devices likewise, by their Lighting
//! and Illumination collection and the lamps they report.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use crate::catalog::{self, DeviceModel};
use crate::driver::touchbar::{self, TouchBar};
use crate::driver::{self, lamparray, prehkeytec, vial, Driver, Face, Input, LockState};
use crate::store::{Binding, Store};

const SCAN_INTERVAL: Duration = Duration::from_secs(2);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
/// How long one poll waits for input on one deck.
const POLL_TIMEOUT_MS: i32 = 5;
const IDLE_SLEEP: Duration = Duration::from_millis(100);
/// How long a deck that refused to open waits before it is tried again: long
/// enough not to hammer a busy device, short enough that quitting the vendor app
/// or installing the udev rule is picked up without a replug.
const RETRY_REFUSED: Duration = Duration::from_secs(15);
/// How long a serial port that did not answer its probe waits before it is
/// asked again, on a protocol that asks again. Probes run off the device
/// thread (a DJI probe takes 1.5 s), so a silent port never stalls the decks.
const SERIAL_REPROBE: Duration = Duration::from_secs(30);

/// One input from one deck, on its way to the dispatcher.
#[derive(Debug, Clone)]
pub struct Press {
    pub device_id: String,
    pub model: &'static DeviceModel,
    pub input: Input,
}

/// A deck the loop has seen this session.
#[derive(Debug, Clone)]
pub struct DeviceStatus {
    pub model: &'static DeviceModel,
    pub serial: Option<String>,
    pub connected: bool,
    /// Why the deck is present but not usable (typically permissions).
    pub error: Option<String>,
    /// Whether its keys are unlocked, on a deck that locks them (a Vial board).
    pub lock: Option<LockState>,
}

#[derive(Default)]
pub struct Snapshot {
    pub devices: BTreeMap<String, DeviceStatus>,
    /// Set when the HID subsystem itself could not start.
    pub hid_error: Option<String>,
    pub running: bool,
}

enum Command {
    Repaint(String),
    Brightness(String, u8),
    Unlock(String),
}

/// The handle the HTTP side holds.
#[derive(Clone)]
pub struct Manager {
    commands: Option<Sender<Command>>,
    snapshot: Arc<RwLock<Snapshot>>,
}

impl Manager {
    /// A manager with no device loop, for tests and headless runs
    /// (`RYU_DECKS_HID=off`). Layouts still save; nothing is painted.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            commands: None,
            snapshot: Arc::new(RwLock::new(Snapshot::default())),
        }
    }

    /// Start the device loop on its own thread. `touch_bar` is the link to the
    /// main thread on a Mac that has a Touch Bar.
    pub fn spawn(
        store: Arc<Store>,
        presses: UnboundedSender<Press>,
        touch_bar: Option<touchbar::Link>,
    ) -> Self {
        let (commands, inbox) = mpsc::channel();
        let snapshot = Arc::new(RwLock::new(Snapshot {
            running: true,
            ..Snapshot::default()
        }));
        let shared = Arc::clone(&snapshot);
        let spawned = std::thread::Builder::new()
            .name("ryu-decks-hid".to_owned())
            .spawn(move || DeviceLoop::new(store, presses, shared, touch_bar).run(&inbox));
        if let Err(error) = spawned {
            tracing::error!(%error, "ryu-decks: could not start the device thread");
            if let Ok(mut snapshot) = snapshot.write() {
                snapshot.running = false;
                snapshot.hid_error = Some(format!("Could not start the device thread: {error}"));
            }
        }
        Self {
            commands: Some(commands),
            snapshot,
        }
    }

    #[cfg(test)]
    pub fn with_snapshot(snapshot: Snapshot) -> Self {
        Self {
            commands: None,
            snapshot: Arc::new(RwLock::new(snapshot)),
        }
    }

    /// Re-read a deck's layout and paint it, if it is connected.
    pub fn repaint(&self, device_id: &str) {
        self.send(Command::Repaint(device_id.to_owned()));
    }

    pub fn set_brightness(&self, device_id: &str, percent: u8) {
        self.send(Command::Brightness(device_id.to_owned(), percent));
    }

    /// Start unlocking a deck's keys; the user then holds its unlock keys.
    pub fn unlock(&self, device_id: &str) {
        self.send(Command::Unlock(device_id.to_owned()));
    }

    fn send(&self, command: Command) {
        if let Some(commands) = &self.commands {
            // The loop only exits with the process; a send error means it died.
            if commands.send(command).is_err() {
                tracing::warn!("ryu-decks: the device thread is gone; command dropped");
            }
        }
    }

    pub fn read<T>(&self, read: impl FnOnce(&Snapshot) -> T) -> T {
        let guard = self
            .snapshot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        read(&guard)
    }
}

struct OpenDeck {
    model: &'static DeviceModel,
    driver: Box<dyn Driver>,
    /// The lock as last published in the snapshot.
    lock: Option<LockState>,
    /// The MIDI ports a MIDI deck was opened through, exactly as listed.
    midi_ports: Option<driver::midi::PortPair>,
}

/// One HID collection worth opening, found by a scan.
#[derive(Debug, Clone)]
struct Candidate {
    model: &'static DeviceModel,
    serial: Option<String>,
    path: CString,
    interface: i32,
    usage_page: u16,
    /// USB rather than Bluetooth. A keypad reachable both ways is driven over
    /// the cable, which does not drop when the keypad sleeps.
    wired: bool,
    /// A USB serial port (Loupedeck, TourBox) rather than a HID collection; `path` is
    /// then the port name.
    serial_port: bool,
    /// A Linux input event node (the Griffin PowerMate) rather than a HID
    /// collection; `path` is then `/dev/input/eventN`.
    evdev: bool,
    /// A pair of USB MIDI ports (a Novation Launchpad) rather than a HID
    /// collection; `path` is then [`driver::midi::Found::path`].
    midi: Option<driver::midi::PortPair>,
    /// The second interface a deck draws its faces through (the Mountain
    /// DisplayPad), or the lighting collection of a deck paired by usage page
    /// (a Wooting keyboard); `path` is then its control interface.
    display_path: Option<CString>,
    /// The usage page of `display_path`, on a deck paired by usage page.
    display_usage_page: u16,
    /// Other paths of this deck's collection with the same usage page (the
    /// Mystrix's two vendor collections on Windows), tried in turn with the
    /// protocol's [`catalog::Protocol::open_probe`] when `path` refuses it, or
    /// (the Adafruit MacroPad's three on Windows) told apart by descriptor.
    alternates: Vec<CString>,
    /// Why a Vial board cannot be a deck, found when its layout was read; it
    /// is shown instead of opening it.
    refusal: Option<String>,
}

impl Candidate {
    /// Lower is better: wired before wireless, then the first interface.
    fn rank(&self) -> (bool, i32) {
        (!self.wired, self.interface)
    }
}

/// What a serial port said to its probe.
#[derive(Clone, Copy, Debug)]
enum PortAnswer {
    /// Nothing; a protocol that asks again does so after [`SERIAL_REPROBE`].
    Silent,
    /// It answered, but not as a deck Ryu drives (a 16 x 16 monome grid), so
    /// it is never asked again while it stays plugged in.
    NotDriven,
    /// The model it answered as.
    Deck(&'static DeviceModel),
}

/// What probing one serial port found, and when.
struct SerialProbe {
    answer: PortAnswer,
    /// The port's USB serial number, which a device's sibling ports share.
    serial: Option<String>,
    at: Instant,
}

/// Serial ports by name: what their last probe found, and the probes still
/// running on their own threads.
#[derive(Default)]
struct SerialProbes {
    done: BTreeMap<String, SerialProbe>,
    running: BTreeMap<String, RunningProbe>,
}

impl SerialProbe {
    /// Whether the port answered as a deck.
    const fn answered(&self) -> bool {
        matches!(self.answer, PortAnswer::Deck(_))
    }

    /// Whether the port said nothing to its probe.
    const fn silent(&self) -> bool {
        matches!(self.answer, PortAnswer::Silent)
    }
}

/// What a probe thread hands back: `None` when the port would not open, else
/// what the port answered.
type ProbeAnswer = Option<PortAnswer>;

/// A probe on its own thread.
struct RunningProbe {
    serial: Option<String>,
    handle: std::thread::JoinHandle<ProbeAnswer>,
}

impl RunningProbe {
    /// The probe's answer once its thread has finished, or the probe back
    /// while it is still asking. A thread that panicked counts as a port that
    /// would not open, so it is asked again on the next scan.
    fn finish(self) -> Result<ProbeAnswer, Self> {
        if self.handle.is_finished() {
            Ok(self.handle.join().ok().flatten())
        } else {
            Err(self)
        }
    }
}

/// Whether a port is probed (again): always the first time; after that only
/// a silent port (not one that answered as something Ryu does not drive) of
/// a protocol that asks again, once [`SERIAL_REPROBE`] has passed, and only while no sibling port under the same USB serial answered.
fn should_probe(
    cached: Option<&SerialProbe>,
    reprobes: bool,
    sibling_answered: bool,
    now: Instant,
) -> bool {
    cached.is_none_or(|probe| {
        probe.silent() && reprobes && !sibling_answered && probe.at + SERIAL_REPROBE <= now
    })
}

/// Decks on USB serial ports (Loupedeck, TourBox, the Yamaha CC121MK2, DJI
/// remotes, monome grids and arcs).
///
/// A monome has no USB id of its own: it is found by its USB serial number or
/// strings ([`driver::monome::identify`]), and a mext grid or arc is claimed as
/// the model its probe answered with.
///
/// A port on a generic USB id that other boards share, or on a device that
/// lists a silent second port (a DJI remote), is claimed only once it answers
/// the protocol's probe. `probes` remembers each port's answer, so a board that
/// is not a deck is asked once, until it is unplugged. A DJI remote's silent
/// port is asked again every [`SERIAL_REPROBE`] until its sibling answers,
/// since the remote may not have been ready. Each probe runs on its own thread
/// and a port is claimed on the first scan after it answered, so a port that
/// keeps not answering never holds up the decks already open.
fn serial_candidates(probes: &mut SerialProbes) -> Vec<Candidate> {
    let ports = match serialport::available_ports() {
        Ok(ports) => ports,
        Err(error) => {
            tracing::debug!(%error, "ryu-decks: could not list serial ports");
            return Vec::new();
        }
    };
    probes
        .done
        .retain(|name, _| ports.iter().any(|port| &port.port_name == name));
    probes
        .running
        .retain(|name, _| ports.iter().any(|port| &port.port_name == name));
    ports
        .into_iter()
        .filter_map(|port| {
            let serialport::SerialPortType::UsbPort(usb) = port.port_type else {
                return None;
            };
            let mut model = driver::monome::identify(
                usb.serial_number.as_deref(),
                usb.manufacturer.as_deref(),
                usb.product.as_deref(),
            )
            .or_else(|| catalog::identify_serial(usb.vid, usb.pid))?;
            if model.protocol.needs_serial_probe(usb.vid, usb.pid) {
                if let Some(running) = probes.running.remove(&port.port_name) {
                    let serial = running.serial.clone();
                    match running.finish() {
                        Ok(Some(answer)) => {
                            if !matches!(answer, PortAnswer::Deck(_)) {
                                tracing::debug!(port = %port.port_name, model = model.name, "ryu-decks: serial port did not answer as a deck; leaving it alone");
                            }
                            probes.done.insert(
                                port.port_name.clone(),
                                SerialProbe {
                                    answer,
                                    serial,
                                    at: Instant::now(),
                                },
                            );
                        }
                        // The port would not open: asked again next scan.
                        Ok(None) => {}
                        Err(running) => {
                            probes.running.insert(port.port_name.clone(), running);
                            return None;
                        }
                    }
                }
                let sibling_answered = usb.serial_number.is_some()
                    && probes.done.iter().any(|(name, probe)| {
                        name != &port.port_name && probe.answered() && probe.serial == usb.serial_number
                    });
                let cached = probes.done.get(&port.port_name);
                if should_probe(
                    cached,
                    model.protocol.reprobes_silent_ports(),
                    sibling_answered,
                    Instant::now(),
                ) {
                    let name = port.port_name.clone();
                    let handle = std::thread::Builder::new()
                        .name("ryu-decks-probe".to_owned())
                        .spawn(move || probe_serial(model, &name))
                        .ok()?;
                    probes.running.insert(
                        port.port_name.clone(),
                        RunningProbe {
                            serial: usb.serial_number.clone(),
                            handle,
                        },
                    );
                    return None;
                }
                match cached.map(|probe| probe.answer) {
                    Some(PortAnswer::Deck(found)) => model = found,
                    _ => return None,
                }
            }
            let path = std::ffi::CString::new(port.port_name.clone()).ok()?;
            // A serial the host made up changes with the USB socket; without
            // it the id is the model's, and a second deck is told apart by path.
            let serial = usb
                .serial_number
                .filter(|_| !model.protocol.serial_is_invented());
            Some(Candidate {
                model,
                serial,
                path,
                // macOS lists each port twice; the call-out device (`cu.`) opens
                // without waiting for a carrier, so it ranks first.
                interface: i32::from(port.port_name.contains("/tty.")),
                usage_page: 0,
                wired: true,
                serial_port: true,
                evdev: false,
                midi: None,
                display_path: None,
                display_usage_page: 0,
                alternates: Vec::new(),
                refusal: None,
            })
        })
        .collect()
}

/// What reading a Vial board's layout, or a LampArray's lamps, came to, and when.
struct LayoutProbe {
    /// The device's model, or why it cannot be a deck (shown as is).
    result: Result<&'static DeviceModel, String>,
    at: Instant,
}

/// What a read on its own thread hands back.
type LayoutRead = std::thread::JoinHandle<Result<&'static DeviceModel, String>>;

/// Devices read from the device, by HID path: what their last read found, and
/// the reads still running on their own threads.
#[derive(Default)]
struct LayoutProbes {
    done: BTreeMap<CString, LayoutProbe>,
    running: BTreeMap<CString, LayoutRead>,
}

/// The USB facts a read names its model with, carried to its thread.
struct ReadFacts {
    vendor_id: u16,
    product_id: u16,
    product: Option<String>,
}

/// Devices whose model is read from the device rather than looked up by USB id.
struct ReadModels<'a> {
    /// Whether an enumerated HID collection is one of them.
    matches: &'a dyn Fn(&hidapi::DeviceInfo) -> bool,
    /// Reads the model from one that was opened, on its own thread.
    read: fn(hidapi::HidDevice, &ReadFacts) -> Result<&'static DeviceModel, String>,
    /// Stands in for one that could not be read.
    unread: &'static DeviceModel,
    /// Whether the USB serial tells two of them apart (every Vial board
    /// reports the same one).
    keeps_serial: bool,
    what: &'static str,
}

/// Whether a device is read (again): never while a read is running; always
/// the first time; after that only once a failed read is [`RETRY_REFUSED`] old.
fn should_read(done: Option<&LayoutProbe>, running: bool, now: Instant) -> bool {
    !running && done.is_none_or(|probe| probe.result.is_err() && probe.at + RETRY_REFUSED <= now)
}

/// Devices with no USB ids of their own, claimed by their HID collection once
/// every listed deck has had first claim. Each is read once while it stays
/// plugged in (one that could not be read is read again after
/// [`RETRY_REFUSED`]), and that becomes its model. A read can take many
/// control transfers (one per lamp), so it runs on its own thread and the
/// device is claimed on the first scan after it finished, instead of holding
/// up the decks already open.
fn read_model_candidates(
    api: &hidapi::HidApi,
    probes: &mut LayoutProbes,
    kind: &ReadModels<'_>,
) -> Vec<Candidate> {
    let devices: Vec<&hidapi::DeviceInfo> = api
        .device_list()
        .filter(|info| {
            catalog::identify(info.vendor_id(), info.product_id(), info.usage_page()).is_none()
                && (kind.matches)(info)
        })
        .collect();
    let present = |path: &CString| devices.iter().any(|info| info.path() == path.as_c_str());
    probes.done.retain(|path, _| present(path));
    probes.running.retain(|path, _| present(path));
    let mut found = Vec::new();
    for info in devices {
        let path = info.path().to_owned();
        if probes
            .running
            .get(&path)
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            if let Some(read) = probes.running.remove(&path) {
                let result = read
                    .join()
                    .unwrap_or_else(|_| Err(format!("Ryu could not read this {}.", kind.what)));
                record_read(probes, path.clone(), result, kind.what);
            }
        }
        let now = Instant::now();
        if should_read(
            probes.done.get(&path),
            probes.running.contains_key(&path),
            now,
        ) {
            match api.open_path(&path) {
                Ok(device) => {
                    let facts = ReadFacts {
                        vendor_id: info.vendor_id(),
                        product_id: info.product_id(),
                        product: info.product_string().map(str::to_owned),
                    };
                    let read = kind.read;
                    match std::thread::Builder::new()
                        .name("ryu-decks-read".to_owned())
                        .spawn(move || read(device, &facts))
                    {
                        Ok(handle) => {
                            probes.running.insert(path.clone(), handle);
                        }
                        Err(error) => record_read(
                            probes,
                            path.clone(),
                            Err(open_failure(&error.to_string())),
                            kind.what,
                        ),
                    }
                }
                Err(error) => record_read(
                    probes,
                    path.clone(),
                    Err(open_failure(&error.to_string())),
                    kind.what,
                ),
            }
        }
        // Until its first read finishes a device is not claimed; one being
        // read again keeps what its last read found.
        let Some(probe) = probes.done.get(&path) else {
            continue;
        };
        let (model, refusal) = match &probe.result {
            Ok(model) => (*model, None),
            Err(reason) => (kind.unread, Some(reason.clone())),
        };
        let serial = if kind.keeps_serial {
            catalog::usable_serial(model, info.serial_number()).map(str::to_owned)
        } else {
            None
        };
        found.push(Candidate {
            model,
            serial,
            path,
            interface: info.interface_number(),
            usage_page: info.usage_page(),
            wired: !matches!(info.bus_type(), hidapi::BusType::Bluetooth),
            serial_port: false,
            evdev: false,
            midi: None,
            display_path: None,
            display_usage_page: 0,
            alternates: Vec::new(),
            refusal,
        });
    }
    found
}

/// Remembers what reading a device came to.
fn record_read(
    probes: &mut LayoutProbes,
    path: CString,
    result: Result<&'static DeviceModel, String>,
    what: &str,
) {
    if let Err(reason) = &result {
        tracing::debug!(path = ?path, %reason, what, "ryu-decks: device not usable as a deck");
    }
    probes.done.insert(
        path,
        LayoutProbe {
            result,
            at: Instant::now(),
        },
    );
}

/// QMK boards on Vial firmware, on any USB id: Vial's serial-number marker on
/// QMK's raw-HID collection. Every Vial board reports the same serial, so none
/// is used; identical boards are told apart by path.
fn vial_candidates(api: &hidapi::HidApi, probes: &mut LayoutProbes) -> Vec<Candidate> {
    read_model_candidates(
        api,
        probes,
        &ReadModels {
            matches: &|info| {
                vial::is_vial_collection(info.serial_number(), info.usage_page(), info.usage())
            },
            read: read_vial,
            unread: &vial::UNREAD,
            keeps_serial: false,
            what: "Vial board",
        },
    )
}

/// HID LampArray devices (Windows Dynamic Lighting), on any USB id: their
/// Lighting and Illumination collection. The lamps they report make the model.
/// A keyboard already driven as a deck through its own collections (a Wooting
/// on firmware with Dynamic Lighting) is not lit a second way.
fn lamp_array_candidates(api: &hidapi::HidApi, probes: &mut LayoutProbes) -> Vec<Candidate> {
    read_model_candidates(
        api,
        probes,
        &ReadModels {
            matches: &|info| {
                lamparray::is_lamp_array_collection(info.usage_page(), info.usage())
                    && !catalog::driven_by_paired_collections(info.vendor_id(), info.product_id())
            },
            read: read_lamp_array,
            unread: &lamparray::UNREAD,
            keeps_serial: true,
            what: "LampArray",
        },
    )
}

/// The model and serial a keyboard named for itself when asked.
struct IdentityProbe {
    model: &'static DeviceModel,
    serial: Option<String>,
}

/// Keyboards whose product id does not settle their model (the PrehKeyTec POS
/// keyboards) are asked which one they are, once while they stay plugged in,
/// and take the model and serial they name. One that does not answer keeps its
/// product id's model and its USB serial.
fn ask_identities(
    api: &hidapi::HidApi,
    probes: &mut BTreeMap<CString, IdentityProbe>,
    found: &mut [Candidate],
) {
    probes.retain(|path, _| found.iter().any(|candidate| &candidate.path == path));
    for candidate in found
        .iter_mut()
        .filter(|candidate| candidate.model.protocol.asks_identity())
    {
        if !probes.contains_key(&candidate.path) {
            let Some(probe) = probe_identity(api, candidate) else {
                continue;
            };
            probes.insert(candidate.path.clone(), probe);
        }
        if let Some(probe) = probes.get(&candidate.path) {
            candidate.model = probe.model;
            candidate.serial.clone_from(&probe.serial);
        }
    }
}

/// Opens a keyboard just long enough to ask which model it is. `None` when it
/// would not open, so it is asked again next scan; the reference prefers the
/// serial it names over the USB one.
fn probe_identity(api: &hidapi::HidApi, candidate: &Candidate) -> Option<IdentityProbe> {
    let mut device = match api.open_path(&candidate.path) {
        Ok(device) => device,
        Err(error) => {
            tracing::debug!(path = ?candidate.path, %error, "ryu-decks: could not open the keyboard to ask its model");
            return None;
        }
    };
    let identity = prehkeytec::query_identity(&mut device, prehkeytec::IDENTITY_TIMEOUT)
        .unwrap_or_else(|error| {
            tracing::debug!(path = ?candidate.path, %error, "ryu-decks: the keyboard's identity query failed");
            None
        });
    let model = identity
        .as_ref()
        .and_then(prehkeytec::model_for_identity)
        .unwrap_or(candidate.model);
    if identity.is_none() {
        tracing::debug!(path = ?candidate.path, model = model.name, "ryu-decks: no identity answer; using the product id's model");
    }
    let serial = identity
        .map(|identity| identity.serial)
        .filter(|serial| !serial.is_empty())
        .or_else(|| candidate.serial.clone());
    Some(IdentityProbe { model, serial })
}

/// Reads an opened Vial board's layout.
fn read_vial(
    mut device: hidapi::HidDevice,
    facts: &ReadFacts,
) -> Result<&'static DeviceModel, String> {
    match vial::probe(&mut device) {
        Ok(board) => Ok(vial::register(
            board,
            facts.vendor_id,
            facts.product_id,
            facts.product.as_deref(),
        )),
        Err(driver::DriverError::Refused(reason)) => Err(reason),
        Err(error) => Err(open_failure(&error.to_string())),
    }
}

/// Reads an opened LampArray's report descriptor and lamps.
fn read_lamp_array(
    mut device: hidapi::HidDevice,
    facts: &ReadFacts,
) -> Result<&'static DeviceModel, String> {
    let mut descriptor = vec![0; lamparray::DESCRIPTOR_CAPACITY];
    let len = device
        .get_report_descriptor(&mut descriptor)
        .map_err(|error| open_failure(&error.to_string()))?;
    descriptor.truncate(len);
    match lamparray::probe(&mut device, &descriptor) {
        Ok(array) => Ok(lamparray::register(
            array,
            facts.vendor_id,
            facts.product_id,
            facts.product.as_deref(),
        )),
        Err(driver::DriverError::Refused(reason)) => Err(reason),
        Err(error) => Err(open_failure(&error.to_string())),
    }
}

/// The model the serial port `name` answers as, probed as a `model` deck (a
/// mext monome names its own grid or arc); `None` when it would not open
/// (busy, or not permitted), so it is asked again next scan.
fn probe_serial(model: &'static DeviceModel, name: &str) -> ProbeAnswer {
    let port = open_serial(model, name).ok()?;
    let mut link = driver::SerialLink(port);
    let answered = |answered: bool| {
        if answered {
            PortAnswer::Deck(model)
        } else {
            PortAnswer::Silent
        }
    };
    Some(match model.protocol {
        catalog::Protocol::TourBoxSerial => {
            answered(driver::tourbox::probe(&mut link).unwrap_or(false))
        }
        catalog::Protocol::DjiDuml => answered(driver::dji::probe(&mut link).unwrap_or(false)),
        catalog::Protocol::MonomeMext => match driver::monome::probe(&mut link) {
            Ok(driver::monome::MextAnswer::Model(found)) => PortAnswer::Deck(found),
            Ok(driver::monome::MextAnswer::Undriven) => PortAnswer::NotDriven,
            Ok(driver::monome::MextAnswer::Silent) | Err(_) => PortAnswer::Silent,
        },
        _ => PortAnswer::Deck(model),
    })
}

fn open_serial(
    model: &DeviceModel,
    name: &str,
) -> Result<Box<dyn serialport::SerialPort>, serialport::Error> {
    let builder = serialport::new(name, model.serial_baud()).timeout(Duration::from_millis(100));
    if model.protocol.holds_dtr() {
        let mut port = open_holding_dtr(builder.dtr_on_open(true))?;
        port.write_request_to_send(true)?;
        port.clear(serialport::ClearBuffer::Input)?;
        return Ok(port);
    }
    let port = builder.open()?;
    if matches!(
        model.protocol,
        catalog::Protocol::TourBoxSerial
            | catalog::Protocol::DjiDuml
            | catalog::Protocol::MonomeMext
            | catalog::Protocol::MonomeSeries
            | catalog::Protocol::Monome40h
    ) {
        // Anything left over from before would read as presses (or, on a DJI
        // remote or a monome, as stale replies).
        port.clear(serialport::ClearBuffer::Input)?;
    }
    Ok(port)
}

/// Opens a port whose device must keep seeing DTR, even after Ryu closes it.
/// Closing a unix tty drops DTR while HUPCL is set, which serialport never
/// clears, so it is cleared here; Windows leaves DTR alone on close.
#[cfg(unix)]
fn open_holding_dtr(
    builder: serialport::SerialPortBuilder,
) -> Result<Box<dyn serialport::SerialPort>, serialport::Error> {
    use std::os::fd::AsRawFd;
    let port = builder.open_native()?;
    keep_dtr_on_close(port.as_raw_fd())?;
    Ok(Box::new(port))
}

#[cfg(not(unix))]
fn open_holding_dtr(
    builder: serialport::SerialPortBuilder,
) -> Result<Box<dyn serialport::SerialPort>, serialport::Error> {
    builder.open()
}

/// Clears HUPCL on an open tty, so closing it leaves the modem lines as they are.
#[cfg(unix)]
fn keep_dtr_on_close(fd: std::os::fd::RawFd) -> Result<(), serialport::Error> {
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: `fd` is the open port's descriptor and `termios` is written by
    // tcgetattr before it is read; it is only read when the call succeeded.
    let mut termios = unsafe {
        if libc::tcgetattr(fd, termios.as_mut_ptr()) != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        termios.assume_init()
    };
    termios.c_cflag &= !libc::HUPCL;
    // SAFETY: `fd` is still open and `termios` is a fully initialised value.
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

/// An opened deck: its transport, and the display interface of a deck that
/// draws through a second one.
struct Opened {
    transport: Box<dyn driver::Transport>,
    display: Option<Box<dyn driver::Transport>>,
}

fn open_candidate(api: Option<&hidapi::HidApi>, candidate: &Candidate) -> Result<Opened, String> {
    if let Some(ports) = &candidate.midi {
        let link = driver::midi::open(ports)?;
        return Ok(Opened {
            transport: Box::new(link),
            display: None,
        });
    }
    if candidate.evdev {
        return open_evdev(candidate);
    }
    if candidate.serial_port {
        let name = candidate.path.to_string_lossy();
        let port =
            open_serial(candidate.model, name.as_ref()).map_err(|error| error.to_string())?;
        return Ok(Opened {
            transport: Box::new(driver::SerialLink(port)),
            display: None,
        });
    }
    let api = api.ok_or_else(|| "USB HID is unavailable".to_owned())?;
    if candidate.model.protocol == catalog::Protocol::AdafruitMacroPadHid {
        return open_macropad(api, candidate);
    }
    if candidate.model.protocol.paired_collections().is_some() {
        return open_collection_pair(api, candidate);
    }
    let display = match (
        candidate.model.protocol.paired_interfaces(),
        &candidate.display_path,
    ) {
        (None, _) => None,
        (Some(_), None) => {
            return Err("its display interface is not visible to Ryu".to_owned());
        }
        (Some(_), Some(path)) => {
            let device = api.open_path(path).map_err(|error| error.to_string())?;
            Some(Box::new(device) as Box<dyn driver::Transport>)
        }
    };
    if let Some(probe) = candidate.model.protocol.open_probe() {
        return open_probed(api, candidate, probe).map(|transport| Opened { transport, display });
    }
    let mut opened = open_hid(api, candidate);
    for delay in candidate.model.protocol.open_retry_delays_ms() {
        if opened.is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(*delay));
        opened = api.open_path(&candidate.path);
    }
    let device = opened.map_err(|error| error.to_string())?;
    Ok(Opened {
        transport: Box::new(device),
        display,
    })
}

/// Opens a Linux input event node found by [`evdev_candidates`].
#[cfg(target_os = "linux")]
fn open_evdev(candidate: &Candidate) -> Result<Opened, String> {
    let path = candidate.path.to_string_lossy();
    let link = driver::evdev::EvdevLink::open(path.as_ref()).map_err(|error| error.to_string())?;
    Ok(Opened {
        transport: Box::new(link),
        display: None,
    })
}

#[cfg(not(target_os = "linux"))]
fn open_evdev(_candidate: &Candidate) -> Result<Opened, String> {
    Err("input event nodes exist only on Linux".to_owned())
}

/// Decks the Linux kernel keeps from hidraw and drives with an input driver of
/// its own (the Griffin PowerMate), found by their input event nodes.
#[cfg(target_os = "linux")]
fn evdev_candidates() -> Vec<Candidate> {
    driver::evdev::nodes()
        .into_iter()
        .filter_map(|node| {
            let model = catalog::identify_evdev(node.vendor_id, node.product_id)?;
            Some(Candidate {
                model,
                serial: catalog::usable_serial(model, node.uniq.as_deref()).map(str::to_owned),
                path: node.path,
                interface: 0,
                usage_page: 0,
                wired: true,
                serial_port: false,
                evdev: true,
                midi: None,
                display_path: None,
                display_usage_page: 0,
                alternates: Vec::new(),
                refusal: None,
            })
        })
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn evdev_candidates() -> Vec<Candidate> {
    Vec::new()
}

/// Decks on USB MIDI (the Novation Launchpads), found by their port names.
/// Two identical decks get different paths, so the second is told apart by
/// [`select_candidates`].
fn midi_candidates(scanner: &mut driver::midi::Scanner) -> Vec<Candidate> {
    scanner
        .scan()
        .into_iter()
        .filter_map(|found| {
            let path = CString::new(found.path()).ok()?;
            Some(Candidate {
                model: found.model,
                serial: None,
                path,
                interface: 0,
                usage_page: 0,
                wired: true,
                serial_port: false,
                evdev: false,
                midi: Some(found.ports),
                display_path: None,
                display_usage_page: 0,
                alternates: Vec::new(),
                refusal: None,
            })
        })
        .collect()
}

/// Opens an Adafruit MacroPad's collections and keeps the ones whose report
/// descriptors are the Companion firmware's: one path carrying both roles
/// (Linux, macOS), or the key collection and the colour collection apart
/// (Windows). A board on any other CircuitPython code is refused with
/// [`MACROPAD_FIRMWARE_MISSING`] in the message.
fn open_macropad(api: &hidapi::HidApi, candidate: &Candidate) -> Result<Opened, String> {
    let mut keys: Option<hidapi::HidDevice> = None;
    let mut lights: Option<hidapi::HidDevice> = None;
    let mut failure = None;
    let mut read_any = false;
    for path in std::iter::once(&candidate.path).chain(&candidate.alternates) {
        let device = match api.open_path(path) {
            Ok(device) => device,
            Err(error) => {
                failure = Some(error.to_string());
                continue;
            }
        };
        let mut descriptor = vec![0; 4096];
        let roles = match device.get_report_descriptor(&mut descriptor) {
            Ok(len) => driver::macropad::collection_roles(descriptor.get(..len).unwrap_or(&[])),
            Err(error) => {
                failure = Some(error.to_string());
                continue;
            }
        };
        read_any = true;
        match (roles.reads_keys, roles.takes_colours) {
            (true, true) => {
                keys = Some(device);
                lights = None;
                break;
            }
            (true, false) if keys.is_none() => keys = Some(device),
            (false, true) if lights.is_none() => lights = Some(device),
            _ => {}
        }
        if keys.is_some() && lights.is_some() {
            break;
        }
    }
    match (keys, lights) {
        (Some(keys), lights) => Ok(Opened {
            transport: Box::new(keys),
            display: lights.map(|device| Box::new(device) as Box<dyn driver::Transport>),
        }),
        (None, _) if !read_any => Err(failure.unwrap_or_else(|| "no collection opened".to_owned())),
        (None, _) => Err(MACROPAD_FIRMWARE_MISSING.to_owned()),
    }
}

/// Why an Adafruit MacroPad on other CircuitPython code is not a deck.
const MACROPAD_FIRMWARE_MISSING: &str = "This MacroPad is not running the Companion HID firmware. Copy boot.py, code.py, settings.py and lib/neopixel.mpy from maximkulkin/nodejs-macropad onto its CIRCUITPY drive, then press its reset button.";

/// Opens a deck driven through two collections told apart by usage page,
/// each wrapped with its page. A lighting collection that will not open
/// leaves the deck's keys readable but unlit.
fn open_collection_pair(api: &hidapi::HidApi, candidate: &Candidate) -> Result<Opened, String> {
    let device = api
        .open_path(&candidate.path)
        .map_err(|error| error.to_string())?;
    let display = candidate
        .display_path
        .as_ref()
        .and_then(|path| match api.open_path(path) {
            Ok(device) => Some(Box::new(driver::HidCollection {
                device,
                usage_page: candidate.display_usage_page,
            }) as Box<dyn driver::Transport>),
            Err(error) => {
                tracing::warn!(model = candidate.model.id, %error, "ryu-decks: the lighting collection would not open");
                None
            }
        });
    Ok(Opened {
        transport: Box::new(driver::HidCollection {
            device,
            usage_page: candidate.usage_page,
        }),
        display,
    })
}

/// Opens a candidate's HID path: exclusively first where the protocol asks
/// for it and the host can (macOS), then shared. hidapi's exclusive setting is
/// process-wide, so it is put back to shared straight after this one open.
fn open_hid(api: &hidapi::HidApi, candidate: &Candidate) -> hidapi::HidResult<hidapi::HidDevice> {
    #[cfg(target_os = "macos")]
    if candidate.model.protocol.opens_exclusive_first() {
        api.set_open_exclusive(true);
        let exclusive = api.open_path(&candidate.path);
        api.set_open_exclusive(false);
        match exclusive {
            Ok(device) => return Ok(device),
            Err(error) => {
                tracing::debug!(%error, "ryu-decks: exclusive open refused; opening shared");
            }
        }
    }
    api.open_path(&candidate.path)
}

/// The first of a candidate's paths that opens and takes `probe`.
fn open_probed(
    api: &hidapi::HidApi,
    candidate: &Candidate,
    probe: &[u8],
) -> Result<Box<dyn driver::Transport>, String> {
    let mut failure = String::new();
    for path in std::iter::once(&candidate.path).chain(&candidate.alternates) {
        let attempt = api.open_path(path).and_then(|device| {
            device.write(probe)?;
            Ok(device)
        });
        match attempt {
            Ok(device) => return Ok(Box::new(device)),
            Err(error) => failure = error.to_string(),
        }
    }
    Err(failure)
}

/// One HID collection seen by a scan, as far as pairing interfaces goes.
struct Sibling<'a> {
    vendor_id: u16,
    product_id: u16,
    serial: Option<&'a str>,
    interface: i32,
    usage_page: u16,
    path: &'a CStr,
}

/// How many leading bytes two HID paths share.
fn shared_prefix(a: &CStr, b: &CStr) -> usize {
    a.to_bytes()
        .iter()
        .zip(b.to_bytes())
        .take_while(|(x, y)| x == y)
        .count()
}

/// Whether two collections are on the same USB ids and serial.
fn same_device(a: &Sibling<'_>, b: &Sibling<'_>) -> bool {
    a.vendor_id == b.vendor_id && a.product_id == b.product_id && a.serial == b.serial
}

/// Whether the same device lists a control collection on a page `pages`
/// prefers to this one's (a Wooting with both analog collections is read
/// through the one that gives key positions).
fn outranked_control(control: &Sibling<'_>, pages: &[u16], all: &[Sibling<'_>]) -> bool {
    let rank = |page: u16| pages.iter().position(|preferred| *preferred == page);
    let Some(mine) = rank(control.usage_page) else {
        return false;
    };
    all.iter().any(|other| {
        same_device(other, control) && rank(other.usage_page).is_some_and(|theirs| theirs < mine)
    })
}

/// The collection that belongs with a control collection on a deck paired by
/// usage page: same USB ids and serial, a page in `pages` (earlier preferred),
/// then the path nearest the control path. A collection on the control's own
/// path (one HID node carrying both) is not a separate one to open.
fn collection_sibling(
    control: &Sibling<'_>,
    pages: &[u16],
    all: &[Sibling<'_>],
) -> Option<(CString, u16)> {
    let rank = |page: u16| pages.iter().position(|preferred| *preferred == page);
    all.iter()
        .filter(|other| same_device(other, control) && other.path != control.path)
        .filter_map(|other| rank(other.usage_page).map(|at| (at, other)))
        .max_by_key(|(at, other)| {
            (
                std::cmp::Reverse(*at),
                shared_prefix(other.path, control.path),
            )
        })
        .map(|(_, other)| (other.path.to_owned(), other.usage_page))
}

/// The display interface that belongs with a control interface: same USB ids,
/// same serial, the display interface number. With several to choose from (two
/// decks without serials), the path sharing the longest prefix with the control
/// path wins, which keeps a USB location together where the path carries one.
fn display_sibling(control: &Sibling<'_>, display: i32, all: &[Sibling<'_>]) -> Option<CString> {
    all.iter()
        .filter(|other| same_device(other, control) && other.interface == display)
        .max_by_key(|other| shared_prefix(other.path, control.path))
        .map(|other| other.path.to_owned())
}

/// A refusal to open, remembered so a rescan does not retry (and log) every two
/// seconds.
struct Refusal {
    message: String,
    at: Instant,
}

/// FNV-1a over a HID path, to tell apart identical decks that report no serial.
fn path_tag(path: &CString) -> String {
    let hash = path
        .as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("p{:08x}", hash & 0xffff_ffff)
}

/// Pick one collection per physical deck, keyed by device id.
///
/// A deck may enumerate several collections (several interfaces, or the same
/// keypad over USB and Bluetooth); the best-ranked one wins. Two candidates on
/// the same interface and usage page but different paths, with no serial to tell
/// them apart, are two physical decks of one model: the later one gets a
/// path-derived suffix instead of being silently dropped.
/// Whether an open MIDI deck came back on other ports between two scans. Its
/// path leaves out ALSA's client number, which changes on replug, so the deck
/// would look still present; the old link cannot tell it is dead (its input
/// callback is its own, and sends to a gone ALSA port do not fail). The ports
/// as listed keep the client number, so a change in them is a replug.
fn midi_replugged(
    opened: Option<&driver::midi::PortPair>,
    listed: Option<&driver::midi::PortPair>,
) -> bool {
    matches!((opened, listed), (Some(opened), Some(listed)) if opened != listed)
}

/// [`select_candidates_sticky`] with no memory of earlier scans.
#[cfg(test)]
fn select_candidates(found: impl IntoIterator<Item = Candidate>) -> BTreeMap<String, Candidate> {
    select_candidates_sticky(found, &mut BTreeMap::new())
}

/// [`select_candidates`], keeping each serial-less deck's id with its path.
///
/// Without a serial, only the path tells two identical decks apart, and the
/// first one scanned would take the bare model id. `sticky` (path to id, from
/// the previous scan) lets a deck that keeps its path keep its id when its twin
/// is unplugged or the OS lists the pair in another order, so neither inherits
/// the other's layout. Candidates are also taken in path order, not listing
/// order, so a first scan hands out ids the same way every time.
fn select_candidates_sticky(
    found: impl IntoIterator<Item = Candidate>,
    sticky: &mut BTreeMap<CString, String>,
) -> BTreeMap<String, Candidate> {
    let mut found: Vec<Candidate> = found.into_iter().collect();
    // Decks that already have an id claim it before a newcomer can take it bare.
    found.sort_by(|a, b| {
        (!sticky.contains_key(&a.path), &a.path).cmp(&(!sticky.contains_key(&b.path), &b.path))
    });
    let mut present: BTreeMap<String, Candidate> = BTreeMap::new();
    for candidate in found {
        let mut id = catalog::device_id(candidate.model, candidate.serial.as_deref());
        let serial_less = candidate.serial.as_deref().unwrap_or_default().is_empty();
        let remembered = serial_less
            .then(|| sticky.get(&candidate.path))
            .flatten()
            .filter(|known| **known == id || known.starts_with(&format!("{id}-")));
        if let Some(known) = remembered {
            id.clone_from(known);
        }
        if let Some(existing) = present.get_mut(&id) {
            // Another collection of the same deck, on another path: kept as a
            // path to try if the protocol must probe for the right one.
            let same_deck_elsewhere = candidate.model.protocol.keeps_alternate_paths()
                && !serial_less
                && existing.path != candidate.path
                && existing.interface == candidate.interface
                && existing.usage_page == candidate.usage_page
                && existing.wired == candidate.wired;
            if same_deck_elsewhere {
                if !existing.alternates.contains(&candidate.path) {
                    existing.alternates.push(candidate.path);
                }
                continue;
            }
        }
        let another_deck = remembered.is_none()
            && present.get(&id).is_some_and(|existing| {
                serial_less
                    && existing.path != candidate.path
                    && existing.interface == candidate.interface
                    && existing.usage_page == candidate.usage_page
                    && existing.wired == candidate.wired
            });
        if another_deck {
            id = format!("{id}-{}", path_tag(&candidate.path));
        }
        let keep_existing = present
            .get(&id)
            .is_some_and(|existing| existing.rank() <= candidate.rank());
        if !keep_existing {
            present.insert(id, candidate);
        }
    }
    // Remember this scan's serial-less decks, and forget paths that left.
    *sticky = present
        .iter()
        .filter(|(_, candidate)| candidate.serial.as_deref().unwrap_or_default().is_empty())
        .map(|(id, candidate)| (candidate.path.clone(), id.clone()))
        .collect();
    present
}

struct DeviceLoop {
    store: Arc<Store>,
    presses: UnboundedSender<Press>,
    snapshot: Arc<RwLock<Snapshot>>,
    open: BTreeMap<String, OpenDeck>,
    /// Decks that refused to open; retried after [`RETRY_REFUSED`] and cleared
    /// when the deck is unplugged.
    refused: BTreeMap<String, Refusal>,
    /// Dropped for good once the main thread stops answering.
    touch_bar: Option<touchbar::Link>,
    /// Serial ports on a generic USB id, by name: whether a deck answered the
    /// probe there. Forgotten when the port goes away.
    serial_probes: SerialProbes,
    /// Vial boards by HID path: the model their layout made, or why not.
    vial_probes: LayoutProbes,
    /// LampArrays by HID path: the model their lamps made, or why not.
    lamp_array_probes: LayoutProbes,
    /// Keyboards that name their own model, by HID path: what they said.
    identity_probes: BTreeMap<CString, IdentityProbe>,
    /// Lists USB MIDI ports through one long-lived client.
    midi: driver::midi::Scanner,
    /// Serial-less decks by HID path: the id each was given, so a deck that keeps
    /// its path keeps its id (and layout) whatever order the OS lists its twin in.
    serial_less_ids: BTreeMap<CString, String>,
}

impl DeviceLoop {
    fn new(
        store: Arc<Store>,
        presses: UnboundedSender<Press>,
        snapshot: Arc<RwLock<Snapshot>>,
        touch_bar: Option<touchbar::Link>,
    ) -> Self {
        Self {
            store,
            presses,
            snapshot,
            open: BTreeMap::new(),
            refused: BTreeMap::new(),
            touch_bar,
            serial_probes: SerialProbes::default(),
            vial_probes: LayoutProbes::default(),
            lamp_array_probes: LayoutProbes::default(),
            identity_probes: BTreeMap::new(),
            midi: driver::midi::Scanner::default(),
            serial_less_ids: BTreeMap::new(),
        }
    }

    fn run(mut self, inbox: &Receiver<Command>) {
        let mut api = None;
        let mut last_scan: Option<Instant> = None;
        let mut last_heartbeat = Instant::now();
        loop {
            if last_scan.is_none_or(|at| at.elapsed() >= SCAN_INTERVAL) {
                last_scan = Some(Instant::now());
                self.scan(&mut api);
            }
            loop {
                match inbox.try_recv() {
                    Ok(command) => self.handle(command),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            if last_heartbeat.elapsed() >= HEARTBEAT_INTERVAL {
                last_heartbeat = Instant::now();
                let mut lost = Vec::new();
                for (id, deck) in &mut self.open {
                    if let Err(error) = deck.driver.heartbeat() {
                        tracing::info!(device = %id, %error, "ryu-decks: deck stopped answering");
                        lost.push(id.clone());
                    }
                }
                for id in lost {
                    self.disconnect(&id);
                }
            }
            if self.open.is_empty() {
                std::thread::sleep(IDLE_SLEEP);
            } else {
                self.poll_all();
            }
        }
    }

    fn poll_all(&mut self) {
        let mut lost = Vec::new();
        let mut locks = Vec::new();
        for (id, deck) in &mut self.open {
            let lock = deck.driver.lock_state();
            if lock != deck.lock {
                deck.lock.clone_from(&lock);
                locks.push((id.clone(), lock));
            }
            match deck.driver.poll(POLL_TIMEOUT_MS) {
                Ok(inputs) => {
                    for input in inputs {
                        let press = Press {
                            device_id: id.clone(),
                            model: deck.model,
                            input,
                        };
                        if self.presses.send(press).is_err() {
                            return;
                        }
                    }
                }
                Err(error) => {
                    tracing::info!(device = %id, %error, "ryu-decks: deck disconnected");
                    lost.push(id.clone());
                }
            }
        }
        for (id, lock) in locks {
            self.update(|snapshot| {
                if let Some(status) = snapshot.devices.get_mut(&id) {
                    status.lock = lock;
                }
            });
        }
        for id in lost {
            self.disconnect(&id);
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Repaint(id) => {
                if let Some(deck) = self.open.get_mut(&id) {
                    if let Err(error) = paint(&self.store, &id, deck) {
                        tracing::warn!(device = %id, %error, "ryu-decks: repaint failed");
                    }
                }
            }
            Command::Brightness(id, percent) => {
                if let Some(deck) = self.open.get_mut(&id) {
                    if let Err(error) = deck.driver.set_brightness(percent) {
                        tracing::warn!(device = %id, %error, "ryu-decks: brightness failed");
                    }
                }
            }
            Command::Unlock(id) => {
                if let Some(deck) = self.open.get_mut(&id) {
                    if let Err(error) = deck.driver.start_unlock() {
                        tracing::warn!(device = %id, %error, "ryu-decks: unlock failed");
                    }
                }
            }
        }
    }

    fn scan(&mut self, api: &mut Option<hidapi::HidApi>) {
        if api.is_none() {
            match hidapi::HidApi::new() {
                Ok(fresh) => {
                    *api = Some(fresh);
                    self.update(|snapshot| snapshot.hid_error = None);
                }
                Err(error) => {
                    // Serial and MIDI decks (Loupedeck, TourBox, Launchpad) still work without HID.
                    self.update(|snapshot| {
                        snapshot.hid_error = Some(format!("USB HID is unavailable: {error}"));
                    });
                }
            }
        }
        let mut found = Vec::new();
        if let Some(api) = api.as_mut() {
            if let Err(error) = api.refresh_devices() {
                tracing::warn!(%error, "ryu-decks: HID rescan failed");
                return;
            }
            let siblings: Vec<Sibling<'_>> = api
                .device_list()
                .map(|info| Sibling {
                    vendor_id: info.vendor_id(),
                    product_id: info.product_id(),
                    serial: info.serial_number(),
                    interface: info.interface_number(),
                    usage_page: info.usage_page(),
                    path: info.path(),
                })
                .collect();
            found.extend(
                api.device_list()
                    .zip(&siblings)
                    .filter_map(|(info, sibling)| {
                        let model = catalog::identify(
                            info.vendor_id(),
                            info.product_id(),
                            info.usage_page(),
                        )?;
                        // The Galleon's panel is one interface of a keyboard.
                        if model.hid_interface().is_some_and(|panel| {
                            !panel.accepts(info.interface_number(), info.usage())
                        }) {
                            return None;
                        }
                        // An Xbox controller's other collections are not its gamepad.
                        if !model
                            .protocol
                            .accepts_collection(info.usage_page(), info.usage())
                        {
                            return None;
                        }
                        let wired = !matches!(info.bus_type(), hidapi::BusType::Bluetooth);
                        if model.protocol.wired_only() && !wired {
                            return None;
                        }
                        // A deck on two interfaces is found by its control interface only.
                        let mut display_path = match model.protocol.paired_interfaces() {
                            Some(pair) if info.interface_number() != pair.control => return None,
                            Some(pair) => display_sibling(sibling, pair.display, &siblings),
                            None => None,
                        };
                        // A deck on two collections told apart by page: the
                        // preferred control collection, and its lighting one.
                        let mut display_usage_page = 0;
                        if let Some(pair) = model.protocol.paired_collections() {
                            if outranked_control(sibling, pair.control, &siblings) {
                                return None;
                            }
                            if let Some((path, page)) =
                                collection_sibling(sibling, pair.display, &siblings)
                            {
                                display_path = Some(path);
                                display_usage_page = page;
                            }
                        }
                        Some(Candidate {
                            model,
                            serial: catalog::usable_serial(model, info.serial_number())
                                .map(str::to_owned),
                            path: info.path().to_owned(),
                            interface: info.interface_number(),
                            usage_page: info.usage_page(),
                            wired,
                            serial_port: false,
                            evdev: false,
                            midi: None,
                            display_path,
                            display_usage_page,
                            alternates: Vec::new(),
                            refusal: None,
                        })
                    }),
            );
            ask_identities(api, &mut self.identity_probes, &mut found);
            found.extend(vial_candidates(api, &mut self.vial_probes));
            found.extend(lamp_array_candidates(api, &mut self.lamp_array_probes));
        }
        found.extend(serial_candidates(&mut self.serial_probes));
        found.extend(evdev_candidates());
        found.extend(midi_candidates(&mut self.midi));
        let present = select_candidates_sticky(found, &mut self.serial_less_ids);

        let gone: Vec<String> = self
            .open
            .keys()
            .chain(self.refused.keys())
            .filter(|id| !present.contains_key(*id) && id.as_str() != touchbar::MODEL_ID)
            .cloned()
            .chain(self.open.iter().filter_map(|(id, deck)| {
                let candidate = present.get(id)?;
                midi_replugged(deck.midi_ports.as_ref(), candidate.midi.as_ref())
                    .then(|| id.clone())
            }))
            .collect();
        for id in gone {
            self.disconnect(&id);
        }
        self.attach_touch_bar();

        for (id, candidate) in present {
            let waiting = self
                .refused
                .get(&id)
                .is_some_and(|refusal| refusal.at.elapsed() < RETRY_REFUSED);
            if self.open.contains_key(&id) || waiting {
                continue;
            }
            if let Some(reason) = candidate.refusal {
                self.record_refusal(id, candidate.model, None, &reason, reason.clone());
                continue;
            }
            match open_candidate(api.as_ref(), &candidate) {
                Ok(opened) => {
                    let mut deck = OpenDeck {
                        model: candidate.model,
                        driver: driver::for_model_with_display(
                            candidate.model,
                            opened.transport,
                            opened.display,
                        ),
                        lock: None,
                        midi_ports: candidate.midi.clone(),
                    };
                    let painted = paint(&self.store, &id, &mut deck).and_then(|()| {
                        match self.store.brightness(&id).ok().flatten() {
                            Some(percent) => deck.driver.set_brightness(percent),
                            None => Ok(()),
                        }
                    });
                    if let Err(driver::DriverError::Refused(reason)) = painted {
                        self.refuse(id, candidate.model, candidate.serial, &reason);
                        continue;
                    }
                    if let Err(error) = painted {
                        tracing::warn!(device = %id, %error, "ryu-decks: first paint failed");
                    }
                    tracing::info!(device = %id, model = candidate.model.name, "ryu-decks: deck connected");
                    self.refused.remove(&id);
                    deck.lock = deck.driver.lock_state();
                    let lock = deck.lock.clone();
                    self.open.insert(id.clone(), deck);
                    self.update(|snapshot| {
                        snapshot.devices.insert(
                            id,
                            DeviceStatus {
                                model: candidate.model,
                                serial: candidate.serial,
                                connected: true,
                                error: None,
                                lock,
                            },
                        );
                    });
                }
                Err(error) => self.refuse(id, candidate.model, candidate.serial, &error),
            }
        }
    }

    /// Remembers a deck that would not open (or would not work once opened),
    /// so it is retried only after [`RETRY_REFUSED`], and shows why.
    fn refuse(
        &mut self,
        id: String,
        model: &'static DeviceModel,
        serial: Option<String>,
        error: &str,
    ) {
        self.record_refusal(id, model, serial, error, open_failure(error));
    }

    /// [`DeviceLoop::refuse`], with the message to show already written.
    fn record_refusal(
        &mut self,
        id: String,
        model: &'static DeviceModel,
        serial: Option<String>,
        error: &str,
        message: String,
    ) {
        let repeated = self
            .refused
            .get(&id)
            .is_some_and(|refusal| refusal.message == message);
        if repeated {
            tracing::debug!(device = %id, %error, "ryu-decks: deck still refuses to open");
        } else {
            tracing::warn!(device = %id, %error, "ryu-decks: could not open deck");
        }
        self.refused.insert(
            id.clone(),
            Refusal {
                message: message.clone(),
                at: Instant::now(),
            },
        );
        self.update(|snapshot| {
            snapshot.devices.insert(
                id,
                DeviceStatus {
                    model,
                    serial,
                    connected: false,
                    error: Some(message),
                    lock: None,
                },
            );
        });
    }

    /// Add the Touch Bar as a deck, once, if this Mac has one. It is never
    /// unplugged; if its main thread stops, Ryu stops trying.
    fn attach_touch_bar(&mut self) {
        let Some(link) = self.touch_bar.clone() else {
            return;
        };
        if self.open.contains_key(touchbar::MODEL_ID) {
            return;
        }
        let Some(model) = catalog::by_id(touchbar::MODEL_ID) else {
            return;
        };
        let mut deck = OpenDeck {
            model,
            driver: Box::new(TouchBar::new(model, link)),
            lock: None,
            midi_ports: None,
        };
        if let Err(error) = paint(&self.store, touchbar::MODEL_ID, &mut deck) {
            tracing::warn!(%error, "ryu-decks: the Touch Bar is not answering; leaving it alone");
            self.touch_bar = None;
            return;
        }
        tracing::info!("ryu-decks: Touch Bar attached");
        self.open.insert(touchbar::MODEL_ID.to_owned(), deck);
        self.update(|snapshot| {
            snapshot.devices.insert(
                touchbar::MODEL_ID.to_owned(),
                DeviceStatus {
                    model,
                    serial: None,
                    connected: true,
                    error: None,
                    lock: None,
                },
            );
        });
    }

    fn disconnect(&mut self, id: &str) {
        self.open.remove(id);
        self.refused.remove(id);
        self.update(|snapshot| {
            // A Vial board or LampArray that could not be read has no layout to keep.
            if snapshot.devices.get(id).is_some_and(|status| {
                std::ptr::eq(status.model, &vial::UNREAD)
                    || std::ptr::eq(status.model, &lamparray::UNREAD)
            }) {
                snapshot.devices.remove(id);
            } else if let Some(status) = snapshot.devices.get_mut(id) {
                status.connected = false;
                status.error = None;
                status.lock = None;
            }
        });
    }

    fn update(&self, change: impl FnOnce(&mut Snapshot)) {
        let mut guard = self
            .snapshot
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        change(&mut guard);
    }
}

/// A user-facing reason for a deck that is plugged in but would not open.
fn open_failure(raw: &str) -> String {
    if raw == MACROPAD_FIRMWARE_MISSING {
        return raw.to_owned();
    }
    let lower = raw.to_ascii_lowercase();
    if cfg!(target_os = "linux") && (lower.contains("permission") || lower.contains("access")) {
        return "Linux is not letting Ryu open this deck. Install the udev rule from the Decks guide, then unplug and replug it.".to_owned();
    }
    if lower.contains("exclusive") || lower.contains("busy") {
        return "Another app is using this deck. Quit the maker's app (Stream Deck, Ulanzi Studio, Work Louder Input, Loupedeck, Razer Synapse, Logi Options+, Logitech G HUB, Mountain Base Camp, the Xencelabs driver, TourBox Console, 3DxWare, Yamaha ControlCenter, DJI Assistant, GameSir Connect, SteelSeries GG, Novation Components, Ableton Live, Maschine, Traktor or Komplete Kontrol with Native Instruments' NIHardwareAgent and NIHostIntegrationAgent, serialosc, Wootility or the Mirabox / Ajazz app), turn off Windows Dynamic Lighting or OpenRGB for the device, and replug it.".to_owned();
    }
    format!("Ryu could not open this deck: {raw}. Quit the maker's app if it is running, then replug it.")
}

/// Load a deck's layout from the store and send every key face.
fn paint(store: &Store, device_id: &str, deck: &mut OpenDeck) -> driver::DriverResult<()> {
    let bindings = store.bindings(device_id).unwrap_or_else(|error| {
        tracing::warn!(device = %device_id, %error, "ryu-decks: could not read the layout");
        Vec::new()
    });
    for key in 0..deck.model.total_keys() {
        let face = bindings
            .iter()
            .find(|binding| binding.key == key)
            .map(face_of);
        deck.driver.set_face(key, face.as_ref())?;
    }
    if deck.model.deck_lights > 0 {
        let stored = store.deck_light(device_id).unwrap_or_else(|error| {
            tracing::warn!(device = %device_id, %error, "ryu-decks: could not read the deck lights");
            None
        });
        let color = stored.and_then(|color| parse_hex_color(&color));
        deck.driver.set_deck_light(color)?;
    }
    let lights = deck.model.dial_lights;
    if lights != catalog::DialLights::None {
        let colors = store.dial_colors(device_id).unwrap_or_else(|error| {
            tracing::warn!(device = %device_id, %error, "ryu-decks: could not read the dial lights");
            Default::default()
        });
        for dial in 0..deck.model.dials {
            // A shared light takes the one colour stored for dial 0.
            let stored = if lights == catalog::DialLights::Shared {
                colors.get(&0)
            } else {
                colors.get(&dial)
            };
            let color = stored.and_then(|color| parse_hex_color(color));
            deck.driver.set_dial_light(dial, color)?;
        }
    }
    deck.driver.flush()
}

/// The face a stored binding paints.
#[must_use]
pub fn face_of(binding: &Binding) -> Face {
    Face {
        label: binding.label.clone(),
        color: parse_hex_color(&binding.color).unwrap_or([0, 0, 0]),
        image: binding
            .face_png
            .as_deref()
            .and_then(|png| image::load_from_memory(png).ok())
            .map(image::DynamicImage::into_rgba8),
    }
}

/// `#rrggbb` to bytes.
#[must_use]
pub fn parse_hex_color(value: &str) -> Option<[u8; 3]> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::testing::Recorder;
    use crate::store::Action;

    #[test]
    fn a_midi_deck_back_on_new_alsa_ports_is_reopened() {
        let pair = |client: u8| driver::midi::PortPair {
            input: format!("Launchpad X:Launchpad X LPX MIDI {client}:1"),
            output: format!("Launchpad X:Launchpad X LPX MIDI {client}:1"),
            input_nth: 0,
            output_nth: 0,
        };
        // Same path, but ALSA gave the replugged deck another client.
        assert!(midi_replugged(Some(&pair(24)), Some(&pair(28))));
        assert!(!midi_replugged(Some(&pair(24)), Some(&pair(24))));
        // Not a MIDI deck.
        assert!(!midi_replugged(None, None));
    }

    #[test]
    fn silent_serial_ports_are_asked_again_only_where_the_protocol_wants_it() {
        let now = Instant::now();
        let silent = SerialProbe {
            answer: PortAnswer::Silent,
            serial: Some("3NZ".to_owned()),
            at: now,
        };
        let later = now + SERIAL_REPROBE;
        assert!(should_probe(None, false, false, now));
        // A TourBox-style probe is final until the port goes away.
        assert!(!should_probe(Some(&silent), false, false, later));
        // A DJI remote's silent port is asked again after the pause...
        assert!(!should_probe(Some(&silent), true, false, now));
        assert!(should_probe(Some(&silent), true, false, later));
        // ...unless its sibling answered, which makes it the silent one.
        assert!(!should_probe(Some(&silent), true, true, later));
        // A port that answered is never asked again.
        let answered = SerialProbe {
            answer: PortAnswer::Deck(catalog::by_id("dji-rc-n1").expect("listed")),
            serial: Some("3NZ".to_owned()),
            at: now,
        };
        assert!(!should_probe(Some(&answered), true, false, later));
        // Nor is one that answered as a deck Ryu does not drive (a 16 x 16
        // monome grid), which would otherwise be reopened every pause.
        let undriven = SerialProbe {
            answer: PortAnswer::NotDriven,
            ..silent
        };
        assert!(!should_probe(Some(&undriven), true, false, later));
    }

    #[test]
    fn a_read_device_is_read_again_only_after_a_failure_and_never_twice_at_once() {
        let now = Instant::now();
        let failed = LayoutProbe {
            result: Err("no lamps".to_owned()),
            at: now,
        };
        assert!(should_read(None, false, now));
        // A read still running on its thread is waited for, not repeated.
        assert!(!should_read(None, true, now));
        assert!(!should_read(Some(&failed), false, now));
        assert!(should_read(Some(&failed), false, now + RETRY_REFUSED));
        assert!(!should_read(Some(&failed), true, now + RETRY_REFUSED));
        let read = LayoutProbe {
            result: Ok(&lamparray::UNREAD),
            at: now,
        };
        assert!(!should_read(Some(&read), false, now + RETRY_REFUSED));
    }

    #[test]
    fn a_running_serial_probe_is_waited_for_not_joined() {
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let mut running = RunningProbe {
            serial: None,
            handle: std::thread::spawn(move || {
                wait.recv().ok();
                Some(PortAnswer::Deck(
                    catalog::by_id("dji-rc-n1").expect("listed"),
                ))
            }),
        };
        // Still asking: handed back at once instead of blocking the loop.
        running = match running.finish() {
            Err(running) => running,
            Ok(answer) => panic!("finished before it was released: {answer:?}"),
        };
        release.send(()).expect("probe thread is waiting");
        while !running.handle.is_finished() {
            std::thread::yield_now();
        }
        assert!(
            matches!(running.finish(), Ok(Some(PortAnswer::Deck(model))) if model.id == "dji-rc-n1")
        );
    }

    #[test]
    fn hex_colours_parse_strictly() {
        assert_eq!(parse_hex_color("#2D7ff9"), Some([0x2d, 0x7f, 0xf9]));
        assert_eq!(parse_hex_color("2d7ff9"), None);
        assert_eq!(parse_hex_color("#2d7ff"), None);
        assert_eq!(parse_hex_color("#zz7ff9"), None);
    }

    #[test]
    fn painting_sends_every_key_bound_or_not() {
        let store = Store::in_memory().unwrap();
        store
            .put_binding(&Binding {
                device_id: "openai-codex-micro".to_owned(),
                key: 2,
                label: "Review".to_owned(),
                color: "#00ff00".to_owned(),
                face_png: None,
                action: Action::None,
                updated_at: 0,
            })
            .unwrap();
        let recorder = Recorder::default();
        let model = catalog::by_id("openai-codex-micro").unwrap();
        let mut deck = OpenDeck {
            model,
            driver: driver::for_model(model, Box::new(recorder.clone())),
            lock: None,
            midi_ports: None,
        };
        paint(&store, "openai-codex-micro", &mut deck).unwrap();
        let sent: Vec<u8> = recorder
            .writes()
            .iter()
            .flat_map(|report| report[3..3 + usize::from(report[2])].to_vec())
            .collect();
        let message: serde_json::Value =
            serde_json::from_slice(String::from_utf8(sent).unwrap().trim().as_bytes()).unwrap();
        let params = message["params"].as_array().unwrap();
        assert_eq!(params.len(), 6);
        assert_eq!(params[2]["c"], 0x00ff00);
        assert_eq!(params[0]["e"], 0);
    }

    fn candidate(
        model: &str,
        serial: Option<&str>,
        path: &str,
        interface: i32,
        wired: bool,
    ) -> Candidate {
        Candidate {
            model: catalog::by_id(model).unwrap(),
            serial: serial.map(str::to_owned),
            path: CString::new(path).unwrap(),
            interface,
            usage_page: 0xff00,
            wired,
            serial_port: false,
            evdev: false,
            midi: None,
            display_path: None,
            display_usage_page: 0,
            alternates: Vec::new(),
            refusal: None,
        }
    }

    #[test]
    fn a_keypad_on_usb_and_bluetooth_is_driven_over_usb() {
        let picked = select_candidates([
            candidate("openai-codex-micro", Some("A1"), "bt", -1, false),
            candidate("openai-codex-micro", Some("A1"), "usb", 2, true),
        ]);
        assert_eq!(picked.len(), 1);
        assert_eq!(
            picked["openai-codex-micro-a1"].path.to_str().unwrap(),
            "usb"
        );
    }

    #[test]
    fn one_deck_with_several_interfaces_opens_once() {
        let picked = select_candidates([
            candidate("ulanzi-d200", None, "if1", 1, true),
            candidate("ulanzi-d200", None, "if0", 0, true),
        ]);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked["ulanzi-d200"].path.to_str().unwrap(), "if0");
    }

    #[test]
    fn a_displaypad_pairs_its_control_interface_with_its_display_interface() {
        let path = |raw: &'static str| CStr::from_bytes_with_nul(raw.as_bytes()).unwrap();
        let sibling = |serial, interface, raw| Sibling {
            vendor_id: 0x3282,
            product_id: 0x0009,
            serial,
            interface,
            usage_page: 0xff00,
            path: path(raw),
        };
        let all = [
            sibling(Some("A"), 0, "1-2:1.0\0"),
            sibling(Some("A"), 1, "1-2:1.1\0"),
            sibling(Some("A"), 3, "1-2:1.3\0"),
            sibling(Some("B"), 1, "1-4:1.1\0"),
            sibling(Some("B"), 3, "1-4:1.3\0"),
            Sibling {
                vendor_id: 0x0fd9,
                product_id: 0x0080,
                serial: Some("A"),
                interface: 1,
                usage_page: 0xff00,
                path: path("1-2:1.9\0"),
            },
        ];
        let paired = |control: usize| {
            display_sibling(&all[control], 1, &all).map(|path| path.into_string().unwrap())
        };
        assert_eq!(paired(2).as_deref(), Some("1-2:1.1"));
        assert_eq!(paired(4).as_deref(), Some("1-4:1.1"));
        // Without serials, the path nearest the control interface wins.
        let unnamed = [
            sibling(None, 1, "1-4:1.1\0"),
            sibling(None, 1, "1-2:1.1\0"),
            sibling(None, 3, "1-2:1.3\0"),
        ];
        assert_eq!(
            display_sibling(&unnamed[2], 1, &unnamed)
                .unwrap()
                .to_str()
                .unwrap(),
            "1-2:1.1"
        );
        assert!(display_sibling(&all[2], 2, &all).is_none());
    }

    #[test]
    fn a_wooting_pairs_its_analog_collection_with_its_lighting_one_by_page() {
        let path = |raw: &'static str| CStr::from_bytes_with_nul(raw.as_bytes()).unwrap();
        let sibling = |serial, usage_page, raw| Sibling {
            vendor_id: 0x31e3,
            product_id: 0x1220,
            serial,
            interface: -1,
            usage_page,
            path: path(raw),
        };
        let all = [
            sibling(Some("W1"), 0x0001, "w1-kbd\0"),
            sibling(Some("W1"), 0xff54, "w1-v1\0"),
            sibling(Some("W1"), 0xff53, "w1-v2\0"),
            sibling(Some("W1"), 0x1337, "w1-rgb\0"),
            sibling(Some("W2"), 0xff53, "w2-v2\0"),
            sibling(Some("W2"), 0xff55, "w2-rgb3\0"),
        ];
        let pages = catalog::Protocol::WootingAnalog
            .paired_collections()
            .unwrap();
        // The V1 analog collection gives way to the V2 one beside it.
        assert!(outranked_control(&all[1], pages.control, &all));
        assert!(!outranked_control(&all[2], pages.control, &all));
        assert!(!outranked_control(&all[4], pages.control, &all));
        let lights = |control: usize| {
            collection_sibling(&all[control], pages.display, &all)
                .map(|(path, page)| (path.into_string().unwrap(), page))
        };
        assert_eq!(lights(2), Some(("w1-rgb".to_owned(), 0x1337)));
        assert_eq!(lights(4), Some(("w2-rgb3".to_owned(), 0xff55)));
        // One HID node listing both collections is not a second one to open.
        let shared = [
            sibling(None, 0xff53, "hidraw3\0"),
            sibling(None, 0x1337, "hidraw3\0"),
        ];
        assert!(collection_sibling(&shared[0], pages.display, &shared).is_none());
    }

    #[test]
    fn a_mystrix_keeps_its_other_vendor_collection_to_probe() {
        let picked = select_candidates([
            candidate("203systems-mystrix", Some("ABC"), "col05", 0, true),
            candidate("203systems-mystrix", Some("ABC"), "col06", 0, true),
            // Linux and macOS list both collections on one path.
            candidate("203systems-mystrix", Some("ABC"), "col06", 0, true),
        ]);
        assert_eq!(picked.len(), 1);
        let deck = &picked["203systems-mystrix-abc"];
        assert_eq!(deck.path.to_str().unwrap(), "col05");
        assert_eq!(deck.alternates, vec![CString::new("col06").unwrap()]);
        // Decks that need no probe never collect alternates.
        let plain = select_candidates([
            candidate("openai-codex-micro", Some("A1"), "one", 2, true),
            candidate("openai-codex-micro", Some("A1"), "two", 2, true),
        ]);
        assert!(plain["openai-codex-micro-a1"].alternates.is_empty());
    }

    #[test]
    fn two_identical_decks_without_serials_both_open() {
        let picked = select_candidates([
            candidate("ulanzi-d200", None, "/dev/hidraw3", 0, true),
            candidate("ulanzi-d200", None, "/dev/hidraw7", 0, true),
        ]);
        assert_eq!(picked.len(), 2);
        assert!(picked.contains_key("ulanzi-d200"));
        let second = picked.keys().find(|id| *id != "ulanzi-d200").unwrap();
        assert!(second.starts_with("ulanzi-d200-p"), "{second}");
        assert!(catalog::model_for_device_id(second).is_some());
    }

    #[test]
    fn identical_serial_less_decks_keep_their_ids_across_scans() {
        let mut sticky = BTreeMap::new();
        let deck = |path| candidate("ulanzi-d200", None, path, 0, true);
        let first =
            select_candidates_sticky([deck("/dev/hidraw7"), deck("/dev/hidraw3")], &mut sticky);
        let bare = first
            .keys()
            .find(|id| id.as_str() == "ulanzi-d200")
            .unwrap();
        assert_eq!(first[bare].path.to_str().unwrap(), "/dev/hidraw3");
        let tagged = first.keys().find(|id| *id != bare).unwrap().clone();
        assert_eq!(first[&tagged].path.to_str().unwrap(), "/dev/hidraw7");
        // The pair listed in the other order: nobody swaps.
        let swapped =
            select_candidates_sticky([deck("/dev/hidraw3"), deck("/dev/hidraw7")], &mut sticky);
        assert_eq!(
            swapped["ulanzi-d200"].path.to_str().unwrap(),
            "/dev/hidraw3"
        );
        assert_eq!(swapped[&tagged].path.to_str().unwrap(), "/dev/hidraw7");
        // The bare-id deck leaves: the other keeps its own id, not the bare one.
        let alone = select_candidates_sticky([deck("/dev/hidraw7")], &mut sticky);
        assert_eq!(alone.len(), 1);
        assert_eq!(alone[&tagged].path.to_str().unwrap(), "/dev/hidraw7");
    }

    #[test]
    fn the_touch_bar_attaches_painted_and_survives_rescans() {
        let store = Arc::new(Store::in_memory().unwrap());
        store
            .put_binding(&Binding {
                device_id: touchbar::MODEL_ID.to_owned(),
                key: 1,
                label: "Ship".to_owned(),
                color: "#16a34a".to_owned(),
                face_png: None,
                action: Action::None,
                updated_at: 0,
            })
            .unwrap();
        let (presses, _inbox) = tokio::sync::mpsc::unbounded_channel();
        let (link, end) = touchbar::link();
        let snapshot = Arc::new(RwLock::new(Snapshot::default()));
        let mut device_loop = DeviceLoop::new(store, presses, Arc::clone(&snapshot), Some(link));
        device_loop.attach_touch_bar();
        device_loop.attach_touch_bar();
        let touchbar::Command::Show(buttons) = end.commands.try_recv().unwrap() else {
            panic!("expected the bound key");
        };
        assert_eq!(buttons.len(), 1);
        assert_eq!((buttons[0].key, buttons[0].label.as_str()), (1, "Ship"));
        assert!(end.commands.try_recv().is_err(), "attached once");
        assert!(snapshot.read().unwrap().devices[touchbar::MODEL_ID].connected);

        // With the main thread gone, the next attempt gives up for good.
        drop(end);
        device_loop.disconnect(touchbar::MODEL_ID);
        device_loop.attach_touch_bar();
        assert!(device_loop.touch_bar.is_none());
        assert!(!device_loop.open.contains_key(touchbar::MODEL_ID));
    }

    #[test]
    fn open_failures_name_the_likely_fix() {
        assert!(open_failure("device is busy").contains("Another app"));
        // The busy message names the apps the newer families are held by.
        let busy = open_failure("resource busy");
        for app in [
            "NIHardwareAgent",
            "serialosc",
            "Wootility",
            "Dynamic Lighting",
            "SteelSeries GG",
        ] {
            assert!(busy.contains(app), "{app}");
        }
        assert!(open_failure("weird").starts_with("Ryu could not open this deck: weird"));
        // A MacroPad on other CircuitPython code is told what to copy, as is.
        assert_eq!(
            open_failure(MACROPAD_FIRMWARE_MISSING),
            MACROPAD_FIRMWARE_MISSING
        );
    }
}
