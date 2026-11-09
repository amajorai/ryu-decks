//! PreSonus' FaderPort 2 (2018), FaderPort 8 and FaderPort 16, over
//! class-compliant USB MIDI ([`super::midi`]), in their native (Studio One)
//! mode. The three share one protocol; only the note tables and the number of
//! strips differ.
//!
//! The deck must already be in native mode, chosen on the deck itself as it
//! powers up: in its Mackie Control or HUI mode it speaks another protocol,
//! which Ryu does not drive (Ryu logs a hint on a note it does not know).
//! There is no handshake.
//!
//! Ports: the FaderPort 2 lists `PreSonus FP2`; the FaderPort 8 and 16 list
//! `PreSonus FP8 Port 1` / `PreSonus FP16 Port 1` and a `Port 2` that native
//! mode does not use. [`port_matches`] takes the model's `FP2` / `FP8` /
//! `FP16` word next to `PRESONUS` (or `FaderPort 8` and the like), never a
//! `PORT 2`, and on Windows the first interface.
//!
//! In, on any channel:
//! - Buttons and fader touches are notes. A Note On above velocity 64 presses
//!   one; a Note On at 64 or less releases it. The reference also reads a
//!   Note Off's velocity that way; Ryu deliberately takes every Note Off as a
//!   release.
//! - Strip `s` (0-7): Solo `08+s`, Mute `10+s`, Select `18+s`, fader touch
//!   `68+s`. On the FaderPort 16, strips 8-15 (`i = s - 8`): Solo
//!   `50 51 52 58 54 55 59 57`, Mute `78+i`, Select `07 21 22 ... 27`, touch
//!   `68+s`.
//! - Each fader is a 14-bit pitch bend on its strip's channel, 0 to 16368:
//!   absolute, so its first value seeds it and every 16 tenth-bit units (64
//!   steps end to end) turn its dial a step.
//! - The Pan/Param encoder is `CC 16` and the Session Navigator `CC 60`:
//!   bit 6 set turns anticlockwise, bits 0-5 count the steps.
//!
//! Out:
//! - A button's light is `90 <note> <7F on / 00 off>`. The RGB buttons take
//!   their colour as `91 / 92 / 93 <note> <7-bit red / green / blue>`, shown
//!   while the note is on; Ryu resends the colour only when it changes.
//! - At open, before the lights, each strip's scribble strip is cleared
//!   (FaderPort 8 and 16: `F0 00 01 06 <02, or 16 on the FP16> 13 <s> 10 F7`,
//!   then `... 12 <s> <line> 00 F7` for its four lines), its value bar turned
//!   off (`B0 <bar> 00`, `B0 <bar mode> 04`) and its meter and peak reset.
//!   The faders' motors are never sent (the reference parks them at the
//!   bottom; Ryu leaves them where they are).
//! - The deck's MIDI buffer is small, so writes wait in a queue drained from
//!   poll and flush, never by sleeping in the shared loop: 400 µs per byte
//!   before a message, 1.5 ms before a blue colour message, nothing before a
//!   red or green one (the pace is zero in tests).
//! - The FaderPort 16 needs `A0 00 00` about once a second to stay with the
//!   host. No brightness command.
//! - Letting go turns every light off.
//!
//! Facts from Ardour's FaderPort 8 control surface (C++, GPL-2.0-or-later),
//! with the USB ids from Ableton Live's FaderPort Remote Scripts (decompiled,
//! no licence; facts only); no code was copied. Which of the shared buttons
//! the FaderPort 2 really has is partly inferred.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const FADERPORT2_ID: &str = "presonus-faderport-2";
pub const FADERPORT8_ID: &str = "presonus-faderport-8";
pub const FADERPORT16_ID: &str = "presonus-faderport-16";

pub const FP2_KEYS: u8 = 28;
pub const FP2_DIALS: u8 = 2;
pub const FP2_ROWS: u8 = 4;
pub const FP2_COLS: u8 = 7;
/// Select, Mute, Solo and touch per strip, then the 42 global buttons.
pub const FP8_KEYS: u8 = 8 * 4 + GLOBALS_LEN as u8;
/// The two encoders, then the eight faders.
pub const FP8_DIALS: u8 = 2 + 8;
pub const FP8_ROWS: u8 = 10;
pub const FP8_COLS: u8 = 8;
pub const FP16_KEYS: u8 = 16 * 4 + GLOBALS_LEN as u8;
pub const FP16_DIALS: u8 = 2 + 16;
pub const FP16_ROWS: u8 = 7;
pub const FP16_COLS: u8 = 16;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const RED: u8 = 0x91;
const GREEN: u8 = 0x92;
const BLUE: u8 = 0x93;
const KEEP_ALIVE_STATUS: u8 = 0xa0;
const CONTROL_CHANGE: u8 = 0xb0;
const PITCH_BEND: u8 = 0xe0;
const PRESS_ABOVE: u8 = 0x40;
const ON: u8 = 0x7f;
const OFF: u8 = 0x00;
const PARAM_CC: u8 = 0x10;
const NAVIGATOR_CC: u8 = 0x3c;
const PARAM_PUSH: u8 = 0x20;
const NAVIGATOR_PUSH: u8 = 0x53;
const FOOTSWITCH: u8 = 0x66;
const FIRST_TOUCH: u8 = 0x68;
const LAST_TOUCH: u8 = 0x77;
/// A relative turn's direction bit and step count.
const ANTICLOCKWISE: u8 = 0x40;
const STEPS: u8 = 0x3f;

const SYSEX_HEADER: [u8; 4] = [0xf0, 0x00, 0x01, 0x06];
const SYSEX_END: u8 = 0xf7;
const TEXT: u8 = 0x12;
const STRIP_MODE: u8 = 0x13;
/// Strip mode 0 with its clear bit.
const MODE_CLEAR: u8 = 0x10;
const LINES: u8 = 4;
const BAR_OFF: u8 = 4;
const KEEP_ALIVE: [u8; 3] = [KEEP_ALIVE_STATUS, 0x00, 0x00];

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
const BLACK: [u8; 3] = [0, 0, 0];
/// Tenth-bit fader units per dial step: 64 steps end to end.
const FADER_STEP: i32 = 16;
/// The pace unit: gaps are counted in these.
const PACE_UNIT: Duration = Duration::from_micros(100);
/// Gaps, in pace units: before each byte of most messages, and before a
/// blue colour message.
const GAP_PER_BYTE: u32 = 4;
const GAP_BLUE: u32 = 15;
/// How far behind the clock may fall: the most written at once after a
/// quiet spell.
const MAX_CREDIT: Duration = Duration::from_millis(10);
const KEEP_ALIVE_EVERY: Duration = Duration::from_secs(1);
const READ_LEN: usize = 256;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// The global buttons of the FaderPort 8 and 16, in key order.
const GLOBALS_LEN: usize = 42;
const GLOBALS: [(u8, &str); GLOBALS_LEN] = [
    (0x00, "Arm"),
    (0x01, "Solo Clear"),
    (0x02, "Mute Clear"),
    (0x03, "Bypass"),
    (0x04, "Macro"),
    (0x05, "Link"),
    (0x06, "Shift L"),
    (0x46, "Shift R"),
    (0x28, "Track"),
    (0x29, "Send"),
    (0x2a, "Pan"),
    (0x2b, "Plugins"),
    (0x2e, "Prev"),
    (0x2f, "Next"),
    (0x20, "Param Push"),
    (0x53, "Navigator Push"),
    (0x36, "Channel"),
    (0x37, "Zoom"),
    (0x38, "Scroll"),
    (0x39, "Bank"),
    (0x3a, "Master"),
    (0x3b, "Click"),
    (0x3c, "Section"),
    (0x3d, "Marker"),
    (0x3e, "Audio"),
    (0x3f, "VI"),
    (0x40, "Bus"),
    (0x41, "VCA"),
    (0x42, "All"),
    (0x4a, "Read"),
    (0x4b, "Write"),
    (0x4c, "Trim"),
    (0x4d, "Touch"),
    (0x4e, "Latch"),
    (0x4f, "Off"),
    (0x56, "Loop"),
    (0x5b, "Rewind"),
    (0x5c, "Fast Forward"),
    (0x5d, "Stop"),
    (0x5e, "Play"),
    (0x5f, "Record"),
    (FOOTSWITCH, "Footswitch"),
];

/// The FaderPort 2's buttons, in key order. The reference shares the FaderPort
/// 8's table with it and marks only some notes as missing, so this set is
/// partly inferred.
const FP2: [(u8, &str); FP2_KEYS as usize] = [
    (0x00, "Arm"),
    (0x03, "Bypass"),
    (0x05, "Link"),
    (0x06, "Shift"),
    (0x08, "Solo"),
    (0x10, "Mute"),
    (0x18, "Select"),
    (PARAM_PUSH, "Encoder Push"),
    (0x2a, "Pan"),
    (0x2e, "Prev"),
    (0x2f, "Next"),
    (0x36, "Channel"),
    (0x38, "Scroll"),
    (0x3a, "Master"),
    (0x3b, "Click"),
    (0x3c, "Section"),
    (0x3d, "Marker"),
    (0x4a, "Read"),
    (0x4b, "Write"),
    (0x4d, "Touch"),
    (0x56, "Loop"),
    (0x5b, "Rewind"),
    (0x5c, "FFwd"),
    (0x5d, "Stop"),
    (0x5e, "Play"),
    (0x5f, "Record"),
    (FOOTSWITCH, "Footswitch"),
    (FIRST_TOUCH, "Fader Touch"),
];

/// Strip rows in key order; the fader touches are the fourth.
const SELECT: usize = 0;
const MUTE: usize = 1;
const SOLO: usize = 2;
const UPPER_SOLO: [u8; 8] = [0x50, 0x51, 0x52, 0x58, 0x54, 0x55, 0x59, 0x57];

const STRIP_NAMES: [[&str; 16]; 4] = [
    [
        "Select 1",
        "Select 2",
        "Select 3",
        "Select 4",
        "Select 5",
        "Select 6",
        "Select 7",
        "Select 8",
        "Select 9",
        "Select 10",
        "Select 11",
        "Select 12",
        "Select 13",
        "Select 14",
        "Select 15",
        "Select 16",
    ],
    [
        "Mute 1", "Mute 2", "Mute 3", "Mute 4", "Mute 5", "Mute 6", "Mute 7", "Mute 8", "Mute 9",
        "Mute 10", "Mute 11", "Mute 12", "Mute 13", "Mute 14", "Mute 15", "Mute 16",
    ],
    [
        "Solo 1", "Solo 2", "Solo 3", "Solo 4", "Solo 5", "Solo 6", "Solo 7", "Solo 8", "Solo 9",
        "Solo 10", "Solo 11", "Solo 12", "Solo 13", "Solo 14", "Solo 15", "Solo 16",
    ],
    [
        "Touch 1", "Touch 2", "Touch 3", "Touch 4", "Touch 5", "Touch 6", "Touch 7", "Touch 8",
        "Touch 9", "Touch 10", "Touch 11", "Touch 12", "Touch 13", "Touch 14", "Touch 15",
        "Touch 16",
    ],
];

/// The note of strip `strip`'s button in `row`.
const fn strip_note(row: usize, strip: u8) -> u8 {
    if strip < 8 {
        return match row {
            SELECT => 0x18 + strip,
            MUTE => 0x10 + strip,
            SOLO => 0x08 + strip,
            _ => FIRST_TOUCH + strip,
        };
    }
    let upper = strip - 8;
    match row {
        SELECT if upper == 0 => 0x07,
        SELECT => 0x20 + upper,
        MUTE => 0x78 + upper,
        SOLO => UPPER_SOLO[upper as usize & 7],
        _ => FIRST_TOUCH + strip,
    }
}

/// Every key's note on a deck of `STRIPS` strips: Select, Mute, Solo and
/// touch rows, then the globals.
const fn table<const N: usize, const STRIPS: u8>() -> [u8; N] {
    let mut notes = [0u8; N];
    let mut row = 0;
    while row < 4 {
        let mut strip = 0;
        while strip < STRIPS {
            notes[row * STRIPS as usize + strip as usize] = strip_note(row, strip);
            strip += 1;
        }
        row += 1;
    }
    let mut at = 0;
    while at < GLOBALS_LEN {
        notes[4 * STRIPS as usize + at] = GLOBALS[at].0;
        at += 1;
    }
    notes
}

const fn fp2_table() -> [u8; FP2_KEYS as usize] {
    let mut notes = [0u8; FP2_KEYS as usize];
    let mut at = 0;
    while at < FP2.len() {
        notes[at] = FP2[at].0;
        at += 1;
    }
    notes
}

const FP2_NOTES: [u8; FP2_KEYS as usize] = fp2_table();
const FP8_NOTES: [u8; FP8_KEYS as usize] = table::<{ FP8_KEYS as usize }, 8>();
const FP16_NOTES: [u8; FP16_KEYS as usize] = table::<{ FP16_KEYS as usize }, 16>();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Fp2,
    Fp8,
    Fp16,
}

impl Kind {
    fn of(model: &DeviceModel) -> Option<Self> {
        if model.protocol != Protocol::PresonusFaderPort8Midi {
            return None;
        }
        match model.id {
            FADERPORT2_ID => Some(Self::Fp2),
            FADERPORT8_ID => Some(Self::Fp8),
            FADERPORT16_ID => Some(Self::Fp16),
            _ => None,
        }
    }

    const fn notes(self) -> &'static [u8] {
        match self {
            Self::Fp2 => &FP2_NOTES,
            Self::Fp8 => &FP8_NOTES,
            Self::Fp16 => &FP16_NOTES,
        }
    }

    const fn strips(self) -> u8 {
        match self {
            Self::Fp2 => 1,
            Self::Fp8 => 8,
            Self::Fp16 => 16,
        }
    }

    /// The model word in its port name.
    const fn word(self) -> &'static str {
        match self {
            Self::Fp2 => "FP2",
            Self::Fp8 => "FP8",
            Self::Fp16 => "FP16",
        }
    }

    const fn number(self) -> &'static str {
        match self {
            Self::Fp2 => "2",
            Self::Fp8 => "8",
            Self::Fp16 => "16",
        }
    }

    /// The SysEx device byte.
    const fn device(self) -> u8 {
        match self {
            Self::Fp16 => 0x16,
            _ => 0x02,
        }
    }

    /// Whether it has scribble strips.
    const fn has_display(self) -> bool {
        !matches!(self, Self::Fp2)
    }

    /// The dial of strip 0's fader.
    const fn first_fader_dial(self) -> u8 {
        match self {
            Self::Fp2 => 1,
            _ => 2,
        }
    }

    /// The dial an encoder's Control Change turns.
    const fn encoder_dial(self, cc: u8) -> Option<u8> {
        match (self, cc) {
            (_, PARAM_CC) | (Self::Fp2, NAVIGATOR_CC) => Some(0),
            (_, NAVIGATOR_CC) => Some(1),
            _ => None,
        }
    }

    /// The key a note presses.
    fn key_of(self, note: u8) -> Option<u8> {
        let note = if self == Self::Fp2 && note == NAVIGATOR_PUSH {
            PARAM_PUSH
        } else {
            note
        };
        let at = self.notes().iter().position(|known| *known == note)?;
        u8::try_from(at).ok()
    }

    /// Whether a note's light takes a colour.
    const fn is_rgb(self, note: u8) -> bool {
        matches!(note, 0x03..=0x05 | 0x3e..=0x42 | 0x4a..=0x4f | 0x18..=0x1f)
            || (matches!(self, Self::Fp16) && matches!(note, 0x07 | 0x21..=0x27))
            || (matches!(self, Self::Fp2) && matches!(note, 0x2a | 0x36 | 0x38))
    }
}

/// Whether a note's button has a light: not the encoder pushes, the
/// footswitch or a fader touch.
const fn has_light(note: u8) -> bool {
    !matches!(
        note,
        PARAM_PUSH | NAVIGATOR_PUSH | FOOTSWITCH | FIRST_TOUCH..=LAST_TOUCH
    )
}

/// Whether a normalised port name is this model's native port: `PRESONUS`
/// and its `FP2` / `FP8` / `FP16` word (or the word first), or `FADERPORT`
/// and its number, never a `PORT 2`, and on Windows the first interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    let Some(kind) = Kind::of(model) else {
        return false;
    };
    if name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.to_ascii_uppercase();
    if base.contains("PORT 2") || base.contains("PORT2") {
        return false;
    }
    let words: Vec<&str> = base
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let presonus = words.contains(&"PRESONUS");
    let by_word = words
        .iter()
        .enumerate()
        .any(|(at, word)| *word == kind.word() && (presonus || at == 0));
    let joined = format!("FADERPORT{}", kind.number());
    let by_number = words.iter().enumerate().any(|(at, word)| {
        *word == joined
            || (*word == "FADERPORT" && words.get(at + 1).copied() == Some(kind.number()))
    });
    by_word || by_number
}

/// The legend on each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    let kind = Kind::of(model)?;
    if kind == Kind::Fp2 {
        return Some(FP2.iter().map(|(_, name)| *name).collect());
    }
    let strips = usize::from(kind.strips());
    let mut names = Vec::with_capacity(strips * 4 + GLOBALS_LEN);
    for row in STRIP_NAMES {
        names.extend(row.iter().take(strips).copied());
    }
    names.extend(GLOBALS.iter().map(|(_, name)| *name));
    Some(names)
}

/// The keys with a light.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    let kind = Kind::of(model)?;
    Some(
        (0u8..)
            .zip(kind.notes())
            .filter(|(_, note)| has_light(**note))
            .map(|(key, _)| key)
            .collect(),
    )
}

/// A relative turn: bit 6 anticlockwise, bits 0-5 the steps.
fn relative(value: u8) -> Option<i8> {
    let steps = i8::try_from(value & STEPS).ok()?;
    if steps == 0 {
        return None;
    }
    Some(if value & ANTICLOCKWISE == 0 {
        steps
    } else {
        -steps
    })
}

/// How long to wait before writing `message`, in pace units.
fn gap(message: &[u8]) -> u32 {
    match message {
        [RED | GREEN, _, _] => 0,
        [BLUE, _, _] => GAP_BLUE,
        _ => GAP_PER_BYTE.saturating_mul(u32::try_from(message.len()).unwrap_or(u32::MAX)),
    }
}

fn sysex(device: u8, body: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(SYSEX_HEADER.len() + 2 + body.len());
    message.extend_from_slice(&SYSEX_HEADER);
    message.push(device);
    message.extend_from_slice(body);
    message.push(SYSEX_END);
    message
}

/// A fader read as a dial: its first position seeds it, then every
/// [`FADER_STEP`] tenth-bit units moved turn the dial one step.
#[derive(Debug, Default, Clone, Copy)]
struct Fader {
    last: Option<i32>,
    rest: i32,
}

impl Fader {
    fn moved(&mut self, bend: u16) -> i8 {
        let position = i32::from(bend >> 4);
        let Some(old) = self.last.replace(position) else {
            return 0;
        };
        let moved = self.rest + position - old;
        let steps = moved / FADER_STEP;
        self.rest = moved - steps * FADER_STEP;
        i8::try_from(steps.clamp(-127, 127)).unwrap_or(0)
    }
}

pub struct FaderPort8 {
    kind: Kind,
    transport: Box<dyn Transport>,
    /// The strips' reset and every light are queued.
    started: bool,
    /// Writes waiting their turn, drained by poll and flush.
    outbox: VecDeque<Vec<u8>>,
    /// When the last write went out, as far as pacing counts.
    clock: Instant,
    pace: Duration,
    keep_alive: Duration,
    last_keep_alive: Instant,
    colors: Vec<[u8; 3]>,
    /// Each key's light as last queued: on or off, and an RGB key's 7-bit
    /// colour.
    sent_on: Vec<Option<bool>>,
    sent_color: Vec<Option<[u8; 3]>>,
    held: Vec<bool>,
    faders: Vec<Fader>,
    /// The hint about another mode has been logged.
    warned: bool,
    parser: Parser,
}

impl FaderPort8 {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model).unwrap_or(Kind::Fp8);
        let keys = kind.notes().len();
        let now = Instant::now();
        Self {
            kind,
            transport,
            started: false,
            outbox: VecDeque::new(),
            clock: now,
            pace: PACE_UNIT,
            keep_alive: KEEP_ALIVE_EVERY,
            last_keep_alive: now,
            colors: vec![BLACK; keys],
            sent_on: vec![None; keys],
            sent_color: vec![None; keys],
            held: vec![false; keys],
            faders: vec![Fader::default(); usize::from(kind.strips())],
            warned: false,
            parser: Parser::default(),
        }
    }

    /// Each strip's display cleared and its bar, meter and peak reset.
    fn queue_strip_resets(&mut self) {
        let device = self.kind.device();
        for strip in 0..self.kind.strips() {
            if self.kind.has_display() {
                self.outbox
                    .push_back(sysex(device, &[STRIP_MODE, strip, MODE_CLEAR]));
                for line in 0..LINES {
                    self.outbox
                        .push_back(sysex(device, &[TEXT, strip, line, 0x00]));
                }
            }
            let (bar, bar_mode, meter, peak) = if strip < 8 {
                (0x30 + strip, 0x38 + strip, 0xd0 + strip, 0xd8 + strip)
            } else {
                let upper = strip - 8;
                (0x40 + upper, 0x48 + upper, 0xc0 + upper, 0xc8 + upper)
            };
            self.outbox.extend([
                vec![CONTROL_CHANGE, bar, 0x00],
                vec![CONTROL_CHANGE, bar_mode, BAR_OFF],
                vec![meter, 0x00],
                vec![peak, 0x00],
            ]);
        }
    }

    /// Queues a key's light if it differs from what was queued, replacing
    /// that key's light still waiting.
    fn queue_key(&mut self, key: usize) {
        let Some(note) = self.kind.notes().get(key).copied() else {
            return;
        };
        if !has_light(note) {
            return;
        }
        let color = self.colors.get(key).copied().unwrap_or(BLACK);
        let on = color.iter().any(|channel| *channel >= DARK);
        let rgb = self.kind.is_rgb(note);
        let seven = color.map(|channel| channel >> 1);
        let sent_on = self.sent_on.get(key).copied().flatten();
        let sent_color = self.sent_color.get(key).copied().flatten();
        let color_due = rgb && on && sent_color != Some(seven);
        if sent_on == Some(on) && !color_due {
            return;
        }
        let waiting = |queued: &Vec<u8>| {
            queued.len() == 3 && (NOTE_ON..=BLUE).contains(&queued[0]) && queued[1] == note
        };
        let mut color_due = color_due;
        if self.outbox.iter().any(waiting) {
            self.outbox.retain(|queued| !waiting(queued));
            color_due = rgb && on;
        }
        if color_due {
            let [red, green, blue] = seven;
            self.outbox.extend([
                vec![RED, note, red],
                vec![GREEN, note, green],
                vec![BLUE, note, blue],
            ]);
            if let Some(sent) = self.sent_color.get_mut(key) {
                *sent = Some(seven);
            }
        }
        self.outbox
            .push_back(vec![NOTE_ON, note, if on { ON } else { OFF }]);
        if let Some(sent) = self.sent_on.get_mut(key) {
            *sent = Some(on);
        }
    }

    fn queue_changes(&mut self) {
        for key in 0..self.colors.len() {
            self.queue_key(key);
        }
    }

    fn start(&mut self) {
        self.started = true;
        self.outbox.clear();
        self.sent_on.fill(None);
        self.sent_color.fill(None);
        self.queue_strip_resets();
        self.queue_changes();
        self.last_keep_alive = Instant::now();
    }

    /// Writes the queued messages whose gap has passed, never sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        let now = Instant::now();
        if let Some(floor) = now.checked_sub(MAX_CREDIT) {
            self.clock = self.clock.max(floor);
        }
        while let Some(message) = self.outbox.front() {
            let due = self.clock + self.pace * gap(message);
            if due > now {
                break;
            }
            self.transport.write(message)?;
            self.outbox.pop_front();
            self.clock = due;
        }
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

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        // SysEx and anything else from the deck is not used.
        let Message::Channel([status, first, second]) = *message else {
            return;
        };
        let channel = status & 0x0f;
        match status & 0xf0 {
            kind @ (NOTE_ON | NOTE_OFF) => {
                if let Some(key) = self.kind.key_of(first) {
                    self.press(key, kind == NOTE_ON && second > PRESS_ABOVE, events);
                } else if kind == NOTE_ON && !self.warned {
                    self.warned = true;
                    tracing::warn!(
                        note = first,
                        "the FaderPort sent a note its native mode does not; \
                         if it is in Mackie Control or HUI mode, power it up in native \
                         (Studio One) mode for Ryu to read it"
                    );
                }
            }
            CONTROL_CHANGE => {
                if let (Some(index), Some(delta)) =
                    (self.kind.encoder_dial(first), relative(second))
                {
                    events.push(Input::DialTurn { index, delta });
                }
            }
            PITCH_BEND => {
                let bend = u16::from(first & 0x7f) | (u16::from(second & 0x7f) << 7);
                let Some(fader) = self.faders.get_mut(usize::from(channel)) else {
                    return;
                };
                let delta = fader.moved(bend);
                if delta != 0 {
                    events.push(Input::DialTurn {
                        index: self.kind.first_fader_dial() + channel,
                        delta,
                    });
                }
            }
            _ => {}
        }
    }
}

impl Driver for FaderPort8 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
            self.start();
        }
        if self.kind == Kind::Fp16 && self.last_keep_alive.elapsed() >= self.keep_alive {
            self.last_keep_alive = Instant::now();
            self.outbox.push_back(KEEP_ALIVE.to_vec());
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

    /// The strips' reset and every light the first time, then the lights
    /// that changed; whatever the pace allows goes out now, the rest from
    /// later polls.
    fn flush(&mut self) -> DriverResult<()> {
        if self.started {
            self.queue_changes();
        } else {
            self.start();
        }
        self.pump()
    }

    /// The FaderPorts have no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for FaderPort8 {
    /// Turns every light off, each write its gap after the last (zero in
    /// tests), stopping at the first failed write (an unplugged deck).
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        self.outbox.clear();
        for note in self
            .kind
            .notes()
            .iter()
            .copied()
            .filter(|note| has_light(*note))
        {
            self.outbox.push_back(vec![NOTE_ON, note, OFF]);
        }
        while let Some(message) = self.outbox.pop_front() {
            let wait = self.pace * gap(&message);
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
            // A deck that has gone fails its first write: stop there, so the
            // shared device loop is not held through every remaining gap.
            if self.transport.write(&message).is_err() {
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

    fn open(id: &str) -> (FaderPort8, Recorder) {
        let recorder = Recorder::default();
        let mut deck = FaderPort8::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        deck.pace = Duration::ZERO;
        deck.keep_alive = Duration::from_secs(3600);
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

    /// FaderPort 8: 8 strips of mode, 4 text clears, bar, bar mode, meter
    /// and peak; then 63 lights.
    const FP8_START: usize = 8 * 9 + 63;

    #[test]
    fn the_note_tables_match_the_reference() {
        assert_eq!(FP8_NOTES.len(), 74);
        assert_eq!(
            FP8_NOTES[..8],
            [0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f]
        );
        assert_eq!(
            FP8_NOTES[8..16],
            [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17]
        );
        assert_eq!(
            FP8_NOTES[16..24],
            [0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f]
        );
        assert_eq!(
            FP8_NOTES[24..32],
            [0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f]
        );
        assert_eq!((FP8_NOTES[32], FP8_NOTES[73]), (0x00, 0x66));
        assert_eq!(FP16_NOTES.len(), 106);
        assert_eq!(
            FP16_NOTES[8..16],
            [0x07, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27]
        );
        assert_eq!(
            FP16_NOTES[24..32],
            [0x78, 0x79, 0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x7f]
        );
        assert_eq!(
            FP16_NOTES[40..48],
            [0x50, 0x51, 0x52, 0x58, 0x54, 0x55, 0x59, 0x57]
        );
        assert_eq!((FP16_NOTES[48], FP16_NOTES[63]), (0x68, 0x77));
        assert_eq!(FP16_NOTES[64..], FP8_NOTES[32..]);
        // No note names two keys.
        for notes in [&FP2_NOTES[..], &FP8_NOTES[..], &FP16_NOTES[..]] {
            for (at, note) in notes.iter().enumerate() {
                assert!(!notes[at + 1..].contains(note), "{note:#x}");
            }
        }
    }

    /// A transport whose device is gone: every write fails and is counted.
    struct Gone(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl Transport for Gone {
        fn write(&mut self, _data: &[u8]) -> DriverResult<()> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(super::super::DriverError::Io("gone".to_owned()))
        }

        fn read_timeout(&mut self, _buf: &mut [u8], _timeout_ms: i32) -> DriverResult<usize> {
            Ok(0)
        }

        fn send_feature_report(&mut self, _data: &[u8]) -> DriverResult<()> {
            Ok(())
        }

        fn get_feature_report(&mut self, _buf: &mut [u8]) -> DriverResult<usize> {
            Ok(0)
        }
    }

    #[test]
    fn dropping_a_deck_that_has_gone_stops_at_the_first_failed_write() {
        let (mut deck, _recorder) = open(FADERPORT8_ID);
        deck.poll(0).unwrap();
        let writes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        deck.transport = Box::new(Gone(writes.clone()));
        drop(deck);
        assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn opening_resets_the_strips_then_sends_every_light() {
        let (mut deck, recorder) = open(FADERPORT8_ID);
        deck.set_face(0, Some(&lit([255, 0, 128]))).unwrap();
        deck.set_face(32, Some(&lit([255, 255, 255]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), FP8_START + 3);
        assert_eq!(
            writes[..9],
            [
                vec![0xf0, 0x00, 0x01, 0x06, 0x02, 0x13, 0x00, 0x10, 0xf7],
                vec![0xf0, 0x00, 0x01, 0x06, 0x02, 0x12, 0x00, 0x00, 0x00, 0xf7],
                vec![0xf0, 0x00, 0x01, 0x06, 0x02, 0x12, 0x00, 0x01, 0x00, 0xf7],
                vec![0xf0, 0x00, 0x01, 0x06, 0x02, 0x12, 0x00, 0x02, 0x00, 0xf7],
                vec![0xf0, 0x00, 0x01, 0x06, 0x02, 0x12, 0x00, 0x03, 0x00, 0xf7],
                vec![0xb0, 0x30, 0x00],
                vec![0xb0, 0x38, 0x04],
                vec![0xd0, 0x00],
                vec![0xd8, 0x00],
            ]
        );
        assert_eq!(
            writes[63..68],
            [
                vec![0xf0, 0x00, 0x01, 0x06, 0x02, 0x13, 0x07, 0x10, 0xf7],
                vec![0xf0, 0x00, 0x01, 0x06, 0x02, 0x12, 0x07, 0x00, 0x00, 0xf7],
                vec![0xf0, 0x00, 0x01, 0x06, 0x02, 0x12, 0x07, 0x01, 0x00, 0xf7],
                vec![0xf0, 0x00, 0x01, 0x06, 0x02, 0x12, 0x07, 0x02, 0x00, 0xf7],
                vec![0xf0, 0x00, 0x01, 0x06, 0x02, 0x12, 0x07, 0x03, 0x00, 0xf7],
            ]
        );
        assert_eq!(
            writes[68..72],
            [
                vec![0xb0, 0x37, 0x00],
                vec![0xb0, 0x3f, 0x04],
                vec![0xd7, 0x00],
                vec![0xdf, 0x00],
            ]
        );
        // Select 1 (RGB): its colour then on; Select 2 off.
        assert_eq!(
            writes[72..77],
            [
                vec![0x91, 0x18, 0x7f],
                vec![0x92, 0x18, 0x00],
                vec![0x93, 0x18, 0x40],
                vec![0x90, 0x18, 0x7f],
                vec![0x90, 0x19, 0x00],
            ]
        );
        // No touch notes, encoder pushes or footswitch; Arm (mono) on.
        let lights: Vec<&Vec<u8>> = writes[72..].iter().filter(|w| w[0] == 0x90).collect();
        assert_eq!(lights.len(), 63);
        assert!(lights
            .iter()
            .all(|w| !matches!(w[1], 0x20 | 0x53 | 0x66 | 0x68..=0x6f)));
        assert!(lights.contains(&&vec![0x90, 0x00, 0x7f]));
        assert!(lights.contains(&&vec![0x90, 0x06, 0x00]));
        assert!(lights.contains(&&vec![0x90, 0x46, 0x00]));
        // No fader motor, ever.
        assert!(writes.iter().all(|w| w[0] & 0xf0 != 0xe0));
        // Nothing more until something changes.
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), FP8_START + 3);
    }

    #[test]
    fn colours_go_out_only_when_they_change() {
        let (mut deck, recorder) = open(FADERPORT8_ID);
        deck.flush().unwrap();
        let mut sent = FP8_START;
        // Bypass (0x03, RGB) red.
        deck.set_face(35, Some(&lit([255, 0, 0]))).unwrap();
        // Track (0x28, mono) any colour.
        deck.set_face(40, Some(&lit([0, 0, 90]))).unwrap();
        // Touch keys and past the end: no light.
        deck.set_face(24, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(200, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[sent..],
            [
                vec![0x91, 0x03, 0x7f],
                vec![0x92, 0x03, 0x00],
                vec![0x93, 0x03, 0x00],
                vec![0x90, 0x03, 0x7f],
                vec![0x90, 0x28, 0x7f],
            ]
        );
        sent += 5;
        // Off keeps the colour; on again with the same colour sends no colour.
        deck.set_face(35, None).unwrap();
        deck.flush().unwrap();
        deck.set_face(35, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[sent..],
            [vec![0x90, 0x03, 0x00], vec![0x90, 0x03, 0x7f],]
        );
        sent += 2;
        // A new colour on a lit key.
        deck.set_face(35, Some(&lit([0, 200, 0]))).unwrap();
        // A mono key changing colour stays on and sends nothing.
        deck.set_face(40, Some(&lit([255, 255, 0]))).unwrap();
        deck.set_brightness(0).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[sent..],
            [
                vec![0x91, 0x03, 0x00],
                vec![0x92, 0x03, 0x64],
                vec![0x93, 0x03, 0x00],
                vec![0x90, 0x03, 0x7f],
            ]
        );
    }

    #[test]
    fn writes_are_paced_from_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open(FADERPORT8_ID);
        deck.pace = Duration::from_secs(60);
        deck.clock = Instant::now();
        let begun = Instant::now();
        deck.poll(0).unwrap();
        deck.flush().unwrap();
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert!(recorder.writes().is_empty());
        assert_eq!(deck.outbox.len(), FP8_START);
        // A newer light replaces one still waiting.
        deck.set_face(32, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        deck.set_face(32, None).unwrap();
        deck.flush().unwrap();
        let arm: Vec<_> = deck
            .outbox
            .iter()
            .filter(|w| w[..2] == [0x90, 0x00])
            .collect();
        assert_eq!(arm, [&vec![0x90, 0x00, 0x00]]);
        deck.pace = Duration::ZERO;
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), FP8_START);
        assert!(deck.outbox.is_empty());
    }

    /// The Companion's key help lists these (ui/src/key-editor.tsx).
    #[test]
    fn the_rgb_keys_are_the_ones_the_companion_names() {
        let rgb = |id: &str| -> Vec<usize> {
            let kind = Kind::of(by_id(id).unwrap()).unwrap();
            (0..kind.notes().len())
                .filter(|key| kind.is_rgb(kind.notes()[*key]))
                .collect()
        };
        assert_eq!(rgb(FADERPORT2_ID), [1, 2, 6, 8, 11, 12, 17, 18, 19]);
        let mut fp8: Vec<usize> = (0..8).collect();
        fp8.extend([35, 36, 37]);
        fp8.extend(56..=66);
        assert_eq!(rgb(FADERPORT8_ID), fp8);
        let mut fp16: Vec<usize> = (0..16).collect();
        fp16.extend([67, 68, 69]);
        fp16.extend(88..=98);
        assert_eq!(rgb(FADERPORT16_ID), fp16);
    }

    #[test]
    fn gaps_follow_the_reference() {
        assert_eq!(gap(&[0x91, 0x03, 0x7f]), 0);
        assert_eq!(gap(&[0x92, 0x03, 0x7f]), 0);
        assert_eq!(gap(&[0x93, 0x03, 0x7f]), 15);
        assert_eq!(gap(&[0x90, 0x03, 0x7f]), 12);
        assert_eq!(gap(&[0xd0, 0x00]), 8);
        assert_eq!(gap(&sysex(0x02, &[0x13, 0x00, 0x10])), 36);
    }

    #[test]
    fn buttons_and_touches_are_keys() {
        let (mut deck, recorder) = open(FADERPORT8_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 0x18, 0x7f],
                &[0x90, 0x18, 0x00],
                &[0x90, 0x17, 0x41],
                // At 0x40 a Note On releases; any Note Off releases.
                &[0x90, 0x17, 0x40],
                &[0x90, 0x08, 0x7f],
                &[0x80, 0x08, 0x7f],
                &[0x90, 0x6f, 0x7f],
                &[0x91, 0x6f, 0x00],
                &[0x90, 0x00, 0x7f],
                &[0x90, 0x46, 0x7f],
                &[0x90, 0x66, 0x7f],
                &[0x90, 0x53, 0x7f],
                // Notes this model does not have, SysEx, short and stray input.
                &[0x90, 0x07, 0x7f],
                &[0x90, 0x78, 0x7f],
                &[0xf0, 0x00, 0x01, 0x06, 0x02, 0xf7],
                &[0x90],
                &[0x12, 0x34],
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
                key(16, false),
                key(31, true),
                key(31, false),
                key(32, true),
                key(39, true),
                key(73, true),
                key(47, true),
            ]
        );
    }

    #[test]
    fn the_faderport_16s_upper_strips_use_their_own_notes() {
        let (mut deck, recorder) = open(FADERPORT16_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 0x07, 0x7f],
                &[0x90, 0x27, 0x7f],
                &[0x90, 0x7f, 0x7f],
                &[0x90, 0x58, 0x7f],
                &[0x90, 0x59, 0x7f],
                &[0x90, 0x77, 0x7f],
                &[0x90, 0x53, 0x7f],
                &[0x90, 0x20, 0x7f],
                &[0x90, 0x56, 0x7f],
                &[0xef, 0x00, 0x00],
                &[0xef, 0x00, 0x10],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(8, true),
                key(15, true),
                key(31, true),
                key(43, true),
                key(46, true),
                key(63, true),
                key(79, true),
                key(78, true),
                key(99, true),
                turn(17, 8),
            ]
        );
    }

    #[test]
    fn the_faderport_16_starts_with_its_device_byte_and_keeps_alive() {
        let (mut deck, recorder) = open(FADERPORT16_ID);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 16 * 9 + 16 * 3 + 42 - 3);
        assert_eq!(
            writes[0],
            vec![0xf0, 0x00, 0x01, 0x06, 0x16, 0x13, 0x00, 0x10, 0xf7]
        );
        // Strip 9 (index 8) uses the upper bar, meter and peak.
        assert_eq!(
            writes[8 * 9 + 5..8 * 9 + 9],
            [
                vec![0xb0, 0x40, 0x00],
                vec![0xb0, 0x48, 0x04],
                vec![0xc0, 0x00],
                vec![0xc8, 0x00],
            ]
        );
        let before = writes.len();
        deck.keep_alive = Duration::ZERO;
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes()[before..], [vec![0xa0, 0x00, 0x00]]);
        // The FaderPort 8 never sends it.
        let (mut deck, recorder) = open(FADERPORT8_ID);
        deck.keep_alive = Duration::ZERO;
        deck.poll(0).unwrap();
        deck.poll(0).unwrap();
        assert!(recorder.writes().iter().all(|w| w[0] != 0xa0));
    }

    #[test]
    fn encoders_and_faders_are_dials() {
        let (mut deck, recorder) = open(FADERPORT8_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 0x10, 0x01],
                &[0xb0, 0x10, 0x43],
                &[0xb5, 0x3c, 0x02],
                &[0xb0, 0x3c, 0x41],
                // No steps, and other controllers.
                &[0xb0, 0x10, 0x40],
                &[0xb0, 0x11, 0x01],
                // Fader 1: seeded, then 16 tenth-bit units a step.
                &[0xe0, 0x00, 0x40],
                &[0xe0, 0x00, 0x42],
                &[0xe0, 0x70, 0x7f],
                &[0xe0, 0x00, 0x00],
                // Fader 8; a ninth does not exist.
                &[0xe7, 0x00, 0x00],
                &[0xe7, 0x00, 0x08],
                &[0xe8, 0x00, 0x00],
                &[0xe8, 0x00, 0x10],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, 1),
                turn(0, -3),
                turn(1, 2),
                turn(1, -1),
                turn(2, 1),
                turn(2, 30),
                turn(2, -63),
                turn(9, 4),
            ]
        );
    }

    #[test]
    fn the_faderport_2_has_one_strip_and_no_display() {
        let (mut deck, recorder) = open(FADERPORT2_ID);
        deck.set_face(1, Some(&lit([0, 0, 255]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 4 + 25 + 3);
        assert_eq!(
            writes[..4],
            [
                vec![0xb0, 0x30, 0x00],
                vec![0xb0, 0x38, 0x04],
                vec![0xd0, 0x00],
                vec![0xd8, 0x00],
            ]
        );
        assert!(writes.iter().all(|w| w[0] != 0xf0));
        assert_eq!(
            writes[5..9],
            [
                vec![0x91, 0x03, 0x00],
                vec![0x92, 0x03, 0x00],
                vec![0x93, 0x03, 0x7f],
                vec![0x90, 0x03, 0x7f],
            ]
        );
        queue(
            &recorder,
            &[
                &[0x90, 0x18, 0x7f],
                &[0x90, 0x20, 0x7f],
                &[0x90, 0x20, 0x00],
                // Navigator push is the encoder push too.
                &[0x90, 0x53, 0x7f],
                &[0x90, 0x68, 0x7f],
                &[0x90, 0x66, 0x7f],
                // Notes the FaderPort 2 does not have.
                &[0x90, 0x19, 0x7f],
                &[0x90, 0x46, 0x7f],
                &[0xb0, 0x10, 0x01],
                &[0xb0, 0x3c, 0x41],
                &[0xe0, 0x00, 0x00],
                &[0xe0, 0x00, 0x08],
                &[0xe1, 0x00, 0x40],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(6, true),
                key(7, true),
                key(7, false),
                key(7, true),
                key(27, true),
                key(26, true),
                turn(0, 1),
                turn(0, -1),
                turn(1, 4),
            ]
        );
    }

    #[test]
    fn letting_go_turns_every_light_off() {
        let (mut deck, recorder) = open(FADERPORT8_ID);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes.len(), 63);
        assert!(writes
            .iter()
            .all(|w| w.len() == 3 && w[0] == 0x90 && w[2] == 0));
        assert!(writes.contains(&vec![0x90, 0x06, 0x00]));
        assert!(writes.contains(&vec![0x90, 0x46, 0x00]));
        // Never opened: nothing sent.
        let (deck, recorder) = open(FADERPORT8_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn each_model_matches_only_its_own_port() {
        let cases = [
            ("PreSonus FP2", false, Some(FADERPORT2_ID)),
            ("PreSonus FP2", true, Some(FADERPORT2_ID)),
            (
                "PreSonus FP2:PreSonus FP2 MIDI 1 24:0",
                false,
                Some(FADERPORT2_ID),
            ),
            ("FP2", false, Some(FADERPORT2_ID)),
            ("PreSonus FP8 Port 1", false, Some(FADERPORT8_ID)),
            (
                "PreSonus FP8:PreSonus FP8 Port 1 24:0",
                false,
                Some(FADERPORT8_ID),
            ),
            ("PRESONUS FP8", true, Some(FADERPORT8_ID)),
            ("FaderPort 8", false, Some(FADERPORT8_ID)),
            ("PreSonus FP16 Port 1", false, Some(FADERPORT16_ID)),
            ("FaderPort16", false, Some(FADERPORT16_ID)),
            // Port 2, a second Windows interface, others.
            ("PreSonus FP8 Port 2", false, None),
            ("PreSonus FP16:PreSonus FP16 Port 2 24:1", false, None),
            ("MIDIIN2 (PreSonus FP8)", true, None),
            ("Some FP8 cable", false, None),
            ("ATOM", false, Some(super::super::presonus_atom::ATOM_ID)),
            (
                "FaderPort",
                false,
                Some(super::super::faderport::FADERPORT_ID),
            ),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_views_name_every_key_and_light_the_buttons() {
        for (id, keys, dials, rows, cols, lit) in [
            (FADERPORT2_ID, 28, 2, 4, 7, 25),
            (FADERPORT8_ID, 74, 10, 10, 8, 63),
            (FADERPORT16_ID, 106, 18, 7, 16, 87),
        ] {
            let model = by_id(id).unwrap();
            assert_eq!(model.dial_lights, DialLights::None);
            let view = ModelView::from(model);
            assert_eq!(
                view.key_names.as_ref().unwrap().len(),
                usize::from(keys),
                "{id}"
            );
            assert_eq!(view.lit_keys.as_ref().unwrap().len(), lit, "{id}");
            assert_eq!((view.keys, view.extra_keys, view.dials), (keys, 0, dials));
            assert_eq!((view.rows, view.cols), (rows, cols));
            assert!(usize::from(rows) * usize::from(cols) >= usize::from(keys));
            assert_eq!(view.key_light_colors, "rgb");
            assert!(view.key_lights && !view.brightness);
        }
        let names = key_names(by_id(FADERPORT16_ID).unwrap()).unwrap();
        assert_eq!(
            (names[0], names[15], names[16], names[63], names[64], names[105]),
            (
                "Select 1",
                "Select 16",
                "Mute 1",
                "Touch 16",
                "Arm",
                "Footswitch"
            )
        );
        let names = key_names(by_id(FADERPORT2_ID).unwrap()).unwrap();
        assert_eq!((names[7], names[27]), ("Encoder Push", "Fader Touch"));
    }
}
