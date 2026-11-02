//! Novation's Launchkey MK3 keyboards (Mini, 25, 37, 49 and 61) in DAW mode,
//! over class-compliant USB MIDI ([`super::midi`]).
//!
//! Each Launchkey lists two MIDI interfaces: the first carries the piano keys
//! and is never opened; the second (its `DAW` port) carries the pads, buttons,
//! knobs, faders and lights. [`port_matches`] finds that second one by name:
//! `Launchkey Mini MK3 DAW Port`, `... MIDI 2` or `... Launchkey Mi #2` on the
//! Mini, `Launchkey MK3 <size> LKMK3 DAW In` / `... DAW Out` or `... MIDI 2`
//! on the others, and on Windows interface 2 (`MIDIIN2 (...)`). Windows names
//! every full-size Launchkey `LKMK3 MIDI`, so there each one is taken to be a
//! 25 (whose controls the 37, 49 and 61 also have).
//!
//! On open Ryu turns DAW mode on (Note On `9F 0C 7F`) and waits up to a second
//! for the keyboard to echo it, then picks the session pad layout (`BF 03 02`),
//! the volume knob mode (`BF 09 01`) and, on the 49 and 61, the volume fader
//! mode (`BF 0A 01`), and paints every light. The keyboard reports the user
//! picking another pad, knob or fader mode on `BF 03`, `BF 09` and `BF 0A`;
//! Ryu picks its own back and repaints. `9F 0C 00` (DAW mode off) makes Ryu
//! turn it on again; on drop Ryu turns it off, which hands the keyboard back.
//!
//! In: the pads are notes 96-103 (top row) and 112-119 (bottom row) on
//! channel 1, or, after a stray layout change, drum notes on channel 10 or
//! device-select notes 64-71 and 80-87; the buttons are Control Change, 127
//! pressed and 0 released, on channel 1 (`CC 104`, `105`, Shift `108`) or
//! channel 16 (the rest). Up and Down also send a redundant `CC 104` / `105`
//! on channel 16, which is dropped. Knobs (`CC 21-28`) and the 49's and 61's
//! faders (`CC 53-61`) on channel 16 are absolute, turned into dial steps of
//! one MIDI unit from the first value seen.
//!
//! Out: a pad takes an index into Novation's 128-colour palette by Note On on
//! channel 1 and its session note; the scene buttons, Up, Down and the fader
//! buttons take a palette index by Control Change on channel 1; the other lit
//! buttons have one colour, at off, half or full (`0`, `63`, `127`) by Control
//! Change on channel 16. Shift and Device Select have no light. There is no
//! brightness command: brightness scales the colour before the palette lookup.
//!
//! Facts from Ableton Live's Launchkey MK3 remote scripts (no licence; read
//! for note and controller numbers, channels and the palette only) and
//! DrivenByMoss (LGPL-3.0, for port names and the start-up order); no code was
//! copied.

use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const MINI_ID: &str = "novation-launchkey-mini-mk3";
pub const K25_ID: &str = "novation-launchkey-25-mk3";
pub const K37_ID: &str = "novation-launchkey-37-mk3";
pub const K49_ID: &str = "novation-launchkey-49-mk3";
pub const K61_ID: &str = "novation-launchkey-61-mk3";

/// The 16 pads, in two rows of eight; they are keys 0 to 15.
pub const PADS: usize = 16;
const ROW: u8 = 8;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// Channel 1 and channel 16, low nibble of the status byte.
const CH1: u8 = 0x00;
const CH16: u8 = 0x0f;
/// The drum layout's channel (10).
const DRUM: u8 = 0x09;

/// Note 12 on channel 16 turns DAW mode on (127) or off (0).
const DAW_NOTE: u8 = 0x0c;
const DAW_ON: [u8; 3] = [NOTE_ON | CH16, DAW_NOTE, 0x7f];
const DAW_OFF: [u8; 3] = [NOTE_ON | CH16, DAW_NOTE, 0x00];
/// Pad layout, knob mode and fader mode controllers (channel 16).
const PAD_LAYOUT: u8 = 0x03;
const POT_MODE: u8 = 0x09;
const FADER_MODE: u8 = 0x0a;
/// The session pad layout, and the volume knob and fader mode.
const SESSION: u8 = 2;
const VOLUME: u8 = 1;

/// The session layout's first note on each pad row.
const SESSION_ROWS: [u8; 2] = [96, 112];
/// The session rows, then the device-select layout's (full-size only).
const FULL_SIZE_ROWS: [u8; 4] = [96, 112, 64, 80];
/// The drum layout's notes, top row then bottom row, left to right.
const DRUM_NOTES: [u8; PADS] = [
    40, 41, 42, 43, 48, 49, 50, 51, 36, 37, 38, 39, 44, 45, 46, 47,
];

/// The knobs, then (49 and 61) the faders and master fader.
const POTS_FIRST_CC: u8 = 21;
const FADERS_FIRST_CC: u8 = 53;
const POTS: u8 = 8;
const FADERS: u8 = 9;

/// How long Ryu waits for DAW mode's echo before painting anyway.
const ECHO_WAIT: Duration = Duration::from_secs(1);
const MONO_OFF: u8 = 0;
const MONO_HALF: u8 = 63;
const MONO_ON: u8 = 127;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// Novation's 128-colour palette, index to `0xRRGGBB`.
pub(crate) const PALETTE: [u32; 128] = [
    0x00_0000, 0x1e_1e1e, 0x7f_7f7f, 0xff_ffff, 0xff_4c4c, 0xff_0000, 0x59_0000, 0x19_0000,
    0xff_bd6c, 0xff_5400, 0x59_1d00, 0x27_1b00, 0xff_ff4c, 0xff_ff00, 0x59_5900, 0x19_1900,
    0x88_ff4c, 0x54_ff00, 0x1d_5900, 0x14_2b00, 0x4c_ff4c, 0x00_ff00, 0x00_5900, 0x00_1900,
    0x4c_ff5e, 0x00_ff19, 0x00_590d, 0x00_1902, 0x4c_ff88, 0x00_ff55, 0x00_591d, 0x00_1f12,
    0x4c_ffb7, 0x00_ff99, 0x00_5935, 0x00_1912, 0x4c_c3ff, 0x00_a9ff, 0x00_4152, 0x00_1019,
    0x4c_88ff, 0x00_55ff, 0x00_1d59, 0x00_0819, 0x4c_4cff, 0x00_00ff, 0x00_0059, 0x00_0019,
    0x87_4cff, 0x54_00ff, 0x19_0064, 0x0f_0030, 0xff_4cff, 0xff_00ff, 0x59_0059, 0x19_0019,
    0xff_4c87, 0xff_0054, 0x59_001d, 0x22_0013, 0xff_1500, 0x99_3500, 0x79_5100, 0x43_6400,
    0x03_3900, 0x00_5735, 0x00_547f, 0x00_00ff, 0x00_454f, 0x25_00cc, 0x7f_7f7f, 0x20_2020,
    0xff_0000, 0xbd_ff2d, 0xaf_ed06, 0x64_ff09, 0x10_8b00, 0x00_ff87, 0x00_a9ff, 0x00_2aff,
    0x3f_00ff, 0x7a_00ff, 0xb2_1a7d, 0x40_2100, 0xff_4a00, 0x88_e106, 0x72_ff15, 0x00_ff00,
    0x3b_ff26, 0x59_ff71, 0x38_ffcc, 0x5b_8aff, 0x31_51c6, 0x87_7fe9, 0xd3_1dff, 0xff_005d,
    0xff_7f00, 0xb9_b000, 0x90_ff00, 0x83_5d07, 0x39_2b00, 0x14_4c10, 0x0d_5038, 0x15_152a,
    0x16_205a, 0x69_3c1c, 0xa8_000a, 0xde_513d, 0xd8_6a1c, 0xff_e126, 0x9e_e12f, 0x67_b50f,
    0x1e_1e30, 0xdc_ff6b, 0x80_ffbd, 0x9a_99ff, 0x8e_66ff, 0x40_4040, 0x75_7575, 0xe0_ffff,
    0xa0_0000, 0x35_0000, 0x1a_d000, 0x07_4200, 0xb9_b000, 0x3f_3100, 0xb3_5f00, 0x4b_1502,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Mini,
    /// The 25 and 37.
    Small,
    /// The 49 and 61, with faders and fader buttons.
    Large,
}

impl Kind {
    fn of(model: &DeviceModel) -> Option<Self> {
        match model.id {
            MINI_ID => Some(Self::Mini),
            K25_ID | K37_ID => Some(Self::Small),
            K49_ID | K61_ID => Some(Self::Large),
            _ => None,
        }
    }

    const fn buttons(self) -> &'static [Button] {
        match self {
            Self::Mini => &MINI_BUTTONS,
            Self::Small => SMALL_BUTTONS,
            Self::Large => &LARGE_BUTTONS,
        }
    }

    const fn dials(self) -> u8 {
        match self {
            Self::Mini | Self::Small => POTS,
            Self::Large => POTS + FADERS,
        }
    }

    /// The dial a knob or fader controller on channel 16 turns.
    fn dial_of_cc(self, cc: u8) -> Option<u8> {
        if let Some(at) = cc.checked_sub(POTS_FIRST_CC).filter(|at| *at < POTS) {
            return Some(at);
        }
        if self == Self::Large {
            return cc
                .checked_sub(FADERS_FIRST_CC)
                .filter(|at| *at < FADERS)
                .map(|at| POTS + at);
        }
        None
    }

    /// The key a pad note is, on `channel`.
    fn pad_of_note(self, channel: u8, note: u8) -> Option<usize> {
        let rows: &[u8] = match (channel, self) {
            (CH1, Self::Mini) => &SESSION_ROWS,
            (CH1, _) => &FULL_SIZE_ROWS,
            (DRUM, _) => return DRUM_NOTES.iter().position(|drum| *drum == note),
            _ => return None,
        };
        rows.iter().enumerate().find_map(|(row, first)| {
            note.checked_sub(*first)
                .filter(|col| *col < ROW)
                .map(|col| (row % 2) * usize::from(ROW) + usize::from(col))
        })
    }
}

/// How a button's light is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Light {
    None,
    /// A palette index by Control Change on channel 1.
    Rgb,
    /// Off, half or full by Control Change on channel 16.
    Mono,
}

/// A button: the controller it sends, the channel it sends it on, and its light.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Button {
    cc: u8,
    channel: u8,
    light: Light,
    name: &'static str,
    /// (row, column) in the model's grid.
    place: (u8, u8),
}

const fn button(cc: u8, channel: u8, light: Light, name: &'static str, place: (u8, u8)) -> Button {
    Button {
        cc,
        channel,
        light,
        name,
        place,
    }
}

/// The Mini's buttons: keys 16 to 22.
const MINI_BUTTONS: [Button; 7] = [
    button(104, CH1, Light::Rgb, "Scene Launch", (0, 8)),
    button(105, CH1, Light::Rgb, "Stop/Solo/Mute", (1, 8)),
    button(103, CH16, Light::Mono, "Track Left", (2, 1)),
    button(102, CH16, Light::Mono, "Track Right", (2, 2)),
    button(108, CH1, Light::None, "Shift", (2, 0)),
    button(115, CH16, Light::Mono, "Play", (2, 6)),
    button(117, CH16, Light::Mono, "Record", (2, 7)),
];

/// The 25's and 37's buttons (keys 16 to 32), which the 49 and 61 share.
/// Up and Down are pressed on channel 16 but lit on channel 1.
const LARGE_BUTTONS: [Button; 26] = [
    button(104, CH1, Light::Rgb, "Scene Launch", (1, 8)),
    button(105, CH1, Light::Rgb, "Stop/Solo/Mute", (2, 8)),
    button(106, CH16, Light::Rgb, "Up", (1, 9)),
    button(107, CH16, Light::Rgb, "Down", (2, 9)),
    button(103, CH16, Light::Mono, "Track Left", (0, 8)),
    button(102, CH16, Light::Mono, "Track Right", (0, 9)),
    button(108, CH1, Light::None, "Shift", (3, 0)),
    button(115, CH16, Light::Mono, "Play", (3, 4)),
    button(116, CH16, Light::Mono, "Stop", (3, 5)),
    button(117, CH16, Light::Mono, "Record", (3, 6)),
    button(118, CH16, Light::Mono, "Loop", (3, 7)),
    button(74, CH16, Light::Mono, "Capture MIDI", (0, 0)),
    button(75, CH16, Light::Mono, "Quantise", (0, 1)),
    button(76, CH16, Light::Mono, "Click", (0, 2)),
    button(77, CH16, Light::Mono, "Undo", (0, 3)),
    // Neither reference lights Device Select.
    button(51, CH16, Light::None, "Device Select", (0, 5)),
    button(52, CH16, Light::Mono, "Device Lock", (0, 6)),
    // The 49 and 61 only: the fader buttons (lit on channel 1) and Arm/Select.
    button(37, CH16, Light::Rgb, "Fader Button 1", (4, 0)),
    button(38, CH16, Light::Rgb, "Fader Button 2", (4, 1)),
    button(39, CH16, Light::Rgb, "Fader Button 3", (4, 2)),
    button(40, CH16, Light::Rgb, "Fader Button 4", (4, 3)),
    button(41, CH16, Light::Rgb, "Fader Button 5", (4, 4)),
    button(42, CH16, Light::Rgb, "Fader Button 6", (4, 5)),
    button(43, CH16, Light::Rgb, "Fader Button 7", (4, 6)),
    button(44, CH16, Light::Rgb, "Fader Button 8", (4, 7)),
    button(45, CH16, Light::Mono, "Arm/Select", (4, 8)),
];

/// The buttons the 25 and 37 have.
const SMALL_BUTTONS: &[Button] = LARGE_BUTTONS.split_at(17).0;

const PAD_NAMES: [&str; PADS] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16",
];

/// Which Launchkey a port's normalised name says it is, if it is a
/// Launchkey's DAW port. The first interface (the piano keys) never matches.
fn kind_of_port(name: &PortName) -> Option<Kind> {
    let base = name.base.as_str();
    // Windows names the second interface `MIDIIN2 (...)`, the first bare.
    let daw = match name.interface {
        Some(interface) => interface == 2,
        None => {
            base.ends_with("DAW Port")
                || base.ends_with("LKMK3 DAW")
                || base.ends_with(" MIDI 2")
                || base.ends_with(" #2")
        }
    };
    if !daw {
        return None;
    }
    if base.contains("Launchkey Mini MK3") {
        return Some(Kind::Mini);
    }
    if let Some(at) = base.find("Launchkey MK3 ") {
        let size = base.get(at + "Launchkey MK3 ".len()..)?.get(..2)?;
        return match size {
            "25" | "37" => Some(Kind::Small),
            "49" | "61" => Some(Kind::Large),
            _ => None,
        };
    }
    // Windows: every full-size Launchkey, whatever its size.
    (name.interface.is_some() && base.starts_with("LKMK3 MIDI")).then_some(Kind::Small)
}

/// The model rows a Launchkey port name picks: the size in the name, else
/// (Windows) the 25.
fn size_of_port(base: &str) -> Option<&'static str> {
    if base.contains("Launchkey Mini MK3") {
        return Some(MINI_ID);
    }
    let Some(at) = base.find("Launchkey MK3 ") else {
        return Some(K25_ID);
    };
    match base.get(at + "Launchkey MK3 ".len()..)?.get(..2)? {
        "25" => Some(K25_ID),
        "37" => Some(K37_ID),
        "49" => Some(K49_ID),
        "61" => Some(K61_ID),
        _ => None,
    }
}

/// Whether a MIDI port, by its normalised name, is this model's DAW port.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    Kind::of(model).is_some()
        && kind_of_port(name).is_some()
        && size_of_port(&name.base) == Some(model.id)
}

/// The legend on each key, on a Launchkey.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    let kind = Kind::of(model)?;
    Some(
        PAD_NAMES
            .iter()
            .copied()
            .chain(kind.buttons().iter().map(|button| button.name))
            .collect(),
    )
}

/// Each key's cell in the model's grid, on a Launchkey: the pads in two rows
/// of eight (rows 1 and 2 on the full-size ones), the buttons around them.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    let kind = Kind::of(model)?;
    let first_row = u16::from(kind != Kind::Mini);
    let cols = u16::from(model.cols);
    let pads = (0..PADS as u16).map(|pad| (first_row + pad / 8) * cols + pad % 8);
    let buttons = kind
        .buttons()
        .iter()
        .map(|button| u16::from(button.place.0) * cols + u16::from(button.place.1));
    Some(pads.chain(buttons).collect())
}

/// The keys with a light: every pad and the lit buttons.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    let kind = Kind::of(model)?;
    let pads = 0..PADS as u8;
    let buttons = (PADS as u8..)
        .zip(kind.buttons())
        .filter(|(_, button)| button.light != Light::None)
        .map(|(index, _)| index);
    Some(pads.chain(buttons).collect())
}

/// A colour scaled by `percent`.
fn scaled(color: [u8; 3], percent: u8) -> [u8; 3] {
    let percent = u16::from(percent.min(100));
    color.map(|channel| ((u16::from(channel) * percent + 50) / 100) as u8)
}

/// The nearest palette index to a colour: index 0 (off) only for black, and
/// the lowest index on a tie.
pub(crate) fn palette_index(color: [u8; 3]) -> u8 {
    if color == BLACK {
        return 0;
    }
    let distance = |rgb: u32| {
        let [_, red, green, blue] = rgb.to_be_bytes();
        [red, green, blue]
            .iter()
            .zip(color)
            .map(|(have, want)| {
                let delta = i32::from(*have) - i32::from(want);
                delta * delta
            })
            .sum::<i32>()
    };
    (1u8..)
        .zip(PALETTE.iter().skip(1))
        .min_by_key(|(_, rgb)| distance(**rgb))
        .map_or(0, |(index, _)| index)
}

/// Off, half or full for a single-colour light, from the scaled colour's
/// brightest channel (so a full red or blue is full, not half).
fn mono_level(color: [u8; 3]) -> u8 {
    match color.into_iter().max().unwrap_or(0) {
        0 => MONO_OFF,
        1..=127 => MONO_HALF,
        _ => MONO_ON,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// DAW mode not asked for yet (or the keyboard left it).
    Closed,
    /// DAW mode asked for at this moment; waiting for the echo.
    Waiting(Instant),
    /// Layout and modes picked; lights are sent.
    Ready,
}

pub struct Launchkey {
    kind: Kind,
    transport: Box<dyn Transport>,
    stage: Stage,
    /// Whether DAW mode was ever turned on, so it is turned off on drop.
    opened: bool,
    echo_wait: Duration,
    colors: Vec<[u8; 3]>,
    /// What each key's light was last sent; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    /// Each dial's last absolute value; `None` until one arrives.
    dials: Vec<Option<u8>>,
    brightness: u8,
    parser: Parser,
}

impl Launchkey {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model).unwrap_or(Kind::Small);
        let keys = PADS + kind.buttons().len();
        Self {
            kind,
            transport,
            stage: Stage::Closed,
            opened: false,
            echo_wait: ECHO_WAIT,
            colors: vec![BLACK; keys],
            sent: vec![None; keys],
            held: vec![false; keys],
            dials: vec![None; usize::from(kind.dials())],
            brightness: 100,
            parser: Parser::default(),
        }
    }

    /// Asks for DAW mode, once until the keyboard leaves it.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if self.stage == Stage::Closed {
            self.transport.write(&DAW_ON)?;
            self.opened = true;
            self.stage = Stage::Waiting(Instant::now());
        }
        Ok(())
    }

    /// Picks the session layout and the volume modes and forgets the lights,
    /// so the next flush paints them all.
    fn assert_modes(&mut self) -> DriverResult<()> {
        self.transport
            .write(&[CONTROL_CHANGE | CH16, PAD_LAYOUT, SESSION])?;
        self.transport
            .write(&[CONTROL_CHANGE | CH16, POT_MODE, VOLUME])?;
        if self.kind == Kind::Large {
            self.transport
                .write(&[CONTROL_CHANGE | CH16, FADER_MODE, VOLUME])?;
        }
        self.sent.fill(None);
        self.dials.fill(None);
        self.stage = Stage::Ready;
        Ok(())
    }

    fn release_pads(&mut self, events: &mut Vec<Input>) {
        for pad in 0..PADS {
            self.press(pad, false, events);
        }
    }

    /// Handles one message; returns whether the lights must be repainted.
    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> DriverResult<bool> {
        let Message::Channel([status, number, value]) = *message else {
            return Ok(false);
        };
        let kind = status & 0xf0;
        let channel = status & 0x0f;
        match (kind, channel, number) {
            (NOTE_ON, CH16, DAW_NOTE) => {
                if value > 0 {
                    // The echo (or another app turning DAW mode on): pick
                    // Ryu's layout and modes and repaint.
                    self.assert_modes()?;
                    return Ok(true);
                }
                self.release_pads(events);
                for key in PADS..self.held.len() {
                    self.press(key, false, events);
                }
                self.stage = Stage::Closed;
                return Ok(true);
            }
            (CONTROL_CHANGE, CH16, PAD_LAYOUT) => {
                if value != SESSION {
                    self.release_pads(events);
                    self.transport
                        .write(&[CONTROL_CHANGE | CH16, PAD_LAYOUT, SESSION])?;
                    self.sent
                        .get_mut(..PADS)
                        .into_iter()
                        .flatten()
                        .for_each(|sent| *sent = None);
                    return Ok(true);
                }
                return Ok(false);
            }
            (CONTROL_CHANGE, CH16, POT_MODE) => {
                if value != VOLUME {
                    self.transport
                        .write(&[CONTROL_CHANGE | CH16, POT_MODE, VOLUME])?;
                    self.dials
                        .iter_mut()
                        .take(usize::from(POTS))
                        .for_each(|dial| *dial = None);
                }
                return Ok(false);
            }
            (CONTROL_CHANGE, CH16, FADER_MODE) if self.kind == Kind::Large => {
                if value != VOLUME {
                    self.transport
                        .write(&[CONTROL_CHANGE | CH16, FADER_MODE, VOLUME])?;
                    self.dials
                        .iter_mut()
                        .skip(usize::from(POTS))
                        .for_each(|dial| *dial = None);
                }
                return Ok(false);
            }
            _ => {}
        }
        match kind {
            NOTE_ON | NOTE_OFF => {
                if let Some(pad) = self.kind.pad_of_note(channel, number) {
                    self.press(pad, kind == NOTE_ON && value > 0, events);
                }
            }
            CONTROL_CHANGE => {
                if let Some(at) = self
                    .kind
                    .buttons()
                    .iter()
                    .position(|button| button.cc == number && button.channel == channel)
                {
                    self.press(PADS + at, value > 0, events);
                } else if channel == CH16 {
                    if let Some(dial) = self.kind.dial_of_cc(number) {
                        self.turn(dial, value, events);
                    }
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn press(&mut self, key: usize, pressed: bool, events: &mut Vec<Input>) {
        let Some(held) = self.held.get_mut(key) else {
            return;
        };
        if *held == pressed {
            return;
        }
        *held = pressed;
        if let Ok(index) = u8::try_from(key) {
            events.push(Input::Key { index, pressed });
        }
    }

    /// An absolute value: the first one seeds the dial, each later one turns
    /// it by how far it moved.
    fn turn(&mut self, dial: u8, value: u8, events: &mut Vec<Input>) {
        let Some(last) = self.dials.get_mut(usize::from(dial)) else {
            return;
        };
        let Some(old) = last.replace(value) else {
            return;
        };
        let moved = i16::from(value) - i16::from(old);
        if moved != 0 {
            events.push(Input::DialTurn {
                index: dial,
                delta: i8::try_from(moved.clamp(-127, 127)).unwrap_or(0),
            });
        }
    }

    /// The message that sets key `key`'s light at the current colour and
    /// brightness, if it has one.
    fn light_message(&self, key: usize) -> Option<[u8; 3]> {
        let color = scaled(*self.colors.get(key)?, self.brightness);
        if key < PADS {
            let row = key / usize::from(ROW);
            let col = (key % usize::from(ROW)) as u8;
            let note = SESSION_ROWS.get(row)? + col;
            return Some([NOTE_ON | CH1, note, palette_index(color)]);
        }
        let button = self.kind.buttons().get(key - PADS)?;
        match button.light {
            Light::None => None,
            Light::Rgb => Some([CONTROL_CHANGE | CH1, button.cc, palette_index(color)]),
            Light::Mono => Some([CONTROL_CHANGE | CH16, button.cc, mono_level(color)]),
        }
    }
}

impl Driver for Launchkey {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_started()?;
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
        let mut repaint = false;
        for message in &messages {
            repaint |= self.receive(message, &mut events)?;
        }
        if let Stage::Waiting(since) = self.stage {
            if since.elapsed() >= self.echo_wait {
                // No echo: carry on as if it had come.
                self.assert_modes()?;
                repaint = true;
            }
        }
        if repaint {
            self.flush()?;
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// One message per light that changed, once DAW mode is on.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        if self.stage != Stage::Ready {
            return Ok(());
        }
        for key in 0..self.sent.len() {
            let Some(message) = self.light_message(key) else {
                continue;
            };
            if self.sent.get(key).copied().flatten() != Some(message[2]) {
                self.transport.write(&message)?;
                if let Some(sent) = self.sent.get_mut(key) {
                    *sent = Some(message[2]);
                }
            }
        }
        Ok(())
    }

    /// No brightness command: the colours are scaled before the palette
    /// lookup and every light is sent again.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }
}

impl Drop for Launchkey {
    /// Turns DAW mode off, which hands the keyboard back to its own modes.
    fn drop(&mut self) {
        if self.opened {
            let _ = self.transport.write(&DAW_OFF);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify, identify_midi, ModelView, Protocol, MODELS};

    fn open(id: &str) -> (Launchkey, Recorder) {
        let recorder = Recorder::default();
        let deck = Launchkey::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// Opens a deck and answers DAW mode, so it is ready.
    fn ready(id: &str) -> (Launchkey, Recorder) {
        let (mut deck, recorder) = open(id);
        recorder.queue_read(vec![0x9f, 0x0c, 0x7f]);
        deck.poll(0).unwrap();
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

    fn feed(deck: &mut Launchkey, recorder: &Recorder, messages: &[&[u8]]) -> Vec<Input> {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
        deck.poll(0).unwrap()
    }

    #[test]
    fn colours_take_the_nearest_palette_entry() {
        assert_eq!(palette_index([0, 0, 0]), 0);
        assert_eq!(palette_index([255, 255, 255]), 3);
        assert_eq!(palette_index([255, 0, 0]), 5);
        assert_eq!(palette_index([0, 255, 0]), 21);
        assert_eq!(palette_index([0, 0, 255]), 45);
        assert_eq!(palette_index([255, 0, 255]), 53);
        assert_eq!(palette_index([128, 0, 128]), 54);
        assert_eq!(palette_index([255, 255, 0]), 13);
        // Near-black is still lit, never off.
        assert_ne!(palette_index([1, 1, 1]), 0);
        // Exact duplicates go to the lowest index.
        assert_eq!(palette_index([0x7f, 0x7f, 0x7f]), 2);
        assert_eq!(palette_index([0x00, 0xa9, 0xff]), 37);
        assert_eq!(PALETTE.len(), 128);
    }

    #[test]
    fn single_colour_lights_are_off_half_or_full() {
        assert_eq!(mono_level([0, 0, 0]), 0);
        assert_eq!(mono_level([255, 255, 255]), 127);
        assert_eq!(mono_level([0, 255, 0]), 127);
        assert_eq!(mono_level([0, 0, 255]), 127);
        assert_eq!(mono_level([100, 0, 0]), 63);
        assert_eq!(mono_level(scaled([255, 255, 255], 40)), 63);
        assert_eq!(mono_level(scaled([255, 255, 255], 0)), 0);
    }

    #[test]
    fn opening_turns_daw_mode_on_and_paints_after_the_echo() {
        let (mut deck, recorder) = open(K25_ID);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(15, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(16, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(18, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(23, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        // Nothing is painted before the echo.
        assert_eq!(recorder.writes(), vec![vec![0x9f, 0x0c, 0x7f]]);
        recorder.queue_read(vec![0x9f, 0x0c, 0x7f]);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[1], vec![0xbf, 0x03, 0x02]);
        assert_eq!(writes[2], vec![0xbf, 0x09, 0x01]);
        // 16 pads and 15 lit buttons (Shift and Device Select have none).
        assert_eq!(writes.len(), 3 + 16 + 15);
        assert_eq!(writes[3], vec![0x90, 0x60, 0x05]);
        assert_eq!(writes[4], vec![0x90, 0x61, 0x00]);
        assert_eq!(writes[3 + 8], vec![0x90, 0x70, 0x00]);
        assert_eq!(writes[3 + 15], vec![0x90, 0x77, 0x2d]);
        assert_eq!(writes[19], vec![0xb0, 104, 21]);
        assert_eq!(writes[20], vec![0xb0, 105, 0]);
        // Up is lit on channel 1, Track Left (mono) on channel 16.
        assert_eq!(writes[21], vec![0xb0, 106, 3]);
        assert_eq!(writes[23], vec![0xbf, 103, 0]);
        // Key 23 is Play: Shift (key 22) is skipped.
        assert_eq!(writes[25], vec![0xbf, 115, 127]);
        // Nothing changed: nothing sent; one change, one message.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 34);
        deck.set_face(1, Some(&lit([255, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[34..], [vec![0x90, 0x61, 13]]);
    }

    #[test]
    fn without_an_echo_it_paints_anyway() {
        let (mut deck, recorder) = open(K49_ID);
        deck.echo_wait = Duration::ZERO;
        deck.set_face(33, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(41, Some(&lit([255, 0, 0]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[..4],
            [
                vec![0x9f, 0x0c, 0x7f],
                vec![0xbf, 0x03, 0x02],
                vec![0xbf, 0x09, 0x01],
                vec![0xbf, 0x0a, 0x01],
            ]
        );
        // Fader button 1 is lit on channel 1; Arm/Select on channel 16.
        assert!(writes.contains(&vec![0xb0, 37, 5]));
        assert!(writes.contains(&vec![0xbf, 45, 127]));
        assert_eq!(writes.len(), 4 + 16 + 15 + 9);
    }

    #[test]
    fn pads_buttons_and_knobs_map_by_note_controller_and_channel() {
        let (mut deck, recorder) = ready(K25_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x90, 0x60, 0x7f],
                &[0x80, 0x60, 0x00],
                &[0x90, 0x77, 0x40],
                &[0x90, 0x77, 0x00],
                // Aftertouch is ignored.
                &[0xa0, 0x70, 0x30],
                &[0xd0, 0x30],
                &[0xb0, 104, 127],
                &[0xb0, 104, 0],
                // Up, then its redundant CC 104 on channel 16: dropped.
                &[0xbf, 106, 127],
                &[0xbf, 104, 127],
                &[0xbf, 106, 0],
                &[0xbf, 104, 0],
                &[0xb0, 108, 127],
                &[0xbf, 52, 127],
                // Shift on channel 16 is not Shift.
                &[0xbf, 108, 127],
                // Knob 1 seeds, then turns; knob 8 seeds.
                &[0xbf, 21, 64],
                &[0xbf, 21, 70],
                &[0xbf, 21, 60],
                &[0xbf, 28, 0],
                // Faders are not on a 25.
                &[0xbf, 53, 10],
                &[0xbf, 53, 20],
                // Knobs on channel 1 are not Ryu's.
                &[0xb0, 21, 10],
            ],
        );
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(0, false),
                key(15, true),
                key(15, false),
                key(16, true),
                key(16, false),
                key(18, true),
                key(18, false),
                key(22, true),
                key(32, true),
                Input::DialTurn { index: 0, delta: 6 },
                Input::DialTurn {
                    index: 0,
                    delta: -10
                },
            ]
        );
    }

    #[test]
    fn drum_and_device_select_notes_still_press_pads() {
        let (mut deck, recorder) = ready(K37_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x99, 40, 100],
                &[0x89, 40, 0],
                &[0x99, 47, 100],
                &[0x90, 64, 100],
                &[0x90, 87, 100],
            ],
        );
        // Pad 16 is still held when its device-select note comes.
        assert_eq!(
            events,
            vec![key(0, true), key(0, false), key(15, true), key(0, true)]
        );
        // The Mini has no device-select layout.
        let (mut mini, recorder) = ready(MINI_ID);
        assert!(feed(&mut mini, &recorder, &[&[0x90, 64, 100]]).is_empty());
    }

    #[test]
    fn the_mini_reads_its_seven_buttons_and_drops_the_rest() {
        let (mut deck, recorder) = ready(MINI_ID);
        let writes = recorder.writes();
        // No fader mode on the Mini.
        assert_eq!(
            writes,
            [
                vec![0x9f, 0x0c, 0x7f],
                vec![0xbf, 0x03, 0x02],
                vec![0xbf, 0x09, 0x01],
            ]
            .into_iter()
            .chain((0..16).map(|pad| vec![0x90, SESSION_ROWS[pad / 8] + (pad % 8) as u8, 0]))
            .chain([
                vec![0xb0, 104, 0],
                vec![0xb0, 105, 0],
                vec![0xbf, 103, 0],
                vec![0xbf, 102, 0],
                vec![0xbf, 115, 0],
                vec![0xbf, 117, 0],
            ])
            .collect::<Vec<_>>()
        );
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xb0, 105, 127],
                &[0xbf, 103, 127],
                &[0xbf, 102, 127],
                &[0xbf, 117, 127],
                // Not on the Mini.
                &[0xbf, 116, 127],
                &[0xbf, 106, 127],
                &[0xbf, 53, 1],
            ],
        );
        assert_eq!(
            events,
            vec![key(17, true), key(18, true), key(19, true), key(22, true)]
        );
    }

    #[test]
    fn faders_are_dials_on_the_49_and_61() {
        let (mut deck, recorder) = ready(K61_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xbf, 53, 0],
                &[0xbf, 53, 127],
                &[0xbf, 61, 100],
                &[0xbf, 61, 99],
                &[0xbf, 37, 127],
                &[0xbf, 45, 127],
            ],
        );
        assert_eq!(
            events,
            vec![
                Input::DialTurn {
                    index: 8,
                    delta: 127
                },
                Input::DialTurn {
                    index: 16,
                    delta: -1
                },
                key(33, true),
                key(41, true),
            ]
        );
    }

    #[test]
    fn mode_changes_on_the_keyboard_are_undone() {
        let (mut deck, recorder) = ready(K49_ID);
        deck.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x90, 0x60, 0x7f],
                // The user picks the drum layout: the held pad is let go.
                &[0xbf, 0x03, 0x01],
                &[0xbf, 0x09, 0x02],
                &[0xbf, 0x0a, 0x04],
                // Ryu's own choices echoed back change nothing.
                &[0xbf, 0x03, 0x02],
                &[0xbf, 0x09, 0x01],
            ],
        );
        assert_eq!(events, vec![key(0, true), key(0, false)]);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes[0], vec![0xbf, 0x03, 0x02]);
        assert_eq!(writes[1], vec![0xbf, 0x09, 0x01]);
        assert_eq!(writes[2], vec![0xbf, 0x0a, 0x01]);
        // Then the pads are repainted, and only the pads.
        assert_eq!(writes.len(), 3 + 16);
        assert_eq!(writes[3 + 3], vec![0x90, 0x63, 5]);
    }

    #[test]
    fn leaving_daw_mode_turns_it_on_again() {
        let (mut deck, recorder) = ready(MINI_ID);
        let before = recorder.writes().len();
        feed(&mut deck, &recorder, &[&[0x9f, 0x0c, 0x00]]);
        assert_eq!(recorder.writes()[before..], [vec![0x9f, 0x0c, 0x7f]]);
        feed(&mut deck, &recorder, &[&[0x9f, 0x0c, 0x7f]]);
        assert_eq!(recorder.writes()[before + 1], vec![0xbf, 0x03, 0x02]);
        assert_eq!(recorder.writes().len(), before + 1 + 2 + 22);
    }

    #[test]
    fn brightness_scales_colours_and_drop_leaves_daw_mode() {
        let (mut deck, recorder) = ready(K25_ID);
        deck.set_face(0, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(23, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_brightness(40).unwrap();
        assert_eq!(
            recorder.writes()[before..],
            // White at 40% is the palette's 757575.
            [vec![0x90, 0x60, 118], vec![0xbf, 115, 63]]
        );
        deck.set_brightness(0).unwrap();
        assert_eq!(
            recorder.writes()[before + 2..],
            [vec![0x90, 0x60, 0], vec![0xbf, 115, 0]]
        );
        drop(deck);
        assert_eq!(recorder.writes().last(), Some(&vec![0x9f, 0x0c, 0x00]));
        // A deck that never opened sends nothing.
        let (deck, recorder) = open(K25_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn short_or_odd_input_is_ignored() {
        let (mut deck, recorder) = ready(K61_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x90],
                &[0xf8],
                &[
                    0xf0, 0x7e, 0x00, 0x06, 0x02, 0x00, 0x20, 0x29, 0x37, 0x01, 0xf7,
                ],
                &[0xe0, 0x00, 0x40],
                &[0xbf, 0x7f, 0x7f],
                &[],
            ],
        );
        assert!(events.is_empty());
        // Running status across reads: a status byte, then its data.
        let events = feed(&mut deck, &recorder, &[&[0x90], &[0x61, 0x7f, 0x62, 0x7f]]);
        assert_eq!(events, vec![key(1, true), key(2, true)]);
    }

    #[test]
    fn only_daw_ports_match_and_the_size_picks_the_model() {
        let cases: [(&str, bool, Option<&str>); 18] = [
            ("Launchkey Mini MK3 DAW Port", false, Some(MINI_ID)),
            ("Launchkey Mini MK3 MIDI Port", false, None),
            (
                "Launchkey Mini MK3:Launchkey Mini MK3 MIDI 2 24:1",
                false,
                Some(MINI_ID),
            ),
            (
                "Launchkey Mini MK3:Launchkey Mini MK3 MIDI 1 24:0",
                false,
                None,
            ),
            (
                "Launchkey Mini MK3:Launchkey Mini MK3 Launchkey Mi #2 24:1",
                false,
                Some(MINI_ID),
            ),
            (
                "Launchkey Mini MK3:Launchkey Mini MK3 Launchkey Mi 24:0",
                false,
                None,
            ),
            ("MIDIIN2 (Launchkey Mini MK3 MID", true, Some(MINI_ID)),
            ("MIDIOUT2 (Launchkey Mini MK3 MI", true, Some(MINI_ID)),
            ("Launchkey Mini MK3 MIDI", true, None),
            ("Launchkey MK3 25 LKMK3 DAW Out", false, Some(K25_ID)),
            ("Launchkey MK3 37 LKMK3 DAW In", false, Some(K37_ID)),
            (
                "Launchkey MK3 49:Launchkey MK3 49 LKMK3 DAW Out 28:1",
                false,
                Some(K49_ID),
            ),
            (
                "Launchkey MK3 61:Launchkey MK3 61 MIDI 2 28:1",
                false,
                Some(K61_ID),
            ),
            ("Launchkey MK3 61 LKMK3 MIDI Out", false, None),
            ("Launchkey MK3 88 LKMK3 DAW Out", false, None),
            ("MIDIIN2 (LKMK3 MIDI)", true, Some(K25_ID)),
            ("LKMK3 MIDI", true, None),
            (
                "Launchpad X:Launchpad X LPX MIDI 24:1",
                false,
                Some("novation-launchpad-x"),
            ),
        ];
        for (raw, windows, expected) in cases {
            let found = identify_midi(&normalise(raw), windows).map(|model| model.id);
            assert_eq!(found, expected, "{raw}");
        }
    }

    #[test]
    fn the_rows_describe_the_keys_the_driver_reads() {
        for id in [MINI_ID, K25_ID, K37_ID, K49_ID, K61_ID] {
            let model = by_id(id).unwrap();
            let kind = Kind::of(model).unwrap();
            assert_eq!(model.protocol, Protocol::LaunchkeyMk3Midi, "{id}");
            assert!(model.protocol.is_midi(), "{id}");
            assert_eq!(usize::from(model.keys), PADS + kind.buttons().len(), "{id}");
            assert_eq!(model.dials, kind.dials(), "{id}");
            // Found by port name only: the USB id never identifies it.
            for product in model.product_ids {
                assert!(
                    identify(model.vendor_id, *product, 0x0001).is_none(),
                    "{id}"
                );
            }
            let view = ModelView::from(model);
            let cells = view.key_cells.unwrap();
            assert_eq!(cells.len(), usize::from(model.keys), "{id}");
            let mut unique = cells.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), cells.len(), "{id}");
            assert!(cells
                .iter()
                .all(|cell| *cell < u16::from(model.rows) * u16::from(model.cols)));
            assert_eq!(view.key_names.unwrap().len(), usize::from(model.keys));
            assert!(!view.lit_keys.unwrap().contains(&22) || id == MINI_ID);
        }
        assert_eq!(
            MODELS
                .iter()
                .filter(|model| model.protocol == Protocol::LaunchkeyMk3Midi)
                .count(),
            5
        );
    }
}
