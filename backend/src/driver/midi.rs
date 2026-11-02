//! Class-compliant USB MIDI as a [`Transport`], shared by every MIDI deck (the
//! Novation Launchpads first).
//!
//! A MIDI device has no USB id that midir can see, so a deck is found by the
//! name its host gives the port: each MIDI model names a [`PortRule`], a token
//! from its port name plus, on Windows, which of the device's MIDI interfaces
//! it is driven through. Names are first reduced to what an input and its
//! output share ([`normalise`]): ALSA appends the sequencer's `client:port`
//! numbers (which change on replug), Windows wraps a device's second and later
//! interfaces in `MIDIIN<n> (...)` / `MIDIOUT<n> (...)` and numbers duplicate
//! devices `2- `, and CoreMIDI and Windows may append `In` / `Out`. The ALSA
//! port number is kept apart ([`PortName::alsa_port`]) for decks whose ports
//! differ only by it (the Ableton Push 2).
//!
//! [`Scanner`] keeps one input and one output client alive for listing ports,
//! since every client is an ALSA sequencer client of its own. midir's
//! `connect` consumes its client, so [`open`] makes a fresh pair for each deck,
//! opening the output first. A port another app holds (Windows allows one
//! user; Novation Components or a DAW may hold it) fails to open, which is
//! reported like any deck that would not open and retried on a later scan.
//!
//! One midir input callback is one whole message, with running status already
//! expanded and SysEx reassembled to its `F7`; the callback queues each one, and
//! [`Transport::read_timeout`] hands them out one at a time. Writes are whole
//! messages too. On Linux this needs `libasound.so.2` at run time.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use super::{DriverError, DriverResult, Transport};
use crate::catalog::{self, DeviceModel};

/// The client name Ryu shows other MIDI software.
const CLIENT_NAME: &str = "Ryu Decks";

/// How a MIDI model's ports are found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortRule {
    /// Found in the port name, followed by ` MI` (its `MIDI` port, which ALSA
    /// may cut short): `LPX` matches `LPX MIDI` and never `LPX DAW`.
    pub token: &'static str,
    /// Which of the device's MIDI interfaces (from 1) Windows must list it
    /// as, since Windows names the first interface `<token> MIDI` whatever it
    /// carries: `MIDIIN2 (...)` is interface 2.
    pub windows_interface: u8,
}

/// A port name reduced to what an input and its output share.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PortName {
    pub base: String,
    /// The `n` of a Windows `MIDIIN<n> (...)` / `MIDIOUT<n> (...)` wrapper.
    pub interface: Option<u8>,
    /// The port number of an ALSA `<client>:<port>` suffix (the client
    /// number changes on replug; the port number is the device's own, and
    /// one ALSA port carries both directions). Decks whose ports share a
    /// name tell them apart by it (the Ableton Push 2's User port is 1).
    pub alsa_port: Option<u16>,
}

/// Strips a trailing ALSA ` <client>:<port>`, returning the port number.
fn strip_alsa_numbers(name: &str) -> (&str, Option<u16>) {
    let Some((head, tail)) = name.rsplit_once(' ') else {
        return (name, None);
    };
    let Some((client, port)) = tail.split_once(':') else {
        return (name, None);
    };
    let numbers = !client.is_empty()
        && !port.is_empty()
        && client.bytes().all(|byte| byte.is_ascii_digit())
        && port.bytes().all(|byte| byte.is_ascii_digit());
    match port.parse::<u16>() {
        Ok(port) if numbers => (head, Some(port)),
        _ => (name, None),
    }
}

/// Strips a Windows duplicate-device prefix such as `2- `.
fn strip_duplicate_prefix(name: &str) -> &str {
    let Some((number, rest)) = name.split_once("- ") else {
        return name;
    };
    if !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()) {
        rest
    } else {
        name
    }
}

/// Strips a Windows `MIDIIN<n> (` / `MIDIOUT<n> (` wrapper and its `)`.
/// Windows cuts port names to 31 characters, which may take the `)` (and the
/// end of the name) with it, so a missing `)` is accepted.
fn strip_windows_wrapper(name: &str) -> (&str, Option<u8>) {
    for prefix in ["MIDIIN", "MIDIOUT"] {
        let Some(rest) = name.strip_prefix(prefix) else {
            continue;
        };
        let Some((number, inner)) = rest.split_once(" (") else {
            continue;
        };
        let inner = inner.strip_suffix(')').unwrap_or(inner);
        if let Ok(interface) = number.parse::<u8>() {
            return (inner, Some(interface));
        }
    }
    (name, None)
}

/// The jack names KORG's devices give each direction: the inputs from the
/// deck (the nanoKONTROLs' `SLIDER/KNOB`, the padKONTROL's `PORT A`), then
/// the output to it. Case matters, and the padKONTROL's `PORT B`, `MIDI IN`
/// and `MIDI OUT` are kept, so they stay apart from its Native Mode port.
const KORG_JACKS: [&str; 3] = ["SLIDER/KNOB", "PORT A", "CTRL"];

/// Strips the direction KORG's jack names add (the nanoKONTROL2's input is
/// `... SLIDER/KNOB` and its output `... CTRL`, the padKONTROL's `... PORT
/// A` and `... CTRL`, with ` _` before them in the USB jack strings newer
/// ALSA uses), so the two directions share a name.
/// After ` _ `, whatever is left of one by Windows' 31-character cut goes too.
fn strip_jack_direction(name: &str) -> &str {
    if let Some((head, tail)) = name.rsplit_once(" _ ") {
        if !tail.is_empty() && KORG_JACKS.iter().any(|jack| jack.starts_with(tail)) {
            return head.trim_end();
        }
    }
    KORG_JACKS
        .iter()
        .find_map(|jack| name.strip_suffix(jack)?.strip_suffix(' '))
        .unwrap_or(name)
}

/// Whether two Windows interface names are the same one, one or both cut
/// short at 31 characters (`MIDIOUT<n> (` is a character longer than
/// `MIDIIN<n> (`, so the output's name loses one more).
fn same_cut_short(a: &PortName, b: &PortName) -> bool {
    a.interface.is_some()
        && a.interface == b.interface
        && !a.base.is_empty()
        && !b.base.is_empty()
        && (a.base.starts_with(&b.base) || b.base.starts_with(&a.base))
}

/// A port name as the host lists it, reduced to what both directions of one
/// device interface share.
#[must_use]
pub fn normalise(raw: &str) -> PortName {
    let (name, alsa_port) = strip_alsa_numbers(raw.trim());
    let name = strip_duplicate_prefix(name);
    let (name, interface) = strip_windows_wrapper(name);
    let name = strip_duplicate_prefix(name);
    let name = name
        .strip_suffix(" In")
        .or_else(|| name.strip_suffix(" Out"))
        .unwrap_or(name);
    let name = strip_jack_direction(name);
    PortName {
        base: name.trim().to_owned(),
        interface,
        alsa_port,
    }
}

/// Whether a port is the one `rule` drives. `windows` says the name follows
/// Windows' scheme, where the interface number decides.
#[must_use]
pub fn matches(rule: PortRule, name: &PortName, windows: bool) -> bool {
    if !name.base.contains(&format!("{} MI", rule.token)) {
        return false;
    }
    !windows || name.interface.unwrap_or(1) == rule.windows_interface
}

/// The input and output port names one deck is opened through, exactly as
/// the host lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortPair {
    pub input: String,
    pub output: String,
    /// Which port of that name this is, counting from 0 in the host's list:
    /// hosts (CoreMIDI) list two identical decks under one raw name, so the
    /// name alone would open the first one twice.
    pub input_nth: usize,
    pub output_nth: usize,
}

/// A MIDI deck found among the ports.
#[derive(Debug, Clone)]
pub struct Found {
    pub model: &'static DeviceModel,
    pub name: PortName,
    /// Counts identical decks whose ports normalise to the same name.
    pub ordinal: usize,
    pub ports: PortPair,
}

impl Found {
    /// A key for this deck that survives replugging (ALSA's numbers are not
    /// in it), told apart from an identical deck by its ordinal.
    #[must_use]
    pub fn path(&self) -> String {
        format!(
            "midi:{}#{}#{}",
            self.name.base,
            self.name.interface.unwrap_or(0),
            self.ordinal
        )
    }
}

/// Pairs each deck's input port with its output port, by normalised name, in
/// the order the host lists them.
#[must_use]
pub fn pair_ports(inputs: &[String], outputs: &[String], windows: bool) -> Vec<Found> {
    // Each claimed port with its place among the ports of the same raw name.
    let claim = |list: &[String]| {
        list.iter()
            .enumerate()
            .filter_map(|(at, raw)| {
                let name = normalise(raw);
                let model = catalog::identify_midi(&name, windows)?;
                let nth = list[..at].iter().filter(|earlier| *earlier == raw).count();
                Some((name, model, raw.clone(), nth))
            })
            .collect::<Vec<_>>()
    };
    let mut outs = claim(outputs);
    let mut found: Vec<Found> = Vec::new();
    for (name, model, input, input_nth) in claim(inputs) {
        // ALSA and CoreMIDI list a port both ways under one name, which tells
        // two identical decks apart; Windows names the two directions apart.
        let same_model = |out_model: &&'static DeviceModel| out_model.id == model.id;
        let Some(at) = outs
            .iter()
            .position(|(_, out_model, raw, _)| same_model(out_model) && *raw == input)
            .or_else(|| {
                outs.iter()
                    .position(|(out, out_model, _, _)| same_model(out_model) && *out == name)
            })
            .or_else(|| {
                outs.iter().position(|(out, out_model, _, _)| {
                    windows && same_model(out_model) && same_cut_short(&name, out)
                })
            })
        else {
            continue;
        };
        let (_, _, output, output_nth) = outs.remove(at);
        let ordinal = found
            .iter()
            .filter(|deck| deck.model.id == model.id && deck.name == name)
            .count();
        found.push(Found {
            model,
            name,
            ordinal,
            ports: PortPair {
                input,
                output,
                input_nth,
                output_nth,
            },
        });
    }
    found
}

/// Lists MIDI ports through one long-lived input and output client.
#[derive(Default)]
pub struct Scanner {
    clients: Option<(midir::MidiInput, midir::MidiOutput)>,
    warned: bool,
    /// The port names last listed, so a change in them is logged once: a
    /// deck that is not found can then be matched against what the host
    /// calls it (Windows MIDI Services may rename ports).
    seen: Vec<String>,
}

impl Scanner {
    /// The MIDI decks plugged in now. Empty when the host has no MIDI (no
    /// ALSA sequencer). `libasound.so.2` itself is linked dynamically, so a
    /// host without it never starts the sidecar.
    pub fn scan(&mut self) -> Vec<Found> {
        if self.clients.is_none() {
            let clients = midir::MidiInput::new(CLIENT_NAME).and_then(|input| {
                midir::MidiOutput::new(CLIENT_NAME).map(|output| (input, output))
            });
            match clients {
                Ok(clients) => self.clients = Some(clients),
                Err(error) => {
                    if !self.warned {
                        self.warned = true;
                        tracing::debug!(%error, "ryu-decks: MIDI is unavailable");
                    }
                    return Vec::new();
                }
            }
        }
        let Some((input, output)) = &self.clients else {
            return Vec::new();
        };
        let inputs: Vec<String> = input
            .ports()
            .iter()
            .filter_map(|port| input.port_name(port).ok())
            .collect();
        let outputs: Vec<String> = output
            .ports()
            .iter()
            .filter_map(|port| output.port_name(port).ok())
            .collect();
        let found = pair_ports(&inputs, &outputs, cfg!(windows));
        let names: Vec<String> = inputs.iter().chain(&outputs).cloned().collect();
        if names != self.seen {
            tracing::debug!(
                inputs = ?inputs,
                outputs = ?outputs,
                decks = found.len(),
                "ryu-decks: MIDI ports changed"
            );
            self.seen = names;
        }
        found
    }
}

/// An open MIDI deck: whole messages out, and the messages the input
/// callback queued in.
pub struct MidiLink {
    output: midir::MidiOutputConnection,
    /// Kept open for as long as the link lives; its callback feeds `inbox`.
    _input: midir::MidiInputConnection<()>,
    inbox: Receiver<Vec<u8>>,
}

/// Opens a deck's ports with fresh clients: the output first, then the input
/// with nothing filtered (SysEx included).
pub fn open(ports: &PortPair) -> Result<MidiLink, String> {
    let output = midir::MidiOutput::new(CLIENT_NAME).map_err(|error| error.to_string())?;
    let port = output
        .ports()
        .into_iter()
        .filter(|port| {
            output
                .port_name(port)
                .is_ok_and(|name| name == ports.output)
        })
        .nth(ports.output_nth)
        .ok_or_else(|| "its MIDI output port is gone".to_owned())?;
    let output = output
        .connect(&port, "Ryu Decks out")
        .map_err(|error| format!("its MIDI output is busy or gone ({error})"))?;
    let mut input = midir::MidiInput::new(CLIENT_NAME).map_err(|error| error.to_string())?;
    input.ignore(midir::Ignore::None);
    let port = input
        .ports()
        .into_iter()
        .filter(|port| input.port_name(port).is_ok_and(|name| name == ports.input))
        .nth(ports.input_nth)
        .ok_or_else(|| "its MIDI input port is gone".to_owned())?;
    let (sender, inbox) = mpsc::channel();
    let input = input
        .connect(
            &port,
            "Ryu Decks in",
            move |_stamp, message, ()| {
                let _ = sender.send(message.to_vec());
            },
            (),
        )
        .map_err(|error| format!("its MIDI input is busy or gone ({error})"))?;
    Ok(MidiLink {
        output,
        _input: input,
        inbox,
    })
}

impl Transport for MidiLink {
    fn write(&mut self, data: &[u8]) -> DriverResult<()> {
        self.output
            .send(data)
            .map_err(|error| DriverError::Io(error.to_string()))
    }

    /// One whole MIDI message, cut to `buf`.
    fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> DriverResult<usize> {
        let message = if timeout_ms <= 0 {
            match self.inbox.try_recv() {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return Ok(0),
                Err(TryRecvError::Disconnected) => {
                    return Err(DriverError::Io("the MIDI input closed".to_owned()))
                }
            }
        } else {
            let wait = Duration::from_millis(u64::try_from(timeout_ms).unwrap_or(0));
            match self.inbox.recv_timeout(wait) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => return Ok(0),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(DriverError::Io("the MIDI input closed".to_owned()))
                }
            }
        };
        let len = message.len().min(buf.len());
        buf[..len].copy_from_slice(&message[..len]);
        Ok(len)
    }

    fn send_feature_report(&mut self, _data: &[u8]) -> DriverResult<()> {
        Err(DriverError::Io(
            "a MIDI deck has no feature reports".to_owned(),
        ))
    }

    fn get_feature_report(&mut self, _buf: &mut [u8]) -> DriverResult<usize> {
        Err(DriverError::Io(
            "a MIDI deck has no feature reports".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LPX: PortRule = PortRule {
        token: "LPX",
        windows_interface: 2,
    };
    const PRO: PortRule = PortRule {
        token: "LPProMK3",
        windows_interface: 1,
    };
    const MINI: PortRule = PortRule {
        token: "LPMiniMK3",
        windows_interface: 2,
    };

    fn name(base: &str, interface: Option<u8>) -> PortName {
        PortName {
            base: base.to_owned(),
            interface,
            alsa_port: None,
        }
    }

    fn alsa(base: &str, port: u16) -> PortName {
        PortName {
            base: base.to_owned(),
            interface: None,
            alsa_port: Some(port),
        }
    }

    #[test]
    fn names_lose_what_changes_between_hosts_and_directions() {
        let cases = [
            (
                "Launchpad X:Launchpad X LPX MIDI 24:1",
                alsa("Launchpad X:Launchpad X LPX MIDI", 1),
            ),
            (
                "Ableton Push 2:Ableton Push 2 MIDI 2 24:1",
                alsa("Ableton Push 2:Ableton Push 2 MIDI 2", 1),
            ),
            (
                "Launchpad Mini MK3 LPMiniMK3 MI",
                name("Launchpad Mini MK3 LPMiniMK3 MI", None),
            ),
            ("LPX MIDI", name("LPX MIDI", None)),
            ("LPX MIDI In", name("LPX MIDI", None)),
            ("LPX MIDI Out", name("LPX MIDI", None)),
            ("MIDIIN2 (LPX MIDI)", name("LPX MIDI", Some(2))),
            ("MIDIOUT2 (LPX MIDI)", name("LPX MIDI", Some(2))),
            ("2- MIDIIN2 (LPX MIDI)", name("LPX MIDI", Some(2))),
            ("MIDIIN3 (2- LPProMK3 DAW)", name("LPProMK3 DAW", Some(3))),
            // Not ALSA numbers: kept.
            ("Some Synth 2", name("Some Synth 2", None)),
            ("Weird a:b", name("Weird a:b", None)),
            ("MIDIINx (LPX MIDI)", name("MIDIINx (LPX MIDI)", None)),
            // KORG's jack names lose their direction.
            ("nanoKONTROL2 SLIDER/KNOB", name("nanoKONTROL2", None)),
            ("nanoKONTROL2 CTRL", name("nanoKONTROL2", None)),
            ("nanoKONTROL2 _ CTRL", name("nanoKONTROL2", None)),
            (
                "nanoKONTROL2:nanoKONTROL2 _ CTRL 24:0",
                alsa("nanoKONTROL2:nanoKONTROL2", 0),
            ),
            (
                "nanoKONTROL Studio _ SLIDER/KNO",
                name("nanoKONTROL Studio", None),
            ),
            ("padKONTROL 1 PORT A", name("padKONTROL 1", None)),
            ("padKONTROL 1 CTRL", name("padKONTROL 1", None)),
            (
                "padKONTROL:padKONTROL 1 PORT A 24:1",
                alsa("padKONTROL:padKONTROL 1", 1),
            ),
            // Its other ports keep their names, so they never pair with it.
            ("padKONTROL 1 PORT B", name("padKONTROL 1 PORT B", None)),
            ("padKONTROL _ PORT B", name("padKONTROL _ PORT B", None)),
            ("padKONTROL 1 MIDI IN", name("padKONTROL 1 MIDI IN", None)),
            ("padKONTROL 1 MIDI OUT", name("padKONTROL 1 MIDI OUT", None)),
            ("MPD218 Port A", name("MPD218 Port A", None)),
            // Other decks' names are unchanged by that.
            (
                "Launch Control XL:Launch Control XL MIDI 1 24:0",
                alsa("Launch Control XL:Launch Control XL MIDI 1", 0),
            ),
            ("APC40 mkII", name("APC40 mkII", None)),
            ("MIDIIN2 (APC MINI)", name("APC MINI", Some(2))),
            ("X-TOUCH MINI", name("X-TOUCH MINI", None)),
            ("Some Synth _ Other", name("Some Synth _ Other", None)),
            ("Ctrl CTRLX", name("Ctrl CTRLX", None)),
            // Cut short at 31 characters, `)` and all.
            (
                "MIDIIN2 (Launchkey Mini MK3 MID",
                name("Launchkey Mini MK3 MID", Some(2)),
            ),
        ];
        for (raw, expected) in cases {
            assert_eq!(normalise(raw), expected, "{raw}");
        }
    }

    #[test]
    fn only_the_midi_interface_matches() {
        let alsa_and_mac = [
            ("Launchpad X:Launchpad X LPX MIDI 24:1", LPX, true),
            ("Launchpad X:Launchpad X LPX DAW 24:0", LPX, false),
            ("LPX MIDI", LPX, true),
            ("LPX DAW", LPX, false),
            ("Launchpad Mini MK3 LPMiniMK3 MI", MINI, true),
            (
                "Launchpad Mini MK3:Launchpad Mini MK3 LPMiniMK3 DA 28:0",
                MINI,
                false,
            ),
            ("Launchpad Pro MK3 LPProMK3 MIDI", PRO, true),
            (
                "Launchpad Pro MK3:Launchpad Pro MK3 LPProMK3 DIN 20:1",
                PRO,
                false,
            ),
            (
                "Launchpad Pro MK3:Launchpad Pro MK3 LPProMK3 DAW 20:2",
                PRO,
                false,
            ),
            ("Midi Through:Midi Through Port-0 14:0", LPX, false),
            // Another model's port.
            ("LPProMK3 MIDI", LPX, false),
        ];
        for (raw, rule, expected) in alsa_and_mac {
            assert_eq!(matches(rule, &normalise(raw), false), expected, "{raw}");
        }
        let windows = [
            // The Mini and X list their DAW interface first, so its bare name
            // is "<token> MIDI"; their MIDI interface is the second.
            ("LPX MIDI", LPX, false),
            ("MIDIIN2 (LPX MIDI)", LPX, true),
            ("MIDIOUT2 (LPX MIDI)", LPX, true),
            ("LPMiniMK3 MIDI", MINI, false),
            ("MIDIIN2 (LPMiniMK3 MIDI)", MINI, true),
            // The Pro lists its MIDI interface first.
            ("LPProMK3 MIDI", PRO, true),
            ("MIDIIN2 (LPProMK3 DIN)", PRO, false),
            ("MIDIIN3 (LPProMK3 DAW)", PRO, false),
            ("MIDIIN2 (LPProMK3 MIDI)", PRO, false),
        ];
        for (raw, rule, expected) in windows {
            assert_eq!(matches(rule, &normalise(raw), true), expected, "{raw}");
        }
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn identical_decks_under_one_raw_name_keep_their_own_port_order() {
        // CoreMIDI lists two of the same Launchpad under the same name.
        let names = strings(&["Launchpad X LPX MIDI", "Launchpad X LPX MIDI"]);
        let found = pair_ports(&names, &names, false);
        assert_eq!(found.len(), 2);
        assert_eq!(
            (found[0].ports.input_nth, found[0].ports.output_nth),
            (0, 0)
        );
        assert_eq!(
            (found[1].ports.input_nth, found[1].ports.output_nth),
            (1, 1)
        );
        assert_ne!(found[0].ports, found[1].ports);
    }

    #[test]
    fn inputs_pair_with_their_outputs() {
        let inputs = strings(&[
            "Midi Through:Midi Through Port-0 14:0",
            "Launchpad X:Launchpad X LPX DAW 24:0",
            "Launchpad X:Launchpad X LPX MIDI 24:1",
            "Launchpad X:Launchpad X LPX MIDI 28:1",
            "Launchpad Pro MK3:Launchpad Pro MK3 LPProMK3 MIDI 32:0",
        ]);
        let outputs = strings(&[
            "Launchpad X:Launchpad X LPX MIDI 28:1",
            "Launchpad X:Launchpad X LPX MIDI 24:1",
            "Launchpad X:Launchpad X LPX DAW 24:0",
        ]);
        let found = pair_ports(&inputs, &outputs, false);
        // The Pro has no output listed, so it is not a deck yet.
        assert_eq!(found.len(), 2);
        assert!(found
            .iter()
            .all(|deck| deck.model.id == "novation-launchpad-x"));
        // Two identical decks: each input pairs with its own output.
        assert_eq!(found[0].ports.input, inputs[2]);
        assert_eq!(found[0].ports.output, outputs[1]);
        assert_eq!(found[1].ports.input, inputs[3]);
        assert_eq!(found[1].ports.output, outputs[0]);
        assert_ne!(found[0].path(), found[1].path());
        assert_eq!(found[0].path(), "midi:Launchpad X:Launchpad X LPX MIDI#0#0");

        let windows = pair_ports(
            &strings(&["LPX MIDI", "MIDIIN2 (LPX MIDI)"]),
            &strings(&["LPX MIDI", "MIDIOUT2 (LPX MIDI)"]),
            true,
        );
        assert_eq!(windows.len(), 1);
        assert_eq!(
            windows[0].ports,
            PortPair {
                input: "MIDIIN2 (LPX MIDI)".to_owned(),
                output: "MIDIOUT2 (LPX MIDI)".to_owned(),
                input_nth: 0,
                output_nth: 0,
            }
        );
        // Names cut short at 31 characters still pair.
        let cut = pair_ports(
            &strings(&["Launchkey Mini MK3 MIDI", "MIDIIN2 (Launchkey Mini MK3 MID"]),
            &strings(&["Launchkey Mini MK3 MIDI", "MIDIOUT2 (Launchkey Mini MK3 MI"]),
            true,
        );
        assert_eq!(cut.len(), 1);
        assert_eq!(cut[0].model.id, "novation-launchkey-mini-mk3");
        assert_eq!(cut[0].ports.output, "MIDIOUT2 (Launchkey Mini MK3 MI");
    }
}
