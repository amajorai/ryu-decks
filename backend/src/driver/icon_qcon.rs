//! iCON Pro Audio's QCon Pro X and its QCon Pro XS1 extender, over
//! class-compliant USB MIDI ([`super::midi`]), in their Mackie Control
//! (MCU) mode.
//!
//! Each unit is its own USB MIDI device with one port each way, so each is
//! its own Ryu deck. The names carry the firmware version: `iCON QCON Pro X
//! V1.15` and `iCON QCON Pro XS1 V1.08`. [`port_matches`] keys on `QCON PRO
//! X` not followed by `S` for the Pro X and on `QCON PRO XS` for the XS1, in
//! any case, so neither is taken for the other and a QCon Pro G2 never
//! matches either. On Windows only the first interface matches.
//!
//! The unit must be in its Mackie Control mode (its default; HUI and the
//! other DAW modes send other notes). Anything Ryu does not expect is
//! logged once as a hint. There is no handshake and no keep-alive.
//!
//! In, on MIDI channel 1: every button and fader touch is a Note On, pressed
//! at a velocity over 64 and released at 64 or under (a Note Off releases
//! too). On the Pro X a key's index is its note, `0x00`-`0x70`, eight to a
//! row: Rec, Solo, Mute, Select and V-Pot push by strip, the 64 global
//! buttons in Mackie note order (`0x28` Track to `0x67` User B), the fader
//! touches and the master fader touch. Where each global button sits on the
//! deck and what iCON prints on it are not in any reference read, so the
//! keys carry the Mackie function names. On the XS1 keys 1-40 are notes
//! `0x00`-`0x27` and keys 41-48 the fader touches `0x68`-`0x6F`. The V-Pots
//! are `CC 0x10+s` (dials 1-8) and the Pro X's jog wheel `CC 0x3C` (dial 9),
//! sign-magnitude relative: bit 6 set turns anticlockwise, bits 0-5 count
//! the steps (0 counts as 1). The motor faders are pitch bend on channels
//! 1-8 (and the Pro X's master on 9): the first value of each only seeds
//! it, then every 256 of its 14 bits (64 steps end to end) turn its dial a
//! step (dials 10-18 on the Pro X, 9-16 on the XS1). A Mackie SysEx from the
//! deck (`F0 00 00 66 <id> ...`) tells Ryu the id to put in its LCD headers.
//!
//! Out, on channel 1: a button's LED is `90 <note> 7F` on and `90 <note> 00`
//! off (the QCon cannot blink). The main 2 x 56 LCD (`F0 00 00 66 <id> 12
//! <00|38> <56 chars> F7`, id `14` on the Pro X, `15` on the XS1 until the
//! deck says otherwise) shows each strip's Select label over its V-Pot push
//! label, six characters each. Opening blanks it and, on the Pro X, the
//! second LCD (`F0 00 00 67 15 13`), the timecode digits (`B0 40-49 20`) and
//! the master meters (`D1`); it turns the V-Pot rings (`B0 30-37 00`), the
//! strip meters (`D0`) and every LED off. Every write waits in a queue
//! drained by poll and flush, a millisecond apart and five after a SysEx (a
//! guess: the deck's buffer size is not known; no wait in tests), and Ryu
//! never sleeps in the poll loop. Letting go blanks the displays and turns
//! the LEDs and rings off at once. Ryu never sends pitch bend (the faders
//! have motors), ring positions, meter levels or the iCON colour SysEx (the
//! LCD is monochrome). There is no brightness command.
//!
//! The QCon Pro G2 and its QCon EX G2 extender ([`Protocol::IconQconG2Midi`])
//! speak the same Mackie Control, with these differences. Bitwig names their
//! ports `iCON QCON Pro G2 V1.13` and `iCON QCON EX<n> G2 V1.13`; Ardour
//! calls the extender a `Pro G2 XS`, so `QCON PRO G2 XS` is taken for it too,
//! and the Pro G2 matcher refuses any name with `XS` or ` EX`. The Pro G2's
//! keys 1-102 are notes `0x00`-`0x65` (Rec to Scrub) and keys 103-111 the
//! fader touches `0x68`-`0x70`; the rear user switches `0x66`/`0x67` are not
//! keys. The EX G2's keys are the XS1's. Their fader dials step every 128
//! units. Opening sends Ardour's wake-up (`F0 00 00 66 <id> 00 F7`), the LCD
//! lines are 55 characters (as Ardour and Bitwig send), there is no second
//! LCD or master meter, and the V-Pot rings (dials 1-8) take a light: `B0
//! 30+s 06` (the centre dot; the QCon shows only dot mode) or `00`. The
//! references disagree on the EX G2's SysEx id (`14` in Ardour, `15` in
//! Bitwig), so until the deck sends a Mackie SysEx of its own Ryu writes
//! each LCD line under both. The printed G2 legends differ from the Mackie
//! names on many globals, and the references disagree on them.
//!
//! Facts from DrivenByMoss's Mackie Control support (Java, LGPL-3.0),
//! Ardour's Mackie Control surface and its QCon device files (C++,
//! GPL-2.0-or-later) and Bitwig's controller extensions' QCon Pro G2
//! definitions (Java, MIT); no code was copied. No USB id is given by any of
//! them. Untested on hardware.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const PRO_X_ID: &str = "icon-qcon-pro-x";
pub const PRO_XS1_ID: &str = "icon-qcon-pro-xs1";
pub const PRO_G2_ID: &str = "icon-qcon-pro-g2";
pub const EX_G2_ID: &str = "icon-qcon-ex-g2";

pub const PRO_X_ROWS: u8 = 15;
pub const PRO_X_COLS: u8 = 8;
/// Notes `0x00` to `0x70`, each its own key.
pub const PRO_X_KEYS: u8 = 113;
/// Eight V-Pots, the jog wheel, eight faders and the master fader.
pub const PRO_X_DIALS: u8 = 18;
pub const XS1_ROWS: u8 = 6;
pub const XS1_COLS: u8 = 8;
/// Rec, Solo, Mute, Select, V-Pot push and fader touch for eight strips.
pub const XS1_KEYS: u8 = 48;
/// Eight V-Pots and eight faders.
pub const XS1_DIALS: u8 = 16;
pub const PRO_G2_ROWS: u8 = 14;
pub const PRO_G2_COLS: u8 = 8;
/// Notes `0x00` to `0x65`, then the fader touches `0x68` to `0x70`.
pub const PRO_G2_KEYS: u8 = 111;
/// Eight V-Pots, the jog wheel, eight faders and the master fader.
pub const PRO_G2_DIALS: u8 = 18;
pub const EX_G2_ROWS: u8 = 6;
pub const EX_G2_COLS: u8 = 8;
/// Rec, Solo, Mute, Select, V-Pot push and fader touch for eight strips.
pub const EX_G2_KEYS: u8 = 48;
/// Eight V-Pots and eight faders.
pub const EX_G2_DIALS: u8 = 16;
/// The G2s' V-Pots have LED rings (dials 1-8); the jog wheel and faders
/// have none.
pub const G2_RINGS: u8 = 8;

const STRIPS: u8 = 8;
/// The XS1's last key with a note under its index, and the first of its
/// fader touches.
const XS1_LAST_STRIP_NOTE: u8 = 0x27;
const XS1_FIRST_TOUCH_KEY: u8 = 40;
const FADER_TOUCH: u8 = 0x68;
const MASTER_TOUCH: u8 = 0x70;
/// The Pro G2's last global note (Scrub) and its first fader touch key.
const G2_LAST_GLOBAL: u8 = 0x65;
/// The Pro G2's rear User A and User B switches, which are not keys.
const USER_SWITCHES: std::ops::RangeInclusive<u8> = 0x66..=0x67;
const G2_FIRST_TOUCH_KEY: u8 = 102;
/// The strip Select notes head the LCD's first line, the V-Pot pushes its
/// second.
const SELECT: u8 = 0x18;
const VPOT_PUSH: u8 = 0x20;
const LAST_STRIP_LIGHT: u8 = 0x1f;
/// The Pro X's global buttons with a light, Track to Scrub.
const GLOBAL_LIGHTS: std::ops::RangeInclusive<u8> = 0x28..=0x65;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const PITCH_BEND: u8 = 0xe0;
const CHANNEL_PRESSURE: u8 = 0xd0;
const STRIP_METER: u8 = 0xd0;
const MASTER_METER: u8 = 0xd1;
const VPOT_CC: u8 = 0x10;
const JOG_CC: u8 = 0x3c;
/// The Mackie expression pedal input, absolute: not read.
const EXTERNAL_CC: u8 = 0x2e;
const RING_CC: u8 = 0x30;
const TIMECODE_CC: u8 = 0x40;
const TIMECODE_DIGITS: u8 = 10;
const BLANK: u8 = 0x20;
/// Velocities over this press.
const PRESS_OVER: u8 = 64;
const SIGN: u8 = 0x40;
const STEPS: u8 = 0x3f;
/// The most steps one V-Pot or jog message turns its dial.
const MAX_STEPS: i8 = 15;

const LED_OFF: u8 = 0;
const LED_ON: u8 = 0x7f;

const SYSEX_START: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
/// Mackie's manufacturer bytes.
const MACKIE: [u8; 3] = [0x00, 0x00, 0x66];
const LCD_COMMAND: u8 = 0x12;
/// Ardour's wake-up, answered by nothing on a QCon.
const HELLO_COMMAND: u8 = 0x00;
const PRO_X_DEVICE: u8 = 0x14;
const EXTENDER_DEVICE: u8 = 0x15;
/// A ring's centre dot, and a ring off.
const RING_DOT: u8 = 0x06;
const RING_OFF: u8 = 0x00;
/// The QCon's second LCD, under its own manufacturer bytes.
const SECOND_LCD: [u8; 5] = [0x00, 0x00, 0x67, 0x15, 0x13];
/// Characters on each LCD line, and the offset of the second line. The
/// G2s are sent 55 of them, as Ardour and Bitwig do.
const LINE: usize = 56;
const G2_LINE: usize = 55;
const LINE_TWO: u8 = 0x38;
/// Characters per strip cell, then a space.
const CELL: usize = 7;
const CELL_TEXT: usize = 6;

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Fader units (of 16384) per dial step: 64 steps from end to end on the
/// Pro X and XS1, 128 (one per change of the upper seven bits) on the G2s.
const FADER_STEP: i32 = 256;
const G2_FADER_STEP: i32 = 128;
const MAX_FADERS: usize = 9;

/// The gap after a short message and after a SysEx (none in tests). The
/// deck's buffer is not documented, so these are a guess.
const SHORT_GAP: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(1)
};
const SYSEX_GAP: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(5)
};
/// How far behind the clock may fall, so a burst after a quiet spell is
/// still paced.
const MAX_CREDIT: Duration = Duration::from_millis(10);
const READ_LEN: usize = 256;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// The Mackie function of each Pro X note, which is its key (the iCON
/// P1-Nano's keys too, [`super::icon_p1`]).
pub(super) const NAMES: [&str; PRO_X_KEYS as usize] = [
    "Rec 1",
    "Rec 2",
    "Rec 3",
    "Rec 4",
    "Rec 5",
    "Rec 6",
    "Rec 7",
    "Rec 8",
    "Solo 1",
    "Solo 2",
    "Solo 3",
    "Solo 4",
    "Solo 5",
    "Solo 6",
    "Solo 7",
    "Solo 8",
    "Mute 1",
    "Mute 2",
    "Mute 3",
    "Mute 4",
    "Mute 5",
    "Mute 6",
    "Mute 7",
    "Mute 8",
    "Select 1",
    "Select 2",
    "Select 3",
    "Select 4",
    "Select 5",
    "Select 6",
    "Select 7",
    "Select 8",
    "V-Pot 1",
    "V-Pot 2",
    "V-Pot 3",
    "V-Pot 4",
    "V-Pot 5",
    "V-Pot 6",
    "V-Pot 7",
    "V-Pot 8",
    // 0x28
    "Track",
    "Send",
    "Pan",
    "Plug-in",
    "EQ",
    "Instrument",
    "Bank <",
    "Bank >",
    // 0x30
    "Channel <",
    "Channel >",
    "Flip",
    "Global View",
    "Name/Value",
    "SMPTE/Beats",
    "F1",
    "F2",
    // 0x38
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "MIDI Tracks",
    "Inputs",
    // 0x40
    "Audio Tracks",
    "Audio Instruments",
    "Aux",
    "Busses",
    "Outputs",
    "User",
    "Shift",
    "Option",
    // 0x48
    "Control",
    "Alt/Cmd",
    "Read",
    "Write",
    "Trim",
    "Touch",
    "Latch",
    "Group",
    // 0x50
    "Save",
    "Undo",
    "Cancel",
    "Enter",
    "Marker",
    "Nudge",
    "Cycle",
    "Drop",
    // 0x58
    "Replace",
    "Click",
    "Clear Solo",
    "Rewind",
    "Forward",
    "Stop",
    "Play",
    "Record",
    // 0x60
    "Up",
    "Down",
    "Left",
    "Right",
    "Zoom",
    "Scrub",
    "User A",
    "User B",
    // 0x68
    "Fader Touch 1",
    "Fader Touch 2",
    "Fader Touch 3",
    "Fader Touch 4",
    "Fader Touch 5",
    "Fader Touch 6",
    "Fader Touch 7",
    "Fader Touch 8",
    // 0x70
    "Master Touch",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    ProX,
    Xs1,
    ProG2,
    ExG2,
}

impl Kind {
    fn of(model: &DeviceModel) -> Option<Self> {
        match (model.protocol, model.id) {
            (Protocol::IconQconProMidi, PRO_X_ID) => Some(Self::ProX),
            (Protocol::IconQconProMidi, PRO_XS1_ID) => Some(Self::Xs1),
            (Protocol::IconQconG2Midi, PRO_G2_ID) => Some(Self::ProG2),
            (Protocol::IconQconG2Midi, EX_G2_ID) => Some(Self::ExG2),
            _ => None,
        }
    }

    const fn keys(self) -> u8 {
        match self {
            Self::ProX => PRO_X_KEYS,
            Self::Xs1 => XS1_KEYS,
            Self::ProG2 => PRO_G2_KEYS,
            Self::ExG2 => EX_G2_KEYS,
        }
    }

    /// A main unit, with global buttons, a jog wheel, a master fader and
    /// timecode digits; not an extender.
    const fn is_main(self) -> bool {
        matches!(self, Self::ProX | Self::ProG2)
    }

    const fn is_g2(self) -> bool {
        matches!(self, Self::ProG2 | Self::ExG2)
    }

    /// The key a note presses.
    fn key(self, note: u8) -> Option<u8> {
        match self {
            Self::ProX => (note <= MASTER_TOUCH).then_some(note),
            Self::ProG2 => match note {
                0..=G2_LAST_GLOBAL => Some(note),
                FADER_TOUCH..=MASTER_TOUCH => Some(note - FADER_TOUCH + G2_FIRST_TOUCH_KEY),
                _ => None,
            },
            Self::Xs1 | Self::ExG2 => match note {
                0..=XS1_LAST_STRIP_NOTE => Some(note),
                _ if (FADER_TOUCH..FADER_TOUCH + STRIPS).contains(&note) => {
                    Some(note - FADER_TOUCH + XS1_FIRST_TOUCH_KEY)
                }
                _ => None,
            },
        }
    }

    /// The note a key sends and lights by.
    fn note(self, key: u8) -> Option<u8> {
        match self {
            Self::ProX => (key < PRO_X_KEYS).then_some(key),
            Self::ProG2 => match key {
                0..=G2_LAST_GLOBAL => Some(key),
                G2_FIRST_TOUCH_KEY..PRO_G2_KEYS => Some(key - G2_FIRST_TOUCH_KEY + FADER_TOUCH),
                _ => None,
            },
            Self::Xs1 | Self::ExG2 => match key {
                0..=XS1_LAST_STRIP_NOTE => Some(key),
                XS1_FIRST_TOUCH_KEY..XS1_KEYS => Some(key - XS1_FIRST_TOUCH_KEY + FADER_TOUCH),
                _ => None,
            },
        }
    }

    /// Rec, Solo, Mute and Select, and on a main unit the global buttons
    /// (on the Pro G2 keys and notes are the same up to Scrub).
    fn has_light(self, key: u8) -> bool {
        key <= LAST_STRIP_LIGHT || (self.is_main() && GLOBAL_LIGHTS.contains(&key))
    }

    fn lit_keys(self) -> Vec<u8> {
        (0..self.keys())
            .filter(|key| self.has_light(*key))
            .collect()
    }

    fn names(self) -> Vec<&'static str> {
        (0..self.keys())
            .filter_map(|key| self.note(key))
            .filter_map(|note| NAMES.get(usize::from(note)).copied())
            .collect()
    }

    const fn first_fader_dial(self) -> u8 {
        if self.is_main() {
            9
        } else {
            8
        }
    }

    const fn faders(self) -> u8 {
        if self.is_main() {
            9
        } else {
            8
        }
    }

    const fn fader_step(self) -> i32 {
        if self.is_g2() {
            G2_FADER_STEP
        } else {
            FADER_STEP
        }
    }

    const fn line_len(self) -> usize {
        if self.is_g2() {
            G2_LINE
        } else {
            LINE
        }
    }

    /// The id in the SysEx headers until the deck sends its own: the
    /// references disagree on the EX G2's (`14` in Ardour, `15` in Bitwig),
    /// so it is sent both.
    const fn devices(self) -> &'static [u8] {
        match self {
            Self::ProX | Self::ProG2 => &[PRO_X_DEVICE],
            Self::Xs1 => &[EXTENDER_DEVICE],
            Self::ExG2 => &[PRO_X_DEVICE, EXTENDER_DEVICE],
        }
    }
}

/// Whether a normalised port name is this model's: on the uppercased name,
/// `QCON PRO X` not followed by `S` for the Pro X, `QCON PRO XS` for the
/// XS1, `QCON PRO G2` without `XS` or ` EX` for the Pro G2, `QCON EX` with
/// `G2` or `QCON PRO G2 XS` for the EX G2, and on Windows the first
/// interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    let Some(kind) = Kind::of(model) else {
        return false;
    };
    if name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.to_ascii_uppercase();
    match kind {
        Kind::Xs1 => base.contains("QCON PRO XS"),
        Kind::ProX => base
            .match_indices("QCON PRO X")
            .any(|(at, found)| base.as_bytes().get(at + found.len()) != Some(&b'S')),
        Kind::ProG2 => {
            base.contains("QCON PRO G2") && !base.contains("XS") && !base.contains(" EX")
        }
        Kind::ExG2 => {
            (base.contains("QCON EX") && base.contains("G2")) || base.contains("QCON PRO G2 XS")
        }
    }
}

/// The Mackie function of each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    Kind::of(model).map(Kind::names)
}

/// The keys with a light: Rec, Solo, Mute, Select and a main unit's global
/// buttons from Track to Scrub.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    Kind::of(model).map(Kind::lit_keys)
}

/// On for any colour that is not dark.
fn is_lit(color: Option<[u8; 3]>) -> bool {
    color.is_some_and(|color| color.iter().any(|channel| *channel >= DARK))
}

/// A sign-magnitude relative turn: bit 6 anticlockwise, bits 0-5 the
/// steps, none counting as one, at most [`MAX_STEPS`].
pub(super) fn relative(value: u8) -> i8 {
    let steps = i8::try_from((value & STEPS).max(1))
        .unwrap_or(MAX_STEPS)
        .min(MAX_STEPS);
    if value & SIGN == 0 {
        steps
    } else {
        -steps
    }
}

/// A label as LCD characters: printable ASCII kept, other whitespace a
/// space, anything else `_`.
pub(super) fn lcd_char(ch: char) -> u8 {
    match u8::try_from(ch) {
        Ok(byte @ 0x20..=0x7e) => byte,
        _ if ch.is_whitespace() => BLANK,
        _ => b'_',
    }
}

/// Writes a label into its strip's cell, padded with spaces.
fn put_cell(line: &mut [u8; LINE], strip: usize, label: &str) {
    let Some(cell) = line.get_mut(strip * CELL..strip * CELL + CELL_TEXT) else {
        return;
    };
    cell.fill(BLANK);
    for (slot, ch) in cell.iter_mut().zip(label.chars()) {
        *slot = lcd_char(ch);
    }
}

fn lcd_message(device: u8, line: u8, text: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(LINE + 8);
    message.push(SYSEX_START);
    message.extend_from_slice(&MACKIE);
    message.extend_from_slice(&[device, LCD_COMMAND, line * LINE_TWO]);
    message.extend_from_slice(text);
    message.push(SYSEX_END);
    message
}

fn second_lcd_blank(line: u8) -> Vec<u8> {
    let mut message = Vec::with_capacity(LINE + 8);
    message.push(SYSEX_START);
    message.extend_from_slice(&SECOND_LCD);
    message.push(line * LINE_TWO);
    message.extend_from_slice(&[BLANK; LINE]);
    message.push(SYSEX_END);
    message
}

/// A motor fader read as a dial: its first value seeds it, then every
/// [`FADER_STEP`] units moved turn the dial one step.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Fader {
    last: Option<u16>,
    rest: i32,
}

impl Fader {
    pub(super) fn moved(&mut self, value: u16, step: i32) -> i8 {
        let Some(old) = self.last.replace(value) else {
            return 0;
        };
        let moved = self.rest + i32::from(value) - i32::from(old);
        let steps = moved / step;
        self.rest = moved - steps * step;
        i8::try_from(steps.clamp(-127, 127)).unwrap_or(0)
    }
}

/// What a queued write sets, so a newer write for the same thing replaces
/// one still waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Led(u8),
    /// A line and the header id it is sent under.
    Line(u8, u8),
    Ring(u8),
    Other,
}

#[derive(Debug)]
struct Out {
    slot: Slot,
    bytes: Vec<u8>,
}

pub struct IconQcon {
    kind: Kind,
    transport: Box<dyn Transport>,
    /// The reset has been queued.
    started: bool,
    /// Writes waiting their turn, drained by poll and flush; dropped, not
    /// sent, when Ryu lets go.
    outbox: VecDeque<Out>,
    /// When the next write may go out.
    ready_at: Instant,
    short_gap: Duration,
    sysex_gap: Duration,
    /// Each key's LED as staged, then as last queued.
    leds: Vec<bool>,
    queued: Vec<bool>,
    held: Vec<bool>,
    /// The main LCD's two lines as staged, then as last queued.
    lines: [[u8; LINE]; 2],
    queued_lines: [Option<[u8; LINE]>; 2],
    /// The ids in the main LCD's SysEx header: the deck's own once it has
    /// sent one, else [`Kind::devices`].
    device: Option<u8>,
    /// Each G2 V-Pot ring as staged, then as last queued.
    rings: [bool; G2_RINGS as usize],
    queued_rings: [bool; G2_RINGS as usize],
    faders: [Fader; MAX_FADERS],
    /// The unexpected-input hint has been logged.
    warned: bool,
    parser: Parser,
}

impl IconQcon {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model).unwrap_or(Kind::Xs1);
        let keys = usize::from(kind.keys());
        Self {
            kind,
            transport,
            started: false,
            outbox: VecDeque::new(),
            ready_at: Instant::now(),
            short_gap: SHORT_GAP,
            sysex_gap: SYSEX_GAP,
            leds: vec![false; keys],
            queued: vec![false; keys],
            held: vec![false; keys],
            lines: [[BLANK; LINE]; 2],
            queued_lines: [None; 2],
            device: None,
            rings: [false; G2_RINGS as usize],
            queued_rings: [false; G2_RINGS as usize],
            faders: [Fader::default(); MAX_FADERS],
            warned: false,
            parser: Parser::default(),
        }
    }

    fn push(&mut self, slot: Slot, bytes: Vec<u8>) {
        if slot != Slot::Other {
            self.outbox.retain(|out| out.slot != slot);
        }
        self.outbox.push_back(Out { slot, bytes });
    }

    /// The SysEx header ids to send under.
    fn devices(&self) -> Vec<u8> {
        self.device
            .map_or_else(|| self.kind.devices().to_vec(), |device| vec![device])
    }

    /// One LCD line, once for each header id.
    fn line_messages(&self, line: u8, text: &[u8; LINE]) -> Vec<(Slot, Vec<u8>)> {
        let text = text.get(..self.kind.line_len()).unwrap_or(text.as_slice());
        self.devices()
            .into_iter()
            .map(|device| (Slot::Line(line, device), lcd_message(device, line, text)))
            .collect()
    }

    /// The messages that blank, zero and darken the whole deck.
    fn reset_messages(&self) -> Vec<(Slot, Vec<u8>)> {
        let mut messages = Vec::new();
        for line in 0..2 {
            messages.extend(self.line_messages(line, &[BLANK; LINE]));
        }
        if self.kind == Kind::ProX {
            for line in 0..2 {
                messages.push((Slot::Other, second_lcd_blank(line)));
            }
        }
        if self.kind.is_main() {
            for digit in 0..TIMECODE_DIGITS {
                messages.push((
                    Slot::Other,
                    vec![CONTROL_CHANGE, TIMECODE_CC + digit, BLANK],
                ));
            }
        }
        for strip in 0..STRIPS {
            messages.push((Slot::Other, vec![CONTROL_CHANGE, RING_CC + strip, 0]));
        }
        messages
    }

    fn led_message(&self, key: u8, on: bool) -> Option<Vec<u8>> {
        let note = self.kind.note(key)?;
        Some(vec![NOTE_ON, note, if on { LED_ON } else { LED_OFF }])
    }

    /// Blank displays, rings, meters and every LED off, then whatever is
    /// already staged.
    fn start(&mut self) {
        self.started = true;
        self.outbox.clear();
        if self.kind.is_g2() {
            for device in self.devices() {
                let hello = vec![
                    SYSEX_START,
                    MACKIE[0],
                    MACKIE[1],
                    MACKIE[2],
                    device,
                    HELLO_COMMAND,
                    SYSEX_END,
                ];
                self.push(Slot::Other, hello);
            }
        }
        for (slot, bytes) in self.reset_messages() {
            self.push(slot, bytes);
        }
        for strip in 0..STRIPS {
            self.push(Slot::Other, vec![STRIP_METER, strip << 4]);
        }
        if self.kind == Kind::ProX {
            self.push(Slot::Other, vec![MASTER_METER, 0x00]);
            self.push(Slot::Other, vec![MASTER_METER, 0x10]);
        }
        for key in self.kind.lit_keys() {
            if let Some(bytes) = self.led_message(key, false) {
                self.push(Slot::Led(key), bytes);
            }
        }
        self.queued.fill(false);
        self.queued_rings = [false; G2_RINGS as usize];
        self.queued_lines = [Some([BLANK; LINE]); 2];
        self.queue_changes();
    }

    /// Queues each LED and LCD line that differs from what was queued.
    fn queue_changes(&mut self) {
        for key in 0..self.kind.keys() {
            let index = usize::from(key);
            let on = self.leds.get(index).copied().unwrap_or(false);
            if !self.kind.has_light(key) || self.queued.get(index) == Some(&on) {
                continue;
            }
            if let Some(queued) = self.queued.get_mut(index) {
                *queued = on;
            }
            if let Some(bytes) = self.led_message(key, on) {
                self.push(Slot::Led(key), bytes);
            }
        }
        if self.kind.is_g2() {
            for strip in 0..G2_RINGS {
                let index = usize::from(strip);
                let on = self.rings[index];
                if self.queued_rings[index] == on {
                    continue;
                }
                self.queued_rings[index] = on;
                let value = if on { RING_DOT } else { RING_OFF };
                self.push(
                    Slot::Ring(strip),
                    vec![CONTROL_CHANGE, RING_CC + strip, value],
                );
            }
        }
        self.queue_lines();
    }

    fn queue_lines(&mut self) {
        for line in 0..2u8 {
            let text = self.lines[usize::from(line)];
            if self.queued_lines[usize::from(line)] == Some(text) {
                continue;
            }
            self.queued_lines[usize::from(line)] = Some(text);
            for (slot, bytes) in self.line_messages(line, &text) {
                self.push(slot, bytes);
            }
        }
    }

    /// Writes the queued messages whose gap has passed, never sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        let now = Instant::now();
        if let Some(floor) = now.checked_sub(MAX_CREDIT) {
            self.ready_at = self.ready_at.max(floor);
        }
        while let Some(out) = self.outbox.front() {
            if self.ready_at > now {
                break;
            }
            self.transport.write(&out.bytes)?;
            let gap = if out.bytes.first() == Some(&SYSEX_START) {
                self.sysex_gap
            } else {
                self.short_gap
            };
            self.outbox.pop_front();
            self.ready_at += gap;
        }
        Ok(())
    }

    fn hint(&mut self) {
        if !self.warned {
            self.warned = true;
            tracing::warn!(
                "the iCON QCon sent a message Ryu does not expect; set it to its Mackie Control \
                 (MCU) mode for Ryu to read it"
            );
        }
    }

    fn note(&mut self, note: u8, pressed: bool, events: &mut Vec<Input>) {
        let Some(key) = self.kind.key(note) else {
            // The Pro G2's rear user switches are deliberately not keys,
            // and a deck in MCU mode sends them: no mode hint.
            if !(self.kind == Kind::ProG2 && USER_SWITCHES.contains(&note)) {
                self.hint();
            }
            return;
        };
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

    fn control(&mut self, number: u8, value: u8, events: &mut Vec<Input>) {
        let index = match number {
            JOG_CC if self.kind.is_main() => STRIPS,
            _ if (VPOT_CC..VPOT_CC + STRIPS).contains(&number) => number - VPOT_CC,
            EXTERNAL_CC => return,
            _ => {
                self.hint();
                return;
            }
        };
        events.push(Input::DialTurn {
            index,
            delta: relative(value),
        });
    }

    fn fader(&mut self, channel: u8, lsb: u8, msb: u8, events: &mut Vec<Input>) {
        if channel >= self.kind.faders() {
            return;
        }
        let Some(fader) = self.faders.get_mut(usize::from(channel)) else {
            return;
        };
        let value = u16::from(lsb & 0x7f) | u16::from(msb & 0x7f) << 7;
        let delta = fader.moved(value, self.kind.fader_step());
        if delta != 0 {
            events.push(Input::DialTurn {
                index: self.kind.first_fader_dial() + channel,
                delta,
            });
        }
    }

    /// A Mackie SysEx from the deck carries the id its LCD answers to.
    fn sysex(&mut self, bytes: &[u8]) {
        if bytes.get(1..4) != Some(&MACKIE[..]) {
            return;
        }
        let Some(&device) = bytes.get(4) else {
            return;
        };
        if device >= 0x80 || self.device == Some(device) {
            return;
        }
        if self.device.is_none() && self.kind.devices() == [device] {
            self.device = Some(device);
            return;
        }
        self.device = Some(device);
        if self.started {
            self.queued_lines = [None; 2];
            self.queue_lines();
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let [status, data1, data2] = match message {
            Message::SysEx(bytes) => return self.sysex(bytes),
            Message::Channel(bytes) => *bytes,
        };
        match status {
            NOTE_ON => self.note(data1, data2 > PRESS_OVER, events),
            NOTE_OFF => self.note(data1, false, events),
            CONTROL_CHANGE => self.control(data1, data2, events),
            _ if status & 0xf0 == PITCH_BEND => self.fader(status & 0x0f, data1, data2, events),
            _ if status & 0xf0 == CHANNEL_PRESSURE => {}
            _ => self.hint(),
        }
    }
}

impl Driver for IconQcon {
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
        if key >= self.kind.keys() {
            return Ok(());
        }
        if self.kind.has_light(key) {
            if let Some(led) = self.leds.get_mut(usize::from(key)) {
                *led = is_lit(face.map(|face| face.color));
            }
        }
        // Both models put Select and the V-Pot pushes under their notes.
        let label = face.map_or("", |face| face.label.as_str());
        if (SELECT..SELECT + STRIPS).contains(&key) {
            put_cell(&mut self.lines[0], usize::from(key - SELECT), label);
        } else if (VPOT_PUSH..VPOT_PUSH + STRIPS).contains(&key) {
            put_cell(&mut self.lines[1], usize::from(key - VPOT_PUSH), label);
        }
        Ok(())
    }

    /// The reset the first time, then the LEDs and LCD lines that changed;
    /// whatever the pace allows goes out now, the rest from later polls.
    fn flush(&mut self) -> DriverResult<()> {
        if self.started {
            self.queue_changes();
        } else {
            self.start();
        }
        self.pump()
    }

    /// A G2's V-Pot ring: its centre dot on or off (the QCon shows only
    /// dot mode). The jog wheel and faders have no light, nor has the Pro
    /// X's or XS1's driver any ring.
    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if self.kind.is_g2() {
            if let Some(ring) = self.rings.get_mut(usize::from(dial)) {
                *ring = is_lit(color);
            }
        }
        Ok(())
    }

    /// The QCon has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for IconQcon {
    /// Turns every LED off and blanks the displays at once, without pacing;
    /// anything still queued is dropped.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        self.outbox.clear();
        for key in self.kind.lit_keys() {
            if let Some(bytes) = self.led_message(key, false) {
                let _ = self.transport.write(&bytes);
            }
        }
        for (_, bytes) in self.reset_messages() {
            let _ = self.transport.write(&bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    fn open(id: &str) -> (IconQcon, Recorder) {
        let recorder = Recorder::default();
        let deck = IconQcon::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    fn face(label: &str, color: [u8; 3]) -> Face {
        Face {
            label: label.to_owned(),
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

    fn lcd(device: u8, offset: u8, text: &str) -> Vec<u8> {
        let mut message = vec![0xf0, 0x00, 0x00, 0x66, device, 0x12, offset];
        let mut chars = text.as_bytes().to_vec();
        chars.resize(56, b' ');
        message.extend(chars);
        message.push(0xf7);
        message
    }

    #[test]
    fn opening_the_pro_x_blanks_and_darkens_everything_without_waiting() {
        assert!(SHORT_GAP.is_zero() && SYSEX_GAP.is_zero());
        let (mut deck, recorder) = open(PRO_X_ID);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 2 + 2 + 10 + 8 + 8 + 2 + 94);
        assert_eq!(writes[0], lcd(0x14, 0x00, ""));
        assert_eq!(writes[1], lcd(0x14, 0x38, ""));
        let mut second = vec![0xf0, 0x00, 0x00, 0x67, 0x15, 0x13, 0x00];
        second.extend([0x20; 56]);
        second.push(0xf7);
        assert_eq!(writes[2], second);
        second[6] = 0x38;
        assert_eq!(writes[3], second);
        for digit in 0..10u8 {
            assert_eq!(
                writes[4 + usize::from(digit)],
                vec![0xb0, 0x40 + digit, 0x20]
            );
        }
        for strip in 0..8u8 {
            assert_eq!(
                writes[14 + usize::from(strip)],
                vec![0xb0, 0x30 + strip, 0x00]
            );
            assert_eq!(writes[22 + usize::from(strip)], vec![0xd0, strip << 4]);
        }
        assert_eq!(writes[30..32], [vec![0xd1, 0x00], vec![0xd1, 0x10]]);
        let leds: Vec<u8> = writes[32..].iter().map(|write| write[1]).collect();
        let expected: Vec<u8> = (0x00..=0x1f).chain(0x28..=0x65).collect();
        assert_eq!(leds, expected);
        assert!(writes[32..]
            .iter()
            .all(|write| write[0] == 0x90 && write[2] == 0));
        // Never pitch bend.
        assert!(writes.iter().all(|write| write[0] & 0xf0 != 0xe0));
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 126);
    }

    #[test]
    fn opening_the_xs1_skips_what_it_does_not_have() {
        let (mut deck, recorder) = open(PRO_XS1_ID);
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 2 + 8 + 8 + 32);
        assert_eq!(writes[0], lcd(0x15, 0x00, ""));
        assert_eq!(writes[1], lcd(0x15, 0x38, ""));
        assert!(writes.iter().all(|write| {
            write.get(3) != Some(&0x67)
                && write[0] != 0xd1
                && !(write[0] == 0xb0 && write[1] >= 0x40)
        }));
        let leds: Vec<u8> = writes[18..].iter().map(|write| write[1]).collect();
        assert_eq!(leds, (0x00..=0x1f).collect::<Vec<u8>>());
    }

    #[test]
    fn writes_are_paced_from_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open(PRO_XS1_ID);
        deck.short_gap = Duration::from_secs(60);
        deck.sysex_gap = Duration::from_secs(60);
        deck.ready_at = Instant::now() + Duration::from_secs(60);
        let begun = Instant::now();
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert!(recorder.writes().is_empty());
        assert_eq!(deck.outbox.len(), 50);
        deck.short_gap = Duration::ZERO;
        deck.sysex_gap = Duration::ZERO;
        deck.ready_at = Instant::now();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 50);
        // Letting go is never paced, and drops whatever is queued.
        deck.set_face(0, Some(&face("", [255, 0, 0]))).unwrap();
        deck.short_gap = Duration::from_secs(60);
        deck.ready_at = Instant::now() + Duration::from_secs(60);
        deck.flush().unwrap();
        assert_eq!(deck.outbox.len(), 1);
        let begun = Instant::now();
        drop(deck);
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert_eq!(recorder.writes().len(), 50 + 32 + 2 + 8);
    }

    #[test]
    fn faces_light_buttons_and_label_the_lcd() {
        let (mut deck, recorder) = open(PRO_X_ID);
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_face(0x18, Some(&face("Vocals", [255, 0, 0])))
            .unwrap();
        deck.set_face(0x19, Some(&face("Bass guitar", [0, 0, 0])))
            .unwrap();
        deck.set_face(0x1f, Some(&face("Dr\u{fc}m\ns", [0, 0, 0])))
            .unwrap();
        deck.set_face(0x20, Some(&face("Pan", [255, 0, 0])))
            .unwrap();
        deck.set_face(0x5e, Some(&face("Play", [0, 200, 0])))
            .unwrap();
        // No light: V-Pot pushes, User A, fader touches; past the end.
        deck.set_face(0x66, Some(&face("", [255, 0, 0]))).unwrap();
        deck.set_face(0x68, Some(&face("", [255, 0, 0]))).unwrap();
        deck.set_face(113, Some(&face("", [255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let mut top = String::from("Vocals Bass g");
        top.push_str(&" ".repeat(49 - top.len()));
        top.push_str("Dr_m s");
        assert_eq!(
            recorder.writes()[before..],
            [
                vec![0x90, 0x18, 0x7f],
                vec![0x90, 0x5e, 0x7f],
                lcd(0x14, 0x00, &top),
                lcd(0x14, 0x38, "Pan"),
            ]
        );
        // Unchanged faces and brightness send nothing; a cleared face blanks.
        deck.set_face(0x5e, Some(&face("Play", [0, 0, 255])))
            .unwrap();
        deck.set_brightness(10).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), before + 4);
        deck.set_face(0x20, None).unwrap();
        deck.set_face(0x18, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[before + 4..],
            [
                vec![0x90, 0x18, 0x00],
                lcd(0x14, 0x00, &top.replacen("Vocals", "      ", 1)),
                lcd(0x14, 0x38, "")
            ]
        );
    }

    #[test]
    fn a_newer_write_replaces_one_still_queued() {
        let (mut deck, recorder) = open(PRO_XS1_ID);
        deck.short_gap = Duration::from_secs(60);
        deck.sysex_gap = Duration::from_secs(60);
        deck.ready_at = Instant::now() + Duration::from_secs(60);
        deck.set_face(0x18, Some(&face("One", [255, 0, 0])))
            .unwrap();
        deck.flush().unwrap();
        deck.set_face(0x18, Some(&face("Two", [0, 0, 0]))).unwrap();
        deck.flush().unwrap();
        deck.short_gap = Duration::ZERO;
        deck.sysex_gap = Duration::ZERO;
        deck.ready_at = Instant::now();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 50);
        assert_eq!(
            writes
                .iter()
                .filter(|write| write.get(1) == Some(&0x18))
                .count(),
            1
        );
        assert!(writes.contains(&vec![0x90, 0x18, 0x00]));
        assert_eq!(
            writes
                .iter()
                .filter(|write| write.get(6) == Some(&0x00))
                .count(),
            1
        );
        assert!(writes.contains(&lcd(0x15, 0x00, "Two")));
    }

    #[test]
    fn a_mackie_sysex_from_the_deck_sets_the_lcd_id() {
        let (mut deck, recorder) = open(PRO_XS1_ID);
        deck.set_face(0x1a, Some(&face("Keys", [0, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        queue(
            &recorder,
            &[
                &[0xf0, 0x00, 0x00, 0x66, 0x14, 0x01, 0xf7],
                // Not Mackie's, or short.
                &[0xf0, 0x00, 0x00, 0x67, 0x10, 0xf7],
                &[0xf0, 0x00, 0xf7],
            ],
        );
        assert!(deck.poll(0).unwrap().is_empty());
        deck.poll(0).unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [lcd(0x14, 0x00, "              Keys"), lcd(0x14, 0x38, "")]
        );
        assert_eq!(deck.device, Some(0x14));
    }

    #[test]
    fn pro_x_notes_are_keys() {
        let (mut deck, recorder) = open(PRO_X_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 0x00, 0x7f],
                &[0x90, 0x00, 0x00],
                &[0x90, 0x18, 0x7f],
                &[0x80, 0x18, 0x7f],
                &[0x90, 0x20, 0x41],
                &[0x90, 0x20, 0x40],
                &[0x90, 0x5e, 0x7f],
                &[0x90, 0x67, 0x7f],
                &[0x90, 0x68, 0x7f],
                &[0x90, 0x70, 0x7f],
                // A repeated press, past the table, another channel, short
                // and stray input, pressure.
                &[0x90, 0x5e, 0x7f],
                &[0x90, 0x71, 0x7f],
                &[0x91, 0x18, 0x7f],
                &[0x90],
                &[0x47, 0x00],
                &[0xd2, 0x05],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(0x18, true),
                key(0x18, false),
                key(0x20, true),
                key(0x20, false),
                key(0x5e, true),
                key(0x67, true),
                key(0x68, true),
                key(0x70, true),
            ]
        );
        assert!(deck.warned);
    }

    #[test]
    fn xs1_strip_notes_and_fader_touches_are_keys() {
        let (mut deck, recorder) = open(PRO_XS1_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 0x07, 0x7f],
                &[0x90, 0x27, 0x7f],
                &[0x90, 0x68, 0x7f],
                &[0x90, 0x6f, 0x7f],
                &[0x90, 0x6f, 0x00],
                // No globals or master on the extender.
                &[0x90, 0x28, 0x7f],
                &[0x90, 0x70, 0x7f],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(7, true),
                key(39, true),
                key(40, true),
                key(47, true),
                key(47, false),
            ]
        );
        assert!(deck.warned);
    }

    #[test]
    fn v_pots_and_the_jog_wheel_are_sign_magnitude_relative() {
        assert_eq!(
            (
                relative(0x01),
                relative(0x41),
                relative(0x40),
                relative(0x00)
            ),
            (1, -1, -1, 1)
        );
        assert_eq!(
            (relative(0x3f), relative(0x7f), relative(0x0f)),
            (15, -15, 15)
        );
        let (mut deck, recorder) = open(PRO_X_ID);
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
                // The expression pedal and another channel are not dials.
                &[0xb0, 0x2e, 0x40],
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
            ]
        );
        // The extender has no jog wheel.
        let (mut deck, recorder) = open(PRO_XS1_ID);
        deck.flush().unwrap();
        queue(&recorder, &[&[0xb0, 0x3c, 0x01], &[0xb0, 0x11, 0x42]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(1, -2)]);
    }

    #[test]
    fn faders_turn_their_dials_after_their_first_value() {
        let (mut deck, recorder) = open(PRO_X_ID);
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
        let (mut deck, recorder) = open(PRO_XS1_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xe7, 0x00, 0x00],
                &[0xe7, 0x00, 0x02],
                // No master on the extender.
                &[0xe8, 0x00, 0x00],
                &[0xe8, 0x00, 0x7f],
            ],
        );
        assert_eq!(deck.poll(0).unwrap(), vec![turn(15, 1)]);
    }

    #[test]
    fn letting_go_darkens_and_blanks_the_pro_x() {
        let (mut deck, recorder) = open(PRO_X_ID);
        deck.set_face(0x18, Some(&face("Vox", [255, 0, 0])))
            .unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes.len(), 94 + 2 + 2 + 10 + 8);
        assert!(writes[..94]
            .iter()
            .all(|write| write[0] == 0x90 && write[2] == 0x00));
        assert_eq!(writes[94], lcd(0x14, 0x00, ""));
        assert_eq!(writes[97][..7], [0xf0, 0x00, 0x00, 0x67, 0x15, 0x13, 0x38]);
        assert_eq!(writes[98], vec![0xb0, 0x40, 0x20]);
        assert_eq!(writes[115], vec![0xb0, 0x37, 0x00]);
        assert!(writes.iter().all(|write| write[0] & 0xf0 != 0xe0));
        // Never opened: nothing sent.
        let (deck, recorder) = open(PRO_X_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn each_unit_matches_only_its_own_port() {
        let cases = [
            ("iCON QCON Pro X V1.15", false, Some(PRO_X_ID)),
            ("iCON QCON Pro X V1.15", true, Some(PRO_X_ID)),
            ("ICON QCON PRO X", false, Some(PRO_X_ID)),
            (
                "iCON QCON Pro X V1.15:iCON QCON Pro X V1.15 MIDI 1 24:0",
                false,
                Some(PRO_X_ID),
            ),
            ("iCON QCON Pro XS1 V1.08", false, Some(PRO_XS1_ID)),
            ("iCON QCON Pro XS1 V1.08", true, Some(PRO_XS1_ID)),
            (
                "iCON QCON Pro XS1 V1.08 MIDI 1 28:0",
                false,
                Some(PRO_XS1_ID),
            ),
            ("MIDIIN2 (iCON QCON Pro X V1.15)", true, None),
            ("iCON QCON Pro G2 V1.13", false, Some(PRO_G2_ID)),
            ("iCON QCON Pro G2 V1.09", true, Some(PRO_G2_ID)),
            ("icon qcon pro g2", false, Some(PRO_G2_ID)),
            (
                "iCON QCON Pro G2 V1.13:iCON QCON Pro G2 V1.13 MIDI 1 24:0",
                false,
                Some(PRO_G2_ID),
            ),
            ("MIDIIN2 (iCON QCON Pro G2 V1.13)", true, None),
            ("iCON QCON EX1 G2 V1.13", false, Some(EX_G2_ID)),
            ("iCON QCON EX3 G2 V1.13", true, Some(EX_G2_ID)),
            ("iCON QCON Pro G2 XS1 V1.13", false, Some(EX_G2_ID)),
            ("iCON QCON EX1 X V1.08", false, None),
            // Other iCON decks.
            (
                "Platform M+ V2.15",
                false,
                Some(crate::driver::icon_platform::PLATFORM_M_PLUS_ID),
            ),
            ("Midi Through:Midi Through Port-0 14:0", false, None),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id),
                expected,
                "{raw}"
            );
            for id in [PRO_X_ID, PRO_XS1_ID, PRO_G2_ID, EX_G2_ID] {
                assert_eq!(
                    port_matches(by_id(id).unwrap(), &name),
                    expected == Some(id),
                    "{raw} {id}"
                );
            }
        }
    }

    #[test]
    fn the_model_views_name_every_key_and_light_only_the_lit_ones() {
        let pro_x = ModelView::from(by_id(PRO_X_ID).unwrap());
        let names = pro_x.key_names.clone().unwrap();
        assert_eq!(names.len(), 113);
        assert_eq!(
            (
                names[0],
                names[0x18],
                names[0x28],
                names[0x5e],
                names[0x67],
                names[0x70]
            ),
            (
                "Rec 1",
                "Select 1",
                "Track",
                "Play",
                "User B",
                "Master Touch"
            )
        );
        let lit = pro_x.lit_keys.clone().unwrap();
        assert_eq!(lit.len(), 94);
        assert!(lit
            .iter()
            .all(|key| !(0x20..=0x27).contains(key) && *key <= 0x65));
        assert_eq!((pro_x.keys, pro_x.extra_keys, pro_x.dials), (113, 0, 18));
        assert_eq!((pro_x.rows, pro_x.cols), (15, 8));

        let xs1 = ModelView::from(by_id(PRO_XS1_ID).unwrap());
        let names = xs1.key_names.clone().unwrap();
        assert_eq!(names.len(), 48);
        assert_eq!(
            (names[0], names[39], names[40], names[47]),
            ("Rec 1", "V-Pot 8", "Fader Touch 1", "Fader Touch 8")
        );
        assert_eq!(xs1.lit_keys.clone().unwrap(), (0..32).collect::<Vec<u8>>());
        assert_eq!((xs1.keys, xs1.extra_keys, xs1.dials), (48, 0, 16));
        assert_eq!((xs1.rows, xs1.cols), (6, 8));

        for view in [pro_x, xs1] {
            assert_eq!(view.dial_lights, DialLights::None);
            assert_eq!(view.deck_lights, 0);
            assert_eq!(view.key_light_colors, "on_off");
            assert!(view.key_lights && !view.brightness);
        }
        for id in [PRO_X_ID, PRO_XS1_ID] {
            assert_eq!(by_id(id).unwrap().dial_light_colors(), 0);
        }
    }

    fn lcd55(device: u8, offset: u8, text: &str) -> Vec<u8> {
        let mut message = lcd(device, offset, text);
        message.remove(message.len() - 2);
        message
    }

    #[test]
    fn opening_the_pro_g2_wakes_blanks_and_darkens_it() {
        let (mut deck, recorder) = open(PRO_G2_ID);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 1 + 2 + 10 + 8 + 8 + 94);
        assert_eq!(writes[0], vec![0xf0, 0x00, 0x00, 0x66, 0x14, 0x00, 0xf7]);
        assert_eq!(writes[1], lcd55(0x14, 0x00, ""));
        assert_eq!(writes[1].len(), 7 + 55 + 1);
        assert_eq!(writes[2], lcd55(0x14, 0x38, ""));
        for digit in 0..10u8 {
            assert_eq!(
                writes[3 + usize::from(digit)],
                vec![0xb0, 0x40 + digit, 0x20]
            );
        }
        for strip in 0..8u8 {
            assert_eq!(
                writes[13 + usize::from(strip)],
                vec![0xb0, 0x30 + strip, 0x00]
            );
            assert_eq!(writes[21 + usize::from(strip)], vec![0xd0, strip << 4]);
        }
        let leds: Vec<u8> = writes[29..].iter().map(|write| write[1]).collect();
        let expected: Vec<u8> = (0x00..=0x1f).chain(0x28..=0x65).collect();
        assert_eq!(leds, expected);
        assert!(writes[29..]
            .iter()
            .all(|write| write[0] == 0x90 && write[2] == 0));
        // No second LCD, master meter or pitch bend.
        assert!(writes.iter().all(|write| {
            write.get(3) != Some(&0x67) && write[0] != 0xd1 && write[0] & 0xf0 != 0xe0
        }));
    }

    #[test]
    fn the_ex_g2_writes_under_both_extender_ids_until_it_names_its_own() {
        let (mut deck, recorder) = open(EX_G2_ID);
        deck.set_face(0x18, Some(&face("Keys", [0, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        // The labelled first line replaces its queued blank.
        assert_eq!(writes.len(), 2 + 2 + 8 + 8 + 32 + 2);
        assert_eq!(writes[0], vec![0xf0, 0x00, 0x00, 0x66, 0x14, 0x00, 0xf7]);
        assert_eq!(writes[1], vec![0xf0, 0x00, 0x00, 0x66, 0x15, 0x00, 0xf7]);
        assert_eq!(writes[2..4], [lcd55(0x14, 0x38, ""), lcd55(0x15, 0x38, "")]);
        assert!(writes
            .iter()
            .all(|write| !(write[0] == 0xb0 && write[1] >= 0x40)));
        assert_eq!(
            writes[writes.len() - 2..],
            [lcd55(0x14, 0x00, "Keys"), lcd55(0x15, 0x00, "Keys")]
        );
        let before = writes.len();
        queue(&recorder, &[&[0xf0, 0x00, 0x00, 0x66, 0x15, 0x01, 0xf7]]);
        deck.poll(0).unwrap();
        deck.poll(0).unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [lcd55(0x15, 0x00, "Keys"), lcd55(0x15, 0x38, "")]
        );
        assert_eq!(deck.device, Some(0x15));
    }

    #[test]
    fn g2_rings_show_their_centre_dot_and_nothing_else_lights() {
        let (mut deck, recorder) = open(PRO_G2_ID);
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_dial_light(0, Some([255, 0, 0])).unwrap();
        deck.set_dial_light(7, Some([0, 0, 200])).unwrap();
        deck.set_dial_light(3, Some([10, 0, 0])).unwrap();
        // The jog wheel and faders have no light.
        deck.set_dial_light(8, Some([255, 255, 255])).unwrap();
        deck.set_dial_light(17, Some([255, 255, 255])).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [vec![0xb0, 0x30, 0x06], vec![0xb0, 0x37, 0x06]]
        );
        deck.set_dial_light(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[before + 2..], [vec![0xb0, 0x30, 0x00]]);
        // The Pro X's driver lights no ring.
        let (mut deck, recorder) = open(PRO_X_ID);
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_dial_light(0, Some([255, 0, 0])).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), before);
    }

    #[test]
    fn pro_g2_notes_and_touches_are_keys() {
        let (mut deck, recorder) = open(PRO_G2_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 0x00, 0x7f],
                &[0x90, 0x00, 0x00],
                &[0x90, 0x5e, 0x7f],
                &[0x80, 0x5e, 0x00],
                &[0x90, 0x65, 0x7f],
                &[0x90, 0x68, 0x7f],
                &[0x90, 0x6f, 0x7f],
                &[0x90, 0x70, 0x7f],
                &[0x90, 0x70, 0x00],
                // The rear user switches: not keys, and no mode hint.
                &[0x90, 0x66, 0x7f],
                &[0x90, 0x67, 0x7f],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(0x5e, true),
                key(0x5e, false),
                key(0x65, true),
                key(102, true),
                key(109, true),
                key(110, true),
                key(110, false),
            ]
        );
        assert!(!deck.warned);
        // Past the master touch is unexpected.
        queue(&recorder, &[&[0x90, 0x71, 0x7f]]);
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(deck.warned);
        // A light on a touch key goes nowhere; the LED of Play goes to 0x5e.
        let before = recorder.writes().len();
        deck.set_face(110, Some(&face("", [255, 0, 0]))).unwrap();
        deck.set_face(0x5e, Some(&face("", [255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[before..], [vec![0x90, 0x5e, 0x7f]]);
    }

    #[test]
    fn g2_encoders_and_faders_turn_their_dials() {
        let (mut deck, recorder) = open(PRO_G2_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 0x10, 0x01],
                &[0xb0, 0x17, 0x45],
                &[0xb0, 0x3c, 0x41],
                // Fader 1 seeded, then up 128 and down 256.
                &[0xe0, 0x00, 0x40],
                &[0xe0, 0x00, 0x41],
                &[0xe0, 0x00, 0x3f],
                // The master: seeded, then up 128.
                &[0xe8, 0x00, 0x00],
                &[0xe8, 0x00, 0x01],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, 1),
                turn(7, -5),
                turn(8, -1),
                turn(9, 1),
                turn(9, -2),
                turn(17, 1)
            ]
        );
        let (mut deck, recorder) = open(EX_G2_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 0x3c, 0x01],
                &[0xe7, 0x00, 0x00],
                &[0xe7, 0x00, 0x01],
                &[0xe8, 0x00, 0x00],
                &[0xe8, 0x00, 0x7f],
                &[0x90, 0x6f, 0x7f],
                &[0x90, 0x28, 0x7f],
            ],
        );
        assert_eq!(deck.poll(0).unwrap(), vec![turn(15, 1), key(47, true)]);
    }

    #[test]
    fn letting_go_darkens_and_blanks_the_ex_g2() {
        let (mut deck, recorder) = open(EX_G2_ID);
        deck.set_dial_light(2, Some([255, 0, 0])).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes.len(), 32 + 4 + 8);
        assert!(writes.contains(&vec![0xb0, 0x32, 0x00]));
        assert!(writes.iter().all(|write| write[0] & 0xf0 != 0xe0));
    }

    #[test]
    fn the_g2_model_views_name_every_key_and_light_only_the_lit_ones() {
        let main = ModelView::from(by_id(PRO_G2_ID).unwrap());
        let names = main.key_names.clone().unwrap();
        assert_eq!(names.len(), 111);
        assert_eq!(
            (names[0], names[0x28], names[0x65], names[102], names[110]),
            ("Rec 1", "Track", "Scrub", "Fader Touch 1", "Master Touch")
        );
        let lit = main.lit_keys.clone().unwrap();
        let expected: Vec<u8> = (0..32).chain(40..=101).collect();
        assert_eq!(lit, expected);
        assert_eq!(
            (main.keys, main.dials, main.rows, main.cols),
            (111, 18, 14, 8)
        );
        let extender = ModelView::from(by_id(EX_G2_ID).unwrap());
        assert_eq!(extender.key_names.clone().unwrap().len(), 48);
        assert_eq!(
            extender.lit_keys.clone().unwrap(),
            (0..32).collect::<Vec<u8>>()
        );
        assert_eq!(
            (extender.keys, extender.dials, extender.rows, extender.cols),
            (48, 16, 6, 8)
        );
        for view in [main, extender] {
            assert_eq!(view.dial_lights, DialLights::PerDial);
            assert_eq!(view.deck_lights, 0);
            assert_eq!(view.key_light_colors, "on_off");
            assert!(view.key_lights && !view.brightness);
        }
        for id in [PRO_G2_ID, EX_G2_ID] {
            assert_eq!(by_id(id).unwrap().dial_light_colors(), 8);
        }
    }
}
