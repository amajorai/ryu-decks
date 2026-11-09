//! Akai's MPK mini IV and MPK mini Plus II, through their `DAW Port`, over
//! class-compliant USB MIDI ([`super::midi`]).
//!
//! Both decks list two MIDI ports each way: the `MIDI Port` (the keybed, which
//! Ryu never opens) and the `DAW Port`, which carries the pads, buttons, knobs
//! and main encoder as a DAW sees them and takes Akai SysEx for the colour
//! screen and the lights. [`port_matches`] takes `MPK mini IV` or `MPK mini
//! Plus II` (any case; never the MPK mini 3, the first MPK mini Plus or a Plus
//! III) when the name says `DAW`, never `MIDI Po`; or, on Windows, the second
//! interface (`MIDIIN2 (MPK mini IV)`); or, under ALSA, `<model> MIDI 2`, or
//! ALSA port 1 when the name says neither `DAW` nor ` MIDI 1`. Older Linux
//! kernels cut the Plus II's two ports to the same 31 characters, so only the
//! ALSA port number tells them apart there. That the DAW port is the second
//! jack (ALSA port 1) is inferred from Windows' numbering.
//!
//! Every Akai command Ryu sends is `F0 47 00 <pid> <cmd> <lenHi> <lenLo>
//! <payload> F7`, with the deck's SysEx product id `5D` (mini IV) or `03`
//! (Plus II). On open Ryu sends the universal device inquiry (`F0 7E 7F 06 01
//! F7`). When the deck's identity reply (`F0 7E <ch> 06 02 47 <pid> 00 19 ...`)
//! names this model, or after [`FALLBACK`] without one (Ryu's own design), it
//! queues, in order: the screen to the host (`1C [02]`), pad mode 1 (`2A
//! [01]`), `2B []` and `3A []` (sent as Bitwig sends them; their meaning is
//! not documented), a request for the DAW/plugin mode (`2D []`), pad notes off
//! the keybed port (`2E [01]`, inferred from Ableton's goodbye message), three
//! lines of fixed text (`Ryu`, the model name, `Connected`), then every light.
//! Bitwig's tempo text (`49`, with the Plus II's product id hard-coded) is not
//! sent. Writes drain from poll and flush, so the deck never makes the shared
//! poll loop wait.
//!
//! The deck reports `F0 47 7F <pid> 2D 00 01 <mode> F7`: mode 0 is DAW mode,
//! anything else plugin mode. Ryu assumes DAW mode until told otherwise. In
//! plugin mode input is ignored, held keys are released, no light is sent and
//! the screen keep-alive stops; going back to DAW mode sends every light
//! again. While in DAW mode the screen-owner command is sent again every
//! [`KEEP_ALIVE`], when the deck reports `19` with no payload, and after each
//! `2D` report. Reports with device id `00` (NKS messages) are ignored; that
//! is Ryu's choice, not Bitwig's.
//!
//! In, under DAW mode (channels from 0):
//!
//! - the eight pads are notes `0x24-0x33` on channel 9 (bank A `24-2B`, bank B
//!   `2C-33`, switched by the deck's own bank buttons; which bank the deck
//!   labels A is not confirmed): within a bank, `base+0..3` is the lower row
//!   and `base+4..7` the upper row. Ryu shows the first bank's upper row as
//!   keys 0-3 and its lower row as keys 4-7, the second bank's as 8-11 and
//!   12-15. A Note On above velocity 0 presses; a Note Off or velocity 0
//!   releases; poly aftertouch is ignored;
//! - on channel 0, pressed above 0: Shift `CC 11` (key 16), - `CC 50`, +
//!   `CC 51`, Tap Tempo / Note Repeat `CC 52`, Undo `CC 49`, Loop `CC 4A`,
//!   Play `CC 4C`, Record `CC 4D` and Overdub `CC 4E` (keys 17-24, hex);
//! - the eight knobs `CC 18-1F` (dials 0-7, top row first) are relative two's
//!   complement: 1-63 turn up, 64-127 turn down by `128 - value` (64 counts as
//!   -64, as Bitwig reads it);
//! - the main encoder `CC 0E` (dial 8) is one step a click: `01` up, `7F`
//!   down, anything else ignored; its push `CC 0D` presses dial 8.
//!
//! Bank A/B (`CC 0C`), Shift with a bank button (`CC 53`), BPM (`CC 78`), the
//! keybed, pitch bend, the mod wheel and other channels are ignored.
//!
//! Out: each pad's light is a Note On on the brightness channel (`0x90 |
//! state`) to its note, with the velocity choosing a colour from the deck's
//! palette (the nearest, weighted as Bitwig weighs it); each button but Shift
//! has a single-colour light, a Note On to its CC number with `7F` lit (any
//! colour but black or near black) or `00` dark. There is no global brightness
//! command: the brightness picks the state channel (0, 1, 2, 3, 4 or 6 for
//! 10, 25, 50, 65, 75 or 100%; channel 5 is never sent), and 0% turns every
//! light off (`96 <note> 00`). Only changed lights are sent.
//!
//! When Ryu lets go it turns every light it may have lit off, hands the screen
//! back to the deck (`1C [00]`) and puts pad notes back on the keybed port
//! (`2E [00]`), unpaced.
//!
//! Facts from Bitwig's MPK mini IV / MPK mini Plus II extension
//! (bitwig/bitwig-extensions, MIT; read for facts only) and its user docs,
//! cross-checked against Ableton's MPK_mini_IV Remote Script as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for the
//! USB id, identity bytes and colours only), and Linux's USB MIDI driver for
//! ALSA port names; no code was copied. The bank labels, the start-up
//! commands `2B` and `3A`, the meaning of `2E`, the line colour index and the
//! DAW port's ALSA number are inferred. Untested on hardware.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const MPK_MINI_IV_ID: &str = "akai-mpk-mini-iv";
pub const MPK_MINI_PLUS_II_ID: &str = "akai-mpk-mini-plus-ii";

/// Four rows of pads (two per bank), then three rows of buttons.
pub const ROWS: u8 = 7;
pub const COLS: u8 = 4;
pub const KEYS: u8 = 25;
pub const DIALS: u8 = 9;
const PADS: u8 = 16;
const SHIFT_KEY: u8 = 16;
const KNOBS: u8 = 8;
const MAIN_DIAL: u8 = 8;

/// The SysEx product ids the two decks answer to.
const MINI_IV_PID: u8 = 0x5d;
const PLUS_II_PID: u8 = 0x03;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// The pads' channel, from 0.
const PAD_CHANNEL: u8 = 9;
const FIRST_PAD_NOTE: u8 = 0x24;
const FIRST_KNOB_CC: u8 = 0x18;
const MAIN_CC: u8 = 0x0e;
const MAIN_PUSH_CC: u8 = 0x0d;
/// Shift, -, +, Tap Tempo, Undo, Loop, Play, Record, Overdub (keys 16-24).
const BUTTON_CCS: [u8; 9] = [0x11, 0x50, 0x51, 0x52, 0x49, 0x4a, 0x4c, 0x4d, 0x4e];

const SYSEX_START: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
const AKAI: u8 = 0x47;
/// The device id the deck's own reports carry; `00` marks NKS messages.
const DEVICE_REPORT: u8 = 0x7f;
const DEVICE_INQUIRY: [u8; 6] = [0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7];

const CMD_LINE_TEXT: u8 = 0x10;
const CMD_LINE_COLOR: u8 = 0x11;
const CMD_SCREEN_STATUS: u8 = 0x19;
const CMD_SCREEN_OWNER: u8 = 0x1c;
const CMD_PAD_MODE: u8 = 0x2a;
const CMD_PADS_DISABLED: u8 = 0x2b;
const CMD_DAW_MODE: u8 = 0x2d;
const CMD_PAD_NOTES: u8 = 0x2e;
const CMD_UNKNOWN_3A: u8 = 0x3a;
const OWNER_DAW: u8 = 0x02;
const OWNER_DECK: u8 = 0x00;
/// Pad mode 1, as Bitwig sets it: the pads light from the host.
const PAD_MODE_CLIPS: u8 = 0x01;

/// Bitwig's default font, 24 point.
const FONT: u8 = 0x02;
/// White, as far as is known: the line colour indices are not documented.
const LINE_COLOR: u8 = 0x03;
const LINE_MAX: usize = 31;

/// The state channel that turns a light off, with velocity 0.
const OFF_STATE: u8 = 0x06;
/// The solid brightness states and the level each shows, in percent.
const STATES: [(u8, u8); 6] = [(0, 10), (1, 25), (2, 50), (3, 65), (4, 75), (6, 100)];
const MONO_ON: u8 = 0x7f;
/// A channel below this is dark.
const DARK: u8 = 32;

/// The gap between two queued writes. No reference asks for one; a
/// millisecond keeps the open burst gentle. None under test.
const PACE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(1)
};
/// How long Ryu waits for an identity reply before setting the deck up
/// anyway (Ryu's own design). None under test.
const FALLBACK: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(500)
};
/// How often the screen-owner command is sent again in DAW mode (Bitwig's
/// 5.8 s). Checked from poll, never slept on. None under test (the tests set
/// their own).
const KEEP_ALIVE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(5800)
};

const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// The deck's pad palette as Bitwig lists it: index, then colour.
const PALETTE: &[(u8, [u8; 3])] = &[
    (0, [0x00, 0x00, 0x00]),
    (2, [0x7f, 0x7f, 0x7f]),
    (3, [0xff, 0xff, 0xff]),
    (4, [0xff, 0x4c, 0x4c]),
    (5, [0xff, 0x00, 0x00]),
    (8, [0xff, 0xbd, 0x6c]),
    (9, [0xff, 0x54, 0x00]),
    (12, [0xff, 0xff, 0x4c]),
    (13, [0xff, 0xff, 0x00]),
    (16, [0x88, 0xff, 0x4c]),
    (17, [0x54, 0xff, 0x00]),
    (18, [0x1d, 0x59, 0x00]),
    (20, [0x4c, 0xff, 0x4c]),
    (21, [0x00, 0xff, 0x00]),
    (22, [0x00, 0x59, 0x00]),
    (24, [0x4c, 0xff, 0x5e]),
    (25, [0x00, 0xff, 0x19]),
    (26, [0x00, 0x59, 0x0d]),
    (28, [0x4c, 0xff, 0x88]),
    (29, [0x00, 0xff, 0x55]),
    (30, [0x00, 0x59, 0x1d]),
    (32, [0x4c, 0xff, 0xb7]),
    (33, [0x00, 0xff, 0x99]),
    (34, [0x00, 0x59, 0x35]),
    (36, [0x4c, 0xc3, 0xff]),
    (37, [0x00, 0xa9, 0xff]),
    (38, [0x00, 0x41, 0x52]),
    (40, [0x4c, 0x88, 0xff]),
    (41, [0x00, 0x55, 0xff]),
    (44, [0x4c, 0x4c, 0xff]),
    (45, [0x00, 0x00, 0xff]),
    (48, [0x87, 0x4c, 0xff]),
    (49, [0x54, 0x00, 0xff]),
    (52, [0xff, 0x4c, 0xff]),
    (53, [0xff, 0x00, 0xff]),
    (56, [0xff, 0x4c, 0x87]),
    (57, [0xff, 0x00, 0x54]),
    (60, [0xff, 0x15, 0x00]),
    (61, [0x99, 0x35, 0x00]),
    (62, [0x79, 0x51, 0x00]),
    (63, [0x43, 0x64, 0x00]),
    (64, [0x03, 0x39, 0x00]),
    (65, [0x00, 0x57, 0x35]),
    (66, [0x00, 0x54, 0x7f]),
    (67, [0x00, 0x00, 0xff]),
    (68, [0x00, 0x45, 0x4f]),
    (69, [0x25, 0x00, 0xcc]),
    (73, [0xbd, 0xff, 0x2d]),
    (74, [0xaf, 0xed, 0x06]),
    (75, [0x64, 0xff, 0x09]),
    (76, [0x10, 0x8b, 0x00]),
    (77, [0x00, 0xff, 0x87]),
    (79, [0x00, 0x2a, 0xff]),
    (80, [0x3f, 0x00, 0xff]),
    (81, [0x7a, 0x00, 0xff]),
    (82, [0xb2, 0x1a, 0x7d]),
    (84, [0xff, 0x4a, 0x00]),
    (85, [0x88, 0xe1, 0x06]),
    (86, [0x72, 0xff, 0x15]),
    (88, [0x3b, 0xff, 0x26]),
    (90, [0x38, 0xff, 0xcc]),
    (91, [0x5b, 0x8a, 0xff]),
    (92, [0x31, 0x51, 0xc6]),
    (93, [0x87, 0x7f, 0xe9]),
    (94, [0xd3, 0x1d, 0xff]),
    (95, [0xff, 0x00, 0x5d]),
    (96, [0xff, 0x7f, 0x00]),
    (97, [0xb9, 0xb0, 0x00]),
    (98, [0x90, 0xff, 0x00]),
    (101, [0x14, 0x4c, 0x10]),
    (102, [0x0d, 0x50, 0x38]),
    (106, [0xa8, 0x00, 0x0a]),
    (107, [0xde, 0x51, 0x3d]),
    (108, [0xd8, 0x6a, 0x1c]),
    (109, [0xff, 0xe1, 0x26]),
    (110, [0x9e, 0xe1, 0x2f]),
    (111, [0x67, 0xb5, 0x0f]),
    (113, [0xdc, 0xff, 0x6b]),
    (114, [0x80, 0xff, 0xbd]),
    (116, [0x8e, 0x66, 0xff]),
    (118, [0x75, 0x75, 0x75]),
    (119, [0xe0, 0xff, 0xff]),
    (120, [0xa0, 0x00, 0x00]),
    (122, [0x1a, 0xd0, 0x00]),
    (126, [0xb3, 0x5f, 0x00]),
];

const NAMES: [&str; KEYS as usize] = [
    "Pad 5",
    "Pad 6",
    "Pad 7",
    "Pad 8",
    "Pad 1",
    "Pad 2",
    "Pad 3",
    "Pad 4",
    "Pad 13",
    "Pad 14",
    "Pad 15",
    "Pad 16",
    "Pad 9",
    "Pad 10",
    "Pad 11",
    "Pad 12",
    "Shift",
    "-",
    "+",
    "Tap Tempo",
    "Undo",
    "Loop",
    "Play",
    "Record",
    "Overdub",
];

const fn is_model(model: &DeviceModel) -> bool {
    matches!(model.protocol, Protocol::AkaiMpkMk4Midi)
}

/// The SysEx product id of a model.
fn pid_of(model: &DeviceModel) -> u8 {
    if model.id == MPK_MINI_PLUS_II_ID {
        PLUS_II_PID
    } else {
        MINI_IV_PID
    }
}

/// Whether `token` is in `base` (both lower case) with no letter or digit
/// right after it, so `mpk mini plus ii` never matches a Plus III.
fn has_token(base: &str, token: &str) -> bool {
    base.match_indices(token).any(|(at, found)| {
        !base
            .as_bytes()
            .get(at + found.len())
            .is_some_and(u8::is_ascii_alphanumeric)
    })
}

/// Whether a normalised port name is this model's DAW port (see the module
/// docs). Port spellings past Bitwig's are inferred.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if !is_model(model) {
        return false;
    }
    let base = name.base.to_ascii_lowercase();
    let token = if model.id == MPK_MINI_PLUS_II_ID {
        "mpk mini plus ii"
    } else {
        "mpk mini iv"
    };
    if !has_token(&base, token) {
        return false;
    }
    if let Some(interface) = name.interface {
        return interface == 2 && !base.contains("midi po");
    }
    if base.contains("midi po") {
        return false;
    }
    base.contains("daw")
        || base.ends_with(" midi 2")
        || (name.alsa_port == Some(1) && !base.ends_with(" midi 1"))
}

/// The legend on each key: the pads by the deck's numbering (1-8 in the
/// first bank, 9-16 in the second, bottom row first), then the buttons.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    is_model(model).then(|| NAMES.to_vec())
}

/// Every key but Shift has a light.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    is_model(model).then(|| (0..KEYS).filter(|key| *key != SHIFT_KEY).collect())
}

/// The key a pad note is, from a note's offset: within each bank of eight the
/// first four notes are the lower row, shown second.
const fn pad_key(offset: u8) -> u8 {
    let bank = offset / 8;
    let at = offset % 8;
    bank * 8 + if at < 4 { at + 4 } else { at - 4 }
}

/// The note a key's light is set through: its pad note, or a button's CC.
/// `None` for Shift and anything past the keys.
fn light_note(key: u8) -> Option<u8> {
    if key < PADS {
        // The mapping swaps rows within a bank, so it is its own inverse.
        return Some(FIRST_PAD_NOTE + pad_key(key));
    }
    if key == SHIFT_KEY {
        return None;
    }
    BUTTON_CCS.get(usize::from(key - PADS)).copied()
}

/// One Akai command: `F0 47 00 <pid> <cmd> <lenHi> <lenLo> <payload> F7`.
fn command(pid: u8, cmd: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let mut out = vec![
        SYSEX_START,
        AKAI,
        0x00,
        pid,
        cmd,
        u8::try_from((len >> 7) & 0x7f).unwrap_or(0),
        u8::try_from(len & 0x7f).unwrap_or(0),
    ];
    out.extend(payload.iter().map(|byte| byte & 0x7f));
    out.push(SYSEX_END);
    out
}

/// A screen line's text: font, line, up to 31 printable ASCII characters and
/// a NUL.
fn line_text(pid: u8, line: u8, text: &str) -> Vec<u8> {
    let mut payload = vec![FONT, line];
    payload.extend(
        text.bytes()
            .filter(|byte| (0x20..0x7f).contains(byte))
            .take(LINE_MAX),
    );
    payload.push(0x00);
    command(pid, CMD_LINE_TEXT, &payload)
}

/// The weighted distance Bitwig picks the nearest palette entry by, times
/// 100 to stay in integers.
fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    let squared = |x: u8, y: u8| u32::from(x.abs_diff(y)).pow(2);
    15 * squared(a[0], b[0]) + 35 * squared(a[1], b[1]) + 10 * squared(a[2], b[2])
}

/// The palette index nearest to a colour.
fn palette_index(color: [u8; 3]) -> u8 {
    PALETTE
        .iter()
        .min_by_key(|(_, entry)| distance(color, *entry))
        .map_or(0, |(index, _)| *index)
}

/// The solid state channel nearest a brightness, or `None` at 0%.
fn state(percent: u8) -> Option<u8> {
    if percent == 0 {
        return None;
    }
    STATES
        .iter()
        .min_by_key(|(_, level)| level.abs_diff(percent.min(100)))
        .map(|(state, _)| *state)
}

fn is_dark(color: [u8; 3]) -> bool {
    color.iter().all(|channel| *channel < DARK)
}

/// The Note On that shows `color` on a key's light at `percent`.
fn light(key: u8, color: [u8; 3], percent: u8) -> Option<[u8; 3]> {
    let note = light_note(key)?;
    let off = [NOTE_ON | OFF_STATE, note, 0];
    let Some(state) = state(percent) else {
        return Some(off);
    };
    if is_dark(color) {
        return Some(off);
    }
    if key >= PADS {
        return Some([NOTE_ON | state, note, MONO_ON]);
    }
    match palette_index(color) {
        0 => Some(off),
        index => Some([NOTE_ON | state, note, index]),
    }
}

/// A knob's two's-complement step: 1-63 up, 64-127 down.
fn knob_step(value: u8) -> i8 {
    if value < 64 {
        i8::try_from(value).unwrap_or(0)
    } else {
        i8::try_from(i16::from(value) - 128).unwrap_or(0)
    }
}

/// What queues one write, so a newer one to the same place replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Light(u8),
    Owner,
    Other,
}

pub struct MpkMk4 {
    transport: Box<dyn Transport>,
    pid: u8,
    name: &'static str,
    pace: Duration,
    fallback: Duration,
    keep_alive: Duration,
    /// When the device inquiry was queued.
    inquired: Option<Instant>,
    /// The identity reply named this model.
    replied: bool,
    /// The set-up messages are queued.
    connected: bool,
    /// The deck is in DAW mode, as far as Ryu knows.
    daw: bool,
    /// When the screen-owner command was last queued.
    owner_sent: Instant,
    /// Writes waiting their turn, drained from poll and flush.
    outbox: VecDeque<(Slot, Vec<u8>)>,
    next_send: Instant,
    colors: Vec<[u8; 3]>,
    /// The Note On each key's light was last queued with.
    sent: Vec<Option<[u8; 3]>>,
    brightness: u8,
    held: Vec<bool>,
    pushed: bool,
    parser: Parser,
}

impl MpkMk4 {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let name = if model.id == MPK_MINI_PLUS_II_ID {
            "MPK mini Plus II"
        } else {
            "MPK mini IV"
        };
        Self {
            transport,
            pid: pid_of(model),
            name,
            pace: PACE,
            fallback: FALLBACK,
            keep_alive: KEEP_ALIVE,
            inquired: None,
            replied: false,
            connected: false,
            daw: true,
            owner_sent: Instant::now(),
            outbox: VecDeque::new(),
            next_send: Instant::now(),
            colors: vec![BLACK; usize::from(KEYS)],
            sent: vec![None; usize::from(KEYS)],
            brightness: 100,
            held: vec![false; usize::from(KEYS)],
            pushed: false,
            parser: Parser::default(),
        }
    }

    fn send(&mut self, slot: Slot, message: Vec<u8>) {
        if slot != Slot::Other {
            self.outbox.retain(|(queued, _)| *queued != slot);
        }
        self.outbox.push_back((slot, message));
    }

    /// Writes the queued messages whose turn has come, never sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        while let Some((_, message)) = self.outbox.front() {
            let now = Instant::now();
            if now < self.next_send {
                break;
            }
            self.transport.write(message)?;
            self.outbox.pop_front();
            self.next_send = now + self.pace;
        }
        Ok(())
    }

    /// Queues the device inquiry once.
    fn start(&mut self) {
        if self.inquired.is_none() {
            self.inquired = Some(Instant::now());
            self.send(Slot::Other, DEVICE_INQUIRY.to_vec());
        }
    }

    fn claim_screen(&mut self) {
        self.owner_sent = Instant::now();
        self.send(
            Slot::Owner,
            command(self.pid, CMD_SCREEN_OWNER, &[OWNER_DAW]),
        );
    }

    /// Queues the set-up once the deck answered, or once the wait is over.
    fn connect(&mut self) {
        if self.connected {
            return;
        }
        let waited = self
            .inquired
            .is_some_and(|inquired| inquired.elapsed() >= self.fallback);
        if !self.replied && !waited {
            return;
        }
        self.connected = true;
        let pid = self.pid;
        self.claim_screen();
        self.send(Slot::Other, command(pid, CMD_PAD_MODE, &[PAD_MODE_CLIPS]));
        self.send(Slot::Other, command(pid, CMD_PADS_DISABLED, &[]));
        self.send(Slot::Other, command(pid, CMD_DAW_MODE, &[]));
        self.send(Slot::Other, command(pid, CMD_UNKNOWN_3A, &[]));
        self.send(Slot::Other, command(pid, CMD_PAD_NOTES, &[0x01]));
        for (line, text) in (0u8..).zip(["Ryu", self.name, "Connected"]) {
            self.send(
                Slot::Other,
                command(pid, CMD_LINE_COLOR, &[line, LINE_COLOR]),
            );
            self.send(Slot::Other, line_text(pid, line, text));
        }
        self.sent.fill(None);
    }

    /// Sends the screen-owner command again once it is due, in DAW mode.
    fn keep_screen(&mut self) {
        if self.connected && self.daw && self.owner_sent.elapsed() >= self.keep_alive {
            self.claim_screen();
        }
    }

    /// Queues every light whose Note On changed.
    fn queue_lights(&mut self) {
        if !self.connected || !self.daw {
            return;
        }
        for key in 0..KEYS {
            let color = self.colors.get(usize::from(key)).copied().unwrap_or(BLACK);
            let Some(message) = light(key, color, self.brightness) else {
                continue;
            };
            let Some(sent) = self.sent.get_mut(usize::from(key)) else {
                continue;
            };
            if *sent != Some(message) {
                *sent = Some(message);
                self.send(Slot::Light(key), message.to_vec());
            }
        }
    }

    /// Reads whatever arrived, waiting up to `timeout_ms` for the first.
    fn read(&mut self, timeout_ms: i32) -> DriverResult<Vec<Message>> {
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
        Ok(messages)
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

    fn push(&mut self, pressed: bool, events: &mut Vec<Input>) {
        if self.pushed != pressed {
            self.pushed = pressed;
            events.push(Input::DialPress {
                index: MAIN_DIAL,
                pressed,
            });
        }
    }

    /// Releases everything held, as the deck leaves DAW mode.
    fn release_all(&mut self, events: &mut Vec<Input>) {
        for key in 0..KEYS {
            self.press(key, false, events);
        }
        self.push(false, events);
    }

    fn sysex(&mut self, bytes: &[u8], events: &mut Vec<Input>) {
        // The identity reply: F0 7E <ch> 06 02 47 <pid> 00 19 ...
        if let [SYSEX_START, 0x7e, _, 0x06, 0x02, AKAI, pid, 0x00, 0x19, ..] = *bytes {
            if pid == self.pid {
                self.replied = true;
            }
            return;
        }
        // F0 47 7F <pid> <cmd> <lenHi> <lenLo> <payload> F7
        let [SYSEX_START, AKAI, DEVICE_REPORT, pid, cmd, high, low, ref rest @ ..] = *bytes else {
            return;
        };
        if pid != self.pid {
            return;
        }
        let len = usize::from(high & 0x7f) << 7 | usize::from(low & 0x7f);
        let payload = rest.get(..len).unwrap_or_default();
        match cmd {
            CMD_SCREEN_STATUS if len == 0 && self.connected && self.daw => self.claim_screen(),
            CMD_DAW_MODE => match payload.first() {
                Some(0) => {
                    if !self.daw {
                        self.daw = true;
                        self.sent.fill(None);
                    }
                    if self.connected {
                        self.claim_screen();
                    }
                }
                Some(_) if self.daw => {
                    self.daw = false;
                    self.release_all(events);
                    self.outbox
                        .retain(|(slot, _)| !matches!(slot, Slot::Light(_)));
                }
                _ => {}
            },
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
        if !self.daw {
            return;
        }
        match (status & 0xf0, status & 0x0f) {
            (kind @ (NOTE_ON | NOTE_OFF), PAD_CHANNEL)
                if (FIRST_PAD_NOTE..FIRST_PAD_NOTE + PADS).contains(&number) =>
            {
                self.press(
                    pad_key(number - FIRST_PAD_NOTE),
                    kind == NOTE_ON && value > 0,
                    events,
                );
            }
            (CONTROL_CHANGE, 0) => match number {
                MAIN_PUSH_CC => self.push(value > 0, events),
                MAIN_CC => {
                    let delta = match value {
                        0x01 => 1,
                        0x7f => -1,
                        _ => return,
                    };
                    events.push(Input::DialTurn {
                        index: MAIN_DIAL,
                        delta,
                    });
                }
                cc if (FIRST_KNOB_CC..FIRST_KNOB_CC + KNOBS).contains(&cc) => {
                    let delta = knob_step(value);
                    if delta != 0 {
                        events.push(Input::DialTurn {
                            index: cc - FIRST_KNOB_CC,
                            delta,
                        });
                    }
                }
                cc => {
                    if let Some(at) = BUTTON_CCS.iter().position(|known| *known == cc) {
                        let key = PADS + u8::try_from(at).unwrap_or(0);
                        self.press(key, value > 0, events);
                    }
                }
            },
            _ => {}
        }
    }

    /// Everything poll and flush share after reading.
    fn tick(&mut self) -> DriverResult<()> {
        self.start();
        self.connect();
        self.keep_screen();
        self.queue_lights();
        self.pump()
    }
}

impl Driver for MpkMk4 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.start();
        let mut events = Vec::new();
        for message in &self.read(timeout_ms)? {
            self.receive(message, &mut events);
        }
        self.tick()?;
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// Queues each light whose Note On changed (all of them the first time)
    /// and writes whatever's turn has come.
    fn flush(&mut self) -> DriverResult<()> {
        self.tick()
    }

    /// No brightness command: the brightness picks each light's state
    /// channel, sent on the next flush.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        Ok(())
    }
}

impl Drop for MpkMk4 {
    /// Turns off every light that may be lit, hands the screen back and puts
    /// the pad notes back on the keybed port; written at once, never paced.
    fn drop(&mut self) {
        if !self.connected {
            return;
        }
        let unsent = self
            .outbox
            .iter()
            .any(|(slot, _)| matches!(slot, Slot::Light(_)));
        self.outbox.clear();
        if self.daw {
            for key in 0..KEYS {
                let Some(off) = light(key, BLACK, 0) else {
                    continue;
                };
                let dark = self.sent.get(usize::from(key)) == Some(&Some(off));
                if dark && !unsent {
                    continue;
                }
                let _ = self.transport.write(&off);
            }
        }
        let _ = self
            .transport
            .write(&command(self.pid, CMD_SCREEN_OWNER, &[OWNER_DECK]));
        let _ = self
            .transport
            .write(&command(self.pid, CMD_PAD_NOTES, &[0x00]));
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    const IV_REPLY: [u8; 15] = [
        0xf0, 0x7e, 0x00, 0x06, 0x02, 0x47, 0x5d, 0x00, 0x19, 0x00, 0x01, 0x00, 0x00, 0x00, 0xf7,
    ];

    fn open_model(id: &str) -> (MpkMk4, Recorder) {
        let recorder = Recorder::default();
        let mut deck = MpkMk4::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        deck.keep_alive = Duration::from_secs(3600);
        (deck, recorder)
    }

    fn open() -> (MpkMk4, Recorder) {
        open_model(MPK_MINI_IV_ID)
    }

    /// The set-up writes after the inquiry, before the lights.
    const SETUP: usize = 6 + 6;

    /// A deck past its open burst, with those writes cleared.
    fn ready() -> (MpkMk4, Recorder) {
        let (mut deck, recorder) = open();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 1 + SETUP + 24);
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

    fn queue(recorder: &Recorder, messages: &[&[u8]]) {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
    }

    fn report(pid: u8, cmd: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![
            0xf0,
            0x47,
            0x7f,
            pid,
            cmd,
            0x00,
            u8::try_from(payload.len()).unwrap(),
        ];
        out.extend_from_slice(payload);
        out.push(0xf7);
        out
    }

    fn port(id: &str, name: &str) -> bool {
        port_matches(by_id(id).unwrap(), &normalise(name))
    }

    #[test]
    fn the_daw_port_is_matched_under_every_naming_and_the_midi_port_never() {
        let iv = MPK_MINI_IV_ID;
        let plus = MPK_MINI_PLUS_II_ID;
        // macOS.
        assert!(port(iv, "MPK mini IV DAW Port"));
        assert!(!port(iv, "MPK mini IV MIDI Port"));
        assert!(port(plus, "MPK mini Plus II DAW Port"));
        assert!(!port(plus, "MPK mini Plus II MIDI Port"));
        // Windows: the second interface only; the newer naming spelled out.
        assert!(port(iv, "MIDIIN2 (MPK mini IV)"));
        assert!(port(iv, "MIDIOUT2 (MPK mini IV)"));
        assert!(!port(iv, "MPK mini IV"));
        assert!(!port(iv, "MIDIIN3 (MPK mini IV)"));
        assert!(port(plus, "MIDIIN2 (MPK mini Plus II)"));
        assert!(port(plus, "MIDIOUT2 (MPK mini Plus II)"));
        assert!(!port(plus, "MPK mini Plus II"));
        // Linux, current kernels, through midir.
        assert!(port(iv, "MPK mini IV:MPK mini IV DAW Port 24:1"));
        assert!(!port(iv, "MPK mini IV:MPK mini IV MIDI Port 24:0"));
        assert!(port(
            plus,
            "MPK mini Plus II:MPK mini Plus II DAW Port 28:1"
        ));
        assert!(!port(
            plus,
            "MPK mini Plus II:MPK mini Plus II MIDI Port 28:0"
        ));
        // Older kernels, cut to 31 characters.
        assert!(port(iv, "MPK mini IV:MPK mini IV MPK mini IV DAW Por 24:1"));
        assert!(!port(
            iv,
            "MPK mini IV:MPK mini IV MPK mini IV MIDI Po 24:0"
        ));
        assert!(port(
            plus,
            "MPK mini Plus II:MPK mini Plus II MPK mini Plus  28:1"
        ));
        assert!(!port(
            plus,
            "MPK mini Plus II:MPK mini Plus II MPK mini Plus  28:0"
        ));
        // Jacks with no name.
        assert!(port(iv, "MPK mini IV:MPK mini IV MIDI 2 24:1"));
        assert!(!port(iv, "MPK mini IV:MPK mini IV MIDI 1 24:0"));
        // Other decks.
        assert!(!port(iv, "MPK mini Plus II DAW Port"));
        assert!(!port(plus, "MPK mini IV DAW Port"));
        assert!(!port(plus, "MPK mini Plus DAW Port"));
        assert!(!port(plus, "MPK mini Plus III DAW Port"));
        assert!(!port(iv, "MPK mini 3"));
        assert!(!port(iv, "MPK Mini Mk II"));
        assert!(!port(iv, "MPK mini Plus"));
        // Any case.
        assert!(port(iv, "MPK MINI IV DAW PORT"));
        let found = identify_midi(&normalise("MPK mini Plus II DAW Port"), false).unwrap();
        assert_eq!(found.id, plus);
        let found = identify_midi(&normalise("MIDIIN2 (MPK mini IV)"), true).unwrap();
        assert_eq!(found.id, iv);
        assert!(identify_midi(&normalise("MPK mini IV"), true).is_none());
    }

    #[test]
    fn the_models_light_every_key_but_shift_and_no_dial() {
        for id in [MPK_MINI_IV_ID, MPK_MINI_PLUS_II_ID] {
            let model = by_id(id).unwrap();
            assert_eq!(model.dial_lights, DialLights::None);
            assert_eq!(model.dial_light_colors(), 0);
            assert_eq!(
                (model.rows, model.cols, model.keys, model.dials),
                (7, 4, 25, 9)
            );
            assert!(model.image.is_none() && model.info_image.is_none());
            let view = ModelView::from(model);
            assert!(view.key_lights && view.brightness);
            let lit = lit_keys(model).unwrap();
            assert_eq!(lit.len(), 24);
            assert!(!lit.contains(&SHIFT_KEY));
            assert_eq!(key_names(model).unwrap().len(), usize::from(KEYS));
        }
        let iv = ModelView::from(by_id(MPK_MINI_IV_ID).unwrap());
        assert_eq!(iv.usb_ids, vec!["09e8:005d".to_owned()]);
        assert!(ModelView::from(by_id(MPK_MINI_PLUS_II_ID).unwrap())
            .usb_ids
            .is_empty());
    }

    #[test]
    fn open_inquires_then_sets_up_on_the_identity_reply() {
        let (mut deck, recorder) = open();
        deck.fallback = Duration::from_secs(3600);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(20, Some(&lit([255, 255, 255]))).unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes(), vec![DEVICE_INQUIRY.to_vec()]);
        // Another deck's reply does not count.
        let mut other = IV_REPLY;
        other[6] = 0x03;
        queue(&recorder, &[&other]);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 1);
        queue(&recorder, &[&IV_REPLY]);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 1 + SETUP + 24);
        let expected: Vec<Vec<u8>> = vec![
            vec![0xf0, 0x47, 0x00, 0x5d, 0x1c, 0x00, 0x01, 0x02, 0xf7],
            vec![0xf0, 0x47, 0x00, 0x5d, 0x2a, 0x00, 0x01, 0x01, 0xf7],
            vec![0xf0, 0x47, 0x00, 0x5d, 0x2b, 0x00, 0x00, 0xf7],
            vec![0xf0, 0x47, 0x00, 0x5d, 0x2d, 0x00, 0x00, 0xf7],
            vec![0xf0, 0x47, 0x00, 0x5d, 0x3a, 0x00, 0x00, 0xf7],
            vec![0xf0, 0x47, 0x00, 0x5d, 0x2e, 0x00, 0x01, 0x01, 0xf7],
            vec![0xf0, 0x47, 0x00, 0x5d, 0x11, 0x00, 0x02, 0x00, 0x03, 0xf7],
            vec![
                0xf0, 0x47, 0x00, 0x5d, 0x10, 0x00, 0x06, 0x02, 0x00, b'R', b'y', b'u', 0x00, 0xf7,
            ],
        ];
        assert_eq!(writes[1..9], expected[..]);
        let mut model_line = vec![0xf0, 0x47, 0x00, 0x5d, 0x10, 0x00, 14, 0x02, 0x01];
        model_line.extend_from_slice(b"MPK mini IV");
        model_line.extend_from_slice(&[0x00, 0xf7]);
        assert_eq!(writes[10], model_line);
        assert_eq!(
            writes[11],
            vec![0xf0, 0x47, 0x00, 0x5d, 0x11, 0x00, 0x02, 0x02, 0x03, 0xf7]
        );
        // Key 0 is the first bank's upper row: note 0x28, red is index 5.
        assert_eq!(writes[13], vec![0x96, 0x28, 0x05]);
        assert_eq!(writes[14], vec![0x96, 0x29, 0x00]);
        assert_eq!(writes[17], vec![0x96, 0x24, 0x00]);
        // Key 17 (-) is CC 0x50; key 20 (Undo) is lit by CC 0x49.
        assert_eq!(writes[13 + 16], vec![0x96, 0x50, 0x00]);
        assert_eq!(writes[13 + 19], vec![0x96, 0x49, 0x7f]);
        assert_eq!(writes.last(), Some(&vec![0x96, 0x4e, 0x00]));
        // Nothing twice.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 1 + SETUP + 24);
    }

    #[test]
    fn with_no_reply_the_deck_is_set_up_after_the_wait() {
        let (mut deck, recorder) = open();
        deck.fallback = Duration::from_secs(3600);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 1);
        deck.fallback = Duration::ZERO;
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 1 + SETUP + 24);
    }

    #[test]
    fn the_plus_ii_uses_its_own_product_id_and_name() {
        let (mut deck, recorder) = open_model(MPK_MINI_PLUS_II_ID);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[1],
            vec![0xf0, 0x47, 0x00, 0x03, 0x1c, 0x00, 0x01, 0x02, 0xf7]
        );
        let mut model_line = vec![0xf0, 0x47, 0x00, 0x03, 0x10, 0x00, 19, 0x02, 0x01];
        model_line.extend_from_slice(b"MPK mini Plus II");
        model_line.extend_from_slice(&[0x00, 0xf7]);
        assert_eq!(writes[10], model_line);
        // Its reports carry 03; the IV's 5D is not its own.
        queue(
            &recorder,
            &[&report(0x5d, 0x2d, &[0x01]), &[0x99, 0x24, 100]],
        );
        assert_eq!(deck.poll(0).unwrap(), vec![key(4, true)]);
    }

    #[test]
    fn writes_are_paced_by_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open();
        deck.pace = Duration::from_secs(60);
        let begun = Instant::now();
        deck.poll(0).unwrap();
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert_eq!(recorder.writes(), vec![DEVICE_INQUIRY.to_vec()]);
        deck.pace = Duration::ZERO;
        deck.next_send = Instant::now();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 1 + SETUP + 24);
    }

    #[test]
    fn brightness_picks_the_state_channel_and_only_changes_are_sent() {
        let (mut deck, recorder) = ready();
        deck.set_face(8, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(24, Some(&lit([200, 200, 0]))).unwrap();
        // Shift has no light.
        deck.set_face(16, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![vec![0x96, 0x30, 45], vec![0x96, 0x4e, 0x7f]]
        );
        deck.set_brightness(50).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[2], vec![0x92, 0x30, 45]);
        assert_eq!(writes[3], vec![0x92, 0x4e, 0x7f]);
        assert_eq!(writes.len(), 4);
        deck.set_brightness(70).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[4], vec![0x93, 0x30, 45]);
        deck.set_brightness(5).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[6], vec![0x90, 0x30, 45]);
        deck.set_brightness(0).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[8], vec![0x96, 0x30, 0x00]);
        assert_eq!(writes[9], vec![0x96, 0x4e, 0x00]);
        assert_eq!(writes.len(), 10);
        // Channel 5 is never sent.
        for percent in 0..=100 {
            assert_ne!(state(percent), Some(5));
        }
    }

    #[test]
    fn colours_take_the_nearest_palette_entry_and_dark_ones_go_off() {
        assert_eq!(palette_index([255, 255, 255]), 3);
        assert_eq!(palette_index([0, 255, 0]), 21);
        assert_eq!(palette_index([255, 127, 0]), 96);
        assert_eq!(palette_index([127, 127, 127]), 2);
        assert_eq!(light(0, [20, 20, 20], 100), Some([0x96, 0x28, 0x00]));
        assert_eq!(light(16, [255, 255, 255], 100), None);
        assert_eq!(light(25, [255, 255, 255], 100), None);
    }

    #[test]
    fn pads_and_buttons_press_and_release_keys() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                // First bank: lower row 0x24-0x27, upper row 0x28-0x2B.
                &[0x99, 0x24, 100],
                &[0x89, 0x24, 0],
                &[0x99, 0x2b, 90],
                &[0x99, 0x2b, 0],
                // Second bank.
                &[0x99, 0x2c, 1],
                &[0x99, 0x33, 1],
                &[0xb0, 0x11, 0x7f],
                &[0xb0, 0x50, 0x7f],
                &[0xb0, 0x52, 0x7f],
                &[0xb0, 0x49, 0x7f],
                &[0xb0, 0x4e, 0x7f],
                &[0xb0, 0x4e, 0x00],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(4, true),
                key(4, false),
                key(3, true),
                key(3, false),
                key(12, true),
                key(11, true),
                key(16, true),
                key(17, true),
                key(19, true),
                key(20, true),
                key(24, true),
                key(24, false),
            ]
        );
    }

    #[test]
    fn knobs_are_twos_complement_and_the_main_encoder_clicks() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0xb0, 0x18, 0x01],
                &[0xb0, 0x18, 0x3f],
                &[0xb0, 0x1f, 0x7f],
                &[0xb0, 0x1b, 0x41],
                &[0xb0, 0x1c, 0x40],
                &[0xb0, 0x1c, 0x00],
                &[0xb0, 0x0e, 0x01],
                &[0xb0, 0x0e, 0x7f],
                &[0xb0, 0x0e, 0x02],
                &[0xb0, 0x0e, 0x41],
                &[0xb0, 0x0d, 0x7f],
                &[0xb0, 0x0d, 0x00],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, 1),
                turn(0, 63),
                turn(7, -1),
                turn(3, -63),
                turn(4, -64),
                turn(8, 1),
                turn(8, -1),
                Input::DialPress {
                    index: 8,
                    pressed: true
                },
                Input::DialPress {
                    index: 8,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn aftertouch_bend_unmodelled_controls_and_other_channels_are_ignored() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0xa9, 0x24, 50],
                &[0x90, 60, 100],
                &[0x98, 0x24, 100],
                &[0x99, 0x23, 100],
                &[0x99, 0x34, 100],
                &[0xe0, 0, 64],
                &[0xb0, 0x01, 64],
                &[0xb0, 0x0c, 0x7f],
                &[0xb0, 0x53, 0x7f],
                &[0xb0, 0x78, 0x7f],
                &[0xb1, 0x11, 0x7f],
                // Pad mode reports and NKS messages change nothing.
                &report(0x5d, 0x2a, &[0x02]),
                &[0xf0, 0x47, 0x00, 0x5d, 0x2d, 0x00, 0x01, 0x01, 0xf7],
            ],
        );
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn plugin_mode_mutes_input_and_lights_until_daw_mode_returns() {
        let (mut deck, recorder) = ready();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        queue(&recorder, &[&[0x99, 0x25, 100]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(5, true)]);
        queue(
            &recorder,
            &[
                &report(0x5d, 0x2d, &[0x01]),
                &[0x99, 0x26, 100],
                &[0xb0, 0x18, 0x01],
            ],
        );
        assert_eq!(deck.poll(0).unwrap(), vec![key(5, false)]);
        deck.set_face(1, Some(&lit([0, 255, 0]))).unwrap();
        deck.keep_alive = Duration::ZERO;
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
        // DAW mode again: the screen is claimed and every light sent again.
        deck.keep_alive = Duration::from_secs(3600);
        queue(&recorder, &[&report(0x5d, 0x2d, &[0x00])]);
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(writes.len(), 1 + 24);
        assert_eq!(
            writes[0],
            vec![0xf0, 0x47, 0x00, 0x5d, 0x1c, 0x00, 0x01, 0x02, 0xf7]
        );
        assert_eq!(writes[1], vec![0x96, 0x28, 0x05]);
        assert_eq!(writes[2], vec![0x96, 0x29, 21]);
        queue(&recorder, &[&[0x99, 0x26, 100]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(6, true)]);
    }

    #[test]
    fn the_screen_is_claimed_again_when_due_and_when_the_deck_asks() {
        let (mut deck, recorder) = ready();
        let owner = vec![0xf0, 0x47, 0x00, 0x5d, 0x1c, 0x00, 0x01, 0x02, 0xf7];
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
        deck.keep_alive = Duration::ZERO;
        deck.flush().unwrap();
        assert_eq!(recorder.writes(), vec![owner.clone()]);
        deck.keep_alive = Duration::from_secs(3600);
        // A screen status with no payload asks for it; one with a payload
        // does not.
        queue(&recorder, &[&report(0x5d, 0x19, &[])]);
        deck.poll(0).unwrap();
        queue(&recorder, &[&report(0x5d, 0x19, &[0x01])]);
        deck.poll(0).unwrap();
        // A DAW report claims it too.
        queue(&recorder, &[&report(0x5d, 0x2d, &[0x00])]);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes(), vec![owner.clone(), owner.clone(), owner]);
    }

    #[test]
    fn short_and_malformed_input_never_panics() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0x99],
                &[0xb0, 0x18],
                &[0xf0],
                &[0xf7],
                &[0xf0, 0x47, 0x7f, 0x5d, 0x2d, 0xf7],
                &[0xf0, 0x47, 0x7f, 0x5d, 0x2d, 0x00, 0x05, 0xf7],
                &[0xf0, 0x47, 0x7f, 0x5d, 0x2d, 0x7f, 0x7f, 0x00, 0xf7],
                &[0xf0, 0x7e, 0x00, 0x06, 0x02, 0x47, 0xf7],
                &[0xff, 0xfe, 0x00],
            ],
        );
        deck.poll(0).unwrap();
    }

    #[test]
    fn dropping_turns_lit_lights_off_and_hands_the_deck_back_without_pacing() {
        let (mut deck, recorder) = ready();
        deck.pace = Duration::from_secs(60);
        deck.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(21, Some(&lit([0, 0, 255]))).unwrap();
        // The first light goes out; the second waits its turn.
        deck.flush().unwrap();
        let begun = Instant::now();
        drop(deck);
        assert!(begun.elapsed() < Duration::from_secs(1));
        let writes = recorder.writes();
        assert_eq!(writes.first(), Some(&vec![0x96, 0x2b, 0x05]));
        // A light was left unsent, so every light goes off.
        assert_eq!(writes.len(), 1 + 24 + 2);
        assert_eq!(writes[1], vec![0x96, 0x28, 0x00]);
        assert_eq!(
            writes[25],
            vec![0xf0, 0x47, 0x00, 0x5d, 0x1c, 0x00, 0x01, 0x00, 0xf7]
        );
        assert_eq!(
            writes[26],
            vec![0xf0, 0x47, 0x00, 0x5d, 0x2e, 0x00, 0x01, 0x00, 0xf7]
        );
    }

    #[test]
    fn dropping_a_dark_deck_only_hands_it_back() {
        let (deck, recorder) = ready();
        drop(deck);
        assert_eq!(recorder.writes().len(), 2);
    }

    #[test]
    fn dropping_a_deck_never_set_up_sends_nothing_more() {
        let (mut deck, recorder) = open();
        deck.fallback = Duration::from_secs(3600);
        deck.poll(0).unwrap();
        drop(deck);
        assert_eq!(recorder.writes(), vec![DEVICE_INQUIRY.to_vec()]);
    }
}
