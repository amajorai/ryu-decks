//! The Novation Launchpad Pro (the RGB Launchpad Pro of 2015, before the
//! MK3), driven in its Programmer layout over class-compliant USB MIDI
//! ([`super::midi`]).
//!
//! It has two USB MIDI interfaces Ryu could see: the first, the Live port, is
//! Ableton Live's; the second, the Standalone port, is the one Ryu uses. Its
//! names are `Launchpad Pro Standalone Port` on macOS (and on Linux kernels
//! that use the USB jack names), `Launchpad Pro MIDI 2` on older ALSA, and
//! `MIDIIN2 (Launchpad Pro)` / `MIDIOUT2 (Launchpad Pro)` on Windows, where
//! the first interface is plain `Launchpad Pro`. A third port carries the DIN
//! MIDI sockets. A `Launchpad Pro MK3` name is the later model
//! ([`super::launchpad`]), never this one.
//!
//! Out, SysEx with Novation's header `F0 00 20 29 02 10`:
//! - `21 01` selects Standalone mode and `2C 03` the Programmer layout, which
//!   every open sends again since the device keeps whatever it was left in;
//! - `0E 00` sets every LED to palette colour 0, off, and `0A 63 00` the logo
//!   too;
//! - `0B` then up to 48 `<led> R G B` entries, six bits a channel.
//!
//! LED numbers are `10 * row + column` from the bottom left of a 10 x 10
//! square: the pads 11 to 88, the left column 80 down to 10, the right column
//! 89 down to 19, the top row 91 to 98, the bottom row 1 to 8, and the logo
//! 99. The corners other than the logo are no keys; the top-left one is the
//! Setup button, which the firmware keeps.
//!
//! In, on MIDI channel 1 in the Programmer layout: the pads are Note On
//! `90 n v` (any velocity above 0 presses; 0 or Note Off releases) and every
//! round button is Control Change `B0 cc v` (127 down, 0 up). Polyphonic
//! aftertouch the pads send while held, channel pressure, other channels and
//! SysEx are ignored.
//!
//! There is no brightness command, so Ryu dims by scaling the colours it
//! sends. When it lets go it turns every LED and the logo off and selects the
//! Session layout again.
//!
//! Facts from Bitwig's Launchpad Pro extension (bitwig-extensions, MIT),
//! DrivenByMoss (LGPL-3.0) and Ableton Live's Launchpad Pro MIDI remote script
//! (Ableton's, read for facts only); no code was copied. Untested on hardware;
//! the logo's RGB form (`0B 63 R G B`) follows from the shared LED numbering
//! and is not shown verbatim in those references.

use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const PRO_ID: &str = "novation-launchpad-pro";

/// Novation's SysEx header and the Launchpad Pro's device byte.
const HEADER: [u8; 6] = [0xf0, 0x00, 0x20, 0x29, 0x02, 0x10];
const SYSEX_END: u8 = 0xf7;
const SET_MODE: u8 = 0x21;
const STANDALONE_MODE: u8 = 0x01;
const SET_LAYOUT: u8 = 0x2c;
const PROGRAMMER_LAYOUT: u8 = 0x03;
const SESSION_LAYOUT: u8 = 0x00;
const SET_ALL: u8 = 0x0e;
const PALETTE: u8 = 0x0a;
const RGB: u8 = 0x0b;
const NOTE_ON: u8 = 0x90;
const NOTE_OFF: u8 = 0x80;
const CONTROL_CHANGE: u8 = 0xb0;
/// The keys: the 8 x 8 pads, then the top row, the left column, the right
/// column and the bottom row.
const KEYS: usize = 96;
const GRID: usize = 64;
const TOP: usize = 64;
const LEFT: usize = 72;
const RIGHT: usize = 80;
const BOTTOM: usize = 88;
/// The LEDs Ryu sets: every key, then the logo.
const LEDS: usize = KEYS + 1;
const LOGO: usize = KEYS;
const LOGO_LED: u8 = 99;
/// The most `<led> R G B` entries one RGB message carries.
const MAX_ENTRIES: usize = 48;
/// Larger than any message the device sends Ryu needs to read.
const READ_LEN: usize = 512;
const MAX_MESSAGES_PER_POLL: usize = 64;
const BLACK: [u8; 3] = [0, 0, 0];

/// Whether a MIDI port, by its normalised name, is a Launchpad Pro's
/// Standalone port: Windows' interface 2 of a device named `Launchpad Pro`,
/// or a name with `Launchpad Pro Standalone Port` or ending in
/// `Launchpad Pro MIDI 2`. The Live port, the DIN port and every Launchpad
/// Pro MK3 name are refused.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.id != PRO_ID {
        return false;
    }
    let lower = name.base.to_ascii_lowercase();
    match name.interface {
        Some(2) => lower == "launchpad pro",
        Some(_) => false,
        None => {
            lower.contains("launchpad pro standalone port")
                || lower.ends_with("launchpad pro midi 2")
        }
    }
}

const PAD_NAMES: [&str; GRID] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16", "Pad 17", "Pad 18", "Pad 19",
    "Pad 20", "Pad 21", "Pad 22", "Pad 23", "Pad 24", "Pad 25", "Pad 26", "Pad 27", "Pad 28",
    "Pad 29", "Pad 30", "Pad 31", "Pad 32", "Pad 33", "Pad 34", "Pad 35", "Pad 36", "Pad 37",
    "Pad 38", "Pad 39", "Pad 40", "Pad 41", "Pad 42", "Pad 43", "Pad 44", "Pad 45", "Pad 46",
    "Pad 47", "Pad 48", "Pad 49", "Pad 50", "Pad 51", "Pad 52", "Pad 53", "Pad 54", "Pad 55",
    "Pad 56", "Pad 57", "Pad 58", "Pad 59", "Pad 60", "Pad 61", "Pad 62", "Pad 63", "Pad 64",
];

/// The top row left to right, the left and right columns top to bottom, then
/// the bottom row left to right.
const BUTTONS: [&str; KEYS - GRID] = [
    "Up",
    "Down",
    "Left",
    "Right",
    "Session",
    "Note",
    "Device",
    "User",
    "Shift",
    "Click",
    "Undo",
    "Delete",
    "Quantise",
    "Duplicate",
    "Double",
    "Record",
    "Scene 1",
    "Scene 2",
    "Scene 3",
    "Scene 4",
    "Scene 5",
    "Scene 6",
    "Scene 7",
    "Scene 8",
    "Record Arm",
    "Track Select",
    "Mute",
    "Solo",
    "Volume",
    "Pan",
    "Sends",
    "Stop Clip",
];

/// The legend on each key, on a Launchpad Pro.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.id == PRO_ID).then(|| PAD_NAMES.iter().chain(&BUTTONS).copied().collect())
}

/// Each key's cell in the 10 x 10 grid: the pads in the middle, the round
/// buttons around them, the four corners empty.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    (model.id == PRO_ID).then(|| {
        let cols = u16::from(model.cols);
        (0..KEYS)
            .map(|index| {
                let (row, col) = cell(index);
                u16::from(row) * cols + u16::from(col)
            })
            .collect()
    })
}

/// A key's (row, column) in the 10 x 10 grid.
fn cell(index: usize) -> (u8, u8) {
    let at = (index % 8) as u8;
    if index < GRID {
        ((index / 8) as u8 + 1, at + 1)
    } else if index < LEFT {
        (0, at + 1)
    } else if index < RIGHT {
        (at + 1, 0)
    } else if index < BOTTOM {
        (at + 1, 9)
    } else {
        (9, at + 1)
    }
}

/// An LED's number, which for a key is also the note or controller it sends.
fn led(index: usize) -> u8 {
    let at = (index % 8) as u8;
    if index < GRID {
        (8 - (index / 8) as u8) * 10 + at + 1
    } else if index < LEFT {
        91 + at
    } else if index < RIGHT {
        80 - 10 * at
    } else if index < BOTTOM {
        89 - 10 * at
    } else if index < KEYS {
        1 + at
    } else {
        LOGO_LED
    }
}

/// The key a pad's note belongs to.
fn key_of_note(note: u8) -> Option<usize> {
    let row = note / 10;
    let col = note % 10;
    if !(1..=8).contains(&row) || !(1..=8).contains(&col) {
        return None;
    }
    Some(usize::from(8 - row) * 8 + usize::from(col - 1))
}

/// The key a round button's controller belongs to.
fn key_of_controller(number: u8) -> Option<usize> {
    let row = number / 10;
    let col = number % 10;
    match (row, col) {
        (9, 1..=8) => Some(TOP + usize::from(col - 1)),
        (1..=8, 0) => Some(LEFT + usize::from(8 - row)),
        (1..=8, 9) => Some(RIGHT + usize::from(8 - row)),
        (0, 1..=8) => Some(BOTTOM + usize::from(col - 1)),
        _ => None,
    }
}

/// A whole SysEx message: the header, `body`, then `F7`.
fn sysex(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER.len() + body.len() + 1);
    out.extend_from_slice(&HEADER);
    out.extend_from_slice(body);
    out.push(SYSEX_END);
    out
}

/// An 8-bit colour scaled by `percent` and cut to the device's 6 bits.
fn six_bit(color: [u8; 3], percent: u8) -> [u8; 3] {
    color.map(|channel| ((u16::from(channel) * u16::from(percent) / 100) >> 2) as u8)
}

pub struct LaunchpadPro {
    transport: Box<dyn Transport>,
    /// In the Programmer layout with every LED cleared, as far as Ryu knows.
    started: bool,
    /// Each LED's colour at full brightness: the keys, then the logo.
    colors: [[u8; 3]; LEDS],
    /// What each LED was last set to (6 bits a channel).
    sent: [Option<[u8; 3]>; LEDS],
    held: [bool; KEYS],
    brightness: u8,
}

impl LaunchpadPro {
    #[must_use]
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            colors: [BLACK; LEDS],
            sent: [None; LEDS],
            held: [false; KEYS],
            brightness: 100,
        }
    }

    /// Selects Standalone mode and the Programmer layout, then turns every
    /// LED and the logo off. Nothing waits for the device.
    fn start(&mut self) -> DriverResult<()> {
        self.transport.write(&sysex(&[SET_MODE, STANDALONE_MODE]))?;
        self.transport
            .write(&sysex(&[SET_LAYOUT, PROGRAMMER_LAYOUT]))?;
        self.transport.write(&sysex(&[SET_ALL, 0]))?;
        self.transport.write(&sysex(&[PALETTE, LOGO_LED, 0]))?;
        self.sent = [Some(BLACK); LEDS];
        self.started = true;
        Ok(())
    }

    fn receive(&mut self, message: &[u8]) -> Option<Input> {
        let (&status, &number, &value) = (message.first()?, message.get(1)?, message.get(2)?);
        let (key, pressed) = match status {
            NOTE_ON => (key_of_note(number)?, value > 0),
            NOTE_OFF => (key_of_note(number)?, false),
            CONTROL_CHANGE => (key_of_controller(number)?, value > 0),
            _ => return None,
        };
        let held = self.held.get_mut(key)?;
        if *held == pressed {
            return None;
        }
        *held = pressed;
        u8::try_from(key)
            .ok()
            .map(|index| Input::Key { index, pressed })
    }
}

impl Driver for LaunchpadPro {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
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
            events.extend(self.receive(buf.get(..read).unwrap_or_default()));
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let index = usize::from(key);
        if index < KEYS {
            self.colors[index] = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        self.colors[LOGO] = color.unwrap_or(BLACK);
        Ok(())
    }

    /// RGB messages of up to 48 entries for every LED whose colour changed;
    /// nothing when none did.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.started {
            self.start()?;
        }
        let changed: Vec<(usize, [u8; 3])> = (0..LEDS)
            .map(|index| (index, six_bit(self.colors[index], self.brightness)))
            .filter(|(index, color)| self.sent[*index] != Some(*color))
            .collect();
        for chunk in changed.chunks(MAX_ENTRIES) {
            let mut body = Vec::with_capacity(1 + chunk.len() * 4);
            body.push(RGB);
            for (index, [r, g, b]) in chunk {
                body.extend_from_slice(&[led(*index), *r, *g, *b]);
            }
            self.transport.write(&sysex(&body))?;
            for (index, color) in chunk {
                self.sent[*index] = Some(*color);
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }
}

impl Drop for LaunchpadPro {
    /// Turns every LED and the logo off, then selects the Session layout.
    fn drop(&mut self) {
        if self.started {
            let _ = self.transport.write(&sysex(&[SET_ALL, 0]));
            let _ = self.transport.write(&sysex(&[PALETTE, LOGO_LED, 0]));
            let _ = self.transport.write(&sysex(&[SET_LAYOUT, SESSION_LAYOUT]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, ModelView};

    fn pro() -> (LaunchpadPro, Recorder) {
        let recorder = Recorder::default();
        (LaunchpadPro::new(Box::new(recorder.clone())), recorder)
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

    fn message(body: &[u8]) -> Vec<u8> {
        let mut out = vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x10];
        out.extend_from_slice(body);
        out.push(0xf7);
        out
    }

    /// The `(led, r, g, b)` entries of an RGB message.
    fn entries(message: &[u8]) -> Vec<[u8; 4]> {
        assert_eq!(&message[..7], &[0xf0, 0x00, 0x20, 0x29, 0x02, 0x10, 0x0b]);
        assert_eq!(message.last(), Some(&0xf7));
        let body = &message[7..message.len() - 1];
        assert_eq!(body.len() % 4, 0);
        body.chunks(4)
            .map(|entry| [entry[0], entry[1], entry[2], entry[3]])
            .collect()
    }

    #[test]
    fn opening_selects_standalone_programmer_and_clears() {
        let (mut deck, recorder) = pro();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                message(&[0x21, 0x01]),
                message(&[0x2c, 0x03]),
                message(&[0x0e, 0x00]),
                message(&[0x0a, 0x63, 0x00]),
            ]
        );
    }

    #[test]
    fn keys_map_to_programmer_numbers() {
        assert_eq!((led(0), led(7), led(56), led(63)), (81, 88, 11, 18));
        assert_eq!((led(64), led(71)), (91, 98));
        assert_eq!((led(72), led(79)), (80, 10));
        assert_eq!((led(80), led(87)), (89, 19));
        assert_eq!((led(88), led(95)), (1, 8));
        assert_eq!(led(LOGO), 99);
        for index in 0..KEYS {
            let number = led(index);
            let back = if index < GRID {
                key_of_note(number)
            } else {
                key_of_controller(number)
            };
            assert_eq!(back, Some(index));
        }
        let numbers: std::collections::HashSet<u8> = (0..LEDS).map(led).collect();
        assert_eq!(numbers.len(), LEDS);
        for corner in [0, 9, 90, 99] {
            assert_eq!(key_of_note(corner), None);
            assert_eq!(key_of_controller(corner), None);
        }
    }

    #[test]
    fn a_full_frame_is_chunked_at_48_entries() {
        let (mut deck, recorder) = pro();
        deck.flush().unwrap();
        for key in 0..96 {
            deck.set_face(key, Some(&lit([0xff, 0x80, 0x04]))).unwrap();
        }
        deck.set_face(96, Some(&lit([1, 1, 1]))).unwrap();
        deck.set_deck_light(Some([0x40, 0, 0xff])).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 4 + 3);
        let first = entries(&writes[4]);
        let second = entries(&writes[5]);
        let third = entries(&writes[6]);
        assert_eq!((first.len(), second.len(), third.len()), (48, 48, 1));
        assert_eq!(writes[4].len(), 7 + 4 * 48 + 1);
        assert_eq!(first[0], [81, 63, 32, 1]);
        assert_eq!(second[16], [91, 63, 32, 1]);
        assert_eq!(second[47], [8, 63, 32, 1]);
        assert_eq!(third[0], [99, 16, 0, 63]);
        assert!(writes[4..]
            .iter()
            .all(|write| write[1..write.len() - 1].iter().all(|byte| *byte < 0x80)));
    }

    #[test]
    fn flush_sends_only_changed_leds_and_brightness_scales() {
        let (mut deck, recorder) = pro();
        deck.flush().unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes().len(),
            4,
            "black needs nothing after the clear"
        );

        deck.set_face(80, Some(&lit([200, 100, 0]))).unwrap();
        deck.flush().unwrap();
        deck.set_brightness(50).unwrap();
        deck.set_brightness(150).unwrap();
        deck.set_face(80, None).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 8);
        assert_eq!(entries(&writes[4]), vec![[89, 50, 25, 0]]);
        assert_eq!(entries(&writes[5]), vec![[89, 25, 12, 0]]);
        assert_eq!(entries(&writes[6]), vec![[89, 50, 25, 0]]);
        assert_eq!(entries(&writes[7]), vec![[89, 0, 0, 0]]);
    }

    #[test]
    fn notes_and_controllers_press_and_release() {
        let (mut deck, recorder) = pro();
        recorder.queue_read(vec![0x90, 81, 1]);
        recorder.queue_read(vec![0xa0, 81, 90]);
        recorder.queue_read(vec![0xd0, 90]);
        recorder.queue_read(vec![0x90, 81, 0]);
        recorder.queue_read(vec![0x90, 18, 127]);
        recorder.queue_read(vec![0x80, 18, 64]);
        recorder.queue_read(vec![0xb0, 91, 127]);
        recorder.queue_read(vec![0xb0, 91, 0]);
        recorder.queue_read(vec![0xb0, 80, 127]);
        recorder.queue_read(vec![0xb0, 10, 127]);
        recorder.queue_read(vec![0xb0, 89, 127]);
        recorder.queue_read(vec![0xb0, 19, 127]);
        recorder.queue_read(vec![0xb0, 1, 127]);
        recorder.queue_read(vec![0xb0, 8, 127]);
        recorder.queue_read(vec![0xb0, 8, 0]);
        // A button as a note, a pad as a controller, another channel, a
        // repeated press, corners, SysEx, short and realtime messages.
        recorder.queue_read(vec![0x90, 91, 127]);
        recorder.queue_read(vec![0xb0, 81, 127]);
        recorder.queue_read(vec![0x91, 81, 127]);
        recorder.queue_read(vec![0xb1, 91, 127]);
        recorder.queue_read(vec![0xb0, 80, 127]);
        recorder.queue_read(vec![0xb0, 99, 127]);
        recorder.queue_read(vec![0xb0, 0, 127]);
        recorder.queue_read(vec![0xb0, 90, 127]);
        recorder.queue_read(vec![0xb0, 9, 127]);
        recorder.queue_read(vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x10, 0x2d, 0xf7]);
        recorder.queue_read(vec![0x90, 81]);
        recorder.queue_read(vec![0xf8]);
        recorder.queue_read(vec![]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                press(0),
                release(0),
                press(63),
                release(63),
                press(64),
                release(64),
                press(72),
                press(79),
                press(80),
                press(87),
                press(88),
                press(95),
                release(95),
            ]
        );
    }

    #[test]
    fn letting_go_clears_and_returns_to_session() {
        let (mut deck, recorder) = pro();
        deck.set_face(3, Some(&lit([9, 9, 9]))).unwrap();
        deck.set_deck_light(Some([9, 9, 9])).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(
            writes[before..].to_vec(),
            vec![
                message(&[0x0e, 0x00]),
                message(&[0x0a, 0x63, 0x00]),
                message(&[0x2c, 0x00]),
            ]
        );

        let (deck, recorder) = pro();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn names_and_cells_fill_the_catalog_grid() {
        let model = by_id(PRO_ID).unwrap();
        let view = ModelView::from(model);
        let names = view.key_names.unwrap();
        let cells = view.key_cells.unwrap();
        assert_eq!((names.len(), cells.len()), (96, 96));
        let unique: std::collections::HashSet<u16> = cells.iter().copied().collect();
        assert_eq!(unique.len(), 96);
        assert!(cells
            .iter()
            .all(|cell| *cell < 100 && ![0, 9, 90, 99].contains(cell)));
        assert_eq!(
            (cells[0], cells[63], cells[64], cells[72], cells[80], cells[88]),
            (11, 88, 1, 10, 19, 91)
        );
        assert_eq!(
            (names[64], names[72], names[80], names[88], names[95]),
            ("Up", "Shift", "Scene 1", "Record Arm", "Stop Clip")
        );
    }

    #[test]
    fn port_names() {
        for (raw, windows, pro) in [
            ("MIDIIN2 (Launchpad Pro)", true, true),
            ("MIDIOUT2 (Launchpad Pro)", true, true),
            ("MIDIIN2 (2- Launchpad Pro)", true, true),
            ("Launchpad Pro Standalone Port", false, true),
            ("Launchpad Pro:Launchpad Pro MIDI 2 24:1", false, true),
            (
                "Launchpad Pro:Launchpad Pro Standalone Port 24:1",
                false,
                true,
            ),
            ("Launchpad Pro", true, false),
            ("Launchpad Pro", false, false),
            ("Launchpad Pro MIDI 1", false, false),
            ("Launchpad Pro:Launchpad Pro MIDI 1 24:0", false, false),
            ("Launchpad Pro MIDI 3", false, false),
            ("Launchpad Pro Live Port", false, false),
            ("Launchpad Pro MIDI Port", false, false),
            ("MIDIIN3 (Launchpad Pro)", true, false),
            ("Launchpad Pro MK3 LPProMK3 MIDI", false, false),
            (
                "Launchpad Pro MK3:Launchpad Pro MK3 MIDI 2 20:1",
                false,
                false,
            ),
            ("MIDIIN2 (LPProMK3 MIDI)", true, false),
            ("Launchpad MK2", false, false),
        ] {
            let name = normalise(raw);
            assert_eq!(port_matches(by_id(PRO_ID).unwrap(), &name), pro, "{raw}");
            let found = identify_midi(&name, windows).map(|model| model.id);
            if pro {
                assert_eq!(found, Some(PRO_ID), "{raw}");
            } else {
                assert_ne!(found, Some(PRO_ID), "{raw}");
            }
        }
        // The later models keep their own names.
        assert_eq!(
            identify_midi(&normalise("Launchpad MK2"), false).map(|model| model.id),
            Some(super::super::launchpad_mk2::MK2_ID)
        );
        assert_ne!(
            identify_midi(&normalise("Launchpad Pro MK3 LPProMK3 MIDI"), false)
                .map(|model| model.id),
            None
        );
    }
}
