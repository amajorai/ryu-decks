//! Arturia's KeyLab Essential mk3 (49, 61 and 88 keys), in its DAW program,
//! over class-compliant USB MIDI ([`super::midi`]).
//!
//! The three sizes differ only in their keybed. Ryu drives the main MIDI
//! port, `KeyLab Essential <n> mk3 MIDI` on macOS and Linux and `KL Essential
//! <n> mk3 MIDI` on Windows (as Bitwig's extension lists them). [`port_matches`]
//! takes a name containing `keylab essential <n> mk3 MI` or `kl essential <n>
//! mk3 MI` in any case, where `<n>` is the model's own size, with none of
//! `DIN`, `THRU`, `MCU`, `HUI` or `ALV`, and on Windows only the first
//! interface. The deck's second port is named by neither reference, so it is
//! kept out by the ` MI` after the token. Linux ALSA client names are not
//! confirmed.
//!
//! Every message Ryu sends is SysEx, each starting `F0 00 20 6B 7F 42`. On
//! open it queues, in order: the DAW connection message (`... 02 0F 40 5A 01
//! F7`), a request for the active program (`... 01 11 40 02 F7`), a fixed
//! two-line message on the text screen (`... 04 01 60 12 01 "Ryu" 00 02
//! "Connected" 00 F7`), a realign of each of the nine knobs to 64 (`... 02 0F
//! 40 <03 + d> 40 F7`), then every light. The queue drains from poll and
//! flush, so the deck never makes the shared poll loop wait. Ryu does not wait
//! for the identity reply before connecting (as for the MiniLab 3).
//!
//! The deck reports its program as `... 21 11 40 02 00 <p> F7`, `p` 0 for
//! Arturia, 1 for DAW and 2 for User, both as the answer to the request and
//! when the program is changed on the deck. Ryu assumes the DAW program until
//! told otherwise. Under any other program the user's own mapping is active,
//! so input is ignored, held keys are released and no light is sent; when the
//! DAW program is back, the knobs are realigned and re-seeded, the faders
//! re-seeded and every light sent again. The pad-bank report (`... 21 11 40 00
//! 00 <b> F7`) is ignored: the notes already say which bank a pad is in.
//!
//! In, under the DAW program (channels from 0):
//!
//! - the pads are notes on channel 10: a Note On above velocity 0 presses, a
//!   Note Off or velocity 0 releases. Bank A's top row is notes 40-43 (keys
//!   0-3) and its bottom row 36-39 (keys 4-7); bank B's top row 48-51 (keys
//!   8-11) and bottom row 44-47 (keys 12-15). The deck's own Bank button picks
//!   the bank. Bank B's row order is an inference: Ableton lists bank B as one
//!   flat row, and its LED map (44-47 to `28-2B`, 48-51 to `24-27`) read with
//!   bank A's (whose top row 40-43 is `1C-1F`) puts 48-51 on top; Bitwig maps
//!   bank B's LEDs linearly instead (44 + i to `24 + i`). Ryu follows Ableton;
//! - on channel 0, pressed above 0: Context 1-4 `CC 44-47` (keys 16-19), Save
//!   `CC 40`, Punch `CC 41`, Undo `CC 42`, Redo `CC 43` (keys 20-23), Loop
//!   `CC 24`, Rewind `CC 25`, Fast-forward `CC 26`, Metronome `CC 27`, Stop
//!   `CC 20`, Play `CC 21`, Record `CC 22`, Tap `CC 23` (keys 24-31), Part `CC
//!   119` (key 32) and Bank `CC 118` (key 33, which also flips the deck's pad
//!   bank itself). Pad notes 40-47 and button CCs 40-47 are told apart by
//!   their status byte;
//! - the nine knobs `CC 96-104` (dials 0-8) are absolute: the first value
//!   seeds a dial, then each later one turns it by the difference. Once a knob
//!   has been still for [`IDLE`], Ryu realigns it to 64 so it never pins at 0
//!   or 127 (Ryu's own design). The first value after a realign is read
//!   against both 64 and the value before it, and the smaller step wins;
//! - the display encoder `CC 116` (dial 9) is relative with 64 as no motion
//!   (value - 64); its push `CC 117` presses dial 9;
//! - the nine faders `CC 105-113` (dials 10-18) are absolute, with no motor:
//!   the first value seeds a dial, then it turns a step per two units. Ryu
//!   never sends them anything (the deck has no realign for them).
//!
//! Keybed notes, other channels, pitch bend, the mod wheel, sustain,
//! aftertouch, other CCs and any other SysEx are ignored.
//!
//! Out: each key but Bank takes one colour, 7 bits a channel, with a state
//! byte Ryu always sends as `20` (solid): `... 04 01 16 <led> <r> <g> <b> 20
//! F7`. Pad key `k` is LED `1C + k`; Context 1-4 are `18-1B`; Save, Punch,
//! Undo, Redo, Loop, Rewind, Fast-forward, Metronome, Stop, Play, Record and
//! Tap are `0C-17`; Part is `07`. There is no brightness command, so Ryu
//! scales the colours, and 0% turns every light off. Only changed lights are
//! sent. There is no keep-alive.
//!
//! When Ryu lets go it turns every light it may have lit off, then sends the
//! disconnect message (`... 02 0F 40 5A 00 F7`), unpaced.
//!
//! Facts from Bitwig's KeyLab Essential mk3 extension (bitwig/bitwig-extensions,
//! MIT) and Ableton's KeyLab_Essential_mk3 Remote Script as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for
//! facts and the USB ids only); no code was copied. Untested on hardware.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const KEYLAB_ESSENTIAL_49_MK3_ID: &str = "arturia-keylab-essential-49-mk3";
pub const KEYLAB_ESSENTIAL_61_MK3_ID: &str = "arturia-keylab-essential-61-mk3";
pub const KEYLAB_ESSENTIAL_88_MK3_ID: &str = "arturia-keylab-essential-88-mk3";

/// Bank A, bank B, Context and Save to Redo, the transport row, then Part
/// and Bank.
pub const ROWS: u8 = 5;
pub const COLS: u8 = 8;
pub const KEYS: u8 = 34;
pub const DIALS: u8 = 19;
const BANK_KEY: u8 = 33;
const KNOBS: u8 = 9;
const MAIN_DIAL: u8 = 9;
const FIRST_FADER_DIAL: u8 = 10;
const FADERS: u8 = 9;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// The pads' channel, from 0.
const PAD_CHANNEL: u8 = 10;
/// The CC each key from 16 on sends, in key order.
const BUTTON_CCS: [u8; (KEYS - 16) as usize] = [
    44, 45, 46, 47, 40, 41, 42, 43, 24, 25, 26, 27, 20, 21, 22, 23, 119, 118,
];
const FIRST_KNOB_CC: u8 = 96;
const FIRST_FADER_CC: u8 = 105;
const MAIN_CC: u8 = 116;
const MAIN_PUSH_CC: u8 = 117;
/// Fader units per dial step.
const FADER_STEP: i16 = 2;
/// No motion on the relative display encoder, and where Ryu realigns knobs.
const CENTRE: u8 = 64;

const HEAD: [u8; 6] = [0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42];
const SYSEX_END: u8 = 0xf7;
const CONNECT: [u8; 12] = [
    0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x0f, 0x40, 0x5a, 0x01, 0xf7,
];
const DISCONNECT: [u8; 12] = [
    0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x0f, 0x40, 0x5a, 0x00, 0xf7,
];
const REQUEST_PROGRAM: [u8; 11] = [
    0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x01, 0x11, 0x40, 0x02, 0xf7,
];
/// A program report from the deck, before its value.
const PROGRAM_HEAD: [u8; 11] = [
    0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x21, 0x11, 0x40, 0x02, 0x00,
];
const DAW_PROGRAM: u8 = 0x01;
const FIRST_KNOB_ID: u8 = 0x03;
/// Ryu's light state: solid (the others blink or pulse).
const SOLID: u8 = 0x20;
const FIRST_PAD_LED: u8 = 0x1c;
const FIRST_CONTEXT_LED: u8 = 0x18;
/// Save, the first of twelve keys with consecutive LEDs (Save to Tap).
const FIRST_SAVE_LED: u8 = 0x0c;
const PART_LED: u8 = 0x07;
const LINE_1: &str = "Ryu";
const LINE_2: &str = "Connected";
const LINE_1_MAX: usize = 11;
const LINE_2_MAX: usize = 20;

/// The gap between two queued writes. No reference asks for one; a
/// millisecond keeps the open burst gentle. None under test.
const PACE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(1)
};
/// How long a knob must be still before Ryu realigns it to 64. None under
/// test.
const IDLE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(200)
};

const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const NAMES: [&str; KEYS as usize] = [
    "Pad A5",
    "Pad A6",
    "Pad A7",
    "Pad A8",
    "Pad A1",
    "Pad A2",
    "Pad A3",
    "Pad A4",
    "Pad B5",
    "Pad B6",
    "Pad B7",
    "Pad B8",
    "Pad B1",
    "Pad B2",
    "Pad B3",
    "Pad B4",
    "Context 1",
    "Context 2",
    "Context 3",
    "Context 4",
    "Save",
    "Punch",
    "Undo",
    "Redo",
    "Loop",
    "Rewind",
    "Fast-forward",
    "Metronome",
    "Stop",
    "Play",
    "Record",
    "Tap",
    "Part",
    "Bank",
];

/// The keybed size each model's port names carry.
fn size(model: &DeviceModel) -> Option<&'static str> {
    match model.id {
        KEYLAB_ESSENTIAL_49_MK3_ID => Some("49"),
        KEYLAB_ESSENTIAL_61_MK3_ID => Some("61"),
        KEYLAB_ESSENTIAL_88_MK3_ID => Some("88"),
        _ => None,
    }
}

/// Whether a normalised port name is this size of KeyLab Essential mk3's main
/// MIDI port, on Windows its first interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.protocol != Protocol::ArturiaKeyLabEssential3Midi || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let Some(size) = size(model) else {
        return false;
    };
    let base = name.base.to_ascii_lowercase();
    let named = ["keylab essential", "kl essential"]
        .iter()
        .any(|maker| base.contains(&format!("{maker} {size} mk3 mi")));
    named
        && !["din", "thru", "mcu", "hui", "alv"]
            .iter()
            .any(|other| base.contains(other))
}

/// The legend on each key: the pads by bank and panel position, then the
/// buttons.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::ArturiaKeyLabEssential3Midi).then(|| NAMES.to_vec())
}

/// Every key but Bank has a light.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    (model.protocol == Protocol::ArturiaKeyLabEssential3Midi)
        .then(|| (0..KEYS).filter(|key| *key != BANK_KEY).collect())
}

/// The LED a key's light is set through; `None` for Bank.
const fn led(key: u8) -> Option<u8> {
    match key {
        0..=15 => Some(FIRST_PAD_LED + key),
        16..=19 => Some(FIRST_CONTEXT_LED + key - 16),
        20..=31 => Some(FIRST_SAVE_LED + key - 20),
        32 => Some(PART_LED),
        _ => None,
    }
}

/// The key a pad note is, bank A's and bank B's top rows first.
const fn pad_key(note: u8) -> Option<u8> {
    match note {
        40..=43 => Some(note - 40),
        36..=39 => Some(note - 36 + 4),
        48..=51 => Some(note - 48 + 8),
        44..=47 => Some(note - 44 + 12),
        _ => None,
    }
}

/// One key light: 7 bits a channel, solid.
const fn light(led: u8, [red, green, blue]: [u8; 3]) -> [u8; 15] {
    let [a, b, c, d, e, f] = HEAD;
    [
        a, b, c, d, e, f, 0x04, 0x01, 0x16, led, red, green, blue, SOLID, SYSEX_END,
    ]
}

/// Sets knob `dial`'s value inside the deck.
const fn realign(dial: u8, value: u8) -> [u8; 12] {
    let [a, b, c, d, e, f] = HEAD;
    [
        a,
        b,
        c,
        d,
        e,
        f,
        0x02,
        0x0f,
        0x40,
        FIRST_KNOB_ID + dial,
        value,
        SYSEX_END,
    ]
}

/// A two-line text message on the whole screen.
fn display(line_1: &str, line_2: &str) -> Vec<u8> {
    let text = |line: &str, max: usize| -> Vec<u8> {
        line.bytes()
            .filter(|byte| (0x20..0x7f).contains(byte))
            .take(max)
            .collect()
    };
    let mut out = HEAD.to_vec();
    out.extend_from_slice(&[0x04, 0x01, 0x60, 0x12, 0x01]);
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
struct Knob {
    last: Option<u8>,
    /// The value before Ryu realigned it, while a turn may still be read
    /// against it.
    before_centre: Option<u8>,
    /// When it last moved, until it is realigned.
    moved: Option<Instant>,
}

pub struct KeyLabEssential3 {
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
    pushed: bool,
    knobs: [Knob; KNOBS as usize],
    faders: [Option<u8>; FADERS as usize],
    fader_rest: [i16; FADERS as usize],
    parser: Parser,
}

impl KeyLabEssential3 {
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
            pushed: false,
            knobs: [Knob::default(); KNOBS as usize],
            faders: [None; FADERS as usize],
            fader_rest: [0; FADERS as usize],
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

    /// Realigns every knob to 64, re-seeds the faders and forgets which
    /// lights were sent, so the next flush sends them all.
    fn enter_daw(&mut self) {
        for dial in 0..KNOBS {
            self.send(Slot::Realign(dial), realign(dial, CENTRE).to_vec());
            if let Some(knob) = self.knobs.get_mut(usize::from(dial)) {
                *knob = Knob {
                    last: Some(CENTRE),
                    ..Knob::default()
                };
            }
        }
        self.faders = [None; FADERS as usize];
        self.fader_rest = [0; FADERS as usize];
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

    /// Realigns every knob that has been still long enough to 64.
    fn recentre(&mut self) {
        if !self.daw {
            return;
        }
        for dial in 0..KNOBS {
            let Some(knob) = self.knobs.get_mut(usize::from(dial)) else {
                continue;
            };
            let Some(moved) = knob.moved else {
                continue;
            };
            if moved.elapsed() < self.idle {
                continue;
            }
            knob.moved = None;
            if knob.last != Some(CENTRE) {
                knob.before_centre = knob.last;
                knob.last = Some(CENTRE);
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

    fn push(&mut self, pressed: bool, events: &mut Vec<Input>) {
        if self.pushed != pressed {
            self.pushed = pressed;
            events.push(Input::DialPress {
                index: MAIN_DIAL,
                pressed,
            });
        }
    }

    /// Releases everything held, as the deck leaves the DAW program.
    fn release_all(&mut self, events: &mut Vec<Input>) {
        for key in 0..KEYS {
            self.press(key, false, events);
        }
        self.push(false, events);
    }

    fn knob(&mut self, dial: u8, value: u8, events: &mut Vec<Input>) {
        let Some(knob) = self.knobs.get_mut(usize::from(dial)) else {
            return;
        };
        knob.moved = Some(Instant::now());
        let before = knob.before_centre.take();
        let Some(last) = knob.last.replace(value) else {
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
        match *bytes.strip_prefix(&PROGRAM_HEAD[..])? {
            [value, SYSEX_END] => Some(value == DAW_PROGRAM),
            _ => None,
        }
    }

    fn control(&mut self, number: u8, value: u8, events: &mut Vec<Input>) {
        if let Some(at) = BUTTON_CCS.iter().position(|known| *known == number) {
            let key = 16 + u8::try_from(at).unwrap_or(0);
            self.press(key, value > 0, events);
            return;
        }
        match number {
            MAIN_PUSH_CC => self.push(value > 0, events),
            MAIN_CC => {
                let delta = i16::from(value) - i16::from(CENTRE);
                if delta != 0 {
                    events.push(Input::DialTurn {
                        index: MAIN_DIAL,
                        delta: clamp(delta),
                    });
                }
            }
            cc if (FIRST_KNOB_CC..FIRST_KNOB_CC + KNOBS).contains(&cc) => {
                self.knob(cc - FIRST_KNOB_CC, value, events);
            }
            cc if (FIRST_FADER_CC..FIRST_FADER_CC + FADERS).contains(&cc) => {
                self.fader(cc - FIRST_FADER_CC, value, events);
            }
            _ => {}
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
            (kind @ (NOTE_ON | NOTE_OFF), PAD_CHANNEL) => {
                if let Some(key) = pad_key(number) {
                    self.press(key, kind == NOTE_ON && value > 0, events);
                }
            }
            (CONTROL_CHANGE, 0) => self.control(number, value, events),
            _ => {}
        }
    }
}

impl Driver for KeyLabEssential3 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.start();
        let mut events = Vec::new();
        for message in &self.read(timeout_ms)? {
            self.receive(message, &mut events);
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

impl Drop for KeyLabEssential3 {
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

    const LIT: usize = 33;

    fn open() -> (KeyLabEssential3, Recorder) {
        let recorder = Recorder::default();
        let deck = KeyLabEssential3::new(
            by_id(KEYLAB_ESSENTIAL_49_MK3_ID).unwrap(),
            Box::new(recorder.clone()),
        );
        (deck, recorder)
    }

    /// A deck past its open burst, with those writes cleared.
    fn ready() -> (KeyLabEssential3, Recorder) {
        let (mut deck, recorder) = open();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 3 + 9 + LIT);
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
            0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x04, 0x01, 0x16, led, rgb[0], rgb[1], rgb[2],
            0x20, 0xf7,
        ]
    }

    fn realign_bytes(id: u8) -> Vec<u8> {
        vec![
            0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x02, 0x0f, 0x40, id, 0x40, 0xf7,
        ]
    }

    fn program_bytes(value: u8) -> Vec<u8> {
        vec![
            0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x21, 0x11, 0x40, 0x02, 0x00, value, 0xf7,
        ]
    }

    fn port(id: &str, name: &str) -> bool {
        port_matches(by_id(id).unwrap(), &normalise(name))
    }

    #[test]
    fn each_size_matches_only_its_own_main_port() {
        let k49 = KEYLAB_ESSENTIAL_49_MK3_ID;
        assert!(port(k49, "KeyLab Essential 49 mk3 MIDI"));
        assert!(port(k49, "KL Essential 49 mk3 MIDI"));
        assert!(port(k49, "2- KL Essential 49 mk3 MIDI"));
        assert!(port(
            k49,
            "KeyLab Essential 49 mk3:KeyLab Essential 49 mk3 MIDI 24:0"
        ));
        assert!(!port(k49, "KeyLab Essential 61 mk3 MIDI"));
        assert!(!port(k49, "KeyLab Essential 49 mk3 DIN THRU"));
        assert!(!port(k49, "KeyLab Essential 49 mk3 MCU/HUI"));
        assert!(!port(k49, "KeyLab Essential 49 mk3 ALV"));
        assert!(!port(k49, "MIDIIN2 (KL Essential 49 mk3 MIDI)"));
        assert!(!port(k49, "KeyLab Essential 49 MIDI"));
        assert!(!port(k49, "KeyLab Essential 49 mk3"));
        assert!(port(KEYLAB_ESSENTIAL_61_MK3_ID, "KL Essential 61 mk3 MIDI"));
        assert!(port(
            KEYLAB_ESSENTIAL_88_MK3_ID,
            "KeyLab Essential 88 mk3 MIDI"
        ));
        assert!(!port(
            KEYLAB_ESSENTIAL_88_MK3_ID,
            "KeyLab Essential 61 mk3 MIDI"
        ));
        for (name, id) in [
            ("KeyLab Essential 49 mk3 MIDI", k49),
            ("KL Essential 61 mk3 MIDI", KEYLAB_ESSENTIAL_61_MK3_ID),
            ("KeyLab Essential 88 mk3 MIDI", KEYLAB_ESSENTIAL_88_MK3_ID),
        ] {
            assert_eq!(identify_midi(&normalise(name), false).unwrap().id, id);
        }
    }

    #[test]
    fn the_models_light_every_key_but_bank_and_no_dial() {
        for (id, usb) in [
            (KEYLAB_ESSENTIAL_49_MK3_ID, "1c75:024c"),
            (KEYLAB_ESSENTIAL_61_MK3_ID, "1c75:028c"),
            (KEYLAB_ESSENTIAL_88_MK3_ID, "1c75:02cc"),
        ] {
            let model = by_id(id).unwrap();
            assert_eq!(model.dial_lights, DialLights::None);
            assert_eq!(model.dial_light_colors(), 0);
            assert_eq!(
                (model.rows, model.cols, model.keys, model.dials),
                (5, 8, 34, 19)
            );
            let view = ModelView::from(model);
            assert_eq!(view.usb_ids, vec![usb.to_owned()]);
            assert!(view.key_lights && view.brightness);
            let lit = lit_keys(model).unwrap();
            assert_eq!(lit.len(), LIT);
            assert!(!lit.contains(&BANK_KEY));
        }
    }

    #[test]
    fn open_connects_requests_the_program_writes_the_screen_realigns_then_lights() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 128, 0]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 3 + 9 + LIT);
        assert_eq!(writes[0], CONNECT);
        assert_eq!(writes[1], REQUEST_PROGRAM);
        let mut screen = vec![
            0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x04, 0x01, 0x60, 0x12, 0x01,
        ];
        screen.extend_from_slice(b"Ryu");
        screen.extend_from_slice(&[0x00, 0x02]);
        screen.extend_from_slice(b"Connected");
        screen.extend_from_slice(&[0x00, 0xf7]);
        assert_eq!(writes[2], screen);
        for dial in 0..9u8 {
            assert_eq!(writes[3 + usize::from(dial)], realign_bytes(3 + dial));
        }
        assert_eq!(writes[11], realign_bytes(0x0b));
        // Pads 1C-2B, Context 18-1B, Save to Tap 0C-17, Part 07.
        assert_eq!(writes[12], led_bytes(0x1c, [127, 64, 0]));
        assert_eq!(writes[13], led_bytes(0x1d, [0, 0, 0]));
        assert_eq!(writes[27], led_bytes(0x2b, [0, 0, 0]));
        assert_eq!(writes[28], led_bytes(0x18, [0, 0, 0]));
        assert_eq!(writes[31], led_bytes(0x1b, [0, 0, 0]));
        assert_eq!(writes[32], led_bytes(0x0c, [0, 0, 0]));
        assert_eq!(writes[43], led_bytes(0x17, [0, 0, 0]));
        assert_eq!(writes[44], led_bytes(0x07, [0, 0, 0]));
        // Only SysEx is ever sent, and only once.
        assert!(writes.iter().all(|write| write[0] == 0xf0));
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 3 + 9 + LIT);
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
        assert_eq!(recorder.writes().len(), 3 + 9 + LIT);
    }

    #[test]
    fn flushes_send_only_changed_lights_and_brightness_scales_them() {
        let (mut deck, recorder) = ready();
        deck.set_face(8, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(29, Some(&lit([40, 160, 255]))).unwrap();
        deck.set_face(32, Some(&lit([0, 255, 0]))).unwrap();
        // Bank has no light.
        deck.set_face(33, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                led_bytes(0x24, [127, 127, 127]),
                led_bytes(0x15, [20, 80, 127]),
                led_bytes(0x07, [0, 127, 0]),
            ]
        );
        deck.set_brightness(50).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[3], led_bytes(0x24, [63, 63, 63]));
        deck.set_brightness(0).unwrap();
        deck.set_face(8, None).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 9);
        assert_eq!(writes[6], led_bytes(0x24, [0, 0, 0]));
        assert_eq!(writes[7], led_bytes(0x15, [0, 0, 0]));
        assert_eq!(writes[8], led_bytes(0x07, [0, 0, 0]));
    }

    #[test]
    fn pads_follow_the_panel_rows_of_each_bank() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0x9a, 40, 100],
                &[0x8a, 40, 0],
                &[0x9a, 36, 100],
                &[0x9a, 36, 0],
                &[0x9a, 43, 1],
                &[0x9a, 39, 1],
                &[0x9a, 48, 1],
                &[0x9a, 51, 1],
                &[0x9a, 44, 1],
                &[0x9a, 47, 1],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(4, true),
                key(4, false),
                key(3, true),
                key(7, true),
                key(8, true),
                key(11, true),
                key(12, true),
                key(15, true),
            ]
        );
    }

    #[test]
    fn buttons_press_and_release_keys() {
        let (mut deck, recorder) = ready();
        let mut sent: Vec<Vec<u8>> = Vec::new();
        let mut want = Vec::new();
        for (at, cc) in BUTTON_CCS.iter().enumerate() {
            sent.push(vec![0xb0, *cc, 127]);
            sent.push(vec![0xb0, *cc, 0]);
            let index = 16 + u8::try_from(at).unwrap();
            want.push(key(index, true));
            want.push(key(index, false));
        }
        for message in &sent {
            recorder.queue_read(message.clone());
        }
        assert_eq!(deck.poll(0).unwrap(), want);
        // Spot-check the panel order.
        queue(
            &recorder,
            &[
                &[0xb0, 44, 127],
                &[0xb0, 40, 127],
                &[0xb0, 24, 127],
                &[0xb0, 20, 127],
                &[0xb0, 23, 127],
                &[0xb0, 119, 127],
                &[0xb0, 118, 127],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(16, true),
                key(20, true),
                key(24, true),
                key(28, true),
                key(31, true),
                key(32, true),
                key(33, true),
            ]
        );
    }

    #[test]
    fn keybed_wheels_aftertouch_and_other_channels_are_ignored() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                // Keybed on channel 0, pad notes on another channel.
                &[0x90, 60, 100],
                &[0x90, 40, 100],
                &[0x99, 36, 100],
                // Pad notes outside 36-51 on the pads' channel.
                &[0x9a, 35, 100],
                &[0x9a, 52, 100],
                // Aftertouch, pitch bend, mod wheel, sustain, a button CC on
                // channel 1, an unknown CC.
                &[0xaa, 36, 50],
                &[0xda, 50],
                &[0xe0, 0, 64],
                &[0xb0, 1, 64],
                &[0xb0, 64, 127],
                &[0xb1, 44, 127],
                &[0xb0, 114, 127],
                // The identity reply, a pad-bank report and a short SysEx.
                &[
                    0xf0, 0x7e, 0x7f, 0x06, 0x02, 0x00, 0x20, 0x6b, 0x02, 0x00, 0x05, 0x00, 0xf7,
                ],
                &[
                    0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x21, 0x11, 0x40, 0x00, 0x00, 0x01, 0xf7,
                ],
                &[0xf0, 0x00, 0x20, 0xf7],
            ],
        );
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn the_display_encoder_is_relative_around_64_and_its_push_is_a_dial_press() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0xb0, 116, 65],
                &[0xb0, 116, 61],
                &[0xb0, 116, 64],
                &[0xb0, 117, 127],
                &[0xb0, 117, 0],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(9, 1),
                turn(9, -3),
                Input::DialPress {
                    index: 9,
                    pressed: true
                },
                Input::DialPress {
                    index: 9,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn knobs_turn_from_64_and_are_realigned_once_still() {
        let (mut deck, recorder) = ready();
        deck.idle = Duration::from_secs(60);
        // The knobs start at the realigned 64.
        queue(&recorder, &[&[0xb0, 96, 66], &[0xb0, 104, 60]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(0, 2), turn(8, -4)]);
        queue(&recorder, &[&[0xb0, 96, 67]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(0, 1)]);
        assert!(recorder.writes().is_empty());
        // Still long enough: both are put back to 64.
        deck.idle = Duration::ZERO;
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![realign_bytes(0x03), realign_bytes(0x0b)]
        );
        // The next value counts from 64.
        queue(&recorder, &[&[0xb0, 96, 65]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(0, 1)]);
    }

    #[test]
    fn a_turn_that_beat_the_realign_counts_from_the_old_value() {
        let (mut deck, recorder) = ready();
        queue(&recorder, &[&[0xb0, 98, 100]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(2, 36)]);
        assert_eq!(recorder.writes(), vec![realign_bytes(0x05)]);
        // The deck had not taken the realign yet.
        deck.idle = Duration::from_secs(60);
        queue(&recorder, &[&[0xb0, 98, 101]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(2, 1)]);
    }

    #[test]
    fn faders_seed_then_step_every_two_units_and_are_never_sent() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0xb0, 105, 10],
                &[0xb0, 105, 14],
                &[0xb0, 105, 15],
                &[0xb0, 105, 16],
                &[0xb0, 113, 127],
                &[0xb0, 113, 117],
                &[0xb0, 106, 0],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(10, 2), turn(10, 1), turn(18, -5)]
        );
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn another_program_mutes_input_and_lights_until_the_daw_program_returns() {
        let (mut deck, recorder) = ready();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        queue(&recorder, &[&[0x9a, 41, 100], &[0xb0, 117, 127]]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(1, true),
                Input::DialPress {
                    index: 9,
                    pressed: true
                }
            ]
        );
        // The Arturia program: held keys are let go, input ignored.
        queue(
            &recorder,
            &[&program_bytes(0x00), &[0x9a, 42, 100], &[0xb0, 116, 70]],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(1, false),
                Input::DialPress {
                    index: 9,
                    pressed: false
                }
            ]
        );
        deck.set_face(1, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
        // The User program is not the DAW one either.
        queue(&recorder, &[&program_bytes(0x02)]);
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(recorder.writes().is_empty());
        // Back to DAW: realign, then every light again.
        queue(&recorder, &[&program_bytes(0x01)]);
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(writes.len(), 9 + LIT);
        assert_eq!(writes[0], realign_bytes(0x03));
        assert_eq!(writes[9], led_bytes(0x1c, [127, 0, 0]));
        assert_eq!(writes[10], led_bytes(0x1d, [0, 127, 0]));
        queue(&recorder, &[&[0x9a, 42, 100]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(2, true)]);
    }

    #[test]
    fn short_and_malformed_input_never_panics() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0x9a],
                &[0xb0, 96],
                &[0xf0],
                &[0xf7],
                &[
                    0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x21, 0x11, 0x40, 0x02, 0x00, 0xf7,
                ],
                &[
                    0xf0, 0x00, 0x20, 0x6b, 0x7f, 0x42, 0x21, 0x11, 0x40, 0x02, 0xf7,
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
        assert_eq!(writes.first(), Some(&led_bytes(0x1f, [127, 0, 0])));
        // A light was left unsent, so every light goes off.
        assert_eq!(writes.len(), 1 + LIT + 1);
        assert_eq!(writes[1], led_bytes(0x1c, [0, 0, 0]));
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
