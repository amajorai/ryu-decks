//! Akai Professional's Fire (the FL Studio controller), over class-compliant
//! USB MIDI ([`super::midi`]).
//!
//! The Fire lists one MIDI port each way named `FL STUDIO FIRE` (with a
//! ` Jack <n>` suffix on some macOS hosts, and `Akai Fire` on some Windows
//! ones), so the shared `<token> MI` rule cannot take it: [`port_matches`]
//! takes a name containing `FL STUDIO FIRE` or `AKAI FIRE` in any case, and on
//! Windows only the first interface. Nothing is sent to start it.
//!
//! Keys: the 64 pads (4 rows of 16) in reading order from the top left
//! (keys 0-63), then the 20 buttons (64-83): Mute/Solo 1-4, Pattern Up,
//! Pattern Down, Browser, Grid Left, Grid Right, the knob-mode (Bank) button,
//! Step, Note, Drum, Perform, Shift, Alt, Pattern/Song, Play, Stop and Rec.
//! The 128 x 64 OLED is the info screen (key 84). The Volume, Pan, Filter and
//! Resonance knobs are dials 1-4 and the Select encoder is dial 5, whose push
//! presses it.
//!
//! In: pads are notes 54-117 on any channel, the top row 54-69 and the bottom
//! row 102-117, so key = note - 54; any velocity presses and a Note Off, or
//! velocity 0, releases. Each button is a Note On / Off of its own number
//! ([`BUTTON_NOTES`]); the Select push is note 25. The knobs are relative
//! two's-complement Control Change on channel 1 (1-63 up, 127 down by one):
//! CC 16-19 and the Select encoder CC 118. Touching a knob sends note 16-19,
//! which is ignored, as is everything else.
//!
//! Out: pad colours are SysEx `F0 47 7F 43 65 <len hi> <len lo> {<pad> <r>
//! <g> <b>}... F7` with 7-bit channels, all changed pads in one message. Each
//! button light is `B0 <its note> <value>` from that button's fixed colours
//! (dim 1 or bright 3 for single-colour ones, and 1-4 dim and bright of the
//! two colours for the bicolour ones), the nearest to the key's colour; Bank
//! has none. The OLED takes eight SysEx `0E` bands of 8 rows, each 147 bytes
//! of 7 pixels per byte in the deck's own bit order ([`band_bytes`]); a band
//! is sent only when it changed, and the whole screen again on every
//! heartbeat once a second has passed since the last write, because the
//! display sleeps when it is left alone. There is no brightness command:
//! brightness scales the pad colours and picks dim or bright button lights,
//! and 0% also blanks the OLED. The four track LEDs and the knob-mode LEDs
//! are not keys and are turned off on open and when Ryu lets go, which also
//! turns every pad, button and pixel off.
//!
//! Facts from DrivenByMoss (git-moss/DrivenByMoss, LGPL-3.0; its Akai Fire
//! support read for the port names, note and controller numbers, light
//! values and display format); no code was copied.

use std::time::{Duration, Instant};

use super::face;
use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, KeyImage, Protocol};

pub const FIRE_ID: &str = "akai-fire";

pub const PADS: u8 = 64;
pub const KEYS: u8 = 84;
pub const DIALS: u8 = 5;
/// The knob-mode (Bank) button, which has no light of its own.
pub const BANK_KEY: u8 = 73;
/// Pads per row.
const PAD_COLS: u8 = 16;
/// The grid the Companion lays the deck out on: a row of top buttons, the
/// four pad rows with each Mute/Solo button to their left, and the bottom
/// row of buttons.
pub const GRID_ROWS: u8 = 6;
pub const GRID_COLS: u8 = 17;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const SYSEX: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
/// Akai, all-call device, Fire.
const SYSEX_HEADER: [u8; 3] = [0x47, 0x7f, 0x43];
const PAD_COLOURS: u8 = 0x65;
const OLED_WRITE: u8 = 0x0e;

/// The top-left pad's note.
const FIRST_PAD_NOTE: u8 = 54;
/// The Select encoder's push.
const SELECT_PUSH_NOTE: u8 = 0x19;
/// The Select encoder's dial.
const SELECT_DIAL: u8 = 4;
/// Volume, Pan, Filter, Resonance, then the Select encoder.
const DIAL_CCS: [u8; DIALS as usize] = [0x10, 0x11, 0x12, 0x13, 0x76];
/// The knob-mode LEDs: 16 turns all four off.
const MODE_LEDS_CC: u8 = 0x1b;
const MODE_LEDS_OFF: u8 = 16;
/// The four track indicator LEDs beside the Mute/Solo buttons.
const TRACK_LEDS_CC: u8 = 0x28;

/// Each button's note (and light controller), keys 64-83 in order.
pub const BUTTON_NOTES: [u8; (KEYS - PADS) as usize] = [
    0x24, 0x25, 0x26, 0x27, // Mute/Solo 1-4
    0x1f, 0x20, 0x21, 0x22, 0x23, // Pattern Up, Pattern Down, Browser, Grid Left, Grid Right
    0x1a, // Bank
    0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, // Step, Note, Drum, Perform, Shift, Alt
    0x32, 0x33, 0x34, 0x35, // Pattern/Song, Play, Stop, Rec
];

/// The colours a button's light can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Palette {
    None,
    Red,
    Green,
    Orange,
    /// 1 dim red, 2 dim orange, 3 red, 4 orange.
    RedOrange,
    /// 1 dim green, 2 dim orange, 3 green, 4 orange.
    GreenOrange,
}

const PALETTES: [Palette; (KEYS - PADS) as usize] = [
    Palette::Green,
    Palette::Green,
    Palette::Green,
    Palette::Green,
    Palette::Red,
    Palette::Red,
    Palette::Red,
    Palette::Red,
    Palette::Red,
    Palette::None,
    Palette::RedOrange,
    Palette::RedOrange,
    Palette::RedOrange,
    Palette::RedOrange,
    Palette::RedOrange,
    Palette::Orange,
    Palette::GreenOrange,
    Palette::GreenOrange,
    Palette::Orange,
    Palette::RedOrange,
];

const NAMES: [&str; (KEYS - PADS) as usize] = [
    "Mute/Solo 1",
    "Mute/Solo 2",
    "Mute/Solo 3",
    "Mute/Solo 4",
    "Pattern Up",
    "Pattern Down",
    "Browser",
    "Grid Left",
    "Grid Right",
    "Bank",
    "Step",
    "Note",
    "Drum",
    "Perform",
    "Shift",
    "Alt",
    "Pattern/Song",
    "Play",
    "Stop",
    "Rec",
];

/// Each button's (row, column) on the [`GRID_ROWS`] x [`GRID_COLS`] grid.
const BUTTON_CELLS: [(u8, u8); (KEYS - PADS) as usize] = [
    (1, 0),
    (2, 0),
    (3, 0),
    (4, 0),
    (0, 12),
    (0, 13),
    (0, 14),
    (0, 15),
    (0, 16),
    (0, 0),
    (5, 0),
    (5, 1),
    (5, 2),
    (5, 3),
    (5, 5),
    (5, 6),
    (5, 12),
    (5, 13),
    (5, 15),
    (5, 16),
];

/// The OLED's size and its 8-row bands.
const OLED_WIDTH: usize = 128;
const OLED_HEIGHT: usize = 64;
const BANDS: usize = OLED_HEIGHT / 8;
/// One band's pixel bytes: 7 pixels each.
pub const BAND_BYTES: usize = 147;
/// Each pixel's bit number within its group of 7 columns of a band, by row
/// within the band and column within the group.
const BIT_ORDER: [[u8; 7]; 8] = [
    [13, 19, 25, 31, 37, 43, 49],
    [0, 20, 26, 32, 38, 44, 50],
    [1, 7, 27, 33, 39, 45, 51],
    [2, 8, 14, 34, 40, 46, 52],
    [3, 9, 15, 21, 41, 47, 53],
    [4, 10, 16, 22, 28, 48, 54],
    [5, 11, 17, 23, 29, 35, 55],
    [6, 12, 18, 24, 30, 36, 42],
];

/// How long the OLED may go without a write before a heartbeat sends it again.
const KEEP_ALIVE: Duration = Duration::from_secs(1);
/// A button light is off while the brightest channel (after brightness) is
/// under this, dim under [`BRIGHT`], and bright from it.
const DARK: u16 = 32;
const BRIGHT: u16 = 128;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

type Band = [u8; BAND_BYTES];

/// Whether a normalised port name is a Fire port: `FL STUDIO FIRE` or `AKAI
/// FIRE` in any case, and on Windows the first interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.protocol != Protocol::AkaiFireMidi || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.to_ascii_uppercase();
    base.contains("FL STUDIO FIRE") || base.contains("AKAI FIRE")
}

/// The legend on each key: the pads by row and column, then the buttons.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    const PAD_NAMES: [&str; PADS as usize] = [
        "Pad 1-1", "Pad 1-2", "Pad 1-3", "Pad 1-4", "Pad 1-5", "Pad 1-6", "Pad 1-7", "Pad 1-8",
        "Pad 1-9", "Pad 1-10", "Pad 1-11", "Pad 1-12", "Pad 1-13", "Pad 1-14", "Pad 1-15",
        "Pad 1-16", "Pad 2-1", "Pad 2-2", "Pad 2-3", "Pad 2-4", "Pad 2-5", "Pad 2-6", "Pad 2-7",
        "Pad 2-8", "Pad 2-9", "Pad 2-10", "Pad 2-11", "Pad 2-12", "Pad 2-13", "Pad 2-14",
        "Pad 2-15", "Pad 2-16", "Pad 3-1", "Pad 3-2", "Pad 3-3", "Pad 3-4", "Pad 3-5", "Pad 3-6",
        "Pad 3-7", "Pad 3-8", "Pad 3-9", "Pad 3-10", "Pad 3-11", "Pad 3-12", "Pad 3-13",
        "Pad 3-14", "Pad 3-15", "Pad 3-16", "Pad 4-1", "Pad 4-2", "Pad 4-3", "Pad 4-4", "Pad 4-5",
        "Pad 4-6", "Pad 4-7", "Pad 4-8", "Pad 4-9", "Pad 4-10", "Pad 4-11", "Pad 4-12", "Pad 4-13",
        "Pad 4-14", "Pad 4-15", "Pad 4-16",
    ];
    (model.protocol == Protocol::AkaiFireMidi).then(|| {
        PAD_NAMES
            .iter()
            .chain(NAMES.iter())
            .copied()
            .collect::<Vec<_>>()
    })
}

/// Each key's cell on the deck's grid: the pads in rows 1-4 right of the
/// Mute/Solo buttons, the other buttons above and below.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    if model.protocol != Protocol::AkaiFireMidi {
        return None;
    }
    let cols = u16::from(model.cols);
    let pads = (0..PADS).map(|key| (1 + key / PAD_COLS, 1 + key % PAD_COLS));
    Some(
        pads.chain(BUTTON_CELLS.iter().copied())
            .map(|(row, col)| u16::from(row) * cols + u16::from(col))
            .collect(),
    )
}

/// Every key but Bank has a light.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    (model.protocol == Protocol::AkaiFireMidi)
        .then(|| (0..KEYS).filter(|key| *key != BANK_KEY).collect())
}

/// The key a note presses.
fn key_of_note(note: u8) -> Option<u8> {
    if (FIRST_PAD_NOTE..FIRST_PAD_NOTE + PADS).contains(&note) {
        return Some(note - FIRST_PAD_NOTE);
    }
    BUTTON_NOTES
        .iter()
        .position(|button| *button == note)
        .and_then(|at| u8::try_from(at).ok())
        .map(|at| PADS + at)
}

/// The dial a controller turns.
fn dial_of_cc(cc: u8) -> Option<u8> {
    DIAL_CCS
        .iter()
        .position(|dial| *dial == cc)
        .and_then(|at| u8::try_from(at).ok())
}

/// A relative two's-complement step: 1-63 up, 64-127 down (127 is -1).
fn relative(value: u8) -> i8 {
    let value = i16::from(value & 0x7f);
    let delta = if value < 64 { value } else { value - 128 };
    i8::try_from(delta).unwrap_or(0)
}

/// A colour at a brightness, each channel halved to the deck's 7 bits.
fn pad_rgb(color: [u8; 3], percent: u8) -> [u8; 3] {
    color.map(|channel| {
        let scaled = u16::from(channel) * u16::from(percent.min(100)) / 100;
        u8::try_from(scaled >> 1).unwrap_or(0x7f)
    })
}

/// The squared distance between a colour scaled to full strength and a hue.
fn distance(color: [u8; 3], hue: [u8; 3]) -> u32 {
    let peak = u32::from(color.into_iter().max().unwrap_or(0).max(1));
    color
        .into_iter()
        .zip(hue)
        .map(|(channel, target)| {
            let full = u32::from(channel) * 255 / peak;
            full.abs_diff(u32::from(target)).pow(2)
        })
        .sum()
}

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const ORANGE: [u8; 3] = [255, 160, 0];

/// The value a button's light takes for a colour at a brightness: off while
/// dark, else the nearest of its colours, dim or bright.
fn button_value(palette: Palette, color: [u8; 3], percent: u8) -> u8 {
    let peak = u16::from(color.into_iter().max().unwrap_or(0)) * u16::from(percent.min(100)) / 100;
    if palette == Palette::None || peak < DARK {
        return 0;
    }
    let bright = peak >= BRIGHT;
    let (first, second) = match palette {
        Palette::RedOrange => (RED, Some(ORANGE)),
        Palette::GreenOrange => (GREEN, Some(ORANGE)),
        _ => (RED, None),
    };
    let use_second = second.is_some_and(|second| distance(color, second) < distance(color, first));
    match (use_second, bright) {
        (false, false) => 1,
        (true, false) => 2,
        (false, true) => 3,
        (true, true) => 4,
    }
}

/// The OLED's eight bands from a face packed in rows (16 bytes a row, the
/// leftmost pixel in the top bit), in the deck's own order: for the pixel at
/// column `x` and row `y` of a band, bit `b = BIT_ORDER[y][x % 7]` is bit `b %
/// 7` of byte `(x / 7) * 8 + b / 7`.
#[must_use]
pub fn band_bytes(rows: &[u8]) -> [Band; BANDS] {
    let row_bytes = OLED_WIDTH / 8;
    let mut bands = [[0u8; BAND_BYTES]; BANDS];
    for (band_index, band) in bands.iter_mut().enumerate() {
        for (y, order) in BIT_ORDER.iter().enumerate() {
            let row = band_index * 8 + y;
            for x in 0..OLED_WIDTH {
                let lit = rows
                    .get(row * row_bytes + x / 8)
                    .is_some_and(|byte| byte & (0x80 >> (x % 8)) != 0);
                if !lit {
                    continue;
                }
                let bit = usize::from(order.get(x % 7).copied().unwrap_or(0));
                if let Some(byte) = band.get_mut((x / 7) * 8 + bit / 7) {
                    *byte |= 1 << (bit % 7);
                }
            }
        }
    }
    bands
}

/// A SysEx message for the Fire: the header, the command, the payload's
/// length in two 7-bit bytes, the payload and the end byte.
fn sysex(command: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let mut message = Vec::with_capacity(payload.len() + 8);
    message.push(SYSEX);
    message.extend_from_slice(&SYSEX_HEADER);
    message.push(command);
    message.push(u8::try_from((len >> 7) & 0x7f).unwrap_or(0));
    message.push(u8::try_from(len & 0x7f).unwrap_or(0));
    message.extend_from_slice(payload);
    message.push(SYSEX_END);
    message
}

/// One OLED band's write: band `band` across all 128 columns.
fn band_message(band: u8, bytes: &Band) -> Vec<u8> {
    let mut payload = Vec::with_capacity(BAND_BYTES + 4);
    payload.extend_from_slice(&[band, band, 0x00, 0x7f]);
    payload.extend_from_slice(bytes);
    sysex(OLED_WRITE, &payload)
}

pub struct Fire {
    transport: Box<dyn Transport>,
    info: Option<(u8, KeyImage)>,
    /// Every light and the whole screen sent once since open.
    started: bool,
    percent: u8,
    colors: [[u8; 3]; KEYS as usize],
    /// The pad colour or button value each key was last sent.
    sent_pads: [Option<[u8; 3]>; PADS as usize],
    sent_buttons: [Option<u8>; (KEYS - PADS) as usize],
    /// The screen as the face draws it, and as last sent.
    screen: [Band; BANDS],
    sent_screen: [Option<Band>; BANDS],
    screen_sent_at: Option<Instant>,
    held: [bool; KEYS as usize],
    select_held: bool,
    parser: Parser,
}

impl Fire {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            info: model.info_key().zip(model.info_image),
            started: false,
            percent: 100,
            colors: [BLACK; KEYS as usize],
            sent_pads: [None; PADS as usize],
            sent_buttons: [None; (KEYS - PADS) as usize],
            screen: [[0; BAND_BYTES]; BANDS],
            sent_screen: [None; BANDS],
            screen_sent_at: None,
            held: [false; KEYS as usize],
            select_held: false,
            parser: Parser::default(),
        }
    }

    fn color(&self, key: u8) -> [u8; 3] {
        self.colors.get(usize::from(key)).copied().unwrap_or(BLACK)
    }

    /// Turns off the lights that are not keys: the knob-mode and track LEDs.
    fn clear_indicators(&mut self) -> DriverResult<()> {
        self.transport
            .write(&[CONTROL_CHANGE, MODE_LEDS_CC, MODE_LEDS_OFF])?;
        for led in 0..4 {
            self.transport
                .write(&[CONTROL_CHANGE, TRACK_LEDS_CC + led, 0])?;
        }
        Ok(())
    }

    /// Sends the pads whose colour changed (all of them the first time) in one
    /// message.
    fn send_pads(&mut self, all: bool) -> DriverResult<()> {
        let mut payload = Vec::new();
        for pad in 0..PADS {
            let rgb = pad_rgb(self.color(pad), self.percent);
            let Some(sent) = self.sent_pads.get_mut(usize::from(pad)) else {
                continue;
            };
            if all || *sent != Some(rgb) {
                payload.extend_from_slice(&[pad, rgb[0], rgb[1], rgb[2]]);
                *sent = Some(rgb);
            }
        }
        if payload.is_empty() {
            return Ok(());
        }
        self.transport.write(&sysex(PAD_COLOURS, &payload))
    }

    fn send_buttons(&mut self, all: bool) -> DriverResult<()> {
        for (at, (note, palette)) in BUTTON_NOTES.iter().zip(PALETTES).enumerate() {
            if palette == Palette::None {
                continue;
            }
            let key = PADS + u8::try_from(at).unwrap_or(0);
            let value = button_value(palette, self.color(key), self.percent);
            let Some(sent) = self.sent_buttons.get_mut(at) else {
                continue;
            };
            if all || *sent != Some(value) {
                *sent = Some(value);
                self.transport.write(&[CONTROL_CHANGE, *note, value])?;
            }
        }
        Ok(())
    }

    /// Sends the bands that changed (every band with `all`).
    fn send_screen(&mut self, all: bool) -> DriverResult<()> {
        let blank = [0u8; BAND_BYTES];
        for band in 0..BANDS {
            let bytes = if self.percent == 0 {
                blank
            } else {
                self.screen.get(band).copied().unwrap_or(blank)
            };
            let Some(sent) = self.sent_screen.get_mut(band) else {
                continue;
            };
            if all || *sent != Some(bytes) {
                *sent = Some(bytes);
                let index = u8::try_from(band).unwrap_or(0);
                self.transport.write(&band_message(index, &bytes))?;
                self.screen_sent_at = Some(Instant::now());
            }
        }
        Ok(())
    }

    fn send(&mut self, all: bool) -> DriverResult<()> {
        if all {
            self.clear_indicators()?;
        }
        self.send_pads(all)?;
        self.send_buttons(all)?;
        self.send_screen(all)?;
        self.started = true;
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
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        match status & 0xf0 {
            NOTE_ON | NOTE_OFF => {
                let pressed = status & 0xf0 == NOTE_ON && value > 0;
                if number == SELECT_PUSH_NOTE {
                    if self.select_held != pressed {
                        self.select_held = pressed;
                        events.push(Input::DialPress {
                            index: SELECT_DIAL,
                            pressed,
                        });
                    }
                } else if let Some(key) = key_of_note(number) {
                    self.press(key, pressed, events);
                }
            }
            CONTROL_CHANGE if status == CONTROL_CHANGE => {
                if let Some(dial) = dial_of_cc(number) {
                    let delta = relative(value);
                    if delta != 0 {
                        events.push(Input::DialTurn { index: dial, delta });
                    }
                }
            }
            _ => {}
        }
    }
}

impl Driver for Fire {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
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

    /// The info key draws the OLED; every other key keeps its colour for its
    /// light.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some((info, format)) = self.info {
            if key == info {
                let rows = face::encode(face, &format)?;
                self.screen = band_bytes(&rows);
                return Ok(());
            }
        }
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// Everything the first time (the deck shows its own state until then),
    /// then only what changed.
    fn flush(&mut self) -> DriverResult<()> {
        let all = !self.started;
        self.send(all)
    }

    /// The Fire has no brightness command: the pads are scaled, the buttons
    /// shown dim or bright, and 0% also blanks the screen.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.percent = percent.min(100);
        if self.started {
            self.send(false)?;
        }
        Ok(())
    }

    /// Sends the whole screen again unless it was written in the last second:
    /// the OLED sleeps when left alone.
    fn heartbeat(&mut self) -> DriverResult<()> {
        if !self.started {
            return Ok(());
        }
        if self
            .screen_sent_at
            .is_some_and(|at| at.elapsed() < KEEP_ALIVE)
        {
            return Ok(());
        }
        self.send_screen(true)
    }
}

impl Drop for Fire {
    /// Turns every pad, button, indicator and pixel off: the deck would
    /// otherwise keep them.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        let mut payload = Vec::with_capacity(usize::from(PADS) * 4);
        for pad in 0..PADS {
            payload.extend_from_slice(&[pad, 0, 0, 0]);
        }
        let _ = self.transport.write(&sysex(PAD_COLOURS, &payload));
        for (note, palette) in BUTTON_NOTES.iter().zip(PALETTES) {
            if palette != Palette::None {
                let _ = self.transport.write(&[CONTROL_CHANGE, *note, 0]);
            }
        }
        let _ = self.clear_indicators();
        let blank = [0u8; BAND_BYTES];
        for band in 0..BANDS {
            let index = u8::try_from(band).unwrap_or(0);
            let _ = self.transport.write(&band_message(index, &blank));
        }
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, ModelView};

    fn open() -> (Fire, Recorder) {
        let recorder = Recorder::default();
        let deck = Fire::new(by_id(FIRE_ID).unwrap(), Box::new(recorder.clone()));
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

    fn queue(recorder: &Recorder, messages: &[&[u8]]) {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
    }

    #[test]
    fn the_model_has_its_keys_screen_and_dials() {
        let model = by_id(FIRE_ID).unwrap();
        assert_eq!(model.protocol, Protocol::AkaiFireMidi);
        assert_eq!((model.keys, model.dials), (KEYS, DIALS));
        assert_eq!(model.info_key(), Some(84));
        assert_eq!(model.total_keys(), 85);
        let view = ModelView::from(model);
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), usize::from(KEYS));
        assert_eq!(names[0], "Pad 1-1");
        assert_eq!(names[63], "Pad 4-16");
        assert_eq!(names[64], "Mute/Solo 1");
        assert_eq!(names[usize::from(BANK_KEY)], "Bank");
        assert_eq!(names[83], "Rec");
        let lit = view.lit_keys.unwrap();
        assert_eq!(lit.len(), usize::from(KEYS) - 1);
        assert!(!lit.contains(&BANK_KEY));
        let cells = view.key_cells.unwrap();
        assert_eq!(cells.len(), usize::from(KEYS));
        let total = u16::from(GRID_ROWS) * u16::from(GRID_COLS);
        let mut seen = std::collections::HashSet::new();
        for cell in &cells {
            assert!(*cell < total, "{cell}");
            assert!(seen.insert(*cell), "{cell}");
        }
        // Top-left pad right of Mute/Solo 1; the first Mute/Solo at its left.
        assert_eq!(cells[0], u16::from(GRID_COLS) + 1);
        assert_eq!(cells[64], u16::from(GRID_COLS));
    }

    #[test]
    fn ports_are_found_by_name() {
        let model = by_id(FIRE_ID).unwrap();
        for name in [
            "FL STUDIO FIRE",
            "FL STUDIO FIRE Jack 1",
            "Akai Fire",
            "FL STUDIO FIRE:FL STUDIO FIRE MIDI 1 24:0",
            "fl studio fire",
        ] {
            assert!(port_matches(model, &normalise(name)), "{name}");
            assert_eq!(
                identify_midi(&normalise(name), false).map(|found| found.id),
                Some(FIRE_ID),
                "{name}"
            );
        }
        for name in ["FIRE", "Campfire", "APC MINI", "MIDIIN2 (FL STUDIO FIRE)"] {
            assert!(!port_matches(model, &normalise(name)), "{name}");
        }
        let other = by_id("akai-lpd8").unwrap();
        assert!(!port_matches(other, &normalise("FL STUDIO FIRE")));
    }

    #[test]
    fn notes_map_to_pads_buttons_and_the_select_push() {
        assert_eq!(key_of_note(54), Some(0));
        assert_eq!(key_of_note(69), Some(15));
        assert_eq!(key_of_note(70), Some(16));
        assert_eq!(key_of_note(102), Some(48));
        assert_eq!(key_of_note(117), Some(63));
        assert_eq!(key_of_note(0x24), Some(64));
        assert_eq!(key_of_note(0x27), Some(67));
        assert_eq!(key_of_note(0x1f), Some(68));
        assert_eq!(key_of_note(0x23), Some(72));
        assert_eq!(key_of_note(0x1a), Some(BANK_KEY));
        assert_eq!(key_of_note(0x2c), Some(74));
        assert_eq!(key_of_note(0x30), Some(78));
        assert_eq!(key_of_note(0x33), Some(81));
        assert_eq!(key_of_note(0x35), Some(83));
        for note in [0, 0x10, 0x13, 0x19, 0x1b, 0x28, 0x2b, 118, 127] {
            assert_eq!(key_of_note(note), None, "{note}");
        }
        assert_eq!(dial_of_cc(0x10), Some(0));
        assert_eq!(dial_of_cc(0x13), Some(3));
        assert_eq!(dial_of_cc(0x76), Some(4));
        assert_eq!(dial_of_cc(0x14), None);
        assert_eq!(relative(1), 1);
        assert_eq!(relative(63), 63);
        assert_eq!(relative(64), -64);
        assert_eq!(relative(127), -1);
        assert_eq!(relative(0), 0);
    }

    #[test]
    fn input_turns_into_keys_and_dials() {
        let (mut deck, recorder) = open();
        queue(
            &recorder,
            &[
                &[0x90, 54, 100],
                &[0x80, 54, 0],
                &[0x91, 117, 20],
                &[0x91, 117, 0],
                &[0x90, 0x33, 127],
                &[0x80, 0x33, 0],
                // Touching Volume is ignored; the turn is a step.
                &[0x90, 0x10, 127],
                &[0xb0, 0x10, 0x01],
                &[0xb0, 0x11, 0x7f],
                &[0xb0, 0x76, 0x03],
                // Another channel's controller and an unknown one are dropped.
                &[0xb1, 0x10, 0x01],
                &[0xb0, 0x40, 0x01],
                &[0x90, 0x19, 127],
                &[0x80, 0x19, 0],
                &[0x80, 0x10, 0],
                &[0xf0, 0x47, 0x7f, 0x43, 0xf7],
                &[0x90],
            ],
        );
        let events = deck.poll(0).unwrap();
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(0, false),
                key(63, true),
                key(63, false),
                key(81, true),
                key(81, false),
                Input::DialTurn { index: 0, delta: 1 },
                Input::DialTurn {
                    index: 1,
                    delta: -1
                },
                Input::DialTurn { index: 4, delta: 3 },
                Input::DialPress {
                    index: 4,
                    pressed: true
                },
                Input::DialPress {
                    index: 4,
                    pressed: false
                },
            ]
        );
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn the_first_flush_sends_everything_and_later_ones_only_changes() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 128]))).unwrap();
        deck.set_face(64, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        // Indicators off first.
        assert_eq!(writes[0], vec![0xb0, 0x1b, 0x10]);
        assert_eq!(
            writes[1..5].to_vec(),
            vec![
                vec![0xb0, 0x28, 0],
                vec![0xb0, 0x29, 0],
                vec![0xb0, 0x2a, 0],
                vec![0xb0, 0x2b, 0],
            ]
        );
        // Every pad in one message: 64 x 4 = 256 = 2 * 128 + 0.
        let pads = &writes[5];
        assert_eq!(pads.len(), 264);
        assert_eq!(
            pads[..9],
            [0xf0, 0x47, 0x7f, 0x43, 0x65, 0x02, 0x00, 0, 127]
        );
        assert_eq!(pads[9..11], [0, 64]);
        assert_eq!(pads[11..15], [1, 0, 0, 0]);
        assert_eq!(pads[263], 0xf7);
        // Then the 19 button lights (Bank has none): Mute/Solo 1 bright green.
        assert_eq!(writes[6], vec![0xb0, 0x24, 3]);
        assert_eq!(writes[7], vec![0xb0, 0x25, 0]);
        let buttons: Vec<u8> = writes[6..25].iter().map(|write| write[1]).collect();
        assert!(!buttons.contains(&0x1a));
        assert_eq!(buttons.len(), 19);
        // Then the eight screen bands.
        for (band, write) in writes[25..33].iter().enumerate() {
            assert_eq!(write.len(), 159);
            assert_eq!(
                write[..11],
                [0xf0, 0x47, 0x7f, 0x43, 0x0e, 0x01, 0x17, band as u8, band as u8, 0, 0x7f]
            );
            assert!(write[11..158].iter().all(|byte| *byte == 0));
            assert_eq!(write[158], 0xf7);
        }
        assert_eq!(writes.len(), 33);

        // Nothing changed: nothing sent.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 33);

        // One pad and one button change.
        deck.set_face(5, Some(&lit([20, 40, 60]))).unwrap();
        deck.set_face(83, Some(&lit([255, 200, 0]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[33],
            vec![0xf0, 0x47, 0x7f, 0x43, 0x65, 0x00, 0x04, 5, 10, 20, 30, 0xf7]
        );
        assert_eq!(writes[34], vec![0xb0, 0x35, 4]);
        assert_eq!(writes.len(), 35);
    }

    #[test]
    fn button_values_follow_each_palette() {
        let full = 100;
        assert_eq!(button_value(Palette::Red, [255, 0, 0], full), 3);
        assert_eq!(button_value(Palette::Red, [0, 0, 255], full), 3);
        assert_eq!(button_value(Palette::Red, [80, 0, 0], full), 1);
        assert_eq!(button_value(Palette::Red, [255, 0, 0], 40), 1);
        assert_eq!(button_value(Palette::Red, [20, 20, 20], full), 0);
        assert_eq!(button_value(Palette::Red, [255, 255, 255], 0), 0);
        assert_eq!(button_value(Palette::Green, [0, 255, 0], full), 3);
        assert_eq!(button_value(Palette::Orange, [255, 128, 0], full), 3);
        assert_eq!(button_value(Palette::RedOrange, [255, 0, 0], full), 3);
        assert_eq!(button_value(Palette::RedOrange, [255, 165, 0], full), 4);
        assert_eq!(button_value(Palette::RedOrange, [255, 255, 0], full), 4);
        assert_eq!(button_value(Palette::RedOrange, [100, 60, 0], full), 2);
        assert_eq!(button_value(Palette::GreenOrange, [0, 255, 0], full), 3);
        assert_eq!(button_value(Palette::GreenOrange, [255, 200, 0], full), 4);
        assert_eq!(button_value(Palette::GreenOrange, [0, 90, 0], full), 1);
        assert_eq!(button_value(Palette::None, [255, 255, 255], full), 0);
        assert_eq!(pad_rgb([255, 128, 1], 100), [127, 64, 0]);
        assert_eq!(pad_rgb([255, 255, 255], 50), [63, 63, 63]);
    }

    #[test]
    fn the_screen_is_packed_seven_pixels_a_byte() {
        let mut rows = vec![0u8; 16 * 64];
        // (0, 0): bit 13 -> byte 1, bit 6.
        rows[0] |= 0x80;
        let bands = band_bytes(&rows);
        assert_eq!(bands[0][1], 1 << 6);
        assert_eq!(bands[0].iter().filter(|byte| **byte != 0).count(), 1);
        // (0, 1): bit 0 -> byte 0, bit 0.
        let mut rows = vec![0u8; 16 * 64];
        rows[16] |= 0x80;
        assert_eq!(band_bytes(&rows)[0][0], 1);
        // (7, 8): row 0 of band 1, second group: bit 13 -> byte 9, bit 6.
        let mut rows = vec![0u8; 16 * 64];
        rows[8 * 16] |= 0x80 >> 7;
        let bands = band_bytes(&rows);
        assert_eq!(bands[1][9], 1 << 6);
        assert!(bands[0].iter().all(|byte| *byte == 0));
        // (127, 63): column 1 of group 18, row 7 of band 7: bit 12 -> byte 145, bit 5.
        let mut rows = vec![0u8; 16 * 64];
        rows[63 * 16 + 15] |= 0x01;
        assert_eq!(band_bytes(&rows)[7][145], 1 << 5);
        // Every pixel lit fills each band's bits without overflowing.
        let bands = band_bytes(&[0xff; 16 * 64]);
        let set: u32 = bands[0].iter().map(|byte| byte.count_ones()).sum();
        assert_eq!(set, 128 * 8);
        assert!(bands[0].iter().all(|byte| *byte < 0x80));
        // Short input is blank, never a panic.
        assert!(band_bytes(&[]).iter().flatten().all(|byte| *byte == 0));
    }

    #[test]
    fn the_info_key_draws_the_screen_and_heartbeats_keep_it_awake() {
        let (mut deck, recorder) = open();
        let mut picture = RgbaImage::from_pixel(128, 64, Rgba([0, 0, 0, 255]));
        picture.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        let face = Face {
            label: String::new(),
            color: [0, 0, 0],
            image: Some(picture),
        };
        deck.set_face(84, Some(&face)).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        let band0 = &writes[25];
        assert_eq!(band0[7], 0);
        assert_eq!(band0[11 + 1], 1 << 6);
        // The info key is not a pad light.
        assert_eq!(writes[5][7..11], [0, 0, 0, 0]);
        // Only the changed band goes out.
        deck.set_face(84, None).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 34);
        assert_eq!(writes[33][7], 0);
        assert!(writes[33][11..158].iter().all(|byte| *byte == 0));
        // Written just now: the heartbeat waits.
        deck.heartbeat().unwrap();
        assert_eq!(recorder.writes().len(), 34);
        // Once a second has passed, the whole screen goes out again.
        deck.screen_sent_at = Instant::now().checked_sub(KEEP_ALIVE);
        deck.heartbeat().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 42);
        assert!(writes[34..].iter().all(|write| write[4] == 0x0e));
    }

    #[test]
    fn brightness_scales_pads_and_zero_blanks_the_screen() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(68, Some(&lit([255, 0, 0]))).unwrap();
        let mut picture = RgbaImage::from_pixel(128, 64, Rgba([255, 255, 255, 255]));
        picture.put_pixel(0, 0, Rgba([0, 0, 0, 255]));
        let face = Face {
            label: String::new(),
            color: [0, 0, 0],
            image: Some(picture),
        };
        deck.set_face(84, Some(&face)).unwrap();
        // Before the first flush nothing is sent.
        deck.set_brightness(50).unwrap();
        assert!(recorder.writes().is_empty());
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[5][7..11], [0, 63, 63, 63]);
        // Pattern Up's full red at half brightness is dim.
        assert_eq!(writes[10], vec![0xb0, 0x1f, 1]);
        deck.set_brightness(0).unwrap();
        let writes = recorder.writes()[33..].to_vec();
        assert_eq!(
            writes[0],
            vec![0xf0, 0x47, 0x7f, 0x43, 0x65, 0x00, 0x04, 0, 0, 0, 0, 0xf7]
        );
        assert_eq!(writes[1], vec![0xb0, 0x1f, 0]);
        assert_eq!(writes.len(), 10);
        assert!(writes[2..]
            .iter()
            .all(|write| write[4] == 0x0e && write[11..158].iter().all(|byte| *byte == 0)));
    }

    #[test]
    fn letting_go_turns_everything_off() {
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
        let (mut opened, recorder) = open();
        opened.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        opened.flush().unwrap();
        let before = recorder.writes().len();
        drop(opened);
        let writes = recorder.writes()[before..].to_vec();
        assert_eq!(writes[0].len(), 264);
        assert!(writes[0][7..263]
            .chunks(4)
            .enumerate()
            .all(|(pad, entry)| entry == [pad as u8, 0, 0, 0]));
        assert_eq!(
            writes[1..20].iter().filter(|write| write[2] == 0).count(),
            19
        );
        assert_eq!(writes[20], vec![0xb0, 0x1b, 0x10]);
        assert_eq!(writes[24], vec![0xb0, 0x2b, 0]);
        assert_eq!(writes.len(), 33);
        assert!(writes[25..].iter().all(|write| write.len() == 159));
    }
}
