//! PreSonus ATOM SQ in its native (host-controlled) mode, over class-compliant
//! USB MIDI ([`super::midi`]).
//!
//! The ATOM SQ lists one MIDI port each way named `ATM SQ`. [`port_matches`]
//! takes a name holding `ATM SQ` (or, defensively, `ATOM SQ`) in any case,
//! and on Windows only the first interface; the plain ATOM never matches.
//!
//! Native mode: `8F 00 01` (Note Off, channel 16) turns it on and `8F 00 00`
//! turns it off. At open Ryu also sends both firmware toggles off
//! (`F0 00 01 06 22 13 00 F7` for the lower pads, `... 14 00 F7` for the
//! upper), so the host owns every pad. Everything else is on channel 1
//! except the touch strip.
//!
//! In: the upper pad row is notes 52-67 (keys 0-15) and the lower row notes
//! 36-51 (keys 16-31), left to right; Plus and Minus are notes 0 and 1 (keys
//! 61 and 62). A Note On above velocity 0 presses; a Note Off or velocity 0
//! releases. The other buttons are momentary Control Change (above 0
//! pressed, 0 released), keys 32-60 in [`CONTROLS`] order. Notes and
//! controllers share numbers (CC 36-41 are the display buttons, notes 36-41
//! lower pads), so a message is read by its type and number together. The
//! eight encoders are Control Change 14-21 (dials 1-8) and the display
//! encoder Control Change 29 (dial 9), relative: 1-64 turn clockwise by the
//! value, 65-127 anticlockwise by the value less 64. The touch strip is a
//! pitch bend on channel 16, 14-bit absolute: its first value seeds dial 10,
//! then each 128 units moved turn it a step. Aftertouch, Program Change,
//! SysEx, other channels and unknown numbers are dropped.
//!
//! Out: an RGB control (the 32 pads, Plus, Minus, Play and Bank A-H) takes
//! four messages on its own number: red, green and blue (7-bit) on channels
//! 2, 3 and 4, then channel 1 with 127 (lit steady) or 0 (off), as Note Ons
//! for a note control and Control Changes for a controller. The other
//! buttons' single-colour lights are their Control Change, 127 on or 0 off:
//! a colour that is not dark turns one on. The 25 touch-strip LEDs (Control
//! Change 55-79) are turned off. There is no brightness command, so Ryu
//! scales the colours itself, and 0% turns every light off. The character
//! display takes one SysEx per line (`F0 00 01 06 22 12 <line> 00 5B 5B 01
//! <ASCII> F7`): Ryu writes its name on the track line (6), blanks the device
//! line (7), and writes the labels of keys 41-46 (display buttons 1-6),
//! centred, on the button lines 0, 1, 2, 11, 12 and 13, resending a line only
//! when its text changes. Writes are queued and drained a few at a time from
//! [`Driver::poll`] and [`Driver::flush`], so the shared poll loop never waits
//! for the deck. When Ryu lets go it turns every light off and native mode
//! off.
//!
//! Facts from Ableton Live's ATOM SQ Remote Script and the `ableton.v2` /
//! `ableton.v3` control surface frameworks it builds on, as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for
//! facts and the USB id only); no code was copied.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const ATOM_SQ_ID: &str = "presonus-atom-sq";

/// The pads, keys 0-31.
pub const PADS: u8 = 32;
/// The buttons, keys 32-62.
pub const BUTTONS: u8 = 31;
pub const KEYS: u8 = PADS + BUTTONS;
/// Eight encoders, the display encoder and the touch strip.
pub const DIALS: u8 = 10;
pub const ROWS: u8 = 4;
pub const COLS: u8 = 16;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const PITCH_BEND: u8 = 0xe0;
/// The touch strip's channel (16).
const STRIP_CHANNEL: u8 = 0x0f;
/// An RGB light's red, green and blue parts go on channels 2, 3 and 4.
const RED: u8 = 1;
const GREEN: u8 = 2;
const BLUE: u8 = 3;

/// Native mode on and off: Note Off on channel 16, note 0.
pub const NATIVE_ON: [u8; 3] = [NOTE_OFF | 0x0f, 0x00, 0x01];
pub const NATIVE_OFF: [u8; 3] = [NOTE_OFF | 0x0f, 0x00, 0x00];
/// The ATOM SQ's SysEx header: PreSonus (00 01 06), then the deck (22).
const SYSEX_HEADER: [u8; 5] = [0xf0, 0x00, 0x01, 0x06, 0x22];
/// The firmware toggles for the lower and upper pads, both off: the host
/// owns every pad.
pub const LOWER_FIRMWARE_OFF: [u8; 8] = [0xf0, 0x00, 0x01, 0x06, 0x22, 0x13, 0x00, 0xf7];
pub const UPPER_FIRMWARE_OFF: [u8; 8] = [0xf0, 0x00, 0x01, 0x06, 0x22, 0x14, 0x00, 0xf7];
/// The display-text command and the bytes before the text.
const DISPLAY_TEXT: u8 = 0x12;
const DISPLAY_PREFIX: [u8; 4] = [0x00, 0x5b, 0x5b, 0x01];

/// An RGB light lit steady (1 blinks and 2 pulse; Ryu never sends those).
const STEADY: u8 = 0x7f;
const LED_ON: u8 = 0x7f;
const LED_OFF: u8 = 0x00;

/// The encoders' first controller (dials 0-7), and the display encoder's.
const FIRST_ENCODER: u8 = 14;
const ENCODERS: u8 = 8;
const DISPLAY_ENCODER: u8 = 29;
const DISPLAY_ENCODER_DIAL: u8 = 8;
const STRIP_DIAL: u8 = 9;
/// Touch-strip units (of 16384) per dial step: 128 steps end to end.
const STRIP_STEP: i32 = 128;
/// The touch strip's 25 LEDs, left to right.
const STRIP_FIRST_LED: u8 = 55;
const STRIP_LEDS: u8 = 25;

/// What a key is on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Note(u8),
    Cc(u8),
}

use Control::{Cc, Note};

/// Every key's control, keys 0-62 in order.
const CONTROLS: [Control; KEYS as usize] = [
    // Upper pads, notes 52-67.
    Note(52),
    Note(53),
    Note(54),
    Note(55),
    Note(56),
    Note(57),
    Note(58),
    Note(59),
    Note(60),
    Note(61),
    Note(62),
    Note(63),
    Note(64),
    Note(65),
    Note(66),
    Note(67),
    // Lower pads, notes 36-51.
    Note(36),
    Note(37),
    Note(38),
    Note(39),
    Note(40),
    Note(41),
    Note(42),
    Note(43),
    Note(44),
    Note(45),
    Note(46),
    Note(47),
    Note(48),
    Note(49),
    Note(50),
    Note(51),
    // Bank A-H.
    Cc(0),
    Cc(1),
    Cc(2),
    Cc(3),
    Cc(4),
    Cc(5),
    Cc(6),
    Cc(7),
    // Display buttons 1-6.
    Cc(36),
    Cc(37),
    Cc(38),
    Cc(39),
    Cc(40),
    Cc(41),
    // Shift, Song, Instrument, Editor, User.
    Cc(31),
    Cc(32),
    Cc(33),
    Cc(34),
    Cc(35),
    // Display Left and Right.
    Cc(42),
    Cc(43),
    // Up, Down, Left, Right.
    Cc(87),
    Cc(89),
    Cc(90),
    Cc(102),
    // Click, Record, Play, Stop.
    Cc(105),
    Cc(107),
    Cc(109),
    Cc(111),
    // Plus and Minus.
    Note(0),
    Note(1),
];

const NAMES: [&str; KEYS as usize] = [
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
    "Bank A",
    "Bank B",
    "Bank C",
    "Bank D",
    "Bank E",
    "Bank F",
    "Bank G",
    "Bank H",
    "Display 1",
    "Display 2",
    "Display 3",
    "Display 4",
    "Display 5",
    "Display 6",
    "Shift",
    "Song",
    "Instrument",
    "Editor",
    "User",
    "Display Left",
    "Display Right",
    "Up",
    "Down",
    "Left",
    "Right",
    "Click",
    "Record",
    "Play",
    "Stop",
    "Plus",
    "Minus",
];

/// The first bank key and the Play key: with the pads, Plus and Minus, the
/// keys with an RGB light.
const FIRST_BANK_KEY: u8 = 32;
const PLAY_KEY: u8 = 59;
/// The display buttons' keys and the display lines holding their labels.
const FIRST_DISPLAY_KEY: u8 = 40;
const LABEL_LINES: [u8; 6] = [0, 1, 2, 11, 12, 13];
/// The track-name and device-name lines.
const TRACK_LINE: u8 = 6;
const DEVICE_LINE: u8 = 7;
const NAME_WIDTH: usize = 18;
const LABEL_WIDTH: usize = 14;
/// Every line Ryu writes: the track line, the device line, then the labels.
const LINES: usize = 2 + LABEL_LINES.len();

/// The gap between two bursts of writes.
const PACE: Duration = Duration::from_millis(2);
/// Writes per burst.
const BURST: usize = 16;
/// A button's colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 256;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// Whether a normalised port name is an ATOM SQ's: `ATM SQ` (or `ATOM SQ`) in
/// any case, and on Windows the first interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.protocol != Protocol::PresonusAtomSqMidi || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.to_ascii_uppercase();
    base.contains("ATM SQ") || base.contains("ATOM SQ")
}

/// The legend on each key: the pads in reading order, then the buttons'
/// printed names.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::PresonusAtomSqMidi).then(|| NAMES.to_vec())
}

/// Whether a key's light takes a colour (rather than only on or off).
const fn is_rgb(key: u8) -> bool {
    key < FIRST_BANK_KEY + 8 || key == PLAY_KEY || key >= KEYS - 2
}

/// The key a control presses.
fn key_of(control: Control) -> Option<u8> {
    CONTROLS
        .iter()
        .position(|known| *known == control)
        .and_then(|at| u8::try_from(at).ok())
}

/// The dial a controller turns.
const fn dial_of_cc(cc: u8) -> Option<u8> {
    if cc >= FIRST_ENCODER && cc < FIRST_ENCODER + ENCODERS {
        Some(cc - FIRST_ENCODER)
    } else if cc == DISPLAY_ENCODER {
        Some(DISPLAY_ENCODER_DIAL)
    } else {
        None
    }
}

/// An encoder's movement: 1-64 clockwise by the value, 65-127 anticlockwise
/// by the value less 64; 0 is no movement.
fn delta(value: u8) -> Option<i8> {
    match value {
        1..=64 => i8::try_from(value).ok(),
        65..=127 => i8::try_from(64 - i16::from(value)).ok(),
        _ => None,
    }
}

/// One 8-bit channel at a brightness, as the 7-bit value the deck takes.
fn level(channel: u8, percent: u8) -> u8 {
    let scaled = u16::from(channel) * u16::from(percent.min(100)) / 100;
    u8::try_from(scaled >> 1).unwrap_or(0x7f)
}

/// Text as the display takes it: printable ASCII (anything else becomes
/// `?`), cut to `width`, and padded with spaces to it, centred or not.
fn fit(text: &str, width: usize, centred: bool) -> Vec<u8> {
    let chars: Vec<u8> = text
        .trim()
        .chars()
        .take(width)
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                u8::try_from(c).unwrap_or(b'?')
            } else {
                b'?'
            }
        })
        .collect();
    let spare = width - chars.len();
    let before = if centred { spare / 2 } else { 0 };
    let mut line = vec![b' '; before];
    line.extend(chars);
    line.resize(width, b' ');
    line
}

/// One display line's SysEx.
fn line_message(line: u8, text: &[u8]) -> Vec<u8> {
    let mut message = SYSEX_HEADER.to_vec();
    message.push(DISPLAY_TEXT);
    message.push(line);
    message.extend(DISPLAY_PREFIX);
    message.extend(text);
    message.push(0xf7);
    message
}

/// The touch strip read as a dial: its first value seeds it, then every
/// [`STRIP_STEP`] units moved turn the dial one step.
#[derive(Debug, Default)]
struct Strip {
    last: Option<u16>,
    rest: i32,
}

impl Strip {
    fn moved(&mut self, value: u16) -> i8 {
        let Some(old) = self.last.replace(value) else {
            return 0;
        };
        let moved = self.rest + i32::from(value) - i32::from(old);
        let steps = moved / STRIP_STEP;
        self.rest = moved - steps * STRIP_STEP;
        i8::try_from(steps.clamp(-127, 127)).unwrap_or(0)
    }
}

pub struct AtomSq {
    transport: Box<dyn Transport>,
    pace: Duration,
    /// Native mode, the toggles, every light and the display are queued.
    started: bool,
    /// Writes waiting their turn: up to [`BURST`] go out each time `pace`
    /// has passed, so the shared poll loop never sleeps for the deck.
    outbox: VecDeque<Vec<u8>>,
    /// When the next burst may go out.
    next_send: Instant,
    colors: [[u8; 3]; KEYS as usize],
    labels: [String; LABEL_LINES.len()],
    percent: u8,
    /// What each key's light was last queued as: an RGB key's 7-bit colour,
    /// or a button's state in the first byte; `None` until it is.
    sent: [Option<[u8; 3]>; KEYS as usize],
    /// The text last queued on each line Ryu writes.
    sent_lines: [Option<Vec<u8>>; LINES],
    held: [bool; KEYS as usize],
    strip: Strip,
    parser: Parser,
}

impl AtomSq {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            pace: PACE,
            started: false,
            outbox: VecDeque::new(),
            next_send: Instant::now(),
            colors: [BLACK; KEYS as usize],
            labels: Default::default(),
            percent: 100,
            sent: [None; KEYS as usize],
            sent_lines: Default::default(),
            held: [false; KEYS as usize],
            strip: Strip::default(),
            parser: Parser::default(),
        }
    }

    /// The light a key should show: an RGB key's 7-bit colour, or a button's
    /// state in the first byte.
    fn wanted(&self, key: u8) -> [u8; 3] {
        let color = self.colors.get(usize::from(key)).copied().unwrap_or(BLACK);
        if is_rgb(key) {
            color.map(|channel| level(channel, self.percent))
        } else if self.percent > 0 && color.iter().any(|channel| *channel >= DARK) {
            [LED_ON, 0, 0]
        } else {
            [LED_OFF, 0, 0]
        }
    }

    /// Queues one key's light, replacing that key's light still waiting.
    fn queue(&mut self, key: u8, light: [u8; 3]) {
        let Some(control) = CONTROLS.get(usize::from(key)).copied() else {
            return;
        };
        let (kind, number) = match control {
            Note(note) => (NOTE_ON, note),
            Cc(cc) => (CONTROL_CHANGE, cc),
        };
        self.outbox.retain(|queued| {
            !(queued.len() == 3
                && queued[0] & 0xf0 == kind
                && queued[0] & 0x0f <= BLUE
                && queued[1] == number)
        });
        if is_rgb(key) {
            let [red, green, blue] = light;
            let state = if light == BLACK { LED_OFF } else { STEADY };
            self.outbox.extend([
                vec![kind | RED, number, red],
                vec![kind | GREEN, number, green],
                vec![kind | BLUE, number, blue],
                vec![kind, number, state],
            ]);
        } else {
            self.outbox.push_back(vec![kind, number, light[0]]);
        }
        if let Some(sent) = self.sent.get_mut(usize::from(key)) {
            *sent = Some(light);
        }
    }

    /// Queues every light whose wanted state differs from what was sent.
    fn queue_changes(&mut self) {
        for key in 0..KEYS {
            let light = self.wanted(key);
            if self.sent.get(usize::from(key)).copied().flatten() != Some(light) {
                self.queue(key, light);
            }
        }
    }

    /// Every display line Ryu writes, with its text: Ryu's name, a blank
    /// device line, then the display buttons' labels.
    fn wanted_lines(&self) -> [(u8, Vec<u8>); LINES] {
        let label = |at: usize| {
            let text = self.labels.get(at).map_or("", String::as_str);
            (
                LABEL_LINES.get(at).copied().unwrap_or(0),
                fit(text, LABEL_WIDTH, true),
            )
        };
        [
            (TRACK_LINE, fit("Ryu", NAME_WIDTH, false)),
            (DEVICE_LINE, fit("", NAME_WIDTH, false)),
            label(0),
            label(1),
            label(2),
            label(3),
            label(4),
            label(5),
        ]
    }

    /// Queues every display line whose text differs from what was sent,
    /// replacing that line's text still waiting.
    fn queue_lines(&mut self) {
        for (at, (line, text)) in self.wanted_lines().into_iter().enumerate() {
            let Some(sent) = self.sent_lines.get_mut(at) else {
                continue;
            };
            if sent.as_ref() == Some(&text) {
                continue;
            }
            *sent = Some(text.clone());
            self.outbox.retain(|queued| {
                !(queued.starts_with(&SYSEX_HEADER)
                    && queued.get(5) == Some(&DISPLAY_TEXT)
                    && queued.get(6) == Some(&line))
            });
            self.outbox.push_back(line_message(line, &text));
        }
    }

    /// Every touch-strip LED off.
    fn queue_strip_off(&mut self) {
        for led in 0..STRIP_LEDS {
            self.outbox
                .push_back(vec![CONTROL_CHANGE, STRIP_FIRST_LED + led, LED_OFF]);
        }
    }

    /// Native mode on, both firmware toggles off, every light, the strip off,
    /// then the display.
    fn start(&mut self) {
        self.started = true;
        self.outbox.clear();
        self.outbox.push_back(NATIVE_ON.to_vec());
        self.outbox.push_back(LOWER_FIRMWARE_OFF.to_vec());
        self.outbox.push_back(UPPER_FIRMWARE_OFF.to_vec());
        self.sent = [None; KEYS as usize];
        self.sent_lines = Default::default();
        self.queue_changes();
        self.queue_strip_off();
        self.queue_lines();
    }

    /// Writes the queued messages whose turn has come, without ever sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        while !self.outbox.is_empty() {
            let now = Instant::now();
            if now < self.next_send {
                break;
            }
            for _ in 0..BURST {
                let Some(message) = self.outbox.front() else {
                    break;
                };
                self.transport.write(message)?;
                self.outbox.pop_front();
            }
            self.next_send = now + self.pace;
        }
        Ok(())
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

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        if status == PITCH_BEND | STRIP_CHANNEL {
            let position = u16::from(number & 0x7f) | (u16::from(value & 0x7f) << 7);
            let delta = self.strip.moved(position);
            if delta != 0 {
                events.push(Input::DialTurn {
                    index: STRIP_DIAL,
                    delta,
                });
            }
            return;
        }
        if status & 0x0f != 0 {
            return;
        }
        match status & 0xf0 {
            NOTE_ON | NOTE_OFF => {
                if let Some(key) = key_of(Note(number)) {
                    self.press(key, status & 0xf0 == NOTE_ON && value > 0, events);
                }
            }
            CONTROL_CHANGE => {
                if let Some(dial) = dial_of_cc(number) {
                    if let Some(delta) = delta(value) {
                        events.push(Input::DialTurn { index: dial, delta });
                    }
                } else if let Some(key) = key_of(Cc(number)) {
                    self.press(key, value > 0, events);
                }
            }
            _ => {}
        }
    }
}

impl Driver for AtomSq {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
            self.start();
        }
        self.pump()?;
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
        if let Some(label) = key
            .checked_sub(FIRST_DISPLAY_KEY)
            .and_then(|at| self.labels.get_mut(usize::from(at)))
        {
            *label = face.map(|face| face.label.clone()).unwrap_or_default();
        }
        Ok(())
    }

    /// Native mode, every light and the display the first time, then only
    /// the lights and lines that changed.
    fn flush(&mut self) -> DriverResult<()> {
        if self.started {
            self.queue_changes();
            self.queue_lines();
        } else {
            self.start();
        }
        self.pump()
    }

    /// The ATOM SQ has no brightness command: Ryu scales the colours, and 0%
    /// turns every light off.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.percent = percent.min(100);
        if self.started {
            self.queue_changes();
            self.pump()?;
        }
        Ok(())
    }
}

impl Drop for AtomSq {
    /// Every light and the strip off, then native mode off, giving the deck
    /// back its own modes. Bursts are `pace` apart (zero in tests).
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        self.outbox.clear();
        for key in 0..KEYS {
            let light = if is_rgb(key) { BLACK } else { [LED_OFF, 0, 0] };
            self.queue(key, light);
        }
        self.queue_strip_off();
        self.outbox.push_back(NATIVE_OFF.to_vec());
        let mut first = true;
        while !self.outbox.is_empty() {
            if !first && !self.pace.is_zero() {
                std::thread::sleep(self.pace);
            }
            first = false;
            for _ in 0..BURST {
                let Some(message) = self.outbox.pop_front() else {
                    break;
                };
                let _ = self.transport.write(&message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    fn open() -> (AtomSq, Recorder) {
        let recorder = Recorder::default();
        let mut deck = AtomSq::new(by_id(ATOM_SQ_ID).unwrap(), Box::new(recorder.clone()));
        deck.pace = Duration::ZERO;
        (deck, recorder)
    }

    /// 43 RGB controls of four writes and 20 single-colour ones.
    const LIGHTS: usize = 43 * 4 + 20;
    /// Native mode, two toggles, the lights, 25 strip LEDs and 8 lines.
    const START: usize = 3 + LIGHTS + 25 + 8;

    fn lit(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    fn labelled(label: &str) -> Face {
        Face {
            label: label.to_owned(),
            color: BLACK,
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

    fn rgb(kind: u8, number: u8, color: [u8; 3], state: u8) -> Vec<Vec<u8>> {
        vec![
            vec![kind | 1, number, color[0]],
            vec![kind | 2, number, color[1]],
            vec![kind | 3, number, color[2]],
            vec![kind, number, state],
        ]
    }

    fn line(id: u8, text: &str) -> Vec<u8> {
        let mut message = vec![
            0xf0, 0x00, 0x01, 0x06, 0x22, 0x12, id, 0x00, 0x5b, 0x5b, 0x01,
        ];
        message.extend(text.bytes());
        message.push(0xf7);
        message
    }

    #[test]
    fn controls_map_to_keys_and_dials_by_type_and_number() {
        assert_eq!(CONTROLS.len(), usize::from(KEYS));
        assert_eq!(key_of(Note(52)), Some(0));
        assert_eq!(key_of(Note(67)), Some(15));
        assert_eq!(key_of(Note(36)), Some(16));
        assert_eq!(key_of(Note(51)), Some(31));
        assert_eq!(key_of(Note(0)), Some(61));
        assert_eq!(key_of(Note(1)), Some(62));
        assert_eq!(key_of(Cc(0)), Some(32));
        assert_eq!(key_of(Cc(7)), Some(39));
        // The display buttons share numbers with lower pads.
        assert_eq!(key_of(Cc(36)), Some(40));
        assert_eq!(key_of(Cc(41)), Some(45));
        assert_eq!(key_of(Cc(31)), Some(46));
        assert_eq!(key_of(Cc(35)), Some(50));
        assert_eq!(key_of(Cc(42)), Some(51));
        assert_eq!(key_of(Cc(43)), Some(52));
        assert_eq!(key_of(Cc(87)), Some(53));
        assert_eq!(key_of(Cc(102)), Some(56));
        assert_eq!(key_of(Cc(105)), Some(57));
        assert_eq!(key_of(Cc(111)), Some(60));
        for note in [2, 35, 68, 127] {
            assert_eq!(key_of(Note(note)), None, "{note}");
        }
        for cc in [8, 14, 29, 44, 55, 88, 127] {
            assert_eq!(key_of(Cc(cc)), None, "{cc}");
        }
        assert_eq!(dial_of_cc(14), Some(0));
        assert_eq!(dial_of_cc(21), Some(7));
        assert_eq!(dial_of_cc(29), Some(8));
        assert_eq!(dial_of_cc(13), None);
        assert_eq!(dial_of_cc(22), None);
        // Signed bit, 64 the largest clockwise turn.
        assert_eq!(delta(1), Some(1));
        assert_eq!(delta(64), Some(64));
        assert_eq!(delta(65), Some(-1));
        assert_eq!(delta(127), Some(-63));
        assert_eq!(delta(0), None);
        assert_eq!(delta(128), None);
        let rgb_keys = (0..KEYS).filter(|key| is_rgb(*key)).count();
        assert_eq!(rgb_keys, 43);
        assert!(is_rgb(59) && is_rgb(61) && is_rgb(62) && is_rgb(39));
        assert!(!is_rgb(40) && !is_rgb(58) && !is_rgb(60));
    }

    #[test]
    fn display_text_is_ascii_cut_and_padded() {
        assert_eq!(fit("Ryu", 6, false), b"Ryu   ".to_vec());
        assert_eq!(fit("Mute", 8, true), b"  Mute  ".to_vec());
        assert_eq!(fit("Caf\u{e9}", 4, false), b"Caf?".to_vec());
        assert_eq!(
            fit("A very long label here", 14, true),
            b"A very long la".to_vec()
        );
        assert_eq!(fit("", 3, true), b"   ".to_vec());
    }

    #[test]
    fn the_first_poll_starts_native_mode_and_sends_every_light_and_line() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(16, Some(&lit([0, 128, 64]))).unwrap();
        deck.set_face(32, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(46, Some(&lit([200, 200, 200]))).unwrap();
        deck.set_face(60, Some(&lit([10, 10, 10]))).unwrap();
        deck.set_face(40, Some(&labelled("Mute"))).unwrap();
        assert!(recorder.writes().is_empty());
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), START);
        assert_eq!(writes[0], vec![0x8f, 0x00, 0x01]);
        assert_eq!(
            writes[1],
            vec![0xf0, 0x00, 0x01, 0x06, 0x22, 0x13, 0x00, 0xf7]
        );
        assert_eq!(
            writes[2],
            vec![0xf0, 0x00, 0x01, 0x06, 0x22, 0x14, 0x00, 0xf7]
        );
        // Key 0 is note 52: pure red, lit steady.
        assert_eq!(writes[3..7], rgb(0x90, 52, [0x7f, 0, 0], 0x7f));
        // Key 1 is note 53, black: off.
        assert_eq!(writes[7..11], rgb(0x90, 53, [0, 0, 0], 0x00));
        // Key 16 is note 36.
        let at = 3 + 16 * 4;
        assert_eq!(writes[at..at + 4], rgb(0x90, 36, [0, 64, 32], 0x7f));
        // Bank A is CC 0, as Control Changes.
        let at = 3 + 32 * 4;
        assert_eq!(writes[at..at + 4], rgb(0xb0, 0, [0, 0, 0x7f], 0x7f));
        // Display button 1 (CC 36) is single-colour and off; Shift on.
        let at = 3 + 40 * 4;
        assert_eq!(writes[at], vec![0xb0, 36, 0x00]);
        assert_eq!(writes[at + 6], vec![0xb0, 31, 0x7f]);
        // Play (key 59) is RGB, after the 13 single-colour keys 46-58.
        let play = at + 6 + 13;
        assert_eq!(writes[play..play + 4], rgb(0xb0, 109, [0, 0, 0], 0));
        // Stop is too dark to light.
        assert_eq!(writes[play + 4], vec![0xb0, 111, 0x00]);
        // Plus and Minus are notes 0 and 1.
        assert_eq!(writes[play + 5..play + 9], rgb(0x90, 0, [0, 0, 0], 0));
        assert_eq!(writes[play + 9..play + 13], rgb(0x90, 1, [0, 0, 0], 0));
        // The strip off, then the lines.
        let strip = 3 + LIGHTS;
        assert_eq!(writes[strip], vec![0xb0, 55, 0x00]);
        assert_eq!(writes[strip + 24], vec![0xb0, 79, 0x00]);
        let lines = strip + 25;
        assert_eq!(writes[lines], line(6, "Ryu               "));
        assert_eq!(writes[lines + 1], line(7, &" ".repeat(18)));
        assert_eq!(writes[lines + 2], line(0, "     Mute     "));
        assert_eq!(writes[lines + 3], line(1, &" ".repeat(14)));
        assert_eq!(writes[lines + 7], line(13, &" ".repeat(14)));
        // A flush with nothing changed sends nothing.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), START);
    }

    #[test]
    fn flushes_send_only_changes_and_brightness_scales_colours() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), START);
        deck.set_face(61, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(53, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(45, Some(&labelled("Solo"))).unwrap();
        deck.flush().unwrap();
        let mut expected = vec![vec![0xb0, 87, 0x7f]];
        expected.extend(rgb(0x90, 0, [0, 0, 0x7f], 0x7f));
        expected.push(line(13, "     Solo     "));
        assert_eq!(recorder.writes()[START..], expected);
        // Half brightness halves Plus; Up stays on.
        deck.set_brightness(50).unwrap();
        assert_eq!(
            recorder.writes()[START + 6..],
            rgb(0x90, 0, [0, 0, 63], 0x7f)
        );
        // 0% turns every light off.
        deck.set_brightness(0).unwrap();
        let mut expected = vec![vec![0xb0, 87, 0x00]];
        expected.extend(rgb(0x90, 0, [0, 0, 0], 0x00));
        assert_eq!(recorder.writes()[START + 10..], expected);
    }

    #[test]
    fn pads_buttons_encoders_and_the_strip_are_read() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                // Upper-left pad, released by Note Off.
                &[0x90, 52, 0x40],
                &[0x80, 52, 0x00],
                // Lower-left pad, released by velocity 0, in running status.
                &[0x90, 36, 0x7f, 36, 0x00],
                // Display button 1 is CC 36, not the pad.
                &[0xb0, 36, 0x7f],
                &[0xb0, 36, 0x00],
                // Bank A is CC 0 and Plus note 0.
                &[0xb0, 0, 0x7f],
                &[0x90, 0, 0x7f],
                // Aftertouch and channel pressure.
                &[0xa0, 40, 0x30],
                &[0xd0, 0x30],
                // Encoders: +1, -1, +64, -63, the display encoder +2, no move.
                &[0xb0, 14, 0x01],
                &[0xb0, 15, 0x41],
                &[0xb0, 20, 0x40],
                &[0xb0, 21, 0x7f],
                &[0xb0, 29, 0x02],
                &[0xb0, 16, 0x00],
                // The strip: seed at the centre, +2 steps, then -1.
                &[0xef, 0x00, 0x40],
                &[0xef, 0x00, 0x42],
                &[0xef, 0x00, 0x41],
                // Pitch bend off channel 16, other channels, unknown
                // numbers, Program Change and SysEx.
                &[0xe0, 0x00, 0x7f],
                &[0x91, 52, 0x7f],
                &[0xb1, 0, 0x7f],
                &[0xb0, 55, 0x7f],
                &[0x90, 70, 0x7f],
                &[0xc0, 0x02],
                &[0xf0, 0x7e, 0x00, 0x06, 0x02, 0xf7],
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
                key(16, true),
                key(16, false),
                key(40, true),
                key(40, false),
                key(32, true),
                key(61, true),
                turn(0, 1),
                turn(1, -1),
                turn(6, 64),
                turn(7, -63),
                turn(8, 2),
                turn(9, 2),
                turn(9, -1),
            ]
        );
        // Nothing is answered.
        assert_eq!(recorder.writes().len(), START);
    }

    #[test]
    fn writes_are_paced_in_bursts_without_sleeping() {
        let recorder = Recorder::default();
        let mut deck = AtomSq::new(by_id(ATOM_SQ_ID).unwrap(), Box::new(recorder.clone()));
        deck.pace = Duration::from_secs(3600);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), BURST);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), BURST);
        deck.next_send = Instant::now() - Duration::from_millis(1);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 2 * BURST);
        // A later light replaces the same pad's light still queued, and a
        // later label the same line's.
        deck.set_face(31, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(41, Some(&labelled("Go"))).unwrap();
        deck.flush().unwrap();
        assert_eq!(deck.outbox.len(), START - 2 * BURST);
        // Never sleep on drop in a test.
        deck.pace = Duration::ZERO;
        deck.next_send = Instant::now();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), START);
        assert!(writes
            .windows(4)
            .any(|four| four == rgb(0x90, 51, [0x7f, 0x7f, 0x7f], 0x7f).as_slice()));
        assert!(!writes
            .windows(4)
            .any(|four| four == rgb(0x90, 51, [0, 0, 0], 0).as_slice()));
        assert!(writes.contains(&line(1, "      Go      ")));
        assert!(!writes.contains(&line(1, &" ".repeat(14))));
    }

    #[test]
    fn letting_go_turns_lights_and_native_mode_off() {
        let (mut deck, recorder) = open();
        deck.set_face(5, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        drop(deck);
        let writes = &recorder.writes()[START..];
        assert_eq!(writes.len(), LIGHTS + 25 + 1);
        assert_eq!(writes[..4], rgb(0x90, 52, [0, 0, 0], 0));
        assert_eq!(writes[LIGHTS + 24], vec![0xb0, 79, 0x00]);
        assert_eq!(writes[LIGHTS + 25], vec![0x8f, 0x00, 0x00]);
        assert!(writes
            .iter()
            .all(|write| write.len() == 3 && (write[0] == 0x8f || write[2] == 0)));
        // Never opened: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_atom_sq_ports_match() {
        let model = by_id(ATOM_SQ_ID).unwrap();
        let cases = [
            ("ATM SQ", false, true),
            ("ATM SQ", true, true),
            ("atm sq", false, true),
            ("ATM SQ:ATM SQ MIDI 1 24:0", false, true),
            ("ATOM SQ", false, true),
            ("MIDIIN2 (ATM SQ)", true, false),
            // The plain ATOM and other names.
            ("ATOM", false, false),
            ("ATOM:ATOM MIDI 1 24:0", false, false),
            ("ATM", false, false),
            ("LPD8", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(ATOM_SQ_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_names_every_key() {
        let view = ModelView::from(by_id(ATOM_SQ_ID).unwrap());
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), usize::from(KEYS));
        assert_eq!(names[0], "Pad 1");
        assert_eq!(names[32], "Bank A");
        assert_eq!(names[40], "Display 1");
        assert_eq!(names[46], "Shift");
        assert_eq!(names[60], "Stop");
        assert_eq!(names[62], "Minus");
        assert_eq!(
            (view.rows, view.cols, view.keys, view.dials, view.extra_keys),
            (4, 16, 63, 10, 31)
        );
        assert_eq!(view.lit_keys, None);
        assert_eq!(view.key_light_colors, "rgb");
        assert_eq!(view.dial_lights, DialLights::None);
        assert!(view.key_lights && view.brightness);
    }
}
