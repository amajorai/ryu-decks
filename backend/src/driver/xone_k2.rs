//! Allen & Heath's Xone:K2, over class-compliant USB MIDI ([`super::midi`]).
//!
//! The port is found by name ([`port_matches`]): `XONE:K2` in any case in the
//! normalised name, on Windows the first (or an unnumbered) interface. No
//! reference spells out the full name each host gives the port; Mixxx's
//! learned controller id for the deck is `XONE:K2` and its manual names the
//! device `XONE:K2`, so the match is inferred, as for the Novation Dicer. The
//! Xone:K1 and K3 share Mixxx's map but no reference names their ports, so
//! they are not matched.
//!
//! Everything is a three-byte channel message on the one MIDI channel the
//! unit is set to at power-on (hold the bottom-right encoder while plugging
//! it in); the factory default is channel 15 (nibble `E`). Ryu takes the
//! channel of the first note or controller it reads, and until then writes on
//! channel 15. Units chained by X-Link share the port on other channels; once
//! a channel is taken, the others are ignored. Nothing tells the USB-attached
//! unit apart, so the first unit heard wins (the docs tell users to touch it
//! first). Latching Layers must be off
//! (the factory default), or the button notes shift with the layer.
//!
//! In: Note On (`9n`) presses and Note Off (`8n`) or velocity 0 releases.
//! Column `c` (0-3, left to right) of grid row `r` (0-7, top down) is note
//! `34 - 4r + c` and key `4r + c`: row 0 the top encoder pushes (`34`-`37`),
//! rows 1-3 the three button rows under the knobs (`30`-`3B` down to
//! `28`-`2B`), rows 4-7 the bottom 4 x 4 matrix (`24`-`27` down to
//! `18`-`1B`). Layer is note `0C` (key 32) and Exit Setup note `0F` (key 33);
//! the bottom-left and bottom-right encoder pushes, notes `0D` and `0E`, press
//! dials 21 and 22. Controllers: `CC 00`-`03` are the top encoders (dials
//! 1-4) and `CC 14`/`15` the bottom ones (dials 21, 22), relative: `01` is one
//! step clockwise (attested); `40`-`7F` are read as two's-complement steps
//! anticlockwise (`7F` = -1), an inference since Mixxx only tells `01` from
//! every other value, and any other value is at least one step anticlockwise.
//! `CC 04`-`0F` are the twelve knobs row by row (dials 5-16) and `CC 10`-`13`
//! the four faders (dials 17-20), absolute 0-127: the unit cannot report
//! their positions, so each one's first value only seeds it and later ones
//! turn its dial by the difference.
//!
//! Out, on the taken channel: every grid key and Layer has a tri-colour LED
//! (red, amber or green, no dimming). A grid key's LED is a Note On at its
//! input note plus 0 (red), 36 (amber) or 72 (green) with velocity `7F`, and
//! a Note Off `8n <note> 7F` on the input note turns it off. Mixxx changes
//! colour by sending the new colour note alone; Ryu sends the Note Off first,
//! which is also safe. Layer's colours are notes `0C` (red), `10` (amber) and
//! `14` (green), each lit by Note On `7F` and replacing the last; only `9n 0C
//! 00` is attested to turn it off, so from amber or green Ryu first lights red
//! and then sends that. Exit Setup is only on (`9n 0F 7F`) or off (`8n 0F
//! 00`), as Mixxx sends it; its colour is not known. A colour picks off, red,
//! amber or green; there is no brightness. The faders are not motorised and
//! nothing is ever written to their controllers. There is no handshake or
//! keep-alive. The first poll sends every light (off unless set), later
//! flushes only the changed ones, all again when the channel is taken from
//! the deck, and every light goes off when Ryu lets go.
//!
//! Facts from Mixxx's Allen & Heath Xone K2 mapping and script and the Mixxx
//! manual's Xone K2/K1 page (GPL-2.0-or-later; CC BY-SA 4.0) and Mixxx's
//! udev rules for the vendor id `22f0`; no code was copied.

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const XONE_K2_ID: &str = "allen-heath-xone-k2";

/// Four columns; eight grid rows and a last row for Layer and Exit Setup.
pub const COLS: u8 = 4;
pub const ROWS: u8 = 9;
const GRID_ROWS: u8 = 8;
const GRID_KEYS: u8 = GRID_ROWS * COLS;
pub const LAYER_KEY: u8 = GRID_KEYS;
pub const EXIT_KEY: u8 = GRID_KEYS + 1;
pub const KEYS: u8 = GRID_KEYS + 2;
pub const EXTRA_KEYS: u8 = 2;
/// Top encoders 1-4, knobs 5-16, faders 17-20, bottom encoders 21-22.
pub const DIALS: u8 = 22;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// MIDI channel 15, the factory default.
const DEFAULT_CHANNEL: u8 = 0x0e;

/// The top-left encoder push; each grid row down is four notes lower.
const TOP_NOTE: u8 = 0x34;
const LOWEST_GRID_NOTE: u8 = 0x18;
const LAST_GRID_NOTE: u8 = 0x37;
const LAYER_NOTE: u8 = 0x0c;
const LEFT_PUSH_NOTE: u8 = 0x0d;
const RIGHT_PUSH_NOTE: u8 = 0x0e;
const EXIT_NOTE: u8 = 0x0f;
const LAYER_AMBER_NOTE: u8 = 0x10;
const LAYER_GREEN_NOTE: u8 = 0x14;
/// Dials pressed by the bottom encoder pushes.
const LEFT_ENCODER_DIAL: u8 = 20;
const RIGHT_ENCODER_DIAL: u8 = 21;

/// Grid colour offsets from a key's input note.
const AMBER_OFFSET: u8 = 36;
const GREEN_OFFSET: u8 = 72;
const LED_ON: u8 = 0x7f;

/// The last controller read: `CC 15`, the bottom-right encoder.
const LAST_CC: u8 = 0x15;
const FIRST_KNOB_CC: u8 = 0x04;
const LAST_FADER_CC: u8 = 0x13;

/// Below this level (of 255) a light is off.
const DARK_BELOW: u8 = 32;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// What one LED shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Light {
    Off,
    Red,
    Amber,
    Green,
}

/// Whether a MIDI port, by its normalised name, is the Xone:K2's: `XONE:K2`
/// in any case, on Windows the first (or an unnumbered) interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.protocol == Protocol::AllenHeathXoneK2Midi
        && name.interface.unwrap_or(1) == 1
        && name.base.to_ascii_uppercase().contains("XONE:K2")
}

const KEY_NAMES: [&str; KEYS as usize] = [
    "Encoder 1",
    "Encoder 2",
    "Encoder 3",
    "Encoder 4",
    "Top 1",
    "Top 2",
    "Top 3",
    "Top 4",
    "Top 5",
    "Top 6",
    "Top 7",
    "Top 8",
    "Top 9",
    "Top 10",
    "Top 11",
    "Top 12",
    "Bottom 1",
    "Bottom 2",
    "Bottom 3",
    "Bottom 4",
    "Bottom 5",
    "Bottom 6",
    "Bottom 7",
    "Bottom 8",
    "Bottom 9",
    "Bottom 10",
    "Bottom 11",
    "Bottom 12",
    "Bottom 13",
    "Bottom 14",
    "Bottom 15",
    "Bottom 16",
    "Layer",
    "Exit Setup",
];

/// The name Ryu gives each key: the top encoder pushes, the three button
/// rows under the knobs, the bottom matrix, Layer and Exit Setup.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::AllenHeathXoneK2Midi).then(|| KEY_NAMES.to_vec())
}

/// The grid in reading order, then Layer at the bottom left and Exit Setup
/// at the bottom right, as on the deck.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    (model.protocol == Protocol::AllenHeathXoneK2Midi).then(|| {
        let last_row = u16::from(GRID_KEYS);
        (0..u16::from(GRID_KEYS))
            .chain([last_row, last_row + u16::from(COLS) - 1])
            .collect()
    })
}

/// The input note of a grid key.
const fn grid_note(key: u8) -> u8 {
    TOP_NOTE - 4 * (key / COLS) + key % COLS
}

/// The grid key a note presses.
const fn grid_key(note: u8) -> Option<u8> {
    if note < LOWEST_GRID_NOTE || note > LAST_GRID_NOTE {
        return None;
    }
    let from_bottom = (note - LOWEST_GRID_NOTE) / COLS;
    Some((GRID_ROWS - 1 - from_bottom) * COLS + note % COLS)
}

/// A colour as one of the LED's states: lit only when its red or green is
/// bright enough, then green or red when one clearly outweighs the other,
/// otherwise amber. Blue alone is off.
fn light_of(color: [u8; 3]) -> Light {
    let [red, green, _] = color;
    if red.max(green) < DARK_BELOW {
        return Light::Off;
    }
    let (red, green) = (u16::from(red), u16::from(green));
    if green * 2 > red * 3 {
        Light::Green
    } else if red * 2 > green * 3 {
        Light::Red
    } else {
        Light::Amber
    }
}

/// A relative encoder value as steps: `01`-`3F` clockwise, `40`-`7F`
/// anticlockwise in two's complement, and `00` one step anticlockwise.
fn encoder_steps(value: u8) -> i8 {
    match value {
        1..=63 => i8::try_from(value).unwrap_or(1),
        64..=127 => i8::try_from(i16::from(value) - 128).unwrap_or(-1),
        _ => -1,
    }
}

pub struct XoneK2 {
    transport: Box<dyn Transport>,
    /// Every light sent once since open.
    started: bool,
    /// The channel (0-15) read from and written to.
    channel: u8,
    /// Whether `channel` was taken from the deck.
    adopted: bool,
    colors: [[u8; 3]; KEYS as usize],
    /// What each light was last sent; `None` until it is.
    sent: [Option<Light>; KEYS as usize],
    held: [bool; KEYS as usize],
    pushed: [bool; 2],
    /// Each control's last absolute value; `None` until it first arrives.
    values: [Option<u8>; DIALS as usize],
    parser: Parser,
}

impl XoneK2 {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            channel: DEFAULT_CHANNEL,
            adopted: false,
            colors: [BLACK; KEYS as usize],
            sent: [None; KEYS as usize],
            held: [false; KEYS as usize],
            pushed: [false; 2],
            values: [None; DIALS as usize],
            parser: Parser::default(),
        }
    }

    /// Writes one key's light, knowing what it showed last (`None`: unknown).
    fn write_light(&mut self, key: u8, light: Light, last: Option<Light>) -> DriverResult<()> {
        let on = NOTE_ON | self.channel;
        let off = NOTE_OFF | self.channel;
        match key {
            LAYER_KEY => {
                let note = match light {
                    Light::Off => {
                        if last != Some(Light::Red) {
                            // Only red's off is attested: replace the colour
                            // with red first.
                            self.transport.write(&[on, LAYER_NOTE, LED_ON])?;
                        }
                        return self.transport.write(&[on, LAYER_NOTE, 0]);
                    }
                    Light::Red => LAYER_NOTE,
                    Light::Amber => LAYER_AMBER_NOTE,
                    Light::Green => LAYER_GREEN_NOTE,
                };
                self.transport.write(&[on, note, LED_ON])
            }
            EXIT_KEY => {
                if light == Light::Off {
                    self.transport.write(&[off, EXIT_NOTE, 0])
                } else {
                    self.transport.write(&[on, EXIT_NOTE, LED_ON])
                }
            }
            _ => {
                let note = grid_note(key);
                self.transport.write(&[off, note, LED_ON])?;
                let offset = match light {
                    Light::Off => return Ok(()),
                    Light::Red => 0,
                    Light::Amber => AMBER_OFFSET,
                    Light::Green => GREEN_OFFSET,
                };
                self.transport.write(&[on, note + offset, LED_ON])
            }
        }
    }

    fn press(&mut self, key: u8, pressed: bool, events: &mut Vec<Input>) {
        if let Some(held) = self.held.get_mut(usize::from(key)) {
            if *held != pressed {
                *held = pressed;
                events.push(Input::Key {
                    index: key,
                    pressed,
                });
            }
        }
    }

    fn push(&mut self, side: usize, pressed: bool, events: &mut Vec<Input>) {
        let dial = if side == 0 {
            LEFT_ENCODER_DIAL
        } else {
            RIGHT_ENCODER_DIAL
        };
        if let Some(held) = self.pushed.get_mut(side) {
            if *held != pressed {
                *held = pressed;
                events.push(Input::DialPress {
                    index: dial,
                    pressed,
                });
            }
        }
    }

    fn note(&mut self, note: u8, pressed: bool, events: &mut Vec<Input>) {
        match note {
            LAYER_NOTE => self.press(LAYER_KEY, pressed, events),
            EXIT_NOTE => self.press(EXIT_KEY, pressed, events),
            LEFT_PUSH_NOTE => self.push(0, pressed, events),
            RIGHT_PUSH_NOTE => self.push(1, pressed, events),
            _ => {
                if let Some(key) = grid_key(note) {
                    self.press(key, pressed, events);
                }
            }
        }
    }

    fn control(&mut self, cc: u8, value: u8, events: &mut Vec<Input>) {
        if cc > LAST_CC {
            return;
        }
        let delta = if (FIRST_KNOB_CC..=LAST_FADER_CC).contains(&cc) {
            let Some(last) = self.values.get_mut(usize::from(cc)) else {
                return;
            };
            let Some(old) = last.replace(value) else {
                return;
            };
            let moved = i16::from(value) - i16::from(old);
            if moved == 0 {
                return;
            }
            i8::try_from(moved.clamp(-127, 127)).unwrap_or(0)
        } else {
            encoder_steps(value)
        };
        // Dial indices follow the controller numbers.
        events.push(Input::DialTurn { index: cc, delta });
    }

    /// Handles one message; returns whether it made Ryu take a new channel.
    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> bool {
        let Message::Channel([status, number, value]) = *message else {
            return false;
        };
        let kind = status & 0xf0;
        if !matches!(kind, NOTE_ON | NOTE_OFF | CONTROL_CHANGE) {
            return false;
        }
        let channel = status & 0x0f;
        let mut moved = false;
        if !self.adopted {
            self.adopted = true;
            moved = channel != self.channel;
            self.channel = channel;
        } else if channel != self.channel {
            return false;
        }
        if kind == CONTROL_CHANGE {
            self.control(number, value, events);
        } else {
            self.note(number, kind == NOTE_ON && value > 0, events);
        }
        moved
    }
}

impl Driver for XoneK2 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
            self.flush()?;
        }
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
        let mut events = Vec::new();
        let mut moved = false;
        for message in &messages {
            moved |= self.receive(message, &mut events);
        }
        if moved {
            // The lights went to channel 15; send them all on the deck's.
            self.sent = [None; KEYS as usize];
            self.flush()?;
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// The lights whose state changed (every light the first time).
    fn flush(&mut self) -> DriverResult<()> {
        self.started = true;
        for key in 0..KEYS {
            let index = usize::from(key);
            let mut light = light_of(self.colors[index]);
            if key == EXIT_KEY && light != Light::Off {
                light = Light::Red;
            }
            let last = self.sent[index];
            if last != Some(light) {
                self.write_light(key, light, last)?;
                self.sent[index] = Some(light);
            }
        }
        Ok(())
    }

    /// The LEDs have no dimming.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for XoneK2 {
    /// Turns every light off: the deck would otherwise keep them.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for key in 0..KEYS {
            let last = self.sent[usize::from(key)];
            if last != Some(Light::Off) {
                let _ = self.write_light(key, Light::Off, last);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, Family, ModelView};

    fn fresh() -> (XoneK2, Recorder) {
        let recorder = Recorder::default();
        let deck = XoneK2::new(by_id(XONE_K2_ID).unwrap(), Box::new(recorder.clone()));
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

    fn turn(index: u8, delta: i8) -> Input {
        Input::DialTurn { index, delta }
    }

    #[test]
    fn the_grid_notes_run_down_from_the_top_encoder_pushes() {
        assert_eq!(
            (grid_note(0), grid_note(3), grid_note(4), grid_note(15)),
            (0x34, 0x37, 0x30, 0x2b)
        );
        assert_eq!(
            (grid_note(16), grid_note(28), grid_note(31)),
            (0x24, 0x18, 0x1b)
        );
        for key in 0..GRID_KEYS {
            assert_eq!(grid_key(grid_note(key)), Some(key));
        }
        assert_eq!(grid_key(0x17), None);
        assert_eq!(grid_key(0x38), None);
    }

    #[test]
    fn opening_turns_every_light_off_on_channel_15() {
        let (mut deck, recorder) = fresh();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(16, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(31, Some(&lit([255, 200, 0]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        // 32 grid offs, three grid ons, Layer red then off, Exit Setup off.
        assert_eq!(writes.len(), 32 + 3 + 2 + 1);
        assert_eq!(writes[0], vec![0x8e, 0x34, 0x7f]);
        assert_eq!(writes[1], vec![0x9e, 0x34, 0x7f]);
        assert_eq!(writes[2], vec![0x8e, 0x35, 0x7f]);
        assert!(writes.contains(&vec![0x9e, 0x24 + 72, 0x7f]));
        assert!(writes.contains(&vec![0x9e, 0x1b + 36, 0x7f]));
        assert_eq!(
            writes[writes.len() - 3..],
            [
                vec![0x9e, 0x0c, 0x7f],
                vec![0x9e, 0x0c, 0x00],
                vec![0x8e, 0x0f, 0x00],
            ]
        );
        // Nothing changed: nothing sent.
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), writes.len());
        // Never a controller message: the faders are not motorised.
        assert!(writes.iter().all(|write| write[0] & 0xf0 != 0xb0));
    }

    #[test]
    fn a_colour_change_clears_the_base_note_then_lights_the_new_one() {
        let (mut deck, recorder) = fresh();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_face(5, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        deck.set_face(5, Some(&lit([255, 255, 0]))).unwrap();
        deck.flush().unwrap();
        deck.set_face(5, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        deck.set_face(5, None).unwrap();
        deck.set_face(200, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [
                vec![0x8e, 0x31, 0x7f],
                vec![0x9e, 0x31, 0x7f],
                vec![0x8e, 0x31, 0x7f],
                vec![0x9e, 0x31 + 36, 0x7f],
                vec![0x8e, 0x31, 0x7f],
                vec![0x9e, 0x31 + 72, 0x7f],
                vec![0x8e, 0x31, 0x7f],
            ]
        );
    }

    #[test]
    fn layer_takes_its_own_colour_notes_and_exit_setup_is_on_or_off() {
        let (mut deck, recorder) = fresh();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        for color in [[0, 255, 0], [255, 200, 0], [255, 0, 0], [0, 0, 0]] {
            deck.set_face(LAYER_KEY, Some(&lit(color))).unwrap();
            deck.flush().unwrap();
        }
        deck.set_face(LAYER_KEY, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        deck.set_face(LAYER_KEY, None).unwrap();
        deck.flush().unwrap();
        deck.set_face(EXIT_KEY, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        deck.set_face(EXIT_KEY, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        deck.set_face(EXIT_KEY, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [
                vec![0x9e, 0x14, 0x7f],
                vec![0x9e, 0x10, 0x7f],
                vec![0x9e, 0x0c, 0x7f],
                vec![0x9e, 0x0c, 0x00],
                vec![0x9e, 0x14, 0x7f],
                vec![0x9e, 0x0c, 0x7f],
                vec![0x9e, 0x0c, 0x00],
                vec![0x9e, 0x0f, 0x7f],
                vec![0x8e, 0x0f, 0x00],
            ]
        );
        // Never a Note Off on Layer's amber or green notes.
        assert!(!recorder
            .writes()
            .iter()
            .any(|write| write[0] & 0xf0 == 0x80 && matches!(write[1], 0x10 | 0x14)));
    }

    #[test]
    fn colours_fall_to_off_red_amber_or_green() {
        assert_eq!(light_of([0, 0, 0]), Light::Off);
        assert_eq!(light_of([0, 0, 255]), Light::Off, "blue alone");
        assert_eq!(light_of([20, 31, 255]), Light::Off);
        assert_eq!(light_of([255, 0, 0]), Light::Red);
        assert_eq!(light_of([255, 160, 0]), Light::Red);
        assert_eq!(light_of([255, 170, 0]), Light::Amber);
        assert_eq!(light_of([255, 255, 0]), Light::Amber);
        assert_eq!(light_of([255, 255, 255]), Light::Amber);
        assert_eq!(light_of([100, 200, 0]), Light::Green);
        assert_eq!(light_of([0, 40, 0]), Light::Green);
    }

    #[test]
    fn buttons_encoders_knobs_and_faders_map_to_keys_and_dials() {
        let (mut deck, recorder) = fresh();
        for message in [
            // Top-left encoder push, a bottom-matrix button (running status
            // release), Layer and Exit Setup.
            vec![0x9e, 0x34, 0x7f],
            vec![0x8e, 0x34, 0x7f],
            vec![0x9e, 0x1b, 0x7f, 0x1b, 0x00],
            vec![0x9e, 0x0c, 0x7f],
            vec![0x9e, 0x0c, 0x7f],
            vec![0x8e, 0x0c, 0x00],
            vec![0x9e, 0x0f, 0x7f],
            vec![0x8e, 0x0f, 0x00],
            // The bottom encoder pushes.
            vec![0x9e, 0x0d, 0x7f],
            vec![0x8e, 0x0d, 0x00],
            vec![0x9e, 0x0e, 0x7f],
            vec![0x8e, 0x0e, 0x00],
            // Top encoder 1 clockwise, encoder 4 anticlockwise, the bottom
            // encoders.
            vec![0xbe, 0x00, 0x01],
            vec![0xbe, 0x03, 0x7f],
            vec![0xbe, 0x14, 0x7e],
            vec![0xbe, 0x15, 0x00],
            // A knob and a fader: the first value seeds.
            vec![0xbe, 0x04, 0x40],
            vec![0xbe, 0x04, 0x45],
            vec![0xbe, 0x04, 0x45],
            vec![0xbe, 0x13, 0x7f],
            vec![0xbe, 0x13, 0x00],
            // Ignored: other channels once one is taken, unmapped notes and
            // controllers, short and SysEx messages.
            vec![0x9d, 0x34, 0x7f],
            vec![0xbd, 0x00, 0x01],
            vec![0x9e, 0x10, 0x7f],
            vec![0x9e, 0x38, 0x7f],
            vec![0xbe, 0x16, 0x01],
            vec![0xf0, 0x7e, 0x00, 0xf7],
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
                key(31, true),
                key(31, false),
                key(32, true),
                key(32, false),
                key(33, true),
                key(33, false),
                Input::DialPress {
                    index: 20,
                    pressed: true
                },
                Input::DialPress {
                    index: 20,
                    pressed: false
                },
                Input::DialPress {
                    index: 21,
                    pressed: true
                },
                Input::DialPress {
                    index: 21,
                    pressed: false
                },
                turn(0, 1),
                turn(3, -1),
                turn(20, -2),
                turn(21, -1),
                turn(4, 5),
                turn(19, -127),
            ]
        );
    }

    #[test]
    fn encoder_values_never_turn_nothing() {
        assert_eq!(encoder_steps(1), 1);
        assert_eq!(encoder_steps(63), 63);
        assert_eq!(encoder_steps(64), -64);
        assert_eq!(encoder_steps(127), -1);
        assert_eq!(encoder_steps(0), -1);
        for value in 0..=127 {
            assert_ne!(encoder_steps(value), 0);
        }
    }

    #[test]
    fn the_first_channel_heard_is_taken_and_the_lights_resent_there() {
        let (mut deck, recorder) = fresh();
        deck.set_face(0, Some(&lit([0, 255, 0]))).unwrap();
        deck.poll(0).unwrap();
        let before = recorder.writes().len();
        recorder.queue_read(vec![0x97, 0x30, 0x7f]);
        recorder.queue_read(vec![0x9e, 0x30, 0x7f]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(4, true)]);
        let writes = recorder.writes();
        assert_eq!(writes.len() - before, 32 + 1 + 2 + 1);
        assert_eq!(writes[before], vec![0x87, 0x34, 0x7f]);
        assert_eq!(writes[before + 1], vec![0x97, 0x34 + 72, 0x7f]);
        assert!(writes[before..].iter().all(|write| write[0] & 0x0f == 0x07));
        // Later input on the same channel resends nothing.
        recorder.queue_read(vec![0x87, 0x30, 0x7f]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(4, false)]);
        assert_eq!(recorder.writes().len(), writes.len());
    }

    #[test]
    fn letting_go_turns_every_light_off() {
        let (mut deck, recorder) = fresh();
        deck.set_face(2, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(LAYER_KEY, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(EXIT_KEY, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(EXIT_KEY, Some(&lit([255, 255, 255])))
            .unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        assert_eq!(
            recorder.writes()[before..],
            [
                vec![0x8e, 0x36, 0x7f],
                vec![0x9e, 0x0c, 0x7f],
                vec![0x9e, 0x0c, 0x00],
                vec![0x8e, 0x0f, 0x00],
            ]
        );
        let (deck, recorder) = fresh();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn the_port_is_found_by_xone_k2() {
        let model = by_id(XONE_K2_ID).unwrap();
        for raw in [
            "XONE:K2",
            "XONE:K2 MIDI 1",
            "XONE:K2:XONE:K2 MIDI 1 24:0",
            "Xone:K2",
            "2- XONE:K2",
        ] {
            assert!(port_matches(model, &normalise(raw)), "{raw}");
            assert_eq!(
                identify_midi(&normalise(raw), false).map(|model| model.id),
                Some(XONE_K2_ID),
                "{raw}"
            );
        }
        for raw in ["MIDIIN2 (XONE:K2)", "XONE:K1", "XONE:K3", "Xone K2", "XONE"] {
            assert!(!port_matches(model, &normalise(raw)), "{raw}");
        }
        let dicer = by_id(super::super::novation_dicer::DICER_ID).unwrap();
        assert!(!port_matches(dicer, &normalise("XONE:K2")));
    }

    #[test]
    fn the_row_names_and_places_34_keys_with_tri_colour_lights() {
        let model = by_id(XONE_K2_ID).unwrap();
        assert_eq!(model.family, Family::AllenHeath);
        assert!(model.protocol.is_midi() && !model.protocol.has_brightness());
        assert_eq!(model.dial_lights, DialLights::None);
        assert_eq!(model.dial_light_colors(), 0);
        assert!(model.product_ids.is_empty() && model.more_usb_ids.is_empty());
        assert_eq!((model.keys, model.extra_keys, model.dials), (34, 2, 22));
        let view = ModelView::from(model);
        assert_eq!(view.vendor_id, "22f0");
        assert!(view.key_lights && !view.brightness);
        assert_eq!(view.key_light_colors, "green_red_yellow");
        assert!(view.lit_keys.is_none());
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), usize::from(model.keys));
        assert_eq!(
            (names[0], names[4], names[16], names[32], names[33]),
            ("Encoder 1", "Top 1", "Bottom 1", "Layer", "Exit Setup")
        );
        let cells = view.key_cells.unwrap();
        assert_eq!(cells.len(), usize::from(model.keys));
        assert_eq!((cells[31], cells[32], cells[33]), (31, 32, 35));
        assert!(cells
            .iter()
            .all(|cell| *cell < u16::from(model.rows) * u16::from(model.cols)));
    }
}
