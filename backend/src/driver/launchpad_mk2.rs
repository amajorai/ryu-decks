//! The Novation Launchpad MK2 (the RGB Launchpad of 2015), driven in its
//! Session layout over class-compliant USB MIDI ([`super::midi`]).
//!
//! It has one MIDI interface, named `Launchpad MK2` (ALSA adds ` MIDI 1`;
//! Windows may put a `2- ` duplicate prefix in front). A name that says Mini
//! is a bi-colour Launchpad Mini MK2 instead ([`super::launchpad_legacy`]).
//!
//! Out, SysEx with Novation's header `F0 00 20 29 02 18`:
//! - `22 00` selects the Session layout, which every open sends again since
//!   the device starts in whatever layout it was left in;
//! - `0E 00` sets every LED to palette colour 0, off;
//! - `0B` then up to 80 `<led> R G B` entries, six bits a channel.
//!
//! LED numbers are always the Session layout's, whatever layout is showing:
//! the pads are `10 * row + column` from the bottom left (11 to 88), the
//! right-hand column 89 down to 19, the round top row 104 to 111.
//!
//! In, on MIDI channel 1 in the Session layout: the pads and the right-hand
//! column are Note On `90 n v` (127 down, 0 up; Note Off is taken as up too),
//! the top row Control Change `B0 cc v` (127 down, 0 up). Fader layouts are
//! never selected, so fader controllers never come; other channels, the
//! text-scroll-done SysEx and everything else is ignored.
//!
//! There is no logo light and no brightness command, so Ryu dims by scaling
//! the colours it sends. When it lets go it turns every LED off.
//!
//! Facts per Novation's Launchpad MK2 Programmer's Reference Guide (vendor
//! documentation), cross-checked against the launchy crate (MIT), Mixxx's
//! Launchpad MK2 mapping (GPL-2.0-or-later), DrivenByMoss (LGPL-3.0) and
//! launchpad.py (CC BY 4.0) for the port name; no code was copied.

use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const MK2_ID: &str = "novation-launchpad-mk2";

/// Novation's SysEx header and the MK2's device byte.
const HEADER: [u8; 6] = [0xf0, 0x00, 0x20, 0x29, 0x02, 0x18];
const SYSEX_END: u8 = 0xf7;
const SET_LAYOUT: u8 = 0x22;
const SESSION_LAYOUT: u8 = 0x00;
const SET_ALL: u8 = 0x0e;
const RGB: u8 = 0x0b;
const NOTE_ON: u8 = 0x90;
const NOTE_OFF: u8 = 0x80;
const CONTROL_CHANGE: u8 = 0xb0;
/// The keys: the 8 x 8 pads, the top row, then the right-hand column.
const KEYS: usize = 80;
const GRID: usize = 64;
const TOP: usize = 72;
const FIRST_TOP_CC: u8 = 104;
/// The most `<led> R G B` entries one RGB message may carry.
const MAX_ENTRIES: usize = 80;
/// Larger than any message the device sends Ryu needs to read.
const READ_LEN: usize = 512;
const MAX_MESSAGES_PER_POLL: usize = 64;
const BLACK: [u8; 3] = [0, 0, 0];

/// Whether a MIDI port, by its normalised name, is a Launchpad MK2's: it
/// names `Launchpad MK2` (in any case, not followed by a letter or digit) and
/// is the device's first and only interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    const NAME: &str = "launchpad mk2";
    if model.id != MK2_ID || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let lower = name.base.to_ascii_lowercase();
    lower.match_indices(NAME).any(|(at, _)| {
        !lower
            .get(at + NAME.len()..)
            .unwrap_or("")
            .starts_with(|next: char| next.is_ascii_alphanumeric())
    })
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

/// The top row left to right, then the right-hand column top to bottom.
const BUTTONS: [&str; KEYS - GRID] = [
    "Up",
    "Down",
    "Left",
    "Right",
    "Session",
    "User 1",
    "User 2",
    "Mixer",
    "Volume",
    "Pan",
    "Send A",
    "Send B",
    "Stop",
    "Mute",
    "Solo",
    "Record Arm",
];

/// The legend on each key, on a Launchpad MK2.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.id == MK2_ID).then(|| PAD_NAMES.iter().chain(&BUTTONS).copied().collect())
}

/// Each key's cell in the 9 x 9 grid: the pads one row down, the top row
/// above them, the right-hand column under the empty top-right corner.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    (model.id == MK2_ID).then(|| {
        let cols = u16::from(model.cols);
        (0..KEYS)
            .map(|index| {
                let (row, col) = cell(index);
                u16::from(row) * cols + u16::from(col)
            })
            .collect()
    })
}

/// A key's (row, column) in the 9 x 9 grid.
fn cell(index: usize) -> (u8, u8) {
    let at = (index % 8) as u8;
    if index < GRID {
        ((index / 8) as u8 + 1, at)
    } else if index < TOP {
        (0, at)
    } else {
        (at + 1, 8)
    }
}

/// A key's LED number, which is also the note or controller it sends.
fn led(index: usize) -> u8 {
    let at = (index % 8) as u8;
    if index < GRID {
        (8 - (index / 8) as u8) * 10 + at + 1
    } else if index < TOP {
        FIRST_TOP_CC + at
    } else {
        89 - 10 * at
    }
}

/// The key a Session-layout note (a pad or a right-hand button) belongs to.
fn key_of_note(note: u8) -> Option<usize> {
    let row = note / 10;
    let col = note % 10;
    if !(1..=8).contains(&row) || !(1..=9).contains(&col) {
        return None;
    }
    let from_top = usize::from(8 - row);
    if col == 9 {
        Some(TOP + from_top)
    } else {
        Some(from_top * 8 + usize::from(col - 1))
    }
}

/// The key a top-row controller belongs to.
fn key_of_controller(number: u8) -> Option<usize> {
    let at = number.checked_sub(FIRST_TOP_CC)?;
    (at < 8).then(|| GRID + usize::from(at))
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

pub struct LaunchpadMk2 {
    transport: Box<dyn Transport>,
    /// In the Session layout with every LED cleared, as far as Ryu knows.
    started: bool,
    /// Each key's colour at full brightness.
    colors: [[u8; 3]; KEYS],
    /// What each LED was last set to (6 bits a channel).
    sent: [Option<[u8; 3]>; KEYS],
    held: [bool; KEYS],
    brightness: u8,
}

impl LaunchpadMk2 {
    #[must_use]
    pub fn new(transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            colors: [BLACK; KEYS],
            sent: [None; KEYS],
            held: [false; KEYS],
            brightness: 100,
        }
    }

    /// Selects the Session layout and turns every LED off.
    fn start(&mut self) -> DriverResult<()> {
        self.transport
            .write(&sysex(&[SET_LAYOUT, SESSION_LAYOUT]))?;
        self.transport.write(&sysex(&[SET_ALL, 0]))?;
        self.sent = [Some(BLACK); KEYS];
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

impl Driver for LaunchpadMk2 {
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
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// One RGB message for every LED whose colour changed; nothing when none
    /// did.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.started {
            self.start()?;
        }
        let changed: Vec<(usize, [u8; 3])> = (0..KEYS)
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

impl Drop for LaunchpadMk2 {
    /// Turns every LED off.
    fn drop(&mut self) {
        if self.started {
            let _ = self.transport.write(&sysex(&[SET_ALL, 0]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, ModelView};

    fn mk2() -> (LaunchpadMk2, Recorder) {
        let recorder = Recorder::default();
        (LaunchpadMk2::new(Box::new(recorder.clone())), recorder)
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

    /// The `(led, r, g, b)` entries of an RGB message.
    fn entries(message: &[u8]) -> Vec<[u8; 4]> {
        assert_eq!(&message[..7], &[0xf0, 0x00, 0x20, 0x29, 0x02, 0x18, 0x0b]);
        assert_eq!(message.last(), Some(&0xf7));
        let body = &message[7..message.len() - 1];
        assert_eq!(body.len() % 4, 0);
        body.chunks(4)
            .map(|entry| [entry[0], entry[1], entry[2], entry[3]])
            .collect()
    }

    #[test]
    fn opening_selects_session_layout_and_clears() {
        let (mut deck, recorder) = mk2();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x18, 0x22, 0x00, 0xf7],
                vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x18, 0x0e, 0x00, 0xf7],
            ]
        );
    }

    #[test]
    fn keys_map_to_session_layout_numbers() {
        assert_eq!((led(0), led(7), led(56), led(63)), (81, 88, 11, 18));
        assert_eq!((led(64), led(71)), (104, 111));
        assert_eq!((led(72), led(79)), (89, 19));
        for index in 0..KEYS {
            let number = led(index);
            let back = if !(GRID..TOP).contains(&index) {
                key_of_note(number)
            } else {
                key_of_controller(number)
            };
            assert_eq!(back, Some(index));
        }
        let numbers: std::collections::HashSet<u8> = (0..KEYS).map(led).collect();
        assert_eq!(numbers.len(), KEYS);
    }

    #[test]
    fn a_full_frame_is_one_message_of_80_entries() {
        let (mut deck, recorder) = mk2();
        deck.flush().unwrap();
        for key in 0..80 {
            deck.set_face(key, Some(&lit([0xff, 0x80, 0x04]))).unwrap();
        }
        deck.set_face(80, Some(&lit([1, 1, 1]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 3);
        assert_eq!(writes[2].len(), 7 + 4 * 80 + 1);
        let leds = entries(&writes[2]);
        assert_eq!(leds.len(), 80);
        assert_eq!(leds[0], [81, 63, 32, 1]);
        assert_eq!(leds[64], [104, 63, 32, 1]);
        assert_eq!(leds[79], [19, 63, 32, 1]);
        assert!(writes[2][1..writes[2].len() - 1]
            .iter()
            .all(|byte| *byte < 0x80));
    }

    #[test]
    fn flush_sends_only_changed_leds_and_brightness_scales() {
        let (mut deck, recorder) = mk2();
        deck.flush().unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes().len(),
            2,
            "black needs nothing after the clear"
        );

        deck.set_face(72, Some(&lit([200, 100, 0]))).unwrap();
        deck.flush().unwrap();
        deck.set_brightness(50).unwrap();
        deck.set_brightness(150).unwrap();
        deck.set_face(72, None).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 6);
        assert_eq!(entries(&writes[2]), vec![[89, 50, 25, 0]]);
        assert_eq!(entries(&writes[3]), vec![[89, 25, 12, 0]]);
        assert_eq!(entries(&writes[4]), vec![[89, 50, 25, 0]]);
        assert_eq!(entries(&writes[5]), vec![[89, 0, 0, 0]]);
    }

    #[test]
    fn notes_and_controllers_press_and_release() {
        let (mut deck, recorder) = mk2();
        recorder.queue_read(vec![0x90, 81, 127]);
        recorder.queue_read(vec![0x90, 81, 0]);
        recorder.queue_read(vec![0x90, 18, 127]);
        recorder.queue_read(vec![0x80, 18, 64]);
        recorder.queue_read(vec![0x90, 89, 127]);
        recorder.queue_read(vec![0x90, 19, 127]);
        recorder.queue_read(vec![0xb0, 104, 127]);
        recorder.queue_read(vec![0xb0, 111, 127]);
        recorder.queue_read(vec![0xb0, 111, 0]);
        // A right-hand button as a controller (the MK3 form), a top button as
        // a note, a fader, another channel, a repeated press, out-of-grid
        // notes, text-scroll done, short and realtime messages.
        recorder.queue_read(vec![0xb0, 89, 127]);
        recorder.queue_read(vec![0x90, 104, 127]);
        recorder.queue_read(vec![0xb0, 0x21, 40]);
        recorder.queue_read(vec![0x91, 81, 127]);
        recorder.queue_read(vec![0x90, 89, 127]);
        recorder.queue_read(vec![0x90, 10, 127]);
        recorder.queue_read(vec![0x90, 90, 127]);
        recorder.queue_read(vec![0x90, 0, 127]);
        recorder.queue_read(vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x18, 0x15, 0xf7]);
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
                press(72),
                press(79),
                press(64),
                press(71),
                release(71),
            ]
        );
    }

    #[test]
    fn letting_go_turns_the_leds_off() {
        let (mut deck, recorder) = mk2();
        deck.set_face(3, Some(&lit([9, 9, 9]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 1);
        assert_eq!(
            writes[before],
            vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x18, 0x0e, 0x00, 0xf7]
        );

        let (deck, recorder) = mk2();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn names_and_cells_fill_the_catalog_grid() {
        let model = by_id(MK2_ID).unwrap();
        let view = ModelView::from(model);
        let names = view.key_names.unwrap();
        let cells = view.key_cells.unwrap();
        assert_eq!((names.len(), cells.len()), (80, 80));
        let unique: std::collections::HashSet<u16> = cells.iter().copied().collect();
        assert_eq!(unique.len(), 80);
        assert!(cells.iter().all(|cell| *cell < 81 && *cell != 8));
        assert_eq!((cells[0], cells[64], cells[72], cells[79]), (9, 0, 17, 80));
        assert_eq!(
            (names[64], names[71], names[79]),
            ("Up", "Mixer", "Record Arm")
        );
    }

    #[test]
    fn port_names() {
        for (raw, windows, mk2) in [
            ("Launchpad MK2:Launchpad MK2 MIDI 1 24:0", false, true),
            ("Launchpad MK2", false, true),
            ("Launchpad MK2", true, true),
            ("3- Launchpad MK2", true, true),
            ("launchpad mk2 In", false, true),
            ("MIDIIN2 (Launchpad MK2)", true, false),
            ("Launchpad Mini MK2", false, false),
            ("Launchpad Mini", false, false),
            ("Launchpad", false, false),
            ("Launchpad S", false, false),
            ("Launchpad MK3", false, false),
            ("Launchpad X:Launchpad X LPX MIDI 24:1", false, false),
            ("Launch Control XL", false, false),
        ] {
            let name = normalise(raw);
            assert_eq!(port_matches(by_id(MK2_ID).unwrap(), &name), mk2, "{raw}");
            let found = identify_midi(&name, windows).map(|model| model.id);
            if mk2 {
                assert_eq!(found, Some(MK2_ID), "{raw}");
            } else {
                assert_ne!(found, Some(MK2_ID), "{raw}");
            }
        }
        // The bi-colour Mini MK2 stays a bi-colour Launchpad.
        assert_eq!(
            identify_midi(&normalise("Launchpad Mini"), false).map(|model| model.id),
            Some(super::super::launchpad_legacy::MINI_ID)
        );
    }
}
