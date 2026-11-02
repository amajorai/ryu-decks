//! Novation Dicer: a pair of five-button cue controllers over class-compliant
//! USB MIDI ([`super::midi`]). The second unit chains through the first, so
//! the pair is one USB device, one MIDI port pair and one Ryu deck of ten keys.
//!
//! The port is found by name ([`port_matches`]): a word `Dicer` (in any case)
//! in the name, the USB product string. The exact name each host gives it is
//! not documented, so this is inferred.
//!
//! Everything is a three-byte channel message. The unit set to deck A uses
//! channels 11 to 13 (`9A`, `9B`, `9C`), the one set to deck B channels 14 to
//! 16 (`9D`, `9E`, `9F`); which of its three channels a unit uses follows the
//! unit's mode. Buttons 1 to 5, left to right, are notes `3C` to `40`, and
//! notes `41` to `45` are a second layer of the same buttons (Mixxx reads them
//! as shifted presses; that a held mode button is the shift, and that the mode
//! buttons send nothing themselves, is inferred: the reference lists no other
//! input). In: Note On with a velocity above zero presses, zero or a Note Off
//! (`8A` to `8F`, never seen in the reference but harmless) releases. All six
//! variants of a button (three channels, two layers) are one key, tracked by
//! key so a release on another channel still releases it.
//!
//! Out: `BA 00 00` sets both units to a known state (that it turns the lights
//! off is inferred; it is sent on open and when Ryu lets go). Each button's
//! light is a Note On on the button's channel and note, the velocity a colour
//! byte; the device keeps a page per channel and layer and the host cannot
//! tell which is showing, so one key's light is six messages (three channels,
//! both notes). Only these bytes are attested: `7F` green and `71` dim green,
//! `4F` orange and `41` dim orange, `00` off. Mixxx writes green and orange
//! only on the second and third channels; on the first it writes only `0E`
//! (on, hue unnamed) and `00`, so green and orange there are untested. There
//! is no brightness command: Ryu picks the dim or bright byte, and blue is
//! dropped.
//!
//! Facts per Mixxx's Novation Dicer mapping (GPL-2.0-or-later) and the Linux
//! USB ID list (1235:0015); no code was copied.

use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};

pub const DICER_ID: &str = "novation-dicer";

/// Five buttons on each of the two units.
pub const KEYS: usize = 10;
const BUTTONS: usize = 5;
/// Each unit's three mode channels.
const PAGES: u8 = 3;
/// Note On on channel 11, the deck-A unit's first channel.
const FIRST_NOTE_ON: u8 = 0x9a;
const LAST_NOTE_ON: u8 = 0x9f;
const FIRST_NOTE_OFF: u8 = 0x8a;
const LAST_NOTE_OFF: u8 = 0x8f;
/// Buttons 1 to 5.
const FIRST_NOTE: u8 = 0x3c;
/// Buttons 1 to 5 on the second layer.
const FIRST_SHIFTED_NOTE: u8 = 0x41;
const LAST_NOTE: u8 = 0x45;
/// Control Change on channel 11, controller 0, value 0.
const RESET: [u8; 3] = [0xba, 0x00, 0x00];
const OFF: u8 = 0x00;
const GREEN: u8 = 0x7f;
const SOFT_GREEN: u8 = 0x71;
const ORANGE: u8 = 0x4f;
const SOFT_ORANGE: u8 = 0x41;
/// Below this level (of 255, after brightness) a light is off.
const DARK_BELOW: u16 = 32;
/// From this level on a light takes the bright byte; below, the dim one.
const BRIGHT_FROM: u16 = 128;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// Whether a MIDI port, by its normalised name, is the Dicer's: a word
/// `Dicer` in the name. It has one MIDI interface, so on Windows only the
/// first (or an unnumbered) one.
#[must_use]
pub fn port_matches(model: &crate::catalog::DeviceModel, name: &PortName) -> bool {
    model.id == DICER_ID
        && name.interface.unwrap_or(1) == 1
        && name
            .base
            .split([' ', ':'])
            .any(|word| word.eq_ignore_ascii_case("dicer"))
}

/// The deck-A unit's buttons, then the deck-B unit's.
const KEY_NAMES: [&str; KEYS] = ["A1", "A2", "A3", "A4", "A5", "B1", "B2", "B3", "B4", "B5"];

/// The legend Ryu gives each key, on the Dicer.
#[must_use]
pub fn key_names(model: &crate::catalog::DeviceModel) -> Option<Vec<&'static str>> {
    (model.id == DICER_ID).then(|| KEY_NAMES.to_vec())
}

/// The key a Note On or Off on `status` and `note` belongs to.
fn key_of(status: u8, note: u8) -> Option<usize> {
    let channel = if (FIRST_NOTE_ON..=LAST_NOTE_ON).contains(&status) {
        status - FIRST_NOTE_ON
    } else if (FIRST_NOTE_OFF..=LAST_NOTE_OFF).contains(&status) {
        status - FIRST_NOTE_OFF
    } else {
        return None;
    };
    if !(FIRST_NOTE..=LAST_NOTE).contains(&note) {
        return None;
    }
    let unit = usize::from(channel / PAGES);
    Some(unit * BUTTONS + usize::from(note - FIRST_NOTE) % BUTTONS)
}

/// The colour byte for an RGB colour at `percent` brightness: green when
/// green outweighs red, otherwise orange, dim or bright by level; blue is
/// dropped.
fn color_byte(color: [u8; 3], percent: u8) -> u8 {
    let [red, green, _] = color;
    let level = u16::from(red.max(green)) * u16::from(percent.min(100)) / 100;
    let bright = level >= BRIGHT_FROM;
    match (level < DARK_BELOW, green > red, bright) {
        (true, _, _) => OFF,
        (false, true, true) => GREEN,
        (false, true, false) => SOFT_GREEN,
        (false, false, true) => ORANGE,
        (false, false, false) => SOFT_ORANGE,
    }
}

pub struct NovationDicer {
    transport: Box<dyn Transport>,
    /// Reset as far as Ryu knows.
    started: bool,
    colors: [[u8; 3]; KEYS],
    /// What each key's light was last sent; `None` until it is.
    sent: [Option<u8>; KEYS],
    held: [bool; KEYS],
    brightness: u8,
}

impl NovationDicer {
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

    /// Sets both units to a known state and forgets what the lights show.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if !self.started {
            self.transport.write(&RESET)?;
            self.sent = [None; KEYS];
            self.started = true;
        }
        Ok(())
    }

    /// Writes one key's light on all three of its unit's channels, both layers.
    fn write_light(&mut self, index: usize, value: u8) -> DriverResult<()> {
        let unit = (index / BUTTONS) as u8;
        let button = (index % BUTTONS) as u8;
        for page in 0..PAGES {
            let status = FIRST_NOTE_ON + unit * PAGES + page;
            self.transport
                .write(&[status, FIRST_NOTE + button, value])?;
            self.transport
                .write(&[status, FIRST_SHIFTED_NOTE + button, value])?;
        }
        Ok(())
    }

    fn receive(&mut self, message: &[u8]) -> Option<Input> {
        let (Some(&status), Some(&note), Some(&velocity)) =
            (message.first(), message.get(1), message.get(2))
        else {
            return None;
        };
        let key = key_of(status, note)?;
        let pressed = status >= FIRST_NOTE_ON && velocity > 0;
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

impl Driver for NovationDicer {
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

    /// Six messages per key whose light changed (every key after a reset), and
    /// nothing when none did.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        for index in 0..KEYS {
            let value = color_byte(self.colors[index], self.brightness);
            if self.sent[index] != Some(value) {
                self.write_light(index, value)?;
                self.sent[index] = Some(value);
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }
}

impl Drop for NovationDicer {
    /// Sets both units back to a known state.
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

    fn fresh() -> (NovationDicer, Recorder) {
        let recorder = Recorder::default();
        let deck = NovationDicer::new(Box::new(recorder.clone()));
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
    fn opening_resets_then_paints_every_key_on_every_page() {
        let (mut deck, recorder) = fresh();
        deck.set_face(0, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(9, Some(&lit([255, 0, 0]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[0], vec![0xba, 0x00, 0x00]);
        assert_eq!(writes.len(), 1 + 60);
        assert_eq!(
            writes[1..7],
            [
                vec![0x9a, 0x3c, 0x7f],
                vec![0x9a, 0x41, 0x7f],
                vec![0x9b, 0x3c, 0x7f],
                vec![0x9b, 0x41, 0x7f],
                vec![0x9c, 0x3c, 0x7f],
                vec![0x9c, 0x41, 0x7f],
            ]
        );
        // Key 2 (A2) is dark.
        assert_eq!(writes[7], vec![0x9a, 0x3d, 0x00]);
        assert_eq!(
            writes[55..61],
            [
                vec![0x9d, 0x40, 0x4f],
                vec![0x9d, 0x45, 0x4f],
                vec![0x9e, 0x40, 0x4f],
                vec![0x9e, 0x45, 0x4f],
                vec![0x9f, 0x40, 0x4f],
                vec![0x9f, 0x45, 0x4f],
            ]
        );
        // Nothing changed: nothing sent, and no second reset.
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 61);
    }

    #[test]
    fn a_changed_key_is_six_messages_on_its_units_channels() {
        let (mut deck, recorder) = fresh();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_face(6, Some(&lit([0, 100, 0]))).unwrap();
        deck.set_face(200, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [
                vec![0x9d, 0x3d, 0x71],
                vec![0x9d, 0x42, 0x71],
                vec![0x9e, 0x3d, 0x71],
                vec![0x9e, 0x42, 0x71],
                vec![0x9f, 0x3d, 0x71],
                vec![0x9f, 0x42, 0x71],
            ]
        );
    }

    #[test]
    fn only_attested_colour_bytes_are_sent() {
        assert_eq!(color_byte([0, 0, 0], 100), 0x00);
        assert_eq!(color_byte([0, 0, 255], 100), 0x00, "blue is dropped");
        assert_eq!(color_byte([10, 20, 0], 100), 0x00, "near-black");
        assert_eq!(color_byte([0, 255, 0], 100), 0x7f);
        assert_eq!(color_byte([0, 100, 0], 100), 0x71);
        assert_eq!(color_byte([255, 0, 0], 100), 0x4f);
        assert_eq!(color_byte([255, 255, 0], 100), 0x4f);
        assert_eq!(color_byte([255, 128, 0], 100), 0x4f);
        assert_eq!(color_byte([100, 0, 0], 100), 0x41);
        assert_eq!(color_byte([255, 255, 255], 100), 0x4f);
        assert_eq!(color_byte([0, 255, 0], 40), 0x71);
        assert_eq!(color_byte([0, 255, 0], 0), 0x00);
        let attested = [0x00, 0x7f, 0x71, 0x4f, 0x41];
        for red in (0..=255u16).step_by(15) {
            for green in (0..=255u16).step_by(15) {
                for percent in [0, 10, 50, 100] {
                    let byte = color_byte([red as u8, green as u8, 255], percent);
                    assert!(attested.contains(&byte), "{red} {green} {percent}");
                }
            }
        }
    }

    #[test]
    fn brightness_picks_the_dim_byte_or_turns_the_light_off() {
        let (mut deck, recorder) = fresh();
        deck.set_face(0, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_brightness(30).unwrap();
        deck.set_brightness(0).unwrap();
        deck.set_brightness(200).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 18);
        assert_eq!(writes[before], vec![0x9a, 0x3c, 0x71]);
        assert_eq!(writes[before + 6], vec![0x9a, 0x3c, 0x00]);
        assert_eq!(writes[before + 12], vec![0x9a, 0x3c, 0x7f]);
    }

    #[test]
    fn every_variant_of_a_button_is_one_key() {
        let (mut deck, recorder) = fresh();
        for message in [
            // A1 on each page and layer: only the first press and the last
            // release count, and a release on another channel releases it.
            vec![0x9a, 0x3c, 0x7f],
            vec![0x9b, 0x3c, 0x7f],
            vec![0x9c, 0x41, 0x7f],
            vec![0x9b, 0x41, 0x00],
            vec![0x9a, 0x3c, 0x00],
            // A5 shifted on the third page, released by Note Off.
            vec![0x9c, 0x45, 0x40],
            vec![0x8c, 0x45, 0x40],
            // B1 and B5 on the deck-B unit's channels.
            vec![0x9d, 0x3c, 0x7f],
            vec![0x9e, 0x41, 0x00],
            vec![0x9f, 0x40, 0x7f],
            vec![0x8d, 0x45, 0x00],
            // Ignored: notes outside 3C-45, other channels, CC (the reset),
            // Note Off for a key not held, short, SysEx and realtime.
            vec![0x9a, 0x3b, 0x7f],
            vec![0x9a, 0x46, 0x7f],
            vec![0x90, 0x3c, 0x7f],
            vec![0x99, 0x3c, 0x7f],
            vec![0xba, 0x00, 0x00],
            vec![0x8a, 0x3e, 0x00],
            vec![0x9a, 0x3c],
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
                key(4, true),
                key(4, false),
                key(5, true),
                key(5, false),
                key(9, true),
                key(9, false),
            ]
        );
    }

    #[test]
    fn letting_go_resets_the_units() {
        let (mut deck, recorder) = fresh();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 1);
        assert_eq!(writes[before], vec![0xba, 0x00, 0x00]);
        let (deck, recorder) = fresh();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn the_port_is_found_by_the_word_dicer() {
        let model = by_id(DICER_ID).unwrap();
        for raw in [
            "Dicer",
            "dicer",
            "Dicer:Dicer MIDI 1 24:0",
            "Dicer MIDI 1",
            "2- Dicer",
            "MIDIIN1 (Dicer)",
            "Novation Dicer",
        ] {
            assert!(port_matches(model, &normalise(raw)), "{raw}");
            assert_eq!(
                identify_midi(&normalise(raw), raw.starts_with("MIDI")).map(|model| model.id),
                Some(DICER_ID),
                "{raw}"
            );
        }
        for raw in [
            "MIDIIN2 (Dicer)",
            "Dicers",
            "Launchpad",
            "Launchkey MK3 MIDI",
            "Dice",
        ] {
            assert!(!port_matches(model, &normalise(raw)), "{raw}");
        }
        let launchpad = by_id(super::super::launchpad_legacy::LAUNCHPAD_ID).unwrap();
        assert!(!port_matches(launchpad, &normalise("Dicer")));
    }

    #[test]
    fn the_row_names_ten_keys_with_green_and_orange_lights() {
        let model = by_id(DICER_ID).unwrap();
        assert!(model.protocol.is_midi());
        assert_eq!(model.dial_light_colors(), 0);
        let view = ModelView::from(model);
        assert!(view.key_lights && view.brightness);
        assert_eq!(view.key_light_colors, "green_orange");
        assert_eq!(view.deck_lights, 0);
        assert!(view.lit_keys.is_none() && view.key_cells.is_none());
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), usize::from(model.keys));
        assert_eq!(
            (names[0], names[4], names[5], names[9]),
            ("A1", "A5", "B1", "B5")
        );
    }
}
