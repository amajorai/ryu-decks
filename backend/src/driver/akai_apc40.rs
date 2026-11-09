//! Akai Professional's APC40 MKII, over class-compliant USB MIDI
//! ([`super::midi`]).
//!
//! The APC40 MKII lists one MIDI port each way. Its name holds `APC40 mkII`
//! (Ableton lists the deck as `APC40 mkII` and `Akai APC40 MkII`; the exact
//! name each host gives the port is not in the references), so
//! [`port_matches`] takes any name containing `apc40 mkii` in any case, and on
//! Windows only the first interface.
//!
//! Start-up: the deck powers up in a generic mode whose lights and knob
//! messages differ, so Ryu first sends the universal identity request (`F0 7E
//! 00 06 01 F7`) and waits for Akai's reply (`F0 7E <id> 06 02 47 29 ... F7`),
//! whose byte 13 is the device id later SysEx must carry. It then sends the
//! introduction that selects Ableton Live mode (`F0 47 <id> 29 60 00 04 41
//! <host version: 3 bytes> F7`), turns the 16 knob rings off and centres every
//! ringed knob. No reply in three tries of half a second refuses the deck. A
//! reconnect opens a new driver, which runs all of this again.
//!
//! In (MIDI channel `n` is the status' low nibble; notes 48-52 and 66 are
//! reused across channels 0-7, one per track, so a message is always decoded
//! by channel and note together): the 5 x 8 clip matrix is note `32 + track -
//! 8 * scene` on channel 0 with scene 0 the top row, so the top row is notes
//! 32-39 and the bottom row 0-7; scene launch 82-86; clip stop 52, track
//! select 51, mute 50, solo 49, arm 48 and crossfade assign 66 on channels
//! 0-7; Stop All Clips 81, master select 80, the device buttons 58-65, Pan 87,
//! Sends 88, User 89, Metronome 90, Play 91, Record 93, Session 102, Bank 103,
//! the bank arrows 94-97 (up, down, right, left), Shift 98, Tap Tempo 99,
//! Nudge 100-101, all on channel 0. Note On above velocity 0 presses; Note Off
//! or velocity 0 releases. The foot pedal is `CC 64`, pressed from 64.
//!
//! Knobs and faders are Control Change on channel 0 unless noted. The 8 track
//! knobs (`CC 48-55`) and 8 device knobs (`CC 16-23`) are endless but report
//! an absolute 0-127 in Ableton mode, a value the host can overwrite: Ryu
//! turns the change into a dial step and writes 64 back near either end so
//! they keep turning. Tempo (`CC 13`) and Cue level (`CC 47`) are relative,
//! two's complement. The channel faders (`CC 7` on channels 0-7), master
//! (`CC 14`) and crossfader (`CC 15`) are absolute and not motorised: each
//! one's first value only seeds it, then it turns its dial a step per four
//! units.
//!
//! Out: the 40 clip pads and 5 scene buttons are RGB through a 128-entry
//! palette, Note On on channel 0 whose velocity is the palette index (Ryu
//! picks the nearest entry; a second note on channels 6-15 would blink or
//! pulse it, and Ryu never sends one). Every other light is one colour, Note
//! On on the button's own channel and note: 127 on and 0 off, except clip stop
//! (1 on) and crossfade assign (1 for A, 2 for B: Ryu shows B for a blue
//! face). Stop All Clips, the bank arrows, Shift, Tap Tempo, Nudge and the
//! pedal have no light the references drive. There is no brightness command
//! and no keep-alive. Only changed lights are sent, and Ryu turns them all off
//! when it lets go (leaving the deck in Ableton mode: the command for generic
//! mode is not in the references).
//!
//! Facts from Ableton's APC40 MKII Remote Script as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for
//! note and controller numbers, the handshake, light values, the palette and
//! the USB id only), the USB id cross-checked against the usb.ids database
//! (BSD / GPL); no code was copied.

use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverError, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const APC40_MK2_ID: &str = "akai-apc40-mk2";

pub const KEYS: u8 = 120;
pub const DIALS: u8 = 28;

const MATRIX_KEYS: u8 = 40;
const SCENE_KEY: u8 = 40;
const SCENES: u8 = 5;
const STOP_KEY: u8 = 45;
const STOP_ALL_KEY: u8 = 53;
const SELECT_KEY: u8 = 54;
const MASTER_KEY: u8 = 62;
const MUTE_KEY: u8 = 63;
const SOLO_KEY: u8 = 71;
const ARM_KEY: u8 = 79;
const CROSSFADE_KEY: u8 = 87;
const DEVICE_KEY: u8 = 95;
const PAN_KEY: u8 = 103;
const BANK_KEY: u8 = 110;
const UP_KEY: u8 = 111;
const PEDAL_KEY: u8 = 119;
const TRACKS: u8 = 8;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;

const SCENE_NOTE: u8 = 82;
const STOP_NOTE: u8 = 52;
const STOP_ALL_NOTE: u8 = 81;
const SELECT_NOTE: u8 = 51;
const MASTER_NOTE: u8 = 80;
const MUTE_NOTE: u8 = 50;
const SOLO_NOTE: u8 = 49;
const ARM_NOTE: u8 = 48;
const CROSSFADE_NOTE: u8 = 66;
const DEVICE_NOTE: u8 = 58;
/// Keys 103-118 on channel 0, in key order (note 92 is unused).
const PANEL_NOTES: [u8; 16] = [
    87, 88, 89, 90, 91, 93, 102, 103, 94, 95, 96, 97, 98, 99, 100, 101,
];
const PEDAL_CC: u8 = 64;

const TRACK_KNOB_CC: u8 = 48;
const TRACK_RING_CC: u8 = 56;
const DEVICE_KNOB_CC: u8 = 16;
const DEVICE_RING_CC: u8 = 24;
const TEMPO_CC: u8 = 13;
const CUE_CC: u8 = 47;
const FADER_CC: u8 = 7;
const MASTER_CC: u8 = 14;
const CROSSFADER_CC: u8 = 15;

/// Dials 0-15 are the ringed knobs, 16 Tempo, 17 Cue, 18-25 the channel
/// faders, 26 master and 27 the crossfader.
const KNOBS: u8 = 16;
const TEMPO_DIAL: u8 = 16;
const CUE_DIAL: u8 = 17;
const FADER_DIAL: u8 = 18;
const MASTER_DIAL: u8 = 26;
const CROSSFADER_DIAL: u8 = 27;

/// A ringed knob's centre, written back so it keeps turning.
const CENTRE: u8 = 64;
/// Values this close to either end re-centre the knob.
const NEAR_END: u8 = 8;
/// Fader units (of 128) per dial step: 32 steps from end to end.
const FADER_STEP: i16 = 4;

const SYSEX_START: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
const IDENTITY_REQUEST: [u8; 6] = [0xf0, 0x7e, 0x00, 0x06, 0x01, 0xf7];
const AKAI: u8 = 0x47;
const APC40_MK2_MODEL: u8 = 0x29;
const INTRODUCTION: u8 = 0x60;
const ABLETON_MODE: u8 = 0x41;
/// The host version the introduction reports (any 7-bit values).
const HOST_VERSION: [u8; 3] = [0x0c, 0x00, 0x00];
const IDENTITY_TRIES: usize = 3;
const IDENTITY_WAIT: Duration = Duration::from_millis(500);
/// Messages read while waiting for one identity reply, at most.
const MAX_HANDSHAKE_MESSAGES: usize = 256;

const LED_ON: u8 = 127;
const LED_OFF: u8 = 0;
const STOP_ON: u8 = 1;
const CROSSFADE_A: u8 = 1;
const CROSSFADE_B: u8 = 2;
/// A single-colour light is on while a channel reaches this.
const DARK: u8 = 32;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// The 24-bit colour of each palette index the RGB pads take.
const PALETTE: [u32; 128] = [
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

const NAMES: [&str; KEYS as usize] = [
    "Clip 1-1",
    "Clip 1-2",
    "Clip 1-3",
    "Clip 1-4",
    "Clip 1-5",
    "Clip 1-6",
    "Clip 1-7",
    "Clip 1-8",
    "Clip 2-1",
    "Clip 2-2",
    "Clip 2-3",
    "Clip 2-4",
    "Clip 2-5",
    "Clip 2-6",
    "Clip 2-7",
    "Clip 2-8",
    "Clip 3-1",
    "Clip 3-2",
    "Clip 3-3",
    "Clip 3-4",
    "Clip 3-5",
    "Clip 3-6",
    "Clip 3-7",
    "Clip 3-8",
    "Clip 4-1",
    "Clip 4-2",
    "Clip 4-3",
    "Clip 4-4",
    "Clip 4-5",
    "Clip 4-6",
    "Clip 4-7",
    "Clip 4-8",
    "Clip 5-1",
    "Clip 5-2",
    "Clip 5-3",
    "Clip 5-4",
    "Clip 5-5",
    "Clip 5-6",
    "Clip 5-7",
    "Clip 5-8",
    "Scene 1",
    "Scene 2",
    "Scene 3",
    "Scene 4",
    "Scene 5",
    "Stop 1",
    "Stop 2",
    "Stop 3",
    "Stop 4",
    "Stop 5",
    "Stop 6",
    "Stop 7",
    "Stop 8",
    "Stop All Clips",
    "Select 1",
    "Select 2",
    "Select 3",
    "Select 4",
    "Select 5",
    "Select 6",
    "Select 7",
    "Select 8",
    "Master",
    "Track 1",
    "Track 2",
    "Track 3",
    "Track 4",
    "Track 5",
    "Track 6",
    "Track 7",
    "Track 8",
    "Solo 1",
    "Solo 2",
    "Solo 3",
    "Solo 4",
    "Solo 5",
    "Solo 6",
    "Solo 7",
    "Solo 8",
    "Arm 1",
    "Arm 2",
    "Arm 3",
    "Arm 4",
    "Arm 5",
    "Arm 6",
    "Arm 7",
    "Arm 8",
    "A|B 1",
    "A|B 2",
    "A|B 3",
    "A|B 4",
    "A|B 5",
    "A|B 6",
    "A|B 7",
    "A|B 8",
    "Device Left",
    "Device Right",
    "Bank Left",
    "Bank Right",
    "Device On/Off",
    "Device Lock",
    "Clip/Device View",
    "Detail View",
    "Pan",
    "Sends",
    "User",
    "Metronome",
    "Play",
    "Record",
    "Session",
    "Bank",
    "Up",
    "Down",
    "Right",
    "Left",
    "Shift",
    "Tap Tempo",
    "Nudge -",
    "Nudge +",
    "Foot Pedal",
];

/// Whether a normalised port name is an APC40 MKII's: `apc40 mkii` in any
/// case, and on Windows the first (only) interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.protocol == Protocol::AkaiApc40Mk2Midi
        && name.interface.unwrap_or(1) == 1
        && name.base.to_ascii_lowercase().contains("apc40 mkii")
}

fn is_apc40(model: &DeviceModel) -> bool {
    model.protocol == Protocol::AkaiApc40Mk2Midi
}

/// The legend on each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    is_apc40(model).then(|| NAMES.to_vec())
}

/// Whether a key has a light the references drive.
const fn is_lit(key: u8) -> bool {
    key < KEYS && key != STOP_ALL_KEY && !(key >= UP_KEY && key <= PEDAL_KEY)
}

/// Every key but Stop All Clips, the bank arrows, Shift, Tap Tempo, Nudge and
/// the pedal.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    is_apc40(model).then(|| (0..KEYS).filter(|key| is_lit(*key)).collect())
}

/// Each key's (row, column) in the 14 x 9 grid: the clip matrix top left with
/// the scene column at its right, then a row each for clip stop (Stop All at
/// the right), track select (Master), mute, solo, arm and crossfade assign,
/// then the device buttons (Bank at the right), the mode and transport
/// buttons, and the arrows, Shift, Tap Tempo, Nudge and the pedal.
const fn cell(key: u8) -> (u8, u8) {
    match key {
        0..MATRIX_KEYS => (key / TRACKS, key % TRACKS),
        SCENE_KEY..STOP_KEY => (key - SCENE_KEY, TRACKS),
        STOP_KEY..=STOP_ALL_KEY => (5, key - STOP_KEY),
        SELECT_KEY..=MASTER_KEY => (6, key - SELECT_KEY),
        MUTE_KEY..SOLO_KEY => (7, key - MUTE_KEY),
        SOLO_KEY..ARM_KEY => (8, key - SOLO_KEY),
        ARM_KEY..CROSSFADE_KEY => (9, key - ARM_KEY),
        CROSSFADE_KEY..DEVICE_KEY => (10, key - CROSSFADE_KEY),
        DEVICE_KEY..PAN_KEY => (11, key - DEVICE_KEY),
        BANK_KEY => (11, TRACKS),
        PAN_KEY..BANK_KEY => (12, key - PAN_KEY),
        _ => (13, key - UP_KEY),
    }
}

/// Each key's cell in the model's grid.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    if !is_apc40(model) {
        return None;
    }
    let cols = u16::from(model.cols);
    Some(
        (0..KEYS)
            .map(|key| {
                let (row, col) = cell(key);
                u16::from(row) * cols + u16::from(col)
            })
            .collect(),
    )
}

/// The (channel, note) of a key's button; the pedal is a controller instead.
const fn source_of_key(key: u8) -> Option<(u8, u8)> {
    match key {
        0..MATRIX_KEYS => {
            let (scene, track) = (key / TRACKS, key % TRACKS);
            Some((0, 32 + track - TRACKS * scene))
        }
        SCENE_KEY..STOP_KEY => Some((0, SCENE_NOTE + key - SCENE_KEY)),
        STOP_KEY..STOP_ALL_KEY => Some((key - STOP_KEY, STOP_NOTE)),
        STOP_ALL_KEY => Some((0, STOP_ALL_NOTE)),
        SELECT_KEY..MASTER_KEY => Some((key - SELECT_KEY, SELECT_NOTE)),
        MASTER_KEY => Some((0, MASTER_NOTE)),
        MUTE_KEY..SOLO_KEY => Some((key - MUTE_KEY, MUTE_NOTE)),
        SOLO_KEY..ARM_KEY => Some((key - SOLO_KEY, SOLO_NOTE)),
        ARM_KEY..CROSSFADE_KEY => Some((key - ARM_KEY, ARM_NOTE)),
        CROSSFADE_KEY..DEVICE_KEY => Some((key - CROSSFADE_KEY, CROSSFADE_NOTE)),
        DEVICE_KEY..PAN_KEY => Some((0, DEVICE_NOTE + key - DEVICE_KEY)),
        PAN_KEY..PEDAL_KEY => Some((0, PANEL_NOTES[(key - PAN_KEY) as usize])),
        _ => None,
    }
}

/// The key a note on a channel presses.
fn key_of_note(channel: u8, note: u8) -> Option<u8> {
    let track = (channel < TRACKS).then_some(channel);
    let per_track = |first: u8| track.map(|track| first + track);
    match note {
        0..=39 if channel == 0 => {
            let (scene, track) = (SCENES - 1 - note / TRACKS, note % TRACKS);
            Some(scene * TRACKS + track)
        }
        STOP_NOTE => per_track(STOP_KEY),
        SELECT_NOTE => per_track(SELECT_KEY),
        MUTE_NOTE => per_track(MUTE_KEY),
        SOLO_NOTE => per_track(SOLO_KEY),
        ARM_NOTE => per_track(ARM_KEY),
        CROSSFADE_NOTE => per_track(CROSSFADE_KEY),
        _ if channel != 0 => None,
        82..=86 => Some(SCENE_KEY + note - SCENE_NOTE),
        STOP_ALL_NOTE => Some(STOP_ALL_KEY),
        MASTER_NOTE => Some(MASTER_KEY),
        58..=65 => Some(DEVICE_KEY + note - DEVICE_NOTE),
        _ => (PAN_KEY..)
            .zip(PANEL_NOTES)
            .find(|(_, panel)| *panel == note)
            .map(|(key, _)| key),
    }
}

/// The palette index nearest a colour.
pub(super) fn palette_index([red, green, blue]: [u8; 3]) -> u8 {
    let distance = |entry: u32| {
        let [_, r, g, b] = entry.to_be_bytes();
        let squared = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        squared(r, red) + squared(g, green) + squared(b, blue)
    };
    (0u8..)
        .zip(PALETTE)
        .min_by_key(|(_, entry)| distance(*entry))
        .map_or(0, |(index, _)| index)
}

fn is_dark(color: [u8; 3]) -> bool {
    color.iter().all(|channel| *channel < DARK)
}

/// Blue-ish: blue is the strongest channel.
fn is_blue(color: [u8; 3]) -> bool {
    let [red, green, blue] = color;
    blue > red && blue > green
}

/// The velocity that shows a colour on a key's light.
fn velocity(key: u8, color: [u8; 3]) -> u8 {
    if key < STOP_KEY {
        return palette_index(color);
    }
    if is_dark(color) {
        return LED_OFF;
    }
    match key {
        STOP_KEY..STOP_ALL_KEY => STOP_ON,
        CROSSFADE_KEY..DEVICE_KEY if is_blue(color) => CROSSFADE_B,
        CROSSFADE_KEY..DEVICE_KEY => CROSSFADE_A,
        _ => LED_ON,
    }
}

/// A two's-complement relative value as a step.
fn relative(value: u8) -> i8 {
    let value = i16::from(value & 0x7f);
    let signed = if value < 64 { value } else { value - 128 };
    i8::try_from(signed).unwrap_or(0)
}

/// The identity reply from an APC40 MKII: its device id and firmware.
fn identity_reply(message: &[u8]) -> Option<(u8, String)> {
    let header = message.get(..7)?;
    if header[0] != SYSEX_START
        || header[1] != 0x7e
        || header[3] != 0x06
        || header[4] != 0x02
        || header[5] != AKAI
        || header[6] != APC40_MK2_MODEL
    {
        return None;
    }
    let version = message.get(9..13)?;
    let device_id = *message.get(13)?;
    if device_id > 0x7f {
        return None;
    }
    let firmware = format!(
        "{}.{}",
        ((version[0] & 0x0f) << 4) | (version[1] & 0x0f),
        ((version[2] & 0x0f) << 4) | (version[3] & 0x0f)
    );
    Some((device_id, firmware))
}

/// The introduction that selects Ableton Live mode.
fn introduction(device_id: u8) -> [u8; 12] {
    let [major, minor, bugfix] = HOST_VERSION;
    [
        SYSEX_START,
        AKAI,
        device_id,
        APC40_MK2_MODEL,
        INTRODUCTION,
        0x00,
        0x04,
        ABLETON_MODE,
        major,
        minor,
        bugfix,
        SYSEX_END,
    ]
}

pub struct Apc40Mk2 {
    transport: Box<dyn Transport>,
    /// Ableton mode set and the knobs centred.
    started: bool,
    colors: Vec<[u8; 3]>,
    /// The velocity each key's light was last sent; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    /// Each absolute dial's last value: the knobs from their centre, the
    /// faders `None` until they first move. Unused for Tempo and Cue.
    last: Vec<Option<u8>>,
    /// Fader movement not yet turned into a whole step.
    rest: Vec<i16>,
    parser: Parser,
    /// Identity requests sent so far.
    identity_tries: usize,
    /// When the identity request in flight stops waiting for its reply.
    identity_deadline: Option<Instant>,
}

impl Apc40Mk2 {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            colors: vec![BLACK; usize::from(KEYS)],
            sent: vec![None; usize::from(KEYS)],
            held: vec![false; usize::from(KEYS)],
            last: vec![None; usize::from(DIALS)],
            rest: vec![0; usize::from(DIALS)],
            parser: Parser::default(),
            identity_tries: 0,
            identity_deadline: None,
        }
    }

    /// Asks for the deck's identity, one try at a time, and returns its device
    /// id once it answers; `None` while a try is still waiting. Nothing here
    /// blocks past `wait_ms` of one read, so a deck that does not answer cannot
    /// freeze the shared device loop: the next poll carries on. Anything else
    /// read meanwhile is dropped.
    fn identify(&mut self, wait_ms: i32, now: Instant) -> DriverResult<Option<u8>> {
        if self.identity_deadline.is_none() {
            if self.identity_tries >= IDENTITY_TRIES {
                return Err(DriverError::Refused(
                    "the APC40 MKII did not answer its identity request".to_owned(),
                ));
            }
            self.transport.write(&IDENTITY_REQUEST)?;
            self.identity_tries += 1;
            self.identity_deadline = Some(now + IDENTITY_WAIT);
        }
        let mut buf = [0u8; READ_LEN];
        let mut messages = Vec::new();
        for attempt in 0..MAX_HANDSHAKE_MESSAGES {
            let wait = if attempt == 0 { wait_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            messages.clear();
            self.parser
                .feed(buf.get(..read).unwrap_or_default(), &mut messages);
            for message in &messages {
                if let Message::SysEx(bytes) = message {
                    if let Some((device_id, firmware)) = identity_reply(bytes) {
                        tracing::debug!(device_id, %firmware, "ryu-decks: APC40 MKII identified");
                        return Ok(Some(device_id));
                    }
                }
            }
        }
        if self
            .identity_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            // This try went unanswered: the next call asks again, or refuses.
            self.identity_deadline = None;
        }
        Ok(None)
    }

    /// The handshake, the rings off and every ringed knob centred. Whether the
    /// deck is started: `false` while it has not yet answered its identity
    /// request.
    fn ensure_started(&mut self, wait_ms: i32) -> DriverResult<bool> {
        self.ensure_started_at(wait_ms, Instant::now())
    }

    fn ensure_started_at(&mut self, wait_ms: i32, now: Instant) -> DriverResult<bool> {
        if self.started {
            return Ok(true);
        }
        let Some(device_id) = self.identify(wait_ms, now)? else {
            return Ok(false);
        };
        self.transport.write(&introduction(device_id))?;
        for knob in 0..TRACKS {
            self.transport
                .write(&[CONTROL_CHANGE, TRACK_RING_CC + knob, 0])?;
            self.transport
                .write(&[CONTROL_CHANGE, DEVICE_RING_CC + knob, 0])?;
        }
        for knob in 0..TRACKS {
            self.transport
                .write(&[CONTROL_CHANGE, TRACK_KNOB_CC + knob, CENTRE])?;
            self.transport
                .write(&[CONTROL_CHANGE, DEVICE_KNOB_CC + knob, CENTRE])?;
        }
        for last in self.last.iter_mut().take(usize::from(KNOBS)) {
            *last = Some(CENTRE);
        }
        self.sent.fill(None);
        self.started = true;
        Ok(true)
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

    fn turn(dial: u8, delta: i16, events: &mut Vec<Input>) {
        if delta != 0 {
            events.push(Input::DialTurn {
                index: dial,
                delta: i8::try_from(delta.clamp(-127, 127)).unwrap_or(0),
            });
        }
    }

    /// A ringed knob's absolute value: the change from the last is a turn,
    /// and near either end the knob is set back to its centre. Returns the
    /// controller to re-centre.
    fn knob(&mut self, dial: u8, cc: u8, value: u8, events: &mut Vec<Input>) -> Option<u8> {
        let last = self.last.get_mut(usize::from(dial))?;
        let old = last.unwrap_or(value);
        *last = Some(value);
        Self::turn(dial, i16::from(value) - i16::from(old), events);
        if value <= NEAR_END || value >= 127 - NEAR_END {
            *last = Some(CENTRE);
            return Some(cc);
        }
        None
    }

    /// A fader's absolute position: the first seeds it, each later one turns
    /// its dial a step per four units moved.
    fn fader(&mut self, dial: u8, value: u8, events: &mut Vec<Input>) {
        let at = usize::from(dial);
        let (Some(last), Some(rest)) = (self.last.get_mut(at), self.rest.get_mut(at)) else {
            return;
        };
        let Some(old) = last.replace(value) else {
            return;
        };
        let moved = *rest + i16::from(value) - i16::from(old);
        let steps = moved / FADER_STEP;
        *rest = moved - steps * FADER_STEP;
        Self::turn(dial, steps, events);
    }

    /// One controller message; returns a knob to re-centre.
    fn control(&mut self, channel: u8, cc: u8, value: u8, events: &mut Vec<Input>) -> Option<u8> {
        if cc == FADER_CC && channel < TRACKS {
            self.fader(FADER_DIAL + channel, value, events);
            return None;
        }
        if channel != 0 {
            return None;
        }
        match cc {
            TRACK_KNOB_CC..=55 => self.knob(cc - TRACK_KNOB_CC, cc, value, events),
            DEVICE_KNOB_CC..=23 => self.knob(TRACKS + cc - DEVICE_KNOB_CC, cc, value, events),
            TEMPO_CC => {
                Self::turn(TEMPO_DIAL, i16::from(relative(value)), events);
                None
            }
            CUE_CC => {
                Self::turn(CUE_DIAL, i16::from(relative(value)), events);
                None
            }
            MASTER_CC => {
                self.fader(MASTER_DIAL, value, events);
                None
            }
            CROSSFADER_CC => {
                self.fader(CROSSFADER_DIAL, value, events);
                None
            }
            PEDAL_CC => {
                self.press(PEDAL_KEY, value >= 64, events);
                None
            }
            _ => None,
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> Option<u8> {
        // SysEx (a late identity reply) is not used.
        let Message::Channel([status, number, value]) = *message else {
            return None;
        };
        let channel = status & 0x0f;
        match status & 0xf0 {
            NOTE_ON | NOTE_OFF => {
                if let Some(key) = key_of_note(channel, number) {
                    self.press(key, status & 0xf0 == NOTE_ON && value > 0, events);
                }
                None
            }
            CONTROL_CHANGE => self.control(channel, number, value, events),
            _ => None,
        }
    }
}

impl Driver for Apc40Mk2 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
            if !self.ensure_started(timeout_ms)? {
                return Ok(Vec::new());
            }
            self.flush()?;
        }
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
        let mut recentre = Vec::new();
        for message in &messages {
            if let Some(cc) = self.receive(message, &mut events) {
                if !recentre.contains(&cc) {
                    recentre.push(cc);
                }
            }
        }
        for cc in recentre {
            self.transport.write(&[CONTROL_CHANGE, cc, CENTRE])?;
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// One Note On for each light whose velocity changed (every light the
    /// first time).
    fn flush(&mut self) -> DriverResult<()> {
        if !self.ensure_started(0)? {
            return Ok(());
        }
        for key in (0..KEYS).filter(|key| is_lit(*key)) {
            let (Some((channel, note)), Some(color)) =
                (source_of_key(key), self.colors.get(usize::from(key)))
            else {
                continue;
            };
            let state = velocity(key, *color);
            let Some(sent) = self.sent.get_mut(usize::from(key)) else {
                continue;
            };
            if *sent != Some(state) {
                self.transport.write(&[NOTE_ON | channel, note, state])?;
                *sent = Some(state);
            }
        }
        Ok(())
    }

    /// The APC40 MKII has no brightness command.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for Apc40Mk2 {
    /// Turns every light Ryu lit off: the deck would otherwise keep them.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for key in (0..KEYS).filter(|key| is_lit(*key)) {
            if let Some((channel, note)) = source_of_key(key) {
                let _ = self.transport.write(&[NOTE_ON | channel, note, LED_OFF]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    /// The full universal identity reply: `00 19` announces 25 data bytes
    /// (4 version, the device id, 4 serial and 16 manufacturing bytes).
    const REPLY: [u8; 35] = [
        0xf0, 0x7e, 0x05, 0x06, 0x02, 0x47, 0x29, 0x00, 0x19, 0x00, 0x01, 0x00, 0x03, 0x05, 0x00,
        0x00, 0x12, 0x34, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
        0x0d, 0x0e, 0x0f, 0x10, 0xf7,
    ];

    fn open() -> (Apc40Mk2, Recorder) {
        let recorder = Recorder::default();
        let deck = Apc40Mk2::new(by_id(APC40_MK2_ID).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// An opened deck past its handshake, with what that sent cleared.
    fn started() -> (Apc40Mk2, Recorder) {
        let (mut deck, recorder) = open();
        recorder.queue_read(REPLY.to_vec());
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

    fn queue(recorder: &Recorder, messages: &[&[u8]]) {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
    }

    #[test]
    fn the_handshake_sets_ableton_mode_with_the_replied_device_id() {
        let (mut deck, recorder) = open();
        // A stray press and another device's reply come first.
        queue(
            &recorder,
            &[
                &[0x90, 0x20, 0x7f],
                &[
                    0xf0, 0x7e, 0x00, 0x06, 0x02, 0x47, 0x28, 0, 0, 0, 0, 0, 0, 1, 0xf7,
                ],
                &REPLY,
            ],
        );
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(writes[0], vec![0xf0, 0x7e, 0x00, 0x06, 0x01, 0xf7]);
        assert_eq!(
            writes[1],
            vec![0xf0, 0x47, 0x05, 0x29, 0x60, 0x00, 0x04, 0x41, 0x0c, 0x00, 0x00, 0xf7]
        );
        // Rings off, then every ringed knob centred.
        assert_eq!(writes[2], vec![0xb0, 56, 0]);
        assert_eq!(writes[3], vec![0xb0, 24, 0]);
        assert_eq!(writes[17], vec![0xb0, 31, 0]);
        assert_eq!(writes[18], vec![0xb0, 48, 64]);
        assert_eq!(writes[19], vec![0xb0, 16, 64]);
        assert_eq!(writes[33], vec![0xb0, 23, 64]);
        // Then every light: key 0 is the top-left pad, note 0x20.
        assert_eq!(writes[34], vec![0x90, 0x20, 5]);
        assert_eq!(writes[35], vec![0x90, 0x21, 0]);
        assert_eq!(writes.len(), 34 + 110);
        assert_eq!(identity_reply(&REPLY), Some((5, "1.3".to_owned())));
    }

    #[test]
    fn no_identity_reply_refuses_the_deck() {
        let (mut deck, recorder) = open();
        // Each poll returns at once; a try that times out is asked again by a
        // later poll, and the third unanswered try refuses the deck.
        let start = Instant::now();
        assert!(!deck.ensure_started_at(0, start).unwrap());
        assert_eq!(recorder.writes(), vec![IDENTITY_REQUEST.to_vec()]);
        assert!(!deck
            .ensure_started_at(0, start + Duration::from_millis(100))
            .unwrap());
        assert_eq!(recorder.writes().len(), 1);
        // A try expires at its deadline and the next poll asks again.
        for tick in 1..=5 {
            assert!(!deck
                .ensure_started_at(0, start + IDENTITY_WAIT * tick)
                .unwrap());
        }
        assert!(matches!(
            deck.ensure_started_at(0, start + IDENTITY_WAIT * 6),
            Err(DriverError::Refused(_))
        ));
        assert_eq!(recorder.writes(), vec![IDENTITY_REQUEST.to_vec(); 3]);
        // A short reply is not one.
        assert_eq!(identity_reply(&REPLY[..13]), None);
        assert_eq!(identity_reply(&[0xf0]), None);
    }

    #[test]
    fn every_key_maps_to_its_note_and_back() {
        assert_eq!(source_of_key(0), Some((0, 0x20)));
        assert_eq!(source_of_key(7), Some((0, 0x27)));
        assert_eq!(source_of_key(32), Some((0, 0)));
        assert_eq!(source_of_key(39), Some((0, 0x07)));
        assert_eq!(source_of_key(40), Some((0, 82)));
        assert_eq!(source_of_key(52), Some((7, 52)));
        assert_eq!(source_of_key(81), Some((2, 48)));
        assert_eq!(source_of_key(108), Some((0, 93)));
        assert_eq!(source_of_key(110), Some((0, 103)));
        assert_eq!(source_of_key(118), Some((0, 101)));
        assert_eq!(source_of_key(PEDAL_KEY), None);
        let mut seen = std::collections::HashSet::new();
        for key in 0..PEDAL_KEY {
            let (channel, note) = source_of_key(key).unwrap();
            assert!(seen.insert((channel, note)), "{key}");
            assert_eq!(key_of_note(channel, note), Some(key), "{key}");
        }
        // Unused notes and channels.
        assert_eq!(key_of_note(0, 92), None);
        assert_eq!(key_of_note(1, 0x20), None);
        assert_eq!(key_of_note(8, 48), None);
        assert_eq!(key_of_note(1, 82), None);
        assert_eq!(key_of_note(0, 127), None);
    }

    #[test]
    fn presses_and_both_kinds_of_release() {
        let (mut deck, recorder) = started();
        queue(
            &recorder,
            &[
                &[0x90, 0x20, 0x7f],
                &[0x80, 0x20, 0x7f],
                &[0x90, 0x07, 0x7f],
                &[0x90, 0x07, 0x00],
                // Arm on track 3, solo on track 8.
                &[0x92, 48, 0x7f],
                &[0x97, 49, 0x7f],
                &[0x90, 98, 0x7f],
                // Foot pedal.
                &[0xb0, 64, 0x7f],
                &[0xb0, 64, 0x00],
                // Unknown note, a channel with no tracks, short input.
                &[0x90, 92, 0x7f],
                &[0x98, 48, 0x7f],
                &[0x90],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(39, true),
                key(39, false),
                key(81, true),
                key(78, true),
                key(115, true),
                key(119, true),
                key(119, false),
            ]
        );
    }

    #[test]
    fn knobs_turn_and_recentre_near_their_ends() {
        let (mut deck, recorder) = started();
        queue(
            &recorder,
            &[
                &[0xb0, 48, 66],
                &[0xb0, 48, 63],
                &[0xb0, 48, 5],
                // Device knob 8.
                &[0xb0, 23, 70],
                // Tempo and Cue: two's complement.
                &[0xb0, 13, 0x01],
                &[0xb0, 13, 0x7f],
                &[0xb0, 47, 0x7d],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, 2),
                turn(0, -3),
                turn(0, -58),
                turn(15, 6),
                turn(16, 1),
                turn(16, -1),
                turn(17, -3),
            ]
        );
        assert_eq!(recorder.writes(), vec![vec![0xb0, 0x30, 0x40]]);
        // Centred: the next reading counts from 64.
        queue(&recorder, &[&[0xb0, 48, 65]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(0, 1)]);
        assert_eq!(relative(0x7f), -1);
        assert_eq!(relative(0x01), 1);
        assert_eq!(relative(0x40), -64);
        assert_eq!(relative(0x3f), 63);
        assert_eq!(relative(0x41), -63);
    }

    #[test]
    fn faders_turn_dials_after_their_first_value() {
        let (mut deck, recorder) = started();
        queue(
            &recorder,
            &[
                &[0xb0, 7, 0],
                &[0xb0, 7, 6],
                &[0xb0, 7, 8],
                &[0xb7, 7, 127],
                &[0xb7, 7, 100],
                &[0xb0, 14, 10],
                &[0xb0, 14, 2],
                &[0xb0, 15, 64],
                &[0xb0, 15, 72],
                // Not a fader on this channel.
                &[0xb8, 7, 0],
                &[0xb1, 14, 0],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(18, 1),
                turn(18, 1),
                turn(25, -6),
                turn(26, -2),
                turn(27, 2)
            ]
        );
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn lights_send_only_changes_with_each_kind_of_velocity() {
        let (mut deck, recorder) = started();
        deck.set_face(39, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(44, Some(&lit([0, 0xa9, 0xff]))).unwrap();
        deck.set_face(45, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(81, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(87, Some(&lit([255, 128, 0]))).unwrap();
        deck.set_face(88, Some(&lit([0, 40, 255]))).unwrap();
        // Unlit keys send nothing.
        deck.set_face(53, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(119, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x90, 0x07, 45],
                vec![0x90, 86, 37],
                vec![0x90, 52, 1],
                vec![0x92, 48, 127],
                vec![0x90, 66, 1],
                vec![0x91, 66, 2],
            ]
        );
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 6);
        deck.set_face(39, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[6], vec![0x90, 0x07, 0]);
        deck.set_brightness(10).unwrap();
        assert_eq!(recorder.writes().len(), 7);
    }

    #[test]
    fn the_palette_picks_the_nearest_entry() {
        assert_eq!(palette_index(BLACK), 0);
        assert_eq!(palette_index([255, 255, 255]), 3);
        assert_eq!(palette_index([255, 0, 0]), 5);
        assert_eq!(palette_index([255, 84, 0]), 9);
        assert_eq!(palette_index([255, 255, 0]), 13);
        assert_eq!(palette_index([0, 255, 0]), 21);
        assert_eq!(palette_index([0, 0, 255]), 45);
        assert_eq!(palette_index([255, 0, 255]), 53);
        assert_eq!(PALETTE[37], 0x00_a9ff);
    }

    #[test]
    fn letting_go_turns_every_light_off() {
        let (deck, recorder) = started();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 110);
        assert!(writes.iter().all(|write| write[2] == 0));
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn its_ports_match_in_any_case() {
        let model = by_id(APC40_MK2_ID).unwrap();
        let cases = [
            ("APC40 mkII", false, true),
            ("APC40 mkII:APC40 mkII MIDI 1 24:0", false, true),
            ("Akai APC40 MkII", false, true),
            ("2- APC40 mkII", true, true),
            ("MIDIIN2 (APC40 mkII)", true, false),
            // The first APC40 and the APC minis.
            ("Akai APC40", false, false),
            ("APC MINI", false, false),
            ("APC mini mk2 Control", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(APC40_MK2_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_places_every_key() {
        let model = by_id(APC40_MK2_ID).unwrap();
        let view = ModelView::from(model);
        let cells = view.key_cells.unwrap();
        assert_eq!(cells.len(), 120);
        let unique: std::collections::HashSet<_> = cells.iter().collect();
        assert_eq!(unique.len(), 120);
        let grid = u16::from(model.rows) * u16::from(model.cols);
        assert!(cells.iter().all(|cell| *cell < grid));
        assert_eq!(
            (cells[0], cells[40], cells[53], cells[62], cells[110], cells[119]),
            (0, 8, 5 * 9 + 8, 6 * 9 + 8, 11 * 9 + 8, 13 * 9 + 8)
        );
        let names = view.key_names.unwrap();
        assert_eq!(
            (names[0], names[44], names[53], names[119]),
            ("Clip 1-1", "Scene 5", "Stop All Clips", "Foot Pedal")
        );
        let lit_keys = view.lit_keys.unwrap();
        assert_eq!(lit_keys.len(), 110);
        assert!(!lit_keys.contains(&53) && !lit_keys.contains(&111) && lit_keys.contains(&110));
        assert_eq!(view.key_light_colors, "rgb");
        assert_eq!(view.dial_lights, DialLights::None);
        assert!(view.key_lights && !view.brightness);
        assert_eq!(view.deck_lights, 0);
    }
}
