//! Akai Professional's first APC40 (MK1) and the APC20, over class-compliant
//! USB MIDI ([`super::midi`]), in their Ableton Live mode.
//!
//! Each lists one MIDI port each way. Ableton suggests the names `Akai APC40`
//! and `Akai APC20` (the exact name each host gives the port is not in the
//! references), so [`port_matches`] takes a name containing `apc40` (but not
//! `mkii`, `mk2` or `mk ii`, which is the APC40 MKII's) or `apc20` in any
//! case, and on Windows only the first interface.
//!
//! Start-up, queued and written from the poll loop so it never waits: the
//! universal identity request (`F0 7E 00 06 01 F7`), then the introduction
//! that selects Ableton Live mode (`F0 47 <device id> <model> 60 00 04 41
//! <host version: 3 bytes> F7`, model `73` for the APC40 and `7B` for the
//! APC20) sent at once to every device id (`7F`). An identity reply (`F0 7E
//! <id> 06 02 47 <model> ...`, whose byte 13 is the deck's device id) sends
//! the introduction again with that id and repaints every light; no reply is
//! needed. On the APC40 the 16 knob rings are then turned off and every ringed
//! knob centred. Each SysEx message is followed by a short gap before the next
//! write goes out.
//!
//! In (MIDI channel `n` is the status' low nibble; notes 48-53 are reused
//! across channels 0-7, one per track, so a message is always decoded by
//! channel and note together): the 5 x 8 clip matrix is note `53 + scene` on
//! channel `track`, the top row note 53; scene launch 82-86; clip stop 52,
//! track select 51, activator 50, solo 49 and arm 48 on channels 0-7; master
//! select 80. On the APC40 note 81 is Stop All Clips, the device buttons are
//! 58-65 and Pan, Send A-C, Play, Stop, Record, the arrows (up, down, right,
//! left), Shift, Tap Tempo and Nudge + / - are 87-101; on the APC20 note 81 is
//! Shift. Note On above velocity 0 presses; Note Off or velocity 0 releases.
//! The foot pedals are `CC 64` (and `CC 67` on the APC40), pressed from 64.
//!
//! Dials are Control Change. The APC40's 8 track knobs (`CC 48-55`) and 8
//! device knobs (`CC 16-23`) are endless but report an absolute 0-127 the host
//! can overwrite. Ableton moves the track knobs to channels 1-3 for the send
//! banks, where the deck keeps a value of its own, so Ryu tracks each knob per
//! channel: its first value on a channel only seeds it, then the change is a
//! dial step, and near either end Ryu writes 64 back on that channel so it
//! keeps turning. Cue level (`CC 47`) is relative, two's complement. The
//! channel faders (`CC 7` on channels 0-7; on the APC20 any of `CC 0-7`, as its
//! slider mode picks), master (`CC 14`) and the APC40's crossfader (`CC 15`)
//! are absolute and not motorised: each one's first value only seeds it, then
//! it turns its dial a step per four units.
//!
//! Out: Note On on each button's own channel and note. The clip pads are off
//! (0), green (1), red (3) or amber (5), the nearest to the key's colour (the
//! blinking values are never sent); scene launch and clip stop are 0 off and 1
//! on; every other light is 0 off and 127 on. Stop All Clips, the arrows,
//! Shift, Tap Tempo, Nudge and the pedals have no light. There is no
//! brightness command and no keep-alive. Only changed lights are sent, and Ryu
//! turns them all off when it lets go (leaving the deck in Ableton mode: the
//! command for its generic mode is not in the references).
//!
//! Facts from Ableton's APC40, APC20 and _APC Remote Scripts as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for
//! note and controller numbers, the handshake, light values and the USB ids
//! only) and DrivenByMoss (LGPL-3.0; read for the broadcast introduction, the
//! pad grid, light values and which buttons are lit); no code was copied.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::akai_apc_mini::pad_state;
use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const APC40_MK1_ID: &str = "akai-apc40-mk1";
pub const APC20_ID: &str = "akai-apc20";

pub const APC40_KEYS: u8 = 112;
pub const APC40_DIALS: u8 = 27;
pub const APC20_KEYS: u8 = 88;
pub const APC20_DIALS: u8 = 10;

const TRACKS: u8 = 8;
const MATRIX_KEYS: u8 = 40;
const SCENE_KEY: u8 = 40;
const STOP_KEY: u8 = 45;
/// APC40 key numbers; the APC20's are these less one from track select on.
const STOP_ALL_KEY: u8 = 53;
const SELECT_KEY: u8 = 54;
const MASTER_KEY: u8 = 62;
const ACTIVATOR_KEY: u8 = 63;
const SOLO_KEY: u8 = 71;
const ARM_KEY: u8 = 79;
const DEVICE_KEY: u8 = 87;
const PANEL_KEY: u8 = 95;
/// The first APC40 panel key with no light (Up).
const UP_KEY: u8 = 102;
const TAP_KEY: u8 = 107;
const FOOT_1_KEY: u8 = 110;
const FOOT_2_KEY: u8 = 111;
const APC20_SHIFT_KEY: u8 = 86;
const APC20_PEDAL_KEY: u8 = 87;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;

const MATRIX_NOTE: u8 = 53;
const SCENE_NOTE: u8 = 82;
const STOP_NOTE: u8 = 52;
const STOP_ALL_NOTE: u8 = 81;
const SELECT_NOTE: u8 = 51;
const MASTER_NOTE: u8 = 80;
const ACTIVATOR_NOTE: u8 = 50;
const SOLO_NOTE: u8 = 49;
const ARM_NOTE: u8 = 48;
const DEVICE_NOTE: u8 = 58;
const PANEL_NOTE: u8 = 87;
const FOOT_1_CC: u8 = 64;
const FOOT_2_CC: u8 = 67;

const TRACK_KNOB_CC: u8 = 48;
const TRACK_RING_CC: u8 = 56;
const DEVICE_KNOB_CC: u8 = 16;
const DEVICE_RING_CC: u8 = 24;
const CUE_CC: u8 = 47;
const FADER_CC: u8 = 7;
const MASTER_CC: u8 = 14;
const CROSSFADER_CC: u8 = 15;

/// APC40 dials: 0-15 the ringed knobs, 16 Cue, 17-24 the channel faders, 25
/// master and 26 the crossfader.
const KNOBS: u8 = 16;
const APC40_CUE_DIAL: u8 = 16;
const APC40_FADER_DIAL: u8 = 17;
const APC40_MASTER_DIAL: u8 = 25;
const APC40_CROSSFADER_DIAL: u8 = 26;
/// APC20 dials: 0-7 the channel faders, 8 master and 9 Cue.
const APC20_MASTER_DIAL: u8 = 8;
const APC20_CUE_DIAL: u8 = 9;

/// A ringed knob's centre, written back so it keeps turning.
const CENTRE: u8 = 64;
/// Values this close to either end re-centre the knob.
const NEAR_END: u8 = 8;
/// Fader units (of 128) per dial step: 32 steps from end to end.
const FADER_STEP: i16 = 4;
const CHANNELS: usize = 16;

const SYSEX_START: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
const IDENTITY_REQUEST: [u8; 6] = [0xf0, 0x7e, 0x00, 0x06, 0x01, 0xf7];
const AKAI: u8 = 0x47;
const APC40_MODEL: u8 = 0x73;
const APC20_MODEL: u8 = 0x7b;
/// Every device on the port.
const BROADCAST: u8 = 0x7f;
const INTRODUCTION: u8 = 0x60;
const ABLETON_MODE: u8 = 0x41;
/// The host version the introduction reports (any 7-bit values).
const HOST_VERSION: [u8; 3] = [0x08, 0x02, 0x01];

/// The gap after each SysEx message before the next write goes out.
const PACE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(2)
};

const LED_OFF: u8 = 0;
const LED_ON: u8 = 127;
const SCENE_ON: u8 = 1;
/// A light is on while a channel reaches this.
const DARK: u8 = 32;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const APC40_NAMES: [&str; APC40_KEYS as usize] = [
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
    "Activator 1",
    "Activator 2",
    "Activator 3",
    "Activator 4",
    "Activator 5",
    "Activator 6",
    "Activator 7",
    "Activator 8",
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
    "Clip/Track",
    "Device On/Off",
    "Device Left",
    "Device Right",
    "Detail View",
    "Rec Quantization",
    "MIDI Overdub",
    "Metronome",
    "Pan",
    "Send A",
    "Send B",
    "Send C",
    "Play",
    "Stop",
    "Record",
    "Up",
    "Down",
    "Right",
    "Left",
    "Shift",
    "Tap Tempo",
    "Nudge +",
    "Nudge -",
    "Footswitch 1",
    "Footswitch 2",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Apc40,
    Apc20,
}

impl Kind {
    const fn of(model: &DeviceModel) -> Option<Self> {
        match model.protocol {
            Protocol::AkaiApc40Mk1Midi => Some(Self::Apc40),
            Protocol::AkaiApc20Midi => Some(Self::Apc20),
            _ => None,
        }
    }

    const fn keys(self) -> u8 {
        match self {
            Self::Apc40 => APC40_KEYS,
            Self::Apc20 => APC20_KEYS,
        }
    }

    const fn dials(self) -> u8 {
        match self {
            Self::Apc40 => APC40_DIALS,
            Self::Apc20 => APC20_DIALS,
        }
    }

    const fn model_byte(self) -> u8 {
        match self {
            Self::Apc40 => APC40_MODEL,
            Self::Apc20 => APC20_MODEL,
        }
    }
}

/// Whether a normalised port name is a first APC40's (`apc40` in any case,
/// never an MKII's) or an APC20's, and on Windows the first interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    let Some(kind) = Kind::of(model) else {
        return false;
    };
    if name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.to_ascii_lowercase();
    match kind {
        Kind::Apc40 => {
            base.contains("apc40")
                && !["mkii", "mk2", "mk ii"]
                    .iter()
                    .any(|mark| base.contains(mark))
        }
        Kind::Apc20 => base.contains("apc20"),
    }
}

/// The legend on each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    Kind::of(model).map(|kind| match kind {
        Kind::Apc40 => APC40_NAMES.to_vec(),
        Kind::Apc20 => APC40_NAMES
            .iter()
            .take(usize::from(STOP_ALL_KEY))
            .chain(
                APC40_NAMES
                    .get(usize::from(SELECT_KEY)..usize::from(DEVICE_KEY))
                    .unwrap_or_default(),
            )
            .copied()
            .chain(["Shift", "Foot Pedal"])
            .collect(),
    })
}

/// How a key reaches Ryu: a note on a channel, or a pedal's controller on
/// channel 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Source {
    Note(u8, u8),
    Pedal(u8),
}

/// The APC40's source of each key.
const fn apc40_source(key: u8) -> Option<Source> {
    let (channel, note) = match key {
        0..MATRIX_KEYS => (key % TRACKS, MATRIX_NOTE + key / TRACKS),
        SCENE_KEY..STOP_KEY => (0, SCENE_NOTE + key - SCENE_KEY),
        STOP_KEY..STOP_ALL_KEY => (key - STOP_KEY, STOP_NOTE),
        STOP_ALL_KEY => (0, STOP_ALL_NOTE),
        SELECT_KEY..MASTER_KEY => (key - SELECT_KEY, SELECT_NOTE),
        MASTER_KEY => (0, MASTER_NOTE),
        ACTIVATOR_KEY..SOLO_KEY => (key - ACTIVATOR_KEY, ACTIVATOR_NOTE),
        SOLO_KEY..ARM_KEY => (key - SOLO_KEY, SOLO_NOTE),
        ARM_KEY..DEVICE_KEY => (key - ARM_KEY, ARM_NOTE),
        DEVICE_KEY..PANEL_KEY => (0, DEVICE_NOTE + key - DEVICE_KEY),
        PANEL_KEY..FOOT_1_KEY => (0, PANEL_NOTE + key - PANEL_KEY),
        FOOT_1_KEY => return Some(Source::Pedal(FOOT_1_CC)),
        FOOT_2_KEY => return Some(Source::Pedal(FOOT_2_CC)),
        _ => return None,
    };
    Some(Source::Note(channel, note))
}

/// A key's source on a deck.
const fn source(kind: Kind, key: u8) -> Option<Source> {
    match kind {
        Kind::Apc40 => apc40_source(key),
        Kind::Apc20 => match key {
            0..STOP_ALL_KEY => apc40_source(key),
            STOP_ALL_KEY..APC20_SHIFT_KEY => apc40_source(key + 1),
            APC20_SHIFT_KEY => Some(Source::Note(0, STOP_ALL_NOTE)),
            APC20_PEDAL_KEY => Some(Source::Pedal(FOOT_1_CC)),
            _ => None,
        },
    }
}

/// The key a source presses.
fn key_of(kind: Kind, wanted: Source) -> Option<u8> {
    (0..kind.keys()).find(|key| source(kind, *key) == Some(wanted))
}

/// Whether a key has a light the references drive.
const fn is_lit(kind: Kind, key: u8) -> bool {
    match kind {
        Kind::Apc40 => key < UP_KEY && key != STOP_ALL_KEY,
        Kind::Apc20 => key < APC20_SHIFT_KEY,
    }
}

/// Every key but Stop All Clips, the arrows, Shift, Tap Tempo, Nudge and the
/// pedals.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    Kind::of(model).map(|kind| (0..kind.keys()).filter(|key| is_lit(kind, *key)).collect())
}

/// Each key's (row, column). Both put the clip matrix top left with the scene
/// column at its right and a row for clip stop under it. The APC40 (13 x 9)
/// then has rows for track select (Master at the right), activator, solo, arm,
/// the device buttons, Pan to Record with Tap Tempo, and the arrows, Shift,
/// Nudge and the footswitches; its Stop All Clips ends the clip stop row. The
/// APC20 (10 x 9) has Shift after clip stop, Master after track select and
/// the pedal after the activators, then solo and arm.
const fn cell(kind: Kind, key: u8) -> (u8, u8) {
    match key {
        0..MATRIX_KEYS => (key / TRACKS, key % TRACKS),
        SCENE_KEY..STOP_KEY => (key - SCENE_KEY, TRACKS),
        STOP_KEY..STOP_ALL_KEY => (5, key - STOP_KEY),
        _ => match kind {
            Kind::Apc40 => match key {
                STOP_ALL_KEY => (5, TRACKS),
                SELECT_KEY..=MASTER_KEY => (6, key - SELECT_KEY),
                ACTIVATOR_KEY..SOLO_KEY => (7, key - ACTIVATOR_KEY),
                SOLO_KEY..ARM_KEY => (8, key - SOLO_KEY),
                ARM_KEY..DEVICE_KEY => (9, key - ARM_KEY),
                DEVICE_KEY..PANEL_KEY => (10, key - DEVICE_KEY),
                PANEL_KEY..UP_KEY => (11, key - PANEL_KEY),
                TAP_KEY => (11, 7),
                UP_KEY..TAP_KEY => (12, key - UP_KEY),
                _ => (12, key - UP_KEY - 1),
            },
            Kind::Apc20 => match key {
                APC20_SHIFT_KEY => (5, TRACKS),
                APC20_PEDAL_KEY => (7, TRACKS),
                // Track select to arm sit as on the APC40, a key earlier.
                _ => cell(Kind::Apc40, key + 1),
            },
        },
    }
}

/// Each key's cell in the model's grid.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    let kind = Kind::of(model)?;
    let cols = u16::from(model.cols);
    Some(
        (0..kind.keys())
            .map(|key| {
                let (row, col) = cell(kind, key);
                u16::from(row) * cols + u16::from(col)
            })
            .collect(),
    )
}

/// The velocity that shows a colour on a key's light.
fn velocity(key: u8, color: [u8; 3]) -> u8 {
    if key < MATRIX_KEYS {
        return pad_state(color);
    }
    if color.iter().all(|channel| *channel < DARK) {
        return LED_OFF;
    }
    if key < STOP_ALL_KEY {
        SCENE_ON
    } else {
        LED_ON
    }
}

/// A two's-complement relative value as a step.
fn relative(value: u8) -> i16 {
    let value = i16::from(value & 0x7f);
    if value < 64 {
        value
    } else {
        value - 128
    }
}

/// The identity reply from this model: its device id and firmware.
fn identity_reply(model: u8, message: &[u8]) -> Option<(u8, String)> {
    let header = message.get(..7)?;
    if header != [SYSEX_START, 0x7e, header[2], 0x06, 0x02, AKAI, model] {
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
fn introduction(model: u8, device_id: u8) -> Vec<u8> {
    let [major, minor, bugfix] = HOST_VERSION;
    vec![
        SYSEX_START,
        AKAI,
        device_id,
        model,
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

pub struct Apc40Mk1 {
    kind: Kind,
    transport: Box<dyn Transport>,
    /// The start-up is queued.
    started: bool,
    pace: Duration,
    /// Writes waiting their turn, so the shared poll loop never sleeps.
    outbox: VecDeque<Vec<u8>>,
    /// When the next write may go out after a SysEx message.
    next_send: Instant,
    colors: Vec<[u8; 3]>,
    /// The velocity each key's light was last queued; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    /// Each ringed knob's last value on each channel (channel * 16 + knob),
    /// `None` until it is seen there.
    knobs: Vec<Option<u8>>,
    /// Each fader's last value, `None` until it first moves.
    faders: Vec<Option<u8>>,
    /// Fader movement not yet turned into a whole step.
    rest: Vec<i16>,
    parser: Parser,
}

impl Apc40Mk1 {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model).unwrap_or(Kind::Apc40);
        let keys = usize::from(kind.keys());
        let dials = usize::from(kind.dials());
        Self {
            kind,
            transport,
            started: false,
            pace: PACE,
            outbox: VecDeque::new(),
            next_send: Instant::now(),
            colors: vec![BLACK; keys],
            sent: vec![None; keys],
            held: vec![false; keys],
            knobs: vec![None; CHANNELS * usize::from(KNOBS)],
            faders: vec![None; dials],
            rest: vec![0; dials],
            parser: Parser::default(),
        }
    }

    /// Writes the queued messages whose turn has come: a SysEx message holds
    /// the next write back by the pace, without ever sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        while let Some(message) = self.outbox.front() {
            let now = Instant::now();
            if now < self.next_send {
                break;
            }
            self.transport.write(message)?;
            if message.first() == Some(&SYSEX_START) {
                self.next_send = now + self.pace;
            }
            self.outbox.pop_front();
        }
        Ok(())
    }

    /// The introduction for a device id, then (APC40) the rings off and the
    /// ringed knobs centred on channel 0; every light is repainted next.
    fn queue_setup(&mut self, device_id: u8) {
        self.outbox
            .push_back(introduction(self.kind.model_byte(), device_id));
        if self.kind == Kind::Apc40 {
            for ring in (TRACK_RING_CC..TRACK_RING_CC + TRACKS)
                .chain(DEVICE_RING_CC..DEVICE_RING_CC + TRACKS)
            {
                self.outbox.push_back(vec![CONTROL_CHANGE, ring, 0]);
            }
            for knob in (TRACK_KNOB_CC..TRACK_KNOB_CC + TRACKS)
                .chain(DEVICE_KNOB_CC..DEVICE_KNOB_CC + TRACKS)
            {
                self.outbox.push_back(vec![CONTROL_CHANGE, knob, CENTRE]);
            }
            for last in self.knobs.iter_mut().take(usize::from(KNOBS)) {
                *last = Some(CENTRE);
            }
        }
        self.sent.fill(None);
    }

    /// Queues the identity request and the broadcast introduction once.
    fn ensure_started(&mut self) {
        if self.started {
            return;
        }
        self.outbox.push_back(IDENTITY_REQUEST.to_vec());
        self.queue_setup(BROADCAST);
        self.started = true;
    }

    /// Queues a Note On for each light whose velocity changed.
    fn queue_lights(&mut self) {
        for key in (0..self.kind.keys()).filter(|key| is_lit(self.kind, *key)) {
            let (Some(Source::Note(channel, note)), Some(color)) =
                (source(self.kind, key), self.colors.get(usize::from(key)))
            else {
                continue;
            };
            let state = velocity(key, *color);
            let Some(sent) = self.sent.get_mut(usize::from(key)) else {
                continue;
            };
            if *sent != Some(state) {
                *sent = Some(state);
                self.outbox.push_back(vec![NOTE_ON | channel, note, state]);
            }
        }
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

    /// A ringed knob's absolute value on a channel: the first there seeds it,
    /// later ones turn by the change, and near either end it is set back to
    /// its centre. Returns whether to re-centre it.
    fn knob(&mut self, channel: u8, knob: u8, value: u8, events: &mut Vec<Input>) -> bool {
        let at = usize::from(channel) * usize::from(KNOBS) + usize::from(knob);
        let Some(last) = self.knobs.get_mut(at) else {
            return false;
        };
        if let Some(old) = last.replace(value) {
            Self::turn(knob, i16::from(value) - i16::from(old), events);
        }
        if value <= NEAR_END || value >= 127 - NEAR_END {
            *last = Some(CENTRE);
            return true;
        }
        false
    }

    /// A fader's absolute position: the first seeds it, each later one turns
    /// its dial a step per four units moved.
    fn fader(&mut self, dial: u8, value: u8, events: &mut Vec<Input>) {
        let at = usize::from(dial);
        let (Some(last), Some(rest)) = (self.faders.get_mut(at), self.rest.get_mut(at)) else {
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

    /// One controller message; returns a knob's (channel, controller) to
    /// re-centre.
    fn control(
        &mut self,
        channel: u8,
        cc: u8,
        value: u8,
        events: &mut Vec<Input>,
    ) -> Option<(u8, u8)> {
        let apc40 = self.kind == Kind::Apc40;
        if channel < TRACKS {
            let fader = match self.kind {
                Kind::Apc40 => (cc == FADER_CC).then_some(APC40_FADER_DIAL + channel),
                Kind::Apc20 => (cc <= FADER_CC).then_some(channel),
            };
            if let Some(dial) = fader {
                self.fader(dial, value, events);
                return None;
            }
        }
        if apc40 {
            let knob = match cc {
                TRACK_KNOB_CC..=55 => Some(cc - TRACK_KNOB_CC),
                DEVICE_KNOB_CC..=23 => Some(TRACKS + cc - DEVICE_KNOB_CC),
                _ => None,
            };
            if let Some(knob) = knob {
                return self
                    .knob(channel, knob, value, events)
                    .then_some((channel, cc));
            }
        }
        if channel != 0 {
            return None;
        }
        let (cue, master) = if apc40 {
            (APC40_CUE_DIAL, APC40_MASTER_DIAL)
        } else {
            (APC20_CUE_DIAL, APC20_MASTER_DIAL)
        };
        match cc {
            CUE_CC => Self::turn(cue, relative(value), events),
            MASTER_CC => self.fader(master, value, events),
            CROSSFADER_CC if apc40 => self.fader(APC40_CROSSFADER_DIAL, value, events),
            _ => {
                if let Some(key) = key_of(self.kind, Source::Pedal(cc)) {
                    self.press(key, value >= 64, events);
                }
            }
        }
        None
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> Option<(u8, u8)> {
        match message {
            Message::SysEx(bytes) => {
                let model = self.kind.model_byte();
                if let Some((device_id, firmware)) = identity_reply(model, bytes) {
                    tracing::debug!(device_id, %firmware, "ryu-decks: APC identified");
                    self.queue_setup(device_id);
                }
                None
            }
            Message::Channel([status, number, value]) => {
                let channel = status & 0x0f;
                match status & 0xf0 {
                    NOTE_ON | NOTE_OFF => {
                        if let Some(key) = key_of(self.kind, Source::Note(channel, *number)) {
                            self.press(key, status & 0xf0 == NOTE_ON && *value > 0, events);
                        }
                        None
                    }
                    CONTROL_CHANGE => self.control(channel, *number, *value, events),
                    _ => None,
                }
            }
        }
    }
}

impl Driver for Apc40Mk1 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
            self.ensure_started();
            self.queue_lights();
        }
        self.pump()?;
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
            if let Some(knob) = self.receive(message, &mut events) {
                if !recentre.contains(&knob) {
                    recentre.push(knob);
                }
            }
        }
        for (channel, cc) in recentre {
            self.outbox
                .push_back(vec![CONTROL_CHANGE | channel, cc, CENTRE]);
        }
        // A light a late identity reply asked to repaint.
        self.queue_lights();
        self.pump()?;
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// One Note On for each light whose velocity changed (every light the
    /// first time), behind the start-up.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started();
        self.queue_lights();
        self.pump()
    }

    /// Neither deck has a brightness command.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for Apc40Mk1 {
    /// Turns every light off at once (no wait): the deck would otherwise keep
    /// them.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        self.outbox.clear();
        for key in (0..self.kind.keys()).filter(|key| is_lit(self.kind, *key)) {
            if let Some(Source::Note(channel, note)) = source(self.kind, key) {
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

    /// A universal identity reply: device id 5, firmware 1.3.
    fn reply(model: u8) -> Vec<u8> {
        vec![
            0xf0, 0x7e, 0x05, 0x06, 0x02, 0x47, model, 0x00, 0x19, 0x00, 0x01, 0x00, 0x03, 0x05,
            0x00, 0x00, 0x12, 0x34, 0xf7,
        ]
    }

    fn open(id: &str) -> (Apc40Mk1, Recorder) {
        let recorder = Recorder::default();
        let mut deck = Apc40Mk1::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        deck.pace = Duration::ZERO;
        (deck, recorder)
    }

    /// An opened deck past its start-up, with what that sent cleared.
    fn started(id: &str) -> (Apc40Mk1, Recorder) {
        let (mut deck, recorder) = open(id);
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
    fn apc40_start_up_broadcasts_ableton_mode_without_waiting() {
        let (mut deck, recorder) = open(APC40_MK1_ID);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(writes[0], vec![0xf0, 0x7e, 0x00, 0x06, 0x01, 0xf7]);
        assert_eq!(
            writes[1],
            vec![0xf0, 0x47, 0x7f, 0x73, 0x60, 0x00, 0x04, 0x41, 0x08, 0x02, 0x01, 0xf7]
        );
        // Rings off, then every ringed knob centred.
        assert_eq!(writes[2], vec![0xb0, 56, 0]);
        assert_eq!(writes[9], vec![0xb0, 63, 0]);
        assert_eq!(writes[10], vec![0xb0, 24, 0]);
        assert_eq!(writes[17], vec![0xb0, 31, 0]);
        assert_eq!(writes[18], vec![0xb0, 48, 64]);
        assert_eq!(writes[34 - 1], vec![0xb0, 23, 64]);
        // Then every light: key 0 is the top-left pad, note 53 on channel 0.
        assert_eq!(writes[34], vec![0x90, 53, 3]);
        assert_eq!(writes[35], vec![0x91, 53, 0]);
        assert_eq!(writes.len(), 34 + 101);
    }

    #[test]
    fn the_apc20_has_no_rings_and_an_identity_reply_reintroduces_it() {
        let (mut deck, recorder) = open(APC20_ID);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[0], IDENTITY_REQUEST.to_vec());
        assert_eq!(
            writes[1],
            vec![0xf0, 0x47, 0x7f, 0x7b, 0x60, 0x00, 0x04, 0x41, 0x08, 0x02, 0x01, 0xf7]
        );
        assert_eq!(writes[2], vec![0x90, 53, 0]);
        assert_eq!(writes.len(), 2 + 86);
        recorder.0.lock().unwrap().writes.clear();
        // Another model's reply is ignored; this one's resends the
        // introduction with its device id and repaints every light.
        queue(&recorder, &[&reply(APC40_MODEL), &reply(APC20_MODEL)]);
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(
            writes[0],
            vec![0xf0, 0x47, 0x05, 0x7b, 0x60, 0x00, 0x04, 0x41, 0x08, 0x02, 0x01, 0xf7]
        );
        assert_eq!(writes.len(), 1 + 86);
        assert_eq!(
            identity_reply(APC20_MODEL, &reply(APC20_MODEL)),
            Some((5, "1.3".to_owned()))
        );
        assert_eq!(identity_reply(APC20_MODEL, &reply(APC20_MODEL)[..13]), None);
        assert_eq!(identity_reply(APC20_MODEL, &[0xf0]), None);
    }

    #[test]
    fn writes_after_sysex_wait_their_pace_without_sleeping() {
        let (mut deck, recorder) = open(APC40_MK1_ID);
        deck.pace = Duration::from_secs(60);
        let begun = Instant::now();
        deck.poll(0).unwrap();
        deck.flush().unwrap();
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert_eq!(recorder.writes(), vec![IDENTITY_REQUEST.to_vec()]);
        deck.pace = Duration::ZERO;
        deck.next_send = Instant::now();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 1 + 33 + 101);
        // Letting go does not wait either.
        deck.pace = Duration::from_secs(60);
        let begun = Instant::now();
        drop(deck);
        assert!(begun.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn every_apc40_key_maps_to_its_source_and_back() {
        let apc40 = Kind::Apc40;
        assert_eq!(source(apc40, 0), Some(Source::Note(0, 53)));
        assert_eq!(source(apc40, 7), Some(Source::Note(7, 53)));
        assert_eq!(source(apc40, 39), Some(Source::Note(7, 57)));
        assert_eq!(source(apc40, 44), Some(Source::Note(0, 86)));
        assert_eq!(source(apc40, 52), Some(Source::Note(7, 52)));
        assert_eq!(source(apc40, 53), Some(Source::Note(0, 81)));
        assert_eq!(source(apc40, 62), Some(Source::Note(0, 80)));
        assert_eq!(source(apc40, 81), Some(Source::Note(2, 48)));
        assert_eq!(source(apc40, 94), Some(Source::Note(0, 65)));
        assert_eq!(source(apc40, 95), Some(Source::Note(0, 87)));
        assert_eq!(source(apc40, 106), Some(Source::Note(0, 98)));
        assert_eq!(source(apc40, 109), Some(Source::Note(0, 101)));
        assert_eq!(source(apc40, 110), Some(Source::Pedal(64)));
        assert_eq!(source(apc40, 111), Some(Source::Pedal(67)));
        assert_eq!(source(apc40, 112), None);
        for kind in [Kind::Apc40, Kind::Apc20] {
            let mut seen = std::collections::HashSet::new();
            for key in 0..kind.keys() {
                let found = source(kind, key).unwrap();
                assert!(seen.insert(found), "{kind:?} {key}");
                assert_eq!(key_of(kind, found), Some(key), "{kind:?} {key}");
            }
        }
        // The APC20: note 81 is Shift, one pedal.
        assert_eq!(source(Kind::Apc20, 53), Some(Source::Note(0, 51)));
        assert_eq!(source(Kind::Apc20, 61), Some(Source::Note(0, 80)));
        assert_eq!(source(Kind::Apc20, 85), Some(Source::Note(7, 48)));
        assert_eq!(source(Kind::Apc20, 86), Some(Source::Note(0, 81)));
        assert_eq!(source(Kind::Apc20, 87), Some(Source::Pedal(64)));
        assert_eq!(key_of(Kind::Apc20, Source::Note(0, 58)), None);
        assert_eq!(key_of(Kind::Apc20, Source::Pedal(67)), None);
        // Unused notes and channels.
        assert_eq!(key_of(apc40, Source::Note(8, 48)), None);
        assert_eq!(key_of(apc40, Source::Note(1, 82)), None);
        assert_eq!(key_of(apc40, Source::Note(0, 102)), None);
    }

    #[test]
    fn presses_and_both_kinds_of_release() {
        let (mut deck, recorder) = started(APC40_MK1_ID);
        queue(
            &recorder,
            &[
                // Top-left and bottom-right pads.
                &[0x90, 53, 0x7f],
                &[0x80, 53, 0x7f],
                &[0x97, 57, 0x7f],
                &[0x97, 57, 0x00],
                // Arm on track 3, Stop All, Shift.
                &[0x92, 48, 0x7f],
                &[0x90, 81, 0x7f],
                &[0x90, 98, 0x7f],
                // Footswitches.
                &[0xb0, 64, 0x7f],
                &[0xb0, 67, 0x40],
                &[0xb0, 64, 0x00],
                // Unknown note, a channel with no tracks, short input.
                &[0x90, 102, 0x7f],
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
                key(53, true),
                key(106, true),
                key(110, true),
                key(111, true),
                key(110, false),
            ]
        );
        let (mut deck, recorder) = started(APC20_ID);
        queue(
            &recorder,
            &[
                &[0x90, 81, 0x7f],
                &[0x93, 50, 0x7f],
                &[0xb0, 64, 0x7f],
                &[0xb0, 67, 0x7f],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![key(86, true), key(65, true), key(87, true)]
        );
    }

    #[test]
    fn knobs_turn_per_channel_and_recentre_where_they_arrived() {
        let (mut deck, recorder) = started(APC40_MK1_ID);
        queue(
            &recorder,
            &[
                &[0xb0, 48, 66],
                &[0xb0, 48, 63],
                &[0xb0, 48, 5],
                // Device knob 8.
                &[0xb0, 23, 70],
                // Track knob 1 on the Send A bank: seeds, then turns.
                &[0xb1, 48, 100],
                &[0xb1, 48, 103],
                &[0xb1, 48, 120],
                // Cue: two's complement.
                &[0xb0, 47, 0x7d],
                &[0xb0, 47, 0x02],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, 2),
                turn(0, -3),
                turn(0, -58),
                turn(15, 6),
                turn(0, 3),
                turn(0, 17),
                turn(16, -3),
                turn(16, 2),
            ]
        );
        assert_eq!(
            recorder.writes(),
            vec![vec![0xb0, 48, 64], vec![0xb1, 48, 64]]
        );
        // Centred: the next reading on each channel counts from 64.
        queue(&recorder, &[&[0xb0, 48, 65], &[0xb1, 48, 62]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(0, 1), turn(0, -2)]);
        assert_eq!(relative(0x7f), -1);
        assert_eq!(relative(0x40), -64);
        assert_eq!(relative(0x3f), 63);
    }

    #[test]
    fn faders_turn_dials_after_their_first_value() {
        let (mut deck, recorder) = started(APC40_MK1_ID);
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
                // Not a fader on this channel or deck.
                &[0xb8, 7, 0],
                &[0xb1, 14, 0],
                &[0xb0, 3, 9],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(17, 1),
                turn(17, 1),
                turn(24, -6),
                turn(25, -2),
                turn(26, 2)
            ]
        );
        assert!(recorder.writes().is_empty());
        // The APC20's faders follow its slider mode: CC 0-7.
        let (mut deck, recorder) = started(APC20_ID);
        queue(
            &recorder,
            &[
                &[0xb2, 7, 40],
                &[0xb2, 3, 48],
                &[0xb0, 14, 0],
                &[0xb0, 14, 8],
                &[0xb0, 47, 0x01],
                // No knobs or crossfader here.
                &[0xb0, 48, 100],
                &[0xb0, 15, 0],
                &[0xb0, 15, 64],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(2, 2), turn(8, 2), turn(9, 1)]
        );
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn lights_send_only_changes_with_each_kind_of_velocity() {
        let (mut deck, recorder) = started(APC40_MK1_ID);
        deck.set_face(0, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(39, Some(&lit([255, 200, 0]))).unwrap();
        deck.set_face(40, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(45, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(62, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(81, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(101, Some(&lit([255, 0, 0]))).unwrap();
        // Unlit keys send nothing.
        deck.set_face(53, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(102, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(111, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x90, 53, 1],
                vec![0x97, 57, 5],
                vec![0x90, 82, 1],
                vec![0x90, 52, 1],
                vec![0x90, 80, 127],
                vec![0x92, 48, 127],
                vec![0x90, 93, 127],
            ]
        );
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 7);
        deck.set_face(39, Some(&lit([10, 10, 10]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[7], vec![0x97, 57, 0]);
        deck.set_brightness(10).unwrap();
        assert_eq!(recorder.writes().len(), 8);
        assert_eq!(velocity(0, [255, 0, 0]), 3);
        assert_eq!(velocity(0, BLACK), 0);
    }

    #[test]
    fn letting_go_turns_every_light_off() {
        let (deck, recorder) = started(APC40_MK1_ID);
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 101);
        assert!(writes.iter().all(|write| write[2] == 0));
        let (deck, recorder) = started(APC20_ID);
        drop(deck);
        assert_eq!(recorder.writes().len(), 86);
        let (deck, recorder) = open(APC20_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn its_ports_match_in_any_case() {
        let cases = [
            ("Akai APC40", false, Some(APC40_MK1_ID)),
            ("APC40", false, Some(APC40_MK1_ID)),
            (
                "Akai APC40:Akai APC40 MIDI 1 24:0",
                false,
                Some(APC40_MK1_ID),
            ),
            ("2- Akai APC40", true, Some(APC40_MK1_ID)),
            ("MIDIIN2 (Akai APC40)", true, None),
            ("Akai APC20", false, Some(APC20_ID)),
            ("apc20", false, Some(APC20_ID)),
            // The MKII is never the first APC40.
            ("APC40 mkII", false, Some("akai-apc40-mk2")),
            ("Akai APC40 MK2", false, None),
            ("Akai APC40 MK II", false, None),
            ("APC MINI", false, Some("akai-apc-mini")),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id),
                expected,
                "{raw}"
            );
        }
        let mk1 = by_id(APC40_MK1_ID).unwrap();
        assert!(!port_matches(mk1, &normalise("Akai APC20")));
        assert!(!port_matches(
            by_id(APC20_ID).unwrap(),
            &normalise("Akai APC40")
        ));
    }

    #[test]
    fn the_model_views_place_every_key() {
        for (id, keys, lit, dials) in [(APC40_MK1_ID, 112, 101, 27), (APC20_ID, 88, 86, 10)] {
            let model = by_id(id).unwrap();
            let view = ModelView::from(model);
            let cells = view.key_cells.unwrap();
            assert_eq!(cells.len(), keys, "{id}");
            let unique: std::collections::HashSet<_> = cells.iter().collect();
            assert_eq!(unique.len(), keys, "{id}");
            let grid = u16::from(model.rows) * u16::from(model.cols);
            assert!(cells.iter().all(|cell| *cell < grid), "{id}");
            assert_eq!(view.key_names.unwrap().len(), keys);
            assert_eq!(view.lit_keys.unwrap().len(), lit);
            assert_eq!(usize::from(model.dials), dials);
            assert_eq!(view.key_light_colors, "green_red_yellow");
            assert_eq!(view.dial_lights, DialLights::None);
            assert!(view.key_lights && !view.brightness);
            assert_eq!(view.deck_lights, 0);
        }
        let apc40 = ModelView::from(by_id(APC40_MK1_ID).unwrap());
        let cells = apc40.key_cells.unwrap();
        assert_eq!(
            (cells[0], cells[40], cells[53], cells[62], cells[107], cells[111]),
            (0, 8, 5 * 9 + 8, 6 * 9 + 8, 11 * 9 + 7, 12 * 9 + 8)
        );
        let names = apc40.key_names.unwrap();
        assert_eq!(
            (names[53], names[87], names[95], names[111]),
            ("Stop All Clips", "Clip/Track", "Pan", "Footswitch 2")
        );
        let apc20 = ModelView::from(by_id(APC20_ID).unwrap());
        let cells = apc20.key_cells.unwrap();
        assert_eq!(
            (cells[53], cells[61], cells[62], cells[85], cells[86], cells[87]),
            (6 * 9, 6 * 9 + 8, 7 * 9, 9 * 9 + 7, 5 * 9 + 8, 7 * 9 + 8)
        );
        let names = apc20.key_names.unwrap();
        assert_eq!(
            (names[52], names[53], names[61], names[85], names[86], names[87]),
            (
                "Stop 8",
                "Select 1",
                "Master",
                "Arm 8",
                "Shift",
                "Foot Pedal"
            )
        );
    }
}
