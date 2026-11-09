//! iCON Pro Audio's Platform M+ and Platform M, over class-compliant USB
//! MIDI ([`super::midi`]), in their Mackie Control mode.
//!
//! Each deck lists one MIDI port each way, whose name carries the firmware
//! version: `Platform M+ V2.15` (seen from V1.07 to V2.21), `Platform M
//! V1.14`, and on Linux a ` MIDI 1` after it. [`port_matches`] keys on
//! `PLATFORM M+` for the M+ and on `PLATFORM M` not followed by `+` for the
//! M, so neither is taken for the other, and a `Platform X` extender or
//! another iCON deck (QCon, P1-M, V1-M) never matches.
//!
//! The deck is put in Mackie Control mode on the deck itself at power-up; in
//! its other ("standard") mode its knobs send absolute values, which Ryu
//! cannot read. A V-Pot value with more than 15 steps is taken as that mode,
//! ignored, and logged once as a hint. There is no handshake and no
//! keep-alive.
//!
//! In, on MIDI channel 1: every button and fader touch is a Note On, pressed
//! at a velocity over 64 and released at 64 or under (a Note Off releases
//! too, defensively). Ryu lays the keys out 6 x 9: per strip (columns 1-8)
//! Select (`0x18+s`), Mute (`0x10+s`), Solo (`0x08+s`), Rec (`0x00+s`) and
//! the fader touch (`0x68+s`), and in column 9 Mixer (`0x54`, the Mackie
//! Marker note), Read (`0x4A`), Write (`0x4B`), Cycle (`0x56`) and the
//! master fader touch (`0x70`); the bottom row is Channel < > (`0x30`,
//! `0x31`), Bank < > (`0x2E`, `0x2F`), Rewind, Forward, Stop, Play and Record
//! (`0x5B`-`0x5F`). A V-Pot push (`0x20+s`) presses its dial. The V-Pots are
//! `CC 0x10+s` and the jog wheel `CC 0x3C`, both signed-bit relative: bit 6
//! set turns anticlockwise, bits 0-5 count the steps (0 counts as 1). The
//! nine motor faders are pitch bend on channels 1-9 (the master on 9): the
//! first value of each only seeds it, then every 256 of its 14 bits (64
//! steps end to end) turn its dial a step. Zoom and Lock send nothing.
//!
//! Out, on channel 1: a button's LED is `90 <note> 7F` on and `90 <note> 00`
//! off (`01` would blink; Ryu does not use it). Opening queues every LED off;
//! writes wait in a queue drained by poll and flush, a millisecond apart
//! (the deck's buffer size is not known; no wait in tests), and Ryu never
//! sleeps in the poll loop. Ryu never sends pitch bend (the faders have
//! motors), the V-Pot rings (`B0 30-37`, not documented for this deck), the
//! meters (`D0`) or the Mackie display SysEx: the deck has no display. There
//! is no brightness command.
//!
//! Facts from Ardour's Mackie Control surface and its Platform M+ device file
//! (C++, GPL-2.0-or-later), DrivenByMoss (Java, LGPL-3.0), bjoluc's
//! cubase-mcu-midiremote (TypeScript, MIT), Bitwig's extensions (Java, MIT)
//! and jskeet's DemoCode IconPlatform (C#, Apache-2.0); no code was copied.
//! The Platform M's layout is inferred from the M+'s (the references cover
//! both with one layout). No USB id is given by any of them. Untested on
//! hardware.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const PLATFORM_M_PLUS_ID: &str = "icon-platform-m-plus";
pub const PLATFORM_M_ID: &str = "icon-platform-m";

pub const ROWS: u8 = 6;
pub const COLS: u8 = 9;
/// Six rows of nine: four button rows, the fader touches, the bottom row.
pub const KEYS: u8 = ROWS * COLS;
/// The eight V-Pots, the jog wheel, the eight faders and the master fader.
pub const DIALS: u8 = 18;
/// The row of fader touches, which have no light.
const TOUCH_ROW: u8 = 4;
/// The keys with a light: all but the fader touches.
const LIT: usize = (KEYS - COLS) as usize;
const STRIPS: u8 = 8;
const JOG_DIAL: u8 = 8;
const FIRST_FADER_DIAL: u8 = 9;
/// The eight strip faders and the master.
const FADERS: usize = 9;

/// Each key's note, row by row.
const NOTES: [u8; KEYS as usize] = [
    // Select 1-8, Mixer.
    0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x54, // Mute 1-8, Read.
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x4a, // Solo 1-8, Write.
    0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x4b, // Rec 1-8, Cycle.
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x56,
    // Fader touch 1-8, master fader touch.
    0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70,
    // Channel < >, Bank < >, Rewind, Forward, Stop, Play, Record.
    0x30, 0x31, 0x2e, 0x2f, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f,
];

const NAMES: [&str; KEYS as usize] = [
    "Select 1",
    "Select 2",
    "Select 3",
    "Select 4",
    "Select 5",
    "Select 6",
    "Select 7",
    "Select 8",
    "Mixer",
    "Mute 1",
    "Mute 2",
    "Mute 3",
    "Mute 4",
    "Mute 5",
    "Mute 6",
    "Mute 7",
    "Mute 8",
    "Read",
    "Solo 1",
    "Solo 2",
    "Solo 3",
    "Solo 4",
    "Solo 5",
    "Solo 6",
    "Solo 7",
    "Solo 8",
    "Write",
    "Rec 1",
    "Rec 2",
    "Rec 3",
    "Rec 4",
    "Rec 5",
    "Rec 6",
    "Rec 7",
    "Rec 8",
    "Cycle",
    "Touch 1",
    "Touch 2",
    "Touch 3",
    "Touch 4",
    "Touch 5",
    "Touch 6",
    "Touch 7",
    "Touch 8",
    "Master Touch",
    "Channel <",
    "Channel >",
    "Bank <",
    "Bank >",
    "Rewind",
    "Forward",
    "Stop",
    "Play",
    "Record",
];

/// The keys with a light, for the Companion: every row but the touches.
pub const LIT_KEYS: [u8; LIT] = {
    let mut keys = [0u8; LIT];
    let mut key = 0;
    let mut at = 0;
    while key < KEYS {
        if key / COLS != TOUCH_ROW {
            keys[at] = key;
            at += 1;
        }
        key += 1;
    }
    keys
};

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const PITCH_BEND: u8 = 0xe0;
const VPOT_PUSH: u8 = 0x20;
const VPOT_CC: u8 = 0x10;
const JOG_CC: u8 = 0x3c;
/// Velocities over this press.
const PRESS_OVER: u8 = 64;
const SIGN: u8 = 0x40;
const STEPS: u8 = 0x3f;
/// More V-Pot steps than this in one message: the deck is in its standard
/// mode, sending positions.
const MAX_VPOT_STEPS: u8 = 0x0f;

const LED_OFF: u8 = 0;
const LED_ON: u8 = 0x7f;

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Fader units (of 16384) per dial step: 64 steps from end to end.
const FADER_STEP: i32 = 256;

/// The gap between two writes (none in tests).
const PACE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(1)
};
/// How far behind the clock may fall, so a burst after a quiet spell is
/// still paced.
const MAX_CREDIT: Duration = Duration::from_millis(10);
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

fn is_platform(model: &DeviceModel) -> bool {
    model.protocol == Protocol::IconPlatformMMidi
}

/// Whether a normalised port name is this model's: `PLATFORM M+` in any case
/// for the M+, `PLATFORM M` not followed by `+` for the M, and on Windows the
/// first interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if !is_platform(model) || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.to_ascii_uppercase();
    match model.id {
        PLATFORM_M_PLUS_ID => base.contains("PLATFORM M+"),
        PLATFORM_M_ID => base.match_indices("PLATFORM M").any(|(at, found)| {
            matches!(
                base.as_bytes().get(at + found.len()),
                None | Some(b' ' | b'V')
            )
        }),
        _ => false,
    }
}

/// The legend on each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    is_platform(model).then(|| NAMES.to_vec())
}

/// The keys with a light: every key but the fader touches.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    is_platform(model).then(|| LIT_KEYS.to_vec())
}

fn has_light(key: usize) -> bool {
    key / usize::from(COLS) != usize::from(TOUCH_ROW)
}

/// On for any colour that is not dark.
fn is_lit(color: Option<[u8; 3]>) -> bool {
    color.is_some_and(|color| color.iter().any(|channel| *channel >= DARK))
}

/// A signed-bit relative turn: bit 6 anticlockwise, bits 0-5 the steps,
/// none counting as one.
fn relative(value: u8) -> i8 {
    let steps = i8::try_from((value & STEPS).max(1)).unwrap_or(i8::MAX);
    if value & SIGN == 0 {
        steps
    } else {
        -steps
    }
}

/// A motor fader read as a dial: its first value seeds it, then every
/// [`FADER_STEP`] units moved turn the dial one step.
#[derive(Debug, Default, Clone, Copy)]
struct Fader {
    last: Option<u16>,
    rest: i32,
}

impl Fader {
    fn moved(&mut self, value: u16) -> i8 {
        let Some(old) = self.last.replace(value) else {
            return 0;
        };
        let moved = self.rest + i32::from(value) - i32::from(old);
        let steps = moved / FADER_STEP;
        self.rest = moved - steps * FADER_STEP;
        i8::try_from(steps.clamp(-127, 127)).unwrap_or(0)
    }
}

pub struct IconPlatform {
    transport: Box<dyn Transport>,
    /// Every LED off has been queued.
    started: bool,
    /// Writes waiting their turn, drained by poll and flush.
    outbox: VecDeque<[u8; 3]>,
    /// When the last write went out, as far as pacing counts.
    clock: Instant,
    pace: Duration,
    /// Each key's LED as staged, then as last queued.
    leds: [bool; KEYS as usize],
    queued: [bool; KEYS as usize],
    held: [bool; KEYS as usize],
    pushed: [bool; STRIPS as usize],
    faders: [Fader; FADERS],
    /// The standard-mode hint has been logged.
    warned: bool,
    parser: Parser,
}

impl IconPlatform {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            outbox: VecDeque::new(),
            clock: Instant::now(),
            pace: PACE,
            leds: [false; KEYS as usize],
            queued: [false; KEYS as usize],
            held: [false; KEYS as usize],
            pushed: [false; STRIPS as usize],
            faders: [Fader::default(); FADERS],
            warned: false,
            parser: Parser::default(),
        }
    }

    /// Every LED off, then whatever is already lit.
    fn start(&mut self) {
        self.started = true;
        self.outbox.clear();
        for (key, note) in NOTES.iter().enumerate() {
            if has_light(key) {
                self.outbox.push_back([NOTE_ON, *note, LED_OFF]);
            }
        }
        self.queued = [false; KEYS as usize];
        self.queue_changes();
    }

    /// Queues each LED that differs from what was queued, replacing a write
    /// for the same note still waiting.
    fn queue_changes(&mut self) {
        for (key, note) in NOTES.iter().enumerate() {
            let on = self.leds[key];
            if !has_light(key) || self.queued[key] == on {
                continue;
            }
            self.queued[key] = on;
            self.outbox.retain(|queued| queued[1] != *note);
            self.outbox
                .push_back([NOTE_ON, *note, if on { LED_ON } else { LED_OFF }]);
        }
    }

    /// Writes the queued messages whose gap has passed, never sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        let now = Instant::now();
        if let Some(floor) = now.checked_sub(MAX_CREDIT) {
            self.clock = self.clock.max(floor);
        }
        while let Some(message) = self.outbox.front() {
            let due = self.clock + self.pace;
            if due > now {
                break;
            }
            self.transport.write(message)?;
            self.outbox.pop_front();
            self.clock = due;
        }
        Ok(())
    }

    fn press(&mut self, key: usize, pressed: bool, events: &mut Vec<Input>) {
        let Some(held) = self.held.get_mut(key) else {
            return;
        };
        if *held != pressed {
            *held = pressed;
            events.push(Input::Key {
                index: u8::try_from(key).unwrap_or(u8::MAX),
                pressed,
            });
        }
    }

    fn note(&mut self, note: u8, pressed: bool, events: &mut Vec<Input>) {
        if let Some(key) = NOTES.iter().position(|candidate| *candidate == note) {
            self.press(key, pressed, events);
            return;
        }
        let strip = note.wrapping_sub(VPOT_PUSH);
        if let Some(held) = self.pushed.get_mut(usize::from(strip)) {
            if *held != pressed {
                *held = pressed;
                events.push(Input::DialPress {
                    index: strip,
                    pressed,
                });
            }
        }
    }

    fn control(&mut self, number: u8, value: u8, events: &mut Vec<Input>) {
        let index = match number {
            JOG_CC => JOG_DIAL,
            _ if (VPOT_CC..VPOT_CC + STRIPS).contains(&number) => {
                if value & STEPS > MAX_VPOT_STEPS {
                    if !self.warned {
                        self.warned = true;
                        tracing::warn!(
                            "the iCON Platform sent an absolute V-Pot value, so it is in its \
                             standard mode; power it up in Mackie Control mode for Ryu to read it"
                        );
                    }
                    return;
                }
                number - VPOT_CC
            }
            _ => return,
        };
        events.push(Input::DialTurn {
            index,
            delta: relative(value),
        });
    }

    fn fader(&mut self, channel: u8, lsb: u8, msb: u8, events: &mut Vec<Input>) {
        let Some(fader) = self.faders.get_mut(usize::from(channel)) else {
            return;
        };
        let value = u16::from(lsb & 0x7f) | u16::from(msb & 0x7f) << 7;
        let delta = fader.moved(value);
        if delta != 0 {
            events.push(Input::DialTurn {
                index: FIRST_FADER_DIAL + channel,
                delta,
            });
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        // SysEx is not used.
        let Message::Channel([status, data1, data2]) = *message else {
            return;
        };
        match status {
            NOTE_ON => self.note(data1, data2 > PRESS_OVER, events),
            NOTE_OFF => self.note(data1, false, events),
            CONTROL_CHANGE => self.control(data1, data2, events),
            _ if status & 0xf0 == PITCH_BEND => self.fader(status & 0x0f, data1, data2, events),
            _ => {}
        }
    }
}

impl Driver for IconPlatform {
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
        let key = usize::from(key);
        // The fader touches have no light.
        if has_light(key) {
            if let Some(led) = self.leds.get_mut(key) {
                *led = is_lit(face.map(|face| face.color));
            }
        }
        Ok(())
    }

    /// Every LED off the first time, then the LEDs that changed; whatever the
    /// pace allows goes out now, the rest from later polls.
    fn flush(&mut self) -> DriverResult<()> {
        if self.started {
            self.queue_changes();
        } else {
            self.start();
        }
        self.pump()
    }

    /// The Platform has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for IconPlatform {
    /// Turns every LED off at once, without pacing.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for (key, note) in NOTES.iter().enumerate() {
            if has_light(key) {
                let _ = self.transport.write(&[NOTE_ON, *note, LED_OFF]);
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

    fn open(id: &str) -> (IconPlatform, Recorder) {
        let recorder = Recorder::default();
        let deck = IconPlatform::new(by_id(id).unwrap(), Box::new(recorder.clone()));
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

    /// The notes of the 45 lit keys, in key order.
    fn lit_notes() -> Vec<u8> {
        LIT_KEYS
            .iter()
            .map(|key| NOTES[usize::from(*key)])
            .collect()
    }

    #[test]
    fn opening_turns_every_led_off_without_waiting() {
        assert!(PACE.is_zero());
        let (mut deck, recorder) = open(PLATFORM_M_PLUS_ID);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 45);
        for (write, note) in writes.iter().zip(lit_notes()) {
            assert_eq!(*write, vec![0x90, note, 0x00]);
        }
        // Never a fader touch's note.
        assert!(writes
            .iter()
            .all(|write| !(0x68..=0x70).contains(&write[1])));
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 45);
    }

    #[test]
    fn writes_are_paced_from_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open(PLATFORM_M_PLUS_ID);
        deck.pace = Duration::from_secs(60);
        deck.clock = Instant::now();
        let begun = Instant::now();
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert!(recorder.writes().is_empty());
        assert_eq!(deck.outbox.len(), 45);
        deck.pace = Duration::ZERO;
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 45);
        // Letting go is never paced.
        deck.pace = Duration::from_secs(60);
        let begun = Instant::now();
        drop(deck);
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert_eq!(recorder.writes().len(), 90);
    }

    #[test]
    fn faces_set_before_opening_replace_their_off_message() {
        let (mut deck, recorder) = open(PLATFORM_M_PLUS_ID);
        deck.set_face(8, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 45);
        assert_eq!(writes[44], vec![0x90, 0x54, 0x7f]);
        assert_eq!(writes.iter().filter(|write| write[1] == 0x54).count(), 1);
    }

    #[test]
    fn faces_light_their_buttons_on_flush() {
        let (mut deck, recorder) = open(PLATFORM_M_PLUS_ID);
        deck.flush().unwrap();
        deck.set_face(0, Some(&lit([255, 128, 0]))).unwrap();
        deck.set_face(17, Some(&lit([0, 0, 200]))).unwrap();
        deck.set_face(35, Some(&lit([40, 40, 40]))).unwrap();
        deck.set_face(53, Some(&lit([255, 255, 255]))).unwrap();
        // Dark and none stay off; touches have no light; past the end.
        deck.set_face(1, Some(&lit([0, 0, 0]))).unwrap();
        deck.set_face(2, Some(&lit([20, 20, 20]))).unwrap();
        deck.set_face(3, None).unwrap();
        deck.set_face(36, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(44, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(54, Some(&lit([255, 0, 0]))).unwrap();
        assert_eq!(recorder.writes().len(), 45);
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[45..],
            [
                vec![0x90, 0x18, 0x7f],
                vec![0x90, 0x4a, 0x7f],
                vec![0x90, 0x56, 0x7f],
                vec![0x90, 0x5f, 0x7f],
            ]
        );
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[49..], [vec![0x90, 0x18, 0x00]]);
        // Unchanged faces and brightness send nothing.
        deck.set_face(17, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_brightness(10).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 50);
        // Only Note On on channel 1 ever goes out.
        assert!(recorder.writes().iter().all(|write| write[0] == 0x90));
    }

    #[test]
    fn buttons_and_touches_are_keys() {
        let (mut deck, recorder) = open(PLATFORM_M_PLUS_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 0x18, 0x7f],
                &[0x90, 0x18, 0x00],
                &[0x90, 0x54, 0x7f],
                &[0x80, 0x54, 0x7f],
                &[0x90, 0x17, 0x41],
                &[0x90, 0x17, 0x40],
                &[0x90, 0x4b, 0x7f],
                &[0x90, 0x00, 0x7f],
                &[0x90, 0x56, 0x7f],
                &[0x90, 0x6f, 0x7f],
                &[0x90, 0x70, 0x7f],
                &[0x90, 0x70, 0x00],
                &[0x90, 0x30, 0x7f],
                &[0x90, 0x2f, 0x7f],
                &[0x90, 0x5e, 0x7f],
                &[0x90, 0x5f, 0x7f],
                // V-Pot pushes press their dials.
                &[0x90, 0x20, 0x7f],
                &[0x90, 0x27, 0x7f],
                &[0x90, 0x20, 0x00],
                // A repeated press, unknown notes, another channel, SysEx,
                // short and stray input.
                &[0x90, 0x5e, 0x7f],
                &[0x90, 0x75, 0x7f],
                &[0x90, 0x28, 0x7f],
                &[0x91, 0x18, 0x7f],
                &[0xf0, 0x00, 0x00, 0x66, 0x14, 0xf7],
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
                key(16, true),
                key(16, false),
                key(26, true),
                key(27, true),
                key(35, true),
                key(43, true),
                key(44, true),
                key(44, false),
                key(45, true),
                key(48, true),
                key(52, true),
                key(53, true),
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                Input::DialPress {
                    index: 7,
                    pressed: true
                },
                Input::DialPress {
                    index: 0,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn v_pots_and_the_jog_wheel_are_signed_bit_relative() {
        assert_eq!(
            (
                relative(0x01),
                relative(0x41),
                relative(0x40),
                relative(0x00)
            ),
            (1, -1, -1, 1)
        );
        assert_eq!((relative(0x3f), relative(0x7f)), (63, -63));
        let (mut deck, recorder) = open(PLATFORM_M_PLUS_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 0x10, 0x01],
                &[0xb0, 0x10, 0x41],
                &[0xb0, 0x17, 0x07],
                &[0xb0, 0x13, 0x47],
                &[0xb0, 0x3c, 0x01],
                &[0xb0, 0x3c, 0x45],
                &[0xb0, 0x3c, 0x00],
                // Standard mode's positions are ignored.
                &[0xb0, 0x11, 0x50],
                &[0xb0, 0x11, 0x10],
                // Rings, an expression pedal and another channel are not dials.
                &[0xb0, 0x30, 0x01],
                &[0xb0, 0x2e, 0x01],
                &[0xb1, 0x10, 0x01],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, 1),
                turn(0, -1),
                turn(7, 7),
                turn(3, -7),
                turn(8, 1),
                turn(8, -5),
                turn(8, 1),
            ]
        );
        assert!(deck.warned);
    }

    #[test]
    fn faders_turn_their_dials_after_their_first_value() {
        let (mut deck, recorder) = open(PLATFORM_M_PLUS_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                // Fader 1: seeded at 0x2000, then up 256, 255 more, 1 more,
                // and down to 0.
                &[0xe0, 0x00, 0x40],
                &[0xe0, 0x00, 0x42],
                &[0xe0, 0x7f, 0x43],
                &[0xe0, 0x00, 0x44],
                &[0xe0, 0x00, 0x00],
                // The master: seeded at 0, then the top.
                &[0xe8, 0x00, 0x00],
                &[0xe8, 0x70, 0x7f],
                // Channels 10-16 are not faders.
                &[0xe9, 0x00, 0x00],
                &[0xe9, 0x00, 0x7f],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(9, 1), turn(9, 1), turn(9, -34), turn(17, 63)]
        );
    }

    #[test]
    fn letting_go_turns_every_led_off() {
        let (mut deck, recorder) = open(PLATFORM_M_ID);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes.len(), 45);
        for (write, note) in writes.iter().zip(lit_notes()) {
            assert_eq!(*write, vec![0x90, note, 0x00]);
        }
        // Never opened: nothing sent.
        let (deck, recorder) = open(PLATFORM_M_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn each_platform_matches_only_its_own_port() {
        let cases = [
            ("Platform M+ V2.15", false, Some(PLATFORM_M_PLUS_ID)),
            ("Platform M+ V2.15", true, Some(PLATFORM_M_PLUS_ID)),
            ("Platform M+ V1.07", false, Some(PLATFORM_M_PLUS_ID)),
            (
                "Platform M+ V2.15:Platform M+ V2.15 MIDI 1 24:0",
                false,
                Some(PLATFORM_M_PLUS_ID),
            ),
            (
                "Platform M+ V2.15 MIDI 1 24:0",
                false,
                Some(PLATFORM_M_PLUS_ID),
            ),
            ("Platform M V1.14", false, Some(PLATFORM_M_ID)),
            ("PLATFORM M", false, Some(PLATFORM_M_ID)),
            ("Platform M V1.14 MIDI 1 24:0", false, Some(PLATFORM_M_ID)),
            ("MIDIIN2 (Platform M+ V2.15)", true, None),
            // Extenders and other decks.
            ("Platform X1 V2.15", false, None),
            (
                "Platform X+ V2.15",
                false,
                Some(crate::driver::icon_platform_x::PLATFORM_X_PLUS_ID),
            ),
            ("Platform MX", false, None),
            (
                "QCon Pro G2 V1.13",
                false,
                Some(crate::driver::icon_qcon::PRO_G2_ID),
            ),
            ("Midi Through:Midi Through Port-0 14:0", false, None),
        ];
        let plus = by_id(PLATFORM_M_PLUS_ID).unwrap();
        let m = by_id(PLATFORM_M_ID).unwrap();
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id),
                expected,
                "{raw}"
            );
            assert_eq!(
                port_matches(plus, &name),
                expected == Some(PLATFORM_M_PLUS_ID),
                "{raw}"
            );
            assert_eq!(
                port_matches(m, &name),
                expected == Some(PLATFORM_M_ID),
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_views_name_every_key_and_light_all_but_the_touches() {
        for id in [PLATFORM_M_PLUS_ID, PLATFORM_M_ID] {
            let model = by_id(id).unwrap();
            assert_eq!(model.dial_light_colors(), 0);
            let view = ModelView::from(model);
            let names = view.key_names.unwrap();
            assert_eq!(names.len(), 54);
            assert_eq!(
                (names[0], names[8], names[17], names[26], names[44], names[53]),
                (
                    "Select 1",
                    "Mixer",
                    "Read",
                    "Write",
                    "Master Touch",
                    "Record"
                )
            );
            let lit = view.lit_keys.unwrap();
            assert_eq!(lit.len(), 45);
            assert!(lit.iter().all(|key| !(36..=44).contains(key)));
            assert_eq!((view.keys, view.extra_keys, view.dials), (54, 0, 18));
            assert_eq!((view.rows, view.cols), (6, 9));
            assert_eq!(view.dial_lights, DialLights::None);
            assert_eq!(view.deck_lights, 0);
            assert_eq!(view.key_light_colors, "on_off");
            assert!(view.key_lights && !view.brightness);
        }
    }
}
