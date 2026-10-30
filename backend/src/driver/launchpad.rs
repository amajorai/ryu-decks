//! Novation Launchpads of the MK3 generation: the Launchpad Mini MK3, Launchpad
//! X and Launchpad Pro MK3, driven in Programmer mode over class-compliant USB
//! MIDI ([`super::midi`]).
//!
//! Each lists two or three MIDI port pairs. Ryu uses only the one named `MIDI`
//! (the second pair on the Mini and X, the first on the Pro): in Programmer
//! mode only it reports presses. The `DAW` port takes colours too but never
//! reports a press, and the Pro's `DIN` port is its 5-pin MIDI passthrough.
//!
//! Out, all SysEx with Novation's header `F0 00 20 29 02 <dev>` (`dev` 0D on the
//! Mini MK3, 0C on the X, 0E on the Pro MK3):
//! - `0E 01` enters Programmer mode and `0E 00` returns to Live mode, which
//!   the device boots in, so every open sends `0E 01` again;
//! - `09 01` wakes the Mini's and X's LEDs from sleep;
//! - `03` then up to 81 (Mini, X) or 106 (Pro) colour specs, each `<type>
//!   <led> <data>`; Ryu sends only type 3, `R G B` at 7 bits a channel.
//!
//! In, on MIDI channel 1: a pad is Note On `90 n v`, pressed while `v` is
//! above zero (velocity on the X and Pro, always 127 on the Mini) and released
//! by `v` = 0 or by Note Off `80 n`; every other button is a Control Change
//! `B0 cc v`, 127 down and 0 up. In Programmer mode a control's number is
//! `10 * row + column` counted from the bottom left of the 8 x 8 grid, so the
//! pads are notes 11 to 88, and the logo is LED 99, which never sends.
//! Aftertouch (`A0`, `D0`), other channels and other SysEx are ignored; a
//! layout or mode readback saying the device left Programmer mode (best
//! effort: the device may report it only on its DAW port) makes Ryu enter it
//! again and repaint.
//!
//! The brightness command the Mini and X take (`08`) changes a stored setting,
//! and the Pro has none, so Ryu dims by scaling the colours it sends on all
//! three and leaves the stored setting alone. LED feedback, velocity and
//! aftertouch settings are never sent either. When Ryu lets go it turns every
//! LED off and returns the device to Live mode.
//!
//! Facts per Novation's Programmer's reference manuals for the three devices,
//! cross-checked against launchpad.js (GPL-3.0), Mixxx's Launchpad Mini MK3
//! mapping (GPL-2.0-or-later) and Ableton Live's Launchpad MIDI remote scripts
//! (for port order, SysEx ids and button legends); no code was copied.

use super::midi::PortRule;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const MINI_MK3_ID: &str = "novation-launchpad-mini-mk3";
pub const X_ID: &str = "novation-launchpad-x";
pub const PRO_MK3_ID: &str = "novation-launchpad-pro-mk3";

/// Novation's SysEx header, before the device byte.
const HEADER: [u8; 5] = [0xf0, 0x00, 0x20, 0x29, 0x02];
const SYSEX_END: u8 = 0xf7;
const LAYOUT: u8 = 0x00;
const LED_LIGHTING: u8 = 0x03;
const LED_SLEEP: u8 = 0x09;
const MODE: u8 = 0x0e;
const LIVE: u8 = 0x00;
const PROGRAMMER: u8 = 0x01;
const RGB_SPEC: u8 = 0x03;
const NOTE_ON: u8 = 0x90;
const NOTE_OFF: u8 = 0x80;
const CONTROL_CHANGE: u8 = 0xb0;
/// The logo's LED, a light Ryu sets as the deck light.
pub const LOGO_LED: u8 = 99;
/// Larger than any message the device sends Ryu needs to read.
const READ_LEN: usize = 512;
const MAX_MESSAGES_PER_POLL: usize = 64;
const BLACK: [u8; 3] = [0, 0, 0];

/// One key: the note or controller number it sends and its LED takes, and its
/// cell in the model's grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Control {
    pub number: u8,
    /// A pad, which sends notes; every other button sends controller changes.
    pub note: bool,
    pub cell: (u8, u8),
}

const UNSET: Control = Control {
    number: 0,
    note: false,
    cell: (0, 0),
};

/// The 8 x 8 pads, top-left first: key `k` is note `(8 - k / 8) * 10 + k % 8 + 1`,
/// one row and `col_offset` columns into the grid.
const fn pad(key: usize, col_offset: u8) -> Control {
    let row = key / 8;
    let col = key % 8;
    Control {
        number: ((8 - row) * 10 + col + 1) as u8,
        note: true,
        cell: (row as u8 + 1, col as u8 + col_offset),
    }
}

/// The Mini MK3's and X's 9 x 9 layout: the pads, the top row (CC 91 to 98),
/// then the right column top to bottom (CC 89 to 19). The logo is the top-right
/// cell.
const fn small_layout() -> [Control; 80] {
    let mut out = [UNSET; 80];
    let mut key = 0;
    while key < 64 {
        out[key] = pad(key, 0);
        key += 1;
    }
    let mut at = 0;
    while at < 8 {
        let at8 = at as u8;
        out[64 + at] = Control {
            number: 91 + at8,
            note: false,
            cell: (0, at8),
        };
        out[72 + at] = Control {
            number: 89 - 10 * at8,
            note: false,
            cell: (1 + at8, 8),
        };
        at += 1;
    }
    out
}

/// The Pro MK3's 11 x 10 layout: the pads; the top row (CC 90 to 98, Shift in
/// the corner); the left column (CC 80 to 10) and right column (CC 89 to 19),
/// top to bottom; then the two rows under the pads (CC 101 to 108, then CC 1 to
/// 8). The logo is the top-right cell; Setup, bottom left, is not reported.
const fn pro_layout() -> [Control; 105] {
    let mut out = [UNSET; 105];
    let mut key = 0;
    while key < 64 {
        out[key] = pad(key, 1);
        key += 1;
    }
    let mut at = 0;
    while at < 9 {
        out[64 + at] = Control {
            number: 90 + at as u8,
            note: false,
            cell: (0, at as u8),
        };
        at += 1;
    }
    let mut at = 0;
    while at < 8 {
        let at8 = at as u8;
        out[73 + at] = Control {
            number: 80 - 10 * at8,
            note: false,
            cell: (1 + at8, 0),
        };
        out[81 + at] = Control {
            number: 89 - 10 * at8,
            note: false,
            cell: (1 + at8, 9),
        };
        out[89 + at] = Control {
            number: 101 + at8,
            note: false,
            cell: (9, 1 + at8),
        };
        out[97 + at] = Control {
            number: 1 + at8,
            note: false,
            cell: (10, 1 + at8),
        };
        at += 1;
    }
    out
}

const SMALL_LAYOUT: [Control; 80] = small_layout();
const PRO_LAYOUT: [Control; 105] = pro_layout();

const PAD_NAMES: [&str; 64] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16", "Pad 17", "Pad 18", "Pad 19",
    "Pad 20", "Pad 21", "Pad 22", "Pad 23", "Pad 24", "Pad 25", "Pad 26", "Pad 27", "Pad 28",
    "Pad 29", "Pad 30", "Pad 31", "Pad 32", "Pad 33", "Pad 34", "Pad 35", "Pad 36", "Pad 37",
    "Pad 38", "Pad 39", "Pad 40", "Pad 41", "Pad 42", "Pad 43", "Pad 44", "Pad 45", "Pad 46",
    "Pad 47", "Pad 48", "Pad 49", "Pad 50", "Pad 51", "Pad 52", "Pad 53", "Pad 54", "Pad 55",
    "Pad 56", "Pad 57", "Pad 58", "Pad 59", "Pad 60", "Pad 61", "Pad 62", "Pad 63", "Pad 64",
];

const SCENES: [&str; 8] = [
    "Scene 1", "Scene 2", "Scene 3", "Scene 4", "Scene 5", "Scene 6", "Scene 7", "Scene 8",
];

const MINI_BUTTONS: [&str; 16] = [
    "Up",
    "Down",
    "Left",
    "Right",
    "Session",
    "Drums",
    "Keys",
    "User",
    "Scene 1",
    "Scene 2",
    "Scene 3",
    "Scene 4",
    "Scene 5",
    "Scene 6",
    "Scene 7",
    "Stop Solo Mute",
];

const X_TOP: [&str; 8] = [
    "Up",
    "Down",
    "Left",
    "Right",
    "Session",
    "Note",
    "Custom",
    "Capture MIDI",
];

const PRO_BUTTONS: [&str; 41] = [
    // Top row, CC 90 to 98.
    "Shift",
    "Left",
    "Right",
    "Session",
    "Note",
    "Chord",
    "Custom",
    "Sequencer",
    "Projects",
    // Left column, CC 80 to 10.
    "Up",
    "Down",
    "Clear",
    "Duplicate",
    "Quantise",
    "Fixed Length",
    "Play",
    "Record",
    // Right column, CC 89 to 19.
    "Scene 1",
    "Scene 2",
    "Scene 3",
    "Scene 4",
    "Scene 5",
    "Scene 6",
    "Scene 7",
    "Scene 8",
    // Under the pads, CC 101 to 108.
    "Track 1",
    "Track 2",
    "Track 3",
    "Track 4",
    "Track 5",
    "Track 6",
    "Track 7",
    "Track 8",
    // Bottom row, CC 1 to 8.
    "Record Arm",
    "Mute",
    "Solo",
    "Volume",
    "Pan",
    "Sends",
    "Device",
    "Stop Clip",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    MiniMk3,
    X,
    ProMk3,
}

impl Kind {
    fn of(model: &DeviceModel) -> Option<Self> {
        match model.id {
            MINI_MK3_ID => Some(Self::MiniMk3),
            X_ID => Some(Self::X),
            PRO_MK3_ID => Some(Self::ProMk3),
            _ => None,
        }
    }

    /// The SysEx device byte after Novation's header.
    const fn device(self) -> u8 {
        match self {
            Self::MiniMk3 => 0x0d,
            Self::X => 0x0c,
            Self::ProMk3 => 0x0e,
        }
    }

    /// The layout number a layout readback names for Programmer mode.
    const fn programmer_layout(self) -> u8 {
        match self {
            Self::MiniMk3 | Self::X => 0x7f,
            Self::ProMk3 => 0x11,
        }
    }

    const fn layout(self) -> &'static [Control] {
        match self {
            Self::MiniMk3 | Self::X => &SMALL_LAYOUT,
            Self::ProMk3 => &PRO_LAYOUT,
        }
    }

    const fn has_led_sleep(self) -> bool {
        matches!(self, Self::MiniMk3 | Self::X)
    }

    const fn port(self) -> PortRule {
        match self {
            Self::MiniMk3 => PortRule {
                token: "LPMiniMK3",
                windows_interface: 2,
            },
            Self::X => PortRule {
                token: "LPX",
                windows_interface: 2,
            },
            Self::ProMk3 => PortRule {
                token: "LPProMK3",
                windows_interface: 1,
            },
        }
    }

    fn names(self) -> Vec<&'static str> {
        let buttons: Vec<&'static str> = match self {
            Self::MiniMk3 => MINI_BUTTONS.to_vec(),
            Self::X => X_TOP.iter().chain(&SCENES).copied().collect(),
            Self::ProMk3 => PRO_BUTTONS.to_vec(),
        };
        PAD_NAMES.iter().copied().chain(buttons).collect()
    }
}

/// How a Launchpad's MIDI port is found.
#[must_use]
pub fn port_rule(model: &DeviceModel) -> Option<PortRule> {
    Kind::of(model).map(Kind::port)
}

/// The legend on each key, on a Launchpad.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    Kind::of(model).map(Kind::names)
}

/// Each key's cell in the model's grid, on a Launchpad.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    Kind::of(model).map(|kind| {
        kind.layout()
            .iter()
            .map(|control| {
                u16::from(control.cell.0) * u16::from(model.cols) + u16::from(control.cell.1)
            })
            .collect()
    })
}

/// A whole SysEx message: the header, the device byte, `body`, then `F7`.
fn sysex(device: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER.len() + body.len() + 2);
    out.extend_from_slice(&HEADER);
    out.push(device);
    out.extend_from_slice(body);
    out.push(SYSEX_END);
    out
}

/// An 8-bit colour scaled by `percent` and cut to the device's 7 bits.
fn seven_bit(color: [u8; 3], percent: u8) -> [u8; 3] {
    color.map(|channel| ((u16::from(channel) * u16::from(percent) / 100) >> 1) as u8)
}

pub struct Launchpad {
    transport: Box<dyn Transport>,
    kind: Kind,
    /// In Programmer mode as far as Ryu knows.
    started: bool,
    /// Each LED's colour at full brightness: the keys in order, then the logo.
    colors: Vec<[u8; 3]>,
    /// What each LED was last sent (7 bits a channel); `None` until it is.
    sent: Vec<Option<[u8; 3]>>,
    held: Vec<bool>,
    brightness: u8,
}

impl Launchpad {
    /// A Launchpad deck. A model that is not a Launchpad is driven as a Mini MK3.
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model).unwrap_or(Kind::MiniMk3);
        let keys = kind.layout().len();
        Self {
            transport,
            kind,
            started: false,
            colors: vec![BLACK; keys + 1],
            sent: vec![None; keys + 1],
            held: vec![false; keys],
            brightness: 100,
        }
    }

    fn led(&self, index: usize) -> u8 {
        self.kind
            .layout()
            .get(index)
            .map_or(LOGO_LED, |control| control.number)
    }

    /// Enters Programmer mode, wakes the LEDs, and forgets what they show.
    fn start(&mut self) -> DriverResult<()> {
        let device = self.kind.device();
        self.transport.write(&sysex(device, &[MODE, PROGRAMMER]))?;
        if self.kind.has_led_sleep() {
            self.transport.write(&sysex(device, &[LED_SLEEP, 0x01]))?;
        }
        self.sent.fill(None);
        self.started = true;
        Ok(())
    }

    fn ensure_started(&mut self) -> DriverResult<()> {
        if self.started {
            Ok(())
        } else {
            self.start()
        }
    }

    /// One LED-lighting message with an RGB spec for each LED in `leds`.
    fn lighting(&self, leds: &[(u8, [u8; 3])]) -> Vec<u8> {
        let mut body = Vec::with_capacity(1 + leds.len() * 5);
        body.push(LED_LIGHTING);
        for (led, [r, g, b]) in leds {
            body.extend_from_slice(&[RGB_SPEC, *led, *r, *g, *b]);
        }
        sysex(self.kind.device(), &body)
    }

    /// The key a note (`note`) or controller number belongs to.
    fn key_of(&self, number: u8, note: bool) -> Option<usize> {
        self.kind
            .layout()
            .iter()
            .position(|control| control.number == number && control.note == note)
    }

    /// Whether a SysEx message says the device has left Programmer mode.
    fn left_programmer_mode(&self, message: &[u8]) -> bool {
        let header_ok = message.starts_with(&HEADER) && message.get(5) == Some(&self.kind.device());
        if !header_ok {
            return false;
        }
        match (message.get(6), message.get(7)) {
            (Some(&LAYOUT), Some(&layout)) => {
                layout != SYSEX_END && layout != self.kind.programmer_layout()
            }
            (Some(&MODE), Some(&mode)) => mode == LIVE,
            _ => false,
        }
    }

    fn receive(&mut self, message: &[u8]) -> DriverResult<Option<Input>> {
        let (Some(&status), Some(&number), Some(&value)) =
            (message.first(), message.get(1), message.get(2))
        else {
            return Ok(None);
        };
        if status == 0xf0 {
            if self.left_programmer_mode(message) {
                self.start()?;
                self.flush()?;
            }
            return Ok(None);
        }
        let (note, pressed) = match status {
            NOTE_ON => (true, value > 0),
            NOTE_OFF => (true, false),
            CONTROL_CHANGE => (false, value > 0),
            _ => return Ok(None),
        };
        let Some(key) = self.key_of(number, note) else {
            return Ok(None);
        };
        let Some(held) = self.held.get_mut(key) else {
            return Ok(None);
        };
        if *held == pressed {
            return Ok(None);
        }
        *held = pressed;
        Ok(u8::try_from(key)
            .ok()
            .map(|index| Input::Key { index, pressed }))
    }
}

impl Driver for Launchpad {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
            self.start()?;
            self.flush()?;
        }
        let mut events = Vec::new();
        let mut buf = [0u8; READ_LEN];
        for attempt in 0..MAX_MESSAGES_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            events.extend(self.receive(&buf[..read])?);
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let index = usize::from(key);
        if index < self.kind.layout().len() {
            self.colors[index] = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        if let Some(logo) = self.colors.last_mut() {
            *logo = color.unwrap_or(BLACK);
        }
        Ok(())
    }

    /// One LED-lighting message for every LED whose colour changed; nothing
    /// when none did.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        let mut changed = Vec::new();
        for index in 0..self.colors.len() {
            let color = seven_bit(self.colors[index], self.brightness);
            if self.sent[index] != Some(color) {
                changed.push((index, color));
            }
        }
        if changed.is_empty() {
            return Ok(());
        }
        let leds: Vec<(u8, [u8; 3])> = changed
            .iter()
            .map(|(index, color)| (self.led(*index), *color))
            .collect();
        self.transport.write(&self.lighting(&leds))?;
        for (index, color) in changed {
            self.sent[index] = Some(color);
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }
}

impl Drop for Launchpad {
    /// Turns every LED off and hands the device back in Live mode.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        let leds: Vec<(u8, [u8; 3])> = (0..self.colors.len())
            .map(|index| (self.led(index), BLACK))
            .collect();
        let _ = self.transport.write(&self.lighting(&leds));
        let _ = self
            .transport
            .write(&sysex(self.kind.device(), &[MODE, LIVE]));
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, ModelView};

    fn launchpad(id: &str) -> (Launchpad, Recorder) {
        let recorder = Recorder::default();
        let deck = Launchpad::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    fn lit(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    fn press(index: u8) -> Input {
        Input::Key {
            index,
            pressed: true,
        }
    }

    fn release(index: u8) -> Input {
        Input::Key {
            index,
            pressed: false,
        }
    }

    /// The `(led, r, g, b)` specs of an LED-lighting message.
    fn specs(message: &[u8], device: u8) -> Vec<[u8; 4]> {
        assert_eq!(&message[..7], &[0xf0, 0x00, 0x20, 0x29, 0x02, device, 0x03]);
        assert_eq!(message.last(), Some(&0xf7));
        let body = &message[7..message.len() - 1];
        assert_eq!(body.len() % 5, 0);
        body.chunks(5)
            .map(|spec| {
                assert_eq!(spec[0], 0x03, "only RGB specs");
                [spec[1], spec[2], spec[3], spec[4]]
            })
            .collect()
    }

    #[test]
    fn each_model_enters_programmer_mode_with_its_device_byte() {
        for (id, device, sleep) in [
            (MINI_MK3_ID, 0x0d, true),
            (X_ID, 0x0c, true),
            (PRO_MK3_ID, 0x0e, false),
        ] {
            let (mut deck, recorder) = launchpad(id);
            deck.flush().unwrap();
            let writes = recorder.writes();
            assert_eq!(
                writes[0],
                vec![0xf0, 0x00, 0x20, 0x29, 0x02, device, 0x0e, 0x01, 0xf7],
                "{id}"
            );
            let mut at = 1;
            if sleep {
                assert_eq!(
                    writes[1],
                    vec![0xf0, 0x00, 0x20, 0x29, 0x02, device, 0x09, 0x01, 0xf7]
                );
                at = 2;
            }
            // Then every LED, black, in one message: no brightness command.
            assert_eq!(writes.len(), at + 1, "{id}");
            let leds = specs(&writes[at], device);
            let expected = if device == 0x0e { 106 } else { 81 };
            assert_eq!(leds.len(), expected);
            assert_eq!(writes[at].len(), 8 + 5 * expected);
            assert!(leds.iter().all(|led| led[1..] == [0, 0, 0]));
            assert_eq!(leds.last().unwrap()[0], LOGO_LED);
        }
    }

    #[test]
    fn keys_map_to_programmer_mode_numbers() {
        assert_eq!(SMALL_LAYOUT[0].number, 81);
        assert_eq!(SMALL_LAYOUT[7].number, 88);
        assert_eq!(SMALL_LAYOUT[63].number, 18);
        assert!(SMALL_LAYOUT[..64].iter().all(|control| control.note));
        assert_eq!((SMALL_LAYOUT[64].number, SMALL_LAYOUT[71].number), (91, 98));
        assert_eq!((SMALL_LAYOUT[72].number, SMALL_LAYOUT[79].number), (89, 19));
        assert_eq!(PRO_LAYOUT[0].number, 81);
        assert_eq!(PRO_LAYOUT[63].number, 18);
        assert_eq!((PRO_LAYOUT[64].number, PRO_LAYOUT[64].note), (90, false));
        assert_eq!(PRO_LAYOUT[72].number, 98);
        assert_eq!((PRO_LAYOUT[73].number, PRO_LAYOUT[80].number), (80, 10));
        assert_eq!((PRO_LAYOUT[81].number, PRO_LAYOUT[88].number), (89, 19));
        assert_eq!((PRO_LAYOUT[89].number, PRO_LAYOUT[96].number), (101, 108));
        assert_eq!((PRO_LAYOUT[97].number, PRO_LAYOUT[104].number), (1, 8));
        // Every number is used once, and never the logo's.
        for layout in [&SMALL_LAYOUT[..], &PRO_LAYOUT[..]] {
            let numbers: std::collections::HashSet<u8> =
                layout.iter().map(|control| control.number).collect();
            assert_eq!(numbers.len(), layout.len());
            assert!(!numbers.contains(&LOGO_LED));
        }
    }

    #[test]
    fn names_and_cells_fill_the_catalog_grid() {
        for id in [MINI_MK3_ID, X_ID, PRO_MK3_ID] {
            let model = by_id(id).unwrap();
            let view = ModelView::from(model);
            let names = view.key_names.unwrap();
            let cells = view.key_cells.unwrap();
            assert_eq!(names.len(), usize::from(model.keys), "{id}");
            assert_eq!(cells.len(), usize::from(model.keys), "{id}");
            let unique: std::collections::HashSet<u16> = cells.iter().copied().collect();
            assert_eq!(unique.len(), cells.len(), "{id}");
            let grid = u16::from(model.rows) * u16::from(model.cols);
            assert!(cells.iter().all(|cell| *cell < grid), "{id}");
        }
        let mini = ModelView::from(by_id(MINI_MK3_ID).unwrap());
        assert_eq!(mini.key_cells.as_ref().unwrap()[0], 9);
        assert_eq!(mini.key_names.as_ref().unwrap()[79], "Stop Solo Mute");
        let x = ModelView::from(by_id(X_ID).unwrap());
        assert_eq!(x.key_names.as_ref().unwrap()[71], "Capture MIDI");
        let pro = ModelView::from(by_id(PRO_MK3_ID).unwrap());
        // Pad 1 sits one column in, past the left column.
        assert_eq!(pro.key_cells.as_ref().unwrap()[0], 11);
        assert_eq!(pro.key_names.as_ref().unwrap()[64], "Shift");
        assert_eq!(pro.key_names.as_ref().unwrap()[104], "Stop Clip");
        // Stop Clip is the last cell of the bottom row but one.
        assert_eq!(pro.key_cells.as_ref().unwrap()[104], 10 * 10 + 8);
    }

    #[test]
    fn notes_and_controllers_press_and_release() {
        let (mut deck, recorder) = launchpad(X_ID);
        recorder.queue_read(vec![0x90, 81, 64]);
        recorder.queue_read(vec![0x90, 81, 0]);
        recorder.queue_read(vec![0x90, 18, 127]);
        recorder.queue_read(vec![0x80, 18, 64]);
        recorder.queue_read(vec![0xb0, 91, 127]);
        recorder.queue_read(vec![0xb0, 91, 0]);
        recorder.queue_read(vec![0xb0, 19, 127]);
        // Aftertouch, another channel, an unknown number, a note sent as a
        // controller, a repeated press, short and realtime messages.
        recorder.queue_read(vec![0xa0, 81, 30]);
        recorder.queue_read(vec![0xd0, 30]);
        recorder.queue_read(vec![0x91, 81, 127]);
        recorder.queue_read(vec![0x90, 19, 127]);
        recorder.queue_read(vec![0xb0, 81, 127]);
        recorder.queue_read(vec![0xb0, 19, 127]);
        recorder.queue_read(vec![0x90, 81]);
        recorder.queue_read(vec![0xf8]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                press(0),
                release(0),
                press(63),
                release(63),
                press(64),
                release(64),
                press(79),
            ]
        );

        let (mut pro, recorder) = launchpad(PRO_MK3_ID);
        recorder.queue_read(vec![0xb0, 90, 127]);
        recorder.queue_read(vec![0xb0, 8, 127]);
        recorder.queue_read(vec![0xb0, 101, 127]);
        recorder.queue_read(vec![0x90, 11, 100]);
        assert_eq!(
            pro.poll(0).unwrap(),
            vec![press(64), press(104), press(89), press(56)]
        );
    }

    #[test]
    fn flush_sends_only_changed_leds_in_one_message() {
        let (mut deck, recorder) = launchpad(MINI_MK3_ID);
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), before, "nothing changed");

        deck.set_face(0, Some(&lit([0xff, 0x80, 0x01]))).unwrap();
        deck.set_face(79, Some(&lit([0x10, 0x20, 0x30]))).unwrap();
        deck.set_face(5, None).unwrap();
        deck.set_face(200, Some(&lit([1, 1, 1]))).unwrap();
        deck.set_deck_light(Some([0, 0xfe, 0])).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 1);
        assert_eq!(
            specs(&writes[before], 0x0d),
            vec![
                [81, 0x7f, 0x40, 0x00],
                [19, 0x08, 0x10, 0x18],
                [99, 0x00, 0x7f, 0x00],
            ]
        );
        assert!(writes[before][1..writes[before].len() - 1]
            .iter()
            .all(|byte| *byte < 0x80));
    }

    #[test]
    fn brightness_scales_the_colours_and_never_touches_the_setting() {
        for (id, device) in [(X_ID, 0x0c), (PRO_MK3_ID, 0x0e)] {
            let (mut deck, recorder) = launchpad(id);
            deck.set_face(0, Some(&lit([200, 100, 0]))).unwrap();
            deck.flush().unwrap();
            let before = recorder.writes().len();
            deck.set_brightness(50).unwrap();
            deck.set_brightness(150).unwrap();
            deck.set_brightness(0).unwrap();
            let writes = recorder.writes();
            assert_eq!(writes.len(), before + 3, "{id}");
            assert_eq!(specs(&writes[before], device), vec![[81, 50, 25, 0]]);
            assert_eq!(specs(&writes[before + 1], device), vec![[81, 100, 50, 0]]);
            assert_eq!(specs(&writes[before + 2], device), vec![[81, 0, 0, 0]]);
            assert!(
                writes.iter().all(|write| write.get(6) != Some(&0x08)),
                "no brightness command"
            );
        }
    }

    #[test]
    fn letting_go_turns_the_leds_off_then_returns_to_live_mode() {
        let (mut deck, recorder) = launchpad(PRO_MK3_ID);
        deck.set_face(3, Some(&lit([9, 9, 9]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 2);
        let leds = specs(&writes[before], 0x0e);
        assert_eq!(leds.len(), 106);
        assert!(leds.iter().all(|led| led[1..] == [0, 0, 0]));
        assert_eq!(
            writes[before + 1],
            vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x0e, 0x0e, 0x00, 0xf7]
        );

        // A deck that never started sends nothing.
        let (deck, recorder) = launchpad(X_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn leaving_programmer_mode_is_undone_and_repainted() {
        let (mut deck, recorder) = launchpad(PRO_MK3_ID);
        deck.set_face(0, Some(&lit([2, 4, 6]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        // The Programmer layout, another device's header and a stray SysEx change nothing.
        recorder.queue_read(vec![
            0xf0, 0x00, 0x20, 0x29, 0x02, 0x0e, 0x00, 0x11, 0x00, 0x00, 0xf7,
        ]);
        recorder.queue_read(vec![
            0xf0, 0x00, 0x20, 0x29, 0x02, 0x0d, 0x00, 0x00, 0x00, 0x00, 0xf7,
        ]);
        recorder.queue_read(vec![0xf0, 0x7e, 0x00, 0x06, 0x02, 0xf7]);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), before);

        // Session layout: Programmer mode again, and every LED.
        recorder.queue_read(vec![
            0xf0, 0x00, 0x20, 0x29, 0x02, 0x0e, 0x00, 0x00, 0x00, 0x00, 0xf7,
        ]);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 2);
        assert_eq!(
            writes[before],
            vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x0e, 0x0e, 0x01, 0xf7]
        );
        let leds = specs(&writes[before + 1], 0x0e);
        assert_eq!(leds.len(), 106);
        assert_eq!(leds[0], [81, 1, 2, 3]);

        // A Live-mode readback on the X does the same.
        let (mut x, recorder) = launchpad(X_ID);
        x.flush().unwrap();
        let before = recorder.writes().len();
        recorder.queue_read(vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x0c, 0x0e, 0x00, 0xf7]);
        x.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 3);
        assert_eq!(&writes[before][6..8], &[0x0e, 0x01]);
        assert_eq!(&writes[before + 1][6..8], &[0x09, 0x01]);
    }
}
