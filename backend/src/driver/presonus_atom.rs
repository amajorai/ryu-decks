//! PreSonus ATOM pad controller in its native (host-controlled) mode, over
//! class-compliant USB MIDI ([`super::midi`]).
//!
//! The ATOM lists one MIDI port each way named `ATOM` (the bare name, so the
//! shared `<token> MI` rule cannot take it). [`port_matches`] takes a name
//! with the word `atom` in any case, never one naming the ATOM SQ (a
//! different deck with another layout) or a longer word such as `atomic`, and
//! on Windows only the first interface.
//!
//! Native mode: `8F 00 7F` (Note Off, channel 16) turns it on and `8F 00 00`
//! turns it off, giving the deck back its own instrument modes. The deck
//! answers the first with `BF 7F 7F`, which Ryu does not wait for. Everything
//! else is on channel 1; other channels are dropped.
//!
//! In: the 16 velocity-sensitive pads are notes 36-51, the lowest row first,
//! so in reading order key `k` (row `k / 4`, column `k % 4`) is note
//! `48 - 4 * row + column`. A Note On above velocity 0 presses; a Note Off or
//! velocity 0 releases. The 20 buttons are momentary Control Change (127
//! pressed, 0 released), keys 16-35 in [`BUTTON_CCS`] order. The four endless
//! encoders are Control Change 14-17, dials 1-4, sign and size: 1-63 turn
//! clockwise by the value, 65-127 anticlockwise by the value less 64.
//! Aftertouch, Program Change, pitch bend, SysEx and unknown numbers are
//! dropped.
//!
//! Out: a pad's light is four Note Ons on its note: red, green and blue
//! (7-bit) on channels 2, 3 and 4, then channel 1 with 127 (lit steady) or 0
//! (off). The buttons' single-colour lights are their own Control Change, 127
//! on or 0 off: a colour that is not dark turns one on. There is no
//! brightness command, so Ryu scales the pad colours itself, and 0% turns
//! every light off. Writes are queued and drained a few at a time from
//! [`Driver::poll`] and [`Driver::flush`], so the shared poll loop never waits
//! for the deck. When Ryu lets go it turns every light off and native mode
//! off.
//!
//! Facts from Ableton Live's ATOM Remote Script and the `ableton.v3` control
//! surface framework it builds on, as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for
//! facts and the USB id only); no code was copied.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const ATOM_ID: &str = "presonus-atom";

/// The pads, keys 0-15.
pub const PADS: u8 = 16;
/// The buttons, keys 16-35.
pub const BUTTONS: u8 = 20;
pub const KEYS: u8 = PADS + BUTTONS;
pub const DIALS: u8 = 4;
const PAD_COLS: u8 = 4;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// The pad lights' red, green and blue parts, on channels 2, 3 and 4.
const RED_ON: u8 = NOTE_ON | 1;
const GREEN_ON: u8 = NOTE_ON | 2;
const BLUE_ON: u8 = NOTE_ON | 3;
/// The bottom-left pad.
const FIRST_NOTE: u8 = 36;
/// The top-left pad, key 0.
const TOP_NOTE: u8 = 48;
const LAST_NOTE: u8 = 51;
/// The first encoder's controller.
const FIRST_ENCODER: u8 = 14;

/// Native mode on and off: Note Off on channel 16, note 0.
pub const NATIVE_ON: [u8; 3] = [NOTE_OFF | 0x0f, 0x00, 0x7f];
pub const NATIVE_OFF: [u8; 3] = [NOTE_OFF | 0x0f, 0x00, 0x00];

/// A pad lit steady (1 blinks and 2 pulses; Ryu never sends those).
const PAD_STEADY: u8 = 0x7f;
const PAD_OFF: u8 = 0x00;
const LED_ON: u8 = 0x7f;
const LED_OFF: u8 = 0x00;

/// Each button's controller, keys 16-35 in this order.
pub const BUTTON_CCS: [u8; BUTTONS as usize] = [
    32, 104, 24, 25, 26, 27, 29, 30, 31, 85, 86, 87, 89, 90, 102, 103, 105, 107, 109, 111,
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
    "Shift",
    "Zoom",
    "Note Repeat",
    "Full Level",
    "Bank",
    "Preset",
    "Show/Hide",
    "Nudge",
    "Editor",
    "Set Loop",
    "Setup",
    "Up",
    "Down",
    "Left",
    "Right",
    "Select",
    "Click",
    "Record",
    "Play",
    "Stop",
];

/// The gap between two bursts of writes.
const PACE: Duration = Duration::from_millis(2);
/// Writes per burst.
const BURST: usize = 16;
/// A button's colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// Whether a normalised port name is an ATOM's: the word `atom` in any case,
/// never the ATOM SQ's, and on Windows the first interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.protocol != Protocol::PresonusAtomMidi || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.to_ascii_lowercase();
    let mut atom = false;
    for word in base.split(|c: char| !c.is_ascii_alphanumeric()) {
        if word.starts_with("sq") || word.starts_with("atomsq") {
            return false;
        }
        atom |= word == "atom";
    }
    atom
}

/// The legend on each key: the pads in reading order, then the buttons'
/// printed names.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::PresonusAtomMidi).then(|| NAMES.to_vec())
}

/// The key a pad note presses.
const fn key_of_note(note: u8) -> Option<u8> {
    if note < FIRST_NOTE || note > LAST_NOTE {
        return None;
    }
    let from_bottom = note - FIRST_NOTE;
    let row = PAD_COLS - 1 - from_bottom / PAD_COLS;
    Some(row * PAD_COLS + from_bottom % PAD_COLS)
}

/// The note of a pad key.
const fn note_of_key(key: u8) -> Option<u8> {
    if key >= PADS {
        return None;
    }
    Some(TOP_NOTE - PAD_COLS * (key / PAD_COLS) + key % PAD_COLS)
}

/// The key a button's controller presses.
fn key_of_cc(cc: u8) -> Option<u8> {
    BUTTON_CCS
        .iter()
        .position(|button| *button == cc)
        .and_then(|at| u8::try_from(at).ok())
        .map(|at| PADS + at)
}

/// The dial an encoder's controller turns.
const fn dial_of_cc(cc: u8) -> Option<u8> {
    if cc >= FIRST_ENCODER && cc < FIRST_ENCODER + DIALS {
        Some(cc - FIRST_ENCODER)
    } else {
        None
    }
}

/// An encoder's movement: bit 6 is the sign, bits 0-5 the size.
fn delta(value: u8) -> Option<i8> {
    match value {
        1..=63 => i8::try_from(value).ok(),
        65..=127 => i8::try_from(64 - i16::from(value)).ok(),
        _ => None,
    }
}

/// One 8-bit channel at a brightness, as the 7-bit value the deck takes.
fn level(channel: u8, percent: u8) -> u8 {
    let scaled = u16::from(channel) * u16::from(percent.min(100)) / 100;
    u8::try_from(scaled >> 1).unwrap_or(0x7f)
}

pub struct Atom {
    transport: Box<dyn Transport>,
    pace: Duration,
    /// Native mode and every light are queued.
    started: bool,
    /// Writes waiting their turn: up to [`BURST`] go out each time `pace`
    /// has passed, so the shared poll loop never sleeps for the deck.
    outbox: VecDeque<[u8; 3]>,
    /// When the next burst may go out.
    next_send: Instant,
    colors: [[u8; 3]; KEYS as usize],
    percent: u8,
    /// What each key's light was last queued as: a pad's 7-bit colour, or a
    /// button's state in the first byte; `None` until it is.
    sent: [Option<[u8; 3]>; KEYS as usize],
    held: [bool; KEYS as usize],
    parser: Parser,
}

impl Atom {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            pace: PACE,
            started: false,
            outbox: VecDeque::new(),
            next_send: Instant::now(),
            colors: [BLACK; KEYS as usize],
            percent: 100,
            sent: [None; KEYS as usize],
            held: [false; KEYS as usize],
            parser: Parser::default(),
        }
    }

    /// The light a key should show: a pad's 7-bit colour, or a button's
    /// state in the first byte.
    fn wanted(&self, key: u8) -> [u8; 3] {
        let color = self.colors.get(usize::from(key)).copied().unwrap_or(BLACK);
        if key < PADS {
            color.map(|channel| level(channel, self.percent))
        } else if self.percent > 0 && color.iter().any(|channel| *channel >= DARK) {
            [LED_ON, 0, 0]
        } else {
            [LED_OFF, 0, 0]
        }
    }

    /// Queues one key's light, replacing that key's light still waiting.
    fn queue(&mut self, key: u8, light: [u8; 3]) {
        if let Some(note) = note_of_key(key) {
            self.outbox
                .retain(|queued| !(matches!(queued[0], 0x90..=0x93) && queued[1] == note));
            let [red, green, blue] = light;
            let state = if light == BLACK { PAD_OFF } else { PAD_STEADY };
            self.outbox.extend([
                [RED_ON, note, red],
                [GREEN_ON, note, green],
                [BLUE_ON, note, blue],
                [NOTE_ON, note, state],
            ]);
        } else if let Some(cc) = BUTTON_CCS.get(usize::from(key.wrapping_sub(PADS))) {
            let cc = *cc;
            self.outbox
                .retain(|queued| !(queued[0] == CONTROL_CHANGE && queued[1] == cc));
            self.outbox.push_back([CONTROL_CHANGE, cc, light[0]]);
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

    /// Native mode on, then every light.
    fn start(&mut self) {
        self.started = true;
        self.outbox.clear();
        self.outbox.push_back(NATIVE_ON);
        self.sent = [None; KEYS as usize];
        self.queue_changes();
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
        // Native mode's answer (`BF 7F 7F`) and everything else off channel 1.
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
                if let Some(dial) = dial_of_cc(number) {
                    if let Some(delta) = delta(value) {
                        events.push(Input::DialTurn { index: dial, delta });
                    }
                } else if let Some(key) = key_of_cc(number) {
                    self.press(key, value > 0, events);
                }
            }
            _ => {}
        }
    }
}

impl Driver for Atom {
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
        Ok(())
    }

    /// Native mode and every light the first time, then only the lights that
    /// changed.
    fn flush(&mut self) -> DriverResult<()> {
        if self.started {
            self.queue_changes();
        } else {
            self.start();
        }
        self.pump()
    }

    /// The ATOM has no brightness command: Ryu scales the pad colours, and
    /// 0% turns every light off.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.percent = percent.min(100);
        if self.started {
            self.queue_changes();
            self.pump()?;
        }
        Ok(())
    }
}

impl Drop for Atom {
    /// Every light off, then native mode off, giving the deck back its own
    /// modes. Bursts are `pace` apart (zero in tests).
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        self.outbox.clear();
        for key in 0..KEYS {
            let light = if key < PADS { BLACK } else { [LED_OFF, 0, 0] };
            self.queue(key, light);
        }
        self.outbox.push_back(NATIVE_OFF);
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

    fn open() -> (Atom, Recorder) {
        let recorder = Recorder::default();
        let mut deck = Atom::new(by_id(ATOM_ID).unwrap(), Box::new(recorder.clone()));
        deck.pace = Duration::ZERO;
        (deck, recorder)
    }

    /// Native mode on, 16 pads of four writes and 20 buttons.
    const START: usize = 1 + 16 * 4 + 20;

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

    fn pad(note: u8, rgb: [u8; 3], state: u8) -> Vec<Vec<u8>> {
        vec![
            vec![0x91, note, rgb[0]],
            vec![0x92, note, rgb[1]],
            vec![0x93, note, rgb[2]],
            vec![0x90, note, state],
        ]
    }

    #[test]
    fn notes_and_controllers_map_to_keys_and_dials() {
        // The lowest row of notes is the bottom row of pads.
        assert_eq!(key_of_note(48), Some(0));
        assert_eq!(key_of_note(51), Some(3));
        assert_eq!(key_of_note(44), Some(4));
        assert_eq!(key_of_note(36), Some(12));
        assert_eq!(key_of_note(39), Some(15));
        for note in [0, 35, 52, 60, 127] {
            assert_eq!(key_of_note(note), None, "{note}");
        }
        for key in 0..PADS {
            assert_eq!(note_of_key(key).and_then(key_of_note), Some(key));
        }
        assert_eq!(note_of_key(PADS), None);
        assert_eq!(key_of_cc(32), Some(16));
        assert_eq!(key_of_cc(104), Some(17));
        assert_eq!(key_of_cc(111), Some(35));
        for cc in [0, 14, 28, 88, 127] {
            assert_eq!(key_of_cc(cc), None, "{cc}");
        }
        assert_eq!(dial_of_cc(14), Some(0));
        assert_eq!(dial_of_cc(17), Some(3));
        assert_eq!(dial_of_cc(13), None);
        assert_eq!(dial_of_cc(18), None);
        // Sign and size, not two's complement.
        assert_eq!(delta(1), Some(1));
        assert_eq!(delta(63), Some(63));
        assert_eq!(delta(65), Some(-1));
        assert_eq!(delta(127), Some(-63));
        assert_eq!(delta(0), None);
        assert_eq!(delta(64), None);
    }

    #[test]
    fn the_first_poll_turns_native_mode_on_and_sends_every_light() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(12, Some(&lit([0, 128, 64]))).unwrap();
        deck.set_face(16, Some(&lit([200, 200, 200]))).unwrap();
        deck.set_face(35, Some(&lit([10, 10, 10]))).unwrap();
        assert!(recorder.writes().is_empty());
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), START);
        assert_eq!(writes[0], vec![0x8f, 0x00, 0x7f]);
        // Key 0 is note 48: pure red, lit steady.
        assert_eq!(writes[1..5], pad(0x30, [0x7f, 0, 0], 0x7f));
        // Key 1 is note 49, black: off.
        assert_eq!(writes[5..9], pad(0x31, [0, 0, 0], 0x00));
        // Key 12 is note 36.
        assert_eq!(writes[49..53], pad(36, [0, 64, 32], 0x7f));
        // Shift on, Zoom off ... Stop off (too dark).
        assert_eq!(writes[65], vec![0xb0, 32, 0x7f]);
        assert_eq!(writes[66], vec![0xb0, 104, 0x00]);
        assert_eq!(writes[84], vec![0xb0, 111, 0x00]);
        // A flush with nothing changed sends nothing.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), START);
    }

    #[test]
    fn flushes_send_only_changed_lights_and_brightness_scales_pads() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), START);
        deck.set_face(3, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(34, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        let mut expected = pad(51, [0, 0, 0x7f], 0x7f);
        expected.push(vec![0xb0, 109, 0x7f]);
        assert_eq!(recorder.writes()[START..], expected);
        // Half brightness halves the pad; the button stays on.
        deck.set_brightness(50).unwrap();
        assert_eq!(recorder.writes()[START + 5..], pad(51, [0, 0, 63], 0x7f));
        // 0% turns every light off.
        deck.set_brightness(0).unwrap();
        let mut expected = pad(51, [0, 0, 0], 0x00);
        expected.push(vec![0xb0, 109, 0x00]);
        assert_eq!(recorder.writes()[START + 9..], expected);
    }

    #[test]
    fn pads_buttons_and_encoders_are_read_and_other_traffic_dropped() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                // Native mode's answer.
                &[0xbf, 0x7f, 0x7f],
                // Top-left pad, released by Note Off.
                &[0x90, 48, 0x40],
                &[0x80, 48, 0x00],
                // Bottom-left pad, released by velocity 0, in running status.
                &[0x90, 36, 0x7f, 36, 0x00],
                // Aftertouch and channel pressure.
                &[0xa0, 39, 0x30],
                &[0xd0, 0x30],
                // Shift, then Stop.
                &[0xb0, 32, 0x7f],
                &[0xb0, 32, 0x00],
                &[0xb0, 111, 0x7f],
                // Encoders: +1, -1, +5, -3, and no movement.
                &[0xb0, 14, 0x01],
                &[0xb0, 15, 0x41],
                &[0xb0, 16, 0x05],
                &[0xb0, 17, 0x43],
                &[0xb0, 17, 0x40],
                // Unused controllers and notes, other channels, Program
                // Change, pitch bend and SysEx.
                &[0xb0, 28, 0x7f],
                &[0xb0, 88, 0x7f],
                &[0x90, 52, 0x7f],
                &[0x99, 36, 0x7f],
                &[0xb1, 32, 0x7f],
                &[0xc0, 0x02],
                &[0xe0, 0x00, 0x40],
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
                key(12, true),
                key(12, false),
                key(16, true),
                key(16, false),
                key(35, true),
                turn(0, 1),
                turn(1, -1),
                turn(2, 5),
                turn(3, -3),
            ]
        );
        // Nothing is answered.
        assert_eq!(recorder.writes().len(), START);
    }

    #[test]
    fn writes_are_paced_in_bursts_without_sleeping() {
        let recorder = Recorder::default();
        let mut deck = Atom::new(by_id(ATOM_ID).unwrap(), Box::new(recorder.clone()));
        deck.pace = Duration::from_secs(3600);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), BURST);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), BURST);
        deck.next_send = Instant::now() - Duration::from_millis(1);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 2 * BURST);
        // A later light replaces the same pad's light still queued.
        deck.set_face(15, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(deck.outbox.len(), START - 2 * BURST);
        // Never sleep on drop in a test.
        deck.pace = Duration::ZERO;
        deck.next_send = Instant::now();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), START);
        let note = note_of_key(15).unwrap();
        assert!(writes
            .windows(4)
            .any(|four| four == pad(note, [0x7f, 0x7f, 0x7f], 0x7f).as_slice()));
        assert!(!writes
            .windows(4)
            .any(|four| four == pad(note, [0, 0, 0], 0).as_slice()));
    }

    #[test]
    fn letting_go_turns_lights_and_native_mode_off() {
        let (mut deck, recorder) = open();
        deck.set_face(5, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        drop(deck);
        let writes = &recorder.writes()[START..];
        assert_eq!(writes.len(), START);
        assert_eq!(writes[..4], pad(48, [0, 0, 0], 0));
        assert_eq!(writes[START - 2], vec![0xb0, 111, 0x00]);
        assert_eq!(writes[START - 1], vec![0x8f, 0x00, 0x00]);
        assert!(writes
            .iter()
            .all(|write| write.len() == 3 && (write[0] == 0x8f || write[2] == 0)));
        // Never opened: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_atom_ports_match() {
        let model = by_id(ATOM_ID).unwrap();
        let cases = [
            ("ATOM", false, true),
            ("ATOM", true, true),
            ("Atom", false, true),
            ("ATOM:ATOM MIDI 1 24:0", false, true),
            ("ATOM MIDI 1", false, true),
            ("MIDIIN2 (ATOM)", true, false),
            // The ATOM SQ and other words.
            ("ATOM SQ", false, false),
            ("ATOM SQ:ATOM SQ MIDI 1 24:0", false, false),
            ("ATOMSQ", false, false),
            ("Atomic Keys", false, false),
            ("Anatomy", false, false),
            ("LPD8", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(ATOM_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_names_every_key() {
        let view = ModelView::from(by_id(ATOM_ID).unwrap());
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), usize::from(KEYS));
        assert_eq!(names[0], "Pad 1");
        assert_eq!(names[16], "Shift");
        assert_eq!(names[35], "Stop");
        assert_eq!(
            (view.rows, view.cols, view.keys, view.dials, view.extra_keys),
            (9, 4, 36, 4, 20)
        );
        assert_eq!(view.lit_keys, None);
        assert_eq!(view.key_light_colors, "rgb");
        assert_eq!(view.dial_lights, DialLights::None);
        assert!(view.key_lights && view.brightness);
    }
}
