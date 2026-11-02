//! Mirabox and Ajazz decks (AKP153 / HSV293S, AKP815, AKP03 / N3, AKP05 / N4,
//! Stream Dock 293 V3, M18, Stream Dock XL and N1, the Redragon SS550, the
//! Womier DUO87 module and the MagTran M3, and their rebrands).
//!
//! Every command is a `CRT\0\0` packet padded to the generation's packet size
//! (512 bytes on the first generation, 1024 after), led by HID report id 0. A key
//! face is a JPEG sent with `BAT` and committed with `STP`. Input reports start
//! with `ACK`; byte 9 names the control and, from the third generation on, byte 10
//! says whether it went down or up. Earlier generations report presses only.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::{face, Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, KeyImage, Protocol};

const REPORT_LEN: usize = 512;
const READ_BURST: usize = 16;
/// The decks blank their screens when the host goes quiet; `CONNECT` keeps them on.
const KEEP_ALIVE: Duration = Duration::from_secs(10);

/// AKP153 keys in reading order (three rows of six) -> the deck's own numbering,
/// which runs down the columns from the right.
const AKP153_TO_DEVICE: [u8; 18] = [12, 9, 6, 3, 0, 15, 13, 10, 7, 4, 1, 16, 14, 11, 8, 5, 2, 17];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// Eighteen screen keys in a 3 x 6 grid.
    Akp153,
    /// Fifteen screen keys standing upright, five rows of three.
    Akp815,
    /// Six screen keys, three plain buttons and three knobs.
    Akp03,
    /// Ten screen keys over four knobs and a touch strip.
    Akp05,
    /// Fifteen screen keys, 3 x 5, whose faces are numbered from the bottom row.
    Hsv293V3,
    /// The 293 V3's key grid plus three plain buttons.
    M18,
    /// Thirty-two screen keys, 4 x 8, faces numbered from the bottom row, and two
    /// dials.
    StreamDockXl,
    /// Fifteen screen keys (5 x 3), two buttons and a knob, each with a small
    /// screen; it must be switched into host mode first.
    N1,
    /// The 293 V3's grid on first- or second-generation packets that still
    /// report releases (Redragon SS550).
    Ss550,
    /// Twelve keys, faces and presses both in reading order; it wipes its faces
    /// when it leaves its screensaver and says so with code 50 (Womier DUO87).
    Duo87,
    /// The 293 V3's grid plus three dials (MagTran M3).
    M3,
}

/// `SETLB` always carries this many knob light colours.
const KNOB_LIGHTS: usize = 4;

/// The DUO87's "I was reset" report, after its screensaver.
const DUO87_RESET: u8 = 50;

/// What one input report says happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Key(u8),
    /// A tap on a touch-strip segment: reported once, so it reads as a click.
    StripTap(u8),
    DialPress(u8),
    DialTurn(u8, i8),
}

pub struct Mirabox {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    layout: Layout,
    packet: usize,
    /// Third generation: reports carry releases as well as presses.
    both_states: bool,
    /// Knob pushes report their release too (the AKP05 sends presses only).
    dial_releases: bool,
    awake: bool,
    staged: BTreeMap<u8, Vec<u8>>,
    /// Every face last sent, for decks that wipe them and ask to be repainted.
    shown: BTreeMap<u8, Vec<u8>>,
    /// The deck asked for its faces again.
    repaint: bool,
    /// Knob light colours waiting to be sent, one per dial.
    dial_lights: Option<Vec<[u8; 3]>>,
    /// The deck's own lights' colour, waiting to be sent (M18).
    deck_light: Option<[u8; 3]>,
    keys: Vec<bool>,
    dials: Vec<bool>,
    last_keep_alive: Instant,
}

impl Mirabox {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let layout = match model.id {
            "ajazz-akp815" => Layout::Akp815,
            "ajazz-akp03" | "ajazz-akp03-v3" => Layout::Akp03,
            "ajazz-akp05" => Layout::Akp05,
            "mirabox-293-v3" => Layout::Hsv293V3,
            "mirabox-m18" => Layout::M18,
            "mirabox-stream-dock-xl" => Layout::StreamDockXl,
            "mirabox-n1" => Layout::N1,
            "redragon-ss550" | "redragon-ss550-v3" => Layout::Ss550,
            "womier-duo87" => Layout::Duo87,
            "magtran-m3" => Layout::M3,
            _ => Layout::Akp153,
        };
        let both_states = model.protocol == Protocol::MiraboxV3 || layout == Layout::Ss550;
        Self {
            model,
            transport,
            layout,
            packet: if model.protocol == Protocol::MiraboxV1 {
                512
            } else {
                1024
            },
            both_states,
            dial_releases: both_states && layout != Layout::Akp05,
            awake: false,
            staged: BTreeMap::new(),
            shown: BTreeMap::new(),
            repaint: false,
            dial_lights: None,
            deck_light: None,
            keys: vec![false; usize::from(model.keys)],
            dials: vec![false; usize::from(model.dials)],
            last_keep_alive: Instant::now(),
        }
    }

    /// Send one `CRT` command, padded to a full packet.
    fn command(&mut self, body: &[u8]) -> DriverResult<()> {
        let mut packet = Vec::with_capacity(1 + self.packet);
        packet.extend_from_slice(&[0x00, b'C', b'R', b'T', 0x00, 0x00]);
        packet.extend_from_slice(body);
        packet.resize(1 + self.packet, 0);
        self.transport.write(&packet)
    }

    /// Wake the screens before the first command that needs them. The N1 boots
    /// as a numpad and shows host images only in mode 3 (`MOD` '3'), switched
    /// after `DIS` so the wake-up does not undo it.
    fn wake(&mut self) -> DriverResult<()> {
        if !self.awake {
            self.command(b"DIS")?;
            if self.layout == Layout::N1 {
                self.command(b"MOD\x00\x003")?;
            }
            self.awake = true;
            self.last_keep_alive = Instant::now();
        }
        Ok(())
    }

    /// The deck's own slot for a key face (1-based on the wire).
    fn device_slot(&self, key: u8) -> u8 {
        if let Some(segment) = self.model.strip_segment(key) {
            // The AKP05's strip takes slots 1 to 4.
            return segment + 1;
        }
        let from_bottom = |key: u8| {
            let cols = self.model.cols;
            let rows = self.model.screen_keys() / cols;
            (rows - 1 - key / cols) * cols + key % cols + 1
        };
        match self.layout {
            Layout::Hsv293V3 | Layout::Ss550 | Layout::StreamDockXl | Layout::M3 => {
                from_bottom(key)
            }
            Layout::Duo87 => key + 1,
            // The bottom buttons have no screen; only the grid is numbered.
            Layout::M18 if key < self.model.screen_keys() => from_bottom(key),
            Layout::M18 => key + 1,
            // Keys, then buttons A and B, then the knob: slots 1 to 18 in order.
            Layout::N1 => key + 1,
            Layout::Akp153 => AKP153_TO_DEVICE[usize::from(key)] + 1,
            // Numbered from the bottom-right corner, the panel being upside down.
            Layout::Akp815 => self.model.keys - key,
            Layout::Akp03 => key + 1,
            // Slots 1 to 4 are the touch strip; the top row of keys follows the
            // bottom one.
            Layout::Akp05 if key < 5 => key + 11,
            Layout::Akp05 => key + 1,
        }
    }

    /// The geometry for one key. The AKP153's right-hand column is narrower on
    /// the third generation.
    fn format_for(&self, key: u8) -> Option<KeyImage> {
        if self.model.strip_segment(key).is_some() {
            return self.model.strip_image;
        }
        if self.model.wheel_key() == Some(key) {
            return self.model.wheel_image;
        }
        if self.model.extra_has_screen(key) {
            return self.model.extra_image;
        }
        let mut format = self.model.image?;
        if self.layout == Layout::Akp153 && self.both_states && key % 6 == 5 {
            format.width = 82;
            format.height = 82;
        }
        Some(format)
    }

    fn control(&self, code: u8) -> Option<Control> {
        match self.layout {
            Layout::Akp153 => {
                let slot = code.checked_sub(1)?;
                let key = AKP153_TO_DEVICE.iter().position(|device| *device == slot)?;
                Some(Control::Key(u8::try_from(key).ok()?))
            }
            Layout::Akp815 => {
                let key = self.model.keys.checked_sub(code)?;
                (code > 0).then_some(Control::Key(key))
            }
            Layout::Akp03 => match code {
                1..=6 => Some(Control::Key(code - 1)),
                0x25 => Some(Control::Key(6)),
                0x30 => Some(Control::Key(7)),
                0x31 => Some(Control::Key(8)),
                0x33 => Some(Control::DialPress(0)),
                0x35 => Some(Control::DialPress(1)),
                0x34 => Some(Control::DialPress(2)),
                0x90 => Some(Control::DialTurn(0, -1)),
                0x91 => Some(Control::DialTurn(0, 1)),
                0x50 => Some(Control::DialTurn(1, -1)),
                0x51 => Some(Control::DialTurn(1, 1)),
                0x60 => Some(Control::DialTurn(2, -1)),
                0x61 => Some(Control::DialTurn(2, 1)),
                _ => None,
            },
            // Presses arrive in reading order, whatever order the faces take.
            Layout::Hsv293V3 | Layout::Ss550 | Layout::Duo87 | Layout::StreamDockXl => match code {
                1.. if code <= self.model.keys => Some(Control::Key(code - 1)),
                0x21 if self.layout == Layout::StreamDockXl => Some(Control::DialTurn(0, 1)),
                0x23 if self.layout == Layout::StreamDockXl => Some(Control::DialTurn(0, -1)),
                0x24 if self.layout == Layout::StreamDockXl => Some(Control::DialTurn(1, -1)),
                0x26 if self.layout == Layout::StreamDockXl => Some(Control::DialTurn(1, 1)),
                0x22 if self.layout == Layout::StreamDockXl => Some(Control::DialPress(0)),
                0x25 if self.layout == Layout::StreamDockXl => Some(Control::DialPress(1)),
                _ => None,
            },
            Layout::M18 => match code {
                1..=15 => Some(Control::Key(code - 1)),
                0x25 => Some(Control::Key(15)),
                0x30 => Some(Control::Key(16)),
                0x31 => Some(Control::Key(17)),
                _ => None,
            },
            Layout::M3 => match code {
                1..=15 => Some(Control::Key(code - 1)),
                0x35 => Some(Control::DialPress(0)),
                0x33 => Some(Control::DialPress(1)),
                0x37 => Some(Control::DialPress(2)),
                0x50 => Some(Control::DialTurn(0, -1)),
                0x51 => Some(Control::DialTurn(0, 1)),
                0x90 => Some(Control::DialTurn(1, -1)),
                0x91 => Some(Control::DialTurn(1, 1)),
                0xa0 => Some(Control::DialTurn(2, -1)),
                0xa1 => Some(Control::DialTurn(2, 1)),
                _ => None,
            },
            Layout::N1 => match code {
                1..=15 => Some(Control::Key(code - 1)),
                0x1e => Some(Control::Key(15)),
                0x1f => Some(Control::Key(16)),
                0x23 => Some(Control::DialPress(0)),
                0x32 => Some(Control::DialTurn(0, -1)),
                0x33 => Some(Control::DialTurn(0, 1)),
                _ => None,
            },
            // Key presses arrive in reading order, strip taps left to right.
            // Swipes (0x38, 0x39) are not bound: the deck reports them unreliably.
            Layout::Akp05 => match code {
                1..=10 => Some(Control::Key(code - 1)),
                0x40..=0x43 => Some(Control::StripTap(self.model.keys + code - 0x40)),
                0x37 => Some(Control::DialPress(0)),
                0x35 => Some(Control::DialPress(1)),
                0x33 => Some(Control::DialPress(2)),
                0x36 => Some(Control::DialPress(3)),
                0xa0 => Some(Control::DialTurn(0, -1)),
                0xa1 => Some(Control::DialTurn(0, 1)),
                0x50 => Some(Control::DialTurn(1, -1)),
                0x51 => Some(Control::DialTurn(1, 1)),
                0x90 => Some(Control::DialTurn(2, -1)),
                0x91 => Some(Control::DialTurn(2, 1)),
                0x70 => Some(Control::DialTurn(3, -1)),
                0x71 => Some(Control::DialTurn(3, 1)),
                _ => None,
            },
        }
    }

    fn parse(&mut self, report: &[u8]) -> Vec<Input> {
        // Some platforms hand back the (zero) report id ahead of the data.
        let Some(base) = [0, 1]
            .into_iter()
            .find(|base| report.get(*base..*base + 3) == Some(b"ACK".as_slice()))
        else {
            return Vec::new();
        };
        let Some(&code) = report.get(base + 9) else {
            return Vec::new();
        };
        let state = report.get(base + 10).copied().unwrap_or(0);
        if self.layout == Layout::Duo87 && code == DUO87_RESET {
            self.repaint = true;
            return Vec::new();
        }
        let Some(control) = self.control(code) else {
            return Vec::new();
        };
        // The N1's boot handshake sends a knob press with state 2.
        if self.layout == Layout::N1 && matches!(control, Control::DialPress(_)) && state > 1 {
            return Vec::new();
        }
        match control {
            Control::Key(index) => {
                let pressed = &mut self.keys[usize::from(index)];
                change(pressed, self.both_states, state, |pressed| Input::Key {
                    index,
                    pressed,
                })
            }
            Control::DialPress(index) => {
                let pressed = &mut self.dials[usize::from(index)];
                change(pressed, self.dial_releases, state, |pressed| {
                    Input::DialPress { index, pressed }
                })
            }
            // The Stream Dock XL reports how many detents in the state byte.
            Control::DialTurn(index, direction) if self.layout == Layout::StreamDockXl => {
                if state == 0 {
                    return Vec::new();
                }
                let detents = i8::try_from(state).unwrap_or(i8::MAX);
                vec![Input::DialTurn {
                    index,
                    delta: direction.saturating_mul(detents),
                }]
            }
            Control::DialTurn(index, delta) => vec![Input::DialTurn { index, delta }],
            Control::StripTap(index) => vec![
                Input::Key {
                    index,
                    pressed: true,
                },
                Input::Key {
                    index,
                    pressed: false,
                },
            ],
        }
    }
}

/// Turn one report into press/release inputs. Decks that report presses only get
/// a matching release straight away, so every press reads as a full click.
fn change(
    held: &mut bool,
    both_states: bool,
    state: u8,
    input: impl Fn(bool) -> Input,
) -> Vec<Input> {
    if !both_states {
        return vec![input(true), input(false)];
    }
    let pressed = state != 0;
    if *held == pressed {
        return Vec::new();
    }
    *held = pressed;
    vec![input(pressed)]
}

impl Driver for Mirabox {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut report = [0u8; REPORT_LEN];
        let mut wait = timeout_ms;
        for _ in 0..READ_BURST {
            let len = self.transport.read_timeout(&mut report, wait)?;
            if len == 0 {
                break;
            }
            events.extend(self.parse(&report[..len]));
            wait = 0;
        }
        if std::mem::take(&mut self.repaint) {
            // Wake it again and send back every face it lost.
            self.awake = false;
            self.staged = self.shown.clone();
            self.flush()?;
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let has_screen = key < self.model.screen_keys()
            || self.model.strip_segment(key).is_some()
            || self.model.extra_has_screen(key)
            || self.model.wheel_key() == Some(key);
        if !has_screen {
            return Ok(());
        }
        let Some(format) = self.format_for(key) else {
            return Ok(());
        };
        self.staged.insert(key, face::encode(face, &format)?);
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.wake()?;
        if let Some([r, g, b]) = self.deck_light.take() {
            // The same `SETLB`, with one colour per light (24 on the M18).
            let mut body = b"SETLB".to_vec();
            for _ in 0..self.model.deck_lights {
                body.extend_from_slice(&[r, g, b]);
            }
            self.command(&body)?;
        }
        if let Some(lights) = self.dial_lights.take() {
            // `SETLB` takes four colours; fewer dials (the Stream Dock XL's
            // shared side lights) repeat the last one.
            let mut body = b"SETLB".to_vec();
            for index in 0..KNOB_LIGHTS {
                let [r, g, b] = lights
                    .get(index)
                    .or(lights.last())
                    .copied()
                    .unwrap_or_default();
                body.extend_from_slice(&[r, g, b]);
            }
            self.command(&body)?;
        }
        if self.staged.is_empty() {
            return Ok(());
        }
        let staged = std::mem::take(&mut self.staged);
        for (key, jpeg) in staged {
            let len = u16::try_from(jpeg.len()).map_err(|_| {
                super::DriverError::Image("a key face is larger than 64 KB".to_owned())
            })?;
            let [high, low] = len.to_be_bytes();
            let slot = self.device_slot(key);
            self.command(&[b'B', b'A', b'T', 0x00, 0x00, high, low, slot])?;
            for chunk in jpeg.chunks(self.packet) {
                let mut packet = Vec::with_capacity(1 + self.packet);
                packet.push(0x00);
                packet.extend_from_slice(chunk);
                packet.resize(1 + self.packet, 0);
                self.transport.write(&packet)?;
            }
            if self.layout == Layout::Duo87 {
                self.shown.insert(key, jpeg);
            }
        }
        self.command(b"STP")
    }

    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if self.model.dial_lights == crate::catalog::DialLights::None || dial >= self.model.dials {
            return Ok(());
        }
        let dials = usize::from(self.model.dials);
        let lights = self
            .dial_lights
            .get_or_insert_with(|| vec![[0, 0, 0]; dials]);
        lights[usize::from(dial)] = color.unwrap_or([0, 0, 0]);
        Ok(())
    }

    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        if self.model.deck_lights > 0 {
            self.deck_light = Some(color.unwrap_or([0, 0, 0]));
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.wake()?;
        if self.model.dial_lights != crate::catalog::DialLights::None {
            // The knob lights have a brightness of their own.
            self.command(&[b'L', b'B', b'L', b'I', b'G', percent.min(100)])?;
        }
        self.command(&[b'L', b'I', b'G', 0x00, 0x00, percent.min(100)])
    }

    fn heartbeat(&mut self) -> DriverResult<()> {
        if self.last_keep_alive.elapsed() < KEEP_ALIVE {
            return Ok(());
        }
        self.last_keep_alive = Instant::now();
        self.command(b"CONNECT")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;
    use crate::driver::testing::Recorder;

    fn driver(model: &str) -> (Mirabox, Recorder) {
        let recorder = Recorder::default();
        let model = catalog::by_id(model).unwrap();
        (Mirabox::new(model, Box::new(recorder.clone())), recorder)
    }

    fn report(code: u8, state: u8) -> Vec<u8> {
        let mut report = b"ACK\0\0OK\0\0".to_vec();
        report.push(code);
        report.push(state);
        report.resize(REPORT_LEN, 0);
        report
    }

    #[test]
    fn the_akp153_key_order_round_trips() {
        let (deck, _) = driver("ajazz-akp153");
        for key in 0..18u8 {
            let slot = deck.device_slot(key);
            assert_eq!(deck.control(slot), Some(Control::Key(key)), "key {key}");
        }
        // Top-left is the deck's 13th slot; the top-right corner its 16th.
        assert_eq!(deck.device_slot(0), 13);
        assert_eq!(deck.device_slot(5), 16);
    }

    #[test]
    fn first_generation_presses_read_as_clicks() {
        let (mut deck, recorder) = driver("ajazz-akp153");
        recorder.queue_read(report(13, 0));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 0,
                    pressed: true
                },
                Input::Key {
                    index: 0,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn third_generation_reports_press_and_release() {
        let (mut deck, recorder) = driver("ajazz-akp03-v3");
        recorder.queue_read(report(0x25, 1));
        recorder.queue_read(report(0x25, 0));
        recorder.queue_read(report(0x91, 0));
        recorder.queue_read(report(0x35, 1));
        // Not an ACK report: ignored.
        recorder.queue_read(vec![0u8; 16]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 6,
                    pressed: true
                },
                Input::Key {
                    index: 6,
                    pressed: false
                },
                Input::DialTurn { index: 0, delta: 1 },
                Input::DialPress {
                    index: 1,
                    pressed: true
                },
            ]
        );
    }

    #[test]
    fn a_leading_report_id_is_tolerated() {
        let (mut deck, recorder) = driver("ajazz-akp03");
        let mut with_id = vec![0];
        with_id.extend(report(3, 0));
        recorder.queue_read(with_id);
        assert_eq!(
            deck.poll(0).unwrap()[0],
            Input::Key {
                index: 2,
                pressed: true
            }
        );
    }

    #[test]
    fn faces_upload_as_bat_pages_then_commit() {
        let (mut deck, recorder) = driver("ajazz-akp153-v3");
        let face = Face {
            label: String::new(),
            color: [10, 200, 30],
            image: None,
        };
        deck.set_face(0, Some(&face)).unwrap();
        deck.set_face(5, Some(&face)).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert!(writes.iter().all(|packet| packet.len() == 1025));
        assert_eq!(&writes[0][1..9], b"CRT\0\0DIS");
        let bat: Vec<_> = writes
            .iter()
            .filter(|packet| &packet[6..9] == b"BAT")
            .collect();
        assert_eq!(bat.len(), 2);
        // Key 0 is slot 13; key 5 (narrow column) is slot 16.
        assert_eq!(bat[0][13], 13);
        assert_eq!(bat[1][13], 16);
        let first_len = usize::from(u16::from_be_bytes([bat[0][11], bat[0][12]]));
        let page = writes
            .iter()
            .position(|packet| std::ptr::eq(packet, bat[0]))
            .unwrap()
            + 1;
        // The JPEG follows in report-id-led pages.
        assert_eq!(&writes[page][1..3], &[0xff, 0xd8]);
        assert!(first_len > 0);
        assert_eq!(&writes.last().unwrap()[6..9], b"STP");
    }

    #[test]
    fn the_akp815_counts_its_keys_from_the_far_corner() {
        let (mut deck, recorder) = driver("ajazz-akp815");
        for key in 0..15u8 {
            let slot = deck.device_slot(key);
            assert_eq!(deck.control(slot), Some(Control::Key(key)), "key {key}");
        }
        assert_eq!(deck.device_slot(0), 15);
        assert_eq!(deck.device_slot(14), 1);
        assert_eq!(deck.control(0), None);
        assert_eq!(deck.control(16), None);
        // First generation: a press reads as a click.
        recorder.queue_read(report(15, 0));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 0,
                    pressed: true
                },
                Input::Key {
                    index: 0,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn the_akp05_reads_keys_knobs_and_strip_taps() {
        let (mut deck, recorder) = driver("ajazz-akp05");
        recorder.queue_read(report(1, 1));
        recorder.queue_read(report(1, 0));
        // Knob pushes report the press only, even on this generation.
        recorder.queue_read(report(0x36, 1));
        recorder.queue_read(report(0xa0, 0));
        // A swipe is ignored; a tap on the third strip segment is strip key 12.
        recorder.queue_read(report(0x38, 1));
        recorder.queue_read(report(0x42, 1));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 0,
                    pressed: true
                },
                Input::Key {
                    index: 0,
                    pressed: false
                },
                Input::DialPress {
                    index: 3,
                    pressed: true
                },
                Input::DialPress {
                    index: 3,
                    pressed: false
                },
                Input::DialTurn {
                    index: 0,
                    delta: -1
                },
                Input::Key {
                    index: 12,
                    pressed: true
                },
                Input::Key {
                    index: 12,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn akp05_faces_go_around_the_strip_slots() {
        let (mut deck, recorder) = driver("ajazz-akp05");
        assert_eq!(
            (0..10).map(|key| deck.device_slot(key)).collect::<Vec<_>>(),
            vec![11, 12, 13, 14, 15, 6, 7, 8, 9, 10]
        );
        let face = Face {
            label: String::new(),
            color: [10, 200, 30],
            image: None,
        };
        deck.set_face(0, Some(&face)).unwrap();
        // Strip key 11 is the second segment, slot 2.
        deck.set_face(11, Some(&face)).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert!(writes.iter().all(|packet| packet.len() == 1025));
        let slots: Vec<u8> = writes
            .iter()
            .filter(|packet| &packet[6..9] == b"BAT")
            .map(|packet| packet[13])
            .collect();
        assert_eq!(slots, vec![11, 2]);
    }

    fn bat_slots(recorder: &Recorder) -> Vec<u8> {
        recorder
            .writes()
            .iter()
            .filter(|packet| &packet[6..9] == b"BAT")
            .map(|packet| packet[13])
            .collect()
    }

    fn plain_face() -> Face {
        Face {
            label: String::new(),
            color: [1, 2, 3],
            image: None,
        }
    }

    #[test]
    fn the_293_v3_numbers_faces_from_the_bottom_and_presses_from_the_top() {
        let (mut deck, recorder) = driver("mirabox-293-v3");
        // Top-left face is slot 11, bottom-right slot 5.
        deck.set_face(0, Some(&plain_face())).unwrap();
        deck.set_face(14, Some(&plain_face())).unwrap();
        deck.flush().unwrap();
        assert_eq!(bat_slots(&recorder), vec![11, 5]);
        recorder.queue_read(report(1, 1));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 0,
                pressed: true
            }]
        );
    }

    #[test]
    fn the_m18_grid_flips_and_its_buttons_follow() {
        let (mut deck, recorder) = driver("mirabox-m18");
        deck.set_face(5, Some(&plain_face())).unwrap();
        deck.set_face(16, Some(&plain_face())).unwrap();
        deck.flush().unwrap();
        // Middle row stays put; the bottom buttons have no screen.
        assert_eq!(bat_slots(&recorder), vec![6]);
        recorder.queue_read(report(0x30, 1));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 16,
                pressed: true
            }]
        );
    }

    #[test]
    fn the_stream_dock_xl_counts_detents_and_flips_faces() {
        let (mut deck, recorder) = driver("mirabox-stream-dock-xl");
        deck.set_face(0, Some(&plain_face())).unwrap();
        deck.set_face(31, Some(&plain_face())).unwrap();
        deck.flush().unwrap();
        assert_eq!(bat_slots(&recorder), vec![25, 8]);
        recorder.queue_read(report(0x24, 2));
        recorder.queue_read(report(0x21, 0));
        recorder.queue_read(report(0x25, 1));
        recorder.queue_read(report(32, 1));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialTurn {
                    index: 1,
                    delta: -2
                },
                Input::DialPress {
                    index: 1,
                    pressed: true
                },
                Input::Key {
                    index: 31,
                    pressed: true
                },
            ]
        );
    }

    #[test]
    fn the_n1_switches_mode_and_draws_its_small_screens() {
        let (mut deck, recorder) = driver("mirabox-n1");
        deck.set_face(0, Some(&plain_face())).unwrap();
        deck.set_face(15, Some(&plain_face())).unwrap();
        deck.set_face(17, Some(&plain_face())).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(&writes[0][6..9], b"DIS");
        assert_eq!(&writes[1][6..12], b"MOD\x00\x003");
        // Key 1, button A, then the knob.
        assert_eq!(bat_slots(&recorder), vec![1, 16, 18]);
        // The boot handshake's knob press (state 2) is ignored.
        recorder.queue_read(report(0x23, 2));
        recorder.queue_read(report(0x1f, 1));
        recorder.queue_read(report(0x33, 0));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 16,
                    pressed: true
                },
                Input::DialTurn { index: 0, delta: 1 },
            ]
        );
    }

    #[test]
    fn the_ss550_reports_releases_on_first_generation_packets() {
        let (mut deck, recorder) = driver("redragon-ss550");
        recorder.queue_read(report(3, 1));
        recorder.queue_read(report(3, 0));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 2,
                    pressed: true
                },
                Input::Key {
                    index: 2,
                    pressed: false
                },
            ]
        );
        deck.set_face(0, Some(&plain_face())).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().iter().all(|packet| packet.len() == 513));
        assert_eq!(bat_slots(&recorder), vec![11]);
    }

    #[test]
    fn the_duo87_gets_its_faces_back_after_its_screensaver() {
        let (mut deck, recorder) = driver("womier-duo87");
        deck.set_face(4, Some(&plain_face())).unwrap();
        deck.flush().unwrap();
        assert_eq!(bat_slots(&recorder), vec![5]);
        recorder.queue_read(report(DUO87_RESET, 0));
        assert!(deck.poll(0).unwrap().is_empty());
        // Woken again, then the same face to the same slot.
        assert_eq!(bat_slots(&recorder), vec![5, 5]);
        let wakes = recorder
            .writes()
            .iter()
            .filter(|packet| &packet[6..9] == b"DIS")
            .count();
        assert_eq!(wakes, 2);
    }

    #[test]
    fn the_m3_flips_faces_and_reads_three_dials() {
        let (mut deck, recorder) = driver("magtran-m3");
        deck.set_face(14, Some(&plain_face())).unwrap();
        deck.flush().unwrap();
        assert_eq!(bat_slots(&recorder), vec![5]);
        recorder.queue_read(report(0x37, 1));
        recorder.queue_read(report(0x90, 0));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialPress {
                    index: 2,
                    pressed: true
                },
                Input::DialTurn {
                    index: 1,
                    delta: -1
                },
            ]
        );
    }

    #[test]
    fn knob_lights_go_out_as_one_setlb() {
        let (mut deck, recorder) = driver("ajazz-akp05");
        deck.set_dial_light(2, Some([9, 8, 7])).unwrap();
        deck.flush().unwrap();
        let setlb = recorder
            .writes()
            .into_iter()
            .find(|packet| &packet[6..11] == b"SETLB")
            .unwrap();
        assert_eq!(
            &setlb[11..23],
            &[0, 0, 0, 0, 0, 0, 9, 8, 7, 0, 0, 0],
            "four colours, dial 3 lit"
        );
        // The Stream Dock XL's one colour fills all four.
        let (mut xl, recorder) = driver("mirabox-stream-dock-xl");
        xl.set_dial_light(0, Some([1, 2, 3])).unwrap();
        xl.set_dial_light(1, Some([1, 2, 3])).unwrap();
        xl.set_brightness(40).unwrap();
        xl.flush().unwrap();
        let writes = recorder.writes();
        assert!(writes.iter().any(|packet| &packet[6..12] == b"LBLIG\x28"));
        let setlb = writes
            .iter()
            .find(|packet| &packet[6..11] == b"SETLB")
            .unwrap();
        assert_eq!(&setlb[11..23], &[1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2, 3]);
        // Decks without knob lights send none.
        let (mut akp03, recorder) = driver("ajazz-akp03");
        akp03.set_dial_light(0, Some([1, 2, 3])).unwrap();
        akp03.flush().unwrap();
        assert!(recorder
            .writes()
            .iter()
            .all(|packet| &packet[6..11] != b"SETLB"));
    }

    #[test]
    fn the_m18_lights_take_one_colour_for_all_24() {
        let (mut deck, recorder) = driver("mirabox-m18");
        deck.set_deck_light(Some([5, 6, 7])).unwrap();
        deck.flush().unwrap();
        let setlb = recorder
            .writes()
            .into_iter()
            .find(|packet| &packet[6..11] == b"SETLB")
            .unwrap();
        assert_eq!(&setlb[11..14], &[5, 6, 7]);
        assert_eq!(&setlb[11 + 23 * 3..11 + 24 * 3], &[5, 6, 7]);
        assert_eq!(setlb[11 + 24 * 3], 0, "exactly 24 colours");
        // Other decks ignore it.
        let (mut akp153, recorder) = driver("ajazz-akp153");
        akp153.set_deck_light(Some([5, 6, 7])).unwrap();
        akp153.flush().unwrap();
        assert!(recorder
            .writes()
            .iter()
            .all(|packet| &packet[6..11] != b"SETLB"));
    }

    #[test]
    fn plain_buttons_get_no_face() {
        let (mut deck, recorder) = driver("ajazz-akp03");
        deck.set_face(7, None).unwrap();
        assert!(deck.staged.is_empty());
        deck.set_brightness(150).unwrap();
        let writes = recorder.writes();
        assert_eq!(&writes[1][6..12], &[b'L', b'I', b'G', 0, 0, 100]);
    }
}
