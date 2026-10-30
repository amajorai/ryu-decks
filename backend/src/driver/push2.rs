//! Ableton Push 2's pads, buttons, encoders and touch strip, over
//! class-compliant USB MIDI ([`super::midi`]).
//!
//! Ports: the Push 2 lists two MIDI port pairs, the Live port (the first) for
//! Ableton Live and the User port (the second) for other software. Ryu opens
//! only the User port. [`port_matches`] takes a name containing `Ableton Push
//! 2` (never `Push 3` or the first `Ableton Push`) that is the User port: on
//! Windows the second interface (`MIDIIN2 (Ableton Push 2)` /
//! `MIDIOUT2 (Ableton Push 2)`, the bare name being the Live port), on Linux
//! ALSA port 1 of the `Ableton Push 2` client, on macOS `Ableton Push 2 User
//! Port`. macOS gives two Push 2s the same names, so they pair in the order
//! the host lists them. The display is a separate libusb bulk interface
//! (neither HID nor MIDI) and is not driven: it stays black.
//!
//! Start-up (on the first poll or flush, every wait bounded): the universal
//! identity request `F0 7E 01 06 01 F7`, whose reply (`00 21 1D 67 32 02 00`
//! at bytes 5-11: Ableton, family 0x1967, member 2) is logged but not
//! required; `F0 00 21 1D 01 01 0A 01 F7` (User mode: until it is set the
//! device ignores the User port's lights and sends its input only to the Live
//! port), waiting up to 500 ms for the echo; the colour palette read back one
//! entry at a time with `0x04` (never written: firmware 1.0.63 and later keeps
//! a written palette until it restarts and Live would not send its own again),
//! falling back to Live's own palette at once if the first entry does not
//! answer; the touch strip set to `0x08` (values as pitch bend, no return to
//! centre); then every light and the brightness. Anything else that arrives
//! meanwhile is kept for the first poll. There is no keep-alive.
//!
//! In (the channel nibble is masked): the 64 pads are notes 36-99 with 36 at
//! the bottom left, so the pad at row `r` from the top and column `c` is note
//! `92 - 8r + c` and key `8r + c`; Note On above velocity 0 presses, Note Off
//! or velocity 0 releases. The other 64 buttons are Control Change, 127 on
//! press and 0 on release, keys 64-127 in [`BUTTON_CCS`] order; the User
//! button (CC 59) is never bound, since it switches the device between Live
//! and User. The eleven encoders are two's-complement relative CCs (1-63
//! clockwise, 64-127 back): Tempo CC 14 is dial 1, Swing CC 15 dial 2, the
//! track encoders CC 71-78 dials 3-10 and Master CC 79 dial 11. Their touch
//! notes (0-10) are dropped: the encoders have no push. The touch strip is
//! dial 12: its touch is note 12, its position pitch bend (8 useful bits,
//! `value >> 6`, 0 at the bottom), and every 8 units moved while touched turn
//! the dial a step, upward positive. Pedals, aftertouch and other messages are
//! dropped.
//!
//! Out: every light takes a palette index, a pad by `90 <note> <index>`, a
//! button by `B0 <cc> <index>`, always on channel 1 (other channels animate).
//! The pads and 30 of the buttons are RGB and take the nearest palette colour
//! (index 117, a second black, is skipped); the other 34 buttons are white
//! and take the index whose white level is nearest the colour's luma. Black
//! is index 0 (off). Only changed lights are sent. Brightness is `F0 00 21 1D
//! 01 01 06 <0-127> F7`; the firmware caps it at 8 on USB power alone.
//!
//! Live mode: when a mode report says Live took the device back (`0A 00`, its
//! User button), input is dropped until User or Dual mode returns; then every
//! light and the brightness are sent again. Ryu never switches the mode back
//! on its own. When Ryu lets go it turns every light off, restores the default
//! strip setting (`0x68`) and hands the device back in Live mode, the mode it
//! starts in.
//!
//! Facts from Ableton's Push 2 MIDI and Display Interface Manual and its
//! Push2-map.json (Ableton/push-interface, published under a licence to use
//! the manual; read for facts only), cross-checked against Ableton Live's
//! Push2 and pushbase remote scripts (decompiled, no licence; read for the
//! SysEx framing, the strip modes and the palette only). No code was copied.

use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const PUSH_2_ID: &str = "ableton-push-2";

/// The 64 pads, then the 64 bindable buttons.
pub const KEYS: u8 = 128;
/// The eleven encoders, then the touch strip.
pub const DIALS: u8 = 12;

const PADS: u8 = 64;
const PAD_COLS: u8 = 8;
/// The top-left pad's note; each row down is 8 lower.
const TOP_LEFT_NOTE: u8 = 92;
const LOWEST_PAD_NOTE: u8 = 36;
const HIGHEST_PAD_NOTE: u8 = 99;
const STRIP_TOUCH_NOTE: u8 = 12;

/// The bindable buttons' controllers, keys 64-127 in this order: every
/// button but User (CC 59).
pub const BUTTON_CCS: [u8; 64] = [
    3, 9, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44,
    45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 60, 61, 62, 63, 85, 86, 87, 88, 89, 90,
    102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 116, 117, 118, 119,
];

/// The buttons with an RGB light; the others are white.
const RGB_BUTTON_CCS: [u8; 30] = [
    20, 21, 22, 23, 24, 25, 26, 27, 29, 36, 37, 38, 39, 40, 41, 42, 43, 60, 61, 85, 86, 89, 102,
    103, 104, 105, 106, 107, 108, 109,
];

const USER_CC: u8 = 59;

/// Each button's legend, in [`BUTTON_CCS`] order.
const BUTTON_NAMES: [&str; 64] = [
    "Tap Tempo",
    "Metronome",
    "Lower 1",
    "Lower 2",
    "Lower 3",
    "Lower 4",
    "Lower 5",
    "Lower 6",
    "Lower 7",
    "Lower 8",
    "Master",
    "Stop Clip",
    "Setup",
    "Layout",
    "Convert",
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
    "Add Device",
    "Add Track",
    "Octave Down",
    "Octave Up",
    "Repeat",
    "Accent",
    "Scale",
    "Mute",
    "Solo",
    "Page Left",
    "Page Right",
    "Play",
    "Record",
    "New",
    "Duplicate",
    "Automate",
    "Fixed Length",
    "Upper 1",
    "Upper 2",
    "Upper 3",
    "Upper 4",
    "Upper 5",
    "Upper 6",
    "Upper 7",
    "Upper 8",
    "Device",
    "Browse",
    "Mix",
    "Clip",
    "Quantize",
    "Double Loop",
    "Delete",
    "Undo",
];

/// Each button's (row, column) on the 13 x 14 grid, in [`BUTTON_CCS`] order:
/// columns 0-2 are the left block, 3-10 the pads, 11 the scene column, 12-13
/// the right block; the pads fill rows 4-11.
const BUTTON_CELLS: [(u8, u8); 64] = [
    (0, 0),   // Tap Tempo
    (0, 1),   // Metronome
    (3, 3),   // Lower 1
    (3, 4),   // Lower 2
    (3, 5),   // Lower 3
    (3, 6),   // Lower 4
    (3, 7),   // Lower 5
    (3, 8),   // Lower 6
    (3, 9),   // Lower 7
    (3, 10),  // Lower 8
    (3, 11),  // Master
    (3, 2),   // Stop Clip
    (0, 12),  // Setup
    (8, 13),  // Layout
    (4, 0),   // Convert
    (11, 11), // 1/4
    (10, 11), // 1/4t
    (9, 11),  // 1/8
    (8, 11),  // 1/8t
    (7, 11),  // 1/16
    (6, 11),  // 1/16t
    (5, 11),  // 1/32
    (4, 11),  // 1/32t
    (4, 12),  // Left
    (4, 13),  // Right
    (3, 12),  // Up
    (5, 12),  // Down
    (12, 13), // Select
    (12, 12), // Shift
    (9, 12),  // Note
    (9, 13),  // Session
    (1, 11),  // Add Device
    (2, 11),  // Add Track
    (10, 13), // Octave Down
    (10, 12), // Octave Up
    (7, 12),  // Repeat
    (7, 13),  // Accent
    (8, 12),  // Scale
    (3, 0),   // Mute
    (3, 1),   // Solo
    (11, 12), // Page Left
    (11, 13), // Page Right
    (12, 0),  // Play
    (11, 0),  // Record
    (8, 0),   // New
    (7, 0),   // Duplicate
    (10, 0),  // Automate
    (9, 0),   // Fixed Length
    (0, 3),   // Upper 1
    (0, 4),   // Upper 2
    (0, 5),   // Upper 3
    (0, 6),   // Upper 4
    (0, 7),   // Upper 5
    (0, 8),   // Upper 6
    (0, 9),   // Upper 7
    (0, 10),  // Upper 8
    (1, 12),  // Device
    (2, 12),  // Browse
    (1, 13),  // Mix
    (2, 13),  // Clip
    (6, 0),   // Quantize
    (5, 0),   // Double Loop
    (1, 0),   // Delete
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
/// Ableton's manufacturer id, then Push 2's device and model bytes.
const HEADER: [u8; 6] = [SYSEX_START, 0x00, 0x21, 0x1d, 0x01, 0x01];
const IDENTITY_REQUEST: [u8; 6] = [0xf0, 0x7e, 0x01, 0x06, 0x01, 0xf7];
/// Ableton, family 0x1967 (LSB first) and member 2 (Push 2).
const IDENTITY: [u8; 7] = [0x00, 0x21, 0x1d, 0x67, 0x32, 0x02, 0x00];
const SET_MODE: u8 = 0x0a;
const GET_PALETTE: u8 = 0x04;
const SET_BRIGHTNESS: u8 = 0x06;
const SET_STRIP: u8 = 0x17;
const LIVE_MODE: u8 = 0;
const USER_MODE: u8 = 1;
/// Values as pitch bend, a point, from the bottom, no return to centre.
const STRIP_RYU: u8 = 0x08;
/// The power-on setting: as above, returning to the centre.
const STRIP_DEFAULT: u8 = 0x68;

const IDENTITY_WAIT: Duration = Duration::from_millis(500);
const MODE_WAIT: Duration = Duration::from_millis(500);
const PALETTE_WAIT: Duration = Duration::from_millis(100);
const MAX_HANDSHAKE_MESSAGES: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 256;
const READ_LEN: usize = 64;

const OFF: u8 = 0;
/// Index 117 is black on the RGB lights (and a white level on white ones).
const SECOND_BLACK: usize = 117;
/// A colour whose channels are all under this is black.
const DARK: u8 = 8;
const BLACK: [u8; 3] = [0, 0, 0];

/// Ableton Live's Push 2 palette, `(0xRRGGBB, white level)` by index: the
/// fallback when the device does not answer a palette read.
const FALLBACK_PALETTE: [(u32, u8); 128] = [
    (0x000000, 0),
    (0xff4032, 2),
    (0x800400, 4),
    (0xc93c00, 6),
    (0xac1f00, 8),
    (0x8c5018, 10),
    (0x491804, 12),
    (0xfadc3b, 14),
    (0xffc516, 16),
    (0xb6ff0e, 18),
    (0x79ff18, 20),
    (0x34c216, 22),
    (0x4f8a04, 24),
    (0x62ff55, 26),
    (0x297d53, 28),
    (0x269e72, 30),
    (0x31adff, 32),
    (0x3663fc, 34),
    (0x1a34ff, 36),
    (0x1c0ce6, 38),
    (0x153999, 40),
    (0x3937ff, 42),
    (0x5722ff, 44),
    (0x972bff, 46),
    (0x852178, 48),
    (0xff1032, 50),
    (0xff2bd4, 52),
    (0xa63421, 54),
    (0x995628, 56),
    (0x876700, 58),
    (0x90821f, 60),
    (0x4a8700, 62),
    (0x007f12, 64),
    (0x1853b2, 66),
    (0x624bad, 68),
    (0x733a67, 70),
    (0xf8bcaf, 72),
    (0xff9b76, 74),
    (0xffbf5f, 76),
    (0xd9af71, 78),
    (0xfff480, 80),
    (0xbfba69, 80),
    (0xbccc88, 81),
    (0xaeff99, 81),
    (0x7cdd9f, 82),
    (0x89b47d, 82),
    (0x80f3ff, 83),
    (0x7acefc, 83),
    (0x68a1d3, 84),
    (0x858fc2, 85),
    (0xbbaaf2, 85),
    (0xcdbbe4, 86),
    (0xef8bb0, 86),
    (0x859d8c, 87),
    (0x6b756e, 87),
    (0x84909b, 88),
    (0x6a7075, 88),
    (0x88859d, 89),
    (0x6c6a75, 90),
    (0x9d859c, 90),
    (0x746a74, 91),
    (0x9c9d85, 91),
    (0x74756a, 92),
    (0x9d8484, 92),
    (0x756a6a, 93),
    (0x661914, 93),
    (0x210806, 94),
    (0x460300, 94),
    (0x280000, 95),
    (0x5d1700, 96),
    (0x200d00, 96),
    (0x470c00, 97),
    (0x1c0800, 97),
    (0x3b2b14, 98),
    (0x1c130a, 98),
    (0x250e05, 99),
    (0x0d0602, 99),
    (0x645817, 100),
    (0x201c07, 101),
    (0x664e08, 101),
    (0x211902, 102),
    (0x486605, 102),
    (0x172101, 103),
    (0x306609, 103),
    (0x0f2103, 104),
    (0x144d08, 104),
    (0x061902, 105),
    (0x1f3701, 106),
    (0x0a1100, 106),
    (0x276622, 107),
    (0x0c210b, 107),
    (0x143e29, 108),
    (0x081910, 108),
    (0x004d36, 109),
    (0x00180e, 109),
    (0x134566, 110),
    (0x061621, 110),
    (0x152764, 111),
    (0x070c20, 112),
    (0x0a1466, 112),
    (0x030621, 113),
    (0x0b045c, 113),
    (0x03011d, 114),
    (0x0a1c4c, 114),
    (0x040b1e, 115),
    (0x161666, 115),
    (0x070721, 116),
    (0x220d66, 117),
    (0x0b0421, 117),
    (0x3c1166, 118),
    (0x130521, 118),
    (0x350d30, 119),
    (0x11040f, 119),
    (0x660614, 120),
    (0x210206, 120),
    (0x661154, 121),
    (0x21051b, 122),
    (0x000000, 122),
    (0x595959, 123),
    (0x1a1a1a, 123),
    (0xffffff, 124),
    (0x595959, 124),
    (0xcccccc, 125),
    (0x404040, 125),
    (0x141414, 126),
    (0x0000ff, 126),
    (0x00ff00, 127),
    (0xff0000, 127),
];

/// Whether a normalised port name is a Push 2's User port.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    const NAME: &str = "ableton push 2";
    if model.protocol != Protocol::AbletonPush2Midi {
        return false;
    }
    let lower = name.base.to_ascii_lowercase();
    let Some(at) = lower.find(NAME) else {
        return false;
    };
    let rest = lower.get(at + NAME.len()..).unwrap_or("");
    if rest.starts_with(|next: char| next.is_ascii_digit()) {
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

fn is_push2(model: &DeviceModel) -> bool {
    model.protocol == Protocol::AbletonPush2Midi
}

/// The legend on each key: the pads by row and column, then the buttons.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    is_push2(model).then(|| {
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
    if !is_push2(model) {
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

/// Whether a key's light is RGB: every pad and 30 of the buttons.
fn is_rgb(key: u8) -> bool {
    key < PADS || button_cc(key).is_some_and(|cc| RGB_BUTTON_CCS.contains(&cc))
}

fn dial_of_cc(cc: u8) -> Option<u8> {
    match cc {
        TEMPO_CC => Some(0),
        SWING_CC => Some(1),
        TRACK_CC..=MASTER_CC => Some(2 + cc - TRACK_CC),
        _ => None,
    }
}

/// A two's-complement relative value as a step.
fn relative(value: u8) -> i8 {
    let value = i16::from(value & 0x7f);
    let signed = if value < 64 { value } else { value - 128 };
    i8::try_from(signed).unwrap_or(0)
}

/// One of Push 2's SysEx commands.
fn command(id: u8, args: &[u8]) -> Vec<u8> {
    let mut message = HEADER.to_vec();
    message.push(id);
    message.extend(args.iter().map(|arg| arg & 0x7f));
    message.push(SYSEX_END);
    message
}

/// The arguments of a Push 2 SysEx reply to `id`.
fn reply(message: &[u8], id: u8) -> Option<&[u8]> {
    let rest = message.strip_prefix(&HEADER)?;
    let rest = rest.strip_prefix(&[id])?;
    rest.strip_suffix(&[SYSEX_END])
}

/// The mode a MIDI mode report names.
fn mode_reply(message: &[u8]) -> Option<u8> {
    reply(message, SET_MODE)?.first().copied()
}

/// What the device says it is, from the universal identity reply.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    major: u8,
    minor: u8,
    build: u16,
    serial: u32,
    board: u8,
}

fn identity_reply(message: &[u8]) -> Option<Identity> {
    let start = message.get(..5)?;
    if start[0] != SYSEX_START || start[1] != 0x7e || start[3] != 0x06 || start[4] != 0x02 {
        return None;
    }
    if message.get(5..12)? != IDENTITY {
        return None;
    }
    let serial = message
        .get(16..21)?
        .iter()
        .rev()
        .fold(0u32, |serial, byte| (serial << 7) | u32::from(byte & 0x7f));
    Some(Identity {
        major: *message.get(12)?,
        minor: *message.get(13)?,
        build: u16::from(*message.get(14)? & 0x7f) | (u16::from(*message.get(15)? & 0x7f) << 7),
        serial,
        board: *message.get(21)?,
    })
}

/// A palette entry from a `0x04` reply: its index, colour and white level,
/// each value sent as 7 low bits and then the 8th bit.
fn palette_reply(message: &[u8]) -> Option<(u8, [u8; 3], u8)> {
    let args = reply(message, GET_PALETTE)?;
    if args.len() != 9 {
        return None;
    }
    let value = |at: usize| (args[at] & 0x7f) | ((args[at + 1] & 1) << 7);
    Some((args[0], [value(1), value(3), value(5)], value(7)))
}

/// The colour each palette index shows on an RGB light, and the level it
/// shows on a white one.
#[derive(Debug, Clone)]
struct Palette {
    rgb: [[u8; 3]; 128],
    white: [u8; 128],
}

impl Palette {
    fn fallback() -> Self {
        let mut palette = Self {
            rgb: [BLACK; 128],
            white: [0; 128],
        };
        for (at, (rgb, white)) in FALLBACK_PALETTE.iter().enumerate() {
            let [_, red, green, blue] = rgb.to_be_bytes();
            palette.rgb[at] = [red, green, blue];
            palette.white[at] = *white;
        }
        palette
    }

    /// The index whose colour is nearest, on an RGB light.
    fn nearest_rgb(&self, color: [u8; 3]) -> u8 {
        if color.iter().all(|channel| *channel < DARK) {
            return OFF;
        }
        let distance = |entry: &[u8; 3]| -> u32 {
            entry
                .iter()
                .zip(color)
                .map(|(a, b)| {
                    let d = i32::from(*a) - i32::from(b);
                    d.unsigned_abs() * d.unsigned_abs()
                })
                .sum()
        };
        (1..self.rgb.len())
            .filter(|at| *at != SECOND_BLACK)
            .min_by_key(|at| distance(&self.rgb[*at]))
            .and_then(|at| u8::try_from(at).ok())
            .unwrap_or(OFF)
    }

    /// The index whose white level is nearest the colour's luma, on a white
    /// light.
    fn nearest_white(&self, color: [u8; 3]) -> u8 {
        let [red, green, blue] = color.map(u32::from);
        let luma = (299 * red + 587 * green + 114 * blue) / 1000;
        if luma < u32::from(DARK) {
            return OFF;
        }
        let target = i32::try_from(luma * 128 / 255).unwrap_or(128);
        (0..self.white.len())
            .min_by_key(|at| (i32::from(self.white[*at]) - target).unsigned_abs())
            .and_then(|at| u8::try_from(at).ok())
            .unwrap_or(OFF)
    }
}

/// The touch strip, read as a dial.
#[derive(Debug, Default)]
struct Strip {
    last: Option<i16>,
    rest: i16,
}

pub struct Push2 {
    transport: Box<dyn Transport>,
    /// User mode set, the palette read and the strip set up.
    started: bool,
    /// Live took the device back: input is dropped until User mode returns.
    in_live: bool,
    palette: Palette,
    colors: Vec<[u8; 3]>,
    /// The index each key's light was last sent; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    /// The brightness to send (0-127), once one is set.
    brightness: Option<u8>,
    strip: Strip,
    parser: Parser,
    /// Messages that arrived during the start-up, for the first poll.
    pending: Vec<Message>,
}

impl Push2 {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            in_live: false,
            palette: Palette::fallback(),
            colors: vec![BLACK; usize::from(KEYS)],
            sent: vec![None; usize::from(KEYS)],
            held: vec![false; usize::from(KEYS)],
            brightness: None,
            strip: Strip::default(),
            parser: Parser::default(),
            pending: Vec::new(),
        }
    }

    /// Waits up to `wait` for a SysEx message `pick` accepts. Everything
    /// else read meanwhile is kept for the next poll.
    fn await_reply<T>(
        &mut self,
        wait: Duration,
        pick: impl Fn(&[u8]) -> Option<T>,
    ) -> DriverResult<Option<T>> {
        let start = Instant::now();
        let mut buf = [0u8; READ_LEN];
        let mut messages = Vec::new();
        for _ in 0..MAX_HANDSHAKE_MESSAGES {
            let left = wait.saturating_sub(start.elapsed());
            if left.is_zero() {
                break;
            }
            let wait_ms = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
            let read = self.transport.read_timeout(&mut buf, wait_ms)?;
            if read == 0 {
                break;
            }
            messages.clear();
            self.parser
                .feed(buf.get(..read).unwrap_or_default(), &mut messages);
            let mut found = None;
            for message in messages.drain(..) {
                if found.is_none() {
                    if let Message::SysEx(bytes) = &message {
                        if let Some(value) = pick(bytes) {
                            found = Some(value);
                            continue;
                        }
                    }
                }
                self.pending.push(message);
            }
            if found.is_some() {
                return Ok(found);
            }
        }
        Ok(None)
    }

    /// Reads the palette back, or keeps Live's when the first entry does not
    /// answer. An entry that does not answer later keeps Live's colour.
    fn read_palette(&mut self) -> DriverResult<()> {
        for index in 0..128u8 {
            self.transport.write(&command(GET_PALETTE, &[index]))?;
            let entry = self.await_reply(PALETTE_WAIT, |message| {
                palette_reply(message).filter(|(at, _, _)| *at == index)
            })?;
            let Some((_, rgb, white)) = entry else {
                if index == 0 {
                    tracing::debug!(
                        "ryu-decks: Push 2 did not answer a palette read; using Live's palette"
                    );
                    return Ok(());
                }
                continue;
            };
            let at = usize::from(index);
            if let (Some(slot), Some(level)) =
                (self.palette.rgb.get_mut(at), self.palette.white.get_mut(at))
            {
                *slot = rgb;
                *level = white;
            }
        }
        Ok(())
    }

    /// The start-up, then every light and the brightness.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if self.started {
            return Ok(());
        }
        self.transport.write(&IDENTITY_REQUEST)?;
        match self.await_reply(IDENTITY_WAIT, identity_reply)? {
            Some(identity) => tracing::debug!(
                firmware = %format!("{}.{}.{}", identity.major, identity.minor, identity.build),
                serial = identity.serial,
                board = identity.board,
                "ryu-decks: Push 2 identified"
            ),
            None => tracing::debug!("ryu-decks: Push 2 did not answer its identity request"),
        }
        self.transport.write(&command(SET_MODE, &[USER_MODE]))?;
        let mode = self.await_reply(MODE_WAIT, |message| {
            mode_reply(message).filter(|mode| *mode != LIVE_MODE)
        })?;
        if mode.is_none() {
            tracing::debug!("ryu-decks: Push 2 did not confirm User mode");
        }
        self.read_palette()?;
        self.transport.write(&command(SET_STRIP, &[STRIP_RYU]))?;
        self.started = true;
        self.in_live = false;
        self.repaint()
    }

    /// Sends every light and the brightness again.
    fn repaint(&mut self) -> DriverResult<()> {
        self.sent.fill(None);
        self.write_lights()?;
        self.write_brightness()
    }

    fn write_brightness(&mut self) -> DriverResult<()> {
        if let Some(level) = self.brightness {
            self.transport.write(&command(SET_BRIGHTNESS, &[level]))?;
        }
        Ok(())
    }

    /// The message that sets a key's light to a palette index.
    fn light(key: u8, index: u8) -> Option<[u8; 3]> {
        if key < PADS {
            return Some([NOTE_ON, pad_note(key), index]);
        }
        button_cc(key).map(|cc| [CONTROL_CHANGE, cc, index])
    }

    fn index_for(&self, key: u8) -> u8 {
        let color = self.colors.get(usize::from(key)).copied().unwrap_or(BLACK);
        if is_rgb(key) {
            self.palette.nearest_rgb(color)
        } else {
            self.palette.nearest_white(color)
        }
    }

    /// One message for each light whose index changed.
    fn write_lights(&mut self) -> DriverResult<()> {
        for key in 0..KEYS {
            let index = self.index_for(key);
            let Some(message) = Self::light(key, index) else {
                continue;
            };
            let Some(sent) = self.sent.get_mut(usize::from(key)) else {
                continue;
            };
            if *sent != Some(index) {
                self.transport.write(&message)?;
                *sent = Some(index);
            }
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

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> DriverResult<()> {
        let [status, number, value] = match message {
            Message::SysEx(bytes) => {
                match mode_reply(bytes) {
                    Some(LIVE_MODE) => {
                        if !self.in_live {
                            tracing::debug!("ryu-decks: Live took the Push 2 back");
                            self.in_live = true;
                            self.release_all(events);
                        }
                    }
                    Some(_) if self.started => {
                        self.in_live = false;
                        self.repaint()?;
                    }
                    _ => {}
                }
                return Ok(());
            }
            Message::Channel(bytes) => *bytes,
        };
        if self.in_live {
            return Ok(());
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
        Ok(())
    }
}

impl Driver for Push2 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_started()?;
        let mut messages = std::mem::take(&mut self.pending);
        let mut buf = [0u8; READ_LEN];
        for attempt in 0..MAX_MESSAGES_PER_POLL {
            let wait = if attempt == 0 && messages.is_empty() {
                timeout_ms
            } else {
                0
            };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.parser
                .feed(buf.get(..read).unwrap_or_default(), &mut messages);
        }
        let mut events = Vec::new();
        for message in &messages {
            self.receive(message, &mut events)?;
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        self.write_lights()
    }

    /// The global LED brightness, 0-127; the firmware holds it at 8 or
    /// under on USB power alone.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        let level = (u16::from(percent.min(100)) * 127 + 50) / 100;
        self.brightness = u8::try_from(level).ok();
        if self.started {
            self.write_brightness()?;
        }
        Ok(())
    }
}

impl Drop for Push2 {
    /// Turns every light off, restores the strip's default and hands the
    /// device back in Live mode. Write errors (an unplugged deck) are ignored.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for key in 0..KEYS {
            if let Some(message) = Self::light(key, OFF) {
                let _ = self.transport.write(&message);
            }
        }
        let _ = self.transport.write(&command(SET_STRIP, &[STRIP_DEFAULT]));
        let _ = self.transport.write(&command(SET_MODE, &[LIVE_MODE]));
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, ModelView};

    const IDENTITY_ANSWER: [u8; 23] = [
        0xf0, 0x7e, 0x01, 0x06, 0x02, 0x00, 0x21, 0x1d, 0x67, 0x32, 0x02, 0x00, 0x01, 0x00, 0x2f,
        0x00, 0x73, 0x4d, 0x1f, 0x08, 0x00, 0x01, 0xf7,
    ];
    const USER_ECHO: [u8; 9] = [0xf0, 0x00, 0x21, 0x1d, 0x01, 0x01, 0x0a, 0x01, 0xf7];
    const LIVE_ECHO: [u8; 9] = [0xf0, 0x00, 0x21, 0x1d, 0x01, 0x01, 0x0a, 0x00, 0xf7];

    fn open() -> (Push2, Recorder) {
        let recorder = Recorder::default();
        let deck = Push2::new(by_id(PUSH_2_ID).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// An opened deck past its start-up (Live's palette), with what that
    /// sent cleared.
    fn started() -> (Push2, Recorder) {
        let (mut deck, recorder) = open();
        recorder.queue_read(IDENTITY_ANSWER.to_vec());
        recorder.queue_read(USER_ECHO.to_vec());
        deck.flush().unwrap();
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

    fn feed(deck: &mut Push2, recorder: &Recorder, messages: &[&[u8]]) -> Vec<Input> {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
        deck.poll(0).unwrap()
    }

    fn palette_answer(index: u8, rgb: [u8; 3], white: u8) -> Vec<u8> {
        let mut args = vec![index];
        for value in [rgb[0], rgb[1], rgb[2], white] {
            args.push(value & 0x7f);
            args.push(value >> 7);
        }
        command(GET_PALETTE, &args)
    }

    #[test]
    fn the_start_up_sets_user_mode_reads_the_palette_and_sets_the_strip() {
        let (mut deck, recorder) = open();
        recorder.queue_read(IDENTITY_ANSWER.to_vec());
        // A pad press during the start-up is kept for the first poll.
        recorder.queue_read(vec![0x90, 92, 0x40]);
        recorder.queue_read(USER_ECHO.to_vec());
        for index in 0..128u8 {
            // Index 5 reads as pure blue on this device.
            let rgb = if index == 5 {
                [0, 0, 255]
            } else {
                [10, 10, 10]
            };
            recorder.queue_read(palette_answer(index, rgb, index));
        }
        deck.set_face(0, Some(&lit([0, 0, 250]))).unwrap();
        deck.set_brightness(100).unwrap();
        assert_eq!(deck.poll(0).unwrap(), vec![key(0, true)]);
        let writes = recorder.writes();
        assert_eq!(writes[0], IDENTITY_REQUEST.to_vec());
        assert_eq!(writes[1], USER_ECHO.to_vec());
        assert_eq!(
            writes[2],
            vec![0xf0, 0x00, 0x21, 0x1d, 0x01, 0x01, 0x04, 0x00, 0xf7]
        );
        assert_eq!(writes[129][7], 127);
        assert_eq!(
            writes[130],
            vec![0xf0, 0x00, 0x21, 0x1d, 0x01, 0x01, 0x17, 0x08, 0xf7]
        );
        // Then all 128 lights: the top-left pad takes the device's blue.
        assert_eq!(writes[131], vec![0x90, 92, 5]);
        assert_eq!(writes[132], vec![0x90, 93, 0]);
        assert_eq!(writes[131 + 64], vec![0xb0, 3, 0]);
        assert_eq!(writes.len(), 131 + 128 + 1);
        assert_eq!(
            writes[131 + 128],
            vec![0xf0, 0x00, 0x21, 0x1d, 0x01, 0x01, 0x06, 127, 0xf7]
        );
        assert_eq!(
            identity_reply(&IDENTITY_ANSWER),
            Some(Identity {
                major: 1,
                minor: 0,
                build: 47,
                serial: 17_295_091,
                board: 1,
            })
        );
    }

    #[test]
    fn a_silent_palette_falls_back_to_lives_at_once() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        let writes = recorder.writes();
        // Identity, mode, one palette read, the strip, then the lights.
        assert_eq!(writes[2], command(GET_PALETTE, &[0]));
        assert_eq!(writes[3], command(SET_STRIP, &[STRIP_RYU]));
        assert_eq!(writes.len(), 4 + 128);
        let palette = Palette::fallback();
        assert_eq!(palette.rgb[1], [0xff, 0x40, 0x32]);
        assert_eq!(palette.white[1], 2);
        assert_eq!(palette.rgb[122], [0xcc, 0xcc, 0xcc]);
        assert_eq!(palette.rgb[127], [0xff, 0, 0]);
        assert_eq!(palette.white[127], 127);
        // Malformed replies are not palette entries or identities.
        assert_eq!(palette_reply(&command(GET_PALETTE, &[1, 2])), None);
        assert_eq!(identity_reply(&IDENTITY_ANSWER[..20]), None);
        assert_eq!(identity_reply(&[0xf0]), None);
        assert_eq!(mode_reply(&[0xf0, 0x00]), None);
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
                &[0xb0, 60, 0x7f],
                &[0xb0, USER_CC, 0x7f],
                &[0xb0, 64, 0x7f],
                &[0xb0, 69, 0x7f],
                &[0xb0, 1, 0x40],
            ],
        );
        assert_eq!(
            events,
            vec![
                key(64, true),
                key(64, false),
                key(127, true),
                key(102, true)
            ]
        );
        assert_eq!(BUTTON_CCS.len(), 64);
        assert!(!BUTTON_CCS.contains(&USER_CC));
        assert_eq!((0..KEYS).filter(|key| is_rgb(*key)).count(), 94);
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
                &[0x90, STRIP_TOUCH_NOTE, 0x7f],
                &[0x90, STRIP_TOUCH_NOTE, 0x00],
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
                // A new touch starts again from where it lands.
                &[0x90, STRIP_TOUCH_NOTE, 0x7f],
                &bend(200),
                &bend(207),
            ],
        );
        assert_eq!(events, vec![turn(11, 2), turn(11, -3)]);
    }

    #[test]
    fn lights_take_the_nearest_palette_index_on_channel_1() {
        let (mut deck, recorder) = started();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(63, Some(&lit([0, 0, 255]))).unwrap();
        // Lower 1 is RGB, Tap Tempo white.
        deck.set_face(66, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(64, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(65, Some(&lit([60, 60, 60]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x90, 92, 127],
                vec![0x90, 43, 125],
                vec![0xb0, 3, 126],
                vec![0xb0, 9, 15],
                vec![0xb0, 20, 126],
            ]
        );
        // Nothing changed: nothing sent.
        recorder.0.lock().unwrap().writes.clear();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes(), vec![vec![0x90, 92, 0]]);
        // Index 117 (a second black) is never picked.
        let palette = Palette::fallback();
        assert_ne!(palette.nearest_rgb([9, 9, 9]), 117);
    }

    #[test]
    fn brightness_rounds_to_0_127() {
        let (mut deck, recorder) = started();
        for (percent, level) in [(0, 0), (50, 64), (100, 127), (200, 127), (1, 1)] {
            deck.set_brightness(percent).unwrap();
            assert_eq!(
                recorder.writes().last().unwrap(),
                &command(SET_BRIGHTNESS, &[level])
            );
        }
    }

    #[test]
    fn dropping_turns_the_lights_off_and_hands_back_live_mode() {
        let (mut deck, recorder) = started();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 128 + 2);
        assert_eq!(writes[0], vec![0x90, 92, 0]);
        assert_eq!(writes[127], vec![0xb0, 119, 0]);
        assert_eq!(
            writes[128],
            vec![0xf0, 0x00, 0x21, 0x1d, 0x01, 0x01, 0x17, 0x68, 0xf7]
        );
        assert_eq!(writes[129], LIVE_ECHO.to_vec());
        // A deck that never started sends nothing.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn live_taking_the_deck_back_mutes_input_until_user_mode_returns() {
        let (mut deck, recorder) = started();
        deck.set_brightness(50).unwrap();
        let events = feed(&mut deck, &recorder, &[&[0x90, 92, 0x7f], &LIVE_ECHO]);
        assert_eq!(events, vec![key(0, true), key(0, false)]);
        recorder.0.lock().unwrap().writes.clear();
        let events = feed(&mut deck, &recorder, &[&[0x90, 92, 0x7f], &[0xb0, 14, 1]]);
        assert!(events.is_empty());
        // Ryu never switches the mode back itself.
        assert!(recorder.writes().is_empty());
        let events = feed(&mut deck, &recorder, &[&USER_ECHO, &[0x90, 92, 0x7f]]);
        assert_eq!(events, vec![key(0, true)]);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 128 + 1);
        assert_eq!(writes[128], command(SET_BRIGHTNESS, &[64]));
    }

    #[test]
    fn only_the_user_port_matches() {
        let model = by_id(PUSH_2_ID).unwrap();
        let cases = [
            ("Ableton Push 2 User Port", false, true),
            ("Ableton Push 2 Live Port", false, false),
            ("Ableton Push 2:Ableton Push 2 MIDI 2 24:1", false, true),
            ("Ableton Push 2:Ableton Push 2 MIDI 1 24:0", false, false),
            ("Ableton Push 2:Ableton Push 2 User Port 28:1", false, true),
            ("MIDIIN2 (Ableton Push 2)", true, true),
            ("MIDIOUT2 (Ableton Push 2) 1", true, true),
            ("MIDIIN2 (2- Ableton Push 2)", true, true),
            ("Ableton Push 2", true, false),
            ("Ableton Push 2 1", true, false),
            ("Ableton Push 3 User Port", false, false),
            ("Ableton Push User Port", false, false),
            ("MIDIIN2 (Ableton Push)", true, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            // The first Push's User port is that model's, never this one's.
            assert_eq!(
                identify_midi(&name, windows)
                    .map(|model| model.id)
                    .filter(|id| *id != super::super::push1::PUSH_1_ID),
                expected.then_some(PUSH_2_ID),
                "{raw}"
            );
        }
    }

    #[test]
    fn every_key_has_a_name_and_its_own_cell() {
        let model = by_id(PUSH_2_ID).unwrap();
        let view = ModelView::from(model);
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), usize::from(KEYS));
        assert_eq!(names[0], "Pad 1,1");
        assert_eq!(names[63], "Pad 8,8");
        assert_eq!(names[64], "Tap Tempo");
        assert_eq!(names[127], "Undo");
        let cells = view.key_cells.unwrap();
        assert_eq!(cells.len(), usize::from(KEYS));
        let unique: std::collections::HashSet<_> = cells.iter().collect();
        assert_eq!(unique.len(), cells.len());
        let grid = u16::from(model.rows) * u16::from(model.cols);
        assert!(cells.iter().all(|cell| *cell < grid));
        // The top-left pad, Play bottom left, Select bottom right.
        assert_eq!(cells[0], 4 * 14 + 3);
        assert_eq!(cells[64 + 42], 12 * 14);
        assert_eq!(cells[64 + 27], 12 * 14 + 13);
        assert_eq!(view.key_light_colors, "rgb");
        assert!(view.lit_keys.is_none());
        assert!(view.brightness && view.key_lights);
    }
}
