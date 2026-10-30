//! Mackie's MCU Pro (the Mackie Control Universal Pro) and its XT Pro
//! extender, over the MCU Pro's class-compliant USB MIDI ([`super::midi`]),
//! in Mackie Control mode.
//!
//! The MCU Pro lists four MIDI ports each way, all named after its product
//! string (`MCU Pro USB v3.1`; the version may change, so only `MCU PRO
//! USB` is matched, in any case). The first is the MCU Pro itself: on
//! Windows the plain name, on Linux ALSA port 0. The other three are the DIN
//! MIDI jacks an XT Pro (or any other MIDI gear) hangs off: on Windows they
//! are wrapped `MIDIIN<n> (MCU Pro USB v3.1)` (Bitwig numbers them from 1,
//! Cubase from 2, so any wrapped port is an extender candidate), on Linux
//! ALSA ports 1-3. Each extender port is its own deck. No reference gives
//! the XT Pro's own USB id or port name, so a stand-alone XT Pro on USB is
//! not found. On Windows and Linux the three extender ports are listed as
//! XT Pro decks whether or not anything is chained; each only sends the
//! device query until an XT Pro answers. The macOS names of ports 2-4 are
//! not documented, so there every same-named port becomes an MCU Pro deck
//! that queries both the MCU Pro's and the XT Pro's ids: a chained XT Pro
//! that answers drives that deck with the MCU Pro's layout (its strip keys,
//! fader touches, V-Pots and faders), and only its own lights are written.
//!
//! Opening is silent but for Mackie's device query (`F0 00 00 66 <id> 00
//! F7`, id `14` on the MCU Pro, `15` on the XT Pro, and the Logic Control
//! ids `10`/`11` for a unit booted in that emulation), sent again from the
//! poll loop every two seconds until the deck answers `F0 00 00 66 <id> 01
//! ... F7`. In Mackie mode that answer makes the deck live; in Logic Control
//! mode it carries a challenge, answered (`02`, the seven serial bytes and
//! four response bytes) before the deck confirms (`03`, then Ryu sends `13
//! 00`) or refuses (`04`: Ryu stays silent and logs a hint); with no
//! confirmation two seconds later Ryu logs it once and queries again. A
//! ready answer from a live deck (a unit rebooted behind an open port)
//! darkens and greets it again and resends every light. The id of the
//! answer goes in every later SysEx header. On Windows and Linux the MCU
//! Pro's own port may also go live on any note, Control Change or pitch
//! bend; an extender port (a keyboard's pitch wheel would fake it) and any
//! macOS port wait for the SysEx answer, so a synth on the DIN jacks never
//! gets a note from Ryu.
//!
//! In, on MIDI channel 1: every button and fader touch is a Note On,
//! pressed at a velocity over 64 and released at 64 or under (a Note Off
//! releases too). On the MCU Pro a key's index is its note, `0x00`-`0x70`,
//! eight to a row: Rec, Solo, Mute, Select and V-Pot push by strip, the
//! globals in Mackie note order (`0x28` Track to `0x67` User Switch B), the
//! fader touches (row 14) and the master fader touch (row 15). On the XT
//! Pro keys 1-40 are notes `0x00`-`0x27` and keys 41-48 the fader touches
//! `0x68`-`0x6F`. The V-Pots are `CC 0x10+s` (dials 1-8) and the MCU Pro's
//! jog wheel `CC 0x3C` (dial 9), sign-magnitude relative: bit 6 set turns
//! anticlockwise, bits 0-5 count the steps (0 counts as 1; the jog wheel
//! accelerates, at most 15 a message). The motor faders are pitch bend on
//! channels 1-8 (and the MCU Pro's master on 9): the first value of each
//! only seeds it, then every 128 of its 14 bits turn its dial a step (dials
//! 10-18 on the MCU Pro, 9-16 on the XT Pro). The expression pedal (`CC
//! 0x2E`) is not read.
//!
//! Out, once live, on channel 1: a button's LED is `90 <note> 7F` on and
//! `90 <note> 00` off (V-Pot pushes, foot switches and fader touches have
//! none); a V-Pot's LED ring (the dial's light) is `B0 30+s 2B` (every LED
//! in wrap mode) or `00`. Going live turns every LED, the MCU Pro's SMPTE,
//! Beats and Rude Solo lamps (`90 71-73 00`) and the rings off, writes `Ryu`
//! on the 2 x 56 LCD (`F0 00 00 66 <id> 12 00 <112 chars> F7`) and on the
//! MCU Pro blanks the ten timecode and two assignment digits (`B0 40-4B
//! 20`). Every write waits in a queue drained by poll and flush, a
//! millisecond apart and five after a SysEx (a guess; no wait in tests),
//! and Ryu never sleeps in the poll loop. Letting go turns the lights off
//! and blanks the displays at once. Ryu never sends pitch bend (the faders
//! have motors), a reset, a fader recalibration, touch sensitivity, the
//! backlight or meter-mode SysEx, or HUI's heartbeat. There is no
//! brightness command and no keep-alive.
//!
//! Facts from Ardour's Mackie Control surface and its MCU Pro device files
//! (C++, GPL-2.0-or-later), DrivenByMoss's Mackie Control support (Java,
//! LGPL-3.0), Ableton Live's Mackie Control scripts (decompiled, facts
//! only), and the Cubase MIDI Remote (MIT) and Bitwig (MIT) MCU Pro
//! definitions; no code was copied. Untested on hardware.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::icon_qcon::{relative, Fader, NAMES};
use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const MCU_PRO_ID: &str = "mackie-mcu-pro";
pub const XT_PRO_ID: &str = "mackie-xt-pro";

/// Mackie Designs' USB vendor id and the MCU Pro's product id (from
/// Ableton Live's script), shown only: the deck is found by port name.
pub const MCU_PRO_PRODUCT_ID: u16 = 0x0006;

pub const MCU_ROWS: u8 = 15;
pub const MCU_COLS: u8 = 8;
/// Notes `0x00` to `0x70`, each its own key.
pub const MCU_KEYS: u8 = 113;
/// Eight V-Pots, the jog wheel, eight faders and the master fader.
pub const MCU_DIALS: u8 = 18;
pub const XT_ROWS: u8 = 6;
pub const XT_COLS: u8 = 8;
/// Rec, Solo, Mute, Select, V-Pot push and fader touch for eight strips.
pub const XT_KEYS: u8 = 48;
/// Eight V-Pots and eight faders.
pub const XT_DIALS: u8 = 16;
/// The V-Pots' LED rings (dials 1-8); the jog wheel and faders have none.
pub const RINGS: u8 = 8;

const STRIPS: u8 = 8;
const XT_LAST_STRIP_NOTE: u8 = 0x27;
const XT_FIRST_TOUCH_KEY: u8 = 40;
const FADER_TOUCH: u8 = 0x68;
const MASTER_TOUCH: u8 = 0x70;
const LAST_STRIP_LIGHT: u8 = 0x1f;
/// The MCU Pro's global buttons with a light, Track to Scrub.
const GLOBAL_LIGHTS: std::ops::RangeInclusive<u8> = 0x28..=0x65;
/// SMPTE, Beats and Rude Solo.
const LAMPS: std::ops::RangeInclusive<u8> = 0x71..=0x73;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const PITCH_BEND: u8 = 0xe0;
const CHANNEL_PRESSURE: u8 = 0xd0;
const VPOT_CC: u8 = 0x10;
const JOG_CC: u8 = 0x3c;
const EXTERNAL_CC: u8 = 0x2e;
/// HUI's zone and port select: a unit booted in HUI mode.
const HUI_ZONE_CC: u8 = 0x0f;
const HUI_PORT_CC: u8 = 0x2f;
const RING_CC: u8 = 0x30;
/// The ten timecode digits (`0x40` the rightmost) and the two assignment
/// digits (`0x4A` right, `0x4B` left).
const SEGMENT_CCS: std::ops::RangeInclusive<u8> = 0x40..=0x4b;
const BLANK: u8 = 0x20;
const PRESS_OVER: u8 = 64;

const LED_OFF: u8 = 0;
const LED_ON: u8 = 0x7f;
/// Wrap mode, all eleven LEDs.
const RING_ALL: u8 = 0x2b;
const RING_OFF: u8 = 0;

const SYSEX_START: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
const MACKIE: [u8; 3] = [0x00, 0x00, 0x66];
const QUERY: u8 = 0x00;
const READY: u8 = 0x01;
const CHALLENGE_REPLY: u8 = 0x02;
const CONFIRMED: u8 = 0x03;
const DENIED: u8 = 0x04;
const VERSION_REQUEST: u8 = 0x13;
const LCD: u8 = 0x12;
const MCU_DEVICE: u8 = 0x14;
const XT_DEVICE: u8 = 0x15;
const LC_DEVICE: u8 = 0x10;
const LC_XT_DEVICE: u8 = 0x11;
/// A Logic Control challenge: header, command, seven serial bytes, four
/// challenge bytes, end.
const CHALLENGE_LEN: usize = 18;
const SERIAL: std::ops::Range<usize> = 6..13;
const CHALLENGE: std::ops::Range<usize> = 13..17;
/// Both LCD lines, 56 characters each, from offset 0.
const LCD_CHARS: usize = 112;
const GREETING: &[u8] = b"Ryu";

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Fader units (of 16384) per dial step: 128 from end to end.
const FADER_STEP: i32 = 128;
const MAX_FADERS: usize = 9;

/// The gap after a short message and after a SysEx, a guess (none in
/// tests).
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
/// How often the device query goes out until the deck answers (every poll
/// in tests).
const QUERY_EVERY: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(2)
};
const MAX_CREDIT: Duration = Duration::from_millis(10);
const READ_LEN: usize = 256;
const MAX_MESSAGES_PER_POLL: usize = 64;
/// Whether port names say which port is the MCU Pro's own (Windows and
/// Linux do; macOS's names of the extra ports are not documented).
const NAMED_PORTS: bool = !cfg!(target_os = "macos");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Mcu,
    Xt,
}

impl Kind {
    fn of(model: &DeviceModel) -> Option<Self> {
        match (model.protocol, model.id) {
            (Protocol::MackieMcuProMidi, MCU_PRO_ID) => Some(Self::Mcu),
            (Protocol::MackieXtProMidi, XT_PRO_ID) => Some(Self::Xt),
            _ => None,
        }
    }

    const fn keys(self) -> u8 {
        match self {
            Self::Mcu => MCU_KEYS,
            Self::Xt => XT_KEYS,
        }
    }

    /// Its own SysEx id, then its Logic Control emulation's.
    const fn devices(self) -> [u8; 2] {
        match self {
            Self::Mcu => [MCU_DEVICE, LC_DEVICE],
            Self::Xt => [XT_DEVICE, LC_XT_DEVICE],
        }
    }

    fn key(self, note: u8) -> Option<u8> {
        match self {
            Self::Mcu => (note <= MASTER_TOUCH).then_some(note),
            Self::Xt => match note {
                0..=XT_LAST_STRIP_NOTE => Some(note),
                _ if (FADER_TOUCH..FADER_TOUCH + STRIPS).contains(&note) => {
                    Some(note - FADER_TOUCH + XT_FIRST_TOUCH_KEY)
                }
                _ => None,
            },
        }
    }

    fn note(self, key: u8) -> Option<u8> {
        match self {
            Self::Mcu => (key < MCU_KEYS).then_some(key),
            Self::Xt => match key {
                0..=XT_LAST_STRIP_NOTE => Some(key),
                XT_FIRST_TOUCH_KEY..XT_KEYS => Some(key - XT_FIRST_TOUCH_KEY + FADER_TOUCH),
                _ => None,
            },
        }
    }

    fn has_light(self, key: u8) -> bool {
        key <= LAST_STRIP_LIGHT || (self == Self::Mcu && GLOBAL_LIGHTS.contains(&key))
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
        match self {
            Self::Mcu => 9,
            Self::Xt => 8,
        }
    }

    const fn faders(self) -> u8 {
        match self {
            Self::Mcu => 9,
            Self::Xt => 8,
        }
    }
}

/// Whether a normalised port name is this model's: the uppercased name
/// holds `MCU PRO USB`; the MCU Pro is the unwrapped name on ALSA port 0
/// (or with no ALSA number), the XT Pro any Windows-wrapped port or ALSA
/// ports 1-3.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    let Some(kind) = Kind::of(model) else {
        return false;
    };
    if !name.base.to_ascii_uppercase().contains("MCU PRO USB") {
        return false;
    }
    match kind {
        Kind::Mcu => name.interface.is_none() && matches!(name.alsa_port, None | Some(0)),
        Kind::Xt => matches!(name.interface, Some(1..=4)) || matches!(name.alsa_port, Some(1..=3)),
    }
}

/// The Mackie function of each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    Kind::of(model).map(Kind::names)
}

/// The keys with a light: Rec, Solo, Mute, Select and the MCU Pro's global
/// buttons from Track to Scrub.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    Kind::of(model).map(Kind::lit_keys)
}

fn is_lit(color: Option<[u8; 3]>) -> bool {
    color.is_some_and(|color| color.iter().any(|channel| *channel >= DARK))
}

/// The four response bytes to a Logic Control challenge, computed as C
/// does on promoted bytes: wrapping 32-bit arithmetic, a shift past the
/// width giving zero, the low seven bits kept.
fn challenge_response(challenge: [u8; 4]) -> [u8; 4] {
    let [l0, l1, l2, l3] = challenge.map(i32::from);
    let shift = |value: i32, by: i32| {
        u32::try_from(by)
            .ok()
            .and_then(|by| value.checked_shr(by))
            .unwrap_or(0)
    };
    let shl = |value: i32, by: u32| value.checked_shl(by).unwrap_or(0);
    let r0 = l0.wrapping_add(l1 ^ 0x0a).wrapping_sub(l3);
    let r1 = shift(l2, l3) ^ l0.wrapping_add(l3);
    let r2 = l3.wrapping_sub(shl(l2, 2)) ^ (l0 | l1);
    let r3 = l1.wrapping_sub(l2).wrapping_add(0xf0 ^ shl(l3, 4));
    [r0, r1, r2, r3].map(|value| u8::try_from(value & 0x7f).unwrap_or(0))
}

fn sysex(device: u8, body: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(body.len() + 6);
    message.push(SYSEX_START);
    message.extend_from_slice(&MACKIE);
    message.push(device);
    message.extend_from_slice(body);
    message.push(SYSEX_END);
    message
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Querying, writing nothing else.
    Waiting,
    /// A Logic Control challenge answered; waiting for the confirmation.
    Challenged,
    Live,
    /// The deck refused the handshake: nothing more is written.
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Led(u8),
    Ring(u8),
    Other,
}

#[derive(Debug)]
struct Out {
    slot: Slot,
    bytes: Vec<u8>,
}

pub struct MackieMcu {
    kind: Kind,
    transport: Box<dyn Transport>,
    phase: Phase,
    /// The SysEx id the deck answered with (its own until then).
    device: u8,
    /// When the device query next goes out while waiting.
    query_at: Option<Instant>,
    query_every: Duration,
    outbox: VecDeque<Out>,
    ready_at: Instant,
    short_gap: Duration,
    sysex_gap: Duration,
    leds: Vec<bool>,
    queued: Vec<bool>,
    held: Vec<bool>,
    rings: [bool; RINGS as usize],
    queued_rings: [bool; RINGS as usize],
    faders: [Fader; MAX_FADERS],
    warned: bool,
    /// Whether a Logic Control confirmation that never came was logged.
    unconfirmed_warned: bool,
    /// Whether port names tell the MCU Pro's port from the extenders'.
    named_ports: bool,
    /// The unit that answered: an XT Pro may answer an MCU Pro deck on a
    /// macOS port.
    unit: Kind,
    parser: Parser,
}

mod deck;
#[cfg(test)]
mod tests;
