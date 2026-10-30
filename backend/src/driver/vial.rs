//! QMK keyboards and macropads running Vial firmware, over the VIA / Vial
//! raw-HID protocol.
//!
//! Vial boards share no USB ids: any vendor and product id can carry the
//! firmware. They are found by the serial-number string Vial builds put in every
//! board (`vial:f64c2b3c`) and the raw-HID collection QMK exposes (usage page
//! 0xFF60, usage 0x61). Reports are 32 bytes with no report id, so a write is a
//! 0x00 placeholder and the payload. The firmware answers every packet exactly
//! once, with the same buffer filled in, so Ryu keeps one request in flight and
//! reads each answer before the next write. Nothing arrives unasked: keys are
//! read by polling.
//!
//! The board describes itself. A handshake reads the VIA protocol version
//! (`01`), the Vial keyboard id (`fe 00`: Vial protocol, an 8-byte keyboard UID
//! and whether VialRGB is built in), then the layout definition (`fe 01` for its
//! size, `fe 02` for each 32-byte page): JSON compressed as an XZ stream. Its
//! matrix size and KLE key map (legend `row,col`, after KLE's label reordering)
//! give the keys; encoders are not in the switch matrix and cannot be read here,
//! so a Vial deck has no dials. A model is built from that at run time.
//!
//! Keys come from the switch-matrix state (`02 03`), one bit per switch, each
//! row's bytes most significant first. Vial answers it only once the board is
//! unlocked: the user holds the board's unlock keys (listed by `fe 05`) for
//! about ten seconds while the host polls `fe 07` every 200 ms after `fe 06`.
//! An unlock cannot be cancelled, and while one runs the firmware ignores every
//! lighting and matrix command, so Ryu starts one only when the user asks, and
//! defers lights until it ends.
//!
//! Lights go through VialRGB's direct mode, by `07`/`08` lighting commands:
//! per-LED HSV (hue 0-255 for the whole circle) in runs of up to 9 LEDs. LEDs
//! on a key light that key; the rest (underglow, indicators) are the deck light.
//! The direct effect ignores the board's own brightness, so Ryu scales V itself.
//! Direct mode is entered only while something is lit, without touching EEPROM,
//! and the board's own effect comes back when nothing is or Ryu lets go.
//!
//! Facts per vial-qmk (quantum/via.c, vial.c, vialrgb.c and the raw-HID USB
//! descriptor) and vial-gui (its HID discovery, keyboard_comm.py, kle_serial.py,
//! matrix_test.py and unlocker.py), both GPL-2.0-or-later; no code was copied.

use std::io::Read as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{Driver, DriverError, DriverResult, Face, Input, LockState, Transport};
use crate::catalog::{DeviceModel, DialLights, Family, Protocol, StripPlacement};

/// In the USB serial-number string of every Vial build.
pub const SERIAL_MAGIC: &str = "vial:f64c2b3c";
/// QMK's raw-HID collection (`RAW_USAGE_PAGE`, `RAW_USAGE_ID`).
pub const RAW_USAGE_PAGE: u16 = 0xff60;
pub const RAW_USAGE: u16 = 0x61;

/// The raw-HID payload length, both ways (QMK's `RAW_EPSIZE`).
const PAYLOAD_LEN: usize = 32;
/// A write: the report-id placeholder hidapi expects, then the payload.
const WRITE_LEN: usize = PAYLOAD_LEN + 1;
const REPLY_TIMEOUT: Duration = Duration::from_millis(500);
/// Answers to someone else's request (the Vial app holding the same
/// collection) skipped before giving up on ours.
const STRAY_REPLIES: usize = 8;

/// The VIA protocol Vial speaks.
const VIA_PROTOCOL: u16 = 0x0009;
/// Matrix unlocking arrived in Vial protocol 3; 6 is current.
const VIAL_PROTOCOLS: std::ops::RangeInclusive<u32> = 3..=6;
const MAX_DEFINITION_BYTES: u32 = 64 * 1024;
const MAX_JSON_BYTES: u64 = 1024 * 1024;
/// The largest XZ dictionary accepted (vial-qmk's definitions use 8 MiB).
const MAX_DICTIONARY_BYTES: u64 = 64 << 20;
const MAX_LEDS: u16 = 1024;
/// LEDs per direct-mode packet: 5 bytes of header, 3 per LED.
const FASTSET_LEDS: usize = 9;
const UNLOCK_COUNTER: u8 = 50;
/// The firmware counts a poll only when over 100 ms have passed since the last.
const UNLOCK_POLL: Duration = Duration::from_millis(200);
const MATRIX_POLL: Duration = Duration::from_millis(20);
/// The firmware fills the matrix reply only when `(cols / 8 + 1) * rows` is at
/// most this.
const MATRIX_REPLY_BYTES: usize = 28;
const MAX_UNLOCK_KEYS: usize = 15;

const ID_GET_PROTOCOL_VERSION: u8 = 0x01;
const ID_GET_KEYBOARD_VALUE: u8 = 0x02;
const ID_SWITCH_MATRIX_STATE: u8 = 0x03;
const ID_LIGHTING_SET_VALUE: u8 = 0x07;
const ID_LIGHTING_GET_VALUE: u8 = 0x08;
const ID_UNHANDLED: u8 = 0xff;
const ID_VIAL: u8 = 0xfe;

const VIAL_GET_KEYBOARD_ID: u8 = 0x00;
const VIAL_GET_SIZE: u8 = 0x01;
const VIAL_GET_DEFINITION: u8 = 0x02;
const VIAL_GET_UNLOCK_STATUS: u8 = 0x05;
const VIAL_UNLOCK_START: u8 = 0x06;
const VIAL_UNLOCK_POLL: u8 = 0x07;

const VIALRGB_GET_INFO: u8 = 0x40;
const VIALRGB_GET_MODE: u8 = 0x41;
const VIALRGB_GET_SUPPORTED: u8 = 0x42;
const VIALRGB_GET_NUMBER_LEDS: u8 = 0x43;
const VIALRGB_GET_LED_INFO: u8 = 0x44;
const VIALRGB_SET_MODE: u8 = 0x41;
const VIALRGB_DIRECT_FASTSET: u8 = 0x42;
const VIALRGB_PROTOCOL: u16 = 1;
const VIALRGB_EFFECT_DIRECT: u16 = 1;
const VIALRGB_EFFECT_SOLID_COLOR: u16 = 2;

/// Where KLE moves each newline-separated legend, per alignment `a` (0 to 7):
/// the legend at raw position `i` becomes label `LABEL_MAP[a][i]`, none if -1.
/// Vial reads the matrix position from label 0, `e` (an encoder) from label 4
/// and the layout option from label 8.
const LABEL_MAP: [[i8; 12]; 8] = [
    [0, 6, 2, 8, 9, 11, 3, 5, 1, 4, 7, 10],
    [1, 7, -1, -1, 9, 11, 4, -1, -1, -1, -1, 10],
    [3, -1, 5, -1, 9, 11, -1, -1, 4, -1, -1, 10],
    [4, -1, -1, -1, 9, 11, -1, -1, -1, -1, -1, 10],
    [0, 6, 2, 8, 10, -1, 3, 5, 1, 4, 7, -1],
    [1, 7, -1, -1, 10, -1, 4, -1, -1, -1, -1, -1],
    [3, -1, 5, -1, 10, -1, -1, -1, 4, -1, -1, -1],
    [4, -1, -1, -1, 10, -1, -1, -1, -1, -1, -1, -1],
];
const DEFAULT_ALIGN: usize = 4;
/// A drawn grid any larger than this falls back to the switch matrix.
const MAX_GRID_COLS: i64 = 32;
const MAX_GRID_ROWS: i64 = 16;

const BLACK: [u8; 3] = [0, 0, 0];

/// Whether a HID collection is a Vial board's raw-HID interface.
#[must_use]
pub fn is_vial_collection(serial: Option<&str>, usage_page: u16, usage: u16) -> bool {
    serial.is_some_and(|serial| serial.contains(SERIAL_MAGIC))
        && usage_page == RAW_USAGE_PAGE
        && usage == RAW_USAGE
}

/// What a Vial board told the handshake about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    /// The keyboard UID (`VIAL_KEYBOARD_UID`): per firmware, not per unit.
    pub uid: u64,
    pub rows: u8,
    pub cols: u8,
    /// Each Ryu key's switch-matrix position, in reading order.
    pub keys: Vec<(u8, u8)>,
    /// The rows and columns the keys are drawn on, and each key's cell.
    pub grid: (u8, u8),
    pub cells: Vec<u16>,
    /// The switch-matrix positions held down to unlock the board.
    pub unlock_keys: Vec<(u8, u8)>,
    pub rgb: Option<Rgb>,
}

/// A board's VialRGB lights, on a build with the direct effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgb {
    /// `RGB_MATRIX_MAXIMUM_BRIGHTNESS`: V is clamped to it.
    pub max_value: u8,
    /// Per LED: the Ryu key it lights, or `None` for a deck light.
    pub leds: Vec<Option<u8>>,
}

impl Board {
    fn key_at(&self, row: u8, col: u8) -> Option<u8> {
        self.keys
            .iter()
            .position(|key| *key == (row, col))
            .and_then(|index| u8::try_from(index).ok())
    }
}

/// Why a board whose unlock keys are not all on its drawn layout is unlocked
/// in the Vial app instead.
pub const UNLOCK_KEYS_OFF_LAYOUT: &str = "Some of this keyboard's unlock keys are not on its layout, so Ryu cannot show which keys to hold. Unlock it in the Vial app.";

fn refused(message: impl Into<String>) -> DriverError {
    DriverError::Refused(message.into())
}

fn u16_le(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([
        bytes.get(at).copied().unwrap_or(0),
        bytes.get(at + 1).copied().unwrap_or(0),
    ])
}

fn u32_le(bytes: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    for (offset, slot) in word.iter_mut().enumerate() {
        *slot = bytes.get(at + offset).copied().unwrap_or(0);
    }
    u32::from_le_bytes(word)
}

/// Sends one command and waits for its answer: the first reply that starts with
/// `echo` (a Vial `fe` command overwrites its first bytes, so it passes `&[]`).
fn exchange(
    transport: &mut dyn Transport,
    command: &[u8],
    echo: &[u8],
) -> DriverResult<[u8; PAYLOAD_LEN]> {
    exchange_with(transport, command, &|reply| reply.starts_with(echo))
}

/// [`exchange`], taking the first reply `accept` takes; others (answers to
/// another app on the same collection) are skipped.
fn exchange_with(
    transport: &mut dyn Transport,
    command: &[u8],
    accept: &dyn Fn(&[u8]) -> bool,
) -> DriverResult<[u8; PAYLOAD_LEN]> {
    let mut out = [0u8; WRITE_LEN];
    for (slot, byte) in out[1..].iter_mut().zip(command) {
        *slot = *byte;
    }
    transport.write(&out)?;
    let deadline = Instant::now() + REPLY_TIMEOUT;
    let mut buf = [0u8; 64];
    for _ in 0..=STRAY_REPLIES {
        let left = deadline.saturating_duration_since(Instant::now());
        let wait = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
        let read = transport.read_timeout(&mut buf, wait)?;
        if read == 0 {
            break;
        }
        if accept(&buf[..read]) {
            let mut reply = [0u8; PAYLOAD_LEN];
            let len = read.min(PAYLOAD_LEN);
            reply[..len].copy_from_slice(&buf[..len]);
            return Ok(reply);
        }
    }
    Err(DriverError::Io(format!(
        "the keyboard did not answer command {:02x}",
        command.first().copied().unwrap_or(0)
    )))
}

fn vial(transport: &mut dyn Transport, command: &[u8]) -> DriverResult<[u8; PAYLOAD_LEN]> {
    let mut packet = vec![ID_VIAL];
    packet.extend_from_slice(command);
    exchange(transport, &packet, &[])
}

/// The handshake: everything Ryu needs to know about a Vial board before it
/// can be shown as a deck. Refuses boards Ryu cannot drive.
pub fn probe(transport: &mut dyn Transport) -> DriverResult<Board> {
    // Answered as itself, or as unhandled by a board without VIA.
    let version = exchange_with(transport, &[ID_GET_PROTOCOL_VERSION], &|reply| {
        matches!(
            reply.first(),
            Some(&(ID_GET_PROTOCOL_VERSION | ID_UNHANDLED))
        )
    })?;
    if version[0] == ID_UNHANDLED {
        return Err(refused("This keyboard does not speak VIA."));
    }
    let via = u16::from_be_bytes([version[1], version[2]]);
    // Mid-unlock, the firmware skips even this command and its echo reads 0.
    if via == 0 && vial(transport, &[VIAL_GET_UNLOCK_STATUS])?[1] == 1 {
        return Err(unlock_in_progress());
    }
    if via != VIA_PROTOCOL {
        return Err(refused(format!(
            "This keyboard speaks VIA protocol {via}; Ryu speaks protocol 9, the one Vial uses."
        )));
    }
    let id = vial(transport, &[VIAL_GET_KEYBOARD_ID])?;
    let protocol = u32_le(&id, 0);
    if !VIAL_PROTOCOLS.contains(&protocol) {
        return Err(refused(format!(
            "This keyboard runs Vial protocol {protocol}; Ryu works with protocols 3 to 6. Update its Vial firmware."
        )));
    }
    let mut uid = [0u8; 8];
    uid.copy_from_slice(&id[4..12]);
    let uid = u64::from_le_bytes(uid);
    let has_rgb = id[12] & 1 == 1;

    let size = u32_le(&vial(transport, &[VIAL_GET_SIZE])?, 0);
    if size == 0 || size > MAX_DEFINITION_BYTES {
        return Err(refused(format!(
            "This keyboard reports a {size}-byte layout definition, which Ryu does not read."
        )));
    }
    let mut blob = Vec::with_capacity(size as usize);
    for page in 0..size.div_ceil(PAYLOAD_LEN as u32) {
        let [lo, hi, ..] = page.to_le_bytes();
        blob.extend_from_slice(&vial(transport, &[VIAL_GET_DEFINITION, lo, hi])?);
    }
    blob.truncate(size as usize);
    let definition = decode_definition(&blob).map_err(refused)?;
    let layout = Layout::parse(&definition).map_err(refused)?;

    let status = vial(transport, &[VIAL_GET_UNLOCK_STATUS])?;
    if status[1] == 1 {
        return Err(unlock_in_progress());
    }
    let unlock_keys = status[2..]
        .chunks_exact(2)
        .take(MAX_UNLOCK_KEYS)
        .filter(|pair| pair[0] != 0xff && pair[1] != 0xff)
        .map(|pair| (pair[0], pair[1]))
        .collect();
    let mut board = Board {
        uid,
        rows: layout.rows,
        cols: layout.cols,
        keys: layout.keys,
        grid: layout.grid,
        cells: layout.cells,
        unlock_keys,
        rgb: None,
    };
    if has_rgb {
        board.rgb = probe_rgb(transport, &board)?;
    }
    Ok(board)
}

fn unlock_in_progress() -> DriverError {
    refused("This keyboard is part-way through a Vial unlock and ignores everything else until it ends. Hold its unlock keys until it finishes, or unplug it and plug it back in.")
}

/// The board's VialRGB lights, if it has the direct effect.
fn probe_rgb(transport: &mut dyn Transport, board: &Board) -> DriverResult<Option<Rgb>> {
    let get = |transport: &mut dyn Transport, args: &[u8]| {
        let mut packet = vec![ID_LIGHTING_GET_VALUE];
        packet.extend_from_slice(args);
        exchange(transport, &packet, &packet[..2])
    };
    let info = get(transport, &[VIALRGB_GET_INFO])?;
    if u16_le(&info, 2) != VIALRGB_PROTOCOL {
        return Ok(None);
    }
    let max_value = info[4];
    let mut direct = false;
    let mut after = 0u16;
    // Up to 15 effect ids per page, each above the last page's highest.
    for _ in 0..64 {
        let [lo, hi] = after.to_le_bytes();
        let page = get(transport, &[VIALRGB_GET_SUPPORTED, lo, hi])?;
        let ids: Vec<u16> = page[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .filter(|id| *id != 0xffff)
            .collect();
        direct |= ids.contains(&VIALRGB_EFFECT_DIRECT);
        match ids.iter().max() {
            Some(highest) if *highest > after => after = *highest,
            _ => break,
        }
    }
    if !direct {
        return Ok(None);
    }
    let count = u16_le(&get(transport, &[VIALRGB_GET_NUMBER_LEDS])?, 2).min(MAX_LEDS);
    let mut leds = Vec::with_capacity(usize::from(count));
    for index in 0..count {
        // `08 44` decodes its index wrongly from 256 on, so later LEDs cannot
        // be asked where they sit; they become deck lights.
        let Ok(byte) = u8::try_from(index) else {
            leds.push(None);
            continue;
        };
        let info = get(transport, &[VIALRGB_GET_LED_INFO, byte, 0])?;
        let (row, col) = (info[5], info[6]);
        leds.push(if row == 0xff || col == 0xff {
            None
        } else {
            board.key_at(row, col)
        });
    }
    Ok(Some(Rgb { max_value, leds }))
}

/// The layout definition: XZ-compressed JSON.
fn decode_definition(blob: &[u8]) -> Result<Value, String> {
    let unreadable = "This keyboard's layout definition could not be read.";
    if !xz_dictionary_fits(blob) {
        return Err(unreadable.to_owned());
    }
    let mut json = Vec::new();
    lzma_rust2::XzReader::new(blob, false)
        .take(MAX_JSON_BYTES)
        .read_to_end(&mut json)
        .map_err(|_| unreadable.to_owned())?;
    serde_json::from_slice(&json).map_err(|_| unreadable.to_owned())
}

/// Whether the first block of an XZ stream asks for a dictionary Ryu is willing
/// to allocate. Anything that does not parse as a block is left to the decoder.
fn xz_dictionary_fits(blob: &[u8]) -> bool {
    fn varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..63).step_by(7) {
            let byte = *bytes.get(*at)?;
            *at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
    let parse = || -> Option<bool> {
        // A 12-byte stream header, then the block header's size and flags.
        let flags = *blob.get(13)?;
        let mut at = 14;
        if flags & 0x40 != 0 {
            varint(blob, &mut at)?;
        }
        if flags & 0x80 != 0 {
            varint(blob, &mut at)?;
        }
        for _ in 0..=(flags & 0x03) {
            let filter = varint(blob, &mut at)?;
            let len = usize::try_from(varint(blob, &mut at)?).ok()?;
            if filter == 0x21 {
                let bits = *blob.get(at)?;
                if bits > 40 {
                    return Some(false);
                }
                let size = (2 | u64::from(bits & 1)) << (bits / 2 + 11);
                return Some(size <= MAX_DICTIONARY_BYTES);
            }
            at += len;
        }
        Some(true)
    };
    parse().unwrap_or(true)
}

/// A board's keys and where they are drawn.
#[derive(Debug, PartialEq, Eq)]
struct Layout {
    rows: u8,
    cols: u8,
    keys: Vec<(u8, u8)>,
    grid: (u8, u8),
    cells: Vec<u16>,
}

/// One key from the KLE map: its matrix position and where it is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Placed {
    row: u8,
    col: u8,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    /// A layout option other than the first (a split backspace, say).
    alternate: bool,
}

impl Layout {
    fn parse(definition: &Value) -> Result<Self, String> {
        let dimension = |name: &str| {
            definition
                .pointer(&format!("/matrix/{name}"))
                .and_then(Value::as_u64)
                .and_then(|value| u8::try_from(value).ok())
                .filter(|value| (1..=32).contains(value))
        };
        let (Some(rows), Some(cols)) = (dimension("rows"), dimension("cols")) else {
            return Err("This keyboard's layout definition has no usable matrix size.".to_owned());
        };
        if (usize::from(cols) / 8 + 1) * usize::from(rows) > MATRIX_REPLY_BYTES {
            return Err(format!(
                "Vial cannot report the keys of a {rows} x {cols} switch matrix over raw HID, so Ryu would never see a press."
            ));
        }
        let placed = definition
            .pointer("/layouts/keymap")
            .and_then(Value::as_array)
            .map(|keymap| kle_keys(keymap))
            .unwrap_or_default();
        // The first layout option wins a matrix position several keys share.
        let mut kept: Vec<Placed> = Vec::new();
        for alternate in [false, true] {
            for key in placed.iter().filter(|key| key.alternate == alternate) {
                let inside = key.row < rows && key.col < cols;
                if inside
                    && !kept
                        .iter()
                        .any(|seen| (seen.row, seen.col) == (key.row, key.col))
                {
                    kept.push(*key);
                }
            }
        }
        if kept.is_empty() {
            let keys: Vec<(u8, u8)> = (0..rows)
                .flat_map(|row| (0..cols).map(move |col| (row, col)))
                .collect();
            let cells = (0..keys.len())
                .filter_map(|cell| u16::try_from(cell).ok())
                .collect();
            return Ok(Self {
                rows,
                cols,
                keys,
                grid: (rows, cols),
                cells,
            });
        }
        let (keys, grid, cells) = drawn_grid(&kept).unwrap_or_else(|| {
            let mut keys: Vec<(u8, u8)> = kept.iter().map(|key| (key.row, key.col)).collect();
            keys.sort_unstable();
            let cells = keys
                .iter()
                .map(|(row, col)| u16::from(*row) * u16::from(cols) + u16::from(*col))
                .collect();
            (keys, (rows, cols), cells)
        });
        Ok(Self {
            rows,
            cols,
            keys,
            grid,
            cells,
        })
    }
}

/// The keys laid out as on the board: each in the grid cell under its centre,
/// in reading order. `None` when two keys land in one cell or the grid is
/// unreasonably large.
#[allow(clippy::type_complexity)]
fn drawn_grid(kept: &[Placed]) -> Option<(Vec<(u8, u8)>, (u8, u8), Vec<u16>)> {
    #[allow(clippy::cast_possible_truncation)]
    let centre = |start: f64, size: f64| (start + size / 2.0).floor() as i64;
    let spots: Vec<(i64, i64)> = kept
        .iter()
        .map(|key| (centre(key.y, key.height), centre(key.x, key.width)))
        .collect();
    let top = spots.iter().map(|spot| spot.0).min()?;
    let left = spots.iter().map(|spot| spot.1).min()?;
    let rows = spots.iter().map(|spot| spot.0 - top).max()? + 1;
    let cols = spots.iter().map(|spot| spot.1 - left).max()? + 1;
    if rows > MAX_GRID_ROWS || cols > MAX_GRID_COLS {
        return None;
    }
    let mut ordered: Vec<(u16, (u8, u8))> = Vec::with_capacity(kept.len());
    for (key, (row, col)) in kept.iter().zip(&spots) {
        let cell = u16::try_from((row - top) * cols + (col - left)).ok()?;
        if ordered.iter().any(|(taken, _)| *taken == cell) {
            return None;
        }
        ordered.push((cell, (key.row, key.col)));
    }
    ordered.sort_unstable();
    let grid = (u8::try_from(rows).ok()?, u8::try_from(cols).ok()?);
    Some((
        ordered.iter().map(|(_, key)| *key).collect(),
        grid,
        ordered.iter().map(|(cell, _)| *cell).collect(),
    ))
}

/// Every switch in a KLE key map: keys whose label 0 is `row,col`, skipping
/// encoders (label 4 `e`) and decals.
fn kle_keys(keymap: &[Value]) -> Vec<Placed> {
    let mut keys = Vec::new();
    let mut align = DEFAULT_ALIGN;
    let (mut x, mut y) = (0.0, 0.0);
    let (mut cluster_x, mut cluster_y) = (0.0, 0.0);
    let (mut width, mut height) = (1.0, 1.0);
    let mut decal = false;
    for row in keymap {
        let Some(items) = row.as_array() else {
            continue;
        };
        for item in items {
            match item {
                Value::String(legend) => {
                    let labels = reorder(legend, align);
                    let encoder = labels[4] == Some("e");
                    let position = labels[0].and_then(pair);
                    if let (false, false, Some((row, col))) = (decal, encoder, position) {
                        if let (Ok(row), Ok(col)) = (u8::try_from(row), u8::try_from(col)) {
                            keys.push(Placed {
                                row,
                                col,
                                x,
                                y,
                                width,
                                height,
                                alternate: labels[8]
                                    .and_then(pair)
                                    .is_some_and(|(_, option)| option != 0),
                            });
                        }
                    }
                    x += width;
                    width = 1.0;
                    height = 1.0;
                    decal = false;
                }
                Value::Object(props) => {
                    let number = |name: &str| props.get(name).and_then(Value::as_f64);
                    if let Some(value) = number("rx") {
                        cluster_x = value;
                        x = cluster_x;
                        y = cluster_y;
                    }
                    if let Some(value) = number("ry") {
                        cluster_y = value;
                        x = cluster_x;
                        y = cluster_y;
                    }
                    if let Some(value) = props.get("a").and_then(Value::as_u64) {
                        align = usize::try_from(value)
                            .ok()
                            .filter(|value| *value < LABEL_MAP.len())
                            .unwrap_or(DEFAULT_ALIGN);
                    }
                    x += number("x").unwrap_or(0.0);
                    y += number("y").unwrap_or(0.0);
                    if let Some(value) = number("w") {
                        width = value;
                    }
                    if let Some(value) = number("h") {
                        height = value;
                    }
                    if let Some(value) = props.get("d").and_then(Value::as_bool) {
                        decal = value;
                    }
                }
                _ => {}
            }
        }
        y += 1.0;
        x = cluster_x;
    }
    keys
}

/// A KLE legend's newline-separated labels, moved to their places for `align`.
fn reorder(legend: &str, align: usize) -> [Option<&str>; 12] {
    let mut labels = [None; 12];
    let map = LABEL_MAP.get(align).unwrap_or(&LABEL_MAP[DEFAULT_ALIGN]);
    for (raw, label) in legend.split('\n').enumerate() {
        let Some(place) = map.get(raw).and_then(|place| usize::try_from(*place).ok()) else {
            continue;
        };
        if !label.is_empty() {
            labels[place] = Some(label);
        }
    }
    labels
}

/// `"a,b"` as two numbers.
fn pair(label: &str) -> Option<(u32, u32)> {
    let (a, b) = label.split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// Which switches a matrix reply holds down, row by row (bit `c` = column `c`).
/// `None` when the reply is too short for the matrix.
fn matrix_rows(reply: &[u8], rows: u8, cols: u8) -> Option<Vec<u32>> {
    let bytes_per_row = usize::from(cols).div_ceil(8);
    (0..usize::from(rows))
        .map(|row| {
            let start = 2 + row * bytes_per_row;
            let mut bits = 0u32;
            for col in 0..usize::from(cols) {
                // Most significant byte first: columns 8-15 come before 0-7.
                let byte = reply.get(start + bytes_per_row - 1 - col / 8)?;
                if byte & (1 << (col % 8)) != 0 {
                    bits |= 1 << col;
                }
            }
            Some(bits)
        })
        .collect()
}

/// An RGB colour as QMK HSV, every component 0-255 (hue 0 red, 85 green, 171
/// blue).
fn rgb_to_hsv([r, g, b]: [u8; 3]) -> [u8; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max == 0 {
        return BLACK;
    }
    let delta = f32::from(max - min);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let saturation = (delta * 255.0 / f32::from(max)).round() as u8;
    if max == min {
        return [0, 0, max];
    }
    let (base, toward) = if max == r {
        (0.0, f32::from(g) - f32::from(b))
    } else if max == g {
        (85.0, f32::from(b) - f32::from(r))
    } else {
        (171.0, f32::from(r) - f32::from(g))
    };
    #[allow(clippy::cast_possible_truncation)]
    let hue = (base + 256.0 / 6.0 * toward / delta).round() as i32;
    #[allow(clippy::cast_sign_loss)]
    let hue = hue.rem_euclid(256) as u8;
    [hue, saturation, max]
}

/// Stands in for a Vial board whose layout could not be read, so the reason
/// can be shown. Never driven.
pub static UNREAD: DeviceModel = DeviceModel {
    id: "qmk-vial",
    name: "Vial keyboard",
    family: Family::Qmk,
    vendor_id: 0,
    product_ids: &[],
    more_usb_ids: &[],
    protocol: Protocol::VialRawHid,
    rows: 0,
    cols: 0,
    keys: 0,
    dials: 0,
    extra_keys: 0,
    image: None,
    extra_image: None,
    strip_keys: 0,
    strip_image: None,
    strip_placement: StripPlacement::Below,
    wheel_image: None,
    info_image: None,
    dial_lights: DialLights::None,
    deck_lights: 0,
    key_lights: false,
    usage_page: Some(RAW_USAGE_PAGE),
};

struct Registered {
    model: &'static DeviceModel,
    board: Arc<Board>,
    names: Vec<&'static str>,
}

/// Every Vial model built this run. Each is leaked once and reused when the
/// same board comes back, so replugging does not leak again.
static REGISTRY: Mutex<Vec<Registered>> = Mutex::new(Vec::new());

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// The model for a board that answered [`probe`], named after its USB product
/// string.
pub fn register(
    board: Board,
    vendor_id: u16,
    product_id: u16,
    product: Option<&str>,
) -> &'static DeviceModel {
    let name = product
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("Vial keyboard");
    let mut registry = REGISTRY.lock().unwrap_or_else(PoisonError::into_inner);
    let known = registry.iter().find(|entry| {
        *entry.board == board
            && entry.model.vendor_id == vendor_id
            && entry.model.product_ids == [product_id]
            && entry.model.name == name
    });
    if let Some(entry) = known {
        return entry.model;
    }
    let leds = board
        .rgb
        .as_ref()
        .map(|rgb| rgb.leds.as_slice())
        .unwrap_or_default();
    let model: &'static DeviceModel = Box::leak(Box::new(DeviceModel {
        id: leak(format!("qmk-vial-{:016x}", board.uid)),
        name: leak(name.to_owned()),
        family: Family::Qmk,
        vendor_id,
        product_ids: Box::leak(Box::new([product_id])),
        more_usb_ids: &[],
        protocol: Protocol::VialRawHid,
        rows: board.grid.0,
        cols: board.grid.1,
        keys: u8::try_from(board.keys.len()).unwrap_or(u8::MAX),
        dials: 0,
        extra_keys: 0,
        image: None,
        extra_image: None,
        strip_keys: 0,
        strip_image: None,
        strip_placement: StripPlacement::Below,
        wheel_image: None,
        info_image: None,
        dial_lights: DialLights::None,
        deck_lights: u8::try_from(leds.iter().filter(|led| led.is_none()).count())
            .unwrap_or(u8::MAX),
        key_lights: leds.iter().any(Option::is_some),
        usage_page: Some(RAW_USAGE_PAGE),
    }));
    let names = board
        .keys
        .iter()
        .map(|(row, col)| leak(format!("Row {} col {}", row + 1, col + 1)))
        .collect();
    registry.push(Registered {
        model,
        board: Arc::new(board),
        names,
    });
    model
}

fn registered<T>(model: &DeviceModel, read: impl FnOnce(&Registered) -> T) -> Option<T> {
    let registry = REGISTRY.lock().unwrap_or_else(PoisonError::into_inner);
    registry
        .iter()
        .find(|entry| std::ptr::eq(entry.model, model))
        .map(read)
}

/// Every Vial model built so far, oldest first.
#[must_use]
pub fn models() -> Vec<&'static DeviceModel> {
    let registry = REGISTRY.lock().unwrap_or_else(PoisonError::into_inner);
    registry.iter().map(|entry| entry.model).collect()
}

/// What the handshake learned about a Vial model's board.
#[must_use]
pub fn board(model: &DeviceModel) -> Option<Arc<Board>> {
    registered(model, |entry| Arc::clone(&entry.board))
}

/// Each key's switch-matrix position, as its name.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    registered(model, |entry| entry.names.clone())
}

/// Each key's cell in the model's grid.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    registered(model, |entry| entry.board.cells.clone())
}

/// The keys with a light under them, when only some have one.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    let leds = registered(model, |entry| entry.board.rgb.clone())??.leds;
    let lit: Vec<u8> = (0..model.keys)
        .filter(|key| leds.contains(&Some(*key)))
        .collect();
    (lit.len() < usize::from(model.keys)).then_some(lit)
}

/// The board's matrix lock as last read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Lock {
    unlocked: bool,
    unlocking: bool,
    /// The firmware's countdown, 50 down to 0 while the unlock keys are held.
    counter: u8,
}

pub struct Vial {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    board: Option<Arc<Board>>,
    started: bool,
    lock: Lock,
    last_unlock_poll: Option<Instant>,
    next_matrix: Option<Instant>,
    /// The next matrix read only records which keys are down (just after an
    /// unlock, while the unlock keys are still held).
    baseline: bool,
    pressed: Vec<bool>,
    /// Inputs found outside a poll (releases when the board locked again).
    pending: Vec<Input>,
    /// Each LED's HSV at full brightness.
    colors: Vec<[u8; 3]>,
    /// What each LED was last sent, in direct mode.
    sent: Vec<Option<[u8; 3]>>,
    brightness: u8,
    direct: bool,
    /// The board's own effect (mode, speed, H, S, V), restored on the way out.
    saved_mode: Option<[u8; 6]>,
}

impl Vial {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let board = board(model);
        let leds = board
            .as_ref()
            .and_then(|board| board.rgb.as_ref())
            .map_or(0, |rgb| rgb.leds.len());
        let keys = board.as_ref().map_or(0, |board| board.keys.len());
        Self {
            model,
            transport,
            board,
            started: false,
            lock: Lock {
                unlocked: false,
                unlocking: false,
                counter: UNLOCK_COUNTER,
            },
            last_unlock_poll: None,
            next_matrix: None,
            baseline: true,
            pressed: vec![false; keys],
            pending: Vec::new(),
            colors: vec![BLACK; leds],
            sent: vec![None; leds],
            brightness: 100,
            direct: false,
            saved_mode: None,
        }
    }

    fn board(&self) -> DriverResult<Arc<Board>> {
        self.board.clone().ok_or_else(|| {
            refused("Ryu lost this keyboard's layout. Unplug it and plug it back in.")
        })
    }

    /// Reads the lock once, the first time the deck is used.
    fn start(&mut self) -> DriverResult<()> {
        if self.started {
            return Ok(());
        }
        self.board()?;
        let status = vial(self.transport.as_mut(), &[VIAL_GET_UNLOCK_STATUS])?;
        self.read_lock(&status);
        self.started = true;
        Ok(())
    }

    /// Applies an unlock status (`fe 05`) or poll (`fe 07`) answer.
    fn read_lock(&mut self, status: &[u8; PAYLOAD_LEN]) {
        let was_unlocked = self.lock.unlocked;
        self.lock.unlocked = status[0] == 1;
        self.lock.unlocking = !self.lock.unlocked && status[1] == 1;
        if !self.lock.unlocking {
            self.lock.counter = UNLOCK_COUNTER;
        }
        if was_unlocked && !self.lock.unlocked {
            // Locked again (the Vial app, a flash): let go of every held key.
            for (index, pressed) in self.pressed.iter_mut().enumerate() {
                if std::mem::take(pressed) {
                    if let Ok(index) = u8::try_from(index) {
                        self.pending.push(Input::Key {
                            index,
                            pressed: false,
                        });
                    }
                }
            }
        }
        if !was_unlocked && self.lock.unlocked {
            self.baseline = true;
        }
    }

    fn poll_unlock(&mut self) -> DriverResult<()> {
        let reply = vial(self.transport.as_mut(), &[VIAL_UNLOCK_POLL])?;
        self.last_unlock_poll = Some(Instant::now());
        self.read_lock(&reply);
        if self.lock.unlocking {
            self.lock.counter = reply[2].min(UNLOCK_COUNTER);
        } else {
            // The lights waited for the unlock to end.
            self.flush()?;
        }
        Ok(())
    }

    fn read_matrix(&mut self, reply: &[u8; PAYLOAD_LEN], board: &Board) -> Vec<Input> {
        let Some(rows) = matrix_rows(reply, board.rows, board.cols) else {
            return Vec::new();
        };
        let mut inputs = Vec::new();
        for (index, (row, col)) in board.keys.iter().enumerate() {
            let down = rows
                .get(usize::from(*row))
                .is_some_and(|bits| bits & (1 << col) != 0);
            let (Some(pressed), Ok(key)) = (self.pressed.get_mut(index), u8::try_from(index))
            else {
                continue;
            };
            if *pressed != down {
                *pressed = down;
                if !self.baseline {
                    inputs.push(Input::Key {
                        index: key,
                        pressed: down,
                    });
                }
            }
        }
        self.baseline = false;
        inputs
    }

    fn lighting(&mut self, args: &[u8]) -> DriverResult<[u8; PAYLOAD_LEN]> {
        let mut packet = vec![ID_LIGHTING_SET_VALUE];
        packet.extend_from_slice(args);
        exchange(self.transport.as_mut(), &packet, &packet[..2])
    }

    /// Puts the board's own effect back.
    fn restore(&mut self) -> DriverResult<()> {
        self.direct = false;
        let Some([mode_lo, mode_hi, speed, h, s, v]) = self.saved_mode else {
            return Ok(());
        };
        self.lighting(&[VIALRGB_SET_MODE, mode_lo, mode_hi, speed, h, s, v])?;
        Ok(())
    }

    /// An LED's colour as sent: V scaled by the brightness, up to the board's
    /// maximum.
    fn scaled(&self, [h, s, v]: [u8; 3], max_value: u8) -> [u8; 3] {
        let scaled = u32::from(v) * u32::from(self.brightness) * u32::from(max_value) / (100 * 255);
        [h, s, u8::try_from(scaled).unwrap_or(max_value)]
    }

    fn idle(timeout_ms: i32) {
        if let Ok(ms) = u64::try_from(timeout_ms) {
            if ms > 0 {
                std::thread::sleep(Duration::from_millis(ms));
            }
        }
    }
}

impl Driver for Vial {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.start()?;
        let mut inputs = std::mem::take(&mut self.pending);
        let now = Instant::now();
        if self.lock.unlocking {
            if self
                .last_unlock_poll
                .is_none_or(|at| now.duration_since(at) >= UNLOCK_POLL)
            {
                self.poll_unlock()?;
            } else {
                Self::idle(timeout_ms);
            }
            return Ok(inputs);
        }
        if !self.lock.unlocked {
            Self::idle(timeout_ms);
            return Ok(inputs);
        }
        if let Some(due) = self.next_matrix.filter(|due| now < *due) {
            let left = i32::try_from((due - now).as_millis()).unwrap_or(timeout_ms);
            Self::idle(timeout_ms.min(left));
            return Ok(inputs);
        }
        self.next_matrix = Some(now + MATRIX_POLL);
        let board = self.board()?;
        let reply = exchange(
            self.transport.as_mut(),
            &[ID_GET_KEYBOARD_VALUE, ID_SWITCH_MATRIX_STATE],
            &[ID_GET_KEYBOARD_VALUE, ID_SWITCH_MATRIX_STATE],
        )?;
        inputs.extend(self.read_matrix(&reply, &board));
        Ok(inputs)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let Some(rgb) = self.board.as_ref().and_then(|board| board.rgb.as_ref()) else {
            return Ok(());
        };
        let hsv = rgb_to_hsv(face.map_or(BLACK, |face| face.color));
        for (color, led) in self.colors.iter_mut().zip(&rgb.leds) {
            if *led == Some(key) {
                *color = hsv;
            }
        }
        Ok(())
    }

    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        let Some(rgb) = self.board.as_ref().and_then(|board| board.rgb.as_ref()) else {
            return Ok(());
        };
        let hsv = rgb_to_hsv(color.unwrap_or(BLACK));
        for (slot, led) in self.colors.iter_mut().zip(&rgb.leds) {
            if led.is_none() {
                *slot = hsv;
            }
        }
        Ok(())
    }

    /// Enters direct mode when something is lit (leaves it when nothing is),
    /// then sends each 9-LED run that changed.
    fn flush(&mut self) -> DriverResult<()> {
        let Some(max_value) = self
            .board
            .as_ref()
            .and_then(|board| board.rgb.as_ref())
            .map(|rgb| rgb.max_value)
        else {
            return Ok(());
        };
        self.start()?;
        // The firmware ignores lights mid-unlock; they go out when it ends.
        if self.lock.unlocking {
            return Ok(());
        }
        let lit = self.colors.iter().any(|color| color[2] > 0);
        if !lit {
            return if self.direct { self.restore() } else { Ok(()) };
        }
        if !self.direct {
            if self.saved_mode.is_none() {
                let mode = exchange(
                    self.transport.as_mut(),
                    &[ID_LIGHTING_GET_VALUE, VIALRGB_GET_MODE],
                    &[ID_LIGHTING_GET_VALUE, VIALRGB_GET_MODE],
                )?;
                let mut saved = [0u8; 6];
                saved.copy_from_slice(&mode[2..8]);
                // A board already in direct mode was left there by a run that
                // never put its effect back (the sidecar was killed): its
                // frozen colours are not an effect to restore, so a plain solid
                // colour stands in rather than direct mode again.
                if u16::from_le_bytes([saved[0], saved[1]]) == VIALRGB_EFFECT_DIRECT {
                    let [mode_lo, mode_hi] = VIALRGB_EFFECT_SOLID_COLOR.to_le_bytes();
                    saved[0] = mode_lo;
                    saved[1] = mode_hi;
                    if saved[5] == 0 {
                        saved[5] = max_value;
                    }
                }
                self.saved_mode = Some(saved);
            }
            let [_, _, speed, h, s, v] = self.saved_mode.unwrap_or_default();
            let [mode_lo, mode_hi] = VIALRGB_EFFECT_DIRECT.to_le_bytes();
            self.lighting(&[VIALRGB_SET_MODE, mode_lo, mode_hi, speed, h, s, v])?;
            self.direct = true;
            self.sent.fill(None);
        }
        for first in (0..self.colors.len()).step_by(FASTSET_LEDS) {
            let end = (first + FASTSET_LEDS).min(self.colors.len());
            let run: Vec<[u8; 3]> = self.colors[first..end]
                .iter()
                .map(|color| self.scaled(*color, max_value))
                .collect();
            let changed = run
                .iter()
                .zip(&self.sent[first..end])
                .any(|(color, sent)| *sent != Some(*color));
            if !changed {
                continue;
            }
            let [first_lo, first_hi] = u16::try_from(first).unwrap_or(u16::MAX).to_le_bytes();
            let mut args = vec![
                VIALRGB_DIRECT_FASTSET,
                first_lo,
                first_hi,
                u8::try_from(run.len()).unwrap_or(0),
            ];
            args.extend(run.iter().flatten());
            self.lighting(&args)?;
            for (sent, color) in self.sent[first..end].iter_mut().zip(run) {
                *sent = Some(color);
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }

    /// Re-reads the lock, to notice the board locking again or being unlocked
    /// by the Vial app.
    fn heartbeat(&mut self) -> DriverResult<()> {
        if !self.started || self.lock.unlocking {
            return Ok(());
        }
        let status = vial(self.transport.as_mut(), &[VIAL_GET_UNLOCK_STATUS])?;
        self.read_lock(&status);
        Ok(())
    }

    fn lock_state(&self) -> Option<LockState> {
        let board = self.board.as_ref()?;
        let keys: Vec<u8> = board
            .unlock_keys
            .iter()
            .filter_map(|(row, col)| board.key_at(*row, *col))
            .collect();
        let held = UNLOCK_COUNTER - self.lock.counter.min(UNLOCK_COUNTER);
        Some(LockState {
            unlocked: self.lock.unlocked,
            unlocking: self.lock.unlocking,
            progress: held * 2,
            keys_off_layout: keys.len() < board.unlock_keys.len(),
            keys,
        })
    }

    /// Starts an unlock, only on a board that is locked and not already
    /// unlocking: one cannot be cancelled, and the firmware ignores lights and
    /// keys until the unlock keys have been held long enough.
    fn start_unlock(&mut self) -> DriverResult<()> {
        self.start()?;
        let board = self.board()?;
        let status = vial(self.transport.as_mut(), &[VIAL_GET_UNLOCK_STATUS])?;
        self.read_lock(&status);
        if self.lock.unlocked || self.lock.unlocking {
            return Ok(());
        }
        if board.unlock_keys.is_empty() {
            return Err(refused(
                "This keyboard names no unlock keys, so it cannot be unlocked from Ryu. Unlock it in the Vial app.",
            ));
        }
        // An unlock cannot be cancelled, and one waiting on a key the user
        // is never shown would hold the board until it is unplugged.
        if board
            .unlock_keys
            .iter()
            .any(|(row, col)| board.key_at(*row, *col).is_none())
        {
            return Err(refused(UNLOCK_KEYS_OFF_LAYOUT));
        }
        vial(self.transport.as_mut(), &[VIAL_UNLOCK_START])?;
        self.lock.unlocking = true;
        self.lock.counter = UNLOCK_COUNTER;
        self.last_unlock_poll = Some(Instant::now());
        tracing::info!(device = self.model.id, "ryu-decks: Vial unlock started");
        Ok(())
    }
}

impl Drop for Vial {
    /// Gives the board its own lighting back.
    fn drop(&mut self) {
        if self.direct && !self.lock.unlocking {
            let _ = self.restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::Recorder;
    use super::*;

    /// `{"name":"Test Pad","matrix":{"rows":2,"cols":3},"lighting":"vialrgb",
    /// "layouts":{"keymap":[["0,0","0,1","0,2",{"x":0.5},"0,0\n\n\n\n\n\n\n\n\ne"],
    /// ["1,0",{"w":2},"1,1"]]}}` through Python's `lzma.compress`.
    const DEFINITION: [u8; 200] = [
        0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x04, 0xe6, 0xd6, 0xb4, 0x46, 0x02, 0x00, 0x21,
        0x01, 0x16, 0x00, 0x00, 0x00, 0x74, 0x2f, 0xe5, 0xa3, 0xe0, 0x00, 0xa8, 0x00, 0x86, 0x5d,
        0x00, 0x3d, 0x88, 0x89, 0xc6, 0x54, 0x36, 0xc3, 0x17, 0x4f, 0xe4, 0xe6, 0xbe, 0x7a, 0x4e,
        0x39, 0x15, 0xb8, 0xf2, 0x06, 0x3f, 0x18, 0x7e, 0x67, 0x0d, 0x56, 0x61, 0x74, 0x1e, 0x43,
        0x2d, 0x2c, 0x5e, 0x54, 0x32, 0xed, 0x40, 0x6f, 0x1c, 0x68, 0xb4, 0x14, 0xcf, 0xba, 0x74,
        0x57, 0x67, 0xa7, 0xda, 0x9f, 0x88, 0x5c, 0x7c, 0x57, 0x2c, 0xbc, 0xeb, 0x36, 0x93, 0xed,
        0xba, 0xe5, 0x92, 0x1e, 0xb5, 0x34, 0x7c, 0x1a, 0x39, 0x53, 0x3d, 0x1c, 0x56, 0x99, 0x3e,
        0xd8, 0xb6, 0x02, 0x8c, 0x28, 0x73, 0x60, 0x75, 0xac, 0x02, 0xdd, 0xc4, 0xe4, 0xd4, 0xd2,
        0xde, 0x5b, 0x37, 0x2e, 0xb7, 0xbe, 0xbe, 0xc5, 0x83, 0xa0, 0xa9, 0xac, 0x13, 0xb2, 0x3b,
        0x21, 0x77, 0xc9, 0xca, 0xec, 0x7f, 0xb6, 0x50, 0x03, 0x72, 0xe0, 0x11, 0x0a, 0xff, 0xd7,
        0x4f, 0x92, 0xea, 0xfd, 0x82, 0x71, 0x4d, 0x91, 0x42, 0x5b, 0xb0, 0x0d, 0x81, 0x8b, 0x3a,
        0x00, 0x00, 0x00, 0x3f, 0xe7, 0x3f, 0x05, 0x17, 0x68, 0x93, 0x8f, 0x00, 0x01, 0xa2, 0x01,
        0xa9, 0x01, 0x00, 0x00, 0x30, 0x71, 0x40, 0x9b, 0xb1, 0xc4, 0x67, 0xfb, 0x02, 0x00, 0x00,
        0x00, 0x00, 0x04, 0x59, 0x5a,
    ];

    /// A 32-byte answer starting with `bytes`.
    fn reply(bytes: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; PAYLOAD_LEN];
        out[..bytes.len()].copy_from_slice(bytes);
        out
    }

    fn write(bytes: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; WRITE_LEN];
        out[1..=bytes.len()].copy_from_slice(bytes);
        out
    }

    /// Answers for the whole handshake of the test pad: keyboard UID
    /// 0x0102030405060708, VialRGB with the direct effect, unlock keys (0,0)
    /// and (1,1), and four LEDs: under keys (0,1) and (1,0), then two
    /// underglow LEDs.
    fn queue_handshake(recorder: &Recorder, unlocked: bool) {
        recorder.queue_read(reply(&[0x01, 0x00, 0x09]));
        recorder.queue_read(reply(&[6, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1, 1]));
        recorder.queue_read(reply(&[200, 0, 0, 0]));
        for page in DEFINITION.chunks(PAYLOAD_LEN) {
            recorder.queue_read(reply(page));
        }
        let mut status = vec![0xffu8; PAYLOAD_LEN];
        status[..6].copy_from_slice(&[u8::from(unlocked), 0, 0, 0, 1, 1]);
        recorder.queue_read(status);
        recorder.queue_read(reply(&[0x08, 0x40, 0x01, 0x00, 200]));
        let mut effects = vec![0xffu8; PAYLOAD_LEN];
        effects[..6].copy_from_slice(&[0x08, 0x42, 0x01, 0x00, 0x02, 0x00]);
        recorder.queue_read(effects);
        let mut none = vec![0xffu8; PAYLOAD_LEN];
        none[..2].copy_from_slice(&[0x08, 0x42]);
        recorder.queue_read(none);
        recorder.queue_read(reply(&[0x08, 0x43, 4, 0]));
        // x, y, flags, then the matrix row and column (0xff: not on a key).
        recorder.queue_read(reply(&[0x08, 0x44, 0x30, 0x10, 0x04, 0, 1]));
        recorder.queue_read(reply(&[0x08, 0x44, 0x10, 0x30, 0x04, 1, 0]));
        recorder.queue_read(reply(&[0x08, 0x44, 0x00, 0x40, 0x02, 0xff, 0xff]));
        recorder.queue_read(reply(&[0x08, 0x44, 0xe0, 0x40, 0x02, 0xff, 0xff]));
    }

    fn probed(unlocked: bool) -> (Recorder, &'static DeviceModel) {
        let recorder = Recorder::default();
        queue_handshake(&recorder, unlocked);
        let mut transport = recorder.clone();
        let board = probe(&mut transport).unwrap();
        let model = register(board, 0xfeed, 0x0000, Some("Test Pad"));
        recorder.0.lock().unwrap().writes.clear();
        (recorder, model)
    }

    fn deck(unlocked: bool) -> (Recorder, Vial) {
        let (recorder, model) = probed(unlocked);
        let mut status = vec![0xffu8; PAYLOAD_LEN];
        status[..2].copy_from_slice(&[u8::from(unlocked), 0]);
        recorder.queue_read(status);
        let deck = Vial::new(model, Box::new(recorder.clone()));
        (recorder, deck)
    }

    #[test]
    fn only_a_vial_serial_on_the_raw_hid_collection_is_claimed() {
        assert!(is_vial_collection(Some("vial:f64c2b3c"), 0xff60, 0x61));
        assert!(is_vial_collection(Some("x vial:f64c2b3c"), 0xff60, 0x61));
        assert!(!is_vial_collection(Some("vial:f64c2b3c"), 0x0001, 0x06));
        assert!(!is_vial_collection(Some("vial:f64c2b3c"), 0xff60, 0x62));
        assert!(!is_vial_collection(Some("ABC123"), 0xff60, 0x61));
        assert!(!is_vial_collection(None, 0xff60, 0x61));
    }

    #[test]
    fn the_handshake_reads_the_layout_and_lights() {
        let recorder = Recorder::default();
        queue_handshake(&recorder, false);
        let mut transport = recorder.clone();
        let board = probe(&mut transport).unwrap();
        assert_eq!(board.uid, 0x0102_0304_0506_0708);
        assert_eq!((board.rows, board.cols), (2, 3));
        // The encoder is not a key, and (1,2) is not on the board.
        assert_eq!(board.keys, vec![(0, 0), (0, 1), (0, 2), (1, 0), (1, 1)]);
        // The 2u key's centre sits under the third column.
        assert_eq!(board.grid, (2, 3));
        assert_eq!(board.cells, vec![0, 1, 2, 3, 5]);
        assert_eq!(board.unlock_keys, vec![(0, 0), (1, 1)]);
        let rgb = board.rgb.unwrap();
        assert_eq!(rgb.max_value, 200);
        assert_eq!(rgb.leds, vec![Some(1), Some(3), None, None]);

        let writes = recorder.writes();
        assert!(writes.iter().all(|w| w.len() == WRITE_LEN && w[0] == 0));
        assert_eq!(writes[0], write(&[0x01]));
        assert_eq!(writes[1], write(&[0xfe, 0x00]));
        assert_eq!(writes[2], write(&[0xfe, 0x01]));
        // 200 bytes are seven pages.
        for page in 0..7u8 {
            assert_eq!(writes[3 + usize::from(page)], write(&[0xfe, 0x02, page, 0]));
        }
        assert_eq!(writes[10], write(&[0xfe, 0x05]));
        assert_eq!(writes[11], write(&[0x08, 0x40]));
        assert_eq!(writes[12], write(&[0x08, 0x42, 0x00, 0x00]));
        assert_eq!(writes[13], write(&[0x08, 0x42, 0x02, 0x00]));
        assert_eq!(writes[14], write(&[0x08, 0x43]));
        assert_eq!(writes[15], write(&[0x08, 0x44, 0, 0]));
        assert_eq!(writes[18], write(&[0x08, 0x44, 3, 0]));
        assert_eq!(writes.len(), 19);
    }

    #[test]
    fn boards_ryu_cannot_drive_are_refused() {
        let refusal = |answers: &[Vec<u8>]| {
            let recorder = Recorder::default();
            for answer in answers {
                recorder.queue_read(answer.clone());
            }
            let mut transport = recorder.clone();
            match probe(&mut transport) {
                Err(DriverError::Refused(reason)) => reason,
                other => panic!("expected a refusal, got {other:?}"),
            }
        };
        assert!(refusal(&[reply(&[0xff])]).contains("does not speak VIA"));
        assert!(refusal(&[reply(&[0x01, 0x00, 0x0a])]).contains("protocol 10"));
        // Version 0 and an unlock under way: the firmware is ignoring us.
        assert!(refusal(&[reply(&[0x01]), reply(&[0, 1])]).contains("part-way through"));
        assert!(refusal(&[reply(&[0x01, 0, 9]), reply(&[2])]).contains("Vial protocol 2"));
        assert!(refusal(&[
            reply(&[0x01, 0, 9]),
            reply(&[6]),
            reply(&[0x00, 0x00, 0x02, 0x00])
        ])
        .contains("131072-byte"));
    }

    #[test]
    fn a_matrix_vial_cannot_report_is_refused() {
        let wide = serde_json::json!({ "matrix": { "rows": 8, "cols": 24 } });
        assert!(Layout::parse(&wide).unwrap_err().contains("8 x 24"));
        let fits = serde_json::json!({ "matrix": { "rows": 6, "cols": 17 } });
        let layout = Layout::parse(&fits).unwrap();
        // No key map: every matrix position is a key.
        assert_eq!(layout.keys.len(), 102);
        assert_eq!(layout.grid, (6, 17));
        assert!(Layout::parse(&serde_json::json!({ "matrix": { "rows": 0, "cols": 4 } })).is_err());
    }

    #[test]
    fn kle_labels_are_reordered_by_alignment() {
        // Align 4 (the default): raw 0 is label 0, raw 9 is label 4, raw 3 is label 8.
        let labels = reorder("1,2\n\n\n0,1\n\n\n\n\n\ne", 4);
        assert_eq!(labels[0], Some("1,2"));
        assert_eq!(labels[8], Some("0,1"));
        assert_eq!(labels[4], Some("e"));
        // Align 7 centres everything: raw 0 lands on label 4.
        assert_eq!(reorder("e", 7)[4], Some("e"));
        assert_eq!(reorder("e", 7)[0], None);
    }

    #[test]
    fn the_kle_map_skips_encoders_decals_and_repeated_options() {
        let keymap = serde_json::json!([
            { "name": "metadata" },
            ["0,0", { "d": true }, "0,1", "0,1", { "a": 0 }, "1,1\n\n\n\n\n\n\n\n\ne"],
            [{ "a": 4 }, "1,0\n\n\n0,0", "1,0\n\n\n0,1", "1,1\n\n\n0,1"],
            ["9,9"]
        ]);
        let keys = kle_keys(keymap.as_array().unwrap());
        let positions: Vec<(u8, u8)> = keys.iter().map(|key| (key.row, key.col)).collect();
        // The decal and the encoder are skipped.
        assert_eq!(
            positions,
            vec![(0, 0), (0, 1), (1, 0), (1, 0), (1, 1), (9, 9)]
        );
        assert!(!keys[2].alternate);
        assert!(keys[3].alternate);
        let definition = serde_json::json!({
            "matrix": { "rows": 2, "cols": 2 },
            "layouts": { "keymap": keymap },
        });
        let layout = Layout::parse(&definition).unwrap();
        // Outside the matrix is dropped; the first option keeps (1,0); the
        // alternate (1,1) stays because nothing else has it.
        assert_eq!(layout.keys, vec![(0, 0), (0, 1), (1, 0), (1, 1)]);
        assert_eq!(layout.grid, (2, 3));
        assert_eq!(layout.cells, vec![0, 2, 3, 5]);
    }

    #[test]
    fn matrix_rows_are_most_significant_byte_first() {
        // 2 rows of 10 columns: two bytes a row, columns 8-9 first.
        let mut answer = [0u8; PAYLOAD_LEN];
        answer[..6].copy_from_slice(&[0x02, 0x03, 0x02, 0x01, 0x00, 0x80]);
        assert_eq!(
            matrix_rows(&answer, 2, 10).unwrap(),
            vec![(1 << 9) | 1, 1 << 7]
        );
        assert!(matrix_rows(&answer[..4], 2, 10).is_none());
    }

    #[test]
    fn colours_become_qmk_hsv() {
        assert_eq!(rgb_to_hsv([255, 0, 0]), [0, 255, 255]);
        assert_eq!(rgb_to_hsv([0, 255, 0]), [85, 255, 255]);
        assert_eq!(rgb_to_hsv([0, 0, 255]), [171, 255, 255]);
        assert_eq!(rgb_to_hsv([255, 255, 0]), [43, 255, 255]);
        assert_eq!(rgb_to_hsv([255, 255, 255]), [0, 0, 255]);
        assert_eq!(rgb_to_hsv([0, 0, 0]), [0, 0, 0]);
        assert_eq!(rgb_to_hsv([128, 0, 128]), [213, 255, 128]);
    }

    #[test]
    fn a_board_is_registered_once_and_found_by_its_device_id() {
        let (_, first) = probed(true);
        let (_, again) = probed(true);
        assert!(std::ptr::eq(first, again));
        assert_eq!(first.id, "qmk-vial-0102030405060708");
        assert_eq!(first.name, "Test Pad");
        assert_eq!(
            (first.rows, first.cols, first.keys, first.dials),
            (2, 3, 5, 0)
        );
        assert!(first.key_lights);
        assert_eq!(first.deck_lights, 2);
        assert_eq!(first.protocol, Protocol::VialRawHid);
        let found = crate::catalog::model_for_device_id("qmk-vial-0102030405060708-pab").unwrap();
        assert!(std::ptr::eq(found, first));
        assert_eq!(lit_keys(first), Some(vec![1, 3]));
        assert_eq!(key_names(first).unwrap()[4], "Row 2 col 2");
        assert_eq!(key_cells(first), Some(vec![0, 1, 2, 3, 5]));
    }

    #[test]
    fn keys_are_read_from_the_matrix_once_unlocked() {
        let (recorder, mut deck) = deck(true);
        // The first read sets what is already held, without a press.
        recorder.queue_read(reply(&[0x02, 0x03, 0x01, 0x00]));
        assert!(deck.poll(0).unwrap().is_empty());
        assert_eq!(recorder.writes()[0], write(&[0xfe, 0x05]));
        assert_eq!(recorder.writes()[1], write(&[0x02, 0x03]));
        // (0,0) released, (1,1) pressed; (1,2) is not a key.
        deck.next_matrix = None;
        recorder.queue_read(reply(&[0x02, 0x03, 0x00, 0x06]));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 0,
                    pressed: false
                },
                Input::Key {
                    index: 4,
                    pressed: true
                },
            ]
        );
        // Another app's answer is skipped while ours is awaited.
        deck.next_matrix = None;
        recorder.queue_read(reply(&[0x08, 0x41, 1]));
        recorder.queue_read(reply(&[0x02, 0x03, 0x00, 0x06]));
        assert!(deck.poll(0).unwrap().is_empty());
        // Not polled again before the interval.
        let writes = recorder.writes().len();
        assert!(deck.poll(0).unwrap().is_empty());
        assert_eq!(recorder.writes().len(), writes);
    }

    #[test]
    fn a_locked_board_is_unlocked_only_when_asked() {
        let (recorder, mut deck) = deck(false);
        assert!(deck.poll(0).unwrap().is_empty());
        assert_eq!(recorder.writes(), vec![write(&[0xfe, 0x05])]);
        let state = deck.lock_state().unwrap();
        assert!(!state.unlocked && !state.unlocking);
        assert_eq!(state.keys, vec![0, 4]);
        assert!(!state.keys_off_layout);

        recorder.queue_read(reply(&[0, 0]));
        recorder.queue_read(reply(&[]));
        deck.start_unlock().unwrap();
        assert_eq!(
            recorder.writes()[1..],
            [write(&[0xfe, 0x05]), write(&[0xfe, 0x06])]
        );
        // Lights wait while the unlock runs.
        deck.set_face(
            1,
            Some(&Face {
                label: String::new(),
                color: [255, 0, 0],
                image: None,
            }),
        )
        .unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 3);

        deck.last_unlock_poll = None;
        recorder.queue_read(reply(&[0, 1, 30]));
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes()[3], write(&[0xfe, 0x07]));
        assert_eq!(deck.lock_state().unwrap().progress, 40);
        // Not polled again within 200 ms.
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 4);

        // Unlocked: the deferred lights go out (direct mode, then the LEDs).
        deck.last_unlock_poll = None;
        recorder.queue_read(reply(&[1, 0, 0]));
        recorder.queue_read(reply(&[0x08, 0x41, 0x05, 0x00, 0x80, 0x10, 0xff, 0xc8]));
        recorder.queue_read(reply(&[0x07, 0x41]));
        recorder.queue_read(reply(&[0x07, 0x42]));
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[5], write(&[0x08, 0x41]));
        assert_eq!(
            writes[6],
            write(&[0x07, 0x41, 0x01, 0x00, 0x80, 0x10, 0xff, 0xc8])
        );
        // Red on key 1's LED at full brightness: V scaled to the board's 200.
        assert_eq!(
            writes[7],
            write(&[0x07, 0x42, 0, 0, 4, 0, 255, 200, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        );
        let state = deck.lock_state().unwrap();
        assert!(state.unlocked && !state.unlocking);

        // Asking again once unlocked sends no unlock.
        recorder.queue_read(reply(&[1, 0]));
        deck.start_unlock().unwrap();
        assert_eq!(recorder.writes().last().unwrap(), &write(&[0xfe, 0x05]));
    }

    #[test]
    fn unlock_keys_off_the_layout_are_reported_and_never_started() {
        let (recorder, mut deck) = deck(false);
        assert!(deck.poll(0).unwrap().is_empty());
        // (1,2) is in the 2 x 3 matrix but not on the test pad's layout.
        let mut board = Board::clone(deck.board.as_ref().unwrap());
        board.unlock_keys.push((1, 2));
        deck.board = Some(Arc::new(board));
        let state = deck.lock_state().unwrap();
        assert_eq!(state.keys, vec![0, 4]);
        assert!(state.keys_off_layout);

        recorder.queue_read(reply(&[0, 0]));
        let error = deck.start_unlock().unwrap_err();
        assert!(error.to_string().contains("not on its layout"), "{error}");
        // Only the status was read: no unlock was started.
        assert_eq!(recorder.writes().last().unwrap(), &write(&[0xfe, 0x05]));
        assert!(!deck.lock_state().unwrap().unlocking);
    }

    #[test]
    fn locking_again_releases_held_keys() {
        let (recorder, mut deck) = deck(true);
        recorder.queue_read(reply(&[0x02, 0x03, 0x00, 0x00]));
        deck.poll(0).unwrap();
        deck.next_matrix = None;
        recorder.queue_read(reply(&[0x02, 0x03, 0x04, 0x00]));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 2,
                pressed: true
            }]
        );
        recorder.queue_read(reply(&[0, 0]));
        deck.heartbeat().unwrap();
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 2,
                pressed: false
            }]
        );
    }

    #[test]
    fn lights_send_only_changed_runs_and_restore_the_board_effect() {
        let (recorder, mut deck) = deck(true);
        let face = |color| Face {
            label: String::new(),
            color,
            image: None,
        };
        // Nothing lit: the board keeps its own effect.
        for key in 0..5 {
            deck.set_face(key, None).unwrap();
        }
        deck.set_deck_light(None).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes(), vec![write(&[0xfe, 0x05])]);

        recorder.queue_read(reply(&[0x08, 0x41, 0x02, 0x00, 0x7f, 0x00, 0xff, 0x80]));
        recorder.queue_read(reply(&[0x07, 0x41]));
        recorder.queue_read(reply(&[0x07, 0x42]));
        deck.set_deck_light(Some([0, 0, 255])).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[2],
            write(&[0x07, 0x41, 0x01, 0x00, 0x7f, 0x00, 0xff, 0x80])
        );
        assert_eq!(
            writes[3],
            write(&[0x07, 0x42, 0, 0, 4, 0, 0, 0, 0, 0, 0, 171, 255, 200, 171, 255, 200])
        );
        // Unchanged: nothing sent. Half brightness: the run again, V halved.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 4);
        recorder.queue_read(reply(&[0x07, 0x42]));
        deck.set_brightness(50).unwrap();
        assert_eq!(
            recorder.writes()[4],
            write(&[0x07, 0x42, 0, 0, 4, 0, 0, 0, 0, 0, 0, 171, 255, 100, 171, 255, 100])
        );
        // Everything off: the saved effect comes back.
        recorder.queue_read(reply(&[0x07, 0x41]));
        deck.set_deck_light(None).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[5],
            write(&[0x07, 0x41, 0x02, 0x00, 0x7f, 0x00, 0xff, 0x80])
        );
        // Lit again, then dropped: restored on the way out.
        recorder.queue_read(reply(&[0x07, 0x41]));
        recorder.queue_read(reply(&[0x07, 0x42]));
        deck.set_face(3, Some(&face([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        recorder.queue_read(reply(&[0x07, 0x41]));
        drop(deck);
        assert_eq!(
            recorder.writes().last().unwrap(),
            &write(&[0x07, 0x41, 0x02, 0x00, 0x7f, 0x00, 0xff, 0x80])
        );
    }

    #[test]
    fn a_board_left_in_direct_mode_is_not_restored_to_direct_mode() {
        let (recorder, mut deck) = deck(true);
        // An earlier run was killed in direct mode: the mode read says direct.
        recorder.queue_read(reply(&[0x08, 0x41, 0x01, 0x00, 0x7f, 0x00, 0x00, 0x00]));
        recorder.queue_read(reply(&[0x07, 0x41]));
        recorder.queue_read(reply(&[0x07, 0x42]));
        deck.set_deck_light(Some([0, 0, 255])).unwrap();
        deck.flush().unwrap();
        recorder.queue_read(reply(&[0x07, 0x41]));
        deck.set_deck_light(None).unwrap();
        deck.flush().unwrap();
        // A solid colour (effect 2) at the board's full value comes back.
        assert_eq!(
            recorder.writes().last().unwrap(),
            &write(&[0x07, 0x41, 0x02, 0x00, 0x7f, 0x00, 0x00, 200])
        );
    }

    #[test]
    fn an_oversized_dictionary_is_refused_before_decoding() {
        assert!(xz_dictionary_fits(&DEFINITION));
        let mut huge = DEFINITION;
        huge[16] = 40;
        assert!(!xz_dictionary_fits(&huge));
        assert!(decode_definition(&huge).is_err());
        assert!(decode_definition(&DEFINITION[..100]).is_err());
        assert_eq!(decode_definition(&DEFINITION).unwrap()["matrix"]["cols"], 3);
    }
}
