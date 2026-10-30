//! Behringer's X-Touch One, over class-compliant USB MIDI ([`super::midi`]),
//! in its standalone MIDI CC (or MIDI note) mode.
//!
//! The X-Touch One lists one MIDI port each way, whose name contains
//! `X-Touch One` (in any case). [`port_matches`] requires the `One`, so an
//! X-Touch Mini, a full-size X-Touch or an X-Touch Extender is never taken
//! for it.
//!
//! The user picks MIDI CC mode in the deck's own setup; in its Mackie
//! Control mode it speaks another protocol (pitch bend from the fader, its
//! own display SysEx), which Ryu does not drive. A pitch bend is the one
//! message that tells the two apart, and Ryu logs a hint when it sees one.
//!
//! In, on MIDI channel 1: each of the 35 buttons is Control Change (CC mode)
//! or Note On (note mode) on its own number 0-34, pressed at value 127 and
//! released at anything else (a Note Off releases too, defensively). Touching
//! the fader is number 110 the same way; Ryu makes it key 36, which has no
//! light. The encoder is `CC 80`: relative, 65 a step clockwise and 1 a step
//! anticlockwise; any other value means the deck sends it as an absolute
//! position, and from then on it is read as one (seeded by that value, then a
//! dial step per unit). The jog wheel is `CC 88`, 65 or 1 the same way, other
//! values ignored. The motor fader is `CC 70`, absolute 0 (bottom) to 127
//! (top): its first value only seeds it, and each later one turns its dial a
//! step per two units.
//!
//! Out, on channel 1: a button's LED is `B0 <n> <00 or 7F>` (40 blinks, and
//! Ryu does not use it). The 2 x 7 character backlit LCD is the SysEx
//! `F0 00 20 32 41 4C 00 <attr> <14 characters> F7`, whose attribute's low
//! three bits pick the backlight (0 black, which turns it off, 1 red, 2 green,
//! 3 yellow, 4 blue, 5 magenta, 6 cyan, 7 white); bits 4 and 5 would invert the
//! top and bottom line. Ryu writes its name on the top line and colours the
//! backlight with the deck light (white with none set). The 12-digit
//! 7-segment display is `F0 00 20 32 41 37 <12 segment bytes> <2 dot bytes>
//! F7`, blanked at open and close. The fader meter LED (`B0 5A <level>`, under
//! 8 off) is turned off; the motor fader (`B0 46`) and the encoder ring
//! (`B0 50`, which cannot be fully off) are never sent. There is no brightness
//! command and no keep-alive.
//!
//! Facts from the node-xtouch-one npm package (JavaScript, MIT), cross-checked
//! against DrivenByMoss (Java, LGPL-3.0) for the port name; no code was copied.
//! No USB id is given by either, and how relative turns faster than one step
//! are sent is not known.

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const XTOUCH_ONE_ID: &str = "behringer-x-touch-one";

/// The 35 buttons (0-34, by their own numbers), then the fader touch (35).
pub const KEYS: u8 = 36;
/// The encoder, the jog wheel and the fader.
pub const DIALS: u8 = 3;
/// The buttons with a light: all but the fader touch.
const BUTTONS: u8 = 35;
const FADER_TOUCH_KEY: u8 = 35;
const ENCODER_DIAL: u8 = 0;
const JOG_DIAL: u8 = 1;
const FADER_DIAL: u8 = 2;

const NAMES: [&str; KEYS as usize] = [
    "Segment Solo",
    "BPM",
    "Master",
    "Select",
    "Mute",
    "Ch Solo",
    "Ch Rec",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "Marker",
    "Nudge",
    "Cycle",
    "Drop",
    "Replace",
    "Click",
    "Solo",
    "Rewind",
    "Forward",
    "Stop",
    "Play",
    "Rec",
    "Fader Bank Prev",
    "Fader Bank Next",
    "Channel Prev",
    "Channel Next",
    "Scrub",
    "Up",
    "Left",
    "Enter",
    "Right",
    "Down",
    "Fader Touch",
];

/// The buttons with a light, for the Companion.
pub const LIT_KEYS: [u8; BUTTONS as usize] = {
    let mut keys = [0u8; BUTTONS as usize];
    let mut key = 0;
    while key < BUTTONS {
        keys[key as usize] = key;
        key += 1;
    }
    keys
};

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const PITCH_BEND: u8 = 0xe0;
const FADER_TOUCH: u8 = 110;
const FADER_CC: u8 = 70;
const ENCODER_CC: u8 = 80;
const JOG_CC: u8 = 88;
const FADER_LED_CC: u8 = 90;
const PRESSED: u8 = 127;
/// A relative turn: one step anticlockwise, or clockwise.
const LEFT: u8 = 1;
const RIGHT: u8 = 65;

const LED_OFF: u8 = 0;
const LED_ON: u8 = 0x7f;

/// SysEx start, Behringer's manufacturer id and the X-Touch One's device id.
const SYSEX_HEADER: [u8; 5] = [0xf0, 0x00, 0x20, 0x32, 0x41];
const SYSEX_END: u8 = 0xf7;
const LCD_COMMAND: [u8; 2] = [0x4c, 0x00];
const SEGMENT_COMMAND: u8 = 0x37;
const LCD_CHARS: usize = 14;
const LCD_LINE: usize = 7;
const DIGITS: usize = 12;
/// The top line Ryu writes; the bottom stays blank.
const LCD_TOP: &[u8; LCD_LINE] = b"  Ryu  ";
/// White: the backlight with no deck light set.
const WHITE: u8 = 7;
/// Black: the LCD off.
const LCD_OFF: u8 = 0;

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Half way up a channel: the backlight takes it.
const HALF: u8 = 128;
/// Fader units (of 128) per dial step: 64 steps from end to end.
const FADER_STEP: i16 = 2;
/// Absolute encoder units per dial step.
const ENCODER_STEP: i16 = 1;
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// Whether a normalised port name is an X-Touch One's: `X-TOUCH ONE` in any
/// case, and on Windows the first interface (assumed: the deck lists one).
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.protocol == Protocol::BehringerXTouchOneMidi
        && name.interface.unwrap_or(1) == 1
        && name.base.to_ascii_uppercase().contains("X-TOUCH ONE")
}

/// The legend on each key: the buttons' printed names, then the fader touch.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::BehringerXTouchOneMidi).then(|| NAMES.to_vec())
}

/// The keys with a light: every button, not the fader touch.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    (model.protocol == Protocol::BehringerXTouchOneMidi).then(|| LIT_KEYS.to_vec())
}

/// The key a button or touch number presses.
fn key_of(number: u8) -> Option<u8> {
    match number {
        FADER_TOUCH => Some(FADER_TOUCH_KEY),
        _ if number < BUTTONS => Some(number),
        _ => None,
    }
}

/// On for any colour that is not dark.
fn is_lit(color: Option<[u8; 3]>) -> bool {
    color.is_some_and(|color| color.iter().any(|channel| *channel >= DARK))
}

/// The backlight's three-bit colour: red, green and blue each on from half
/// way; white with no colour set.
fn backlight(color: Option<[u8; 3]>) -> u8 {
    color.map_or(WHITE, |[red, green, blue]| {
        u8::from(red >= HALF) | u8::from(green >= HALF) << 1 | u8::from(blue >= HALF) << 2
    })
}

/// The LCD message: `attr`, then exactly 14 characters (7 per line).
fn lcd_message(attr: u8, text: &[u8; LCD_CHARS]) -> Vec<u8> {
    let mut message = Vec::with_capacity(SYSEX_HEADER.len() + LCD_COMMAND.len() + 2 + LCD_CHARS);
    message.extend_from_slice(&SYSEX_HEADER);
    message.extend_from_slice(&LCD_COMMAND);
    message.push(attr & 0x37);
    message.extend(text.iter().map(|byte| {
        if (0x20..=0x7e).contains(byte) {
            *byte
        } else {
            b' '
        }
    }));
    message.push(SYSEX_END);
    message
}

/// The 7-segment display with every digit and dot off.
fn blank_segments() -> Vec<u8> {
    let mut message = Vec::with_capacity(SYSEX_HEADER.len() + 1 + DIGITS + 2 + 1);
    message.extend_from_slice(&SYSEX_HEADER);
    message.push(SEGMENT_COMMAND);
    message.extend([0u8; DIGITS + 2]);
    message.push(SYSEX_END);
    message
}

/// Ryu's text: its name on the top line, the bottom one blank.
fn ryu_text() -> [u8; LCD_CHARS] {
    let mut text = [b' '; LCD_CHARS];
    text[..LCD_LINE].copy_from_slice(LCD_TOP);
    text
}

/// An absolute control read as a dial: its first value seeds it, then every
/// `step` units moved turn the dial one step.
#[derive(Debug, Default)]
struct Absolute {
    last: Option<u8>,
    rest: i16,
}

impl Absolute {
    fn moved(&mut self, value: u8, step: i16) -> i8 {
        let Some(old) = self.last.replace(value) else {
            return 0;
        };
        let moved = self.rest + i16::from(value) - i16::from(old);
        let steps = moved / step;
        self.rest = moved - steps * step;
        i8::try_from(steps.clamp(-127, 127)).unwrap_or(0)
    }
}

pub struct XTouchOne {
    transport: Box<dyn Transport>,
    /// Every light, the LCD and the segments sent since open.
    started: bool,
    /// Each button's LED value as staged, then as last sent.
    leds: [u8; BUTTONS as usize],
    sent_leds: [u8; BUTTONS as usize],
    /// The LCD attribute (backlight colour) as staged, then as last sent.
    lcd: u8,
    sent_lcd: u8,
    held: [bool; KEYS as usize],
    /// The encoder has sent a value that is not a relative step, so it is
    /// read as an absolute position for the rest of the session.
    encoder_absolute: bool,
    encoder: Absolute,
    fader: Absolute,
    /// The Mackie Control hint has been logged.
    warned: bool,
    parser: Parser,
}

impl XTouchOne {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            leds: [LED_OFF; BUTTONS as usize],
            sent_leds: [LED_OFF; BUTTONS as usize],
            lcd: WHITE,
            sent_lcd: WHITE,
            held: [false; KEYS as usize],
            encoder_absolute: false,
            encoder: Absolute::default(),
            fader: Absolute::default(),
            warned: false,
            parser: Parser::default(),
        }
    }

    /// Every LED as staged, the LCD, a blank 7-segment display and the fader
    /// meter off.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if self.started {
            return Ok(());
        }
        for (number, led) in (0u8..).zip(self.leds) {
            self.transport.write(&[CONTROL_CHANGE, number, led])?;
        }
        self.sent_leds = self.leds;
        self.transport.write(&lcd_message(self.lcd, &ryu_text()))?;
        self.sent_lcd = self.lcd;
        self.transport.write(&blank_segments())?;
        self.transport
            .write(&[CONTROL_CHANGE, FADER_LED_CC, LED_OFF])?;
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

    fn turn_encoder(&mut self, value: u8) -> i8 {
        if !self.encoder_absolute {
            match value {
                LEFT => return -1,
                RIGHT => return 1,
                _ => self.encoder_absolute = true,
            }
        }
        self.encoder.moved(value, ENCODER_STEP)
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        // SysEx is not used.
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        if status & 0xf0 == PITCH_BEND {
            if !self.warned {
                self.warned = true;
                tracing::warn!(
                    "the X-Touch One sent a pitch bend, so it is in Mackie Control mode; \
                     select MIDI CC mode in its setup for Ryu to read it"
                );
            }
            return;
        }
        match status {
            NOTE_ON | NOTE_OFF => {
                if let Some(key) = key_of(number) {
                    self.press(key, status == NOTE_ON && value == PRESSED, events);
                }
            }
            CONTROL_CHANGE => {
                let (index, delta) = match number {
                    ENCODER_CC => (ENCODER_DIAL, self.turn_encoder(value)),
                    JOG_CC => (
                        JOG_DIAL,
                        match value {
                            LEFT => -1,
                            RIGHT => 1,
                            _ => 0,
                        },
                    ),
                    FADER_CC => (FADER_DIAL, self.fader.moved(value, FADER_STEP)),
                    _ => {
                        if let Some(key) = key_of(number) {
                            self.press(key, value == PRESSED, events);
                        }
                        return;
                    }
                };
                if delta != 0 {
                    events.push(Input::DialTurn { index, delta });
                }
            }
            _ => {}
        }
    }
}

impl Driver for XTouchOne {
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
        // The fader touch (key 35) has no light.
        if let Some(led) = self.leds.get_mut(usize::from(key)) {
            *led = if is_lit(face.map(|face| face.color)) {
                LED_ON
            } else {
                LED_OFF
            };
        }
        Ok(())
    }

    /// The LCD backlight, in the nearest of its eight colours; white with
    /// none set (black turns the LCD off).
    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        self.lcd = backlight(color);
        Ok(())
    }

    /// Everything the first time, then one message for each LED that changed
    /// and the LCD when its backlight did.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.started {
            return self.ensure_started();
        }
        for ((number, led), sent) in (0u8..).zip(self.leds).zip(&mut self.sent_leds) {
            if *sent != led {
                self.transport.write(&[CONTROL_CHANGE, number, led])?;
                *sent = led;
            }
        }
        if self.sent_lcd != self.lcd {
            self.transport.write(&lcd_message(self.lcd, &ryu_text()))?;
            self.sent_lcd = self.lcd;
        }
        Ok(())
    }

    /// The X-Touch One has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for XTouchOne {
    /// Turns every light, the fader meter and the displays off.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for number in 0..BUTTONS {
            let _ = self.transport.write(&[CONTROL_CHANGE, number, LED_OFF]);
        }
        let _ = self
            .transport
            .write(&[CONTROL_CHANGE, FADER_LED_CC, LED_OFF]);
        let _ = self.transport.write(&blank_segments());
        let _ = self
            .transport
            .write(&lcd_message(LCD_OFF, &[b' '; LCD_CHARS]));
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, ModelView};

    /// What opening sends: 35 LEDs, the LCD, the segments, the fader meter.
    const INIT: usize = 35 + 3;

    const LCD_WHITE: [u8; 23] = [
        0xf0, 0x00, 0x20, 0x32, 0x41, 0x4c, 0x00, 0x07, 0x20, 0x20, 0x52, 0x79, 0x75, 0x20, 0x20,
        0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0xf7,
    ];
    const SEGMENTS_BLANK: [u8; 21] = [
        0xf0, 0x00, 0x20, 0x32, 0x41, 0x37, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf7,
    ];

    fn open() -> (XTouchOne, Recorder) {
        let recorder = Recorder::default();
        let deck = XTouchOne::new(by_id(XTOUCH_ONE_ID).unwrap(), Box::new(recorder.clone()));
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

    fn queue(recorder: &Recorder, messages: &[&[u8]]) {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
    }

    #[test]
    fn opening_clears_the_lights_and_writes_the_displays() {
        let (mut deck, recorder) = open();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), INIT);
        for (number, write) in writes[..35].iter().enumerate() {
            assert_eq!(*write, vec![0xb0, number as u8, 0]);
        }
        assert_eq!(writes[35], LCD_WHITE.to_vec());
        assert_eq!(writes[36], SEGMENTS_BLANK.to_vec());
        assert_eq!(writes[37], vec![0xb0, 0x5a, 0x00]);
        // Nothing more until something changes.
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), INIT);
    }

    #[test]
    fn faces_set_before_opening_go_out_in_the_first_flush() {
        let (mut deck, recorder) = open();
        deck.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_deck_light(Some([0, 0, 255])).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), INIT);
        assert_eq!(writes[3], vec![0xb0, 3, 0x7f]);
        assert_eq!(writes[35][7], 4);
    }

    #[test]
    fn faces_reach_the_deck_on_flush() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        deck.set_face(0, Some(&lit([255, 128, 0]))).unwrap();
        deck.set_face(23, Some(&lit([0, 0, 200]))).unwrap();
        deck.set_face(34, Some(&lit([255, 255, 255]))).unwrap();
        // Black, near-black and no face stay off.
        deck.set_face(1, Some(&lit([0, 0, 0]))).unwrap();
        deck.set_face(2, Some(&lit([20, 20, 20]))).unwrap();
        deck.set_face(4, None).unwrap();
        // The fader touch has no light; past the end.
        deck.set_face(35, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(36, Some(&lit([255, 0, 0]))).unwrap();
        assert_eq!(recorder.writes().len(), INIT);
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[INIT..],
            [
                vec![0xb0, 0x00, 0x7f],
                vec![0xb0, 0x17, 0x7f],
                vec![0xb0, 0x22, 0x7f],
            ]
        );
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[INIT + 3..], [vec![0xb0, 0x00, 0x00]]);
        // Brightness changes nothing.
        deck.set_brightness(10).unwrap();
        assert_eq!(recorder.writes().len(), INIT + 4);
    }

    #[test]
    fn the_deck_light_colours_the_lcd_backlight() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        let cases: [(Option<[u8; 3]>, u8); 9] = [
            (Some([255, 0, 0]), 1),
            (Some([0, 255, 0]), 2),
            (Some([255, 200, 0]), 3),
            (Some([0, 0, 255]), 4),
            (Some([255, 0, 255]), 5),
            (Some([0, 128, 128]), 6),
            (Some([127, 127, 127]), 0),
            (Some([255, 255, 255]), 7),
            (None, 7),
        ];
        let mut sent = INIT;
        let mut last = WHITE;
        for (color, attr) in cases {
            deck.set_deck_light(color).unwrap();
            deck.flush().unwrap();
            let writes = recorder.writes();
            if attr == last {
                assert_eq!(writes.len(), sent, "{color:?}");
                continue;
            }
            assert_eq!(writes.len(), sent + 1, "{color:?}");
            let lcd = &writes[sent];
            assert_eq!(lcd.len(), 23);
            assert_eq!(lcd[..7], LCD_WHITE[..7]);
            assert_eq!(lcd[7], attr, "{color:?}");
            assert_eq!(lcd[8..], LCD_WHITE[8..]);
            sent += 1;
            last = attr;
        }
    }

    #[test]
    fn buttons_and_the_fader_touch_are_keys() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                // CC mode.
                &[0xb0, 0x00, 0x7f],
                &[0xb0, 0x00, 0x00],
                &[0xb0, 0x17, 0x7f],
                &[0xb0, 0x22, 0x7f],
                &[0xb0, 0x22, 0x00],
                &[0xb0, 0x6e, 0x7f],
                &[0xb0, 0x6e, 0x00],
                // Note mode, with a Note Off and a velocity-0 release.
                &[0x90, 0x05, 0x7f],
                &[0x80, 0x05, 0x40],
                &[0x90, 0x6e, 0x7f],
                &[0x90, 0x6e, 0x00],
                &[0x90, 0x17, 0x00],
                // A repeated release, unknown numbers, another channel,
                // a pitch bend (Mackie Control mode) and SysEx.
                &[0xb0, 0x22, 0x00],
                &[0xb0, 0x23, 0x7f],
                &[0x90, 0x46, 0x7f],
                &[0xb1, 0x01, 0x7f],
                &[0x91, 0x01, 0x7f],
                &[0xe0, 0x00, 0x40],
                &[0xf0, 0x00, 0x20, 0x32, 0xf7],
                // Short and stray input.
                &[0xb0],
                &[0x47, 0x00],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(23, true),
                key(34, true),
                key(34, false),
                key(35, true),
                key(35, false),
                key(5, true),
                key(5, false),
                key(35, true),
                key(35, false),
                key(23, false),
            ]
        );
    }

    #[test]
    fn the_encoder_is_relative_until_it_sends_a_position() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 0x50, 0x41],
                &[0xb0, 0x50, 0x41],
                &[0xb0, 0x50, 0x01],
                // Another value: absolute from here on, seeded by it.
                &[0xb0, 0x50, 0x30],
                &[0xb0, 0x50, 0x33],
                // 65 and 1 are now positions.
                &[0xb0, 0x50, 0x41],
                &[0xb0, 0x50, 0x01],
                &[0xb0, 0x50, 0x01],
                &[0xb0, 0x50, 0x7f],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, 1),
                turn(0, 1),
                turn(0, -1),
                turn(0, 3),
                turn(0, 14),
                turn(0, -64),
                turn(0, 126),
            ]
        );
    }

    #[test]
    fn the_jog_wheel_takes_only_single_steps() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 0x58, 0x41],
                &[0xb0, 0x58, 0x01],
                &[0xb0, 0x58, 0x05],
                &[0xb0, 0x58, 0x45],
                &[0xb0, 0x58, 0x00],
                &[0xb0, 0x58, 0x41],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(1, 1), turn(1, -1), turn(1, 1)]
        );
    }

    #[test]
    fn the_fader_turns_dial_3_after_its_first_value() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 0x46, 0x40],
                &[0xb0, 0x46, 0x44],
                &[0xb0, 0x46, 0x45],
                &[0xb0, 0x46, 0x46],
                &[0xb0, 0x46, 0x00],
                &[0xb0, 0x46, 0x7f],
                // Another channel is not the fader.
                &[0xb1, 0x46, 0x10],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(2, 2), turn(2, 1), turn(2, -35), turn(2, 63)]
        );
    }

    #[test]
    fn letting_go_turns_everything_off() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes.len(), 35 + 3);
        for (number, write) in writes[..35].iter().enumerate() {
            assert_eq!(*write, vec![0xb0, number as u8, 0]);
        }
        assert_eq!(writes[35], vec![0xb0, 0x5a, 0x00]);
        assert_eq!(writes[36], SEGMENTS_BLANK.to_vec());
        let mut lcd_off = vec![0xf0, 0x00, 0x20, 0x32, 0x41, 0x4c, 0x00, 0x00];
        lcd_off.extend([0x20; 14]);
        lcd_off.push(0xf7);
        assert_eq!(writes[37], lcd_off);
        // Never opened: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_the_xtouch_ones_port_matches() {
        let model = by_id(XTOUCH_ONE_ID).unwrap();
        let cases = [
            ("X-Touch One", false, true),
            ("X-Touch One", true, true),
            ("X-TOUCH ONE", false, true),
            ("x-touch one:x-touch one MIDI 1 24:0", false, true),
            ("X-Touch One:X-Touch One MIDI 1 24:0", false, true),
            ("MIDIIN2 (X-Touch One)", true, false),
            // The other X-Touches and an unrelated port.
            ("X-TOUCH MINI", false, false),
            ("X-Touch", false, false),
            ("X-Touch-Ext", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(XTOUCH_ONE_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_names_every_key_and_lights_only_the_buttons() {
        let view = ModelView::from(by_id(XTOUCH_ONE_ID).unwrap());
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), 36);
        assert_eq!(
            (names[0], names[23], names[34], names[35]),
            ("Segment Solo", "Play", "Down", "Fader Touch")
        );
        assert_eq!(view.lit_keys.unwrap(), (0..=34).collect::<Vec<u8>>());
        assert_eq!((view.keys, view.extra_keys, view.dials), (36, 0, 3));
        assert_eq!((view.rows, view.cols), (6, 6));
        assert!(view.key_cells.is_none());
        assert_eq!(view.deck_lights, 1);
        assert_eq!(view.key_light_colors, "on_off");
        assert!(view.key_lights && !view.brightness);
    }

    #[test]
    fn lcd_text_is_always_fourteen_printable_characters() {
        let mut text = [b'A'; LCD_CHARS];
        text[3] = 0x7f;
        text[9] = 0x0a;
        let message = lcd_message(0xff, &text);
        assert_eq!(message.len(), 23);
        assert_eq!(message[7], 0x37);
        assert_eq!((message[8 + 3], message[8 + 9]), (b' ', b' '));
        assert_eq!(message[22], 0xf7);
        assert_eq!(blank_segments().len(), 21);
    }
}
