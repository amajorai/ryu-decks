//! The first Ableton Push (built by Akai, 2013): its pads, buttons, encoders
//! and touch strip, over class-compliant USB MIDI ([`super::midi`]).
//!
//! Ports: the Push lists two MIDI port pairs, the Live port (the first) for
//! Ableton Live and the User port (the second) for other software. Ryu opens
//! only the User port and never switches the device's mode. [`port_matches`]
//! takes a name containing `Ableton Push` not followed by a digit (so never a
//! Push 2 or Push 3) that is the User port: on Windows the second interface
//! (`MIDIIN2 (Ableton Push)` / `MIDIOUT2 (Ableton Push)`, the bare name being
//! the Live port), on Linux ALSA port 1 of the `Ableton Push` client, on
//! macOS `Ableton Push User Port`.
//!
//! Start-up, queued on the first poll or flush and paced out by later polls
//! and flushes so the shared poll loop never waits: the universal identity
//! request `F0 7E 00 06 01 F7` (the reply, `F0 7E <id> 06 02 47 15 00 19`
//! and 25 data bytes, is logged and never required); the touch strip set to
//! mode 1 (`F0 47 7F 15 63 00 01 01 F7`, host-driven volume: no spring back);
//! the four lines of the character display (`F0 47 7F 15 <18+n> 00 45 00`, 68
//! characters, `F7`), "Ryu" on the first; then every light. Live's own
//! handshake (welcome, the dongle challenge, calibration) is for the Live
//! port and is not sent. There is no keep-alive.
//!
//! In (the channel nibble is masked): the 64 pads are notes 36-99 with 36 at
//! the bottom left, so the pad at row `r` from the top and column `c` is note
//! `92 - 8r + c` and key `8r + c`; Note On above velocity 0 presses, Note Off
//! or velocity 0 releases. The other 63 buttons are Control Change, 127 on
//! press and 0 on release, keys 64-126 in [`BUTTON_CCS`] order. The User
//! button (CC 59) is left unbound: with Live running it is the button that
//! takes the device back. The eleven encoders are two's-complement relative
//! CCs (1-63 clockwise, 64-127 back): Tempo CC 14 is dial 1, Swing CC 15
//! dial 2, the track encoders CC 71-78 dials 3-10 and Master CC 79 dial 11.
//! Their touch notes (0-10) are dropped: the encoders have no push. The
//! touch strip is dial 12: its touch is note 12, its position pitch bend
//! (`value >> 6`, 0 at the bottom), and every 8 units moved while touched
//! turn the dial a step, upward positive. Pedals, aftertouch, active sensing
//! and other messages are dropped.
//!
//! Out, always on channel 1 (other channels animate): the pads (`90 <note>`)
//! and the lower button row (CC 102-109) are RGB and take the nearest index
//! of Live's 128-colour palette; the upper button row (CC 20-27) and the
//! scene column (CC 36-43) are red/amber/yellow/green bi-colour lights (red
//! 1/4, amber 7/10, yellow 13/16, green 19/22, dim/full); every other button
//! is a single-colour light, 0 off, 1 dim, 4 full. Only changed lights are
//! sent. There is no brightness command.
//!
//! Live mode (best effort): should a mode report `F0 47 7F 15 62 00 01 00 F7`
//! arrive, input is dropped until `... 62 00 01 01 F7` returns; then every
//! light and the display are sent again. When Ryu lets go it turns every
//! light off, clears the display and sets the strip to mode 0 (as
//! DrivenByMoss does; the power-on mode is not documented anywhere Ryu read)
//! with the bend at 0.
//!
//! Facts from Ableton Live's Push and pushbase MIDI remote scripts
//! (decompiled, no licence; read for facts only) and DrivenByMoss (LGPL-3.0,
//! Jürgen Moßgraber) for the port names, the display messages and the light
//! values. No code was copied.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::akai_apc40::palette_index;
use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const PUSH_1_ID: &str = "ableton-push";

/// The 64 pads, then the 63 bindable buttons.
pub const KEYS: u8 = 127;
/// The eleven encoders, then the touch strip.
pub const DIALS: u8 = 12;

const PADS: u8 = 64;
const PAD_COLS: u8 = 8;
/// The top-left pad's note; each row down is 8 lower.
const TOP_LEFT_NOTE: u8 = 92;
const LOWEST_PAD_NOTE: u8 = 36;
const HIGHEST_PAD_NOTE: u8 = 99;
const STRIP_TOUCH_NOTE: u8 = 12;

/// The bindable buttons' controllers, keys 64-126 in this order: every
/// button but User (CC 59).
pub const BUTTON_CCS: [u8; 63] = [
    3, 9, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
    48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 60, 61, 62, 63, 85, 86, 87, 88, 89, 90, 102, 103,
    104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119,
];

const USER_CC: u8 = 59;

/// Each button's legend, in [`BUTTON_CCS`] order.
const BUTTON_NAMES: [&str; 63] = [
    "Tap Tempo",
    "Metronome",
    "Upper 1",
    "Upper 2",
    "Upper 3",
    "Upper 4",
    "Upper 5",
    "Upper 6",
    "Upper 7",
    "Upper 8",
    "Master",
    "Stop",
    "1/4",
    "1/4t",
    "1/8",
    "1/8t",
    "1/16",
    "1/16t",
    "1/32",
    "1/32t",
    "Left",
    "Right",
    "Up",
    "Down",
    "Select",
    "Shift",
    "Note",
    "Session",
    "Add Effect",
    "Add Track",
    "Octave Down",
    "Octave Up",
    "Repeat",
    "Accent",
    "Scales",
    "Mute",
    "Solo",
    "In",
    "Out",
    "Play",
    "Record",
    "New",
    "Duplicate",
    "Automation",
    "Fixed Length",
    "Lower 1",
    "Lower 2",
    "Lower 3",
    "Lower 4",
    "Lower 5",
    "Lower 6",
    "Lower 7",
    "Lower 8",
    "Device",
    "Browse",
    "Track",
    "Clip",
    "Volume",
    "Pan & Send",
    "Quantize",
    "Double",
    "Delete",
    "Undo",
];

/// Each button's (row, column) on the 13 x 14 grid, in [`BUTTON_CCS`] order:
/// columns 0-1 are the left block, 3-10 the display (rows 0-1), the two
/// button rows under it (rows 2-3) and the pads (rows 4-11), 11 the scene
/// column, 12-13 the right block.
const BUTTON_CELLS: [(u8, u8); 63] = [
    (0, 0),   // Tap Tempo
    (0, 1),   // Metronome
    (2, 3),   // Upper 1
    (2, 4),   // Upper 2
    (2, 5),   // Upper 3
    (2, 6),   // Upper 4
    (2, 7),   // Upper 5
    (2, 8),   // Upper 6
    (2, 9),   // Upper 7
    (2, 10),  // Upper 8
    (2, 11),  // Master
    (3, 11),  // Stop
    (11, 11), // 1/4
    (10, 11), // 1/4t
    (9, 11),  // 1/8
    (8, 11),  // 1/8t
    (7, 11),  // 1/16
    (6, 11),  // 1/16t
    (5, 11),  // 1/32
    (4, 11),  // 1/32t
    (12, 12), // Left
    (12, 13), // Right
    (11, 12), // Up
    (11, 13), // Down
    (10, 12), // Select
    (10, 13), // Shift
    (9, 12),  // Note
    (9, 13),  // Session
    (8, 12),  // Add Effect
    (8, 13),  // Add Track
    (7, 12),  // Octave Down
    (7, 13),  // Octave Up
    (6, 12),  // Repeat
    (6, 13),  // Accent
    (5, 12),  // Scales
    (4, 12),  // Mute
    (4, 13),  // Solo
    (3, 12),  // In
    (3, 13),  // Out
    (11, 0),  // Play
    (10, 0),  // Record
    (9, 0),   // New
    (8, 0),   // Duplicate
    (7, 0),   // Automation
    (6, 0),   // Fixed Length
    (3, 3),   // Lower 1
    (3, 4),   // Lower 2
    (3, 5),   // Lower 3
    (3, 6),   // Lower 4
    (3, 7),   // Lower 5
    (3, 8),   // Lower 6
    (3, 9),   // Lower 7
    (3, 10),  // Lower 8
    (2, 12),  // Device
    (2, 13),  // Browse
    (1, 12),  // Track
    (1, 13),  // Clip
    (0, 12),  // Volume
    (0, 13),  // Pan & Send
    (5, 0),   // Quantize
    (4, 0),   // Double
    (3, 0),   // Delete
    (2, 0),   // Undo
];

/// The pads' legends, row by row from the top left.
const PAD_NAMES: [&str; 64] = [
    "Pad 1,1", "Pad 1,2", "Pad 1,3", "Pad 1,4", "Pad 1,5", "Pad 1,6", "Pad 1,7", "Pad 1,8",
    "Pad 2,1", "Pad 2,2", "Pad 2,3", "Pad 2,4", "Pad 2,5", "Pad 2,6", "Pad 2,7", "Pad 2,8",
    "Pad 3,1", "Pad 3,2", "Pad 3,3", "Pad 3,4", "Pad 3,5", "Pad 3,6", "Pad 3,7", "Pad 3,8",
    "Pad 4,1", "Pad 4,2", "Pad 4,3", "Pad 4,4", "Pad 4,5", "Pad 4,6", "Pad 4,7", "Pad 4,8",
    "Pad 5,1", "Pad 5,2", "Pad 5,3", "Pad 5,4", "Pad 5,5", "Pad 5,6", "Pad 5,7", "Pad 5,8",
    "Pad 6,1", "Pad 6,2", "Pad 6,3", "Pad 6,4", "Pad 6,5", "Pad 6,6", "Pad 6,7", "Pad 6,8",
    "Pad 7,1", "Pad 7,2", "Pad 7,3", "Pad 7,4", "Pad 7,5", "Pad 7,6", "Pad 7,7", "Pad 7,8",
    "Pad 8,1", "Pad 8,2", "Pad 8,3", "Pad 8,4", "Pad 8,5", "Pad 8,6", "Pad 8,7", "Pad 8,8",
];

const PAD_ROW: u8 = 4;
const PAD_COL: u8 = 3;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const PITCH_BEND: u8 = 0xe0;

const TEMPO_CC: u8 = 14;
const SWING_CC: u8 = 15;
const TRACK_CC: u8 = 71;
const MASTER_CC: u8 = 79;
const STRIP_DIAL: u8 = 11;
/// Strip units (of 256) per dial step.
const STRIP_STEP: i16 = 8;

const SYSEX_START: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
/// Akai's manufacturer id, any device, then the Push's model byte.
const HEADER: [u8; 4] = [SYSEX_START, 0x47, 0x7f, 0x15];
const IDENTITY_REQUEST: [u8; 6] = [0xf0, 0x7e, 0x00, 0x06, 0x01, 0xf7];
/// Akai, then the Push's family bytes, after `F0 7E <id> 06 02`.
const IDENTITY: [u8; 4] = [0x47, 0x15, 0x00, 0x19];
const IDENTITY_DATA: usize = 25;
const WRITE_LINE: u8 = 0x18;
const CLEAR_LINE: u8 = 0x1c;
const SET_MODE: u8 = 0x62;
const SET_STRIP: u8 = 0x63;
const LIVE_MODE: u8 = 0;
const USER_MODE: u8 = 1;
/// Host-driven volume: the position stays where it is left.
const STRIP_RYU: u8 = 1;
/// What DrivenByMoss restores on letting go: host-driven pitch bend.
const STRIP_RELEASE: u8 = 0;

pub const LCD_LINES: u8 = 4;
pub const LCD_CHARS: usize = 68;
/// What the display shows while Ryu holds the deck, line by line.
const LCD_TEXT: [&str; 4] = ["Ryu", "", "", ""];

/// The gap after each SysEx message before the next goes out.
const PACE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(2)
};
const MAX_MESSAGES_PER_POLL: usize = 256;
const READ_LEN: usize = 64;

const OFF: u8 = 0;
/// A colour whose channels are all under this is black.
const DARK: u8 = 32;
/// A colour whose strongest channel is under this is dim.
const DIM: u8 = 128;
const BLACK: [u8; 3] = [0, 0, 0];

/// The bi-colour lights' hues and their dim and full values.
const BI_COLOURS: [([u8; 3], u8, u8); 4] = [
    ([255, 0, 0], 1, 4),
    ([255, 128, 0], 7, 10),
    ([255, 255, 0], 13, 16),
    ([0, 255, 0], 19, 22),
];
const SINGLE_DIM: u8 = 1;
const SINGLE_FULL: u8 = 4;

/// Whether a normalised port name is a first Push's User port.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    const NAME: &str = "ableton push";
    if model.protocol != Protocol::AbletonPush1Midi {
        return false;
    }
    let lower = name.base.to_ascii_lowercase();
    let Some(at) = lower.find(NAME) else {
        return false;
    };
    let rest = lower.get(at + NAME.len()..).unwrap_or("");
    if rest
        .trim_start()
        .starts_with(|next: char| next.is_ascii_digit())
    {
        return false;
    }
    if let Some(interface) = name.interface {
        return interface == 2;
    }
    if let Some(port) = name.alsa_port {
        return port == 1;
    }
    rest.contains("user")
}

fn is_push1(model: &DeviceModel) -> bool {
    model.protocol == Protocol::AbletonPush1Midi
}

/// The legend on each key: the pads by row and column, then the buttons.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    is_push1(model).then(|| {
        PAD_NAMES
            .iter()
            .chain(BUTTON_NAMES.iter())
            .copied()
            .collect()
    })
}

/// Each key's (row, column) on the 13 x 14 grid.
fn cell(key: u8) -> Option<(u8, u8)> {
    if key < PADS {
        return Some((PAD_ROW + key / PAD_COLS, PAD_COL + key % PAD_COLS));
    }
    BUTTON_CELLS.get(usize::from(key - PADS)).copied()
}

/// Each key's cell, as `row * cols + col`.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    if !is_push1(model) {
        return None;
    }
    let cols = u16::from(model.cols);
    Some(
        (0..KEYS)
            .filter_map(cell)
            .map(|(row, col)| u16::from(row) * cols + u16::from(col))
            .collect(),
    )
}

/// The key a pad note is, top-left pad first.
fn key_of_pad(note: u8) -> Option<u8> {
    if !(LOWEST_PAD_NOTE..=HIGHEST_PAD_NOTE).contains(&note) {
        return None;
    }
    let from_bottom = note - LOWEST_PAD_NOTE;
    Some((7 - from_bottom / PAD_COLS) * PAD_COLS + from_bottom % PAD_COLS)
}

/// A pad key's note.
const fn pad_note(key: u8) -> u8 {
    TOP_LEFT_NOTE - PAD_COLS * (key / PAD_COLS) + key % PAD_COLS
}

fn key_of_button(cc: u8) -> Option<u8> {
    if cc == USER_CC {
        return None;
    }
    let at = BUTTON_CCS.iter().position(|button| *button == cc)?;
    u8::try_from(at).ok().map(|at| PADS + at)
}

fn button_cc(key: u8) -> Option<u8> {
    BUTTON_CCS.get(usize::from(key.checked_sub(PADS)?)).copied()
}

/// The kind of light a key has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Light {
    Rgb,
    BiColour,
    Single,
}

fn light_kind(key: u8) -> Light {
    if key < PADS {
        return Light::Rgb;
    }
    match button_cc(key) {
        Some(102..=109) => Light::Rgb,
        Some(20..=27 | 36..=43) => Light::BiColour,
        _ => Light::Single,
    }
}

fn is_dark(color: [u8; 3]) -> bool {
    color.iter().all(|channel| *channel < DARK)
}

/// The palette index nearest a colour on an RGB light.
fn rgb_value(color: [u8; 3]) -> u8 {
    if is_dark(color) {
        return OFF;
    }
    palette_index(color)
}

/// The value nearest a colour on a red/amber/yellow/green light: the hue
/// nearest the colour at full strength, dim under half.
fn bi_colour_value(color: [u8; 3]) -> u8 {
    let peak = color.iter().copied().max().unwrap_or(0);
    if peak < DARK {
        return OFF;
    }
    let scaled = color
        .map(|channel| u8::try_from(u16::from(channel) * 255 / u16::from(peak)).unwrap_or(u8::MAX));
    let distance = |hue: &[u8; 3]| -> u32 {
        hue.iter()
            .zip(scaled)
            .map(|(a, b)| {
                let d = i32::from(*a) - i32::from(b);
                d.unsigned_abs() * d.unsigned_abs()
            })
            .sum()
    };
    BI_COLOURS
        .iter()
        .min_by_key(|(hue, _, _)| distance(hue))
        .map_or(OFF, |(_, dim, full)| if peak < DIM { *dim } else { *full })
}

/// The value nearest a colour's luma on a single-colour light.
fn single_value(color: [u8; 3]) -> u8 {
    let [red, green, blue] = color.map(u32::from);
    let luma = (299 * red + 587 * green + 114 * blue) / 1000;
    if luma < u32::from(DARK) {
        OFF
    } else if luma < u32::from(DIM) {
        SINGLE_DIM
    } else {
        SINGLE_FULL
    }
}

/// A two's-complement relative value as a step.
fn relative(value: u8) -> i8 {
    let value = i16::from(value & 0x7f);
    let signed = if value < 64 { value } else { value - 128 };
    i8::try_from(signed).unwrap_or(0)
}

/// One of the Push's SysEx commands, with its two length bytes.
fn command(id: u8, args: &[u8]) -> Vec<u8> {
    let mut message = HEADER.to_vec();
    message.push(id);
    let len = u8::try_from(args.len()).unwrap_or(0x7f);
    message.extend([0x00, len & 0x7f]);
    message.extend(args.iter().map(|arg| arg & 0x7f));
    message.push(SYSEX_END);
    message
}

/// The message that writes a line of the display: 68 characters, anything
/// outside printable ASCII shown as a space.
fn lcd_line(line: u8, text: &str) -> Vec<u8> {
    let mut chars: Vec<u8> = text
        .chars()
        .take(LCD_CHARS)
        .map(|c| {
            u8::try_from(c)
                .ok()
                .filter(|byte| (0x20..0x7f).contains(byte))
                .unwrap_or(b' ')
        })
        .collect();
    chars.resize(LCD_CHARS, b' ');
    let mut args = vec![0x00];
    args.extend(chars);
    command(WRITE_LINE + line, &args)
}

/// The message that clears a line of the display.
fn lcd_clear(line: u8) -> Vec<u8> {
    command(CLEAR_LINE + line, &[])
}

/// The mode a mode report names.
fn mode_reply(message: &[u8]) -> Option<u8> {
    let rest = message.strip_prefix(&HEADER)?;
    match rest {
        [SET_MODE, 0x00, 0x01, mode, SYSEX_END] => Some(*mode),
        _ => None,
    }
}

/// What the device says it is, from the universal identity reply.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    major: u16,
    minor: u16,
    bootloader: bool,
}

fn identity_reply(message: &[u8]) -> Option<Identity> {
    let start = message.get(..5)?;
    if start[0] != SYSEX_START || start[1] != 0x7e || start[3] != 0x06 || start[4] != 0x02 {
        return None;
    }
    if message.get(5..9)? != IDENTITY || message.len() != 9 + IDENTITY_DATA + 1 {
        return None;
    }
    let data = message.get(9..9 + IDENTITY_DATA)?;
    let bootloader = data
        .get(9..25)?
        .iter()
        .zip(1u8..)
        .all(|(byte, expected)| *byte == expected);
    let digit = |at: usize| u16::from(data[at] & 0x7f);
    Some(Identity {
        major: digit(0) * 10 + digit(1),
        minor: digit(2) * 10 + digit(3),
        bootloader,
    })
}

/// The touch strip, read as a dial.
#[derive(Debug, Default)]
struct Strip {
    last: Option<i16>,
    rest: i16,
}

pub struct Push1 {
    transport: Box<dyn Transport>,
    /// The start-up is queued.
    started: bool,
    /// Live took the device back: input is dropped until User mode returns.
    in_live: bool,
    /// Messages waiting their turn; each SysEx is followed by `pace`.
    outbox: VecDeque<Vec<u8>>,
    pace: Duration,
    /// When the next SysEx may go out.
    next_sysex: Instant,
    colors: Vec<[u8; 3]>,
    /// The value each key's light was last queued; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    strip: Strip,
    parser: Parser,
}

impl Push1 {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            in_live: false,
            outbox: VecDeque::new(),
            pace: PACE,
            next_sysex: Instant::now(),
            colors: vec![BLACK; usize::from(KEYS)],
            sent: vec![None; usize::from(KEYS)],
            held: vec![false; usize::from(KEYS)],
            strip: Strip::default(),
            parser: Parser::default(),
        }
    }

    /// Writes the queued messages whose turn has come, without sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        while let Some(message) = self.outbox.front() {
            let sysex = message.first() == Some(&SYSEX_START);
            let now = Instant::now();
            if sysex && now < self.next_sysex {
                break;
            }
            self.transport.write(message)?;
            self.outbox.pop_front();
            if sysex {
                self.next_sysex = now + self.pace;
            }
        }
        Ok(())
    }

    fn queue_display(&mut self) {
        for (line, text) in (0..LCD_LINES).zip(LCD_TEXT) {
            self.outbox.push_back(lcd_line(line, text));
        }
    }

    /// Queues the start-up, then every light.
    fn ensure_started(&mut self) {
        if self.started {
            return;
        }
        self.outbox.push_back(IDENTITY_REQUEST.to_vec());
        self.outbox.push_back(command(SET_STRIP, &[STRIP_RYU]));
        self.queue_display();
        self.started = true;
        self.queue_lights();
    }

    /// The message that sets a key's light to a value.
    fn light(key: u8, value: u8) -> Option<[u8; 3]> {
        if key < PADS {
            return Some([NOTE_ON, pad_note(key), value]);
        }
        button_cc(key).map(|cc| [CONTROL_CHANGE, cc, value])
    }

    fn value_for(&self, key: u8) -> u8 {
        let color = self.colors.get(usize::from(key)).copied().unwrap_or(BLACK);
        match light_kind(key) {
            Light::Rgb => rgb_value(color),
            Light::BiColour => bi_colour_value(color),
            Light::Single => single_value(color),
        }
    }

    /// Queues one message for each light whose value changed.
    fn queue_lights(&mut self) {
        if self.in_live {
            return;
        }
        for key in 0..KEYS {
            let value = self.value_for(key);
            let Some(message) = Self::light(key, value) else {
                continue;
            };
            let Some(sent) = self.sent.get_mut(usize::from(key)) else {
                continue;
            };
            if *sent != Some(value) {
                self.outbox.push_back(message.to_vec());
                *sent = Some(value);
            }
        }
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

    /// Lets go of every held key, as Live takes the device.
    fn release_all(&mut self, events: &mut Vec<Input>) {
        for key in 0..KEYS {
            self.press(key, false, events);
        }
        self.strip = Strip::default();
    }

    fn strip_position(&mut self, lsb: u8, msb: u8, events: &mut Vec<Input>) {
        let position = ((i16::from(msb & 0x7f) << 7) | i16::from(lsb & 0x7f)) >> 6;
        let Some(last) = self.strip.last.replace(position) else {
            return;
        };
        let moved = self.strip.rest + position - last;
        let steps = moved / STRIP_STEP;
        self.strip.rest = moved - steps * STRIP_STEP;
        if steps != 0 {
            events.push(Input::DialTurn {
                index: STRIP_DIAL,
                delta: i8::try_from(steps.clamp(-127, 127)).unwrap_or(0),
            });
        }
    }

    fn note(&mut self, note: u8, pressed: bool, events: &mut Vec<Input>) {
        if let Some(key) = key_of_pad(note) {
            self.press(key, pressed, events);
        } else if note == STRIP_TOUCH_NOTE {
            // Each touch starts from wherever the finger lands.
            self.strip = Strip::default();
        }
        // Notes 0-10 are the encoders' touch, which has no push.
    }

    fn sysex(&mut self, bytes: &[u8], events: &mut Vec<Input>) {
        if let Some(identity) = identity_reply(bytes) {
            tracing::debug!(
                firmware = %format!("{}.{}", identity.major, identity.minor),
                bootloader = identity.bootloader,
                "ryu-decks: Ableton Push identified"
            );
            return;
        }
        match mode_reply(bytes) {
            Some(LIVE_MODE) if !self.in_live => {
                tracing::debug!("ryu-decks: Live took the Ableton Push back");
                self.in_live = true;
                self.release_all(events);
            }
            Some(USER_MODE) if self.in_live => {
                self.in_live = false;
                self.sent.fill(None);
                self.queue_display();
                self.queue_lights();
            }
            _ => {}
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let [status, number, value] = match message {
            Message::SysEx(bytes) => {
                self.sysex(bytes, events);
                return;
            }
            Message::Channel(bytes) => *bytes,
        };
        if self.in_live {
            return;
        }
        match status & 0xf0 {
            NOTE_ON | NOTE_OFF => self.note(number, status & 0xf0 == NOTE_ON && value > 0, events),
            CONTROL_CHANGE => {
                if let Some(dial) = dial_of_cc(number) {
                    let delta = relative(value);
                    if delta != 0 {
                        events.push(Input::DialTurn { index: dial, delta });
                    }
                } else if let Some(key) = key_of_button(number) {
                    self.press(key, value > 0, events);
                }
            }
            // The first bend of a touch seeds the strip.
            PITCH_BEND => self.strip_position(number, value, events),
            _ => {}
        }
    }
}

fn dial_of_cc(cc: u8) -> Option<u8> {
    match cc {
        TEMPO_CC => Some(0),
        SWING_CC => Some(1),
        TRACK_CC..=MASTER_CC => Some(2 + cc - TRACK_CC),
        _ => None,
    }
}

impl Driver for Push1 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_started();
        self.pump()?;
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
        let mut events = Vec::new();
        for message in &messages {
            self.receive(message, &mut events);
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

    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started();
        self.queue_lights();
        self.pump()
    }

    /// The first Push has no brightness command.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for Push1 {
    /// Turns every light off, clears the display and puts the strip back.
    /// Nothing waits; write errors (an unplugged deck) are ignored.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        self.outbox.clear();
        for key in 0..KEYS {
            if let Some(message) = Self::light(key, OFF) {
                let _ = self.transport.write(&message);
            }
        }
        for line in 0..LCD_LINES {
            let _ = self.transport.write(&lcd_clear(line));
        }
        let _ = self.transport.write(&command(SET_STRIP, &[STRIP_RELEASE]));
        let _ = self.transport.write(&[PITCH_BEND, 0x00, 0x00]);
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, ModelView};

    const LIVE_REPORT: [u8; 9] = [0xf0, 0x47, 0x7f, 0x15, 0x62, 0x00, 0x01, 0x00, 0xf7];
    const USER_REPORT: [u8; 9] = [0xf0, 0x47, 0x7f, 0x15, 0x62, 0x00, 0x01, 0x01, 0xf7];

    fn open() -> (Push1, Recorder) {
        let recorder = Recorder::default();
        let deck = Push1::new(by_id(PUSH_1_ID).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// An opened deck past its start-up, with what that sent cleared.
    fn started() -> (Push1, Recorder) {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        while !deck.outbox.is_empty() {
            deck.flush().unwrap();
        }
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

    fn feed(deck: &mut Push1, recorder: &Recorder, messages: &[&[u8]]) -> Vec<Input> {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
        deck.poll(0).unwrap()
    }

    #[test]
    fn the_start_up_sets_the_strip_writes_the_display_then_every_light() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        // A pad pressed meanwhile counts at once.
        recorder.queue_read(vec![0x90, 92, 0x40]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(0, true)]);
        while !deck.outbox.is_empty() {
            deck.flush().unwrap();
        }
        let writes = recorder.writes();
        assert_eq!(writes[0], IDENTITY_REQUEST.to_vec());
        assert_eq!(
            writes[1],
            vec![0xf0, 0x47, 0x7f, 0x15, 0x63, 0x00, 0x01, 0x01, 0xf7]
        );
        let line = &writes[2];
        assert_eq!(line.len(), 77);
        assert_eq!(line[..8], [0xf0, 0x47, 0x7f, 0x15, 0x18, 0x00, 0x45, 0x00]);
        assert_eq!(line[8..11], *b"Ryu");
        assert!(line[11..76].iter().all(|byte| *byte == b' '));
        assert_eq!(line[76], 0xf7);
        for n in 1..4u8 {
            assert_eq!(writes[usize::from(n) + 2][4], 0x18 + n);
            assert_eq!(writes[usize::from(n) + 2].len(), 77);
        }
        // Then all 127 lights: the top-left pad red.
        assert_eq!(writes[6], vec![0x90, 92, 5]);
        assert_eq!(writes[7], vec![0x90, 93, 0]);
        assert_eq!(writes[6 + 64], vec![0xb0, 3, 0]);
        assert_eq!(writes[6 + 126], vec![0xb0, 119, 0]);
        assert_eq!(writes.len(), 6 + 127);
    }

    #[test]
    fn sysex_is_paced_by_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open();
        deck.pace = Duration::from_secs(60);
        let begun = Instant::now();
        deck.flush().unwrap();
        deck.flush().unwrap();
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert_eq!(recorder.writes(), vec![IDENTITY_REQUEST.to_vec()]);
        deck.pace = Duration::ZERO;
        deck.next_sysex = Instant::now();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 6 + 127);
    }

    #[test]
    fn the_identity_reply_is_read_and_logged_only() {
        let mut reply = vec![0xf0, 0x7e, 0x00, 0x06, 0x02, 0x47, 0x15, 0x00, 0x19];
        reply.extend([0, 1, 1, 6]);
        reply.extend([0; 21]);
        reply.push(0xf7);
        assert_eq!(
            identity_reply(&reply),
            Some(Identity {
                major: 1,
                minor: 16,
                bootloader: false
            })
        );
        let mut boot = reply.clone();
        for (at, byte) in boot[18..34].iter_mut().zip(1u8..) {
            *at = byte;
        }
        assert!(identity_reply(&boot).unwrap().bootloader);
        assert_eq!(identity_reply(&reply[..20]), None);
        assert_eq!(identity_reply(&[0xf0]), None);
        assert_eq!(mode_reply(&[0xf0, 0x47]), None);
        let (mut deck, recorder) = started();
        assert!(feed(&mut deck, &recorder, &[&reply, &[0xfe]]).is_empty());
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn pads_map_from_the_top_left() {
        let (mut deck, recorder) = started();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x90, 92, 0x7f],
                &[0x90, 99, 0x01],
                &[0x90, 36, 0x30],
                &[0x90, 43, 0x30],
                &[0x80, 92, 0x00],
                &[0x90, 99, 0x00],
                // Out of the pad range, and short messages.
                &[0x90, 100, 0x7f],
                &[0x90, 35, 0x7f],
                &[0x90],
                &[0xa0, 36, 0x10],
                &[0xd0, 0x20],
            ],
        );
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(7, true),
                key(56, true),
                key(63, true),
                key(0, false),
                key(7, false),
            ]
        );
        assert_eq!(pad_note(0), 92);
        assert_eq!(pad_note(63), 43);
        for key in 0..PADS {
            assert_eq!(key_of_pad(pad_note(key)), Some(key));
        }
    }

    #[test]
    fn buttons_are_keys_64_on_and_user_is_never_bound() {
        let (mut deck, recorder) = started();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xb0, 3, 0x7f],
                &[0xb0, 3, 0x00],
                &[0xb0, 119, 0x7f],
                &[0xb0, 43, 0x7f],
                &[0xb0, USER_CC, 0x7f],
                &[0xb0, 30, 0x7f],
                &[0xb0, 64, 0x7f],
                &[0xb0, 69, 0x7f],
                &[0xb0, 1, 0x40],
            ],
        );
        assert_eq!(
            events,
            vec![key(64, true), key(64, false), key(126, true), key(83, true)]
        );
        assert!(!BUTTON_CCS.contains(&USER_CC));
        assert_eq!(
            (0..KEYS).filter(|k| light_kind(*k) == Light::Rgb).count(),
            72
        );
        assert_eq!(
            (0..KEYS)
                .filter(|k| light_kind(*k) == Light::BiColour)
                .count(),
            16
        );
    }

    #[test]
    fn encoders_turn_by_twos_complement_and_touch_never_presses() {
        let (mut deck, recorder) = started();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xb0, 14, 0x01],
                &[0xb0, 15, 0x7f],
                &[0xb0, 71, 0x40],
                &[0xb0, 78, 0x3f],
                &[0xb0, 79, 0x02],
                &[0x90, 0, 0x7f],
                &[0x90, 10, 0x7f],
                &[0x90, 8, 0x00],
            ],
        );
        assert_eq!(
            events,
            vec![
                turn(0, 1),
                turn(1, -1),
                turn(2, -64),
                turn(9, 63),
                turn(10, 2)
            ]
        );
    }

    fn bend(position: u8) -> Vec<u8> {
        let value = u16::from(position) << 6;
        vec![
            0xe0,
            u8::try_from(value & 0x7f).unwrap(),
            u8::try_from(value >> 7).unwrap(),
        ]
    }

    #[test]
    fn the_strip_turns_dial_12_every_8_units() {
        let (mut deck, recorder) = started();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x90, STRIP_TOUCH_NOTE, 0x7f],
                &bend(100),
                &bend(105),
                &bend(120),
                &bend(90),
                &[0x90, STRIP_TOUCH_NOTE, 0x00],
                &[0x90, STRIP_TOUCH_NOTE, 0x7f],
                &bend(200),
                &bend(207),
            ],
        );
        assert_eq!(events, vec![turn(11, 2), turn(11, -3)]);
    }

    #[test]
    fn each_kind_of_light_takes_its_own_values_on_channel_1() {
        let (mut deck, recorder) = started();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        // Upper 1 bi-colour: green full, Upper 2 amber dim, Upper 3 yellow.
        deck.set_face(66, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(67, Some(&lit([100, 50, 0]))).unwrap();
        deck.set_face(68, Some(&lit([250, 240, 10]))).unwrap();
        // The top scene button (1/32t) red dim.
        deck.set_face(83, Some(&lit([90, 0, 0]))).unwrap();
        // Tap Tempo white full, Metronome dim; Lower 1 RGB blue.
        deck.set_face(64, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(65, Some(&lit([60, 60, 60]))).unwrap();
        deck.set_face(109, Some(&lit([0, 0, 255]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x90, 92, 5],
                vec![0xb0, 3, 4],
                vec![0xb0, 9, 1],
                vec![0xb0, 20, 22],
                vec![0xb0, 21, 7],
                vec![0xb0, 22, 16],
                vec![0xb0, 43, 1],
                vec![0xb0, 102, palette_index([0, 0, 255])],
            ]
        );
        recorder.0.lock().unwrap().writes.clear();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes(), vec![vec![0x90, 92, 0]]);
        assert_eq!(bi_colour_value([10, 10, 10]), 0);
        assert_eq!(single_value([10, 10, 10]), 0);
        deck.set_brightness(50).unwrap();
    }

    #[test]
    fn dropping_turns_the_lights_off_and_clears_the_display() {
        let (mut deck, recorder) = started();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 127 + 4 + 2);
        assert_eq!(writes[0], vec![0x90, 92, 0]);
        assert_eq!(writes[126], vec![0xb0, 119, 0]);
        assert_eq!(
            writes[127],
            vec![0xf0, 0x47, 0x7f, 0x15, 0x1c, 0x00, 0x00, 0xf7]
        );
        assert_eq!(
            writes[130],
            vec![0xf0, 0x47, 0x7f, 0x15, 0x1f, 0x00, 0x00, 0xf7]
        );
        assert_eq!(
            writes[131],
            vec![0xf0, 0x47, 0x7f, 0x15, 0x63, 0x00, 0x01, 0x00, 0xf7]
        );
        assert_eq!(writes[132], vec![0xe0, 0x00, 0x00]);
        // No mode change.
        assert!(!writes.iter().any(|write| mode_reply(write).is_some()));
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn a_live_mode_report_mutes_input_until_user_mode_returns() {
        let (mut deck, recorder) = started();
        let events = feed(&mut deck, &recorder, &[&[0x90, 92, 0x7f], &LIVE_REPORT]);
        assert_eq!(events, vec![key(0, true), key(0, false)]);
        deck.set_face(1, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        let events = feed(&mut deck, &recorder, &[&[0x90, 92, 0x7f], &[0xb0, 14, 1]]);
        assert!(events.is_empty());
        // Ryu never switches the mode itself, nor lights while Live holds it.
        assert!(recorder.writes().is_empty());
        let events = feed(&mut deck, &recorder, &[&USER_REPORT, &[0x90, 92, 0x7f]]);
        assert_eq!(events, vec![key(0, true)]);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 4 + 127);
        assert_eq!(writes[0][4], 0x18);
        assert_eq!(writes[5], vec![0x90, 93, palette_index([0, 255, 0])]);
    }

    #[test]
    fn only_the_user_port_matches() {
        let model = by_id(PUSH_1_ID).unwrap();
        let cases = [
            ("Ableton Push User Port", false, true),
            ("Ableton Push Live Port", false, false),
            ("Ableton Push:Ableton Push MIDI 2 20:1", false, true),
            ("Ableton Push:Ableton Push MIDI 1 20:0", false, false),
            ("MIDIIN2 (Ableton Push)", true, true),
            ("MIDIOUT2 (Ableton Push) 1", true, true),
            ("MIDIIN2 (2- Ableton Push)", true, true),
            ("Ableton Push", true, false),
            ("Ableton Push 2 User Port", false, false),
            ("Ableton Push 3 User Port", false, false),
            ("MIDIIN2 (Ableton Push 2)", true, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            // A Push 2's User port is that model's, never this one's.
            assert_eq!(
                identify_midi(&name, windows)
                    .map(|model| model.id)
                    .filter(|id| *id != super::super::push2::PUSH_2_ID),
                expected.then_some(PUSH_1_ID),
                "{raw}"
            );
        }
    }

    #[test]
    fn every_key_has_a_name_and_its_own_cell() {
        let model = by_id(PUSH_1_ID).unwrap();
        let view = ModelView::from(model);
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), usize::from(KEYS));
        assert_eq!(names[0], "Pad 1,1");
        assert_eq!(names[63], "Pad 8,8");
        assert_eq!(names[64], "Tap Tempo");
        assert_eq!(names[126], "Undo");
        let cells = view.key_cells.unwrap();
        assert_eq!(cells.len(), usize::from(KEYS));
        let unique: std::collections::HashSet<_> = cells.iter().collect();
        assert_eq!(unique.len(), cells.len());
        let grid = u16::from(model.rows) * u16::from(model.cols);
        assert!(cells.iter().all(|cell| *cell < grid));
        assert_eq!(cells[0], 4 * 14 + 3);
        assert_eq!(view.key_light_colors, "rgb");
        assert!(!view.brightness && view.key_lights);
    }
}
