//! Intech Studio's Grid modules (EN16, PO16, BU16, EF44, PBF4 and TEK2) on
//! their factory default profile, over class-compliant USB MIDI
//! ([`super::midi`]). Input only.
//!
//! Every module type enumerates with the same USB ids, the same product
//! string (`Grid`) and so the same MIDI port name, and the firmware answers no
//! SysEx identity request, so MIDI cannot tell the types apart: one generic
//! 4 x 4 model serves them all. Element `i` of the module plugged into USB is
//! key `i` (its button, or the push of its encoder or endless knob) and dial
//! `i` (its turn, potentiometer or fader). Which elements exist depends on the
//! module: EN16 0-15 encoders, PO16 0-15 potentiometers, BU16 0-15 buttons,
//! PBF4 0-3 potentiometers, 4-7 faders and 8-11 buttons, EF44 0-3 encoders and
//! 4-7 faders, TEK2 0-7 buttons and 8-9 endless knobs.
//!
//! [`port_matches`] takes the port by name only: `Grid` (the bare name on
//! macOS and Windows), the ALSA `Grid:Grid MIDI <n>`, a name that is or
//! contains the MIDI interface's own string `Intech Grid MIDI` (which Windows
//! and macOS may use for a composite device's function), or the legacy
//! `Intech Studio: Grid`; on Windows only the first interface. None of these
//! names has been seen on hardware: they follow from the USB descriptors.
//! A name that merely contains `Grid` is never taken.
//!
//! In, per the factory profile: the module plugged into USB is position
//! (0, 0), so its elements use controller and note numbers `32 + i` on MIDI
//! channel `1 + page` (pages 0-3, switched by the module's utility button).
//! A button or push is Note On velocity 127 pressed and Note On velocity 0
//! released (a Note Off releases too). Encoders and endless knobs send an
//! absolute Control Change 0-127 that stops at either end yet is sent again
//! on every further detent there; potentiometers and faders send their
//! absolute 7-bit position whenever it changes. Ryu keeps the last value per
//! channel and dial: the first one only seeds it, then the difference turns
//! the dial, and a repeated 127 or 0 turns it +1 or -1 (an encoder held at
//! its end). Chained modules (numbers outside 32-47, channels 5-16), other
//! messages and SysEx are ignored. No dial is pressed: pushes arrive as keys.
//!
//! Out: nothing. The factory profile has no MIDI receive callbacks, so each
//! element's LED follows only its own value on the module; colours need the
//! Grid's serial (CDC) protocol, which this driver does not speak. Faces,
//! lights and brightness are accepted and dropped.
//!
//! Facts from Intech Studio's grid-fw firmware and grid-editor (both
//! GPL-3.0), read for the USB descriptors, element tables, default actions
//! and MIDI numbering only; no code was copied.

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const GRID_ID: &str = "intech-grid";

pub const ROWS: u8 = 4;
pub const COLS: u8 = 4;
/// One key per element of a 16-element module.
pub const KEYS: u8 = 16;
/// One dial per element of a 16-element module.
pub const DIALS: u8 = 16;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// Pages 0-3 of the USB module are MIDI channels 1-4 (indices 0-3).
const PAGES: u8 = 4;
/// Element 0's note and controller number on the USB module.
const FIRST_NUMBER: u8 = 32;
const TOP: u8 = 127;

const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// Whether a normalised port name is a Grid module's MIDI port.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.protocol != Protocol::IntechGridMidi || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.as_str();
    base == "Grid"
        || base.starts_with("Grid:Grid MIDI")
        || base.contains("Intech Grid MIDI")
        || base == "Intech Studio: Grid"
        || base.starts_with("Intech Studio: Grid:")
}

/// The element (key or dial) a note or controller number is on the USB
/// module, if it is one of its 16.
const fn element_of(number: u8) -> Option<u8> {
    if number >= FIRST_NUMBER && number < FIRST_NUMBER + KEYS {
        Some(number - FIRST_NUMBER)
    } else {
        None
    }
}

/// The steps a control's new absolute value turns its dial after `last`: the
/// difference, or one step on at an end it was already at.
fn steps(last: u8, value: u8) -> i8 {
    match (last, value) {
        (TOP, TOP) => 1,
        (0, 0) => -1,
        _ => i8::try_from(i16::from(value) - i16::from(last)).unwrap_or(0),
    }
}

pub struct Grid {
    transport: Box<dyn Transport>,
    held: [bool; KEYS as usize],
    /// Each dial's last value per page; `None` until it first arrives.
    values: [[Option<u8>; DIALS as usize]; PAGES as usize],
    parser: Parser,
}

impl Grid {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            held: [false; KEYS as usize],
            values: [[None; DIALS as usize]; PAGES as usize],
            parser: Parser::default(),
        }
    }

    fn press(&mut self, key: u8, pressed: bool, events: &mut Vec<Input>) {
        let Some(held) = self.held.get_mut(usize::from(key)) else {
            return;
        };
        if *held == pressed {
            return;
        }
        *held = pressed;
        events.push(Input::Key {
            index: key,
            pressed,
        });
    }

    /// A control's absolute value on one page: the first seeds it, each later
    /// one turns its dial.
    fn control(&mut self, page: u8, dial: u8, value: u8, events: &mut Vec<Input>) {
        let Some(last) = self
            .values
            .get_mut(usize::from(page))
            .and_then(|page| page.get_mut(usize::from(dial)))
        else {
            return;
        };
        let Some(old) = last.replace(value) else {
            return;
        };
        let delta = steps(old, value);
        if delta != 0 {
            events.push(Input::DialTurn { index: dial, delta });
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        let page = status & 0x0f;
        if page >= PAGES {
            return;
        }
        let Some(element) = element_of(number) else {
            return;
        };
        match status & 0xf0 {
            NOTE_ON => self.press(element, value > 0, events),
            NOTE_OFF => self.press(element, false, events),
            CONTROL_CHANGE => self.control(page, element, value, events),
            _ => {}
        }
    }
}

impl Driver for Grid {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut messages = Vec::new();
        let mut buf = [0u8; READ_LEN];
        for attempt in 0..MAX_MESSAGES_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.parser
                .feed(buf.get(..read).unwrap_or_default(), &mut messages);
        }
        for message in &messages {
            self.receive(message, &mut events);
        }
        Ok(events)
    }

    /// The module shows no host colour on the factory profile.
    fn set_face(&mut self, _key: u8, _face: Option<&Face>) -> DriverResult<()> {
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        Ok(())
    }

    /// No brightness command over MIDI.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    fn open() -> (Grid, Recorder) {
        let recorder = Recorder::default();
        let deck = Grid::new(by_id(GRID_ID).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    fn turn(index: u8, delta: i8) -> Input {
        Input::DialTurn { index, delta }
    }

    fn feed(deck: &mut Grid, recorder: &Recorder, messages: &[&[u8]]) -> Vec<Input> {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
        deck.poll(0).unwrap()
    }

    #[test]
    fn numbers_32_to_47_are_the_usb_modules_elements() {
        assert_eq!(element_of(32), Some(0));
        assert_eq!(element_of(47), Some(15));
        for number in [0, 16, 31, 48, 63, 127] {
            assert_eq!(element_of(number), None, "{number}");
        }
        assert_eq!(steps(10, 13), 3);
        assert_eq!(steps(13, 8), -5);
        assert_eq!(steps(127, 127), 1);
        assert_eq!(steps(0, 0), -1);
        assert_eq!(steps(64, 64), 0);
        assert_eq!(steps(0, 127), 127);
        assert_eq!(steps(127, 0), -127);
    }

    #[test]
    fn buttons_press_on_velocity_and_release_on_zero_or_note_off() {
        let (mut deck, recorder) = open();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                // Element 0 pressed, released by a Note On at velocity 0.
                &[0x90, 32, 127],
                &[0x90, 32, 0],
                // Element 15 released by a Note Off.
                &[0x90, 47, 127],
                &[0x80, 47, 0],
                // Element 9 on page 3 (channel 4).
                &[0x93, 41, 127],
                // A repeated press is not a second one.
                &[0x93, 41, 127],
                &[0x93, 41, 0],
            ],
        );
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(0, false),
                key(15, true),
                key(15, false),
                key(9, true),
                key(9, false),
            ]
        );
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn chained_modules_other_channels_and_stray_bytes_are_ignored() {
        let (mut deck, recorder) = open();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                // The module left of and right of the USB one.
                &[0x90, 31, 127],
                &[0x90, 48, 127],
                &[0xb0, 16, 5],
                &[0xb0, 16, 9],
                // A module below it (channel 5) and beyond.
                &[0x94, 32, 127],
                &[0xbf, 33, 3],
                &[0xbf, 33, 9],
                // Aftertouch, Program Change, SysEx, short and stray input.
                &[0xa0, 32, 100],
                &[0xc0, 32],
                &[0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7],
                // Data bytes with no status after the SysEx, then a cut-off
                // message.
                &[0x20, 0x7f],
                &[0x90],
            ],
        );
        assert!(events.is_empty(), "{events:?}");
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn the_first_value_seeds_a_dial_and_later_ones_turn_it() {
        let (mut deck, recorder) = open();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                // An encoder's first detent after power-up only seeds it.
                &[0xb0, 32, 1],
                &[0xb0, 32, 2],
                // A fast turn moves several steps at once.
                &[0xb0, 32, 7],
                &[0xb0, 32, 4],
                // A fader swept far in one message.
                &[0xb0, 39, 0],
                &[0xb0, 39, 127],
            ],
        );
        assert_eq!(
            events,
            vec![turn(0, 1), turn(0, 5), turn(0, -3), turn(7, 127)]
        );
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn an_encoder_held_at_an_end_keeps_turning() {
        let (mut deck, recorder) = open();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xb0, 33, 125],
                &[0xb0, 33, 127],
                &[0xb0, 33, 127],
                &[0xb0, 33, 127],
                &[0xb0, 34, 2],
                &[0xb0, 34, 0],
                &[0xb0, 34, 0],
                // Seeded at the end: the next repeat turns it already.
                &[0xb0, 35, 0],
                &[0xb0, 35, 0],
            ],
        );
        assert_eq!(
            events,
            vec![
                turn(1, 2),
                turn(1, 1),
                turn(1, 1),
                turn(2, -2),
                turn(2, -1),
                turn(3, -1),
            ]
        );
    }

    #[test]
    fn each_page_keeps_its_own_values() {
        let (mut deck, recorder) = open();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xb0, 32, 10],
                // Page 2 (channel 2): its own value, so this only seeds it.
                &[0xb1, 32, 100],
                &[0xb1, 32, 98],
                // Back on page 1 the first value still counts from 10.
                &[0xb0, 32, 12],
                &[0xb3, 47, 64],
                &[0xb3, 47, 65],
            ],
        );
        assert_eq!(events, vec![turn(0, -2), turn(0, 2), turn(15, 1)]);
    }

    #[test]
    fn faces_lights_and_brightness_send_nothing() {
        let (mut deck, recorder) = open();
        let face = Face {
            label: String::from("Go"),
            color: [255, 0, 0],
            image: None,
        };
        deck.set_face(0, Some(&face)).unwrap();
        deck.set_face(15, None).unwrap();
        deck.set_face(200, Some(&face)).unwrap();
        deck.flush().unwrap();
        deck.set_brightness(0).unwrap();
        deck.set_brightness(100).unwrap();
        deck.set_dial_light(3, Some([0, 255, 0])).unwrap();
        deck.set_deck_light(Some([0, 0, 255])).unwrap();
        deck.poll(0).unwrap();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_grid_ports_match() {
        let model = by_id(GRID_ID).unwrap();
        let cases = [
            ("Grid", false, true),
            ("Grid", true, true),
            ("2- Grid", true, true),
            ("Grid:Grid MIDI 1 24:0", false, true),
            ("Intech Grid MIDI", false, true),
            ("Intech Grid MIDI", true, true),
            ("Grid Intech Grid MIDI", false, true),
            ("Intech Studio: Grid", false, true),
            (
                "Intech Studio: Grid:Intech Studio: Grid MIDI 1 24:0",
                false,
                true,
            ),
            ("MIDIIN2 (Grid)", true, false),
            ("MIDIIN2 (Intech Grid MIDI)", true, false),
            // Names that only contain the word.
            ("Grid MIDI", false, false),
            ("GridController", false, false),
            ("Ableton Grid", false, false),
            ("monome grid", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(GRID_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_has_no_lights() {
        let view = ModelView::from(by_id(GRID_ID).unwrap());
        assert_eq!(
            (view.rows, view.cols, view.keys, view.dials),
            (4, 4, 16, 16)
        );
        assert_eq!(view.lit_keys, None);
        assert_eq!(view.dial_lights, DialLights::None);
        assert!(!view.key_lights && !view.brightness);
    }
}
