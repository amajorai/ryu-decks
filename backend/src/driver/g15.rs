//! Logitech G15, G15 v2, G11, G510 and G110: the G-key, mode-key and LCD
//! block of Logitech's gaming keyboards. The ordinary keys stay a keyboard;
//! Ryu drives only the vendor HID collection that carries the G-keys.
//!
//! - The G15 and G15 v2 put the G-keys and the LCD on a USB device of their
//!   own behind the keyboard's built-in hub (046d:c222 and c227); the G11 is
//!   a G15 without the LCD (c225). On the G510 (c22d, or c22e with its USB
//!   audio switched on) they are HID interface 1, beside the boot keyboard on
//!   interface 0. The G110 (c22b) has G-keys but no LCD.
//! - Every key report holds the whole state of the vendor keys, so Ryu raises
//!   a press or release only when a key's bit changes. Each model has its own
//!   report id and length (`Kind::key_report`), and anything else is dropped:
//!   the G15 sends a second report with id `01` per press. The G15 and G11
//!   bitmap is scrambled, so every key reads its own `(byte, mask)`.
//! - Nothing needs a handshake or keep-alive. The G15, G11 and G510 also type
//!   F1 to F12 or 1 to 6 from their G-keys until the host sends an output
//!   report (G15, G11: `02` and 18 zeros; G15 v2: `02` and six zeros) or a
//!   feature report (G510: `01` and 18 zeros). The device only takes the
//!   output report while a read is pending, so Ryu sends it after its first
//!   read. A refusal is logged, not fatal.
//! - The LCD is the G13's: output report `03`, 31 zero bytes, then 960 bytes
//!   of 8-row column bands (see [`g13::lcd_report`]). The glass shows a set
//!   bit as a dark pixel, and the shared packer sets a bit where the face is
//!   bright, so a light label on a dark key reads as dark text on the backlit
//!   screen.
//! - Lights are feature reports. G15 family: `02 01 level 00` for the keyboard
//!   backlight and `02 02 level<<4 00` for the LCD's (levels 0 to 2), and
//!   `02 04 ~mask 00` for the M1 to MR LEDs, a clear bit lighting one (M1 is
//!   bit 0). Reading feature `02` answers `02 keyboard lcd ~mask`. G510:
//!   `05 r g b` for the RGB backlight of the keys and the LCD, and `04 mask`
//!   for the LEDs, not inverted, M1 = 0x80 down to MR = 0x10; it ignores
//!   colours while its light button has the backlight off, which input report
//!   `04` (bit 0x04 of byte 1) says, so the colour is sent again when it comes
//!   back on. G110: `07 blend 00 00 level`, a red-to-blue mix and a level.
//!   Which end of the mix is red is not known, so Ryu keeps the G110's mix and
//!   only sets the level; its LEDs take the G15 report, which no source
//!   confirms the G110 honours, so a refusal is only logged.
//! - Ryu leaves the backlights as it found them until a brightness or deck
//!   light is set, and puts them back when it lets go.
//!
//! Facts per the Linux kernel's hid-lg-g15 driver (GPL-2.0+) and libg15 from
//! g15tools (GPL-2.0), and for the G110's backlight Tom Van Braeckel's
//! Logitech_G110_change_color (GPL-2.0); no code was copied. Only the Linux
//! path is known to pass the 992-byte LCD report as is; on Windows the G510's
//! feature reports and report `04` may sit in a collection of their own.

use super::{g13, Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, KeyImage};

pub const G15_ID: &str = "logitech-g15";
pub const G15_V2_ID: &str = "logitech-g15-v2";
pub const G11_ID: &str = "logitech-g11";
pub const G510_ID: &str = "logitech-g510";
pub const G110_ID: &str = "logitech-g110";

/// Bigger than any report, so a report of another length is seen and dropped.
const READ_BUFFER: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 32;
/// The G15 family's feature report, and its sub-commands.
const G15_FEATURE: u8 = 0x02;
const G15_KEYBOARD_BACKLIGHT: u8 = 0x01;
const G15_LCD_BACKLIGHT: u8 = 0x02;
const G15_LEDS: u8 = 0x04;
/// The G510's feature reports and its light-state input report.
const G510_INIT: u8 = 0x01;
const G510_LEDS: u8 = 0x04;
const G510_BACKLIGHT: u8 = 0x05;
const G510_LIGHT_STATE: u8 = 0x04;
const G510_LIGHT_STATE_LEN: usize = 2;
/// Set in byte 1 of the light-state report while the backlight is off.
const G510_BACKLIGHT_OFF: u8 = 0x04;
/// The G110's backlight feature report: id, mix, two zeros, level.
const G110_BACKLIGHT: u8 = 0x07;
const G110_BACKLIGHT_LEN: usize = 5;
/// A mix halfway between red and blue, used when the G110's own cannot be read.
const G110_EVEN_MIX: u8 = 0x80;
/// Zero bytes after the id in the reports that stop the G-keys typing.
const INIT_ZEROS: usize = 18;
const G15_V2_INIT_ZEROS: usize = 6;
/// The G15 family's backlights have three levels.
const TOP_LEVEL: u8 = 2;
/// Brightness percentages up to this are the middle level.
const MIDDLE_LEVEL_UP_TO: u8 = 66;
/// A light that is only on or off is on for any colour brighter than this, as
/// on the G13.
const ON_OFF_THRESHOLD: u8 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    G15,
    G15V2,
    G11,
    G510,
    G110,
}

/// One key: its legend, its `(row, column)` on the model's picture, and the
/// byte and bit of the key report that hold it.
#[derive(Clone, Copy)]
struct KeyDef {
    name: &'static str,
    place: (u8, u8),
    byte: usize,
    mask: u8,
}

const fn key(name: &'static str, place: (u8, u8), byte: usize, mask: u8) -> KeyDef {
    KeyDef {
        name,
        place,
        byte,
        mask,
    }
}

/// G15 and G11: G1 to G18 spread across bytes 1 to 8 with shifting bits.
const G15_G_KEYS: [KeyDef; 18] = [
    key("G1", (2, 0), 1, 0x01),
    key("G2", (2, 1), 2, 0x02),
    key("G3", (2, 2), 3, 0x04),
    key("G4", (3, 0), 4, 0x08),
    key("G5", (3, 1), 5, 0x10),
    key("G6", (3, 2), 6, 0x20),
    key("G7", (4, 0), 2, 0x01),
    key("G8", (4, 1), 3, 0x02),
    key("G9", (4, 2), 4, 0x04),
    key("G10", (5, 0), 5, 0x08),
    key("G11", (5, 1), 6, 0x10),
    key("G12", (5, 2), 7, 0x20),
    key("G13", (6, 0), 1, 0x04),
    key("G14", (6, 1), 2, 0x08),
    key("G15", (6, 2), 3, 0x10),
    key("G16", (7, 0), 4, 0x20),
    key("G17", (7, 1), 5, 0x40),
    key("G18", (7, 2), 8, 0x40),
];

/// The G15: G1 to G18, M1 to MR, the round button and L2 to L5 under the
/// LCD, and the backlight button. The LCD is key 28.
const G15_KEYS: [KeyDef; 28] = [
    G15_G_KEYS[0],
    G15_G_KEYS[1],
    G15_G_KEYS[2],
    G15_G_KEYS[3],
    G15_G_KEYS[4],
    G15_G_KEYS[5],
    G15_G_KEYS[6],
    G15_G_KEYS[7],
    G15_G_KEYS[8],
    G15_G_KEYS[9],
    G15_G_KEYS[10],
    G15_G_KEYS[11],
    G15_G_KEYS[12],
    G15_G_KEYS[13],
    G15_G_KEYS[14],
    G15_G_KEYS[15],
    G15_G_KEYS[16],
    G15_G_KEYS[17],
    key("M1", (1, 0), 6, 0x01),
    key("M2", (1, 1), 7, 0x02),
    key("M3", (1, 2), 8, 0x04),
    key("MR", (1, 3), 7, 0x40),
    key("L1", (0, 0), 8, 0x80),
    key("L2", (0, 1), 2, 0x80),
    key("L3", (0, 2), 3, 0x80),
    key("L4", (0, 3), 4, 0x80),
    key("L5", (0, 4), 5, 0x80),
    key("Light", (1, 4), 1, 0x80),
];

/// The G11: the G15's keys without the LCD buttons, a row higher.
const G11_KEYS: [KeyDef; 23] = [
    key("G1", (1, 0), 1, 0x01),
    key("G2", (1, 1), 2, 0x02),
    key("G3", (1, 2), 3, 0x04),
    key("G4", (2, 0), 4, 0x08),
    key("G5", (2, 1), 5, 0x10),
    key("G6", (2, 2), 6, 0x20),
    key("G7", (3, 0), 2, 0x01),
    key("G8", (3, 1), 3, 0x02),
    key("G9", (3, 2), 4, 0x04),
    key("G10", (4, 0), 5, 0x08),
    key("G11", (4, 1), 6, 0x10),
    key("G12", (4, 2), 7, 0x20),
    key("G13", (5, 0), 1, 0x04),
    key("G14", (5, 1), 2, 0x08),
    key("G15", (5, 2), 3, 0x10),
    key("G16", (6, 0), 4, 0x20),
    key("G17", (6, 1), 5, 0x40),
    key("G18", (6, 2), 8, 0x40),
    key("M1", (0, 0), 6, 0x01),
    key("M2", (0, 1), 7, 0x02),
    key("M3", (0, 2), 8, 0x04),
    key("MR", (0, 3), 7, 0x40),
    key("Light", (1, 3), 1, 0x80),
];

/// The G15 v2: six G-keys in a column, M1 to MR, the round button and L2 to
/// L5, and the backlight button. The LCD is key 16.
const G15_V2_KEYS: [KeyDef; 16] = [
    key("G1", (2, 0), 1, 0x01),
    key("G2", (3, 0), 1, 0x02),
    key("G3", (4, 0), 1, 0x04),
    key("G4", (5, 0), 1, 0x08),
    key("G5", (6, 0), 1, 0x10),
    key("G6", (7, 0), 1, 0x20),
    key("M1", (1, 0), 1, 0x40),
    key("M2", (1, 1), 1, 0x80),
    key("M3", (1, 2), 2, 0x20),
    key("MR", (1, 3), 2, 0x40),
    key("L1", (0, 0), 2, 0x80),
    key("L2", (0, 1), 2, 0x02),
    key("L3", (0, 2), 2, 0x04),
    key("L4", (0, 3), 2, 0x08),
    key("L5", (0, 4), 2, 0x10),
    key("Light", (1, 4), 2, 0x01),
];

/// The G510: G1 to G18 as a plain bitmap, M1 to MR, L1 to L5, the backlight
/// button, the game-mode switch (held while on) and the headphone and mic
/// mute keys. The LCD is key 31.
const G510_KEYS: [KeyDef; 31] = [
    key("G1", (3, 0), 1, 0x01),
    key("G2", (3, 1), 1, 0x02),
    key("G3", (3, 2), 1, 0x04),
    key("G4", (4, 0), 1, 0x08),
    key("G5", (4, 1), 1, 0x10),
    key("G6", (4, 2), 1, 0x20),
    key("G7", (5, 0), 1, 0x40),
    key("G8", (5, 1), 1, 0x80),
    key("G9", (5, 2), 2, 0x01),
    key("G10", (6, 0), 2, 0x02),
    key("G11", (6, 1), 2, 0x04),
    key("G12", (6, 2), 2, 0x08),
    key("G13", (7, 0), 2, 0x10),
    key("G14", (7, 1), 2, 0x20),
    key("G15", (7, 2), 2, 0x40),
    key("G16", (8, 0), 2, 0x80),
    key("G17", (8, 1), 3, 0x01),
    key("G18", (8, 2), 3, 0x02),
    key("M1", (1, 0), 3, 0x10),
    key("M2", (1, 1), 3, 0x20),
    key("M3", (1, 2), 3, 0x40),
    key("MR", (1, 3), 3, 0x80),
    key("L1", (0, 0), 4, 0x01),
    key("L2", (0, 1), 4, 0x02),
    key("L3", (0, 2), 4, 0x04),
    key("L4", (0, 3), 4, 0x08),
    key("L5", (0, 4), 4, 0x10),
    key("Light", (1, 4), 3, 0x08),
    key("Game mode", (2, 0), 3, 0x04),
    key("Headphone mute", (2, 1), 4, 0x20),
    key("Mic mute", (2, 2), 4, 0x40),
];

/// The G110: G1 to G12, M1 to MR and the backlight button.
const G110_KEYS: [KeyDef; 17] = [
    key("G1", (1, 0), 1, 0x01),
    key("G2", (1, 1), 1, 0x02),
    key("G3", (1, 2), 1, 0x04),
    key("G4", (2, 0), 1, 0x08),
    key("G5", (2, 1), 1, 0x10),
    key("G6", (2, 2), 1, 0x20),
    key("G7", (3, 0), 1, 0x40),
    key("G8", (3, 1), 1, 0x80),
    key("G9", (3, 2), 2, 0x01),
    key("G10", (4, 0), 2, 0x02),
    key("G11", (4, 1), 2, 0x04),
    key("G12", (4, 2), 2, 0x08),
    key("M1", (0, 0), 2, 0x10),
    key("M2", (0, 1), 2, 0x20),
    key("M3", (0, 2), 2, 0x40),
    key("MR", (0, 3), 2, 0x80),
    key("Light", (1, 3), 3, 0x01),
];

impl Kind {
    fn of(model: &DeviceModel) -> Option<Self> {
        match model.id {
            G15_ID => Some(Self::G15),
            G15_V2_ID => Some(Self::G15V2),
            G11_ID => Some(Self::G11),
            G510_ID => Some(Self::G510),
            G110_ID => Some(Self::G110),
            _ => None,
        }
    }

    const fn keys(self) -> &'static [KeyDef] {
        match self {
            Self::G15 => &G15_KEYS,
            Self::G15V2 => &G15_V2_KEYS,
            Self::G11 => &G11_KEYS,
            Self::G510 => &G510_KEYS,
            Self::G110 => &G110_KEYS,
        }
    }

    /// The id and length of the report that carries the keys.
    const fn key_report(self) -> (u8, usize) {
        match self {
            Self::G15 | Self::G11 => (0x02, 9),
            Self::G15V2 => (0x02, 5),
            Self::G510 => (0x03, 5),
            Self::G110 => (0x02, 4),
        }
    }

    /// The key index of M1; M2, M3 and MR follow it.
    const fn first_mode_key(self) -> u8 {
        match self {
            Self::G15 | Self::G11 | Self::G510 => 18,
            Self::G15V2 => 6,
            Self::G110 => 12,
        }
    }

    /// The backlight button's key index.
    const fn light_key(self) -> u8 {
        match self {
            Self::G15 | Self::G510 => 27,
            Self::G11 => 22,
            Self::G15V2 => 15,
            Self::G110 => 16,
        }
    }

    /// Sets its backlight levels with the G15 feature report `02`.
    const fn has_levels(self) -> bool {
        matches!(self, Self::G15 | Self::G15V2 | Self::G11)
    }

    const fn has_lcd(self) -> bool {
        matches!(self, Self::G15 | Self::G15V2 | Self::G510)
    }

    /// The report that stops the G-keys typing, and whether it is a feature
    /// report rather than an output report.
    fn init_report(self) -> Option<(Vec<u8>, bool)> {
        let with_zeros = |id: u8, zeros: usize| {
            let mut report = vec![0u8; 1 + zeros];
            report[0] = id;
            report
        };
        match self {
            Self::G15 | Self::G11 => Some((with_zeros(G15_FEATURE, INIT_ZEROS), false)),
            Self::G15V2 => Some((with_zeros(G15_FEATURE, G15_V2_INIT_ZEROS), false)),
            Self::G510 => Some((with_zeros(G510_INIT, INIT_ZEROS), true)),
            Self::G110 => None,
        }
    }

    /// The feature report that lights the mode LEDs in `mask` (M1 is bit 0).
    fn leds_report(self, mask: u8) -> Vec<u8> {
        if self == Self::G510 {
            let mut bits = 0u8;
            for led in 0..4 {
                if mask & (1 << led) != 0 {
                    bits |= 0x80 >> led;
                }
            }
            vec![G510_LEDS, bits]
        } else {
            vec![G15_FEATURE, G15_LEDS, !mask, 0]
        }
    }
}

/// The G15 family's backlight level for a brightness percentage.
const fn level_for(percent: u8) -> u8 {
    match percent {
        0 => 0,
        1..=MIDDLE_LEVEL_UP_TO => 1,
        _ => TOP_LEVEL,
    }
}

/// Each key's name, on one of these keyboards.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    Kind::of(model).map(|kind| kind.keys().iter().map(|key| key.name).collect())
}

/// Each key's cell in the model's grid, on one of these keyboards.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    Kind::of(model).map(|kind| {
        kind.keys()
            .iter()
            .map(|key| u16::from(key.place.0) * u16::from(model.cols) + u16::from(key.place.1))
            .collect()
    })
}

/// M1, M2, M3 and MR: the keys with an LED, on one of these keyboards.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    Kind::of(model).map(|kind| {
        let first = kind.first_mode_key();
        (first..first + 4).collect()
    })
}

pub struct G15 {
    kind: Kind,
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    /// Whether each key was held at the last key report.
    held: Vec<bool>,
    /// Whether the report that stops the G-keys typing has been sent.
    init_sent: bool,
    /// The LCD frame to show, and the one last sent.
    lcd: Vec<u8>,
    sent_lcd: Option<Vec<u8>>,
    /// The mode LEDs to light (M1 is bit 0), and the mask last sent.
    leds: u8,
    sent_leds: Option<u8>,
    brightness: u8,
    /// Set once Ryu has been given a brightness to show; cleared on the G15
    /// family when the light button takes the backlight over.
    brightness_set: bool,
    /// The backlight as Ryu found it: the G15 family's keyboard and LCD
    /// levels, the G510's colour, or the G110's whole report.
    found_levels: Option<[u8; 2]>,
    found_color: Option<[u8; 3]>,
    found_g110: Option<[u8; G110_BACKLIGHT_LEN]>,
    /// The G510's deck light, before brightness.
    color: Option<[u8; 3]>,
    /// The backlight report last sent.
    sent_backlight: Option<Vec<u8>>,
    /// The G510's light button has its backlight off.
    backlight_off: bool,
}

impl G15 {
    /// Opens one of these keyboards and reads its backlight as it is. Only
    /// the five models `Kind::of` knows have this protocol; any other would
    /// be read as a G15.
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model).unwrap_or(Kind::G15);
        let mut deck = Self {
            kind,
            model,
            transport,
            held: vec![false; kind.keys().len()],
            init_sent: false,
            lcd: vec![0; g13::LCD_BYTES],
            sent_lcd: None,
            leds: 0,
            sent_leds: None,
            brightness: 100,
            brightness_set: false,
            found_levels: None,
            found_color: None,
            found_g110: None,
            color: None,
            sent_backlight: None,
            backlight_off: false,
        };
        match kind {
            Kind::G15 | Kind::G15V2 | Kind::G11 => deck.found_levels = deck.read_levels(),
            Kind::G510 => {
                deck.found_color = deck
                    .read_feature(G510_BACKLIGHT, 4)
                    .and_then(|report| Some([*report.get(1)?, *report.get(2)?, *report.get(3)?]));
            }
            Kind::G110 => {
                deck.found_g110 = deck
                    .read_feature(G110_BACKLIGHT, G110_BACKLIGHT_LEN)
                    .and_then(|report| report.try_into().ok());
            }
        }
        deck
    }

    fn info_image(&self) -> Option<(u8, KeyImage)> {
        self.model.info_key().zip(self.model.info_image)
    }

    /// Reads a feature report of `len` bytes, or `None` if it is refused or short.
    fn read_feature(&mut self, id: u8, len: usize) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; len];
        buf[0] = id;
        match self.transport.get_feature_report(&mut buf) {
            Ok(read) if read >= len && buf[0] == id => Some(buf),
            Ok(_) => None,
            Err(error) => {
                tracing::debug!(%error, "ryu-decks: could not read the {} backlight", self.model.name);
                None
            }
        }
    }

    /// The G15 family's keyboard and LCD backlight levels, from feature `02`.
    fn read_levels(&mut self) -> Option<[u8; 2]> {
        let report = self.read_feature(G15_FEATURE, 4)?;
        let keyboard = *report.get(1)?;
        let lcd = *report.get(2)?;
        // The level comes back as is; take a shifted one too.
        let lcd = if lcd > TOP_LEVEL { lcd >> 4 } else { lcd };
        Some([keyboard.min(TOP_LEVEL), lcd.min(TOP_LEVEL)])
    }

    /// Sends a feature report. None has been checked through hidapi on
    /// hardware, so a refusal is logged rather than failing the paint.
    fn send_feature(&mut self, report: &[u8], what: &str) {
        if let Err(error) = self.transport.send_feature_report(report) {
            tracing::warn!(%error, "ryu-decks: the {} did not take the {what} report", self.model.name);
        }
    }

    fn send_init(&mut self) {
        self.init_sent = true;
        let Some((report, feature)) = self.kind.init_report() else {
            return;
        };
        let sent = if feature {
            self.transport.send_feature_report(&report)
        } else {
            self.transport.write(&report)
        };
        if let Err(error) = sent {
            tracing::warn!(%error, "ryu-decks: the {} did not stop its G-keys typing", self.model.name);
        }
    }

    fn parse(&mut self, report: &[u8], events: &mut Vec<Input>) {
        let Some(&id) = report.first() else {
            return;
        };
        if self.kind == Kind::G510 && (id, report.len()) == (G510_LIGHT_STATE, G510_LIGHT_STATE_LEN)
        {
            let off = report
                .get(1)
                .is_some_and(|byte| byte & G510_BACKLIGHT_OFF != 0);
            if self.backlight_off && !off {
                // Colours sent while it was off were dropped.
                self.sent_backlight = None;
            }
            self.backlight_off = off;
            return;
        }
        if (id, report.len()) != self.kind.key_report() {
            return;
        }
        let light_key = self.kind.light_key();
        let mut light_pressed = false;
        for (index, key) in (0u8..).zip(self.kind.keys()) {
            let now = report
                .get(key.byte)
                .is_some_and(|byte| byte & key.mask != 0);
            let Some(was) = self.held.get_mut(usize::from(index)) else {
                continue;
            };
            if now != *was {
                *was = now;
                events.push(Input::Key {
                    index,
                    pressed: now,
                });
                light_pressed |= now && index == light_key;
            }
        }
        if light_pressed && self.kind.has_levels() {
            // The board stepped its own backlight: it now has the say until
            // Ryu is given a brightness again.
            self.brightness_set = false;
            if let Some(levels) = self.read_levels() {
                self.sent_backlight = Some(self.levels_reports(levels[0], levels[1]).concat());
            }
        }
    }

    /// The G15 family's reports for a keyboard and an LCD backlight level.
    fn levels_reports(&self, keyboard: u8, lcd: u8) -> Vec<Vec<u8>> {
        let mut reports = vec![vec![G15_FEATURE, G15_KEYBOARD_BACKLIGHT, keyboard, 0]];
        if self.kind.has_lcd() {
            reports.push(vec![G15_FEATURE, G15_LCD_BACKLIGHT, lcd << 4, 0]);
        }
        reports
    }

    fn scaled(&self, [r, g, b]: [u8; 3]) -> [u8; 3] {
        let scale = |channel: u8| {
            u8::try_from(u16::from(channel) * u16::from(self.brightness) / 100).unwrap_or(u8::MAX)
        };
        [scale(r), scale(g), scale(b)]
    }

    /// The backlight reports to send now, or `None` to leave it alone.
    fn backlight_reports(&self) -> Option<Vec<Vec<u8>>> {
        match self.kind {
            Kind::G15 | Kind::G15V2 | Kind::G11 => {
                let level = level_for(self.brightness);
                self.brightness_set
                    .then(|| self.levels_reports(level, level))
            }
            Kind::G510 => {
                if !self.brightness_set && self.color.is_none() {
                    // Nothing for Ryu to show: the colour it had goes back,
                    // but only once Ryu has changed it.
                    self.sent_backlight.as_ref()?;
                    let [r, g, b] = self.found_color?;
                    return Some(vec![vec![G510_BACKLIGHT, r, g, b]]);
                }
                let base = self.color.or(self.found_color)?;
                let [r, g, b] = self.scaled(base);
                Some(vec![vec![G510_BACKLIGHT, r, g, b]])
            }
            Kind::G110 => {
                if !self.brightness_set {
                    return None;
                }
                let mix = self.found_g110.map_or(G110_EVEN_MIX, |report| report[1]);
                let level = u8::try_from(u16::from(self.brightness) * 255 / 100).unwrap_or(u8::MAX);
                Some(vec![vec![G110_BACKLIGHT, mix, 0, 0, level]])
            }
        }
    }

    fn send_backlight(&mut self) {
        if self.backlight_off {
            return;
        }
        let Some(reports) = self.backlight_reports() else {
            return;
        };
        let joined = reports.concat();
        if self.sent_backlight.as_ref() == Some(&joined) {
            return;
        }
        for report in &reports {
            self.send_feature(report, "backlight");
        }
        self.sent_backlight = Some(joined);
    }
}

impl Driver for G15 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if !self.init_sent {
                // A read is now pending, so the device takes the report.
                self.send_init();
            }
            if read == 0 {
                break;
            }
            self.parse(buf.get(..read).unwrap_or_default(), &mut events);
        }
        // The G510's backlight may have just come back on.
        self.send_backlight();
        Ok(events)
    }

    /// The LCD takes the info key's face in one bit per pixel; M1 to MR take
    /// an on or off LED from the colour. Other keys have no light of their own.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some((info, format)) = self.info_image() {
            if key == info {
                self.lcd = g13::lcd_frame(face, &format)?;
                return Ok(());
            }
        }
        let Some(bit) = key
            .checked_sub(self.kind.first_mode_key())
            .filter(|bit| *bit < 4)
        else {
            return Ok(());
        };
        let [r, g, b] = face.map_or([0, 0, 0], |face| face.color);
        let mask = 1u8 << bit;
        if r.max(g).max(b) > ON_OFF_THRESHOLD {
            self.leds |= mask;
        } else {
            self.leds &= !mask;
        }
        Ok(())
    }

    /// Sends the LEDs, the backlight and the LCD, each only when it changed.
    fn flush(&mut self) -> DriverResult<()> {
        if self.sent_leds != Some(self.leds) {
            let report = self.kind.leds_report(self.leds);
            self.send_feature(&report, "mode LED");
            self.sent_leds = Some(self.leds);
        }
        self.send_backlight();
        if self.kind.has_lcd() && self.sent_lcd.as_ref() != Some(&self.lcd) {
            self.transport.write(&g13::lcd_report(&self.lcd))?;
            self.sent_lcd = Some(self.lcd.clone());
        }
        Ok(())
    }

    /// The G15 family steps both backlights to off, low or high; the G510
    /// scales its colour and the G110 sets its level.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.brightness_set = true;
        self.flush()
    }

    /// The G510's backlight colour. `None` gives it back the colour it had
    /// when Ryu opened it. Other models have no deck light.
    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        if self.kind == Kind::G510 {
            self.color = color;
        }
        Ok(())
    }
}

impl Drop for G15 {
    /// Turns the LEDs off, blanks the LCD and puts back the backlight Ryu
    /// found, if it changed it.
    fn drop(&mut self) {
        let leds = self.kind.leds_report(0);
        let _ = self.transport.send_feature_report(&leds);
        if self.sent_backlight.is_some() {
            let restore = match self.kind {
                Kind::G15 | Kind::G15V2 | Kind::G11 => self
                    .found_levels
                    .map(|[keyboard, lcd]| self.levels_reports(keyboard, lcd)),
                Kind::G510 => self
                    .found_color
                    .map(|[r, g, b]| vec![vec![G510_BACKLIGHT, r, g, b]]),
                Kind::G110 => self.found_g110.map(|report| vec![report.to_vec()]),
            };
            for report in restore.unwrap_or_default() {
                let _ = self.transport.send_feature_report(&report);
            }
        }
        if self.kind.has_lcd() {
            let _ = self.transport.write(&g13::lcd_report(&[]));
        }
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn open(id: &str) -> (G15, Recorder) {
        open_with(id, &[])
    }

    /// Opens a keyboard that answers its first feature reads with `replies`.
    fn open_with(id: &str, replies: &[&[u8]]) -> (G15, Recorder) {
        let recorder = Recorder::default();
        for reply in replies {
            recorder.queue_feature_reply(reply.to_vec());
        }
        (
            G15::new(by_id(id).unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    fn color(rgb: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color: rgb,
            image: None,
        }
    }

    /// A G15 / G11 key report with each `(byte, mask)` set.
    fn g15_report(bits: &[(usize, u8)]) -> Vec<u8> {
        let mut report = vec![0x02, 0, 0, 0, 0, 0, 0, 0, 0];
        for (byte, mask) in bits {
            report[*byte] |= mask;
        }
        report
    }

    #[test]
    fn every_layout_matches_its_model_and_no_two_keys_share_a_bit_or_cell() {
        for id in [G15_ID, G15_V2_ID, G11_ID, G510_ID, G110_ID] {
            let model = by_id(id).unwrap();
            let kind = Kind::of(model).unwrap();
            assert_eq!(kind.keys().len(), usize::from(model.keys), "{id}");
            let bits: std::collections::HashSet<_> =
                kind.keys().iter().map(|key| (key.byte, key.mask)).collect();
            assert_eq!(bits.len(), kind.keys().len(), "{id} reuses a bit");
            let cells = key_cells(model).unwrap();
            let distinct: std::collections::HashSet<_> = cells.iter().collect();
            assert_eq!(distinct.len(), cells.len(), "{id} reuses a cell");
            let grid = u16::from(model.rows) * u16::from(model.cols);
            assert!(cells.iter().all(|cell| *cell < grid), "{id}");
            let names = key_names(model).unwrap();
            let first = usize::from(kind.first_mode_key());
            assert_eq!(&names[first..first + 4], ["M1", "M2", "M3", "MR"], "{id}");
            assert_eq!(names[usize::from(kind.light_key())], "Light", "{id}");
            assert_eq!(model.info_key().is_some(), kind.has_lcd(), "{id}");
            assert!(kind.keys().iter().all(|key| key.byte < kind.key_report().1));
        }
    }

    #[test]
    fn g15_keys_read_the_scrambled_bitmap() {
        let (mut g15, recorder) = open(G15_ID);
        // G1, G7, G13, G18, M1, MR, L1, L2.
        recorder.queue_read(g15_report(&[
            (1, 0x01),
            (2, 0x01),
            (1, 0x04),
            (8, 0x40),
            (6, 0x01),
            (7, 0x40),
            (8, 0x80),
            (2, 0x80),
        ]));
        // Only G7 stays down.
        recorder.queue_read(g15_report(&[(2, 0x01)]));
        assert_eq!(
            g15.poll(0).unwrap(),
            vec![
                key(0, true),
                key(6, true),
                key(12, true),
                key(17, true),
                key(18, true),
                key(21, true),
                key(22, true),
                key(23, true),
                key(0, false),
                key(12, false),
                key(17, false),
                key(18, false),
                key(21, false),
                key(22, false),
                key(23, false),
            ]
        );
    }

    #[test]
    fn the_second_report_and_other_lengths_are_dropped() {
        let (mut g15, recorder) = open(G15_ID);
        recorder.queue_read(vec![0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        recorder.queue_read(vec![0x02, 0xff, 0xff, 0xff, 0xff]);
        recorder.queue_read(vec![0x02]);
        assert!(g15.poll(0).unwrap().is_empty());
    }

    #[test]
    fn g15_and_g11_stop_the_g_keys_typing_after_the_first_read() {
        let (mut g15, recorder) = open(G15_ID);
        assert!(recorder.writes().is_empty());
        g15.poll(0).unwrap();
        g15.poll(0).unwrap();
        let mut init = vec![0u8; 19];
        init[0] = 0x02;
        assert_eq!(recorder.writes(), vec![init.clone()]);
        let (mut g11, recorder) = open(G11_ID);
        g11.poll(0).unwrap();
        assert_eq!(recorder.writes(), vec![init]);
        let (mut v2, recorder) = open(G15_V2_ID);
        v2.poll(0).unwrap();
        assert_eq!(recorder.writes(), vec![vec![0x02, 0, 0, 0, 0, 0, 0]]);
        let (mut g510, recorder) = open(G510_ID);
        g510.poll(0).unwrap();
        let mut feature = vec![0u8; 19];
        feature[0] = 0x01;
        assert_eq!(recorder.features(), vec![feature]);
        let (mut g110, recorder) = open(G110_ID);
        g110.poll(0).unwrap();
        assert!(recorder.writes().is_empty() && recorder.features().is_empty());
    }

    #[test]
    fn g11_ignores_the_lcd_button_bits() {
        let (mut g11, recorder) = open(G11_ID);
        recorder.queue_read(g15_report(&[(8, 0x80), (2, 0x80), (1, 0x80)]));
        // L1 and L2 are not G11 keys; the light button is key 22.
        assert_eq!(g11.poll(0).unwrap(), vec![key(22, true)]);
    }

    #[test]
    fn g15_v2_keys() {
        let (mut v2, recorder) = open(G15_V2_ID);
        // G1, G6, M1, M2, M3, MR, L1, L5, light.
        recorder.queue_read(vec![0x02, 0xe1, 0xf1, 0, 0]);
        let events = v2.poll(0).unwrap();
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(5, true),
                key(6, true),
                key(7, true),
                key(8, true),
                key(9, true),
                key(10, true),
                key(14, true),
                key(15, true),
            ]
        );
    }

    #[test]
    fn g510_keys_and_the_game_mode_switch() {
        let (mut g510, recorder) = open(G510_ID);
        // G1, G9, G18, game mode on, M1, MR, L1, L5, headphone and mic mute.
        recorder.queue_read(vec![0x03, 0x01, 0x01, 0x02 | 0x04 | 0x10 | 0x80, 0x71]);
        // Only game mode stays on.
        recorder.queue_read(vec![0x03, 0, 0, 0x04, 0]);
        assert_eq!(
            g510.poll(0).unwrap(),
            vec![
                key(0, true),
                key(8, true),
                key(17, true),
                key(18, true),
                key(21, true),
                key(22, true),
                key(26, true),
                key(28, true),
                key(29, true),
                key(30, true),
                key(0, false),
                key(8, false),
                key(17, false),
                key(18, false),
                key(21, false),
                key(22, false),
                key(26, false),
                key(29, false),
                key(30, false),
            ]
        );
    }

    #[test]
    fn g110_keys() {
        let (mut g110, recorder) = open(G110_ID);
        // G1, G8, G12, M1, MR, light.
        recorder.queue_read(vec![0x02, 0x81, 0x98, 0x01]);
        assert_eq!(
            g110.poll(0).unwrap(),
            vec![
                key(0, true),
                key(7, true),
                key(11, true),
                key(12, true),
                key(15, true),
                key(16, true),
            ]
        );
    }

    #[test]
    fn g15_mode_leds_are_an_inverted_mask() {
        let (mut g15, recorder) = open(G15_ID);
        g15.flush().unwrap();
        g15.set_face(18, Some(&color([255, 0, 0]))).unwrap();
        g15.set_face(21, Some(&color([0, 200, 0]))).unwrap();
        g15.set_face(19, Some(&color([5, 5, 5]))).unwrap();
        // A G-key has no LED.
        g15.set_face(0, Some(&color([255, 255, 255]))).unwrap();
        g15.flush().unwrap();
        assert_eq!(
            recorder.features(),
            vec![vec![0x02, 0x04, 0xff, 0], vec![0x02, 0x04, 0xf6, 0]]
        );
    }

    #[test]
    fn g510_mode_leds_count_down_from_the_top_bit() {
        let (mut g510, recorder) = open(G510_ID);
        g510.set_face(18, Some(&color([255, 0, 0]))).unwrap();
        g510.set_face(21, Some(&color([255, 0, 0]))).unwrap();
        g510.flush().unwrap();
        assert_eq!(recorder.features()[0], vec![0x04, 0x90]);
    }

    #[test]
    fn g15_brightness_steps_both_backlights() {
        let (mut g15, recorder) = open_with(G15_ID, &[&[0x02, 2, 1, 0x0f]]);
        assert_eq!(g15.found_levels, Some([2, 1]));
        g15.flush().unwrap();
        // No brightness yet: the backlights are left alone.
        assert_eq!(recorder.features().len(), 1);
        g15.set_brightness(50).unwrap();
        g15.set_brightness(60).unwrap();
        g15.set_brightness(0).unwrap();
        g15.set_brightness(100).unwrap();
        assert_eq!(
            recorder.features()[1..],
            [
                vec![0x02, 0x01, 1, 0],
                vec![0x02, 0x02, 0x10, 0],
                vec![0x02, 0x01, 0, 0],
                vec![0x02, 0x02, 0x00, 0],
                vec![0x02, 0x01, 2, 0],
                vec![0x02, 0x02, 0x20, 0],
            ]
        );
        let (mut g11, recorder) = open(G11_ID);
        g11.set_brightness(30).unwrap();
        assert_eq!(recorder.features()[1..], [vec![0x02, 0x01, 1, 0]]);
    }

    #[test]
    fn the_g15_light_button_hands_the_backlight_to_the_board() {
        let (mut g15, recorder) = open_with(G15_ID, &[&[0x02, 2, 2, 0x0f]]);
        g15.set_brightness(100).unwrap();
        let sent = recorder.features().len();
        // The button stepped the keyboard backlight down; the board says so.
        recorder.queue_feature_reply(vec![0x02, 1, 2, 0x0f]);
        recorder.queue_read(g15_report(&[(1, 0x80)]));
        assert_eq!(g15.poll(0).unwrap(), vec![key(27, true)]);
        g15.flush().unwrap();
        // Nothing is sent back over it until a brightness is set again.
        let features = recorder.features();
        assert_eq!(features.len(), sent);
        g15.set_brightness(100).unwrap();
        assert_eq!(
            recorder.features()[sent..],
            [vec![0x02, 0x01, 2, 0], vec![0x02, 0x02, 0x20, 0]]
        );
    }

    #[test]
    fn g510_colour_is_scaled_and_resent_when_its_light_comes_back() {
        let (mut g510, recorder) = open_with(G510_ID, &[&[0x05, 10, 20, 30]]);
        g510.flush().unwrap();
        // The found colour is not sent back unasked.
        assert_eq!(recorder.features().len(), 1);
        g510.set_deck_light(Some([200, 100, 50])).unwrap();
        g510.set_brightness(50).unwrap();
        assert_eq!(recorder.features()[1], vec![0x05, 100, 50, 25]);
        // The light button turns the backlight off; colours wait.
        recorder.queue_read(vec![0x04, 0x04]);
        g510.poll(0).unwrap();
        g510.set_deck_light(Some([0, 0, 255])).unwrap();
        g510.flush().unwrap();
        let before = recorder.features().len();
        // Back on: the colour goes out again.
        recorder.queue_read(vec![0x04, 0x00]);
        g510.poll(0).unwrap();
        assert_eq!(recorder.features()[before..], [vec![0x05, 0, 0, 127]]);
        // No colour gives back the one it had.
        g510.set_deck_light(None).unwrap();
        g510.flush().unwrap();
        assert_eq!(recorder.features().last().unwrap(), &vec![0x05, 5, 10, 15]);
    }

    #[test]
    fn g510_cleared_light_gives_back_its_colour_without_a_brightness() {
        let (mut g510, recorder) = open_with(G510_ID, &[&[0x05, 10, 20, 30]]);
        g510.set_deck_light(Some([255, 0, 0])).unwrap();
        g510.flush().unwrap();
        assert_eq!(recorder.features().last().unwrap(), &vec![0x05, 255, 0, 0]);
        // No brightness was ever set: clearing the light still puts back the
        // colour the keyboard had, at once rather than on disconnect.
        g510.set_deck_light(None).unwrap();
        g510.flush().unwrap();
        assert_eq!(recorder.features().last().unwrap(), &vec![0x05, 10, 20, 30]);
        let sent = recorder.features().len();
        g510.flush().unwrap();
        assert_eq!(recorder.features().len(), sent);
    }

    #[test]
    fn g110_brightness_keeps_its_mix() {
        let (mut g110, recorder) = open_with(G110_ID, &[&[0x07, 0x30, 0, 0, 0xff]]);
        g110.set_brightness(40).unwrap();
        assert_eq!(
            recorder.features(),
            vec![vec![0x02, 0x04, 0xff, 0], vec![0x07, 0x30, 0, 0, 102]]
        );
        // A G110 whose backlight cannot be read gets an even mix.
        let (mut g110, recorder) = open(G110_ID);
        g110.set_brightness(100).unwrap();
        assert_eq!(recorder.features()[1], vec![0x07, 0x80, 0, 0, 0xff]);
    }

    #[test]
    fn the_info_key_draws_the_lcd_like_the_g13() {
        let (mut g15, recorder) = open(G15_ID);
        assert_eq!(g15.model.info_key(), Some(28));
        let mut picture = RgbaImage::from_pixel(160, 43, Rgba([0, 0, 0, 255]));
        for (x, y) in [(0, 0), (5, 9), (159, 42)] {
            picture.put_pixel(x, y, Rgba([255, 255, 255, 255]));
        }
        let face = Face {
            label: "LCD".to_owned(),
            color: [0, 0, 0],
            image: Some(picture),
        };
        g15.set_face(28, Some(&face)).unwrap();
        g15.flush().unwrap();
        let report = &recorder.writes()[0];
        assert_eq!(report.len(), 992);
        assert_eq!(report[0], 0x03);
        assert!(report[1..32].iter().all(|byte| *byte == 0));
        let lcd = &report[32..];
        assert_eq!(lcd[0], 0x01);
        assert_eq!(lcd[5 + 160], 0x02);
        assert_eq!(lcd[159 + 5 * 160], 0x04);
        assert_eq!(lcd.iter().filter(|byte| **byte != 0).count(), 3);
        // The G11 and G110 have no LCD to write.
        let (mut g11, recorder) = open(G11_ID);
        g11.flush().unwrap();
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn dropping_turns_the_leds_off_restores_the_backlight_and_blanks_the_lcd() {
        let (mut g15, recorder) = open_with(G15_ID, &[&[0x02, 1, 2, 0x0f]]);
        g15.set_brightness(0).unwrap();
        drop(g15);
        let features = recorder.features();
        assert_eq!(
            features[features.len() - 3..],
            [
                vec![0x02, 0x04, 0xff, 0],
                vec![0x02, 0x01, 1, 0],
                vec![0x02, 0x02, 0x20, 0],
            ]
        );
        let writes = recorder.writes();
        let last = writes.last().unwrap();
        assert_eq!(last.len(), 992);
        assert!(last[1..].iter().all(|byte| *byte == 0));
        // A board whose backlight Ryu never touched is left as it is.
        let (g510, recorder) = open_with(G510_ID, &[&[0x05, 1, 2, 3]]);
        drop(g510);
        assert_eq!(recorder.features(), vec![vec![0x04, 0x00]]);
    }
}
