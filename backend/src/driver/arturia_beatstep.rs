//! Arturia's BeatStep pad controller, over class-compliant USB MIDI
//! ([`super::midi`]).
//!
//! The BeatStep lists one MIDI port each way, called `Arturia BeatStep` (the
//! bare name on macOS and Windows, so the shared `<token> MI` rule cannot take
//! it; ALSA adds ` MIDI 1` and its client numbers). [`port_matches`] takes a
//! name containing `Arturia BeatStep` that is not a BeatStep Pro, and on
//! Windows only the first interface.
//!
//! The deck keeps whatever mapping the user last loaded, so Ryu writes its
//! own into the deck's working memory before it reads anything: about a
//! second after the port opens (as Ableton waits), one SysEx per property,
//! each `F0 00 20 6B 7F 42 02 00 <property> <control> <value> F7`, a
//! millisecond apart because the deck drops messages that come faster. The
//! writes wait in a queue that each poll and flush drains as their turn
//! comes, so the deck never makes the shared poll loop sleep.
//! Property 01 is the mode, 02 the MIDI channel (from 0), 03 the note or
//! controller number and 06 the mode option. In order:
//!
//! - the 16 encoders (controls 0x20-0x2F, top row then bottom row, left to
//!   right) and the large encoder (0x30): mode 01, their own control number as
//!   controller, channel 1, option 02 (relative, two's complement);
//! - Shift (0x5E): mode 09 (note), note 7, channel 10, option 01 (momentary);
//! - Stop (0x59) and Play (0x58), as Ableton sets them: mode 08 (controller),
//!   controllers 1 and 2, channel 1, option 01;
//! - the 16 pads (0x70-0x7F, top row then bottom row, left to right): mode 09,
//!   notes 36-51, channel 10, option 01.
//!
//! That is 144 writes, then the 16 pad lights. Nothing is ever stored to a
//! preset slot (command 06), so the deck's Recall button brings back the
//! user's own mapping once Ryu lets go. (Command 01 reads a property, an
//! 11-byte `F0 00 20 6B 7F 42 01 00 <property> <control> F7`; Ryu does not use
//! it.)
//!
//! In, with that mapping: pad `k` (keys 0-15, reading order) is note 36 + k
//! and Shift (key 16) note 7, both on channel 10; a Note On above velocity 0
//! presses, a Note Off or velocity 0 releases. Stop (key 17) and Play (key 18)
//! are controllers 1 and 2 on channel 1, pressed above 0 and released at 0.
//! Encoder `d` (dials 0-15) is controller 0x20 + d and the large encoder (dial
//! 16) controller 0x30, on channel 1, each a relative two's-complement step
//! (1-63 clockwise, 65-127 counter-clockwise). Everything else, aftertouch
//! included, is ignored. The encoders have no push switch.
//!
//! Out: each pad has a red and a blue LED, set with property 0x10: 0x00 off,
//! 0x01 red, 0x10 blue, 0x11 both (pink). There is no dimming. Shift, Stop and
//! Play have no light Ryu can set. The deck turns a pad's LEDs on and off by
//! itself as it is pressed, and clears them all when Shift is released, so Ryu
//! sends a pad's light again after each release and every light after Shift
//! is released. When Ryu lets go, every pad that may be lit goes dark.
//!
//! Facts from Ableton's BeatStep and _Arturia Remote Scripts as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for
//! facts and the USB id only), DrivenByMoss (LGPL-3.0, Jürgen Moßgraber) for
//! the port name, pad lights and their re-sends, and the beatstep npm package
//! (MIT, konsumer) for the large encoder's and Shift's control numbers and the
//! read command; no code was copied.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const BEATSTEP_ID: &str = "arturia-beatstep";

pub const KEYS: u8 = 19;
pub const DIALS: u8 = 17;
/// Shift, Stop and Play, after the 16 pads.
pub const EXTRA_KEYS: u8 = 3;
const PADS: u8 = 16;
const SHIFT_KEY: u8 = 16;
const STOP_KEY: u8 = 17;
const PLAY_KEY: u8 = 18;
/// Two rows of ten: Shift, Stop and Play in the two left-hand columns, then
/// the pads.
pub const ROWS: u8 = 2;
pub const COLS: u8 = 10;
const PAD_COLS: u8 = 8;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// MIDI channel 10, from 0.
const PAD_CHANNEL: u8 = 9;
const FIRST_PAD_NOTE: u8 = 0x24;
const SHIFT_NOTE: u8 = 0x07;
const STOP_CC: u8 = 0x01;
const PLAY_CC: u8 = 0x02;
const FIRST_ENCODER: u8 = 0x20;
const MASTER_ENCODER: u8 = 0x30;

/// Every write to the deck's working memory starts so.
const WRITE_HEAD: [u8; 8] = [0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x00];
const SYSEX_END: u8 = 0xf7;
const MODE: u8 = 0x01;
const CHANNEL: u8 = 0x02;
const IDENTIFIER: u8 = 0x03;
const OPTION: u8 = 0x06;
const COLOUR: u8 = 0x10;
const ENCODER_MODE: u8 = 0x01;
const BUTTON_CC_MODE: u8 = 0x08;
const BUTTON_NOTE_MODE: u8 = 0x09;
const TWOS_COMPLEMENT: u8 = 0x02;
const MOMENTARY: u8 = 0x01;
const FIRST_PAD: u8 = 0x70;
const SHIFT: u8 = 0x5e;
const STOP: u8 = 0x59;
const PLAY: u8 = 0x58;

pub const LED_OFF: u8 = 0x00;
pub const LED_RED: u8 = 0x01;
pub const LED_BLUE: u8 = 0x10;
pub const LED_PINK: u8 = 0x11;

/// How long after the port opens Ryu writes its mapping (as Ableton waits).
const SETTLE: Duration = Duration::from_secs(1);
/// The gap between two writes: the deck drops messages that come faster.
const PACE: Duration = Duration::from_millis(1);

/// A colour is dark (its pad off) while every channel is under this.
const DARK: u8 = 32;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const NAMES: [&str; KEYS as usize] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16", "Shift", "Stop", "Play",
];

/// Whether a normalised port name is a BeatStep's (never a BeatStep Pro's),
/// on Windows its first interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.protocol == Protocol::ArturiaBeatStepMidi
        && name.interface.unwrap_or(1) == 1
        && name.base.contains("Arturia BeatStep")
        && !name.base.to_ascii_lowercase().contains("beatstep pro")
}

/// The legend on each key: the pads in reading order, then Shift, Stop and
/// Play.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::ArturiaBeatStepMidi).then(|| NAMES.to_vec())
}

/// Only the pads have lights.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    (model.protocol == Protocol::ArturiaBeatStepMidi).then(|| (0..PADS).collect())
}

/// Places on a 2 x 10 grid: Shift top left, Stop and Play bottom left, the
/// pads in the eight columns to their right.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    (model.protocol == Protocol::ArturiaBeatStepMidi).then(|| {
        (0..KEYS)
            .map(|key| {
                let (row, col) = match key {
                    SHIFT_KEY => (0, 0),
                    STOP_KEY => (1, 0),
                    PLAY_KEY => (1, 1),
                    pad => (pad / PAD_COLS, COLS - PAD_COLS + pad % PAD_COLS),
                };
                u16::from(row) * u16::from(COLS) + u16::from(col)
            })
            .collect()
    })
}

/// One write to the deck's working memory.
const fn write(property: u8, control: u8, value: u8) -> [u8; 12] {
    let [a, b, c, d, e, f, g, h] = WRITE_HEAD;
    [a, b, c, d, e, f, g, h, property, control, value, SYSEX_END]
}

/// The four writes that program one control.
fn program(out: &mut Vec<[u8; 12]>, control: u8, mode: u8, number: u8, channel: u8, option: u8) {
    out.push(write(MODE, control, mode));
    out.push(write(IDENTIFIER, control, number));
    out.push(write(CHANNEL, control, channel));
    out.push(write(OPTION, control, option));
}

/// Ryu's mapping, in the order it is written: encoders, the large encoder,
/// Shift, Stop, Play, then the pads.
fn setup_messages() -> Vec<[u8; 12]> {
    let mut out = Vec::with_capacity(144);
    for control in (FIRST_ENCODER..FIRST_ENCODER + 16).chain([MASTER_ENCODER]) {
        program(&mut out, control, ENCODER_MODE, control, 0, TWOS_COMPLEMENT);
    }
    program(
        &mut out,
        SHIFT,
        BUTTON_NOTE_MODE,
        SHIFT_NOTE,
        PAD_CHANNEL,
        MOMENTARY,
    );
    program(&mut out, STOP, BUTTON_CC_MODE, STOP_CC, 0, MOMENTARY);
    program(&mut out, PLAY, BUTTON_CC_MODE, PLAY_CC, 0, MOMENTARY);
    for pad in 0..PADS {
        program(
            &mut out,
            FIRST_PAD + pad,
            BUTTON_NOTE_MODE,
            FIRST_PAD_NOTE + pad,
            PAD_CHANNEL,
            MOMENTARY,
        );
    }
    out
}

/// A pad light's state: off when dark; red for reds, oranges and yellows;
/// blue for greens, cyans and blues; pink (both) for purples, magentas,
/// whites and greys.
#[must_use]
pub fn led_state(color: [u8; 3]) -> u8 {
    let [red, green, blue] = color.map(i32::from);
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    if max < i32::from(DARK) {
        return LED_OFF;
    }
    let chroma = max - min;
    // Under a quarter saturated: white or grey.
    if chroma * 4 < max {
        return LED_PINK;
    }
    let hue = if max == red {
        (60 * (green - blue) / chroma).rem_euclid(360)
    } else if max == green {
        120 + 60 * (blue - red) / chroma
    } else {
        240 + 60 * (red - green) / chroma
    };
    match hue {
        75..=269 => LED_BLUE,
        270..=329 => LED_PINK,
        _ => LED_RED,
    }
}

/// A relative two's-complement step: 1-63 clockwise, 65-127 counter-clockwise.
fn delta(value: u8) -> Option<i8> {
    match value {
        1..=63 => i8::try_from(value).ok(),
        65..=127 => i8::try_from(i16::from(value) - 128).ok(),
        _ => None,
    }
}

pub struct BeatStep {
    transport: Box<dyn Transport>,
    opened: Instant,
    settle: Duration,
    pace: Duration,
    /// Ryu's mapping and the first lights are queued.
    set_up: bool,
    /// Writes waiting their turn: one goes out each time `pace` has passed,
    /// so the shared poll loop never sleeps for the deck.
    outbox: VecDeque<[u8; 12]>,
    /// When the next queued write may go out.
    next_send: Instant,
    colors: Vec<[u8; 3]>,
    /// The state each pad was last sent; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    parser: Parser,
}

impl BeatStep {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            opened: Instant::now(),
            settle: SETTLE,
            pace: PACE,
            set_up: false,
            outbox: VecDeque::new(),
            next_send: Instant::now(),
            colors: vec![BLACK; usize::from(PADS)],
            sent: vec![None; usize::from(PADS)],
            held: vec![false; usize::from(KEYS)],
            parser: Parser::default(),
        }
    }

    /// Queues one message; a pad light replaces that pad's light still
    /// waiting in the queue.
    fn send(&mut self, message: [u8; 12]) {
        if message[8] == COLOUR {
            self.outbox
                .retain(|queued| queued[8] != COLOUR || queued[9] != message[9]);
        }
        self.outbox.push_back(message);
    }

    /// Writes the queued messages whose turn has come, the gap the deck needs
    /// apart, without ever sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        while let Some(message) = self.outbox.front() {
            let now = Instant::now();
            if now < self.next_send {
                break;
            }
            self.transport.write(message)?;
            self.outbox.pop_front();
            self.next_send = now + self.pace;
        }
        Ok(())
    }

    /// Whether some of Ryu's mapping is still waiting to be written.
    fn mapping_pending(&self) -> bool {
        self.outbox.iter().any(|queued| queued[8] != COLOUR)
    }

    /// A pad's light, sent whether or not it changed.
    fn send_pad(&mut self, pad: u8) -> DriverResult<()> {
        let Some(color) = self.colors.get(usize::from(pad)) else {
            return Ok(());
        };
        let state = led_state(*color);
        self.send(write(COLOUR, FIRST_PAD + pad, state));
        if let Some(sent) = self.sent.get_mut(usize::from(pad)) {
            *sent = Some(state);
        }
        Ok(())
    }

    /// Queues Ryu's mapping, then every light.
    fn set_up(&mut self) -> DriverResult<()> {
        for message in setup_messages() {
            self.send(message);
        }
        self.set_up = true;
        self.flush()
    }

    /// Reads whatever arrived, waiting up to `timeout_ms` for the first.
    fn read(&mut self, timeout_ms: i32) -> DriverResult<Vec<Message>> {
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
        Ok(messages)
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

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        let (kind, channel) = (status & 0xf0, status & 0x0f);
        match (kind, channel) {
            (NOTE_ON | NOTE_OFF, PAD_CHANNEL) => {
                let key = match number {
                    SHIFT_NOTE => SHIFT_KEY,
                    note if (FIRST_PAD_NOTE..FIRST_PAD_NOTE + PADS).contains(&note) => {
                        note - FIRST_PAD_NOTE
                    }
                    _ => return,
                };
                self.press(key, kind == NOTE_ON && value > 0, events);
            }
            (CONTROL_CHANGE, 0) => match number {
                STOP_CC => self.press(STOP_KEY, value > 0, events),
                PLAY_CC => self.press(PLAY_KEY, value > 0, events),
                cc if (FIRST_ENCODER..FIRST_ENCODER + 16).contains(&cc) || cc == MASTER_ENCODER => {
                    let dial = if cc == MASTER_ENCODER {
                        16
                    } else {
                        cc - FIRST_ENCODER
                    };
                    if let Some(delta) = delta(value) {
                        events.push(Input::DialTurn { index: dial, delta });
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

impl Driver for BeatStep {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.set_up {
            let left = self.settle.saturating_sub(self.opened.elapsed());
            if !left.is_zero() {
                // Anything arriving under the user's own mapping is dropped.
                let wait = i32::try_from(left.as_millis())
                    .unwrap_or(i32::MAX)
                    .min(timeout_ms);
                self.read(wait)?;
                return Ok(Vec::new());
            }
            self.parser = Parser::default();
            self.set_up()?;
        }
        if self.mapping_pending() {
            // Input under a half-written mapping is dropped.
            self.pump()?;
            self.read(timeout_ms)?;
            self.parser = Parser::default();
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        for message in &self.read(timeout_ms)? {
            self.receive(message, &mut events);
        }
        // The deck changes pad lights by itself: put them back.
        for event in &events {
            match *event {
                Input::Key {
                    index: SHIFT_KEY,
                    pressed: false,
                } => {
                    for pad in 0..PADS {
                        self.send_pad(pad)?;
                    }
                }
                Input::Key {
                    index,
                    pressed: false,
                } if index < PADS => self.send_pad(index)?,
                _ => {}
            }
        }
        self.pump()?;
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// One write for each pad light whose state changed (every one the first
    /// time); nothing until Ryu's mapping is written.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.set_up {
            return Ok(());
        }
        for pad in 0..PADS {
            let (Some(color), Some(sent)) = (
                self.colors.get(usize::from(pad)),
                self.sent.get(usize::from(pad)),
            ) else {
                continue;
            };
            if *sent != Some(led_state(*color)) {
                self.send_pad(pad)?;
            }
        }
        self.pump()
    }

    /// The BeatStep has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for BeatStep {
    /// Turns every pad light off that may be on: the deck would otherwise
    /// keep them. Only those writes wait the deck's gap, once, here.
    fn drop(&mut self) {
        if !self.set_up {
            return;
        }
        let unsent = !self.outbox.is_empty();
        self.outbox.clear();
        let mut first = true;
        for pad in 0..PADS {
            let index = usize::from(pad);
            let dark = self.sent.get(index) == Some(&Some(LED_OFF))
                && self.held.get(index) == Some(&false);
            if dark && !unsent {
                continue;
            }
            if !first && !self.pace.is_zero() {
                std::thread::sleep(self.pace);
            }
            first = false;
            // An unplugged deck fails its first write: stop there.
            if self
                .transport
                .write(&write(COLOUR, FIRST_PAD + pad, LED_OFF))
                .is_err()
            {
                break;
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

    fn open() -> (BeatStep, Recorder) {
        let recorder = Recorder::default();
        let mut deck = BeatStep::new(by_id(BEATSTEP_ID).unwrap(), Box::new(recorder.clone()));
        deck.settle = Duration::ZERO;
        deck.pace = Duration::ZERO;
        (deck, recorder)
    }

    /// A deck past its setup, with the setup's writes left out.
    fn ready() -> (BeatStep, Recorder) {
        let (mut deck, recorder) = open();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 160);
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

    fn sysex(property: u8, control: u8, value: u8) -> Vec<u8> {
        vec![
            0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x00, property, control, value, 0xf7,
        ]
    }

    #[test]
    fn the_setup_programs_every_control_in_order_then_the_lights() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        // Nothing is written before the setup.
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 144 + 16);
        assert!(writes.iter().all(|write| write.len() == 12));
        // The first encoder, the last and the large one.
        assert_eq!(
            writes[..4],
            [
                sysex(0x01, 0x20, 0x01),
                sysex(0x03, 0x20, 0x20),
                sysex(0x02, 0x20, 0x00),
                sysex(0x06, 0x20, 0x02)
            ]
        );
        assert_eq!(
            writes[60..64],
            [
                sysex(0x01, 0x2f, 0x01),
                sysex(0x03, 0x2f, 0x2f),
                sysex(0x02, 0x2f, 0x00),
                sysex(0x06, 0x2f, 0x02)
            ]
        );
        assert_eq!(
            writes[64..68],
            [
                sysex(0x01, 0x30, 0x01),
                sysex(0x03, 0x30, 0x30),
                sysex(0x02, 0x30, 0x00),
                sysex(0x06, 0x30, 0x02)
            ]
        );
        // Shift as a note on channel 10; Stop and Play as Ableton sets them.
        assert_eq!(
            writes[68..72],
            [
                sysex(0x01, 0x5e, 0x09),
                sysex(0x03, 0x5e, 0x07),
                sysex(0x02, 0x5e, 0x09),
                sysex(0x06, 0x5e, 0x01)
            ]
        );
        assert_eq!(
            writes[72..76],
            [
                sysex(0x01, 0x59, 0x08),
                sysex(0x03, 0x59, 0x01),
                sysex(0x02, 0x59, 0x00),
                sysex(0x06, 0x59, 0x01)
            ]
        );
        assert_eq!(
            writes[76..80],
            [
                sysex(0x01, 0x58, 0x08),
                sysex(0x03, 0x58, 0x02),
                sysex(0x02, 0x58, 0x00),
                sysex(0x06, 0x58, 0x01)
            ]
        );
        // The pads, first and last.
        assert_eq!(
            writes[80..84],
            [
                sysex(0x01, 0x70, 0x09),
                sysex(0x03, 0x70, 0x24),
                sysex(0x02, 0x70, 0x09),
                sysex(0x06, 0x70, 0x01)
            ]
        );
        assert_eq!(
            writes[140..144],
            [
                sysex(0x01, 0x7f, 0x09),
                sysex(0x03, 0x7f, 0x33),
                sysex(0x02, 0x7f, 0x09),
                sysex(0x06, 0x7f, 0x01)
            ]
        );
        // Every control gets mode, number, channel and option, in that order.
        for (index, write) in writes[..144].iter().enumerate() {
            assert_eq!(write[8], [0x01, 0x03, 0x02, 0x06][index % 4], "{index}");
            assert_eq!(write[6..8], [0x02, 0x00], "{index}");
        }
        // Then every pad light; nothing is stored to a preset.
        assert_eq!(writes[144], sysex(0x10, 0x70, 0x01));
        for (pad, write) in writes[145..].iter().enumerate() {
            assert_eq!(*write, sysex(0x10, 0x71 + u8::try_from(pad).unwrap(), 0));
        }
        assert!(writes.iter().all(|write| write[6] != 0x06));
        // Only once.
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 160);
    }

    #[test]
    fn writes_are_paced_by_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open();
        deck.pace = Duration::from_secs(60);
        let begun = Instant::now();
        // One write per turn of the pace, never a sleep.
        queue(&recorder, &[&[0x99, 0x24, 0x7f]]);
        assert!(deck.poll(0).unwrap().is_empty());
        deck.flush().unwrap();
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert_eq!(recorder.writes(), [sysex(0x01, 0x20, 0x01)]);
        // Once the gap is down, the rest of the mapping and the lights follow,
        // and input counts again.
        deck.pace = Duration::ZERO;
        deck.next_send = Instant::now();
        assert!(deck.poll(0).unwrap().is_empty());
        assert_eq!(recorder.writes().len(), 160);
        queue(&recorder, &[&[0x99, 0x24, 0x7f]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(0, true)]);
        // Repeated re-sends of one pad collapse into one queued write.
        deck.pace = Duration::from_secs(60);
        deck.next_send = Instant::now();
        deck.set_face(1, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(2, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        queue(&recorder, &[&[0x99, 0x26, 0x7f], &[0x89, 0x26, 0x00]]);
        deck.poll(0).unwrap();
        assert_eq!(deck.outbox.len(), 1);
        assert_eq!(recorder.writes().len(), 161);
        // Dropping the deck turns its pads off a pace apart.
        deck.pace = Duration::ZERO;
    }

    #[test]
    fn the_setup_waits_for_the_deck_to_settle() {
        let recorder = Recorder::default();
        let mut deck = BeatStep::new(by_id(BEATSTEP_ID).unwrap(), Box::new(recorder.clone()));
        deck.pace = Duration::ZERO;
        // Under the user's own mapping: dropped.
        queue(&recorder, &[&[0x99, 0x24, 0x7f]]);
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(recorder.writes().is_empty());
        deck.settle = Duration::ZERO;
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 160);
    }

    #[test]
    fn notes_and_buttons_map_to_keys() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0x99, 0x24, 0x40],
                &[0x89, 0x24, 0x00],
                &[0x99, 0x33, 0x7f],
                // Velocity 0 releases.
                &[0x99, 0x33, 0x00],
                &[0x99, 0x07, 0x7f],
                &[0xb0, 0x01, 0x7f],
                &[0xb0, 0x01, 0x00],
                &[0xb0, 0x02, 0x7f],
                &[0x89, 0x07, 0x00],
                &[0xb0, 0x02, 0x00],
                // Aftertouch, other channels, notes outside the pads, SysEx.
                &[0xa9, 0x25, 0x40],
                &[0x90, 0x25, 0x7f],
                &[0x9a, 0x25, 0x7f],
                &[0xb9, 0x01, 0x7f],
                &[0x99, 0x34, 0x7f],
                &[0x99, 0x23, 0x7f],
                &[
                    0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x00, 0x10, 0x70, 0x01, 0xf7,
                ],
                // Real-time bytes, and data bytes with no status.
                &[0xf8, 0xfe],
                &[0xf1, 0x24, 0x7f],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(15, true),
                key(15, false),
                key(16, true),
                key(17, true),
                key(17, false),
                key(18, true),
                key(16, false),
                key(18, false),
            ]
        );
    }

    #[test]
    fn encoders_turn_by_twos_complement_steps() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0xb0, 0x20, 0x01],
                &[0xb0, 0x20, 0x7f],
                &[0xb0, 0x27, 0x05],
                &[0xb0, 0x28, 0x41],
                &[0xb0, 0x2f, 0x3f],
                &[0xb0, 0x30, 0x7e],
                // No movement, and controllers that are not encoders.
                &[0xb0, 0x21, 0x00],
                &[0xb0, 0x21, 0x40],
                &[0xb0, 0x31, 0x01],
                &[0xb0, 0x1f, 0x01],
                &[0xb1, 0x20, 0x01],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, 1),
                turn(0, -1),
                turn(7, 5),
                turn(8, -63),
                turn(15, 63),
                turn(16, -2),
            ]
        );
    }

    #[test]
    fn colours_take_the_nearest_led_state() {
        let cases = [
            ([0, 0, 0], LED_OFF),
            ([31, 31, 31], LED_OFF),
            ([255, 0, 0], LED_RED),
            ([255, 128, 0], LED_RED),
            ([255, 255, 0], LED_RED),
            ([255, 0, 40], LED_RED),
            ([0, 255, 0], LED_BLUE),
            ([0, 255, 255], LED_BLUE),
            ([0, 0, 255], LED_BLUE),
            ([128, 0, 255], LED_PINK),
            ([255, 0, 255], LED_PINK),
            ([255, 255, 255], LED_PINK),
            ([100, 100, 100], LED_PINK),
        ];
        for (color, state) in cases {
            assert_eq!(led_state(color), state, "{color:?}");
        }
        assert_eq!((LED_BLUE, LED_PINK), (16, 17));
    }

    #[test]
    fn later_flushes_send_only_changes() {
        let (mut deck, recorder) = ready();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 160);
        deck.set_face(3, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(9, Some(&lit([255, 255, 255]))).unwrap();
        // Shift, Stop and Play have no light.
        deck.set_face(16, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(18, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[160..],
            [sysex(0x10, 0x73, 0x10), sysex(0x10, 0x79, 0x11)]
        );
        deck.set_brightness(10).unwrap();
        assert_eq!(recorder.writes().len(), 162);
    }

    #[test]
    fn releases_put_the_lights_back() {
        let (mut deck, recorder) = ready();
        deck.set_face(2, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 161);
        // A pad's release re-sends its light, even unchanged; a press does not.
        queue(&recorder, &[&[0x99, 0x26, 0x7f], &[0x89, 0x26, 0x00]]);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes()[161..], [sysex(0x10, 0x72, 0x01)]);
        // Shift's release re-sends all 16.
        queue(&recorder, &[&[0x99, 0x07, 0x7f], &[0x89, 0x07, 0x00]]);
        deck.poll(0).unwrap();
        let writes = &recorder.writes()[162..];
        assert_eq!(writes.len(), 16);
        assert_eq!(writes[2], sysex(0x10, 0x72, 0x01));
        assert_eq!(writes[15], sysex(0x10, 0x7f, 0x00));
        // Stop and Play put nothing back.
        queue(&recorder, &[&[0xb0, 0x01, 0x7f], &[0xb0, 0x01, 0x00]]);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 178);
    }

    #[test]
    fn letting_go_turns_every_lit_pad_off() {
        let (mut deck, recorder) = ready();
        deck.set_face(5, Some(&lit([0, 0, 255]))).unwrap();
        deck.flush().unwrap();
        queue(&recorder, &[&[0x99, 0x2e, 0x7f]]);
        deck.poll(0).unwrap();
        drop(deck);
        // Pad 5, and pad 10, which the deck lights while it is held.
        assert_eq!(
            recorder.writes()[161..],
            [sysex(0x10, 0x75, 0), sysex(0x10, 0x7a, 0)]
        );
        // Lights still queued may not have gone out: every pad goes dark.
        let (mut deck, recorder) = ready();
        deck.pace = Duration::from_secs(60);
        deck.set_face(5, Some(&lit([0, 0, 255]))).unwrap();
        deck.flush().unwrap();
        deck.set_face(6, Some(&lit([0, 0, 255]))).unwrap();
        deck.flush().unwrap();
        deck.pace = Duration::ZERO;
        drop(deck);
        let writes = &recorder.writes()[161..];
        assert_eq!(writes.len(), 16);
        for (pad, write) in writes.iter().enumerate() {
            assert_eq!(*write, sysex(0x10, 0x70 + u8::try_from(pad).unwrap(), 0));
        }
        // Never set up: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_beatstep_ports_match() {
        let model = by_id(BEATSTEP_ID).unwrap();
        let cases = [
            ("Arturia BeatStep", false, true),
            ("Arturia BeatStep MIDI 1 20:0", false, true),
            ("Arturia BeatStep:Arturia BeatStep MIDI 1 20:0", false, true),
            ("Arturia BeatStep", true, true),
            ("MIDIIN2 (Arturia BeatStep)", true, false),
            ("Arturia BeatStep Pro", false, false),
            (
                "Arturia BeatStep Pro:Arturia BeatStep Pro MIDI 1 20:0",
                false,
                false,
            ),
            ("Arturia KeyStep 32", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(BEATSTEP_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_names_and_places_every_key() {
        let model = by_id(BEATSTEP_ID).unwrap();
        let view = ModelView::from(model);
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), 19);
        assert_eq!(
            (names[0], names[15], names[16], names[17], names[18]),
            ("Pad 1", "Pad 16", "Shift", "Stop", "Play")
        );
        assert_eq!(view.lit_keys.unwrap(), (0..16).collect::<Vec<u8>>());
        let cells = view.key_cells.unwrap();
        assert_eq!((cells[0], cells[7], cells[8], cells[15]), (2, 9, 12, 19));
        assert_eq!((cells[16], cells[17], cells[18]), (0, 10, 11));
        assert_eq!(view.key_light_colors, "blue_red");
        assert_eq!(view.dial_lights, DialLights::None);
        assert!(view.key_lights && !view.brightness);
        assert_eq!(
            (view.keys, view.extra_keys, view.dials, view.rows, view.cols),
            (19, 3, 17, 2, 10)
        );
    }
}
