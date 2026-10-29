//! Native Instruments Maschine controllers in HID mode: the Maschine MK2
//! (17cc:1140), Maschine Mikro MK2 (17cc:1200) and Maschine Jam (17cc:1500).
//!
//! Each is one HID device with no handshake, init command or keep-alive. Every
//! input report starts with its report id, and every write is the report id
//! followed by its payload:
//!
//! - Report `01` holds the buttons as a bitfield, least significant bit first,
//!   with the encoders as absolute counters (a 4-bit counter for the main
//!   encoder; the MK2's eight knobs are 10-bit little-endian counters). Ryu
//!   seeds every counter from the first report and turns later changes into a
//!   wrapped, signed delta.
//! - The MK2 and Mikro pads are pressure sensors streaming report `20`: 32
//!   little-endian words, each with the pad in its top four bits and a 12-bit
//!   pressure below. Pad 0 is the top-left pad (NI's "Pad 13"). Ryu presses a
//!   pad above a pressure of 200 and releases it below 100 so it does not
//!   chatter.
//! - The Jam's eight touch strips are report `02`, six bytes a strip with the
//!   position (0 when untouched, up to about 1023) at bytes 2 and 3; Ryu turns a
//!   finger sliding along one into dial turns.
//! - Lights are whole reports: one value per single-colour LED, R G B per
//!   colour LED, or (on the Jam) an index into the Jam's own palette of hues at
//!   four strengths. None has a brightness command, so Ryu scales the values.
//! - The displays take 256-byte chunks behind a 9-byte header: the MK2's two
//!   256 x 64 panels (reports `E0` and `E1`) one bit per pixel in rows, most
//!   significant bit leftmost; the Mikro's 128 x 64 panel (report `E0`) in
//!   8-row pages, one byte per column with bit 0 at the top.
//!
//! The Maschine MK1 is a vendor-class USB device, not HID, and is not driven.
//! NI's own background services may hold the device or switch it to MIDI
//! mode; Ryu only drives it while nothing else does.
//!
//! Facts per shaduzlabs/cabl (MIT), cross-checked for the Mikro MK2 against
//! wrl/maschine.rs (LGPL-3.0); no code was copied.

use std::sync::OnceLock;

use super::{face, Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, KeyImage};

pub const MK2_ID: &str = "ni-maschine-mk2";
pub const MIKRO_MK2_ID: &str = "ni-maschine-mikro-mk2";
pub const JAM_ID: &str = "ni-maschine-jam";

/// The pads stream reports while touched, so drain generously.
const MAX_REPORTS_PER_POLL: usize = 128;
/// Bigger than any input report (the pad report is 65 bytes).
const READ_BUFFER: usize = 128;
const BUTTONS_REPORT: u8 = 0x01;
const STRIPS_REPORT: u8 = 0x02;
const PADS_REPORT: u8 = 0x20;
/// A pad presses above this pressure (of 4095) and releases below `PAD_RELEASE`.
const PAD_PRESS: u16 = 200;
const PAD_RELEASE: u16 = 100;
/// Words in a pad report: two readings of each of the 16 pads.
const PAD_WORDS: usize = 32;
/// How far a finger moves along a Jam touch strip (of about 1024) per dial step.
const STRIP_STEP: i32 = 32;
const STRIPS: usize = 8;
/// Bytes per strip in the Jam's strip report.
const STRIP_BYTES: usize = 6;
/// The first display report; the MK2's second display is `E1`.
const DISPLAY_REPORT: u8 = 0xe0;
/// Image bytes per display write.
const DISPLAY_CHUNK: usize = 256;
/// The MK2's displays: 256 x 64, 32 bytes per row.
const MK2_DISPLAY_ROW: usize = 32;
const MK2_DISPLAY_ROWS: usize = 64;
/// The Mikro's 128 x 64 display: eight pages of 128 bytes.
const MIKRO_DISPLAY_BYTES: usize = 1024;
/// maschine.rs never drives the Mikro's RGB LEDs past half, so Ryu does not
/// either until that is checked on hardware.
const MIKRO_RGB_CEILING: u16 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Mk2,
    MikroMk2,
    Jam,
}

impl Kind {
    fn of(model: &DeviceModel) -> Option<Self> {
        match model.id {
            MK2_ID => Some(Self::Mk2),
            MIKRO_MK2_ID => Some(Self::MikroMk2),
            JAM_ID => Some(Self::Jam),
            _ => None,
        }
    }

    /// Each light report's id and payload length.
    const fn light_reports(self) -> &'static [(u8, usize)] {
        match self {
            Self::Mk2 => &[(0x80, 49), (0x81, 57), (0x82, 32)],
            Self::MikroMk2 => &[(0x80, 78)],
            Self::Jam => &[(0x80, 53), (0x81, 80), (0x82, 88)],
        }
    }

    /// The shortest button report the device sends.
    const fn buttons_len(self) -> usize {
        match self {
            Self::Mk2 => 25,
            Self::MikroMk2 => 6,
            Self::Jam => 17,
        }
    }

    /// Where the button bitfield starts and how many bytes it has.
    const fn button_bytes(self) -> (usize, usize) {
        match self {
            Self::Mk2 => (1, 6),
            Self::MikroMk2 => (1, 4),
            Self::Jam => (2, 15),
        }
    }

    /// The byte holding the main encoder's 4-bit counter, and its dial.
    const fn encoder(self) -> (usize, u8) {
        match self {
            Self::Mk2 => (8, 8),
            Self::MikroMk2 => (5, 0),
            Self::Jam => (1, 8),
        }
    }

    /// The button bit that is the main encoder's push, on decks that have one.
    const fn encoder_push(self) -> Option<u8> {
        match self {
            Self::Mk2 => None,
            Self::MikroMk2 => Some(11),
            Self::Jam => Some(116),
        }
    }

    const fn rgb_ceiling(self) -> u16 {
        match self {
            Self::MikroMk2 => MIKRO_RGB_CEILING,
            Self::Mk2 | Self::Jam => 100,
        }
    }

    fn layout(self) -> &'static [KeyDef] {
        static MK2: OnceLock<Vec<KeyDef>> = OnceLock::new();
        static MIKRO: OnceLock<Vec<KeyDef>> = OnceLock::new();
        static JAM: OnceLock<Vec<KeyDef>> = OnceLock::new();
        match self {
            Self::Mk2 => MK2.get_or_init(mk2_layout),
            Self::MikroMk2 => MIKRO.get_or_init(mikro_layout),
            Self::Jam => JAM.get_or_init(jam_layout),
        }
    }
}

/// Where a key's state comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// A pressure pad, by the pad number in its report word.
    Pad(u8),
    /// A bit of the button bitfield.
    Bit(u8),
}

/// A key's light, as a place in one of the light reports (`at` counts from the
/// first payload byte, after the report id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Light {
    None,
    /// One value, 0 to 255.
    Mono {
        report: u8,
        at: u8,
    },
    /// R, G, B, written `copies` times in a row (an MK2 group button has two LEDs).
    Rgb {
        report: u8,
        at: u8,
        copies: u8,
    },
    /// An index into the Jam's palette.
    Palette {
        report: u8,
        at: u8,
    },
}

const fn mono(report: u8, at: u8) -> Light {
    Light::Mono { report, at }
}

#[derive(Debug, Clone, Copy)]
struct KeyDef {
    name: &'static str,
    source: Source,
    light: Light,
    /// Row and column on the Companion's grid.
    place: (u8, u8),
}

/// The MK2 and Mikro pads in report order: the top row is NI's pads 13 to 16.
const PAD_NAMES: [&str; 16] = [
    "Pad 13", "Pad 14", "Pad 15", "Pad 16", "Pad 9", "Pad 10", "Pad 11", "Pad 12", "Pad 5",
    "Pad 6", "Pad 7", "Pad 8", "Pad 1", "Pad 2", "Pad 3", "Pad 4",
];
const GROUP_NAMES: [&str; 8] = [
    "Group A", "Group B", "Group C", "Group D", "Group E", "Group F", "Group G", "Group H",
];

/// The MK2's 48 buttons, in bitfield order (bit 0 first), with their lights.
const MK2_BUTTONS: [(&str, Light); 48] = [
    ("Screen button 1", mono(0x82, 8)),
    ("Screen button 2", mono(0x82, 9)),
    ("Screen button 3", mono(0x82, 10)),
    ("Screen button 4", mono(0x82, 11)),
    ("Screen button 5", mono(0x82, 12)),
    ("Screen button 6", mono(0x82, 13)),
    ("Screen button 7", mono(0x82, 14)),
    ("Screen button 8", mono(0x82, 15)),
    ("Control", mono(0x82, 0)),
    ("Step", mono(0x82, 1)),
    ("Browse", mono(0x82, 2)),
    ("Sampling", mono(0x82, 3)),
    ("Browse left", mono(0x82, 4)),
    ("Browse right", mono(0x82, 5)),
    ("All", mono(0x82, 6)),
    ("Auto write", mono(0x82, 7)),
    ("Volume", mono(0x82, 24)),
    ("Swing", mono(0x82, 25)),
    ("Tempo", mono(0x82, 26)),
    ("Master left", mono(0x82, 27)),
    ("Master right", mono(0x82, 28)),
    ("Enter", mono(0x82, 29)),
    ("Note repeat", mono(0x82, 30)),
    ("Main", Light::None),
    (GROUP_NAMES[0], mk2_group(0)),
    (GROUP_NAMES[1], mk2_group(1)),
    (GROUP_NAMES[2], mk2_group(2)),
    (GROUP_NAMES[3], mk2_group(3)),
    (GROUP_NAMES[4], mk2_group(4)),
    (GROUP_NAMES[5], mk2_group(5)),
    (GROUP_NAMES[6], mk2_group(6)),
    (GROUP_NAMES[7], mk2_group(7)),
    ("Restart", mono(0x81, 48)),
    ("Transport left", mono(0x81, 49)),
    ("Transport right", mono(0x81, 50)),
    ("Grid", mono(0x81, 51)),
    ("Play", mono(0x81, 52)),
    ("Rec", mono(0x81, 53)),
    ("Erase", mono(0x81, 54)),
    ("Shift", mono(0x81, 55)),
    ("Scene", mono(0x82, 16)),
    ("Pattern", mono(0x82, 17)),
    ("Pad mode", mono(0x82, 18)),
    ("Navigate", mono(0x82, 19)),
    ("Duplicate", mono(0x82, 20)),
    ("Select", mono(0x82, 21)),
    ("Solo", mono(0x82, 22)),
    ("Mute", mono(0x82, 23)),
];

/// An MK2 group button: two RGB LEDs, six bytes apart per group.
const fn mk2_group(group: u8) -> Light {
    Light::Rgb {
        report: 0x81,
        at: group * 6,
        copies: 2,
    }
}

/// The Mikro MK2's 28 buttons as (bitfield bit, name, light), in bit order.
/// Bits 8 to 10 are unused and bit 11 is the encoder push (dial 1).
const MIKRO_BUTTONS: [(u8, &str, Light); 28] = [
    (0, "Shift", mono(0x80, 21)),
    (1, "Erase", mono(0x80, 20)),
    (2, "Rec", mono(0x80, 19)),
    (3, "Play", mono(0x80, 18)),
    (4, "Grid", mono(0x80, 17)),
    (5, "Transport right", mono(0x80, 16)),
    (6, "Transport left", mono(0x80, 15)),
    (7, "Restart", mono(0x80, 14)),
    (12, "Note repeat", mono(0x80, 13)),
    (13, "Sampling", mono(0x80, 12)),
    (14, "Browse", mono(0x80, 11)),
    (
        15,
        "Group",
        Light::Rgb {
            report: 0x80,
            at: 8,
            copies: 1,
        },
    ),
    (16, "Main", mono(0x80, 7)),
    (17, "Nav right", mono(0x80, 6)),
    (18, "Nav left", mono(0x80, 5)),
    (19, "Nav", mono(0x80, 4)),
    (20, "Control", mono(0x80, 3)),
    (21, "F3", mono(0x80, 2)),
    (22, "F2", mono(0x80, 1)),
    (23, "F1", mono(0x80, 0)),
    (24, "Mute", mono(0x80, 29)),
    (25, "Solo", mono(0x80, 28)),
    (26, "Select", mono(0x80, 27)),
    (27, "Duplicate", mono(0x80, 26)),
    (28, "View", mono(0x80, 25)),
    (29, "Pad mode", mono(0x80, 24)),
    (30, "Pattern", mono(0x80, 23)),
    (31, "Scene", mono(0x80, 22)),
];

/// The Jam's single-colour buttons as (bitfield bit, name, offset in report
/// `80`), in bit order. Bit 115 is the encoder touch (ignored) and bit 116
/// its push (dial 9).
const JAM_BUTTONS: [(u8, &str, u8); 35] = [
    (0, "Arrange", 0),
    (9, "Step", 1),
    (10, "Pad mode", 2),
    (11, "Clear", 3),
    (12, "Duplicate", 4),
    (13, "Nav up", 5),
    (14, "Nav left", 6),
    (15, "Nav right", 7),
    (16, "Nav down", 8),
    (17, "Arp", 9),
    (90, "Mst", 10),
    (91, "Grp", 11),
    (92, "In", 12),
    (93, "Cue", 14),
    (94, "Browse", 16),
    (95, "Macro", 17),
    (96, "Level", 18),
    (97, "Aux", 19),
    (98, "Control", 20),
    (99, "Auto", 21),
    (100, "Perform", 22),
    (101, "Variation", 23),
    (102, "Lock", 24),
    (103, "Tune", 25),
    (104, "Swing", 26),
    (105, "Shift", 27),
    (106, "Play", 28),
    (107, "Rec", 29),
    (108, "Transport left", 30),
    (109, "Transport right", 31),
    (110, "Tempo", 32),
    (111, "Grid", 33),
    (112, "Solo", 34),
    (113, "Mute", 35),
    (114, "Select", 36),
];
const JAM_SCENE_NAMES: [&str; 8] = [
    "Scene 1", "Scene 2", "Scene 3", "Scene 4", "Scene 5", "Scene 6", "Scene 7", "Scene 8",
];

/// The 16 pads, on the middle four columns of the top four rows.
fn pads(report: u8, first: u8) -> impl Iterator<Item = KeyDef> {
    (0u8..16).map(move |pad| KeyDef {
        name: PAD_NAMES[usize::from(pad)],
        source: Source::Pad(pad),
        light: Light::Rgb {
            report,
            at: first + 3 * pad,
            copies: 1,
        },
        place: (pad / 4, 2 + pad % 4),
    })
}

/// Keys 1 to 16 are the pads; then the buttons in bitfield order, a row of
/// eight per bitfield byte.
fn mk2_layout() -> Vec<KeyDef> {
    pads(0x80, 0)
        .chain((0u8..).zip(MK2_BUTTONS).map(|(bit, (name, light))| KeyDef {
            name,
            source: Source::Bit(bit),
            light,
            place: (4 + bit / 8, bit % 8),
        }))
        .collect()
}

/// Keys 1 to 16 are the pads; then the 28 buttons in bitfield order, a row per
/// bitfield byte, the highest bit leftmost (F1 before F2).
fn mikro_layout() -> Vec<KeyDef> {
    pads(0x80, 30)
        .chain(MIKRO_BUTTONS.iter().map(|(bit, name, light)| KeyDef {
            name,
            source: Source::Bit(*bit),
            light: *light,
            place: (4 + bit / 8, 7 - bit % 8),
        }))
        .collect()
}

/// Keys 1 to 64 are the 8 x 8 matrix, top-left first; then the scene buttons
/// above it, the group buttons below it, and the other buttons in bit order.
fn jam_layout() -> Vec<KeyDef> {
    let matrix = (0u8..64).map(|cell| KeyDef {
        name: jam_pad_names()[usize::from(cell)],
        source: Source::Bit(18 + cell),
        light: Light::Palette {
            report: 0x81,
            at: 8 + cell,
        },
        place: (1 + cell / 8, cell % 8),
    });
    let scenes = (0u8..8).map(|scene| KeyDef {
        name: JAM_SCENE_NAMES[usize::from(scene)],
        source: Source::Bit(1 + scene),
        light: Light::Palette {
            report: 0x81,
            at: scene,
        },
        place: (0, scene),
    });
    let groups = (0u8..8).map(|group| KeyDef {
        name: GROUP_NAMES[usize::from(group)],
        source: Source::Bit(82 + group),
        light: Light::Palette {
            report: 0x81,
            at: 72 + group,
        },
        place: (9, group),
    });
    let others = (0u8..)
        .zip(JAM_BUTTONS)
        .map(|(at, (bit, name, led))| KeyDef {
            name,
            source: Source::Bit(bit),
            light: mono(0x80, led),
            place: (10 + at / 8, at % 8),
        });
    matrix.chain(scenes).chain(groups).chain(others).collect()
}

/// "Pad 1" to "Pad 64", made once.
fn jam_pad_names() -> &'static [&'static str] {
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        (1..=64)
            .map(|pad| &*Box::leak(format!("Pad {pad}").into_boxed_str()))
            .collect()
    })
}

/// Each key's name, on a Maschine.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    Kind::of(model).map(|kind| kind.layout().iter().map(|key| key.name).collect())
}

/// Each key's cell in the model's grid, on a Maschine.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    Kind::of(model).map(|kind| {
        kind.layout()
            .iter()
            .map(|key| u16::from(key.place.0) * u16::from(model.cols) + u16::from(key.place.1))
            .collect()
    })
}

/// The keys with a light, on a Maschine (all but the MK2's Main button).
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    Kind::of(model).map(|kind| {
        (0u8..)
            .zip(kind.layout())
            .filter(|(_, key)| key.light != Light::None)
            .map(|(index, _)| index)
            .collect()
    })
}

/// The Jam palette's hues, in degrees: red, orange, light orange, yellow-orange,
/// yellow, lime, green, mint, cyan, turquoise, blue, plum, violet, purple,
/// magenta and fuchsia, four entries each from index 4.
const JAM_HUES: [u16; 16] = [
    0, 12, 24, 40, 55, 85, 120, 140, 170, 200, 240, 255, 270, 285, 310, 340,
];
/// Each hue's four entries, dim to bright and then a paler bright one, as
/// (percent of full strength, percent mixed with white).
const JAM_LEVELS: [(u16, u16); 4] = [(15, 0), (40, 0), (100, 0), (100, 30)];
/// The grey and white entries after the hues, in percent.
const JAM_WHITES: [u16; 3] = [15, 40, 90];
/// Entries 0 to 70: four offs, sixteen hues of four, three whites. Entry 71 (a
/// pale cyan) is left out.
const JAM_PALETTE_LEN: u8 = 71;

/// A fully saturated colour at `hue` degrees.
fn hue_rgb(hue: u16) -> [u16; 3] {
    let sector = hue / 60;
    let rising = (hue % 60) * 255 / 60;
    let falling = 255 - rising;
    match sector {
        0 => [255, rising, 0],
        1 => [falling, 255, 0],
        2 => [0, 255, rising],
        3 => [0, falling, 255],
        4 => [rising, 0, 255],
        _ => [255, 0, falling],
    }
}

/// The colour the Jam shows for a palette index (an approximation).
fn jam_palette_color(index: u8) -> [u8; 3] {
    let to_u8 = |value: u16| u8::try_from(value.min(255)).unwrap_or(u8::MAX);
    match index {
        4..=67 => {
            let hue = JAM_HUES[usize::from((index - 4) / 4)];
            let (strength, white) = JAM_LEVELS[usize::from((index - 4) % 4)];
            hue_rgb(hue).map(|channel| {
                let lit = channel * strength / 100;
                to_u8(lit + (255 - lit) * white / 100)
            })
        }
        68..=70 => [to_u8(255 * JAM_WHITES[usize::from(index - 68)] / 100); 3],
        _ => [0, 0, 0],
    }
}

/// The palette index nearest a colour; black is index 0 (off).
fn nearest_jam_color(color: [u8; 3]) -> u8 {
    let distance = |entry: [u8; 3]| -> u32 {
        entry
            .iter()
            .zip(color)
            .map(|(a, b)| u32::from(a.abs_diff(b)).pow(2))
            .sum()
    };
    (0..JAM_PALETTE_LEN)
        .min_by_key(|index| distance(jam_palette_color(*index)))
        .unwrap_or(0)
}

/// `value` scaled by a percentage.
fn scale(value: u8, percent: u16) -> u8 {
    u8::try_from(u16::from(value) * percent.min(100) / 100).unwrap_or(u8::MAX)
}

/// The signed step between two readings of a counter that wraps at `modulus`
/// (a power of two), clamped to what one turn can carry.
fn wrapped_delta(old: u16, new: u16, modulus: u16) -> i8 {
    let half = i32::from(modulus / 2);
    let raw = (i32::from(new) - i32::from(old) + half).rem_euclid(i32::from(modulus)) - half;
    i8::try_from(raw.clamp(i32::from(i8::MIN), i32::from(i8::MAX))).unwrap_or(0)
}

pub struct Maschine {
    model: &'static DeviceModel,
    kind: Kind,
    transport: Box<dyn Transport>,
    keys: &'static [KeyDef],
    /// Each key's light colour before brightness; `None` for off.
    colors: Vec<Option<[u8; 3]>>,
    brightness: u8,
    /// The light reports last sent, in `Kind::light_reports` order.
    sent_lights: Vec<Option<Vec<u8>>>,
    /// The info screen's encoded frame, and each display's bytes last sent.
    screen: Vec<u8>,
    sent_displays: Vec<Option<Vec<u8>>>,
    /// The button bits held at the last button report.
    held: u128,
    pads: [bool; 16],
    encoder: Option<u8>,
    knobs: [Option<u16>; 8],
    strips: [Option<u16>; STRIPS],
    /// Strip movement not yet turned into a whole step.
    strip_rest: [i32; STRIPS],
}

impl Maschine {
    /// A model that is not a Maschine is driven as an MK2.
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model).unwrap_or(Kind::Mk2);
        let keys = kind.layout();
        let screen_len = match kind {
            Kind::Mk2 => 2 * MK2_DISPLAY_ROW * MK2_DISPLAY_ROWS,
            Kind::MikroMk2 => MIKRO_DISPLAY_BYTES,
            Kind::Jam => 0,
        };
        let displays = match kind {
            Kind::Mk2 => 2,
            Kind::MikroMk2 => 1,
            Kind::Jam => 0,
        };
        Self {
            model,
            kind,
            transport,
            keys,
            colors: vec![None; keys.len()],
            brightness: 100,
            sent_lights: vec![None; kind.light_reports().len()],
            screen: vec![0; screen_len],
            sent_displays: vec![None; displays],
            held: 0,
            pads: [false; 16],
            encoder: None,
            knobs: [None; 8],
            strips: [None; STRIPS],
            strip_rest: [0; STRIPS],
        }
    }

    fn parse(&mut self, report: &[u8], events: &mut Vec<Input>) {
        match (self.kind, report.first().copied()) {
            (_, Some(BUTTONS_REPORT)) => self.parse_buttons(report, events),
            (Kind::Mk2 | Kind::MikroMk2, Some(PADS_REPORT)) => self.parse_pads(report, events),
            (Kind::Jam, Some(STRIPS_REPORT)) => self.parse_strips(report, events),
            _ => {}
        }
    }

    fn parse_buttons(&mut self, report: &[u8], events: &mut Vec<Input>) {
        if report.len() < self.kind.buttons_len() {
            return;
        }
        let (first, len) = self.kind.button_bytes();
        let mut bits = 0u128;
        for (at, byte) in report.iter().skip(first).take(len).enumerate() {
            bits |= u128::from(*byte) << (8 * at);
        }
        let changed = bits ^ self.held;
        for (index, key) in (0u8..).zip(self.keys) {
            if let Source::Bit(bit) = key.source {
                let mask = 1u128 << bit;
                if changed & mask != 0 {
                    events.push(Input::Key {
                        index,
                        pressed: bits & mask != 0,
                    });
                }
            }
        }
        let (encoder_at, dial) = self.kind.encoder();
        if let Some(bit) = self.kind.encoder_push() {
            let mask = 1u128 << bit;
            if changed & mask != 0 {
                events.push(Input::DialPress {
                    index: dial,
                    pressed: bits & mask != 0,
                });
            }
        }
        self.held = bits;
        if let Some(value) = report.get(encoder_at).map(|byte| byte & 0x0f) {
            if let Some(old) = self.encoder.replace(value) {
                let delta = wrapped_delta(u16::from(old), u16::from(value), 16);
                if delta != 0 {
                    events.push(Input::DialTurn { index: dial, delta });
                }
            }
        }
        if self.kind == Kind::Mk2 {
            self.parse_knobs(report, events);
        }
    }

    /// The MK2's eight knobs: 10-bit counters from byte 9, two bytes each.
    fn parse_knobs(&mut self, report: &[u8], events: &mut Vec<Input>) {
        for (knob, stored) in (0u8..).zip(self.knobs.iter_mut()) {
            let at = 9 + 2 * usize::from(knob);
            let (Some(low), Some(high)) = (report.get(at), report.get(at + 1)) else {
                return;
            };
            let value = u16::from_le_bytes([*low, *high]) & 0x03ff;
            if let Some(old) = stored.replace(value) {
                let delta = wrapped_delta(old, value, 1024);
                if delta != 0 {
                    events.push(Input::DialTurn { index: knob, delta });
                }
            }
        }
    }

    /// Pressure words: the pad is the top nibble of the second byte. Each pad
    /// appears twice, so the firmer reading of the two counts.
    fn parse_pads(&mut self, report: &[u8], events: &mut Vec<Input>) {
        let mut pressure: [Option<u16>; 16] = [None; 16];
        for word in 0..PAD_WORDS {
            let (Some(low), Some(high)) = (report.get(1 + 2 * word), report.get(2 + 2 * word))
            else {
                break;
            };
            let pad = usize::from(high >> 4);
            let value = (u16::from(high & 0x0f) << 8) | u16::from(*low);
            if let Some(slot) = pressure.get_mut(pad) {
                *slot = Some(slot.map_or(value, |seen| seen.max(value)));
            }
        }
        for (pad, value) in (0u8..).zip(pressure) {
            let Some(value) = value else {
                continue;
            };
            let held = &mut self.pads[usize::from(pad)];
            let now = if *held {
                value >= PAD_RELEASE
            } else {
                value > PAD_PRESS
            };
            if now != *held {
                *held = now;
                if let Some(index) = self
                    .keys
                    .iter()
                    .position(|key| key.source == Source::Pad(pad))
                {
                    events.push(Input::Key {
                        index: u8::try_from(index).unwrap_or(u8::MAX),
                        pressed: now,
                    });
                }
            }
        }
    }

    /// The Jam's touch strips: sliding a finger along one turns its dial.
    fn parse_strips(&mut self, report: &[u8], events: &mut Vec<Input>) {
        for (strip, index) in (0..STRIPS).zip(0u8..) {
            let at = 1 + STRIP_BYTES * strip + 2;
            let (Some(low), Some(high)) = (report.get(at), report.get(at + 1)) else {
                return;
            };
            let position = u16::from_le_bytes([*low, *high]);
            if position == 0 {
                self.strips[strip] = None;
                self.strip_rest[strip] = 0;
                continue;
            }
            if let Some(old) = self.strips[strip].replace(position) {
                let moved = self.strip_rest[strip] + i32::from(position) - i32::from(old);
                let steps = moved / STRIP_STEP;
                self.strip_rest[strip] = moved - steps * STRIP_STEP;
                if steps != 0 {
                    events.push(Input::DialTurn {
                        index,
                        delta: i8::try_from(steps.clamp(-127, 127)).unwrap_or(0),
                    });
                }
            }
        }
    }

    fn info_image(&self) -> Option<(u8, KeyImage)> {
        self.model.info_key().zip(self.model.info_image)
    }

    /// Every light report, built from the key colours and the brightness.
    fn light_reports(&self) -> Vec<Vec<u8>> {
        let layout = self.kind.light_reports();
        let mut reports: Vec<Vec<u8>> = layout
            .iter()
            .map(|(id, len)| {
                let mut report = vec![0u8; len + 1];
                report[0] = *id;
                report
            })
            .collect();
        let brightness = u16::from(self.brightness);
        for (key, color) in self.keys.iter().zip(&self.colors) {
            let Some(color) = color else {
                continue;
            };
            let report_id = match key.light {
                Light::None => continue,
                Light::Mono { report, .. }
                | Light::Rgb { report, .. }
                | Light::Palette { report, .. } => report,
            };
            let Some(report) = layout
                .iter()
                .position(|(id, _)| *id == report_id)
                .and_then(|at| reports.get_mut(at))
            else {
                continue;
            };
            let payload = report.get_mut(1..).unwrap_or_default();
            match key.light {
                Light::Mono { at, .. } => {
                    let level = scale(color[0].max(color[1]).max(color[2]), brightness);
                    if let Some(slot) = payload.get_mut(usize::from(at)) {
                        *slot = level;
                    }
                }
                Light::Rgb { at, copies, .. } => {
                    let ceiling = brightness * self.kind.rgb_ceiling() / 100;
                    let rgb = color.map(|channel| scale(channel, ceiling));
                    for copy in 0..usize::from(copies) {
                        let start = usize::from(at) + 3 * copy;
                        if let Some(slot) = payload.get_mut(start..start + 3) {
                            slot.copy_from_slice(&rgb);
                        }
                    }
                }
                Light::Palette { at, .. } => {
                    let index = nearest_jam_color(color.map(|channel| scale(channel, brightness)));
                    if let Some(slot) = payload.get_mut(usize::from(at)) {
                        *slot = index;
                    }
                }
                Light::None => {}
            }
        }
        reports
    }

    /// Each display's bytes, in the order its chunks are sent.
    fn displays(&self) -> Vec<Vec<u8>> {
        match self.kind {
            // One 512-pixel-wide frame, 64 bytes a row: the left half of every
            // row is display 0, the right half display 1.
            Kind::Mk2 => (0..2)
                .map(|display| {
                    self.screen
                        .chunks(2 * MK2_DISPLAY_ROW)
                        .flat_map(|row| {
                            row.get(display * MK2_DISPLAY_ROW..(display + 1) * MK2_DISPLAY_ROW)
                                .unwrap_or_default()
                        })
                        .copied()
                        .collect()
                })
                .collect(),
            Kind::MikroMk2 => vec![self.screen.clone()],
            Kind::Jam => Vec::new(),
        }
    }

    /// One display's writes: 256-byte chunks, each behind its header.
    fn display_writes(&self, display: usize, bytes: &[u8]) -> Vec<Vec<u8>> {
        let report = DISPLAY_REPORT | u8::try_from(display).unwrap_or(0);
        (0u8..)
            .zip(bytes.chunks(DISPLAY_CHUNK))
            .map(|(chunk, data)| {
                let header = match self.kind {
                    // Eight rows of 32 bytes from row 8 * chunk.
                    Kind::Mk2 => [report, 0, 0, 8 * chunk, 0, 0x20, 0, 0x08, 0],
                    // Two pages of 128 columns from page 2 * chunk.
                    _ => [report, 0, 0, 2 * chunk, 0, 0x80, 0, 0x02, 0],
                };
                let mut write = header.to_vec();
                write.extend_from_slice(data);
                write.resize(header.len() + DISPLAY_CHUNK, 0);
                write
            })
            .collect()
    }
}

impl Driver for Maschine {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.parse(buf.get(..read).unwrap_or_default(), &mut events);
        }
        Ok(events)
    }

    /// The info key draws the display; every other key sets its light.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some((info, format)) = self.info_image() {
            if key == info {
                let mut frame = face::encode(face, &format)?;
                frame.resize(self.screen.len(), 0);
                self.screen = frame;
                return Ok(());
            }
        }
        if let Some(slot) = self.colors.get_mut(usize::from(key)) {
            *slot = face.map(|face| face.color);
        }
        Ok(())
    }

    /// Sends each light report and display that changed, whole.
    fn flush(&mut self) -> DriverResult<()> {
        for (at, report) in self.light_reports().into_iter().enumerate() {
            if self.sent_lights.get(at).and_then(Option::as_ref) != Some(&report) {
                self.transport.write(&report)?;
                if let Some(sent) = self.sent_lights.get_mut(at) {
                    *sent = Some(report);
                }
            }
        }
        for (display, bytes) in self.displays().into_iter().enumerate() {
            if self.sent_displays.get(display).and_then(Option::as_ref) != Some(&bytes) {
                for write in self.display_writes(display, &bytes) {
                    self.transport.write(&write)?;
                }
                if let Some(sent) = self.sent_displays.get_mut(display) {
                    *sent = Some(bytes);
                }
            }
        }
        Ok(())
    }

    /// No brightness command: Ryu scales every light value.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        let percent = percent.min(100);
        if percent == self.brightness {
            return Ok(());
        }
        self.brightness = percent;
        self.flush()
    }
}

impl Drop for Maschine {
    /// Leaves the lights off and the displays blank when Ryu lets go.
    fn drop(&mut self) {
        for (id, len) in self.kind.light_reports() {
            let mut report = vec![0u8; len + 1];
            report[0] = *id;
            let _ = self.transport.write(&report);
        }
        let blank = vec![0u8; self.screen.len() / self.sent_displays.len().max(1)];
        for display in 0..self.sent_displays.len() {
            for write in self.display_writes(display, &blank) {
                let _ = self.transport.write(&write);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn open(id: &str) -> (Maschine, Recorder) {
        let recorder = Recorder::default();
        (
            Maschine::new(by_id(id).unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    fn turn(index: u8, delta: i8) -> Input {
        Input::DialTurn { index, delta }
    }

    fn color(rgb: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color: rgb,
            image: None,
        }
    }

    /// An MK2 button report with the given bits set, the encoder and knobs at `counters`.
    fn mk2_buttons(bits: &[u8], encoder: u8, knobs: [u16; 8]) -> Vec<u8> {
        let mut report = vec![0u8; 25];
        report[0] = 0x01;
        for bit in bits {
            report[1 + usize::from(bit / 8)] |= 1 << (bit % 8);
        }
        report[8] = encoder;
        for (knob, value) in knobs.iter().enumerate() {
            report[9 + 2 * knob..11 + 2 * knob].copy_from_slice(&value.to_le_bytes());
        }
        report
    }

    /// A pad report with both readings of each pad at `pressures[pad]`.
    fn pad_report(pressures: [u16; 16]) -> Vec<u8> {
        let mut report = vec![0x20];
        for word in 0..32 {
            let pad = word % 16;
            let value = (u16::try_from(pad).unwrap() << 12) | pressures[pad];
            report.extend_from_slice(&value.to_le_bytes());
        }
        report
    }

    #[test]
    fn mk2_buttons_follow_the_bitfield_after_the_pads() {
        let (mut mk2, recorder) = open(MK2_ID);
        // Screen button 1, Main, Group A, Shift and Mute.
        recorder.queue_read(mk2_buttons(&[0, 23, 24, 39, 47], 0, [0; 8]));
        recorder.queue_read(mk2_buttons(&[23], 0, [0; 8]));
        assert_eq!(
            mk2.poll(0).unwrap(),
            vec![
                key(16, true),
                key(39, true),
                key(40, true),
                key(55, true),
                key(63, true),
                key(16, false),
                key(40, false),
                key(55, false),
                key(63, false),
            ]
        );
        let names = key_names(mk2.model).unwrap();
        assert_eq!(
            (names[0], names[12], names[16], names[39], names[40], names[63]),
            (
                "Pad 13",
                "Pad 1",
                "Screen button 1",
                "Main",
                "Group A",
                "Mute"
            )
        );
    }

    #[test]
    fn mk2_encoders_are_seeded_then_turn_by_wrapped_deltas() {
        let (mut mk2, recorder) = open(MK2_ID);
        let mut knobs = [0x200u16; 8];
        knobs[0] = 0x3ff;
        recorder.queue_read(mk2_buttons(&[], 0x0f, knobs));
        // The main encoder wraps 15 -> 1 (two steps up); knob 1 wraps 0x3ff -> 2
        // (three up); knob 8 goes down by five.
        knobs[0] = 2;
        knobs[7] = 0x200 - 5;
        recorder.queue_read(mk2_buttons(&[], 0x01, knobs));
        // Back down across the wrap.
        recorder.queue_read(mk2_buttons(&[], 0x0e, knobs));
        assert_eq!(
            mk2.poll(0).unwrap(),
            vec![turn(8, 2), turn(0, 3), turn(7, -5), turn(8, -3)]
        );
    }

    #[test]
    fn short_or_unknown_reports_are_ignored() {
        let (mut mk2, recorder) = open(MK2_ID);
        recorder.queue_read(vec![0x01, 0xff, 0xff]);
        recorder.queue_read(vec![0x20, 0x00]);
        recorder.queue_read(vec![0x02; 60]);
        recorder.queue_read(Vec::new());
        assert!(mk2.poll(0).unwrap().is_empty());
        let (mut jam, recorder) = open(JAM_ID);
        recorder.queue_read(vec![0x01; 10]);
        recorder.queue_read(vec![0x02, 1, 2]);
        assert!(jam.poll(0).unwrap().is_empty());
    }

    #[test]
    fn pads_press_and_release_with_hysteresis() {
        let (mut mk2, recorder) = open(MK2_ID);
        let mut pressures = [0u16; 16];
        pressures[0] = 150;
        recorder.queue_read(pad_report(pressures));
        pressures[0] = 201;
        pressures[12] = 4095;
        recorder.queue_read(pad_report(pressures));
        // Between the lines: still held.
        pressures[0] = 150;
        recorder.queue_read(pad_report(pressures));
        pressures[0] = 99;
        pressures[12] = 0;
        recorder.queue_read(pad_report(pressures));
        assert_eq!(
            mk2.poll(0).unwrap(),
            vec![key(0, true), key(12, true), key(0, false), key(12, false)]
        );
    }

    #[test]
    fn a_pad_is_found_by_its_nibble_not_its_word() {
        let (mut mikro, recorder) = open(MIKRO_MK2_ID);
        // Only the last word names pad 5, firmly; pad 0 reads lightly twice.
        let mut report = vec![0x20];
        for _ in 0..31 {
            report.extend_from_slice(&50u16.to_le_bytes());
        }
        report.extend_from_slice(&((5u16 << 12) | 0x800).to_le_bytes());
        recorder.queue_read(report);
        assert_eq!(mikro.poll(0).unwrap(), vec![key(5, true)]);
    }

    #[test]
    fn mikro_buttons_skip_the_encoder_push() {
        let (mut mikro, recorder) = open(MIKRO_MK2_ID);
        // Shift, the encoder push, Group, F1 and Scene; encoder at 7.
        recorder.queue_read(vec![0x01, 0x01, 0x88, 0x80, 0x80, 7]);
        recorder.queue_read(vec![0x01, 0, 0, 0, 0, 6]);
        assert_eq!(
            mikro.poll(0).unwrap(),
            vec![
                key(16, true),
                key(27, true),
                key(35, true),
                key(43, true),
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                key(16, false),
                key(27, false),
                key(35, false),
                key(43, false),
                Input::DialPress {
                    index: 0,
                    pressed: false
                },
                turn(0, -1),
            ]
        );
        let names = key_names(mikro.model).unwrap();
        assert_eq!(names.len(), 44);
        assert_eq!(
            (names[16], names[27], names[35], names[43]),
            ("Shift", "Group", "F1", "Scene")
        );
    }

    #[test]
    fn jam_matrix_scenes_groups_and_encoder() {
        let (mut jam, recorder) = open(JAM_ID);
        let report = |bits: &[u8], encoder: u8| {
            let mut report = vec![0u8; 17];
            report[0] = 0x01;
            report[1] = encoder;
            for bit in bits {
                report[2 + usize::from(bit / 8)] |= 1 << (bit % 8);
            }
            report
        };
        // Matrix cell 0 and 63, scene 1, group H, Arrange, Select, the encoder
        // touch (ignored) and push.
        recorder.queue_read(report(&[18, 81, 1, 89, 0, 114, 115, 116], 3));
        recorder.queue_read(report(&[], 4));
        let events = jam.poll(0).unwrap();
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(63, true),
                key(64, true),
                key(79, true),
                key(80, true),
                key(114, true),
                Input::DialPress {
                    index: 8,
                    pressed: true
                },
                key(0, false),
                key(63, false),
                key(64, false),
                key(79, false),
                key(80, false),
                key(114, false),
                Input::DialPress {
                    index: 8,
                    pressed: false
                },
                turn(8, 1),
            ]
        );
    }

    #[test]
    fn jam_strips_turn_their_dials_while_touched() {
        let (mut jam, recorder) = open(JAM_ID);
        let report = |strip: usize, position: u16| {
            let mut report = vec![0u8; 49];
            report[0] = 0x02;
            let at = 1 + 6 * strip + 2;
            report[at..at + 2].copy_from_slice(&position.to_le_bytes());
            report
        };
        recorder.queue_read(report(2, 100));
        recorder.queue_read(report(2, 150));
        recorder.queue_read(report(2, 180));
        recorder.queue_read(report(2, 100));
        // Lifted, then touched again elsewhere: no jump.
        recorder.queue_read(report(2, 0));
        recorder.queue_read(report(2, 900));
        assert_eq!(
            jam.poll(0).unwrap(),
            vec![turn(2, 1), turn(2, 1), turn(2, -2)]
        );
    }

    #[test]
    fn mk2_first_flush_sends_every_light_report_and_both_displays() {
        let (mut mk2, recorder) = open(MK2_ID);
        // Pad 13 (top-left) red, Group B blue, Control dim white, Shift.
        mk2.set_face(0, Some(&color([255, 0, 0]))).unwrap();
        mk2.set_face(41, Some(&color([0, 0, 200]))).unwrap();
        mk2.set_face(24, Some(&color([40, 40, 40]))).unwrap();
        mk2.set_face(55, Some(&color([0, 255, 0]))).unwrap();
        // Main has no LED.
        mk2.set_face(39, Some(&color([255, 255, 255]))).unwrap();
        mk2.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 3 + 16);
        let pads = &writes[0];
        assert_eq!((pads.len(), pads[0]), (50, 0x80));
        assert_eq!(pads[1..4], [255, 0, 0]);
        assert_eq!(pads.iter().filter(|byte| **byte != 0).count(), 2);
        let groups = &writes[1];
        assert_eq!((groups.len(), groups[0]), (58, 0x81));
        assert_eq!(groups[7..13], [0, 0, 200, 0, 0, 200]);
        assert_eq!(groups[1 + 55], 255);
        let buttons = &writes[2];
        assert_eq!((buttons.len(), buttons[0]), (33, 0x82));
        assert_eq!(buttons[1], 40);
        assert_eq!(buttons.iter().filter(|byte| **byte != 0).count(), 2);
        for (at, write) in writes[3..].iter().enumerate() {
            let display = u8::try_from(at / 8).unwrap();
            let chunk = u8::try_from(at % 8).unwrap();
            assert_eq!(write.len(), 265);
            assert_eq!(
                write[..9],
                [0xe0 | display, 0, 0, 8 * chunk, 0, 0x20, 0, 0x08, 0]
            );
        }
        // Nothing changed: nothing is sent again.
        mk2.flush().unwrap();
        assert_eq!(recorder.writes().len(), 19);
    }

    #[test]
    fn mk2_display_splits_one_wide_frame_in_rows() {
        let (mut mk2, recorder) = open(MK2_ID);
        assert_eq!(mk2.model.info_key(), Some(64));
        let mut picture = RgbaImage::from_pixel(512, 64, Rgba([0, 0, 0, 255]));
        for (x, y) in [(0, 0), (9, 1), (255, 63), (256, 0), (511, 10)] {
            picture.put_pixel(x, y, Rgba([255, 255, 255, 255]));
        }
        let face = Face {
            label: String::new(),
            color: [0, 0, 0],
            image: Some(picture),
        };
        mk2.set_face(64, Some(&face)).unwrap();
        mk2.flush().unwrap();
        let writes = recorder.writes();
        let left: Vec<u8> = writes[3..11].iter().flat_map(|w| w[9..].to_vec()).collect();
        let right: Vec<u8> = writes[11..19]
            .iter()
            .flat_map(|w| w[9..].to_vec())
            .collect();
        assert_eq!((left.len(), right.len()), (2048, 2048));
        // The most significant bit is the leftmost pixel.
        assert_eq!(left[0], 0x80);
        assert_eq!(left[32 + 1], 0x40);
        assert_eq!(left[63 * 32 + 31], 0x01);
        assert_eq!(left.iter().filter(|byte| **byte != 0).count(), 3);
        assert_eq!(right[0], 0x80);
        assert_eq!(right[10 * 32 + 31], 0x01);
        assert_eq!(right.iter().filter(|byte| **byte != 0).count(), 2);
    }

    #[test]
    fn mikro_lights_and_display_pages() {
        let (mut mikro, recorder) = open(MIKRO_MK2_ID);
        assert_eq!(mikro.model.info_key(), Some(44));
        // Pad 4 (bottom-right) white, at most half on the Mikro; Group green;
        // F1 at full.
        mikro.set_face(15, Some(&color([255, 255, 255]))).unwrap();
        mikro.set_face(27, Some(&color([0, 200, 0]))).unwrap();
        mikro.set_face(35, Some(&color([10, 255, 0]))).unwrap();
        let mut picture = RgbaImage::from_pixel(128, 64, Rgba([0, 0, 0, 255]));
        picture.put_pixel(3, 9, Rgba([255, 255, 255, 255]));
        let face = Face {
            label: String::new(),
            color: [0, 0, 0],
            image: Some(picture),
        };
        mikro.set_face(44, Some(&face)).unwrap();
        mikro.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 1 + 4);
        let leds = &writes[0];
        assert_eq!((leds.len(), leds[0]), (79, 0x80));
        assert_eq!(leds[1], 255);
        assert_eq!(leds[9..12], [0, 100, 0]);
        assert_eq!(leds[1 + 30 + 45..1 + 30 + 48], [127, 127, 127]);
        for (chunk, write) in (0u8..).zip(&writes[1..]) {
            assert_eq!(write.len(), 265);
            assert_eq!(write[..9], [0xe0, 0, 0, 2 * chunk, 0, 0x80, 0, 0x02, 0]);
        }
        // Row 9 is bit 1 of page 1, column 3.
        assert_eq!(writes[1][9 + 128 + 3], 0x02);
    }

    #[test]
    fn brightness_scales_every_light() {
        let (mut mikro, recorder) = open(MIKRO_MK2_ID);
        mikro.set_face(35, Some(&color([200, 0, 0]))).unwrap();
        mikro.set_face(0, Some(&color([200, 100, 0]))).unwrap();
        mikro.flush().unwrap();
        mikro.set_brightness(50).unwrap();
        let writes = recorder.writes();
        let leds = &writes[5];
        assert_eq!(leds[1], 100);
        assert_eq!(leds[31..34], [50, 25, 0]);
    }

    #[test]
    fn jam_lights_take_the_nearest_palette_entry() {
        let (mut jam, recorder) = open(JAM_ID);
        // Matrix cell 0 red, scene 1 blue, group A white, Arrange on.
        jam.set_face(0, Some(&color([255, 0, 0]))).unwrap();
        jam.set_face(64, Some(&color([0, 0, 255]))).unwrap();
        jam.set_face(72, Some(&color([255, 255, 255]))).unwrap();
        jam.set_face(80, Some(&color([0, 90, 0]))).unwrap();
        jam.set_face(63, Some(&color([0, 0, 0]))).unwrap();
        jam.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 3);
        assert_eq!(
            (writes[0].len(), writes[0][0], writes[0][1]),
            (54, 0x80, 90)
        );
        let palette = &writes[1];
        assert_eq!((palette.len(), palette[0]), (81, 0x81));
        assert_eq!(palette[1 + 8], 6);
        assert_eq!(palette[1], 46);
        assert_eq!(palette[1 + 72], 70);
        assert_eq!(palette[1 + 8 + 63], 0);
        assert_eq!((writes[2].len(), writes[2][0]), (89, 0x82));
        // Dim colours land on the dim entries of their hue.
        assert_eq!(nearest_jam_color([38, 0, 0]), 4);
        assert_eq!(nearest_jam_color([0, 0, 0]), 0);
    }

    #[test]
    fn dropping_the_deck_blanks_lights_and_displays() {
        let (mk2, recorder) = open(MK2_ID);
        drop(mk2);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 3 + 16);
        assert!(writes[..3].iter().all(|w| w[1..].iter().all(|b| *b == 0)));
        assert!(writes[3..].iter().all(|w| w[9..].iter().all(|b| *b == 0)));
        let (jam, recorder) = open(JAM_ID);
        drop(jam);
        assert_eq!(recorder.writes().len(), 3);
    }
}
