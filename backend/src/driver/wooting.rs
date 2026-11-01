//! Wooting analog keyboards: every key is a deck key with an RGB light, read
//! from the keyboard's analog stream.
//!
//! A Wooting lists two vendor HID collections beside its ordinary keyboard
//! ones, told apart by usage page rather than interface number, so the device
//! loop pairs them by page (`Protocol::paired_collections`): the deck is found
//! by its analog collection and the lighting collection is opened beside it.
//!
//! In, analog (reports with no id, which the keyboard sends on its own; only
//! keys that are down are listed, and a key missing from the newest report is
//! up; a read that times out changes nothing):
//! - page 0xFF53 (current firmware): sixteen 4-byte entries. Byte 0 is the
//!   matrix position (row in bits 5-7, column in bits 0-4), byte 1 a keycode,
//!   byte 2 holds the actuated bit (bit 0), the keycode's namespace (bits 2-5)
//!   and the depth's low two bits (bits 6-7), byte 3 the depth's high eight.
//!   An entry with depth 0 is padding. One position can be listed several
//!   times (a key bound to several codes): it is pressed if any entry says it
//!   is actuated. Ryu presses on the keyboard's own actuated bit.
//! - page 0xFF54 (the first firmware, and older 31E3 firmware): sixteen 3-byte
//!   entries, a big-endian HID keyboard usage and an 8-bit depth. There is no
//!   position or actuated bit, so Ryu places the usage by the published 6 x 21
//!   matrix chart and presses past 40% of the travel, releasing under 25%.
//!
//! Out, lighting (page 0x1337, or 0xFF55 on firmware with numbered reports):
//! - an 8-byte feature command `[id, magic, DA, command, p3, p2, p1, p0]`
//!   (id 0 and magic D0, or id 1 and magic D1 on 0xFF55), each answered on the
//!   same collection's input (128 bytes on the first firmware, 256 later, in
//!   as many reads as it takes; one report on 0xFF55), which must be read
//!   before the next. Colour init (33) hands the lights to the host after
//!   opening; reset all (32) gives them back to the keyboard's profile.
//! - a colour frame: `[0, D0, DA, 0B]` and 6 x 21 RGB565 words, little-endian,
//!   row by row, 21 columns wide whatever the board's width (257 bytes; boards
//!   that take 64-byte reports get bytes 1 to 256 as four `[0]` + 64-byte
//!   writes); on 0xFF55, `[5, D1, DA, 0B]` and the same words in 2047 bytes.
//!   Whether a 0x1337 board takes 64-byte reports is read from its report
//!   descriptor's first Report Count item (one data byte: small), falling back
//!   to the model's default.
//! - the first firmware instead takes the LED driver's memory: four (One) or
//!   five (Two) 129-byte reports `[0, D0, DA, 0B, slave, register]`, 96 bytes
//!   of gamma-corrected PWM levels and a CRC-16-CCITT of bytes 0 to 126.
//!
//! No keep-alive and no brightness command: Ryu scales the colours itself.
//! The keyboard keeps typing through its ordinary collections while Ryu reads
//! the analog one; Ryu cannot stop that.
//!
//! Facts per WootingKb's MPL-2.0 wooting-rgb-sdk (USB ids, report layouts,
//! commands, the LED index and gamma tables, the CRC) and wooting-analog-sdk
//! (analog pages and report layouts, depth scaling); no code was copied.

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

/// The analog collections, preferred first: 0xFF53 gives matrix positions.
pub const ANALOG_USAGE_PAGES: [u16; 2] = [ANALOG_V2_PAGE, ANALOG_V1_PAGE];
/// The lighting collections: 0x1337, or 0xFF55 with numbered reports.
pub const LIGHT_USAGE_PAGES: [u16; 2] = [LIGHT_PAGE, LIGHT_MULTI_REPORT_PAGE];

const ANALOG_V2_PAGE: u16 = 0xff53;
const ANALOG_V1_PAGE: u16 = 0xff54;
const LIGHT_PAGE: u16 = 0x1337;
const LIGHT_MULTI_REPORT_PAGE: u16 = 0xff55;

/// The colour frame's grid, whatever the board's width.
const FRAME_ROWS: usize = 6;
const FRAME_COLS: usize = 21;
const FRAME_CELLS: usize = FRAME_ROWS * FRAME_COLS;
/// The longest analog report (sixteen 4-byte entries).
const ANALOG_REPORT_LEN: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 32;

const MAGIC: u8 = 0xd0;
const MULTI_REPORT_MAGIC: u8 = 0xd1;
const MAGIC_LOW: u8 = 0xda;
const RAW_COLORS: u8 = 11;
const RESET_ALL: u8 = 32;
const COLOR_INIT: u8 = 33;
/// How long a feature command's answer may take, per read.
const ANSWER_TIMEOUT_MS: i32 = 1000;
const LEGACY_ANSWER_LEN: usize = 128;
const ANSWER_LEN: usize = 256;
const MULTI_REPORT_ANSWER_LEN: usize = 2046;

const FRAME_LEN: usize = 257;
const SMALL_PACKET: usize = 64;
const MULTI_REPORT_FRAME_LEN: usize = 2047;
const MULTI_REPORT_FRAME_ID: u8 = 5;

/// One part of the first firmware's LED memory: 96 PWM bytes.
const LEGACY_PART_LEN: usize = 96;
const LEGACY_REPORT_LEN: usize = 129;
/// Where in a part each of its 24 LEDs' red level sits; green is 0x10 on, blue 0x20.
const LEGACY_PWM_OFFSETS: [u8; 24] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x30, 0x31, 0x32, 0x33,
    0x34, 0x35, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d,
];
const NO_LED: u8 = 0xff;
/// The first firmware's LED index under each cell of the 6 x 21 matrix.
const LEGACY_LED_INDEX: [[u8; FRAME_COLS]; FRAME_ROWS] = [
    [
        0, NO_LED, 11, 12, 23, 24, 36, 47, 85, 84, 49, 48, 59, 61, 73, 81, 80, 113, 114, 115, 116,
    ],
    [
        2, 1, 14, 13, 26, 25, 35, 38, 37, 87, 86, 95, 51, 63, 75, 72, 74, 96, 97, 98, 99,
    ],
    [
        3, 4, 15, 16, 27, 28, 39, 42, 40, 88, 89, 52, 53, 71, 76, 83, 77, 102, 103, 104, 100,
    ],
    [
        5, 6, 17, 18, 29, 30, 41, 46, 44, 90, 93, 54, 57, 65, NO_LED, NO_LED, NO_LED, 105, 106,
        107, NO_LED,
    ],
    [
        9, 8, 19, 20, 31, 34, 32, 45, 43, 91, 92, 55, NO_LED, 66, NO_LED, 78, NO_LED, 108, 109,
        110, 101,
    ],
    [
        10, 22, 21, NO_LED, NO_LED, NO_LED, 33, NO_LED, NO_LED, NO_LED, 94, 58, 67, 68, 70, 79, 82,
        NO_LED, 111, 112, NO_LED,
    ],
];
/// The ANSI left shift's and enter's LEDs, whose levels the ISO keys' LEDs
/// (which no matrix cell names) copy.
const LEGACY_ANSI_SHIFT: u8 = 9;
const LEGACY_ISO_SHIFT: u8 = 7;
const LEGACY_ANSI_ENTER: u8 = 65;
const LEGACY_ISO_ENTER: u8 = 62;
/// The highest LED index the One has; the Two has 116.
const LEGACY_ONE_LAST_LED: u8 = 95;
const LEGACY_TWO_LAST_LED: u8 = 116;
/// The first firmware's gamma curve, level in to PWM level out.
#[rustfmt::skip]
const LEGACY_GAMMA: [u8; 256] = [
    0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4,
    5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8, 9, 9, 9, 10, 10, 10, 10, 10, 10, 11, 11, 11, 11, 11, 11,
    12, 12, 13, 13, 13, 13, 14, 14, 15, 15, 16, 16, 17, 17, 18, 18, 18, 18, 19, 19, 20, 20, 21, 21, 22, 22, 22, 22, 23, 24,
    24, 25, 25, 26, 26, 26, 27, 27, 28, 29, 29, 30, 31, 32, 32, 33, 34, 35, 35, 36, 37, 38, 39, 39, 40, 41, 42, 42, 43, 43,
    44, 44, 45, 45, 46, 47, 48, 49, 50, 50, 51, 51, 52, 52, 53, 54, 54, 55, 55, 56, 56, 57, 57, 58, 58, 59, 59, 60, 60, 61,
    61, 62, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 77, 78, 79, 81, 82, 83, 85, 86, 87, 89, 90, 92, 93, 95,
    96, 98, 99, 101, 102, 104, 105, 107, 109, 110, 112, 114, 115, 117, 119, 120, 122, 124, 126, 127, 129, 131, 133, 135, 137, 138, 140, 142, 144, 146,
    148, 150, 152, 154, 156, 158, 160, 162, 164, 167, 169, 171, 173, 175, 177, 180, 182, 184, 186, 189, 191, 193, 196, 198, 200, 203, 205, 208, 210, 213,
    215, 218, 220, 223, 225, 228, 231, 233, 236, 239, 241, 244, 247, 249, 252, 255,
];

/// Depth (0 to 1) past which a key with no actuated bit (page 0xFF54) presses,
/// and under which a pressed one releases.
const V1_PRESS_DEPTH: f32 = 0.40;
const V1_RELEASE_DEPTH: f32 = 0.25;

/// What the published Wooting Two matrix chart puts in each cell, row by row:
/// the legend, and its HID keyboard usage (0 for a cell with no switch or a
/// key that sends no HID usage).
#[rustfmt::skip]
const TWO_MATRIX: [(&str, u8); FRAME_CELLS] = [
    ("Esc", 0x29), ("No key", 0), ("F1", 0x3a), ("F2", 0x3b), ("F3", 0x3c), ("F4", 0x3d),
    ("F5", 0x3e), ("F6", 0x3f), ("F7", 0x40), ("F8", 0x41), ("F9", 0x42), ("F10", 0x43),
    ("F11", 0x44), ("F12", 0x45), ("Print Screen", 0x46), ("Pause", 0x48),
    ("Scroll Lock", 0x47), ("A1", 0), ("A2", 0), ("A3", 0), ("Mode", 0),

    ("`", 0x35), ("1", 0x1e), ("2", 0x1f), ("3", 0x20), ("4", 0x21), ("5", 0x22),
    ("6", 0x23), ("7", 0x24), ("8", 0x25), ("9", 0x26), ("0", 0x27), ("-", 0x2d),
    ("=", 0x2e), ("Backspace", 0x2a), ("Insert", 0x49), ("Home", 0x4a), ("Page Up", 0x4b),
    ("Num Lock", 0x53), ("Num /", 0x54), ("Num *", 0x55), ("Num -", 0x56),

    ("Tab", 0x2b), ("Q", 0x14), ("W", 0x1a), ("E", 0x08), ("R", 0x15), ("T", 0x17),
    ("Y", 0x1c), ("U", 0x18), ("I", 0x0c), ("O", 0x12), ("P", 0x13), ("[", 0x2f),
    ("]", 0x30), ("\\", 0x31), ("Delete", 0x4c), ("End", 0x4d), ("Page Down", 0x4e),
    ("Num 7", 0x5f), ("Num 8", 0x60), ("Num 9", 0x61), ("Num +", 0x57),

    ("Caps Lock", 0x39), ("A", 0x04), ("S", 0x16), ("D", 0x07), ("F", 0x09), ("G", 0x0a),
    ("H", 0x0b), ("J", 0x0d), ("K", 0x0e), ("L", 0x0f), (";", 0x33), ("'", 0x34),
    ("ISO #", 0x32), ("Enter", 0x28), ("No key", 0), ("No key", 0), ("No key", 0),
    ("Num 4", 0x5c), ("Num 5", 0x5d), ("Num 6", 0x5e), ("No key", 0),

    ("Left Shift", 0xe1), ("ISO \\", 0x64), ("Z", 0x1d), ("X", 0x1b), ("C", 0x06),
    ("V", 0x19), ("B", 0x05), ("N", 0x11), ("M", 0x10), (",", 0x36), (".", 0x37),
    ("/", 0x38), ("No key", 0), ("Right Shift", 0xe5), ("No key", 0), ("Up", 0x52),
    ("No key", 0), ("Num 1", 0x59), ("Num 2", 0x5a), ("Num 3", 0x5b), ("Num Enter", 0x58),

    ("Left Ctrl", 0xe0), ("Left Win", 0xe3), ("Left Alt", 0xe2), ("No key", 0),
    ("No key", 0), ("No key", 0), ("Space", 0x2c), ("No key", 0), ("No key", 0),
    ("No key", 0), ("Right Alt", 0xe6), ("Right Win", 0xe7), ("Fn", 0),
    ("Right Ctrl", 0xe4), ("Left", 0x50), ("Down", 0x51), ("Right", 0x4f), ("No key", 0),
    ("Num 0", 0x62), ("Num .", 0x63), ("No key", 0),
];

/// Each key's legend, on the 21-column boards the chart is drawn for (the
/// Two family). The One, 60HE and 80HE place their keys differently and
/// stay unnamed.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    let wooting = matches!(
        model.protocol,
        Protocol::WootingAnalog | Protocol::WootingLegacy
    );
    (wooting && usize::from(model.cols) == FRAME_COLS)
        .then(|| TWO_MATRIX.iter().map(|(name, _)| *name).collect())
}

/// The matrix cell (row, column) a HID keyboard usage sits at on the chart.
fn cell_of_usage(usage: u8) -> Option<(u8, u8)> {
    if usage == 0 {
        return None;
    }
    let at = TWO_MATRIX.iter().position(|(_, code)| *code == usage)?;
    Some(((at / FRAME_COLS) as u8, (at % FRAME_COLS) as u8))
}

/// `colour * percent / 100`, per channel.
fn dimmed(color: [u8; 3], percent: u8) -> [u8; 3] {
    color.map(|channel| (u16::from(channel) * u16::from(percent) / 100) as u8)
}

/// Red 5 bits, green 6, blue 5.
fn rgb565([r, g, b]: [u8; 3]) -> u16 {
    (u16::from(r & 0xf8) << 8) | (u16::from(g & 0xfc) << 3) | (u16::from(b) >> 3)
}

/// The 8-bit levels an RGB565 word stands for, low bits zero.
fn from_rgb565(word: u16) -> [u8; 3] {
    [
        ((word >> 8) & 0xf8) as u8,
        ((word >> 3) & 0xfc) as u8,
        ((word << 3) & 0xf8) as u8,
    ]
}

/// CRC-16-CCITT: polynomial 0x1021, starting at 0, most significant bit first.
fn crc16_ccitt(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x1021
            };
        }
    }
    crc
}

/// The 8-byte feature command, parameters last to first on the wire.
fn command(multi_report: bool, id: u8, params: [u8; 4]) -> [u8; 8] {
    let [p0, p1, p2, p3] = params;
    if multi_report {
        [1, MULTI_REPORT_MAGIC, MAGIC_LOW, id, p3, p2, p1, p0]
    } else {
        [0, MAGIC, MAGIC_LOW, id, p3, p2, p1, p0]
    }
}

/// How a lighting collection takes its colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LightDialect {
    /// The first firmware's LED memory, in four (One) or five (Two) parts.
    Legacy { parts: u8, last_led: u8 },
    /// The 257-byte frame, whole or as four 64-byte reports.
    Matrix { small_packets: bool },
    /// The 2047-byte frame with report id 5 (page 0xFF55).
    MultiReport,
}

impl LightDialect {
    fn answer_len(self) -> usize {
        match self {
            Self::Legacy { .. } => LEGACY_ANSWER_LEN,
            Self::Matrix { .. } => ANSWER_LEN,
            Self::MultiReport => MULTI_REPORT_ANSWER_LEN,
        }
    }

    const fn multi_report(self) -> bool {
        matches!(self, Self::MultiReport)
    }
}

/// Boards whose lighting collection takes 64-byte reports when the report
/// descriptor cannot be read.
fn small_packets_by_default(model: &DeviceModel) -> bool {
    matches!(
        model.id,
        "wooting-two-he-arm" | "wooting-60he-arm" | "wooting-60he-plus"
    )
}

/// Whether a report descriptor declares 64-byte reports: its first Report
/// Count item carries one data byte (a 256-byte report needs two). `None`
/// when it has no Report Count item.
fn descriptor_says_small(descriptor: &[u8]) -> Option<bool> {
    let mut at = 0;
    while let Some(&prefix) = descriptor.get(at) {
        if prefix == 0xfe {
            // A long item: its data size is the next byte.
            let size = usize::from(*descriptor.get(at + 1)?);
            at += 3 + size;
            continue;
        }
        let size = match prefix & 0x03 {
            3 => 4,
            n => usize::from(n),
        };
        if prefix & 0xfc == 0x94 {
            return Some(size == 1);
        }
        at += 1 + size;
    }
    None
}

/// How the analog collection reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnalogDialect {
    /// Page 0xFF53: positions, an actuated bit and 10-bit depths.
    V2,
    /// Page 0xFF54: HID usages and 8-bit depths, scaled by 1.2 on the first
    /// firmware.
    V1 { legacy_scale: bool },
    /// Page not known: told apart by report length (64 bytes is V2).
    ByLength,
}

/// One key the newest analog report lists.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Reading {
    key: usize,
    /// Pressed per the keyboard (V2), or `None` where only the depth is known.
    actuated: Option<bool>,
    depth: f32,
}

/// The keys a V2 report lists, by Ryu key index on a board `cols` wide.
fn parse_v2(report: &[u8], cols: usize) -> Vec<Reading> {
    report
        .chunks_exact(4)
        .filter_map(|entry| {
            let (position, flags, high) = (entry[0], entry[2], entry[3]);
            let value = (u16::from(high) << 2) | u16::from(flags >> 6);
            if value == 0 {
                return None;
            }
            let row = usize::from(position >> 5);
            let col = usize::from(position & 0x1f);
            if row >= FRAME_ROWS || col >= cols {
                return None;
            }
            Some(Reading {
                key: row * cols + col,
                actuated: Some(flags & 0x01 != 0),
                depth: (f32::from(value) / 1023.0).min(1.0),
            })
        })
        .collect()
}

/// The keys a V1 report lists, placed by the matrix chart.
fn parse_v1(report: &[u8], cols: usize, legacy_scale: bool) -> Vec<Reading> {
    report
        .chunks_exact(3)
        .filter_map(|entry| {
            let (namespace, usage, value) = (entry[0], entry[1], entry[2]);
            if value == 0 || namespace != 0 {
                return None;
            }
            let (row, col) = cell_of_usage(usage)?;
            let (row, col) = (usize::from(row), usize::from(col));
            if col >= cols {
                return None;
            }
            let scale = if legacy_scale { 1.2 } else { 1.0 };
            Some(Reading {
                key: row * cols + col,
                actuated: None,
                depth: (f32::from(value) * scale / 255.0).min(1.0),
            })
        })
        .collect()
}

pub struct Wooting {
    model: &'static DeviceModel,
    analog: Box<dyn Transport>,
    analog_dialect: AnalogDialect,
    /// The lighting collection, when it could be opened.
    lights: Option<(Box<dyn Transport>, LightDialect)>,
    held: Vec<bool>,
    /// The colour each key should show, at full brightness.
    colors: Vec<[u8; 3]>,
    dirty: bool,
    brightness: u8,
}

impl Wooting {
    /// Hands the lights to the host (colour init, its answer read back). With
    /// no lighting collection the keys are still read.
    #[must_use]
    pub fn new(
        model: &'static DeviceModel,
        analog: Box<dyn Transport>,
        lights: Option<Box<dyn Transport>>,
    ) -> Self {
        let legacy = model.protocol == Protocol::WootingLegacy;
        let analog_dialect = match analog.usage_page() {
            _ if legacy => AnalogDialect::V1 { legacy_scale: true },
            Some(ANALOG_V2_PAGE) => AnalogDialect::V2,
            Some(ANALOG_V1_PAGE) => AnalogDialect::V1 {
                legacy_scale: false,
            },
            _ => AnalogDialect::ByLength,
        };
        let lights = lights.map(|mut transport| {
            let dialect = if legacy {
                let two = usize::from(model.cols) == FRAME_COLS;
                LightDialect::Legacy {
                    parts: if two { 5 } else { 4 },
                    last_led: if two {
                        LEGACY_TWO_LAST_LED
                    } else {
                        LEGACY_ONE_LAST_LED
                    },
                }
            } else if transport.usage_page() == Some(LIGHT_MULTI_REPORT_PAGE) {
                LightDialect::MultiReport
            } else {
                let mut descriptor = [0u8; 4096];
                let small = transport
                    .report_descriptor(&mut descriptor)
                    .ok()
                    .and_then(|len| descriptor_says_small(descriptor.get(..len)?))
                    .unwrap_or_else(|| small_packets_by_default(model));
                LightDialect::Matrix {
                    small_packets: small,
                }
            };
            (transport, dialect)
        });
        let keys = usize::from(model.keys);
        let mut deck = Self {
            model,
            analog,
            analog_dialect,
            lights,
            held: vec![false; keys],
            colors: vec![[0; 3]; keys],
            dirty: true,
            brightness: 100,
        };
        if deck.lights.is_none() {
            tracing::warn!(
                device = model.id,
                "ryu-decks: the Wooting's lighting collection is not open; its keys are read but not lit"
            );
        } else if let Err(error) = deck.feature(COLOR_INIT) {
            tracing::warn!(device = model.id, %error, "ryu-decks: Wooting colour init failed");
        }
        deck
    }

    /// Sends one feature command with zero parameters and reads its answer.
    fn feature(&mut self, id: u8) -> DriverResult<()> {
        let Some((transport, dialect)) = self.lights.as_mut() else {
            return Ok(());
        };
        let dialect = *dialect;
        transport.send_feature_report(&command(dialect.multi_report(), id, [0; 4]))?;
        let want = dialect.answer_len();
        let mut answer = vec![0u8; want];
        let mut got = 0;
        while got < want {
            let read = transport.read_timeout(&mut answer[got..], ANSWER_TIMEOUT_MS)?;
            if read == 0 {
                return Err(super::DriverError::Io(format!(
                    "the keyboard answered command {id} with {got} of {want} bytes"
                )));
            }
            got += read;
            if dialect.multi_report() {
                break;
            }
        }
        Ok(())
    }

    /// The frame's 6 x 21 RGB565 words, brightness applied.
    fn frame_words(&self) -> [u16; FRAME_CELLS] {
        let cols = usize::from(self.model.cols);
        let mut words = [0u16; FRAME_CELLS];
        for (key, color) in self.colors.iter().enumerate() {
            let (row, col) = (key / cols, key % cols);
            if row < FRAME_ROWS && col < FRAME_COLS {
                words[row * FRAME_COLS + col] = rgb565(dimmed(*color, self.brightness));
            }
        }
        words
    }

    /// The reports that carry the current colours, in order.
    fn frame_reports(&self, dialect: LightDialect) -> Vec<Vec<u8>> {
        let words = self.frame_words();
        match dialect {
            LightDialect::Legacy { parts, last_led } => legacy_reports(&words, parts, last_led),
            LightDialect::Matrix { small_packets } => {
                let frame = matrix_frame(&words, FRAME_LEN, 0, MAGIC);
                if !small_packets {
                    return vec![frame];
                }
                frame[1..]
                    .chunks(SMALL_PACKET)
                    .map(|chunk| {
                        let mut report = Vec::with_capacity(SMALL_PACKET + 1);
                        report.push(0);
                        report.extend_from_slice(chunk);
                        report
                    })
                    .collect()
            }
            LightDialect::MultiReport => vec![matrix_frame(
                &words,
                MULTI_REPORT_FRAME_LEN,
                MULTI_REPORT_FRAME_ID,
                MULTI_REPORT_MAGIC,
            )],
        }
    }

    /// Turns the readings of one report into key edges.
    fn receive(&mut self, readings: &[Reading], events: &mut Vec<Input>) {
        let mut now = vec![false; self.held.len()];
        for reading in readings {
            let Some(slot) = now.get_mut(reading.key) else {
                continue;
            };
            let was = self.held.get(reading.key).copied().unwrap_or(false);
            let pressed = reading.actuated.unwrap_or_else(|| {
                let threshold = if was {
                    V1_RELEASE_DEPTH
                } else {
                    V1_PRESS_DEPTH
                };
                reading.depth >= threshold
            });
            *slot |= pressed;
        }
        for (key, (held, pressed)) in self.held.iter_mut().zip(now).enumerate() {
            if *held != pressed {
                *held = pressed;
                if let Ok(index) = u8::try_from(key) {
                    events.push(Input::Key { index, pressed });
                }
            }
        }
    }

    fn readings(&self, report: &[u8]) -> Vec<Reading> {
        let cols = usize::from(self.model.cols);
        match self.analog_dialect {
            AnalogDialect::V2 => parse_v2(report, cols),
            AnalogDialect::V1 { legacy_scale } => parse_v1(report, cols, legacy_scale),
            AnalogDialect::ByLength if report.len() >= ANALOG_REPORT_LEN => parse_v2(report, cols),
            AnalogDialect::ByLength => parse_v1(report, cols, false),
        }
    }
}

/// `[id, magic, DA, 0B]` and the words little-endian, zero-padded to `len`.
fn matrix_frame(words: &[u16; FRAME_CELLS], len: usize, id: u8, magic: u8) -> Vec<u8> {
    let mut frame = vec![0u8; len];
    frame[..4].copy_from_slice(&[id, magic, MAGIC_LOW, RAW_COLORS]);
    for (slot, word) in frame[4..].chunks_exact_mut(2).zip(words) {
        slot.copy_from_slice(&word.to_le_bytes());
    }
    frame
}

/// The first firmware's LED memory as its 129-byte reports.
fn legacy_reports(words: &[u16; FRAME_CELLS], parts: u8, last_led: u8) -> Vec<Vec<u8>> {
    fn set(memory: &mut [[u8; LEGACY_PART_LEN]; 5], led: u8, levels: [u8; 3]) {
        let offset = usize::from(LEGACY_PWM_OFFSETS[usize::from(led % 24)]);
        if let Some(part) = memory.get_mut(usize::from(led / 24)) {
            for (shift, level) in [0, 0x10, 0x20].into_iter().zip(levels) {
                if let Some(slot) = part.get_mut(offset + shift) {
                    *slot = level;
                }
            }
        }
    }
    let mut memory = [[0u8; LEGACY_PART_LEN]; 5];
    for (row, leds) in LEGACY_LED_INDEX.iter().enumerate() {
        for (col, &led) in leds.iter().enumerate() {
            if led == NO_LED || led > last_led {
                continue;
            }
            let levels = from_rgb565(words[row * FRAME_COLS + col])
                .map(|level| LEGACY_GAMMA[usize::from(level)]);
            set(&mut memory, led, levels);
            if led == LEGACY_ANSI_SHIFT {
                set(&mut memory, LEGACY_ISO_SHIFT, levels);
            } else if led == LEGACY_ANSI_ENTER {
                set(&mut memory, LEGACY_ISO_ENTER, levels);
            }
        }
    }
    (0..parts)
        .map(|part| {
            let mut report = vec![0u8; LEGACY_REPORT_LEN];
            report[..6].copy_from_slice(&[
                0,
                MAGIC,
                MAGIC_LOW,
                RAW_COLORS,
                part / 2,
                (part % 2) * LEGACY_PART_LEN as u8,
            ]);
            report[6..6 + LEGACY_PART_LEN].copy_from_slice(&memory[usize::from(part)]);
            let crc = crc16_ccitt(&report[..LEGACY_REPORT_LEN - 2]);
            report[LEGACY_REPORT_LEN - 2..].copy_from_slice(&crc.to_le_bytes());
            report
        })
        .collect()
}

impl Driver for Wooting {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; ANALOG_REPORT_LEN];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.analog.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            let readings = self.readings(&buf[..read]);
            self.receive(&readings, &mut events);
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let color = face.map_or([0; 3], |face| face.color);
        if let Some(slot) = self.colors.get_mut(usize::from(key)) {
            if *slot != color {
                *slot = color;
                self.dirty = true;
            }
        }
        Ok(())
    }

    /// One whole frame whenever any key changed.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.dirty {
            return Ok(());
        }
        let Some(dialect) = self.lights.as_ref().map(|(_, dialect)| *dialect) else {
            self.dirty = false;
            return Ok(());
        };
        let reports = self.frame_reports(dialect);
        if let Some((transport, _)) = self.lights.as_mut() {
            for report in &reports {
                transport.write(report)?;
            }
        }
        self.dirty = false;
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        let percent = percent.min(100);
        if percent == self.brightness {
            return Ok(());
        }
        self.brightness = percent;
        self.dirty = true;
        self.flush()
    }
}

impl Drop for Wooting {
    /// Gives the lights back to the keyboard's own profile.
    fn drop(&mut self) {
        if let Err(error) = self.feature(RESET_ALL) {
            tracing::debug!(device = self.model.id, %error, "ryu-decks: Wooting light reset failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::by_id;

    fn face(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    /// A deck on `model` with its analog collection on `analog_page` and its
    /// lights on `light_page` (whose colour-init answer is queued).
    fn deck(
        model: &str,
        analog_page: u16,
        light_page: u16,
        descriptor: Option<Vec<u8>>,
    ) -> (Wooting, Recorder, Recorder) {
        let analog = Recorder::default();
        analog.0.lock().unwrap().usage_page = Some(analog_page);
        let lights = Recorder::default();
        {
            let mut log = lights.0.lock().unwrap();
            log.usage_page = Some(light_page);
            log.descriptor = descriptor;
        }
        let model = by_id(model).unwrap();
        let answer = if model.protocol == Protocol::WootingLegacy {
            vec![0; 128]
        } else if light_page == LIGHT_MULTI_REPORT_PAGE {
            vec![0; 64]
        } else {
            // Two reads make up the 256-byte answer.
            lights.queue_read(vec![0; 128]);
            vec![0; 128]
        };
        lights.queue_read(answer);
        let deck = Wooting::new(
            model,
            Box::new(analog.clone()),
            Some(Box::new(lights.clone())),
        );
        (deck, analog, lights)
    }

    /// A V2 entry: position, keycode, flags and depth high byte.
    fn v2_entry(row: u8, col: u8, actuated: bool, value: u16) -> [u8; 4] {
        [
            (row << 5) | col,
            0x04,
            u8::from(actuated) | (((value & 0x03) as u8) << 6),
            (value >> 2) as u8,
        ]
    }

    fn v2_report(entries: &[[u8; 4]]) -> Vec<u8> {
        let mut report: Vec<u8> = entries.iter().flatten().copied().collect();
        report.resize(64, 0);
        report
    }

    #[test]
    fn opening_sends_colour_init_and_reads_its_whole_answer() {
        let (deck, _, lights) = deck("wooting-two-he", 0xff53, 0x1337, None);
        assert_eq!(lights.features(), vec![vec![0, 0xd0, 0xda, 33, 0, 0, 0, 0]]);
        // Both 128-byte reads were taken as the one answer.
        assert!(lights.0.lock().unwrap().reads.is_empty());
        drop(deck);
        assert_eq!(lights.features()[1], vec![0, 0xd0, 0xda, 32, 0, 0, 0, 0]);
    }

    #[test]
    fn multi_report_commands_use_report_one_and_d1() {
        let (deck, _, lights) = deck("wooting-60he-v2", 0xff53, 0xff55, None);
        assert_eq!(lights.features(), vec![vec![1, 0xd1, 0xda, 33, 0, 0, 0, 0]]);
        drop(deck);
    }

    #[test]
    fn command_parameters_go_out_last_to_first() {
        assert_eq!(
            command(false, 30, [0x25, 0xaa, 0xbb, 0xcc]),
            [0, 0xd0, 0xda, 30, 0xcc, 0xbb, 0xaa, 0x25]
        );
    }

    #[test]
    fn v2_reports_press_on_the_actuated_bit_and_release_when_missing() {
        let (mut deck, analog, _) = deck("wooting-two-he", 0xff53, 0x1337, None);
        // Q (row 2, col 1) past actuation; W (row 2, col 2) only touched.
        analog.queue_read(v2_report(&[
            v2_entry(2, 1, true, 700),
            v2_entry(2, 2, false, 100),
        ]));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 2 * 21 + 1,
                pressed: true
            }]
        );
        // A key listed twice (bound to two codes) counts once.
        analog.queue_read(v2_report(&[
            v2_entry(2, 1, false, 900),
            v2_entry(2, 1, true, 900),
        ]));
        assert!(deck.poll(0).unwrap().is_empty());
        // A timeout changes nothing; an empty report releases everything.
        assert!(deck.poll(0).unwrap().is_empty());
        analog.queue_read(vec![0; 64]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 43,
                pressed: false
            }]
        );
    }

    #[test]
    fn v2_positions_use_the_boards_own_width() {
        let (mut deck, analog, _) = deck("wooting-60he", 0xff53, 0x1337, None);
        analog.queue_read(v2_report(&[
            v2_entry(1, 13, true, 1023),
            // Column 14 is past a 60HE's matrix.
            v2_entry(1, 14, true, 1023),
        ]));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 14 + 13,
                pressed: true
            }]
        );
        // A short, ragged report is read as far as it goes: row 2, col 0
        // down, and row 1, col 13 (no longer listed) up.
        analog.queue_read(vec![0x40, 0x04, 0x41, 0x00, 0x2e, 0x04]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 27,
                    pressed: false
                },
                Input::Key {
                    index: 28,
                    pressed: true
                }
            ]
        );
    }

    #[test]
    fn v1_reports_place_usages_by_the_chart_with_hysteresis() {
        let (mut deck, analog, _) = deck("wooting-two-legacy", 0xff54, 0x1337, None);
        // Esc at 0x80 * 1.2 / 255 = 60% depth; F1 at 10%.
        analog.queue_read(vec![0x00, 0x29, 0x80, 0x00, 0x3a, 0x15]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 0,
                pressed: true
            }]
        );
        // 30% * 255 / 1.2 = 64: still down until under 25%.
        analog.queue_read(vec![0x00, 0x29, 64]);
        assert!(deck.poll(0).unwrap().is_empty());
        analog.queue_read(vec![0x00, 0x29, 40]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 0,
                pressed: false
            }]
        );
        // Num Enter (row 4, col 20) and an unknown usage.
        analog.queue_read(vec![0x00, 0x58, 0xff, 0x00, 0xa5, 0xff, 0x01, 0x29, 0xff]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 4 * 21 + 20,
                pressed: true
            }]
        );
    }

    #[test]
    fn v1_on_newer_firmware_is_not_scaled_up() {
        let (mut deck, analog, _) = deck("wooting-two", 0xff54, 0x1337, None);
        // 101 / 255 is just under the 40% press depth (1.2x would be 48%).
        analog.queue_read(vec![0x00, 0x29, 0x65]);
        assert!(deck.poll(0).unwrap().is_empty());
        analog.queue_read(vec![0x00, 0x29, 0x67]);
        assert_eq!(deck.poll(0).unwrap().len(), 1);
    }

    #[test]
    fn the_matrix_frame_is_21_columns_of_little_endian_rgb565() {
        let (mut deck, _, lights) = deck("wooting-80he", 0xff53, 0x1337, None);
        deck.set_face(0, Some(&face([0xff, 0x00, 0x00]))).unwrap();
        // Row 1, column 0 on a 17-column board is frame cell 21.
        deck.set_face(17, Some(&face([0x00, 0xff, 0x08]))).unwrap();
        deck.flush().unwrap();
        let writes = lights.writes();
        assert_eq!(writes.len(), 1);
        let frame = &writes[0];
        assert_eq!(frame.len(), 257);
        assert_eq!(&frame[..4], &[0, 0xd0, 0xda, 0x0b]);
        assert_eq!(&frame[4..6], &[0x00, 0xf8]);
        assert_eq!(&frame[4 + 42..4 + 44], &[0xe1, 0x07]);
        assert!(frame[6..4 + 42].iter().all(|b| *b == 0));
        // Nothing changed, nothing sent.
        deck.flush().unwrap();
        assert_eq!(lights.writes().len(), 1);
        // Half brightness halves each channel before encoding.
        deck.set_brightness(50).unwrap();
        assert_eq!(&lights.writes()[1][4..6], &[0x00, 0x78]);
    }

    #[test]
    fn small_packet_boards_split_the_frame_in_four() {
        let (mut deck, _, lights) = deck("wooting-two-he-arm", 0xff53, 0x1337, None);
        deck.set_face(1, Some(&face([0xff, 0xff, 0xff]))).unwrap();
        deck.flush().unwrap();
        let writes = lights.writes();
        assert_eq!(writes.len(), 4);
        assert!(writes.iter().all(|w| w.len() == 65 && w[0] == 0));
        assert_eq!(&writes[0][1..6], &[0xd0, 0xda, 0x0b, 0, 0]);
        assert_eq!(&writes[0][6..8], &[0xff, 0xff]);
    }

    #[test]
    fn the_report_descriptor_overrides_the_packet_default() {
        // Report Count with two data bytes (256): big packets on an ARM board.
        let big = vec![
            0x06, 0x37, 0x13, 0x09, 0x01, 0xa1, 0x01, 0x96, 0x00, 0x01, 0xc0,
        ];
        let (mut deck, _, lights) = deck("wooting-two-he-arm", 0xff53, 0x1337, Some(big));
        deck.flush().unwrap();
        assert_eq!(lights.writes()[0].len(), 257);
        assert_eq!(descriptor_says_small(&[0x05, 0x01, 0x95, 0x40]), Some(true));
        // A 0x95 inside an item's data is not a Report Count.
        assert_eq!(
            descriptor_says_small(&[0x06, 0x95, 0xff, 0x96, 0x00, 0x01]),
            Some(false)
        );
        assert_eq!(descriptor_says_small(&[0x06, 0x37]), None);
    }

    #[test]
    fn multi_report_frames_are_report_five_of_2047_bytes() {
        let (mut deck, _, lights) = deck("wooting-60he-v2", 0xff53, 0xff55, None);
        deck.set_face(0, Some(&face([0, 0, 0xff]))).unwrap();
        deck.flush().unwrap();
        let frame = &lights.writes()[0];
        assert_eq!(frame.len(), 2047);
        assert_eq!(&frame[..6], &[5, 0xd1, 0xda, 0x0b, 0x1f, 0x00]);
    }

    #[test]
    fn legacy_frames_are_crc_checked_led_memory() {
        let (mut deck, _, lights) = deck("wooting-two-legacy", 0xff54, 0x1337, None);
        // Esc is LED 0: part 0, offset 0.
        deck.set_face(0, Some(&face([0xff, 0x80, 0x10]))).unwrap();
        // Left Shift (row 4, col 0) is LED 9 and also lights ISO shift (LED 7).
        deck.set_face(4 * 21, Some(&face([0xff, 0xff, 0xff])))
            .unwrap();
        // Enter (row 3, col 13) is LED 65, copied to ISO enter (LED 62).
        deck.set_face(3 * 21 + 13, Some(&face([0xff, 0, 0])))
            .unwrap();
        deck.flush().unwrap();
        let writes = lights.writes();
        assert_eq!(writes.len(), 5);
        for (part, report) in writes.iter().enumerate() {
            assert_eq!(report.len(), 129);
            assert_eq!(
                &report[..6],
                &[0, 0xd0, 0xda, 0x0b, part as u8 / 2, (part as u8 % 2) * 96]
            );
            let crc = crc16_ccitt(&report[..127]);
            assert_eq!(&report[127..], &crc.to_le_bytes());
        }
        let part0 = &writes[0][6..102];
        // RGB565 keeps 0xf8, 0x80 and 0x10; then the gamma curve.
        assert_eq!(
            [part0[0], part0[0x10], part0[0x20]],
            [LEGACY_GAMMA[0xf8], LEGACY_GAMMA[0x80], LEGACY_GAMMA[0x10]]
        );
        assert_eq!(part0[0x0b], LEGACY_GAMMA[0xf8]);
        assert_eq!(part0[0x09], LEGACY_GAMMA[0xf8]);
        let part2 = &writes[2][6..102];
        assert_eq!(part2[0x35], LEGACY_GAMMA[0xf8]);
        assert_eq!(part2[0x32], LEGACY_GAMMA[0xf8]);
        assert_eq!(part2[0x32 + 0x10], 0);
    }

    #[test]
    fn the_legacy_one_sends_four_parts_and_skips_the_twos_leds() {
        let (mut deck, _, lights) = deck("wooting-one-legacy", 0xff54, 0x1337, None);
        // Row 0, col 16 is LED 80: part 3, offset 0x0a.
        deck.set_face(16, Some(&face([0xff, 0xff, 0xff]))).unwrap();
        deck.flush().unwrap();
        let writes = lights.writes();
        assert_eq!(writes.len(), 4);
        assert_eq!(writes[3][6 + 0x0a], LEGACY_GAMMA[0xf8]);
        assert_eq!(writes[3][6 + 0x2a], LEGACY_GAMMA[0xf8]);
        // Row 0, col 17 is LED 113, which only the Two has.
        let mut words = [0u16; FRAME_CELLS];
        words[17] = 0xffff;
        let one = legacy_reports(&words, 4, LEGACY_ONE_LAST_LED);
        assert!(one.iter().all(|w| w[6..102].iter().all(|b| *b == 0)));
        let two = legacy_reports(&words, 5, LEGACY_TWO_LAST_LED);
        // 113 is part 4, LED 17 of it: offset 0x35.
        assert_eq!(two[4][6 + 0x35], LEGACY_GAMMA[0xf8]);
    }

    #[test]
    fn the_crc_is_ccitt_from_zero() {
        // The standard XMODEM check value.
        assert_eq!(crc16_ccitt(b"123456789"), 0x31c3);
    }

    #[test]
    fn without_lights_the_keys_are_still_read() {
        let analog = Recorder::default();
        analog.0.lock().unwrap().usage_page = Some(0xff53);
        let mut deck = Wooting::new(
            by_id("wooting-two").unwrap(),
            Box::new(analog.clone()),
            None,
        );
        deck.set_face(0, Some(&face([1, 2, 3]))).unwrap();
        deck.flush().unwrap();
        analog.queue_read(v2_report(&[v2_entry(0, 0, true, 512)]));
        assert_eq!(deck.poll(0).unwrap().len(), 1);
    }

    #[test]
    fn the_two_chart_names_every_cell() {
        let two = by_id("wooting-two-lekker").unwrap();
        let names = key_names(two).unwrap();
        assert_eq!(names.len(), usize::from(two.keys));
        assert_eq!(names[20], "Mode");
        assert_eq!(names[4 * 21 + 20], "Num Enter");
        assert_eq!(cell_of_usage(0x2c), Some((5, 6)));
        assert!(key_names(by_id("wooting-80he").unwrap()).is_none());
    }
}
