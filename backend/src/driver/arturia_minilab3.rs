//! Arturia's MiniLab 3, in its DAW program, over class-compliant USB MIDI
//! ([`super::midi`]).
//!
//! The MiniLab 3 lists four MIDI ports each way: the main `MIDI` one, `DIN
//! THRU`, `MCU/HUI` and `ALV`. Ryu drives only the main port (never MCU/HUI,
//! which Arturia warns fires notes from the transport pads). Its exact
//! spelling is not confirmed: Ableton finds the deck as `Minilab3`, Arturia's
//! documentation calls the port `MiniLab 3 MIDI`. [`port_matches`] takes a
//! name containing `minilab3` or `minilab 3` in any case (not followed by a
//! digit, so never a MiniLab 37), with none of `DIN`, `THRU`, `MCU`, `HUI` or
//! `ALV`, and on Windows only the first interface.
//!
//! Every message Ryu sends is SysEx, each starting `F0 00 20 6B 7F 42`. On
//! open it queues, in order: the DAW connection message (`... 02 00 40 6A 21
//! F7`), a request for the active program (`... 01 00 40 01 F7`), a fixed
//! full-screen message on the OLED (`Ryu` / `Connected`), a realign of each of
//! the eight encoders to 64 (`... 21 10 00 <7 + d> 00 40 F7`), then every
//! light. The queue drains from poll and flush, so the deck never makes the
//! shared poll loop wait. That the connection message is what puts the deck
//! into its DAW behaviour is inferred: the references only send it first.
//!
//! The deck reports its program as `... 02 00 40 <cmd> <val> F7`: command 01
//! (the answer to the request) means the DAW program when the value is 1, and
//! command 0x62 (a program loaded on the deck) when it is 2. Ryu assumes the
//! DAW program until told otherwise. Under any other program the user's own
//! mapping is active, so input is ignored, held keys are released and no
//! light is sent; when the DAW program is back, every light goes out again and
//! the encoders are realigned and re-seeded.
//!
//! In, under the DAW program (channels from 0):
//!
//! - pad `k` (keys 0-15; bank A is 0-7, bank B 8-15, chosen by the deck's own
//!   bank button) is note `36 + k` on channel 9: a Note On above velocity 0
//!   presses, a Note Off or velocity 0 releases;
//! - on channel 0: Shift `CC 27` (key 16), Loop, Stop, Play, Record and Tap
//!   `CC 105-109` (keys 17-21), pressed above 0;
//! - the eight encoders `CC 86, 87, 89, 90, 110, 111, 116, 117` (dials 0-7)
//!   are absolute: the first value seeds a dial, then each later one turns it
//!   by the difference. Once an encoder has been still for [`IDLE`], Ryu
//!   realigns it to 64 so it never pins at 0 or 127 (Ryu's own design:
//!   Ableton realigns only on entering the DAW program). The first value after
//!   a realign is read against both 64 and the value before it, and the
//!   smaller step wins;
//! - the main encoder is `CC 28` (dial 8), and `CC 29` with Shift held (dial
//!   9), each relative with 64 as no motion (value - 64); its push is `CC 118`
//!   (dial 8) and `CC 119` with Shift (dial 9);
//! - the four faders `CC 14, 15, 30, 31` (dials 10-13) are absolute, with no
//!   motor: the first value seeds a dial, then it turns a step per two units.
//!   Ryu never sends them anything.
//!
//! Keybed notes, other channels, pitch bend, the mod strip, aftertouch and any
//! other SysEx are ignored.
//!
//! Out: each pad (LED `0x34 + k` for bank A, `0x44 + k - 8` for bank B) and
//! each transport key (Loop to Tap, `0x57-0x5B`) takes one colour, 7 bits a
//! channel: `... 02 02 16 <led> <r> <g> <b> F7`. Shift has no light. There is
//! no brightness command, so Ryu scales the colours, and 0% turns every light
//! off. Only changed lights are sent. The deck repaints the pads itself when
//! its program changes, so every light is sent again then; it is also sent
//! again after Shift is released (inferred: the references do not say that
//! Shift changes the pad lights, nor that the transport keys are a Shift layer
//! of the pads).
//!
//! When Ryu lets go it turns every light it may have lit off, then sends the
//! disconnect message (`... 02 00 40 6A 20 F7`), unpaced.
//!
//! Facts from Ableton's MiniLab_3 Remote Script as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for
//! facts and the USB id only) and Arturia's MiniLab 3 support pages for the
//! port names; no code was copied. Untested on hardware.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const MINILAB3_ID: &str = "arturia-minilab-3";

/// Bank A on top, bank B under it, then Shift and the transport keys.
pub const ROWS: u8 = 3;
pub const COLS: u8 = 8;
pub const KEYS: u8 = 22;
pub const DIALS: u8 = 14;
const PADS: u8 = 16;
const SHIFT_KEY: u8 = 16;
const FIRST_TRANSPORT_KEY: u8 = 17;
const ENCODERS: u8 = 8;
const MAIN_DIAL: u8 = 8;
const SHIFTED_MAIN_DIAL: u8 = 9;
const FIRST_FADER_DIAL: u8 = 10;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// The pads' channel, from 0.
const PAD_CHANNEL: u8 = 9;
const FIRST_PAD_NOTE: u8 = 36;
const SHIFT_CC: u8 = 27;
const FIRST_TRANSPORT_CC: u8 = 105;
const MAIN_CC: u8 = 28;
const SHIFTED_MAIN_CC: u8 = 29;
const MAIN_PUSH_CC: u8 = 118;
const SHIFTED_MAIN_PUSH_CC: u8 = 119;
/// The eight encoders in panel order (dials 0-7).
const ENCODER_CCS: [u8; ENCODERS as usize] = [86, 87, 89, 90, 110, 111, 116, 117];
/// The four faders (dials 10-13).
const FADER_CCS: [u8; 4] = [14, 15, 30, 31];
/// Fader units per dial step.
const FADER_STEP: i16 = 2;
/// No motion on the relative main encoder, and where Ryu realigns encoders.
const CENTRE: u8 = 64;

const HEAD: [u8; 6] = [0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42];
const SYSEX_END: u8 = 0xf7;
const CONNECT: [u8; 12] = [
    0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x00, 0x40, 0x6a, 0x21, 0xf7,
];
const DISCONNECT: [u8; 12] = [
    0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x00, 0x40, 0x6a, 0x20, 0xf7,
];
const REQUEST_PROGRAM: [u8; 11] = [
    0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x01, 0x00, 0x40, 0x01, 0xf7,
];
/// A program report from the deck, before its command and value.
const PROGRAM_HEAD: [u8; 9] = [0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x00, 0x40];
const PROGRAM_REPLY: u8 = 0x01;
const PROGRAM_LOADED: u8 = 0x62;
const FIRST_ENCODER_ID: u8 = 7;
const FIRST_BANK_A_LED: u8 = 0x34;
const FIRST_BANK_B_LED: u8 = 0x44;
const FIRST_TRANSPORT_LED: u8 = 0x57;
const LINE_1: &str = "Ryu";
const LINE_2: &str = "Connected";
const LINE_1_MAX: usize = 10;
const LINE_2_MAX: usize = 18;

/// The gap between two queued writes. No reference asks for one; a
/// millisecond keeps the open burst gentle. None under test.
const PACE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(1)
};
/// How long an encoder must be still before Ryu realigns it to 64. None
/// under test.
const IDLE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(200)
};

const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const NAMES: [&str; KEYS as usize] = [
    "Pad A1", "Pad A2", "Pad A3", "Pad A4", "Pad A5", "Pad A6", "Pad A7", "Pad A8", "Pad B1",
    "Pad B2", "Pad B3", "Pad B4", "Pad B5", "Pad B6", "Pad B7", "Pad B8", "Shift", "Loop", "Stop",
    "Play", "Record", "Tap",
];

/// Whether a normalised port name is a MiniLab 3's main MIDI port, on Windows
/// its first interface. Port spellings are assumed (see the module docs).
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.protocol != Protocol::ArturiaMiniLab3Midi || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.to_ascii_lowercase();
    let named = ["minilab3", "minilab 3"].iter().any(|token| {
        base.match_indices(token).any(|(at, found)| {
            !base
                .as_bytes()
                .get(at + found.len())
                .is_some_and(u8::is_ascii_digit)
        })
    });
    named
        && !["din", "thru", "mcu", "hui", "alv"]
            .iter()
            .any(|other| base.contains(other))
}

/// The legend on each key: bank A and bank B pads, Shift, then the transport.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::ArturiaMiniLab3Midi).then(|| NAMES.to_vec())
}

/// Every key but Shift has a light.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    (model.protocol == Protocol::ArturiaMiniLab3Midi)
        .then(|| (0..KEYS).filter(|key| *key != SHIFT_KEY).collect())
}

/// The LED a key's light is set through; `None` for Shift.
const fn led(key: u8) -> Option<u8> {
    match key {
        0..=7 => Some(FIRST_BANK_A_LED + key),
        8..=15 => Some(FIRST_BANK_B_LED + key - 8),
        17..=21 => Some(FIRST_TRANSPORT_LED + key - FIRST_TRANSPORT_KEY),
        _ => None,
    }
}

/// One key light: 7 bits a channel.
const fn light(led: u8, [red, green, blue]: [u8; 3]) -> [u8; 14] {
    let [a, b, c, d, e, f] = HEAD;
    [
        a, b, c, d, e, f, 0x02, 0x02, 0x16, led, red, green, blue, SYSEX_END,
    ]
}

/// Sets encoder `dial`'s value inside the deck.
const fn realign(dial: u8, value: u8) -> [u8; 13] {
    let [a, b, c, d, e, f] = HEAD;
    [
        a,
        b,
        c,
        d,
        e,
        f,
        0x21,
        0x10,
        0x00,
        FIRST_ENCODER_ID + dial,
        0x00,
        value,
        SYSEX_END,
    ]
}

/// A full-screen, non-transient text message with no pictograms.
fn display(line_1: &str, line_2: &str) -> Vec<u8> {
    let text = |line: &str, max: usize| -> Vec<u8> {
        line.bytes()
            .filter(|byte| (0x20..0x7f).contains(byte))
            .take(max)
            .collect()
    };
    let mut out = HEAD.to_vec();
    out.extend_from_slice(&[
        0x04, 0x02, 0x60, 0x1f, 0x07, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01,
    ]);
    out.extend(text(line_1, LINE_1_MAX));
    out.extend_from_slice(&[0x00, 0x02]);
    out.extend(text(line_2, LINE_2_MAX));
    out.extend_from_slice(&[0x00, SYSEX_END]);
    out
}

/// One 8-bit channel at a brightness, as the 7-bit value the deck takes.
fn channel(value: u8, percent: u8) -> u8 {
    let scaled = u16::from(value) * u16::from(percent.min(100)) / 100;
    u8::try_from(scaled >> 1).unwrap_or(0x7f)
}

/// A step clamped to what one dial turn carries.
fn clamp(step: i16) -> i8 {
    i8::try_from(step.clamp(-127, 127)).unwrap_or(0)
}

/// What queues one write, so a newer one to the same place replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Light(u8),
    Realign(u8),
    Other,
}

#[derive(Debug, Default, Clone, Copy)]
struct Encoder {
    last: Option<u8>,
    /// The value before Ryu realigned it, while a turn may still be read
    /// against it.
    before_centre: Option<u8>,
    /// When it last moved, until it is realigned.
    moved: Option<Instant>,
}

pub struct MiniLab3 {
    transport: Box<dyn Transport>,
    pace: Duration,
    idle: Duration,
    /// The open messages are queued.
    started: bool,
    /// The deck is on its DAW program, as far as Ryu knows.
    daw: bool,
    /// Writes waiting their turn, drained from poll and flush.
    outbox: VecDeque<(Slot, Vec<u8>)>,
    next_send: Instant,
    colors: Vec<[u8; 3]>,
    /// The 7-bit colour each key's light was last queued with.
    sent: Vec<Option<[u8; 3]>>,
    brightness: u8,
    held: Vec<bool>,
    pushed: [bool; 2],
    encoders: [Encoder; ENCODERS as usize],
    faders: [Option<u8>; 4],
    fader_rest: [i16; 4],
    parser: Parser,
}

impl MiniLab3 {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            pace: PACE,
            idle: IDLE,
            started: false,
            daw: true,
            outbox: VecDeque::new(),
            next_send: Instant::now(),
            colors: vec![BLACK; usize::from(KEYS)],
            sent: vec![None; usize::from(KEYS)],
            brightness: 100,
            held: vec![false; usize::from(KEYS)],
            pushed: [false; 2],
            encoders: [Encoder::default(); ENCODERS as usize],
            faders: [None; 4],
            fader_rest: [0; 4],
            parser: Parser::default(),
        }
    }

    fn send(&mut self, slot: Slot, message: Vec<u8>) {
        if slot != Slot::Other {
            self.outbox.retain(|(queued, _)| *queued != slot);
        }
        self.outbox.push_back((slot, message));
    }

    /// Writes the queued messages whose turn has come, never sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        while let Some((_, message)) = self.outbox.front() {
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

    /// Queues the open messages once.
    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.send(Slot::Other, CONNECT.to_vec());
        self.send(Slot::Other, REQUEST_PROGRAM.to_vec());
        self.send(Slot::Other, display(LINE_1, LINE_2));
        self.enter_daw();
    }

    /// Realigns every encoder to 64, re-seeds the faders and forgets which
    /// lights were sent, so the next flush sends them all.
    fn enter_daw(&mut self) {
        for dial in 0..ENCODERS {
            self.send(Slot::Realign(dial), realign(dial, CENTRE).to_vec());
            if let Some(encoder) = self.encoders.get_mut(usize::from(dial)) {
                *encoder = Encoder {
                    last: Some(CENTRE),
                    ..Encoder::default()
                };
            }
        }
        self.faders = [None; 4];
        self.fader_rest = [0; 4];
        self.sent.fill(None);
    }

    /// The colour a key's light should show now, 7 bits a channel.
    fn target(&self, key: u8) -> [u8; 3] {
        let color = self.colors.get(usize::from(key)).copied().unwrap_or(BLACK);
        color.map(|value| channel(value, self.brightness))
    }

    /// Queues every light whose colour changed.
    fn queue_lights(&mut self) {
        if !self.daw {
            return;
        }
        for key in 0..KEYS {
            let Some(led) = led(key) else {
                continue;
            };
            let target = self.target(key);
            let Some(sent) = self.sent.get_mut(usize::from(key)) else {
                continue;
            };
            if *sent != Some(target) {
                *sent = Some(target);
                self.send(Slot::Light(led), light(led, target).to_vec());
            }
        }
    }

    /// Realigns every encoder that has been still long enough to 64.
    fn recentre(&mut self) {
        if !self.daw {
            return;
        }
        for dial in 0..ENCODERS {
            let Some(encoder) = self.encoders.get_mut(usize::from(dial)) else {
                continue;
            };
            let Some(moved) = encoder.moved else {
                continue;
            };
            if moved.elapsed() < self.idle {
                continue;
            }
            encoder.moved = None;
            if encoder.last != Some(CENTRE) {
                encoder.before_centre = encoder.last;
                encoder.last = Some(CENTRE);
                self.send(Slot::Realign(dial), realign(dial, CENTRE).to_vec());
            }
        }
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

    fn push(&mut self, dial: u8, pressed: bool, events: &mut Vec<Input>) {
        let Some(held) = self.pushed.get_mut(usize::from(dial - MAIN_DIAL)) else {
            return;
        };
        if *held != pressed {
            *held = pressed;
            events.push(Input::DialPress {
                index: dial,
                pressed,
            });
        }
    }

    /// Releases everything held, as the deck leaves the DAW program.
    fn release_all(&mut self, events: &mut Vec<Input>) {
        for key in 0..KEYS {
            self.press(key, false, events);
        }
        for dial in [MAIN_DIAL, SHIFTED_MAIN_DIAL] {
            self.push(dial, false, events);
        }
    }

    fn encoder(&mut self, dial: u8, value: u8, events: &mut Vec<Input>) {
        let Some(encoder) = self.encoders.get_mut(usize::from(dial)) else {
            return;
        };
        encoder.moved = Some(Instant::now());
        let before = encoder.before_centre.take();
        let Some(last) = encoder.last.replace(value) else {
            return;
        };
        let mut step = i16::from(value) - i16::from(last);
        if let Some(before) = before {
            let other = i16::from(value) - i16::from(before);
            if other.abs() < step.abs() {
                step = other;
            }
        }
        if step != 0 {
            events.push(Input::DialTurn {
                index: dial,
                delta: clamp(step),
            });
        }
    }

    fn fader(&mut self, fader: u8, value: u8, events: &mut Vec<Input>) {
        let at = usize::from(fader);
        let (Some(last), Some(rest)) = (self.faders.get_mut(at), self.fader_rest.get_mut(at))
        else {
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
                index: FIRST_FADER_DIAL + fader,
                delta: clamp(steps),
            });
        }
    }

    /// A program report: whether it names the DAW program, or `None` for
    /// any other SysEx.
    fn program(bytes: &[u8]) -> Option<bool> {
        let rest = bytes.strip_prefix(&PROGRAM_HEAD[..])?;
        match *rest {
            [PROGRAM_REPLY, value, SYSEX_END] => Some(value == 1),
            [PROGRAM_LOADED, value, SYSEX_END] => Some(value == 2),
            _ => None,
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let [status, number, value] = match message {
            Message::SysEx(bytes) => {
                if let Some(daw) = Self::program(bytes) {
                    if daw {
                        self.daw = true;
                        self.enter_daw();
                    } else if self.daw {
                        self.daw = false;
                        self.release_all(events);
                    }
                }
                return;
            }
            Message::Channel(bytes) => *bytes,
        };
        if !self.daw {
            return;
        }
        match (status & 0xf0, status & 0x0f) {
            (kind @ (NOTE_ON | NOTE_OFF), PAD_CHANNEL)
                if (FIRST_PAD_NOTE..FIRST_PAD_NOTE + PADS).contains(&number) =>
            {
                self.press(
                    number - FIRST_PAD_NOTE,
                    kind == NOTE_ON && value > 0,
                    events,
                );
            }
            (CONTROL_CHANGE, 0) => match number {
                SHIFT_CC => self.press(SHIFT_KEY, value > 0, events),
                cc if (FIRST_TRANSPORT_CC..FIRST_TRANSPORT_CC + 5).contains(&cc) => {
                    self.press(
                        FIRST_TRANSPORT_KEY + cc - FIRST_TRANSPORT_CC,
                        value > 0,
                        events,
                    );
                }
                MAIN_PUSH_CC => self.push(MAIN_DIAL, value > 0, events),
                SHIFTED_MAIN_PUSH_CC => self.push(SHIFTED_MAIN_DIAL, value > 0, events),
                MAIN_CC | SHIFTED_MAIN_CC => {
                    let delta = i16::from(value) - i16::from(CENTRE);
                    if delta != 0 {
                        events.push(Input::DialTurn {
                            index: if number == MAIN_CC {
                                MAIN_DIAL
                            } else {
                                SHIFTED_MAIN_DIAL
                            },
                            delta: clamp(delta),
                        });
                    }
                }
                cc => {
                    if let Some(dial) = ENCODER_CCS.iter().position(|known| *known == cc) {
                        self.encoder(u8::try_from(dial).unwrap_or(0), value, events);
                    } else if let Some(fader) = FADER_CCS.iter().position(|known| *known == cc) {
                        self.fader(u8::try_from(fader).unwrap_or(0), value, events);
                    }
                }
            },
            _ => {}
        }
    }
}

impl Driver for MiniLab3 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.start();
        let mut events = Vec::new();
        for message in &self.read(timeout_ms)? {
            self.receive(message, &mut events);
        }
        // Shift may leave the deck's own colours on the pads: put Ryu's back.
        if events.contains(&Input::Key {
            index: SHIFT_KEY,
            pressed: false,
        }) {
            self.sent.fill(None);
        }
        self.recentre();
        self.queue_lights();
        self.pump()?;
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// Queues each light whose colour changed (all of them the first time)
    /// and writes whatever's turn has come.
    fn flush(&mut self) -> DriverResult<()> {
        self.start();
        self.recentre();
        self.queue_lights();
        self.pump()
    }

    /// No brightness command: Ryu scales the colours, sent on the next flush.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        Ok(())
    }
}

impl Drop for MiniLab3 {
    /// Turns off every light that may be lit, then hands the deck back with
    /// the disconnect message; written at once, never paced.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        let unsent = !self.outbox.is_empty();
        self.outbox.clear();
        if self.daw {
            for key in 0..KEYS {
                let Some(led) = led(key) else {
                    continue;
                };
                let dark = self.sent.get(usize::from(key)) == Some(&Some(BLACK));
                if dark && !unsent {
                    continue;
                }
                let _ = self.transport.write(&light(led, BLACK));
            }
        }
        let _ = self.transport.write(&DISCONNECT);
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    fn open() -> (MiniLab3, Recorder) {
        let recorder = Recorder::default();
        let deck = MiniLab3::new(by_id(MINILAB3_ID).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// A deck past its open burst, with those writes cleared.
    fn ready() -> (MiniLab3, Recorder) {
        let (mut deck, recorder) = open();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 3 + 8 + 21);
        recorder.0.lock().unwrap().writes.clear();
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

    fn led_bytes(led: u8, rgb: [u8; 3]) -> Vec<u8> {
        vec![
            0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x02, 0x16, led, rgb[0], rgb[1], rgb[2], 0xf7,
        ]
    }

    fn realign_bytes(eid: u8) -> Vec<u8> {
        vec![
            0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x21, 0x10, 0x00, eid, 0x00, 0x40, 0xf7,
        ]
    }

    fn program_bytes(cmd: u8, value: u8) -> Vec<u8> {
        vec![
            0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x00, 0x40, cmd, value, 0xf7,
        ]
    }

    fn port(name: &str) -> bool {
        port_matches(by_id(MINILAB3_ID).unwrap(), &normalise(name))
    }

    #[test]
    fn the_main_port_is_matched_and_the_others_are_not() {
        // Every spelling here is assumed: no reference lists the port names.
        assert!(port("Minilab3 MIDI"));
        assert!(port("MiniLab 3 MIDI"));
        assert!(port("Minilab3"));
        assert!(port("Minilab3:Minilab3 Minilab3 MIDI 24:0"));
        assert!(!port("Minilab3 DIN THRU"));
        assert!(!port("Minilab3 MCU/HUI"));
        assert!(!port("Minilab3 ALV"));
        assert!(!port("MIDIIN2 (Minilab3)"));
        assert!(!port("MIDIIN3 (Minilab3)"));
        assert!(!port("MiniLab mkII"));
        assert!(!port("MiniLab 37 MIDI"));
        assert!(!port("Arturia BeatStep"));
        let found = identify_midi(&normalise("MiniLab 3 MIDI"), false).unwrap();
        assert_eq!(found.id, MINILAB3_ID);
    }

    #[test]
    fn the_model_has_lights_on_every_key_but_shift_and_none_on_dials() {
        let model = by_id(MINILAB3_ID).unwrap();
        assert_eq!(model.dial_lights, DialLights::None);
        assert_eq!(model.dial_light_colors(), 0);
        assert_eq!(
            (model.rows, model.cols, model.keys, model.dials),
            (3, 8, 22, 14)
        );
        let view = ModelView::from(model);
        assert_eq!(view.usb_ids, vec!["1c75:220b".to_owned()]);
        assert!(view.key_lights && view.brightness);
        let lit = lit_keys(model).unwrap();
        assert_eq!(lit.len(), 21);
        assert!(!lit.contains(&SHIFT_KEY));
    }

    #[test]
    fn open_connects_requests_the_program_writes_the_screen_realigns_then_lights() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 128, 0]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 3 + 8 + 21);
        assert_eq!(writes[0], CONNECT);
        assert_eq!(writes[1], REQUEST_PROGRAM);
        let mut screen = vec![
            0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x04, 0x02, 0x60, 0x1f, 0x07, 0x01, 0x00, 0x00,
            0x00, 0x00, 0x01,
        ];
        screen.extend_from_slice(b"Ryu");
        screen.extend_from_slice(&[0x00, 0x02]);
        screen.extend_from_slice(b"Connected");
        screen.extend_from_slice(&[0x00, 0xf7]);
        assert_eq!(writes[2], screen);
        for dial in 0..8 {
            assert_eq!(
                writes[3 + dial],
                realign_bytes(7 + u8::try_from(dial).unwrap())
            );
        }
        assert_eq!(writes[11], led_bytes(0x34, [127, 64, 0]));
        assert_eq!(writes[12], led_bytes(0x35, [0, 0, 0]));
        assert_eq!(writes[19], led_bytes(0x44, [0, 0, 0]));
        assert_eq!(writes[26], led_bytes(0x4b, [0, 0, 0]));
        assert_eq!(writes[27], led_bytes(0x57, [0, 0, 0]));
        assert_eq!(writes[31], led_bytes(0x5b, [0, 0, 0]));
        // Only SysEx is ever sent, and only once.
        assert!(writes.iter().all(|write| write[0] == 0xf0));
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 32);
    }

    #[test]
    fn writes_are_paced_by_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open();
        deck.pace = Duration::from_secs(60);
        let begun = Instant::now();
        deck.poll(0).unwrap();
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert_eq!(recorder.writes(), vec![CONNECT.to_vec()]);
        deck.pace = Duration::ZERO;
        deck.next_send = Instant::now();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 32);
    }

    #[test]
    fn flushes_send_only_changed_lights_and_brightness_scales_them() {
        let (mut deck, recorder) = ready();
        deck.set_face(8, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(21, Some(&lit([40, 160, 255]))).unwrap();
        // Shift has no light.
        deck.set_face(16, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                led_bytes(0x44, [127, 127, 127]),
                led_bytes(0x5b, [20, 80, 127])
            ]
        );
        deck.set_brightness(50).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[2], led_bytes(0x44, [63, 63, 63]));
        deck.set_brightness(0).unwrap();
        deck.set_face(8, None).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[4], led_bytes(0x44, [0, 0, 0]));
        assert_eq!(writes[5], led_bytes(0x5b, [0, 0, 0]));
        assert_eq!(writes.len(), 6);
    }

    #[test]
    fn pads_and_buttons_press_and_release_keys() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0x99, 36, 100],
                &[0x89, 36, 0],
                &[0x99, 51, 90],
                &[0x99, 51, 0],
                &[0xb0, 27, 127],
                &[0xb0, 105, 127],
                &[0xb0, 109, 127],
                &[0xb0, 109, 0],
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
                key(21, true),
                key(21, false),
            ]
        );
    }

    #[test]
    fn keybed_aftertouch_bend_and_other_channels_are_ignored() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                // Keybed on channel 0, pads' notes on another channel.
                &[0x90, 60, 100],
                &[0x98, 36, 100],
                // Pad notes outside 36-51.
                &[0x99, 35, 100],
                &[0x99, 52, 100],
                // Aftertouch, pitch bend, mod strip, a CC on channel 1.
                &[0xa9, 36, 50],
                &[0xd9, 50],
                &[0xe0, 0, 64],
                &[0xb0, 1, 64],
                &[0xb1, 27, 127],
                // The identity reply and a short SysEx.
                &[
                    0xf0, 0x7e, 0x7f, 0x06, 0x02, 0x00, 0x20, 0x6b, 0x02, 0x00, 0x04, 0x02, 0xf7,
                ],
                &[0xf0, 0x00, 0x20, 0xf7],
            ],
        );
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn the_main_encoder_is_relative_around_64_and_pushes_are_dial_presses() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0xb0, 28, 65],
                &[0xb0, 28, 61],
                &[0xb0, 28, 64],
                &[0xb0, 29, 66],
                &[0xb0, 118, 127],
                &[0xb0, 118, 0],
                &[0xb0, 119, 127],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(8, 1),
                turn(8, -3),
                turn(9, 2),
                Input::DialPress {
                    index: 8,
                    pressed: true
                },
                Input::DialPress {
                    index: 8,
                    pressed: false
                },
                Input::DialPress {
                    index: 9,
                    pressed: true
                },
            ]
        );
    }

    #[test]
    fn encoders_turn_from_64_and_are_realigned_once_still() {
        let (mut deck, recorder) = ready();
        deck.idle = Duration::from_secs(60);
        // The encoders start at the realigned 64.
        queue(&recorder, &[&[0xb0, 86, 66], &[0xb0, 117, 60]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(0, 2), turn(7, -4)]);
        queue(&recorder, &[&[0xb0, 86, 67]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(0, 1)]);
        assert!(recorder.writes().is_empty());
        // Still long enough: both are put back to 64.
        deck.idle = Duration::ZERO;
        deck.flush().unwrap();
        assert_eq!(recorder.writes(), vec![realign_bytes(7), realign_bytes(14)]);
        // The next value counts from 64.
        queue(&recorder, &[&[0xb0, 86, 65]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(0, 1)]);
    }

    #[test]
    fn a_turn_that_beat_the_realign_counts_from_the_old_value() {
        let (mut deck, recorder) = ready();
        queue(&recorder, &[&[0xb0, 89, 100]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(2, 36)]);
        assert_eq!(recorder.writes(), vec![realign_bytes(9)]);
        // The deck had not taken the realign yet.
        deck.idle = Duration::from_secs(60);
        queue(&recorder, &[&[0xb0, 89, 101]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(2, 1)]);
    }

    #[test]
    fn faders_seed_then_step_every_two_units_and_are_never_sent() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0xb0, 14, 10],
                &[0xb0, 14, 14],
                &[0xb0, 14, 15],
                &[0xb0, 14, 16],
                &[0xb0, 31, 127],
                &[0xb0, 31, 117],
                &[0xb0, 15, 0],
                &[0xb0, 30, 0],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(10, 2), turn(10, 1), turn(13, -5)]
        );
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn a_user_program_mutes_input_and_lights_until_the_daw_program_returns() {
        let (mut deck, recorder) = ready();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        queue(&recorder, &[&[0x99, 37, 100]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(1, true)]);
        // The reply to the request: value 0 is not the DAW program.
        queue(
            &recorder,
            &[
                &program_bytes(0x01, 0x00),
                &[0x99, 38, 100],
                &[0xb0, 28, 70],
            ],
        );
        assert_eq!(deck.poll(0).unwrap(), vec![key(1, false)]);
        deck.set_face(1, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
        // A loaded program with value 1 is still not the DAW program.
        queue(&recorder, &[&program_bytes(0x62, 0x01)]);
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(recorder.writes().is_empty());
        // Value 2 on a load is: realign, then every light again.
        queue(&recorder, &[&program_bytes(0x62, 0x02)]);
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(writes.len(), 8 + 21);
        assert_eq!(writes[0], realign_bytes(7));
        assert_eq!(writes[8], led_bytes(0x34, [127, 0, 0]));
        assert_eq!(writes[9], led_bytes(0x35, [0, 127, 0]));
        queue(&recorder, &[&[0x99, 38, 100]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(2, true)]);
    }

    #[test]
    fn a_reply_naming_the_daw_program_resends_everything() {
        let (mut deck, recorder) = ready();
        queue(&recorder, &[&program_bytes(0x01, 0x01)]);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 8 + 21);
    }

    #[test]
    fn releasing_shift_resends_every_light() {
        let (mut deck, recorder) = ready();
        queue(&recorder, &[&[0xb0, 27, 127]]);
        deck.poll(0).unwrap();
        assert!(recorder.writes().is_empty());
        queue(&recorder, &[&[0xb0, 27, 0]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(16, false)]);
        assert_eq!(recorder.writes().len(), 21);
    }

    #[test]
    fn short_and_malformed_input_never_panics() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0x99],
                &[0xb0, 86],
                &[0xf0],
                &[0xf7],
                &[0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x00, 0x40, 0xf7],
                &[
                    0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x00, 0x40, 0x01, 0xf7,
                ],
                &[0xff, 0xfe, 0x00],
            ],
        );
        deck.poll(0).unwrap();
    }

    #[test]
    fn dropping_turns_lit_lights_off_then_disconnects_without_pacing() {
        let (mut deck, recorder) = ready();
        deck.pace = Duration::from_secs(60);
        deck.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(4, Some(&lit([0, 0, 255]))).unwrap();
        // The first light goes out; the second waits its turn.
        deck.flush().unwrap();
        let begun = Instant::now();
        drop(deck);
        assert!(begun.elapsed() < Duration::from_secs(1));
        let writes = recorder.writes();
        assert_eq!(writes.first(), Some(&led_bytes(0x37, [127, 0, 0])));
        // A light was left unsent, so every light goes off.
        assert_eq!(writes.len(), 1 + 21 + 1);
        assert_eq!(writes[1], led_bytes(0x34, [0, 0, 0]));
        assert_eq!(writes.last(), Some(&DISCONNECT.to_vec()));
    }

    #[test]
    fn dropping_a_dark_deck_only_disconnects() {
        let (deck, recorder) = ready();
        drop(deck);
        assert_eq!(recorder.writes(), vec![DISCONNECT.to_vec()]);
    }

    #[test]
    fn dropping_an_unopened_deck_sends_nothing() {
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }
}
