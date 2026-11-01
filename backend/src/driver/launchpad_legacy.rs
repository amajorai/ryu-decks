//! Novation's bi-colour Launchpads: the original Launchpad (MK1), the Launchpad
//! S and the Launchpad Mini (MK1 and MK2, which share a port name and a
//! protocol), over class-compliant USB MIDI ([`super::midi`]).
//!
//! Each lists one MIDI port pair, found by name ([`port_matches`]): every later
//! Launchpad has `Launchpad` in its name too, so the word after it decides
//! (none, `MIDI` or a number for the original, `S`, or `Mini`), and a name
//! naming an MK2, MK3, Pro or X (in any case) is never one of these.
//!
//! Everything is a three-byte message on MIDI channel 1, in the device's
//! default X-Y layout. A grid or scene key is note `16 * row + column`, rows
//! from the top, the scene key being column 8; a top-row key is controller
//! 104 to 111. In: Note On (`90`) with a velocity above zero presses, zero or a
//! Note Off (`80`) releases, and the top row sends `B0 cc` 127 then 0. `B0 00
//! 03` (a text scroll ended, on the S and Mini) and anything else is ignored.
//!
//! Out, a colour byte `(green << 4) | flags | red`, each colour at levels 0 to
//! 3, with the Copy flag (`04`) so both of the device's buffers agree: `90 n v`
//! or `B0 cc v` sets one LED, and `92 v1 v2` sets the next two in rapid-update
//! order (the 64 grid LEDs row by row, the 8 scene LEDs, then the 8 top LEDs),
//! the run ending at the next other message. A whole-deck repaint is 40 rapid
//! messages and a closing `B0 68 v`; fewer changed LEDs go one message each.
//! `B0 00 00` (reset, all LEDs off) is sent on open and when Ryu lets go.
//! There is no master brightness: Ryu scales the levels, and blue is dropped.
//! Ableton Live's handshake messages are never sent.
//!
//! Facts per the launchy crate (MIT), Granjow's launchpad-mini (MIT),
//! FMMT666's launchpad.py (CC BY 4.0), midi-launchpad (BSD), Mixxx's Launchpad
//! mappings (GPL-2.0-or-later) and Ableton Live's Launchpad remote script (for
//! key numbers and legends); no code was copied.

use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const LAUNCHPAD_ID: &str = "novation-launchpad";
pub const S_ID: &str = "novation-launchpad-s";
pub const MINI_ID: &str = "novation-launchpad-mini";

/// Grid, scene and top keys.
pub const KEYS: usize = 80;
const GRID: usize = 64;
const SCENE: usize = 64;
const TOP: usize = 72;
const NOTE_ON: u8 = 0x90;
const NOTE_OFF: u8 = 0x80;
const CONTROL_CHANGE: u8 = 0xb0;
const RAPID: u8 = 0x92;
const FIRST_TOP_CC: u8 = 0x68;
const RESET: [u8; 3] = [CONTROL_CHANGE, 0x00, 0x00];
/// The Copy flag: the LED is written to both buffers.
const COPY: u8 = 0x04;
const BLACK: [u8; 3] = [0, 0, 0];
/// More changed LEDs than this and a rapid repaint (41 messages) is shorter.
const RAPID_ABOVE: usize = 40;
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Launchpad,
    S,
    Mini,
}

impl Kind {
    fn of(model: &DeviceModel) -> Option<Self> {
        match model.id {
            LAUNCHPAD_ID => Some(Self::Launchpad),
            S_ID => Some(Self::S),
            MINI_ID => Some(Self::Mini),
            _ => None,
        }
    }
}

/// Which bi-colour Launchpad a port's name says it is, if any.
fn kind_of_port(base: &str) -> Option<Kind> {
    let lower = base.to_ascii_lowercase();
    if ["mk3", "lpx", "lppro", "lpmini"]
        .iter()
        .any(|word| lower.contains(word))
    {
        return None;
    }
    let newer = ["mk2", "pro"].iter().any(|word| lower.contains(word));
    lower.match_indices("launchpad").find_map(|(at, word)| {
        let rest = lower.get(at + word.len()..)?;
        let next = if rest.is_empty() || rest.starts_with(':') {
            ""
        } else {
            let after = rest.strip_prefix(' ')?;
            after.split([' ', ':']).next().unwrap_or("")
        };
        match next {
            "mini" => Some(Kind::Mini),
            "s" if !newer => Some(Kind::S),
            "" | "midi" if !newer => Some(Kind::Launchpad),
            _ if !newer && !next.is_empty() && next.bytes().all(|byte| byte.is_ascii_digit()) => {
                Some(Kind::Launchpad)
            }
            _ => None,
        }
    })
}

/// Whether a MIDI port, by its normalised name, is this model's. These
/// Launchpads have one MIDI interface, so a Windows name of a later one is not.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    let Some(kind) = Kind::of(model) else {
        return false;
    };
    name.interface.unwrap_or(1) == 1 && kind_of_port(&name.base) == Some(kind)
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

/// The original's and the S's scene keys, top to bottom, then top row.
const LIVE_BUTTONS: [&str; 16] = [
    "Vol", "Pan", "Snd A", "Snd B", "Stop", "Trk On", "Solo", "Arm", "Up", "Down", "Left", "Right",
    "Session", "User 1", "User 2", "Mixer",
];

/// The Mini's scene keys (A to H), then its round top keys (1 to 8).
const MINI_BUTTONS: [&str; 16] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "1", "2", "3", "4", "5", "6", "7", "8",
];

/// The legend on each key, on a bi-colour Launchpad.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    let buttons = match Kind::of(model)? {
        Kind::Launchpad | Kind::S => &LIVE_BUTTONS,
        Kind::Mini => &MINI_BUTTONS,
    };
    Some(PAD_NAMES.iter().chain(buttons).copied().collect())
}

/// Each key's cell in the 9 x 9 grid: the pads one row down, the scene keys
/// in the right column under the empty top-right corner, the top row above.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    Kind::of(model)?;
    let cols = u16::from(model.cols);
    Some(
        (0..KEYS)
            .map(|index| {
                let (row, col) = cell(index);
                u16::from(row) * cols + u16::from(col)
            })
            .collect(),
    )
}

/// A key's (row, column) in the 9 x 9 grid.
fn cell(index: usize) -> (u8, u8) {
    let at = (index % 8) as u8;
    if index < GRID {
        ((index / 8) as u8 + 1, at)
    } else if index < TOP {
        (at + 1, 8)
    } else {
        (0, at)
    }
}

/// The message that sets one key's LED to `value`.
fn led_message(index: usize, value: u8) -> [u8; 3] {
    let at = (index % 8) as u8;
    if index < GRID {
        [NOTE_ON, (index / 8) as u8 * 16 + at, value]
    } else if index < TOP {
        [NOTE_ON, at * 16 + 8, value]
    } else {
        [CONTROL_CHANGE, FIRST_TOP_CC + at, value]
    }
}

/// The key a note (grid or scene) belongs to.
fn key_of_note(note: u8) -> Option<usize> {
    let row = usize::from(note >> 4);
    let col = usize::from(note & 0x0f);
    if row >= 8 {
        None
    } else if col < 8 {
        Some(row * 8 + col)
    } else if col == 8 {
        Some(SCENE + row)
    } else {
        None
    }
}

/// An 8-bit level cut to 0 to 3, then scaled by `percent`. Shared with the
/// first Launchkey Mini ([`super::launchkey_mini`]), whose LEDs take the same
/// levels.
pub(crate) fn level(channel: u8, percent: u8) -> u8 {
    let full = (u16::from(channel) * 3 + 127) / 255;
    ((full * u16::from(percent) + 50) / 100) as u8
}

/// The colour byte for an RGB colour: red and green at four levels, no blue.
fn color_byte(color: [u8; 3], percent: u8) -> u8 {
    let red = level(color[0], percent);
    let green = level(color[1], percent);
    (green << 4) | COPY | red
}

pub struct LaunchpadLegacy {
    transport: Box<dyn Transport>,
    /// Reset as far as Ryu knows.
    started: bool,
    colors: [[u8; 3]; KEYS],
    /// What each LED was last sent; `None` until it is.
    sent: [Option<u8>; KEYS],
    held: [bool; KEYS],
    brightness: u8,
}

impl LaunchpadLegacy {
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

    /// Resets the device (all LEDs off, X-Y layout) and forgets what they show.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if !self.started {
            self.transport.write(&RESET)?;
            self.sent = [None; KEYS];
            self.started = true;
        }
        Ok(())
    }

    fn receive(&mut self, message: &[u8]) -> Option<Input> {
        let (Some(&status), Some(&number), Some(&value)) =
            (message.first(), message.get(1), message.get(2))
        else {
            return None;
        };
        let (key, pressed) = match status {
            NOTE_ON => (key_of_note(number)?, value > 0),
            NOTE_OFF => (key_of_note(number)?, false),
            CONTROL_CHANGE => {
                let at = number.checked_sub(FIRST_TOP_CC).filter(|at| *at < 8)?;
                (TOP + usize::from(at), value > 0)
            }
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

impl Driver for LaunchpadLegacy {
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

    /// After a reset or when more than 40 LEDs changed, every LED in 40 rapid
    /// messages and a closing top-row one; otherwise one message per changed
    /// LED, and nothing when none did.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        let bytes: [u8; KEYS] = self.colors.map(|color| color_byte(color, self.brightness));
        let changed: Vec<usize> = (0..KEYS)
            .filter(|index| self.sent[*index] != Some(bytes[*index]))
            .collect();
        if changed.is_empty() {
            return Ok(());
        }
        let repaint = self.sent.iter().any(Option::is_none) || changed.len() > RAPID_ABOVE;
        if repaint {
            for pair in bytes.chunks_exact(2) {
                self.transport.write(&[RAPID, pair[0], pair[1]])?;
            }
            // Any other message ends the run; this one repeats the first top LED.
            self.transport.write(&led_message(TOP, bytes[TOP]))?;
            self.sent = bytes.map(Some);
        } else {
            for index in changed {
                self.transport.write(&led_message(index, bytes[index]))?;
                self.sent[index] = Some(bytes[index]);
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }
}

impl Drop for LaunchpadLegacy {
    /// Resets the device, which turns every LED off.
    fn drop(&mut self) {
        if self.started {
            let _ = self.transport.write(&RESET);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, ModelView};

    fn fresh() -> (LaunchpadLegacy, Recorder) {
        let recorder = Recorder::default();
        let deck = LaunchpadLegacy::new(Box::new(recorder.clone()));
        (deck, recorder)
    }

    fn lit(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    #[test]
    fn opening_resets_then_paints_every_led_in_rapid_order() {
        let (mut deck, recorder) = fresh();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(1, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(64, Some(&lit([255, 255, 0]))).unwrap();
        deck.set_face(72, Some(&lit([170, 255, 0]))).unwrap();
        deck.set_face(79, Some(&lit([255, 170, 0]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[0], vec![0xb0, 0x00, 0x00]);
        assert_eq!(writes.len(), 1 + 40 + 1);
        assert!(writes[1..41]
            .iter()
            .all(|write| write.len() == 3 && write[0] == 0x92));
        assert_eq!(writes[1], vec![0x92, 0x07, 0x34]);
        assert_eq!(writes[2], vec![0x92, 0x04, 0x04]);
        // Scene 1 is the 65th value; top 1 the 73rd, top 8 the 80th.
        assert_eq!(writes[33], vec![0x92, 0x37, 0x04]);
        assert_eq!(writes[37], vec![0x92, 0x36, 0x04]);
        assert_eq!(writes[40], vec![0x92, 0x04, 0x27]);
        assert_eq!(writes[41], vec![0xb0, 0x68, 0x36]);
        // Nothing changed: nothing sent.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 42);
    }

    #[test]
    fn a_few_changes_go_one_message_each() {
        let (mut deck, recorder) = fresh();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_face(9, Some(&lit([85, 0, 255]))).unwrap();
        deck.set_face(65, Some(&lit([0, 90, 0]))).unwrap();
        deck.set_face(79, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(200, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[before..],
            [
                vec![0x90, 0x11, 0x05],
                vec![0x90, 0x18, 0x14],
                vec![0xb0, 0x6f, 0x07],
            ]
        );
        // 40 changes still go one by one; 41 repaint the deck.
        let (mut deck, recorder) = fresh();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        for index in 0..40 {
            deck.set_face(index, Some(&lit([255, 0, 0]))).unwrap();
        }
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), before + 40);
        assert!(recorder.writes()[before..].iter().all(|w| w[0] != 0x92));
        let before = recorder.writes().len();
        for index in 0..80 {
            deck.set_face(index, Some(&lit([0, 255, 0]))).unwrap();
        }
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 41);
        assert!(writes[before..before + 40]
            .iter()
            .all(|write| *write == vec![0x92, 0x34, 0x34]));
        assert_eq!(writes[before + 40], vec![0xb0, 0x68, 0x34]);
    }

    #[test]
    fn brightness_scales_the_levels() {
        let (mut deck, recorder) = fresh();
        deck.set_face(0, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_brightness(50).unwrap();
        deck.set_brightness(0).unwrap();
        deck.set_brightness(200).unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [
                vec![0x90, 0x00, 0x26],
                vec![0x90, 0x00, 0x04],
                vec![0x90, 0x00, 0x37],
            ]
        );
    }

    #[test]
    fn presses_and_releases_map_to_keys() {
        let (mut deck, recorder) = fresh();
        for message in [
            vec![0x90, 0x00, 0x7f],
            vec![0x90, 0x00, 0x00],
            vec![0x90, 0x77, 0x7f],
            vec![0x80, 0x77, 0x40],
            vec![0x90, 0x18, 0x7f],
            vec![0x90, 0x78, 0x7f],
            vec![0xb0, 0x6f, 0x7f],
            vec![0xb0, 0x6f, 0x00],
            vec![0xb0, 0x68, 0x7f],
            // Column 9, row 8, text ended, an unknown controller, another
            // channel, a repeated press, short, SysEx and realtime messages.
            vec![0x90, 0x09, 0x7f],
            vec![0x90, 0x80, 0x7f],
            vec![0xb0, 0x00, 0x03],
            vec![0xb0, 0x70, 0x7f],
            vec![0x91, 0x00, 0x7f],
            vec![0x90, 0x18, 0x7f],
            vec![0x90, 0x01],
            vec![0xf0, 0x7e, 0x00, 0x06, 0x02, 0xf7],
            vec![0xf8],
        ] {
            recorder.queue_read(message);
        }
        let events = deck.poll(0).unwrap();
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(0, false),
                key(63, true),
                key(63, false),
                key(65, true),
                key(71, true),
                key(79, true),
                key(79, false),
                key(72, true),
            ]
        );
    }

    #[test]
    fn letting_go_resets_the_device() {
        let (mut deck, recorder) = fresh();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 1);
        assert_eq!(writes[before], vec![0xb0, 0x00, 0x00]);
        let (deck, recorder) = fresh();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn ports_are_told_apart_by_the_word_after_launchpad() {
        let cases = [
            ("Launchpad", Some(LAUNCHPAD_ID)),
            ("Launchpad:Launchpad MIDI 1 24:0", Some(LAUNCHPAD_ID)),
            ("2- Launchpad", Some(LAUNCHPAD_ID)),
            ("Launchpad S", Some(S_ID)),
            ("Launchpad S:Launchpad S MIDI 1 20:0", Some(S_ID)),
            ("Launchpad Mini", Some(MINI_ID)),
            ("2- Launchpad Mini", Some(MINI_ID)),
            ("Launchpad Mini:Launchpad Mini MIDI 1 28:0", Some(MINI_ID)),
            ("launchpad mini", Some(MINI_ID)),
            // Later Launchpads, in any case, and other names.
            ("Launchpad MK2", None),
            ("Launchpad Mk2:Launchpad Mk2 MIDI 1 24:0", None),
            ("Launchpad Mini MK3 LPMiniMK3 MIDI", None),
            ("Launchpad Mini Mk3:Launchpad Mini Mk3 MIDI 1 24:0", None),
            ("MIDIIN2 (LPMiniMK3 MIDI)", None),
            ("Launchpad X:Launchpad X LPX MIDI 24:1", None),
            ("Launchpad X MIDI 1", None),
            ("Launchpad Pro", None),
            ("Launchpad Pro MK3 LPProMK3 MIDI", None),
            ("Launchpad Settings", None),
            ("Launchpads", None),
            ("Launch Control", None),
            ("MIDIIN2 (Launchpad)", None),
        ];
        for (raw, expected) in cases {
            let name = normalise(raw);
            let found = [LAUNCHPAD_ID, S_ID, MINI_ID]
                .into_iter()
                .find(|id| port_matches(by_id(id).unwrap(), &name));
            assert_eq!(found, expected, "{raw}");
        }
        // The catalog finds them, and still finds a Mini MK3 as itself.
        assert_eq!(
            identify_midi(&normalise("Launchpad Mini"), true).map(|model| model.id),
            Some(MINI_ID)
        );
        assert_eq!(
            identify_midi(&normalise("Launchpad Mini MK3 LPMiniMK3 MIDI"), false)
                .map(|model| model.id),
            Some(super::super::launchpad::MINI_MK3_ID)
        );
    }

    #[test]
    fn rows_name_and_place_every_key() {
        for id in [LAUNCHPAD_ID, S_ID, MINI_ID] {
            let model = by_id(id).unwrap();
            assert!(model.protocol.is_midi(), "{id}");
            let view = ModelView::from(model);
            assert!(view.brightness && view.key_lights, "{id}");
            assert_eq!(view.key_light_colors, "red_green");
            assert_eq!(view.deck_lights, 0);
            let names = view.key_names.unwrap();
            let cells = view.key_cells.unwrap();
            assert_eq!(names.len(), usize::from(model.keys));
            assert_eq!(cells.len(), usize::from(model.keys));
            let unique: std::collections::HashSet<u16> = cells.iter().copied().collect();
            assert_eq!(unique.len(), KEYS);
            assert!(!unique.contains(&8), "the top-right corner is empty");
            assert!(cells.iter().all(|cell| *cell < 81));
            assert_eq!(
                (cells[0], cells[63], cells[64], cells[71], cells[72]),
                (9, 79, 17, 80, 0)
            );
        }
        let original = ModelView::from(by_id(LAUNCHPAD_ID).unwrap());
        let names = original.key_names.unwrap();
        assert_eq!(
            (names[64], names[71], names[72], names[79]),
            ("Vol", "Arm", "Up", "Mixer")
        );
        let mini = ModelView::from(by_id(MINI_ID).unwrap());
        let names = mini.key_names.unwrap();
        assert_eq!((names[64], names[72]), ("A", "1"));
    }

    #[test]
    fn every_key_has_its_own_message() {
        let numbers: std::collections::HashSet<(u8, u8)> = (0..KEYS)
            .map(|index| {
                let [status, number, _] = led_message(index, 0);
                (status, number)
            })
            .collect();
        assert_eq!(numbers.len(), KEYS);
        for index in 0..TOP {
            let [_, note, _] = led_message(index, 0);
            assert_eq!(key_of_note(note), Some(index));
        }
    }
}
