//! Behringer's X-Touch Mini, over class-compliant USB MIDI ([`super::midi`]),
//! in its Mackie Control ("MC") mode.
//!
//! The X-Touch Mini lists one MIDI port each way, named `X-TOUCH MINI` (in
//! capitals). [`port_matches`] takes a name containing exactly that, so a
//! full-size X-Touch or an X-Touch Extender is never taken for it.
//!
//! The deck powers up in its Standard mode, where its two layers are switched
//! inside the device and its encoders send absolute values; Ryu sends `B0 7F
//! 01` when it opens the deck to select MC mode instead, where the encoders are
//! relative and Layer A and Layer B are two more buttons, then turns every
//! light off. When it lets go it turns them off again and sends `B0 7F 00`, so
//! the deck is back in Standard mode for other apps.
//!
//! In MC mode, every button is a note on MIDI channel 1, pressed by a Note On
//! above velocity 0 and released by velocity 0 (or a Note Off). The notes are
//! not in order: the top row is 89, 90 and 40-45, the bottom row 87, 88, 91,
//! 92, 86 and 93-95, Layer A 84 and Layer B 85. Pushing encoder `n` (0-7) is
//! note `32 + n`. Turning it is `CC 16 + n`, whose value is sign and magnitude:
//! 1-63 clockwise by that many, 65-127 anticlockwise by the value less 64. The
//! fader is a pitch bend (channel 9 on the references' units; any channel is
//! taken), absolute, with no motor: its first value only seeds it, and each
//! later one turns its dial a step per two units of the upper seven bits.
//!
//! Out, on channel 1: a button's LED is a Note On on its own note, velocity 0
//! off, 127 on (1 blinks, and Ryu does not use it); an encoder's 13-LED ring is
//! `CC 48 + n`, whose value is a style in the upper bits and a position in the
//! lower four: Ryu sends 0 (off) or `0x2B` (the fan style at its last
//! position, the whole ring lit). There is no brightness command, no
//! keep-alive and no SysEx.
//!
//! Facts from walfie/xtouchmini (Rust, MIT) and peterdikant/xair-remote
//! (Python, MIT), cross-checked against maartentamboer/X-Touch-Mini-FS2020
//! (MIT), the @zapperment/x-toucher and @dlid/pkv-xmca npm packages and
//! DrivenByMoss (LGPL-3.0); no code was copied. The USB id is not confirmed by
//! any of them, whether `0x2B` or `0x2C` fills the ring and whether MC mode
//! outlasts a power cycle are to be checked on hardware.

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const XTOUCH_MINI_ID: &str = "behringer-x-touch-mini";

/// Sixteen grid buttons (0-15), then Layer A (16) and Layer B (17).
pub const KEYS: u8 = 18;
/// Eight encoders (0-7), then the fader (8).
pub const DIALS: u8 = 9;
/// The encoders, each with a ring light; the fader has none.
pub const ENCODERS: u8 = 8;
const FADER_DIAL: u8 = 8;
/// The grid's width; Layer A and B sit in a ninth column.
const ROW: u8 = 8;
const COLS: u16 = 9;

/// Each key's note, in key order: the top row, the bottom row, Layer A, B.
const NOTES: [u8; KEYS as usize] = [
    89, 90, 40, 41, 42, 43, 44, 45, 87, 88, 91, 92, 86, 93, 94, 95, 84, 85,
];

const NAMES: [&str; KEYS as usize] = [
    "Button 1",
    "Button 2",
    "Button 3",
    "Button 4",
    "Button 5",
    "Button 6",
    "Button 7",
    "Button 8",
    "Button 9",
    "Button 10",
    "Button 11",
    "Button 12",
    "Button 13",
    "Button 14",
    "Button 15",
    "Button 16",
    "Layer A",
    "Layer B",
];

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const PITCH_BEND: u8 = 0xe0;
const PUSH_NOTE: u8 = 32;
const TURN_CC: u8 = 16;
const RING_CC: u8 = 48;
const MODE_CC: u8 = 0x7f;
const MACKIE_MODE: u8 = 1;
const STANDARD_MODE: u8 = 0;
/// A turn's value: the low six bits count, bit 6 says anticlockwise.
const TURN_BACK: u8 = 0x40;

const LED_OFF: u8 = 0;
const LED_ON: u8 = 0x7f;
const RING_OFF: u8 = 0;
/// The fan style (`0x20`) at its last position (11): the whole ring lit.
const RING_FULL: u8 = 0x2b;

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Fader units (of 128) per dial step: 64 steps from end to end.
const FADER_STEP: i16 = 2;
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// Whether a normalised port name is an X-Touch Mini's: `X-TOUCH MINI` in
/// capitals, and on Windows the first (only) interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.protocol == Protocol::BehringerXTouchMiniMidi
        && name.interface.unwrap_or(1) == 1
        && name.base.contains("X-TOUCH MINI")
}

/// The legend on each key: the buttons by number, then the two layer keys.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::BehringerXTouchMiniMidi).then(|| NAMES.to_vec())
}

/// Where each key sits on the 2 x 9 grid: the two button rows, with Layer A
/// and Layer B in the right-hand column.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    (model.protocol == Protocol::BehringerXTouchMiniMidi).then(|| {
        (0..KEYS)
            .map(|key| {
                let (row, col) = if key < 2 * ROW {
                    (key / ROW, key % ROW)
                } else {
                    (key - 2 * ROW, ROW)
                };
                u16::from(row) * COLS + u16::from(col)
            })
            .collect()
    })
}

/// The key a note presses.
fn key_of_note(note: u8) -> Option<u8> {
    (0u8..)
        .zip(NOTES)
        .find(|(_, candidate)| *candidate == note)
        .map(|(key, _)| key)
}

/// How far one turn message moves an encoder: sign and magnitude.
fn turn_delta(value: u8) -> i8 {
    let count = i8::try_from(value & (TURN_BACK - 1)).unwrap_or(0);
    if value & TURN_BACK == 0 {
        count
    } else {
        -count
    }
}

/// On for any colour that is not dark.
fn is_lit(color: Option<[u8; 3]>) -> bool {
    color.is_some_and(|color| color.iter().any(|channel| *channel >= DARK))
}

pub struct XTouchMini {
    transport: Box<dyn Transport>,
    /// MC mode selected and every light cleared since open.
    started: bool,
    /// Each key's LED velocity as staged, then as last sent.
    leds: [u8; KEYS as usize],
    sent_leds: [u8; KEYS as usize],
    /// Each encoder ring's value as staged, then as last sent.
    rings: [u8; ENCODERS as usize],
    sent_rings: [u8; ENCODERS as usize],
    held: [bool; KEYS as usize],
    pushed: [bool; ENCODERS as usize],
    /// The fader's last position (upper seven bits); `None` until it moves.
    fader: Option<u8>,
    /// Fader movement not yet turned into a whole step.
    rest: i16,
    parser: Parser,
}

impl XTouchMini {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            leds: [LED_OFF; KEYS as usize],
            sent_leds: [LED_OFF; KEYS as usize],
            rings: [RING_OFF; ENCODERS as usize],
            sent_rings: [RING_OFF; ENCODERS as usize],
            held: [false; KEYS as usize],
            pushed: [false; ENCODERS as usize],
            fader: None,
            rest: 0,
            parser: Parser::default(),
        }
    }

    /// MC mode, then every LED and ring off.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if self.started {
            return Ok(());
        }
        self.transport
            .write(&[CONTROL_CHANGE, MODE_CC, MACKIE_MODE])?;
        for note in NOTES {
            self.transport.write(&[NOTE_ON, note, LED_OFF])?;
        }
        for ring in 0..ENCODERS {
            self.transport
                .write(&[CONTROL_CHANGE, RING_CC + ring, RING_OFF])?;
        }
        self.sent_leds = [LED_OFF; KEYS as usize];
        self.sent_rings = [RING_OFF; ENCODERS as usize];
        self.started = true;
        Ok(())
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

    fn push(&mut self, dial: u8, pressed: bool, events: &mut Vec<Input>) {
        let Some(pushed) = self.pushed.get_mut(usize::from(dial)) else {
            return;
        };
        if *pushed != pressed {
            *pushed = pressed;
            events.push(Input::DialPress {
                index: dial,
                pressed,
            });
        }
    }

    /// The fader's absolute position: the first seeds it, each later one
    /// turns its dial a step per two units moved.
    fn move_fader(&mut self, value: u8, events: &mut Vec<Input>) {
        let Some(old) = self.fader.replace(value) else {
            return;
        };
        let moved = self.rest + i16::from(value) - i16::from(old);
        let steps = moved / FADER_STEP;
        self.rest = moved - steps * FADER_STEP;
        if steps != 0 {
            events.push(Input::DialTurn {
                index: FADER_DIAL,
                delta: i8::try_from(steps.clamp(-127, 127)).unwrap_or(0),
            });
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        // SysEx is not used.
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        if status & 0xf0 == PITCH_BEND {
            // `number` is the low seven bits; the upper seven are enough.
            if value <= 0x7f {
                self.move_fader(value, events);
            }
            return;
        }
        if status & 0x0f != 0 {
            return;
        }
        match status & 0xf0 {
            NOTE_ON | NOTE_OFF => {
                let pressed = status & 0xf0 == NOTE_ON && value > 0;
                if let Some(dial) = number
                    .checked_sub(PUSH_NOTE)
                    .filter(|dial| *dial < ENCODERS)
                {
                    self.push(dial, pressed, events);
                } else if let Some(key) = key_of_note(number) {
                    self.press(key, pressed, events);
                }
            }
            CONTROL_CHANGE => {
                if let Some(dial) = number.checked_sub(TURN_CC).filter(|dial| *dial < ENCODERS) {
                    let delta = turn_delta(value);
                    if delta != 0 {
                        events.push(Input::DialTurn { index: dial, delta });
                    }
                }
            }
            _ => {}
        }
    }
}

impl Driver for XTouchMini {
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
        if let Some(led) = self.leds.get_mut(usize::from(key)) {
            *led = if is_lit(face.map(|face| face.color)) {
                LED_ON
            } else {
                LED_OFF
            };
        }
        Ok(())
    }

    /// The encoder rings; the fader (dial 9) has no light.
    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if let Some(ring) = self.rings.get_mut(usize::from(dial)) {
            *ring = if is_lit(color) { RING_FULL } else { RING_OFF };
        }
        Ok(())
    }

    /// MC mode the first time, then one message for each LED or ring that
    /// changed.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        for ((note, led), sent) in NOTES.iter().zip(self.leds).zip(&mut self.sent_leds) {
            if *sent != led {
                self.transport.write(&[NOTE_ON, *note, led])?;
                *sent = led;
            }
        }
        for ((cc, ring), sent) in (RING_CC..).zip(self.rings).zip(&mut self.sent_rings) {
            if *sent != ring {
                self.transport.write(&[CONTROL_CHANGE, cc, ring])?;
                *sent = ring;
            }
        }
        Ok(())
    }

    /// The X-Touch Mini has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for XTouchMini {
    /// Turns every light off and puts the deck back in Standard mode.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for note in NOTES {
            let _ = self.transport.write(&[NOTE_ON, note, LED_OFF]);
        }
        for ring in 0..ENCODERS {
            let _ = self
                .transport
                .write(&[CONTROL_CHANGE, RING_CC + ring, RING_OFF]);
        }
        let _ = self
            .transport
            .write(&[CONTROL_CHANGE, MODE_CC, STANDARD_MODE]);
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, ModelView};

    /// What opening sends: MC mode, 18 LEDs off, 8 rings off.
    const INIT: usize = 1 + 18 + 8;

    fn open() -> (XTouchMini, Recorder) {
        let recorder = Recorder::default();
        let deck = XTouchMini::new(by_id(XTOUCH_MINI_ID).unwrap(), Box::new(recorder.clone()));
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
    fn opening_selects_mc_mode_and_clears_every_light() {
        let (mut deck, recorder) = open();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), INIT);
        assert_eq!(writes[0], vec![0xb0, 0x7f, 0x01]);
        let notes: Vec<u8> = writes[1..19]
            .iter()
            .map(|write| {
                assert_eq!((write[0], write[2]), (0x90, 0));
                write[1]
            })
            .collect();
        assert_eq!(
            notes,
            vec![
                0x59, 0x5a, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x57, 0x58, 0x5b, 0x5c, 0x56, 0x5d,
                0x5e, 0x5f, 0x54, 0x55
            ]
        );
        for (ring, write) in writes[19..].iter().enumerate() {
            assert_eq!(*write, vec![0xb0, 0x30 + ring as u8, 0]);
        }
        // Nothing more until something changes.
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), INIT);
    }

    #[test]
    fn faces_and_dial_lights_reach_the_deck_on_flush() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        deck.set_face(0, Some(&lit([255, 128, 0]))).unwrap();
        deck.set_face(12, Some(&lit([0, 0, 200]))).unwrap();
        deck.set_face(17, Some(&lit([255, 255, 255]))).unwrap();
        // Black, near-black and no face stay off.
        deck.set_face(1, Some(&lit([0, 0, 0]))).unwrap();
        deck.set_face(2, Some(&lit([20, 20, 20]))).unwrap();
        deck.set_face(3, None).unwrap();
        // Out of range.
        deck.set_face(18, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_dial_light(0, Some([255, 0, 0])).unwrap();
        deck.set_dial_light(7, Some([0, 255, 0])).unwrap();
        deck.set_dial_light(3, None).unwrap();
        // The fader has no light.
        deck.set_dial_light(8, Some([255, 255, 255])).unwrap();
        assert_eq!(recorder.writes().len(), INIT);
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[INIT..],
            [
                vec![0x90, 0x59, 0x7f],
                vec![0x90, 0x56, 0x7f],
                vec![0x90, 0x55, 0x7f],
                vec![0xb0, 0x30, 0x2b],
                vec![0xb0, 0x37, 0x2b],
            ]
        );
        deck.set_face(0, None).unwrap();
        deck.set_face(12, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_dial_light(7, Some([10, 0, 0])).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[INIT + 5..],
            [vec![0x90, 0x59, 0x00], vec![0xb0, 0x37, 0x00]]
        );
        // Brightness changes nothing.
        deck.set_brightness(10).unwrap();
        assert_eq!(recorder.writes().len(), INIT + 7);
    }

    #[test]
    fn buttons_and_encoder_pushes_map_to_keys_and_dials() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 0x59, 0x7f],
                &[0x90, 0x59, 0x00],
                &[0x90, 0x2d, 0x7f],
                &[0x80, 0x2d, 0x40],
                &[0x90, 0x57, 0x7f],
                &[0x90, 0x56, 0x7f],
                &[0x90, 0x5f, 0x7f],
                &[0x90, 0x54, 0x7f],
                &[0x90, 0x55, 0x7f],
                &[0x90, 0x55, 0x00],
                &[0x90, 0x20, 0x7f],
                &[0x90, 0x20, 0x00],
                &[0x90, 0x27, 0x7f],
                &[0x80, 0x27, 0x00],
                // A repeated press, unknown notes, another channel, SysEx.
                &[0x90, 0x54, 0x7f],
                &[0x90, 0x30, 0x7f],
                &[0x90, 0x00, 0x7f],
                &[0x91, 0x59, 0x7f],
                &[0xf0, 0x00, 0x20, 0x32, 0xf7],
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
                key(7, true),
                key(7, false),
                key(8, true),
                key(12, true),
                key(15, true),
                key(16, true),
                key(17, true),
                key(17, false),
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                Input::DialPress {
                    index: 0,
                    pressed: false
                },
                Input::DialPress {
                    index: 7,
                    pressed: true
                },
                Input::DialPress {
                    index: 7,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn encoder_turns_are_sign_and_magnitude() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 0x10, 0x01],
                &[0xb0, 0x10, 0x0a],
                &[0xb0, 0x13, 0x41],
                &[0xb0, 0x17, 0x48],
                &[0xb0, 0x17, 0x7f],
                &[0xb0, 0x11, 0x3f],
                // No movement, a ring's CC, the mode CC, another channel.
                &[0xb0, 0x10, 0x00],
                &[0xb0, 0x10, 0x40],
                &[0xb0, 0x30, 0x05],
                &[0xb0, 0x7f, 0x01],
                &[0xb1, 0x10, 0x01],
            ],
        );
        let turn = |index, delta| Input::DialTurn { index, delta };
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, 1),
                turn(0, 10),
                turn(3, -1),
                turn(7, -8),
                turn(7, -63),
                turn(1, 63),
            ]
        );
    }

    #[test]
    fn the_fader_turns_dial_9_after_its_first_value() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xe8, 0x00, 0x40],
                &[0xe8, 0x00, 0x44],
                &[0xe8, 0x7f, 0x45],
                &[0xe8, 0x00, 0x46],
                &[0xe0, 0x00, 0x00],
                &[0xe8, 0x00, 0x7f],
                // A note or CC on the fader's channel is not the fader.
                &[0x98, 0x59, 0x7f],
                &[0xb8, 0x10, 0x01],
            ],
        );
        let turn = |delta| Input::DialTurn { index: 8, delta };
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(2), turn(1), turn(-35), turn(63)]
        );
    }

    #[test]
    fn letting_go_clears_the_lights_and_restores_standard_mode() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes.len(), 18 + 8 + 1);
        assert_eq!(writes[0], vec![0x90, 0x59, 0x00]);
        assert_eq!(writes[17], vec![0x90, 0x55, 0x00]);
        assert_eq!(writes[18], vec![0xb0, 0x30, 0x00]);
        assert_eq!(writes[26], vec![0xb0, 0x7f, 0x00]);
        // Never opened: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_the_xtouch_minis_port_matches() {
        let model = by_id(XTOUCH_MINI_ID).unwrap();
        let cases = [
            ("X-TOUCH MINI", false, true),
            ("X-TOUCH MINI", true, true),
            ("X-TOUCH MINI:X-TOUCH MINI MIDI 1 24:0", false, true),
            ("MIDIIN2 (X-TOUCH MINI)", true, false),
            // A full-size X-Touch or an Extender, and an unrelated port.
            ("X-TOUCH MIDI 1", false, false),
            ("X-Touch-Ext", false, false),
            ("X-Touch", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(XTOUCH_MINI_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_places_every_key() {
        let view = ModelView::from(by_id(XTOUCH_MINI_ID).unwrap());
        let cells = view.key_cells.unwrap();
        assert_eq!(
            cells,
            vec![0, 1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15, 16, 8, 17]
        );
        let names = view.key_names.unwrap();
        assert_eq!(
            (names[0], names[15], names[16], names[17]),
            ("Button 1", "Button 16", "Layer A", "Layer B")
        );
        assert_eq!((view.keys, view.extra_keys, view.dials), (18, 2, 9));
        assert!(view.lit_keys.is_none());
    }

    #[test]
    fn every_key_has_its_own_note() {
        for key in 0..KEYS {
            assert_eq!(key_of_note(NOTES[usize::from(key)]), Some(key));
            assert!(!(PUSH_NOTE..PUSH_NOTE + ENCODERS).contains(&NOTES[usize::from(key)]));
        }
        assert_eq!(key_of_note(0), None);
        assert_eq!(turn_delta(0x40), 0);
    }
}
