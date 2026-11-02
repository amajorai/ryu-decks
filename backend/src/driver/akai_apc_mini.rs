//! Akai Professional's first APC mini (MK1), over class-compliant USB MIDI
//! ([`super::midi`]).
//!
//! The APC mini lists one MIDI port each way, named `APC MINI` (in capitals;
//! the MK2 is `APC mini mk2`). [`port_matches`] takes a name containing `APC
//! MINI` and never one containing `mk2` in any case.
//!
//! Everything is on MIDI channel 1, and the deck needs no start-up message: it
//! powers up in its plain note and controller mode. In: the 8 x 8 pads are
//! notes 0-63 with note 0 the bottom-left pad, running left to right and then
//! upwards, so a pad's key (reading order from the top left) is `(7 - note /
//! 8) * 8 + note % 8`; the scene column is notes 82-89 top to bottom (keys
//! 64-71), the track row notes 64-71 left to right (keys 72-79) and Shift
//! note 98 (key 80), the same keys as the APC mini MK2. Any Note On above
//! velocity 0 presses; a Note Off or velocity 0 releases. The nine faders are
//! `CC 48-56` (the master fader last), absolute 0-127, sending nothing until
//! moved, so each one's first value only seeds it and later ones turn its dial
//! a step per two units.
//!
//! Out: every light is a Note On on channel 1 whose velocity is its state. A
//! pad takes 0 off, 1 green, 3 red or 5 yellow (2, 4 and 6 blink those, and
//! Ryu does not use them); a scene or track button 0 off or 1 on (the track
//! row is red; the scene column's colour is not stated in the references).
//! Shift has no light. There is no brightness command and no RGB. The deck
//! does not light a pad itself on a press and keeps the host's lights until
//! power is lost, so the first flush sends every light, later flushes only the
//! changed ones, and Ryu turns them all off when it lets go.
//!
//! Facts from Ableton's APC mini Remote Script as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for
//! note and controller numbers, light values and the USB id only),
//! cross-checked against DrivenByMoss (LGPL-3.0); no code was copied.

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const APC_MINI_ID: &str = "akai-apc-mini";

/// The 8 x 8 pads, keys 0 to 63.
const PADS: u8 = 64;
/// Eight scene keys (64-71), eight track keys (72-79), then Shift (80).
const KEYS: u8 = 81;
const SCENE_KEY: u8 = 64;
const TRACK_KEY: u8 = 72;
const SHIFT_KEY: u8 = 80;
const FADERS: u8 = 9;
const ROW: u8 = 8;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const SCENE_NOTE: u8 = 82;
const TRACK_NOTE: u8 = 64;
const SHIFT_NOTE: u8 = 98;
const FADER_CC: u8 = 48;

const OFF: u8 = 0;
const GREEN: u8 = 1;
const RED: u8 = 3;
const YELLOW: u8 = 5;
const LED_ON: u8 = 1;

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Fader units (of 128) per dial step: 64 steps from end to end.
const FADER_STEP: i16 = 2;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const NAMES: [&str; KEYS as usize] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16", "Pad 17", "Pad 18", "Pad 19",
    "Pad 20", "Pad 21", "Pad 22", "Pad 23", "Pad 24", "Pad 25", "Pad 26", "Pad 27", "Pad 28",
    "Pad 29", "Pad 30", "Pad 31", "Pad 32", "Pad 33", "Pad 34", "Pad 35", "Pad 36", "Pad 37",
    "Pad 38", "Pad 39", "Pad 40", "Pad 41", "Pad 42", "Pad 43", "Pad 44", "Pad 45", "Pad 46",
    "Pad 47", "Pad 48", "Pad 49", "Pad 50", "Pad 51", "Pad 52", "Pad 53", "Pad 54", "Pad 55",
    "Pad 56", "Pad 57", "Pad 58", "Pad 59", "Pad 60", "Pad 61", "Pad 62", "Pad 63", "Pad 64",
    "Scene 1", "Scene 2", "Scene 3", "Scene 4", "Scene 5", "Scene 6", "Scene 7", "Scene 8",
    "Track 1", "Track 2", "Track 3", "Track 4", "Track 5", "Track 6", "Track 7", "Track 8",
    "Shift",
];

/// Whether a normalised port name is an APC mini (MK1) port: `APC MINI` in
/// capitals, never an MK2's `APC mini mk2`, and on Windows the first (only)
/// interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.protocol == Protocol::AkaiApcMiniMidi
        && name.interface.unwrap_or(1) == 1
        && name.base.contains("APC MINI")
        && !name.base.to_ascii_lowercase().contains("mk2")
}

/// The legend on each key: the pads, the scene and track buttons by number,
/// then Shift.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::AkaiApcMiniMidi).then(|| NAMES.to_vec())
}

/// The key a note presses.
const fn key_of_note(note: u8) -> Option<u8> {
    match note {
        0..=63 => Some((ROW - 1 - note / ROW) * ROW + note % ROW),
        64..=71 => Some(TRACK_KEY + note - TRACK_NOTE),
        82..=89 => Some(SCENE_KEY + note - SCENE_NOTE),
        SHIFT_NOTE => Some(SHIFT_KEY),
        _ => None,
    }
}

/// The note of a key: a pad's note counts from the bottom-left pad.
const fn note_of_key(key: u8) -> Option<u8> {
    if key < PADS {
        Some((ROW - 1 - key / ROW) * ROW + key % ROW)
    } else if key < TRACK_KEY {
        Some(SCENE_NOTE + key - SCENE_KEY)
    } else if key < SHIFT_KEY {
        Some(TRACK_NOTE + key - TRACK_KEY)
    } else if key == SHIFT_KEY {
        Some(SHIFT_NOTE)
    } else {
        None
    }
}

/// A pad's state for a colour: off when dark, yellow for white and greys,
/// otherwise by hue: red under 30 degrees or from 330, yellow from 30 to 90,
/// green for the rest (blue and purple included).
pub(super) fn pad_state([red, green, blue]: [u8; 3]) -> u8 {
    let max = red.max(green).max(blue);
    if max < DARK {
        return OFF;
    }
    let min = red.min(green).min(blue);
    let chroma = i32::from(max - min);
    if chroma * 4 < i32::from(max) {
        return YELLOW;
    }
    let hue = if red == max {
        (60 * (i32::from(green) - i32::from(blue)) / chroma).rem_euclid(360)
    } else if green == max {
        120 + 60 * (i32::from(blue) - i32::from(red)) / chroma
    } else {
        240 + 60 * (i32::from(red) - i32::from(green)) / chroma
    };
    match hue {
        0..30 | 330.. => RED,
        30..90 => YELLOW,
        _ => GREEN,
    }
}

/// A scene or track button's state: on for any colour that is not dark.
pub(super) fn led_state(color: [u8; 3]) -> u8 {
    if color.iter().any(|channel| *channel >= DARK) {
        LED_ON
    } else {
        OFF
    }
}

pub struct ApcMini {
    transport: Box<dyn Transport>,
    /// Every light sent once since open.
    started: bool,
    colors: Vec<[u8; 3]>,
    /// The state each lit key was last sent; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    /// Each fader's last position; `None` until it first moves.
    faders: Vec<Option<u8>>,
    /// Fader movement not yet turned into a whole step.
    rest: Vec<i16>,
    parser: Parser,
}

impl ApcMini {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            colors: vec![BLACK; usize::from(KEYS)],
            sent: vec![None; usize::from(SHIFT_KEY)],
            held: vec![false; usize::from(KEYS)],
            faders: vec![None; usize::from(FADERS)],
            rest: vec![0; usize::from(FADERS)],
            parser: Parser::default(),
        }
    }

    fn press(&mut self, key: u8, pressed: bool, events: &mut Vec<Input>) {
        let Some(held) = self.held.get_mut(usize::from(key)) else {
            return;
        };
        if *held != pressed {
            *held = pressed;
            events.push(Input::Key {
                index: key,
                pressed,
            });
        }
    }

    /// A fader's absolute position: the first seeds it, each later one
    /// turns its dial a step per two units moved.
    fn fader(&mut self, dial: u8, value: u8, events: &mut Vec<Input>) {
        let at = usize::from(dial);
        let (Some(last), Some(rest)) = (self.faders.get_mut(at), self.rest.get_mut(at)) else {
            return;
        };
        let Some(old) = last.replace(value) else {
            return;
        };
        let moved = *rest + i16::from(value) - i16::from(old);
        let steps = moved / FADER_STEP;
        *rest = moved - steps * FADER_STEP;
        if steps != 0 {
            events.push(Input::DialTurn {
                index: dial,
                delta: i8::try_from(steps.clamp(-127, 127)).unwrap_or(0),
            });
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        // SysEx (an identity reply, if another app asked) is not used.
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        if status & 0x0f != 0 {
            return;
        }
        match status & 0xf0 {
            NOTE_ON | NOTE_OFF => {
                if let Some(key) = key_of_note(number) {
                    self.press(key, status & 0xf0 == NOTE_ON && value > 0, events);
                }
            }
            CONTROL_CHANGE => {
                if let Some(dial) = number.checked_sub(FADER_CC).filter(|dial| *dial < FADERS) {
                    self.fader(dial, value, events);
                }
            }
            _ => {}
        }
    }
}

impl Driver for ApcMini {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
            self.flush()?;
        }
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

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if key >= SHIFT_KEY {
            return Ok(());
        }
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// One Note On for each light whose state changed (every light the
    /// first time).
    fn flush(&mut self) -> DriverResult<()> {
        self.started = true;
        for key in 0..SHIFT_KEY {
            let (Some(note), Some(color)) = (note_of_key(key), self.colors.get(usize::from(key)))
            else {
                continue;
            };
            let state = if key < PADS {
                pad_state(*color)
            } else {
                led_state(*color)
            };
            let Some(sent) = self.sent.get_mut(usize::from(key)) else {
                continue;
            };
            if *sent != Some(state) {
                self.transport.write(&[NOTE_ON, note, state])?;
                *sent = Some(state);
            }
        }
        Ok(())
    }

    /// The APC mini has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for ApcMini {
    /// Turns every light off: the deck would otherwise keep them.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for note in (0..PADS)
            .chain(TRACK_NOTE..TRACK_NOTE + ROW)
            .chain(SCENE_NOTE..SCENE_NOTE + ROW)
        {
            let _ = self.transport.write(&[NOTE_ON, note, OFF]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    fn open() -> (ApcMini, Recorder) {
        let recorder = Recorder::default();
        let deck = ApcMini::new(by_id(APC_MINI_ID).unwrap(), Box::new(recorder.clone()));
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

    fn queue(recorder: &Recorder, messages: &[&[u8]]) {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
    }

    #[test]
    fn notes_map_to_keys_in_reading_order() {
        assert_eq!(key_of_note(0), Some(56));
        assert_eq!(key_of_note(7), Some(63));
        assert_eq!(key_of_note(56), Some(0));
        assert_eq!(key_of_note(63), Some(7));
        assert_eq!(key_of_note(82), Some(64));
        assert_eq!(key_of_note(89), Some(71));
        assert_eq!(key_of_note(64), Some(72));
        assert_eq!(key_of_note(71), Some(79));
        assert_eq!(key_of_note(98), Some(80));
        for note in [72, 81, 90, 97, 99, 127] {
            assert_eq!(key_of_note(note), None, "{note}");
        }
        for key in 0..KEYS {
            assert_eq!(note_of_key(key).and_then(key_of_note), Some(key));
        }
    }

    #[test]
    fn colours_take_the_nearest_state() {
        assert_eq!(pad_state(BLACK), OFF);
        assert_eq!(pad_state([20, 20, 20]), OFF);
        assert_eq!(pad_state([255, 0, 0]), RED);
        assert_eq!(pad_state([255, 0, 40]), RED);
        assert_eq!(pad_state([255, 0, 255]), GREEN);
        assert_eq!(pad_state([255, 255, 0]), YELLOW);
        assert_eq!(pad_state([255, 128, 0]), YELLOW);
        assert_eq!(pad_state([255, 100, 0]), RED);
        assert_eq!(pad_state([0, 255, 0]), GREEN);
        assert_eq!(pad_state([0, 0, 255]), GREEN);
        assert_eq!(pad_state([0, 255, 255]), GREEN);
        assert_eq!(pad_state([255, 255, 255]), YELLOW);
        assert_eq!(pad_state([128, 128, 128]), YELLOW);
        assert_eq!(led_state([0, 0, 200]), LED_ON);
        assert_eq!(led_state([10, 0, 0]), OFF);
    }

    #[test]
    fn the_first_flush_sends_every_light_and_later_ones_only_changes() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(63, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(64, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(79, Some(&lit([200, 0, 0]))).unwrap();
        // Shift has no light.
        deck.set_face(80, Some(&lit([255, 255, 255]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 80);
        // Key 0 is the top-left pad, note 56.
        assert_eq!(writes[0], vec![0x90, 56, 3]);
        assert_eq!(writes[1], vec![0x90, 57, 0]);
        assert_eq!(writes[63], vec![0x90, 7, 1]);
        assert_eq!(writes[64], vec![0x90, 82, 1]);
        assert_eq!(writes[65], vec![0x90, 83, 0]);
        assert_eq!(writes[72], vec![0x90, 64, 0]);
        assert_eq!(writes[79], vec![0x90, 71, 1]);
        assert!(writes
            .iter()
            .all(|write| write.len() == 3 && write[1] != 98));
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 80);
        deck.set_face(8, Some(&lit([255, 255, 0]))).unwrap();
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[80..],
            [vec![0x90, 56, 0], vec![0x90, 48, 5]]
        );
        // Brightness changes nothing.
        deck.set_brightness(10).unwrap();
        assert_eq!(recorder.writes().len(), 82);
    }

    #[test]
    fn presses_map_and_other_channels_are_dropped() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 0, 0x7f],
                &[0x80, 0, 0x7f],
                &[0x90, 63, 0x40],
                &[0x90, 63, 0x00],
                &[0x90, 82, 0x7f],
                &[0x90, 64, 0x7f],
                &[0x90, 98, 0x7f],
                &[0x80, 98, 0x00],
                // Unknown notes, another channel, and a SysEx reply.
                &[0x90, 72, 0x7f],
                &[0x90, 99, 0x7f],
                &[0x91, 0, 0x7f],
                &[0xf0, 0x7e, 0x00, 0x06, 0x02, 0x47, 0x28, 0xf7],
                // Short and stray input.
                &[0x90],
                &[0x47, 0x00],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(56, true),
                key(56, false),
                key(7, true),
                key(7, false),
                key(64, true),
                key(72, true),
                key(80, true),
                key(80, false),
            ]
        );
    }

    #[test]
    fn faders_turn_dials_after_their_first_value() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 48, 0],
                &[0xb0, 48, 4],
                &[0xb0, 48, 5],
                &[0xb0, 48, 6],
                &[0xb0, 56, 127],
                &[0xb0, 56, 100],
                // Not a fader; another channel.
                &[0xb0, 57, 3],
                &[0xb1, 48, 100],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialTurn { index: 0, delta: 2 },
                Input::DialTurn { index: 0, delta: 1 },
                Input::DialTurn {
                    index: 8,
                    delta: -13
                },
            ]
        );
    }

    #[test]
    fn letting_go_turns_every_light_off() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        drop(deck);
        let writes = &recorder.writes()[80..];
        assert_eq!(writes.len(), 80);
        assert_eq!(writes[0], vec![0x90, 0, 0]);
        assert_eq!(writes[64], vec![0x90, 64, 0]);
        assert_eq!(writes[72], vec![0x90, 82, 0]);
        assert_eq!(writes[79], vec![0x90, 89, 0]);
        // Never opened: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_the_first_apc_minis_ports_match() {
        let model = by_id(APC_MINI_ID).unwrap();
        let cases = [
            ("APC MINI", false, true),
            ("APC MINI:APC MINI MIDI 1 24:0", false, true),
            ("2- APC MINI", true, true),
            ("MIDIIN2 (APC MINI)", true, false),
            // The MK2's ports, in either case, and an unrelated port.
            ("APC mini mk2 Control", false, false),
            ("APC MINI MK2", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(APC_MINI_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_places_every_key() {
        let view = ModelView::from(by_id(APC_MINI_ID).unwrap());
        let cells = view.key_cells.unwrap();
        assert_eq!(cells.len(), 81);
        assert_eq!(
            (cells[0], cells[63], cells[64], cells[71], cells[72], cells[80]),
            (0, 7 * 9 + 7, 8, 7 * 9 + 8, 8 * 9, 80)
        );
        let names = view.key_names.unwrap();
        assert_eq!(
            (names[0], names[63], names[64], names[71], names[72], names[80]),
            ("Pad 1", "Pad 64", "Scene 1", "Scene 8", "Track 1", "Shift")
        );
        assert_eq!(view.lit_keys.unwrap(), (0..80).collect::<Vec<u8>>());
        assert_eq!(view.key_light_colors, "green_red_yellow");
        assert_eq!(view.dial_lights, DialLights::None);
        assert!(view.key_lights && !view.brightness);
    }
}
