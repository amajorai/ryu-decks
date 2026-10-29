//! Livid Instruments' Base (v1) and Base II, over class-compliant USB MIDI
//! ([`super::midi`]).
//!
//! Each deck lists its MIDI ports under a short product token: `Base8` for the
//! Base, `Base2` for the Base II (`Base2`, `Base2 MIDI 1` or `Base2
//! Base_Controls`, and on Windows `MIDIIN<n> (Base8)` and the like). Ableton's
//! script for the deck declares a second port pair whose role is unknown, so
//! [`port_matches`] takes the token only as a whole word, on Windows only the
//! first interface, on ALSA only port 0, and never a name ending `MIDI 2`.
//!
//! Everything Ryu reads is on MIDI channel 1. Keys run in reading order over a
//! 6 x 8 grid: the eight capacitive touch buttons across the top are notes
//! 10-17 (keys 0-7); the 32 pads are notes 36-67 with the top row 60-67, then
//! 52-59, 44-51 and 36-43 (keys 8-39); the eight function buttons are notes
//! 18-25 (keys 40-47, placed in the bottom row as a layout choice). A Note On
//! above velocity 0 presses; a Note Off or velocity 0 releases. The nine
//! touch faders are `CC 1-9` (the master last), absolute 0-127, so each one's
//! first value only seeds it and later ones turn its dial a step per two
//! units. Pad pressure (`CC 36-67`), aftertouch, the fader touch notes (their
//! numbers are not known), the runner LEDs' notes 68-75, every other note and
//! channel and all SysEx are ignored.
//!
//! Out, on the first poll or flush and never awaiting a reply: the pads'
//! output type set to Live mode, `F0 00 01 61 <P> 42` and 32 x `05` then `F7`,
//! where `P` is the product byte (`0C` for the Base, `11` for the Base II);
//! local control off, `BF 7A 40`; then every key light. A light is a Note On
//! on channel 1 whose velocity picks a palette colour: 0 off, 1 white, 5
//! cyan, 9 magenta, 17 red, 33 blue, 65 yellow, 127 green. There is no
//! brightness command. Later flushes send only changed lights, and Ryu turns
//! them off when it lets go. Local control is left off: no reference gives a
//! documented way to turn it back on. Ryu never writes `CC 1-9` (the faders'
//! LED position), `CC 34/35` (the two-character display) or the fader colours.
//!
//! Facts from Livid's own Bitwig scripts for the Base and Base II
//! (LividInstruments/Bitwig, no licence; read for port names, product bytes,
//! the start-up SysEx, palette velocities and control numbers only),
//! cross-checked against aumhaa's Livid_Base_v2 Ableton script and its Livid
//! utilities (aumhaa/m4m8, no licence; note and controller numbers and SysEx
//! framing only); no code was copied.

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const BASE_ID: &str = "livid-base";
pub const BASE_II_ID: &str = "livid-base-ii";

pub const ROWS: u8 = 6;
pub const COLS: u8 = 8;
/// Touch buttons (0-7), pads (8-39), function buttons (40-47).
pub const KEYS: u8 = 48;
/// The nine touch faders; the master is dial 9.
pub const DIALS: u8 = 9;

const PAD_KEY: u8 = 8;
const FUNCTION_KEY: u8 = 40;
const ROW: u8 = 8;

const TOUCH_NOTE: u8 = 10;
const FUNCTION_NOTE: u8 = 18;
const PAD_NOTE: u8 = 36;
const LAST_PAD_NOTE: u8 = 67;
const FADER_CC: u8 = 1;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;

/// Livid's SysEx manufacturer id.
const LIVID: [u8; 3] = [0x00, 0x01, 0x61];
/// The Base's and Base II's SysEx product bytes.
const BASE_PRODUCT: u8 = 0x0c;
const BASE_II_PRODUCT: u8 = 0x11;
/// Sets each of the 32 pads' output type.
const SET_PAD_OUTPUT: u8 = 0x42;
/// The host-lit (Live) pad output type.
const LIVE_PADS: u8 = 0x05;
/// Local control off: `CC 122 = 64` on channel 16.
const LOCAL_OFF: [u8; 3] = [0xbf, 0x7a, 0x40];

pub(super) const OFF: u8 = 0;
pub(super) const WHITE: u8 = 1;
pub(super) const CYAN: u8 = 5;
pub(super) const MAGENTA: u8 = 9;
pub(super) const RED: u8 = 17;
pub(super) const BLUE: u8 = 33;
pub(super) const YELLOW: u8 = 65;
pub(super) const GREEN: u8 = 127;

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Fader units (of 128) per dial step: 64 steps from end to end.
const FADER_STEP: i16 = 2;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const NAMES: [&str; KEYS as usize] = [
    "Touch 1",
    "Touch 2",
    "Touch 3",
    "Touch 4",
    "Touch 5",
    "Touch 6",
    "Touch 7",
    "Touch 8",
    "Pad 1",
    "Pad 2",
    "Pad 3",
    "Pad 4",
    "Pad 5",
    "Pad 6",
    "Pad 7",
    "Pad 8",
    "Pad 9",
    "Pad 10",
    "Pad 11",
    "Pad 12",
    "Pad 13",
    "Pad 14",
    "Pad 15",
    "Pad 16",
    "Pad 17",
    "Pad 18",
    "Pad 19",
    "Pad 20",
    "Pad 21",
    "Pad 22",
    "Pad 23",
    "Pad 24",
    "Pad 25",
    "Pad 26",
    "Pad 27",
    "Pad 28",
    "Pad 29",
    "Pad 30",
    "Pad 31",
    "Pad 32",
    "Function 1",
    "Function 2",
    "Function 3",
    "Function 4",
    "Function 5",
    "Function 6",
    "Function 7",
    "Function 8",
];

/// The port-name token and SysEx product byte of a Base model.
fn product(model: &DeviceModel) -> Option<(&'static str, u8)> {
    if model.protocol != Protocol::LividBaseMidi {
        return None;
    }
    match model.id {
        BASE_ID => Some(("Base8", BASE_PRODUCT)),
        BASE_II_ID => Some(("Base2", BASE_II_PRODUCT)),
        _ => None,
    }
}

/// Whether a normalised port name is this Base model's first port: its token
/// (`Base8` or `Base2`) as a whole word, on Windows the first interface, on
/// ALSA port 0, and never a second `MIDI 2` port.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    let Some((token, _)) = product(model) else {
        return false;
    };
    name.interface.unwrap_or(1) == 1
        && name.alsa_port.unwrap_or(0) == 0
        && !name.base.ends_with("MIDI 2")
        && name
            .base
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word == token)
}

/// The legend on each key: the touch buttons, pads and function buttons by
/// number.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::LividBaseMidi).then(|| NAMES.to_vec())
}

/// The key a note presses.
const fn key_of_note(note: u8) -> Option<u8> {
    match note {
        TOUCH_NOTE..FUNCTION_NOTE => Some(note - TOUCH_NOTE),
        FUNCTION_NOTE..=25 => Some(FUNCTION_KEY + note - FUNCTION_NOTE),
        PAD_NOTE..=LAST_PAD_NOTE => {
            let pad = note - PAD_NOTE;
            Some(PAD_KEY + (3 - pad / ROW) * ROW + pad % ROW)
        }
        _ => None,
    }
}

/// The note of a key: the pads' top row is notes 60-67.
const fn note_of_key(key: u8) -> Option<u8> {
    if key < PAD_KEY {
        Some(TOUCH_NOTE + key)
    } else if key < FUNCTION_KEY {
        let pad = key - PAD_KEY;
        Some(PAD_NOTE + (3 - pad / ROW) * ROW + pad % ROW)
    } else if key < KEYS {
        Some(FUNCTION_NOTE + key - FUNCTION_KEY)
    } else {
        None
    }
}

/// A light's palette velocity for a colour: off when dark, white for low
/// chroma, otherwise by hue.
pub(super) fn palette([red, green, blue]: [u8; 3]) -> u8 {
    let max = red.max(green).max(blue);
    if max < DARK {
        return OFF;
    }
    let min = red.min(green).min(blue);
    let chroma = i32::from(max - min);
    if chroma * 4 < i32::from(max) {
        return WHITE;
    }
    let hue = if red == max {
        (60 * (i32::from(green) - i32::from(blue)) / chroma).rem_euclid(360)
    } else if green == max {
        120 + 60 * (i32::from(blue) - i32::from(red)) / chroma
    } else {
        240 + 60 * (i32::from(red) - i32::from(green)) / chroma
    };
    match hue {
        30..90 => YELLOW,
        90..150 => GREEN,
        150..210 => CYAN,
        210..270 => BLUE,
        270..330 => MAGENTA,
        _ => RED,
    }
}

/// The SysEx that puts all 32 pads in the host-lit Live output type.
fn pad_mode(product: u8) -> Vec<u8> {
    let mut message = vec![0xf0];
    message.extend_from_slice(&LIVID);
    message.extend_from_slice(&[product, SET_PAD_OUTPUT]);
    message.extend_from_slice(&[LIVE_PADS; 32]);
    message.push(0xf7);
    message
}

pub struct Base {
    transport: Box<dyn Transport>,
    product: u8,
    /// The start-up messages and every light sent once since open.
    started: bool,
    colors: Vec<[u8; 3]>,
    /// The velocity each key's light was last sent; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    /// Each fader's last position; `None` until it first moves.
    faders: Vec<Option<u8>>,
    /// Fader movement not yet turned into a whole step.
    rest: Vec<i16>,
    parser: Parser,
}

impl Base {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            product: product(model).map_or(BASE_II_PRODUCT, |(_, byte)| byte),
            started: false,
            colors: vec![BLACK; usize::from(KEYS)],
            sent: vec![None; usize::from(KEYS)],
            held: vec![false; usize::from(KEYS)],
            faders: vec![None; usize::from(DIALS)],
            rest: vec![0; usize::from(DIALS)],
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
        // SysEx (an identity reply or the pad stream) is not used.
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
                if let Some(dial) = number.checked_sub(FADER_CC).filter(|dial| *dial < DIALS) {
                    self.fader(dial, value, events);
                }
            }
            _ => {}
        }
    }
}

impl Driver for Base {
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
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// The start-up messages the first time, then one Note On for each light
    /// whose velocity changed (every light the first time).
    fn flush(&mut self) -> DriverResult<()> {
        if !self.started {
            self.started = true;
            self.transport.write(&pad_mode(self.product))?;
            self.transport.write(&LOCAL_OFF)?;
        }
        for key in 0..KEYS {
            let (Some(note), Some(color)) = (note_of_key(key), self.colors.get(usize::from(key)))
            else {
                continue;
            };
            let velocity = palette(*color);
            let Some(sent) = self.sent.get_mut(usize::from(key)) else {
                continue;
            };
            if *sent != Some(velocity) {
                self.transport.write(&[NOTE_ON, note, velocity])?;
                *sent = Some(velocity);
            }
        }
        Ok(())
    }

    /// The Base has no brightness command.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for Base {
    /// Turns every key light off. Local control stays off: no documented
    /// message turns it back on.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for note in (TOUCH_NOTE..=25).chain(PAD_NOTE..=LAST_PAD_NOTE) {
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

    fn open(id: &str) -> (Base, Recorder) {
        let recorder = Recorder::default();
        let deck = Base::new(by_id(id).unwrap(), Box::new(recorder.clone()));
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

    fn expected_pad_mode(product: u8) -> Vec<u8> {
        let mut bytes = vec![0xf0, 0x00, 0x01, 0x61, product, 0x42];
        bytes.extend([0x05; 32]);
        bytes.push(0xf7);
        bytes
    }

    #[test]
    fn notes_map_to_keys_in_reading_order() {
        assert_eq!(key_of_note(10), Some(0));
        assert_eq!(key_of_note(17), Some(7));
        assert_eq!(key_of_note(60), Some(8));
        assert_eq!(key_of_note(67), Some(15));
        assert_eq!(key_of_note(52), Some(16));
        assert_eq!(key_of_note(36), Some(32));
        assert_eq!(key_of_note(43), Some(39));
        // Note 18 is function button 1, never a touch button.
        assert_eq!(key_of_note(18), Some(40));
        assert_eq!(key_of_note(25), Some(47));
        for note in (0..10).chain(26..36).chain(68..=127) {
            assert_eq!(key_of_note(note), None, "{note}");
        }
        for key in 0..KEYS {
            assert_eq!(note_of_key(key).and_then(key_of_note), Some(key), "{key}");
        }
        assert_eq!(note_of_key(KEYS), None);
    }

    #[test]
    fn colours_take_the_nearest_palette_velocity() {
        assert_eq!(palette(BLACK), OFF);
        assert_eq!(palette([20, 25, 31]), OFF);
        assert_eq!(palette([255, 255, 255]), WHITE);
        assert_eq!(palette([128, 120, 110]), WHITE);
        assert_eq!(palette([255, 0, 0]), RED);
        assert_eq!(palette([255, 0, 40]), RED);
        assert_eq!(palette([255, 255, 0]), YELLOW);
        assert_eq!(palette([255, 128, 0]), YELLOW);
        assert_eq!(palette([0, 255, 0]), GREEN);
        assert_eq!(palette([0, 255, 255]), CYAN);
        assert_eq!(palette([0, 0, 255]), BLUE);
        assert_eq!(palette([255, 0, 255]), MAGENTA);
        assert_eq!(
            [OFF, WHITE, CYAN, MAGENTA, RED, BLUE, YELLOW, GREEN],
            [0, 1, 5, 9, 17, 33, 65, 127]
        );
    }

    #[test]
    fn the_first_flush_sets_live_pads_local_off_and_every_light() {
        for (id, product) in [(BASE_II_ID, 0x11), (BASE_ID, 0x0c)] {
            let (mut deck, recorder) = open(id);
            deck.set_face(0, Some(&lit([255, 255, 255]))).unwrap();
            deck.set_face(8, Some(&lit([255, 0, 0]))).unwrap();
            deck.set_face(47, Some(&lit([0, 0, 255]))).unwrap();
            assert!(
                recorder.writes().is_empty(),
                "nothing before the first poll"
            );
            deck.poll(0).unwrap();
            let writes = recorder.writes();
            assert_eq!(writes.len(), 2 + 48);
            assert_eq!(writes[0], expected_pad_mode(product));
            assert_eq!(writes[0].len(), 39);
            assert_eq!(writes[1], vec![0xbf, 0x7a, 0x40]);
            assert_eq!(writes[2], vec![0x90, 10, 1]);
            assert_eq!(writes[3], vec![0x90, 11, 0]);
            // Key 8 is the top-left pad, note 60.
            assert_eq!(writes[10], vec![0x90, 60, 17]);
            assert_eq!(writes[49], vec![0x90, 25, 33]);
            // Never the fader position, display or fader colour controls.
            assert!(writes[1..]
                .iter()
                .all(|write| write.len() == 3 && (write[0] == 0x90 || write == &LOCAL_OFF)));
            deck.flush().unwrap();
            assert_eq!(recorder.writes().len(), 50);
            deck.set_face(8, None).unwrap();
            deck.set_face(39, Some(&lit([0, 255, 0]))).unwrap();
            deck.flush().unwrap();
            assert_eq!(
                recorder.writes()[50..],
                [vec![0x90, 60, 0], vec![0x90, 43, 127]]
            );
            deck.set_brightness(10).unwrap();
            assert_eq!(recorder.writes().len(), 52);
        }
    }

    #[test]
    fn presses_map_and_other_input_is_ignored() {
        let (mut deck, recorder) = open(BASE_II_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 10, 0x7f],
                &[0x80, 10, 0x00],
                &[0x90, 60, 0x40],
                &[0x90, 60, 0x00],
                &[0x90, 18, 0x7f],
                &[0x80, 18, 0x7f],
                // Pad pressure, aftertouch, touch and runner notes, another
                // channel, the identity reply and the pad stream.
                &[0xb0, 36, 0x40],
                &[0xb0, 67, 0x40],
                &[0xa0, 60, 0x40],
                &[0xd0, 0x40],
                &[0x90, 1, 0x7f],
                &[0x90, 9, 0x7f],
                &[0x90, 68, 0x7f],
                &[0x90, 75, 0x7f],
                &[0x91, 10, 0x7f],
                &[0xb1, 1, 0x40],
                &[
                    0xf0, 0x7e, 0x00, 0x06, 0x02, 0x00, 0x01, 0x61, 0x01, 0x00, 0x11, 0x00, 0x01,
                    0x00, 0x00, 0x00, 0xf7,
                ],
                &[
                    0xf0, 0x00, 0x01, 0x61, 0x11, 0x40, 1, 2, 3, 4, 5, 6, 7, 8, 0xf7,
                ],
                // Short and stray input.
                &[0x90],
                &[0x47, 0x00],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(8, true),
                key(8, false),
                key(40, true),
                key(40, false),
            ]
        );
    }

    #[test]
    fn faders_turn_dials_after_their_first_value() {
        let (mut deck, recorder) = open(BASE_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 1, 0],
                &[0xb0, 1, 4],
                &[0xb0, 1, 5],
                &[0xb0, 1, 6],
                &[0xb0, 9, 127],
                &[0xb0, 9, 100],
                // Not a fader.
                &[0xb0, 0, 3],
                &[0xb0, 10, 3],
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
        let (mut deck, recorder) = open(BASE_II_ID);
        deck.flush().unwrap();
        drop(deck);
        let writes = &recorder.writes()[50..];
        assert_eq!(writes.len(), 16 + 32);
        assert_eq!(writes[0], vec![0x90, 10, 0]);
        assert_eq!(writes[15], vec![0x90, 25, 0]);
        assert_eq!(writes[16], vec![0x90, 36, 0]);
        assert_eq!(writes[47], vec![0x90, 67, 0]);
        // Never opened: nothing sent.
        let (deck, recorder) = open(BASE_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_each_bases_first_port_matches() {
        let base_ii = by_id(BASE_II_ID).unwrap();
        let base = by_id(BASE_ID).unwrap();
        let cases = [
            ("Base2", BASE_II_ID),
            ("Base2 MIDI 1", BASE_II_ID),
            ("Base2 Base_Controls", BASE_II_ID),
            ("Base2:Base2 MIDI 1 24:0", BASE_II_ID),
            ("Base8", BASE_ID),
            ("MIDIIN1 (Base8)", BASE_ID),
        ];
        for (raw, id) in cases {
            let name = normalise(raw);
            assert_eq!(
                (port_matches(base_ii, &name), port_matches(base, &name)),
                (id == BASE_II_ID, id == BASE_ID),
                "{raw}"
            );
            assert_eq!(identify_midi(&name, false).map(|model| model.id), Some(id));
        }
        for raw in [
            "Base",
            "Base Input",
            "MIDIIN2 (Base8)",
            "Base2:Base2 MIDI 2 24:1",
            "Base2 MIDI 2",
            "Base22",
            "Midi Through:Midi Through Port-0 14:0",
        ] {
            let name = normalise(raw);
            assert!(
                !port_matches(base_ii, &name) && !port_matches(base, &name),
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_names_every_key() {
        for id in [BASE_ID, BASE_II_ID] {
            let model = by_id(id).unwrap();
            assert_eq!(model.dial_light_colors(), 0);
            let view = ModelView::from(model);
            let names = view.key_names.unwrap();
            assert_eq!(names.len(), 48);
            assert_eq!(
                (names[0], names[8], names[39], names[40], names[47]),
                ("Touch 1", "Pad 1", "Pad 32", "Function 1", "Function 8")
            );
            assert_eq!(view.key_light_colors, "basic_rgb");
            assert_eq!(view.dial_lights, DialLights::None);
            assert!(view.key_lights && !view.brightness);
        }
    }
}
