//! iCON Pro Audio's Platform X+ extender, over class-compliant USB MIDI
//! ([`super::midi`]), in its Mackie Control (extender) mode.
//!
//! Each unit lists one MIDI port each way, named for the unit number and the
//! firmware version: `Platform X+1 V2.13`, a second unit `Platform X+2 ...`,
//! and on Linux a ` MIDI 1` after it. Each unit is its own deck.
//! [`port_matches`] keys on `PLATFORM X+` in any case, so a Platform M+ or M
//! main unit and the older Platform X without the plus never match. Which
//! Windows interface carries it is not known, so any is taken.
//!
//! The unit is put in Mackie Control mode on the unit itself at power-up; in
//! its other modes it sends other messages. There is no handshake (the
//! extender's optional wake-up SysEx is not needed) and no keep-alive.
//!
//! In, on MIDI channel 1: every button, V-Pot push and fader touch is a Note
//! On, pressed at a velocity over 64 and released at 64 or under (a Note Off
//! releases too). Ryu lays the keys out 6 x 8, a column per strip, top down:
//! the V-Pot push (`0x20+s`), Select (`0x18+s`), Mute (`0x10+s`), Solo
//! (`0x08+s`), Rec (`0x00+s`) and the fader touch (`0x68+s`). The V-Pots are
//! `CC 0x10+s`, signed-bit relative: bit 6 set turns anticlockwise, bits 0-5
//! count the steps (0 counts as 1). The eight motor faders are pitch bend on
//! channels 1-8: the first value of each only seeds it, then every 256 of its
//! 14 bits (two of its upper seven) turn its dial a step. SysEx is ignored.
//!
//! Out, on channel 1: a strip button's LED is `90 <note> 7F` on and
//! `90 <note> 00` off (`01` would blink; Ryu does not use it); V-Pot pushes
//! and fader touches have no light. Opening queues every LED off; writes wait
//! in a queue drained by poll and flush, a millisecond apart (no wait in
//! tests), and Ryu never sleeps in the poll loop. Ryu never sends pitch bend
//! (the faders have motors), the V-Pot rings (`B0 30-37`, not confirmed on
//! this unit) or any display SysEx: the unit has no display. There is no
//! brightness command.
//!
//! Facts from Ardour's Mackie Control surface and its iCON device files (C++,
//! GPL-2.0-or-later), bjoluc's cubase-mcu-midiremote (TypeScript, MIT) and
//! DrivenByMoss (Java, LGPL-3.0); no code was copied. No USB id is given by
//! any of them. Untested on hardware.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const PLATFORM_X_PLUS_ID: &str = "icon-platform-x-plus";

pub const ROWS: u8 = 6;
pub const COLS: u8 = 8;
/// Six rows of eight: V-Pot pushes, four button rows, the fader touches.
pub const KEYS: u8 = ROWS * COLS;
/// The eight V-Pots, then the eight faders.
pub const DIALS: u8 = 16;
const STRIPS: u8 = 8;
/// The keys with a light: the Select, Mute, Solo and Rec rows.
const FIRST_LIT: u8 = COLS;
const LAST_LIT: u8 = 5 * COLS - 1;
const LIT: usize = (LAST_LIT - FIRST_LIT + 1) as usize;
const FIRST_FADER_DIAL: u8 = STRIPS;

/// The first note of each row, top down; strip `s` adds `s`.
const ROW_NOTES: [u8; ROWS as usize] = [0x20, 0x18, 0x10, 0x08, 0x00, 0x68];

/// Each key's note, row by row.
const NOTES: [u8; KEYS as usize] = {
    let mut notes = [0u8; KEYS as usize];
    let mut key = 0;
    while key < KEYS as usize {
        notes[key] = ROW_NOTES[key / COLS as usize] + (key % COLS as usize) as u8;
        key += 1;
    }
    notes
};

/// The keys with a light, for the Companion.
pub const LIT_KEYS: [u8; LIT] = {
    let mut keys = [0u8; LIT];
    let mut at = 0;
    while at < LIT {
        keys[at] = FIRST_LIT + at as u8;
        at += 1;
    }
    keys
};

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const PITCH_BEND: u8 = 0xe0;
const VPOT_CC: u8 = 0x10;
/// Velocities over this press.
const PRESS_OVER: u8 = 64;
const SIGN: u8 = 0x40;
const STEPS: u8 = 0x3f;

const LED_OFF: u8 = 0;
const LED_ON: u8 = 0x7f;

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Fader units (of 16384) per dial step: two of the upper seven bits.
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

fn is_platform_x(model: &DeviceModel) -> bool {
    model.protocol == Protocol::IconPlatformXPlusMidi
}

/// Whether a normalised port name is a Platform X+'s: `PLATFORM X+` in any
/// case, on any Windows interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    is_platform_x(model) && name.base.to_ascii_uppercase().contains("PLATFORM X+")
}

/// The legend on each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    is_platform_x(model).then(|| NAMES.to_vec())
}

/// The keys with a light: the Select, Mute, Solo and Rec rows.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    is_platform_x(model).then(|| LIT_KEYS.to_vec())
}

/// Each key's legend, row by row.
const NAMES: [&str; KEYS as usize] = {
    [
        "V-Pot 1", "V-Pot 2", "V-Pot 3", "V-Pot 4", "V-Pot 5", "V-Pot 6", "V-Pot 7", "V-Pot 8",
        "Select 1", "Select 2", "Select 3", "Select 4", "Select 5", "Select 6", "Select 7",
        "Select 8", "Mute 1", "Mute 2", "Mute 3", "Mute 4", "Mute 5", "Mute 6", "Mute 7", "Mute 8",
        "Solo 1", "Solo 2", "Solo 3", "Solo 4", "Solo 5", "Solo 6", "Solo 7", "Solo 8", "Rec 1",
        "Rec 2", "Rec 3", "Rec 4", "Rec 5", "Rec 6", "Rec 7", "Rec 8", "Touch 1", "Touch 2",
        "Touch 3", "Touch 4", "Touch 5", "Touch 6", "Touch 7", "Touch 8",
    ]
};

fn has_light(key: usize) -> bool {
    (usize::from(FIRST_LIT)..=usize::from(LAST_LIT)).contains(&key)
}

/// On for any colour that is not dark.
fn is_lit(color: Option<[u8; 3]>) -> bool {
    color.is_some_and(|color| color.iter().any(|channel| *channel >= DARK))
}

/// A signed-bit relative turn: bit 6 anticlockwise, bits 0-5 the steps,
/// none counting as one (so `0x00` is +1 and `0x40` is -1).
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

pub struct IconPlatformX {
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
    faders: [Fader; STRIPS as usize],
    parser: Parser,
}

impl IconPlatformX {
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
            faders: [Fader::default(); STRIPS as usize],
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

    fn note(&mut self, note: u8, pressed: bool, events: &mut Vec<Input>) {
        let Some(key) = NOTES.iter().position(|candidate| *candidate == note) else {
            return;
        };
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

    fn control(number: u8, value: u8, events: &mut Vec<Input>) {
        if (VPOT_CC..VPOT_CC + STRIPS).contains(&number) {
            events.push(Input::DialTurn {
                index: number - VPOT_CC,
                delta: relative(value),
            });
        }
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
        // SysEx (the unit's ready message among it) is not used.
        let Message::Channel([status, data1, data2]) = *message else {
            return;
        };
        match status {
            NOTE_ON => self.note(data1, data2 > PRESS_OVER, events),
            NOTE_OFF => self.note(data1, false, events),
            CONTROL_CHANGE => Self::control(data1, data2, events),
            _ if status & 0xf0 == PITCH_BEND => self.fader(status & 0x0f, data1, data2, events),
            _ => {}
        }
    }
}

impl Driver for IconPlatformX {
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
        // V-Pot pushes and fader touches have no light.
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

    /// The Platform X+ has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for IconPlatformX {
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

    const ROW_NAMES: [&str; 6] = ["V-Pot", "Select", "Mute", "Solo", "Rec", "Touch"];

    fn open() -> (IconPlatformX, Recorder) {
        let recorder = Recorder::default();
        let deck = IconPlatformX::new(
            by_id(PLATFORM_X_PLUS_ID).unwrap(),
            Box::new(recorder.clone()),
        );
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

    /// Select, Mute, Solo and Rec 1-8, in key order.
    fn lit_notes() -> Vec<u8> {
        [0x18u8, 0x10, 0x08, 0x00]
            .iter()
            .flat_map(|first| (0..8).map(move |strip| first + strip))
            .collect()
    }

    fn assert_no_pitch_bend(recorder: &Recorder) {
        assert!(recorder
            .writes()
            .iter()
            .all(|write| write.iter().all(|byte| byte & 0xf0 != 0xe0)));
    }

    #[test]
    fn the_note_table_follows_the_strips() {
        assert_eq!(
            &NOTES[..8],
            &[0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27]
        );
        assert_eq!((NOTES[8], NOTES[15]), (0x18, 0x1f));
        assert_eq!((NOTES[16], NOTES[24], NOTES[32]), (0x10, 0x08, 0x00));
        assert_eq!((NOTES[40], NOTES[47]), (0x68, 0x6f));
        assert_eq!(LIT_KEYS.to_vec(), (8..=39).collect::<Vec<u8>>());
    }

    #[test]
    fn opening_turns_every_led_off_without_waiting() {
        assert!(PACE.is_zero());
        let (mut deck, recorder) = open();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 32);
        for (write, note) in writes.iter().zip(lit_notes()) {
            assert_eq!(*write, vec![0x90, note, 0x00]);
        }
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 32);
        assert_no_pitch_bend(&recorder);
    }

    #[test]
    fn writes_are_paced_from_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open();
        deck.pace = Duration::from_secs(60);
        deck.clock = Instant::now();
        let begun = Instant::now();
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert!(recorder.writes().is_empty());
        assert_eq!(deck.outbox.len(), 32);
        deck.pace = Duration::ZERO;
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 32);
        // Letting go is never paced.
        deck.pace = Duration::from_secs(60);
        let begun = Instant::now();
        drop(deck);
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert_eq!(recorder.writes().len(), 64);
    }

    #[test]
    fn faces_light_only_the_strip_buttons() {
        let (mut deck, recorder) = open();
        deck.set_face(8, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 32);
        assert_eq!(writes[31], vec![0x90, 0x18, 0x7f]);
        assert_eq!(writes.iter().filter(|write| write[1] == 0x18).count(), 1);
        deck.set_face(16, Some(&lit([0, 0, 200]))).unwrap();
        deck.set_face(31, Some(&lit([40, 40, 40]))).unwrap();
        deck.set_face(39, Some(&lit([255, 255, 255]))).unwrap();
        // Dark and none stay off; pushes and touches have no light; past the end.
        deck.set_face(9, Some(&lit([20, 20, 20]))).unwrap();
        deck.set_face(10, None).unwrap();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(40, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(48, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[32..],
            [
                vec![0x90, 0x10, 0x7f],
                vec![0x90, 0x0f, 0x7f],
                vec![0x90, 0x07, 0x7f],
            ]
        );
        deck.set_face(8, None).unwrap();
        deck.set_brightness(10).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[35..], [vec![0x90, 0x18, 0x00]]);
        // Only Note On on channel 1 ever goes out.
        assert!(recorder.writes().iter().all(|write| write[0] == 0x90));
        assert_no_pitch_bend(&recorder);
    }

    #[test]
    fn buttons_pushes_and_touches_are_keys() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 0x20, 0x7f],
                &[0x90, 0x20, 0x00],
                &[0x90, 0x27, 0x7f],
                &[0x80, 0x27, 0x7f],
                &[0x90, 0x18, 0x7f],
                &[0x90, 0x17, 0x41],
                &[0x90, 0x17, 0x40],
                &[0x90, 0x08, 0x7f],
                &[0x90, 0x00, 0x7f],
                &[0x90, 0x07, 0x7f],
                &[0x90, 0x68, 0x7f],
                &[0x90, 0x6f, 0x7f],
                &[0x90, 0x6f, 0x00],
                // A repeated press, unknown notes, another channel, SysEx,
                // short and stray input.
                &[0x90, 0x07, 0x7f],
                &[0x90, 0x70, 0x7f],
                &[0x90, 0x54, 0x7f],
                &[0x91, 0x18, 0x7f],
                &[0xf0, 0x00, 0x00, 0x66, 0x15, 0x01, 0xf7],
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
                key(23, true),
                key(23, false),
                key(24, true),
                key(32, true),
                key(39, true),
                key(40, true),
                key(47, true),
                key(47, false),
            ]
        );
    }

    #[test]
    fn v_pots_are_signed_bit_relative() {
        assert_eq!(
            (
                relative(0x00),
                relative(0x01),
                relative(0x40),
                relative(0x41)
            ),
            (1, 1, -1, -1)
        );
        assert_eq!((relative(0x3f), relative(0x7f)), (63, -63));
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 0x10, 0x01],
                &[0xb0, 0x10, 0x41],
                &[0xb0, 0x17, 0x07],
                &[0xb0, 0x13, 0x47],
                &[0xb0, 0x11, 0x00],
                &[0xb0, 0x11, 0x40],
                // Rings, the jog CC, another channel are not dials.
                &[0xb0, 0x30, 0x01],
                &[0xb0, 0x3c, 0x01],
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
                turn(1, 1),
                turn(1, -1),
            ]
        );
    }

    #[test]
    fn faders_turn_their_dials_after_their_first_value() {
        let (mut deck, recorder) = open();
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
                // Fader 8: seeded at 0, then the top.
                &[0xe7, 0x00, 0x00],
                &[0xe7, 0x70, 0x7f],
                // Channel 9 and up are not faders.
                &[0xe8, 0x00, 0x00],
                &[0xe8, 0x00, 0x7f],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(8, 1), turn(8, 1), turn(8, -34), turn(15, 63)]
        );
        assert_no_pitch_bend(&recorder);
    }

    #[test]
    fn letting_go_turns_every_led_off() {
        let (mut deck, recorder) = open();
        deck.set_face(8, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes.len(), 32);
        for (write, note) in writes.iter().zip(lit_notes()) {
            assert_eq!(*write, vec![0x90, note, 0x00]);
        }
        assert_no_pitch_bend(&recorder);
        // Never opened: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_a_platform_x_plus_port_matches() {
        let x_plus = by_id(PLATFORM_X_PLUS_ID).unwrap();
        let cases = [
            ("Platform X+1 V2.13", false, Some(PLATFORM_X_PLUS_ID)),
            ("Platform X+2 V2.13", true, Some(PLATFORM_X_PLUS_ID)),
            ("platform x+1 v2.13", false, Some(PLATFORM_X_PLUS_ID)),
            (
                "Platform X+1 V2.13:Platform X+1 V2.13 MIDI 1 24:0",
                false,
                Some(PLATFORM_X_PLUS_ID),
            ),
            (
                "MIDIIN2 (Platform X+1 V2.13)",
                true,
                Some(PLATFORM_X_PLUS_ID),
            ),
            (
                "Platform M+ V1.07",
                false,
                Some(crate::driver::icon_platform::PLATFORM_M_PLUS_ID),
            ),
            (
                "Platform M V1.14",
                false,
                Some(crate::driver::icon_platform::PLATFORM_M_ID),
            ),
            ("Platform X", false, None),
            ("Platform X2", false, None),
            ("Platform X1 V2.15", false, None),
            ("Midi Through:Midi Through Port-0 14:0", false, None),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id),
                expected,
                "{raw}"
            );
            assert_eq!(
                port_matches(x_plus, &name),
                expected == Some(PLATFORM_X_PLUS_ID),
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_names_every_key_and_lights_the_strip_buttons() {
        let model = by_id(PLATFORM_X_PLUS_ID).unwrap();
        assert_eq!(model.dial_light_colors(), 0);
        let view = ModelView::from(model);
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), 48);
        for (key, name) in names.iter().enumerate() {
            assert_eq!(
                *name,
                format!("{} {}", ROW_NAMES[key / 8], key % 8 + 1),
                "{key}"
            );
        }
        assert_eq!(view.lit_keys.unwrap(), (8..=39).collect::<Vec<u8>>());
        assert_eq!((view.keys, view.extra_keys, view.dials), (48, 0, 16));
        assert_eq!((view.rows, view.cols), (6, 8));
        assert_eq!(view.dial_lights, DialLights::None);
        assert_eq!(view.deck_lights, 0);
        assert_eq!(view.key_light_colors, "on_off");
        assert!(view.key_lights && !view.brightness);
        assert_eq!(view.vendor_id, "0000");
    }
}
