//! Akai Professional's APC Key 25 (MK1) and APC Key 25 MK2, over
//! class-compliant USB MIDI ([`super::midi`]).
//!
//! Ports: the MK1 lists one MIDI port each way, which also carries its piano
//! keys, pitch and modulation and sustain pedal. The MK2 lists two pairs: the
//! first (`Keys`) carries only the piano keys and is never opened, the second
//! (`Control`) carries every button, knob and light. [`port_matches`] takes,
//! in any case, a name containing `APC Key 25` and not `mk2` for the MK1 (on
//! Windows the first interface only), and for the MK2 a name containing `APC
//! Key 25 mk2` that reads as the control port: `Control` (or a name cut short
//! to its first letter) or `MIDI 2` after the deck's name, or on Windows the
//! second interface (`MIDIIN2 (...)`). A name reading `Keys` or `MIDI 1`
//! never matches. The exact port names each host gives are not in the
//! references.
//!
//! Neither deck needs a start-up message: Ryu sends the universal identity
//! request (`F0 7E 7F 06 01 F7`) as Ableton does but needs no answer, then
//! turns every light off. There is no keep-alive.
//!
//! In (everything on MIDI channel 1; any other channel is dropped, which on
//! the MK1 drops its piano keys): the 5 x 8 clip matrix is notes 0-39 with
//! note 0 the bottom-left pad, so the top row (keys 0-7) is notes 32-39 and a
//! pad's key is `(4 - note / 8) * 8 + note % 8`; the scene launch column is
//! notes 82-86 (keys 40-44, top to bottom), the track buttons notes 64-71
//! (keys 45-52), Stop All Clips 81 (key 53), Play 91 (54), Record 93 (55) and
//! Shift 98 (56). Note On above velocity 0 presses; Note Off or velocity 0
//! releases. The eight knobs are `CC 48-55`: absolute 0-127 on the MK1 (the
//! first value only seeds a knob; later ones turn its dial by the change),
//! endless two's complement on the MK2 (1-63 up, 65-127 down). Other
//! controllers, pitch bend, aftertouch, program changes, clock and SysEx are
//! ignored.
//!
//! Out: every light is one Note On, `9n <note> <value>`. MK1: a matrix pad
//! takes 0 off, 1 green, 3 red or 5 amber (2 and 4 blink, unused) on channel
//! 1, the nearest to the key's colour. MK2: a matrix pad takes the index of
//! the nearest of Ableton's 128-colour palette (the APC40 MKII's), on channel
//! 7 for full brightness or channel 2 for half (channels 11 and 15 pulse and
//! blink, never sent); off is velocity 0 on channel 7, as Ableton sends it.
//! On both, a scene button (green) or track button (red) is 1 on or 0 off on
//! channel 1. Stop All Clips, Play, Record and Shift are not lit: whether
//! the hardware lights them is unverified. The MK1 has no brightness command;
//! on the MK2 brightness picks the matrix channel (50% and under is half) and
//! repaints the lit pads. Only changed lights are sent, and Ryu turns them all
//! off when it lets go.
//!
//! Facts from Ableton's APC_Key_25, APC_Key_25_mk2 and _APC Remote Scripts and
//! its v3 control-surface framework as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for
//! note and controller numbers, light values, channels, the palette and the
//! USB ids only); Akai's own protocol document could not be obtained. No code
//! was copied.

use super::akai_apc40::palette_index;
use super::akai_apc_mini::{led_state, pad_state};
use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const APC_KEY_25_ID: &str = "akai-apc-key-25";
pub const APC_KEY_25_MK2_ID: &str = "akai-apc-key-25-mk2";

/// The clip matrix (0-39), scene launch (40-44), track buttons (45-52), Stop
/// All Clips, Play, Record and Shift.
pub const KEYS: u8 = 57;
pub const DIALS: u8 = 8;

const MATRIX_KEYS: u8 = 40;
const ROWS: u8 = 5;
const COLS: u8 = 8;
const SCENE_KEY: u8 = 40;
const TRACK_KEY: u8 = 45;
const STOP_ALL_KEY: u8 = 53;
const PLAY_KEY: u8 = 54;
const RECORD_KEY: u8 = 55;
const SHIFT_KEY: u8 = 56;
/// Keys 0-52 have a light Ryu drives.
const LIT_KEYS: u8 = 53;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const SCENE_NOTE: u8 = 82;
const TRACK_NOTE: u8 = 64;
const STOP_ALL_NOTE: u8 = 81;
const PLAY_NOTE: u8 = 91;
const RECORD_NOTE: u8 = 93;
const SHIFT_NOTE: u8 = 98;
const KNOB_CC: u8 = 48;

/// The MK2 matrix's MIDI channel (from 0) for full brightness, which is also
/// the one its pads are turned off on.
const FULL_CHANNEL: u8 = 6;
/// The MK2 matrix's channel for half brightness.
const HALF_CHANNEL: u8 = 1;
/// Brightness at or under this many percent shows the MK2 matrix at half.
const HALF_UP_TO: u8 = 50;

const IDENTITY_REQUEST: [u8; 6] = [0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7];
const AKAI: u8 = 0x47;
const MK1_MODEL: u8 = 0x27;
const MK2_MODEL: u8 = 0x4e;

const OFF: u8 = 0;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const MATRIX_NAMES: [&str; MATRIX_KEYS as usize] = [
    "Clip 1-1", "Clip 1-2", "Clip 1-3", "Clip 1-4", "Clip 1-5", "Clip 1-6", "Clip 1-7", "Clip 1-8",
    "Clip 2-1", "Clip 2-2", "Clip 2-3", "Clip 2-4", "Clip 2-5", "Clip 2-6", "Clip 2-7", "Clip 2-8",
    "Clip 3-1", "Clip 3-2", "Clip 3-3", "Clip 3-4", "Clip 3-5", "Clip 3-6", "Clip 3-7", "Clip 3-8",
    "Clip 4-1", "Clip 4-2", "Clip 4-3", "Clip 4-4", "Clip 4-5", "Clip 4-6", "Clip 4-7", "Clip 4-8",
    "Clip 5-1", "Clip 5-2", "Clip 5-3", "Clip 5-4", "Clip 5-5", "Clip 5-6", "Clip 5-7", "Clip 5-8",
];

/// The scene launch buttons by their printed (Shift) functions: the MK1 and
/// MK2 swap Mute and Rec Arm.
const MK1_SCENE_NAMES: [&str; 5] = ["Clip Stop", "Solo", "Rec Arm", "Mute", "Select"];
const MK2_SCENE_NAMES: [&str; 5] = ["Clip Stop", "Solo", "Mute", "Rec Arm", "Select"];

/// The track buttons by their printed (Shift) functions, then the rest.
const OTHER_NAMES: [&str; 12] = [
    "Up",
    "Down",
    "Left",
    "Right",
    "Volume",
    "Pan",
    "Send",
    "Device",
    "Stop All Clips",
    "Play",
    "Record",
    "Shift",
];

const fn is_mk1(model: &DeviceModel) -> bool {
    matches!(model.protocol, Protocol::AkaiApcKey25Midi)
}

const fn is_mk2(model: &DeviceModel) -> bool {
    matches!(model.protocol, Protocol::AkaiApcKey25Mk2Midi)
}

const fn is_apc_key25(model: &DeviceModel) -> bool {
    is_mk1(model) || is_mk2(model)
}

/// Whether a normalised MK2 port name is its control port (see the module
/// notes). The marks are heuristics.
fn is_mk2_control_port(name: &PortName) -> bool {
    const NAME: &str = "apc key 25 mk2";
    let lower = name.base.to_ascii_lowercase();
    let Some(at) = lower.rfind(NAME) else {
        return false;
    };
    let rest = lower.get(at + NAME.len()..).unwrap_or("").trim();
    if rest.starts_with('k') || rest.starts_with("midi 1") {
        return false;
    }
    if rest.starts_with('c') || rest.starts_with("midi 2") {
        return true;
    }
    rest.is_empty() && name.interface == Some(2)
}

/// Whether a MIDI port, by its normalised name, is the one this model is
/// driven through.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if is_mk2(model) {
        return is_mk2_control_port(name);
    }
    if !is_mk1(model) || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let lower = name.base.to_ascii_lowercase();
    lower.contains("apc key 25") && !lower.contains("mk2")
}

/// The legend on each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    let scenes = if is_mk1(model) {
        MK1_SCENE_NAMES
    } else if is_mk2(model) {
        MK2_SCENE_NAMES
    } else {
        return None;
    };
    Some(
        MATRIX_NAMES
            .iter()
            .chain(scenes.iter())
            .chain(OTHER_NAMES.iter())
            .copied()
            .collect(),
    )
}

/// The clip matrix and the scene and track buttons.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    is_apc_key25(model).then(|| (0..LIT_KEYS).collect())
}

/// Each key's (row, column) in the 7 x 9 grid: the matrix top left with the
/// scene column at its right, the track buttons underneath with Stop All
/// Clips at the right, then Shift, Play and Record.
const fn cell(key: u8) -> (u8, u8) {
    match key {
        0..MATRIX_KEYS => (key / COLS, key % COLS),
        SCENE_KEY..TRACK_KEY => (key - SCENE_KEY, COLS),
        TRACK_KEY..STOP_ALL_KEY => (ROWS, key - TRACK_KEY),
        STOP_ALL_KEY => (ROWS, COLS),
        SHIFT_KEY => (ROWS + 1, 0),
        _ => (ROWS + 1, key - PLAY_KEY + 1),
    }
}

/// Each key's cell in the model's grid.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    if !is_apc_key25(model) {
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

/// The key a channel-1 note presses.
const fn key_of_note(note: u8) -> Option<u8> {
    match note {
        0..=39 => Some((ROWS - 1 - note / COLS) * COLS + note % COLS),
        82..=86 => Some(SCENE_KEY + note - SCENE_NOTE),
        64..=71 => Some(TRACK_KEY + note - TRACK_NOTE),
        STOP_ALL_NOTE => Some(STOP_ALL_KEY),
        PLAY_NOTE => Some(PLAY_KEY),
        RECORD_NOTE => Some(RECORD_KEY),
        SHIFT_NOTE => Some(SHIFT_KEY),
        _ => None,
    }
}

/// The note of a key.
const fn note_of_key(key: u8) -> Option<u8> {
    match key {
        0..MATRIX_KEYS => Some((ROWS - 1 - key / COLS) * COLS + key % COLS),
        SCENE_KEY..TRACK_KEY => Some(SCENE_NOTE + key - SCENE_KEY),
        TRACK_KEY..STOP_ALL_KEY => Some(TRACK_NOTE + key - TRACK_KEY),
        STOP_ALL_KEY => Some(STOP_ALL_NOTE),
        PLAY_KEY => Some(PLAY_NOTE),
        RECORD_KEY => Some(RECORD_NOTE),
        SHIFT_KEY => Some(SHIFT_NOTE),
        _ => None,
    }
}

/// An MK2 knob's two's-complement step; 0 and 64 are no movement.
fn relative(value: u8) -> Option<i8> {
    match value {
        1..=63 => i8::try_from(value).ok(),
        65..=127 => i8::try_from(i16::from(value) - 128).ok(),
        _ => None,
    }
}

/// Whether a SysEx message is an APC Key 25's identity reply.
fn is_identity_reply(message: &[u8]) -> bool {
    let Some(header) = message.get(..7) else {
        return false;
    };
    header[0] == 0xf0
        && header[1] == 0x7e
        && header[3] == 0x06
        && header[4] == 0x02
        && header[5] == AKAI
        && (header[6] == MK1_MODEL || header[6] == MK2_MODEL)
}

pub struct ApcKey25 {
    transport: Box<dyn Transport>,
    mk2: bool,
    /// The identity request and the first all-off sent.
    started: bool,
    colors: Vec<[u8; 3]>,
    /// The status byte and value each light was last sent; `None` until it is.
    sent: Vec<Option<[u8; 2]>>,
    held: Vec<bool>,
    /// Each MK1 knob's last value; `None` until it first moves.
    knobs: Vec<Option<u8>>,
    /// The channel the MK2 matrix is lit on (its brightness).
    matrix_channel: u8,
    parser: Parser,
}

impl ApcKey25 {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            mk2: is_mk2(model),
            started: false,
            colors: vec![BLACK; usize::from(LIT_KEYS)],
            sent: vec![None; usize::from(LIT_KEYS)],
            held: vec![false; usize::from(KEYS)],
            knobs: vec![None; usize::from(DIALS)],
            matrix_channel: FULL_CHANNEL,
            parser: Parser::default(),
        }
    }

    /// The status byte and value that show a colour on a lit key.
    fn light(&self, key: u8, color: [u8; 3]) -> [u8; 2] {
        if key >= MATRIX_KEYS {
            return [NOTE_ON, led_state(color)];
        }
        if !self.mk2 {
            return [NOTE_ON, pad_state(color)];
        }
        match palette_index(color) {
            OFF => [NOTE_ON | FULL_CHANNEL, OFF],
            index => [NOTE_ON | self.matrix_channel, index],
        }
    }

    /// The identity request and every light off.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if self.started {
            return Ok(());
        }
        self.transport.write(&IDENTITY_REQUEST)?;
        for key in 0..LIT_KEYS {
            let Some(note) = note_of_key(key) else {
                continue;
            };
            let off = self.light(key, BLACK);
            self.transport.write(&[off[0], note, off[1]])?;
            if let Some(sent) = self.sent.get_mut(usize::from(key)) {
                *sent = Some(off);
            }
        }
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

    fn knob(&mut self, dial: u8, value: u8, events: &mut Vec<Input>) {
        let delta = if self.mk2 {
            relative(value)
        } else {
            let Some(last) = self.knobs.get_mut(usize::from(dial)) else {
                return;
            };
            last.replace(value)
                .and_then(|old| i8::try_from(i16::from(value) - i16::from(old)).ok())
        };
        if let Some(delta) = delta.filter(|delta| *delta != 0) {
            events.push(Input::DialTurn { index: dial, delta });
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let [status, number, value] = match message {
            Message::Channel(bytes) => *bytes,
            Message::SysEx(bytes) => {
                if is_identity_reply(bytes) {
                    tracing::debug!("ryu-decks: APC Key 25 identified");
                }
                return;
            }
        };
        if status & 0x0f != 0 {
            return;
        }
        match status & 0xf0 {
            NOTE_ON | NOTE_OFF => {
                if let Some(key) = key_of_note(number) {
                    self.press(key, status & 0xf0 == NOTE_ON && value > 0, events);
                }
            }
            CONTROL_CHANGE => {
                if let Some(dial) = number.checked_sub(KNOB_CC).filter(|dial| *dial < DIALS) {
                    self.knob(dial, value, events);
                }
            }
            _ => {}
        }
    }
}

impl Driver for ApcKey25 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
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
        for message in &messages {
            self.receive(message, &mut events);
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// One Note On for each light whose channel or value changed.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        for key in 0..LIT_KEYS {
            let (Some(note), Some(color)) = (note_of_key(key), self.colors.get(usize::from(key)))
            else {
                continue;
            };
            let state = self.light(key, *color);
            let Some(sent) = self.sent.get_mut(usize::from(key)) else {
                continue;
            };
            if *sent != Some(state) {
                self.transport.write(&[state[0], note, state[1]])?;
                *sent = Some(state);
            }
        }
        Ok(())
    }

    /// The MK2 shows its matrix at half or full brightness by channel; the MK1
    /// has no brightness command.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        if !self.mk2 {
            return Ok(());
        }
        self.matrix_channel = if percent <= HALF_UP_TO {
            HALF_CHANNEL
        } else {
            FULL_CHANNEL
        };
        self.flush()
    }
}

impl Drop for ApcKey25 {
    /// Turns every light off: the deck would otherwise keep them.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for key in 0..LIT_KEYS {
            if let Some(note) = note_of_key(key) {
                let off = self.light(key, BLACK);
                let _ = self.transport.write(&[off[0], note, off[1]]);
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

    fn open(id: &str) -> (ApcKey25, Recorder) {
        let recorder = Recorder::default();
        let deck = ApcKey25::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// An opened deck with its start-up sent and cleared.
    fn started(id: &str) -> (ApcKey25, Recorder) {
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
    fn start_up_asks_for_identity_and_turns_every_light_off() {
        let (mut deck, recorder) = open(APC_KEY_25_ID);
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(writes.len(), 1 + 53);
        assert_eq!(writes[0], IDENTITY_REQUEST.to_vec());
        // Key 0 is the top-left pad, note 32; the bottom-right pad is note 7.
        assert_eq!(writes[1], vec![0x90, 32, 0]);
        assert_eq!(writes[40], vec![0x90, 7, 0]);
        assert_eq!(writes[41], vec![0x90, 82, 0]);
        assert_eq!(writes[46], vec![0x90, 64, 0]);
        assert_eq!(writes[53], vec![0x90, 71, 0]);
        // Nothing has changed, so the next flush sends nothing.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 54);
    }

    #[test]
    fn the_mk2_turns_its_matrix_off_on_the_full_brightness_channel() {
        let (mut deck, recorder) = open(APC_KEY_25_MK2_ID);
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 54);
        assert_eq!(writes[1], vec![0x96, 32, 0]);
        assert_eq!(writes[40], vec![0x96, 7, 0]);
        // Scene and track buttons are on channel 1.
        assert_eq!(writes[41], vec![0x90, 82, 0]);
        assert_eq!(writes[53], vec![0x90, 71, 0]);
    }

    #[test]
    fn every_key_maps_to_its_note_and_back() {
        assert_eq!(note_of_key(0), Some(32));
        assert_eq!(note_of_key(7), Some(39));
        assert_eq!(note_of_key(32), Some(0));
        assert_eq!(note_of_key(39), Some(7));
        assert_eq!(note_of_key(40), Some(82));
        assert_eq!(note_of_key(44), Some(86));
        assert_eq!(note_of_key(45), Some(64));
        assert_eq!(note_of_key(52), Some(71));
        assert_eq!(note_of_key(53), Some(81));
        assert_eq!(note_of_key(54), Some(91));
        assert_eq!(note_of_key(55), Some(93));
        assert_eq!(note_of_key(56), Some(98));
        assert_eq!(note_of_key(57), None);
        for key in 0..KEYS {
            assert_eq!(key_of_note(note_of_key(key).unwrap()), Some(key), "{key}");
        }
        for note in [40, 63, 72, 80, 87, 90, 92, 97, 99, 127] {
            assert_eq!(key_of_note(note), None, "{note}");
        }
    }

    #[test]
    fn presses_and_both_kinds_of_release_on_channel_one_only() {
        let (mut deck, recorder) = started(APC_KEY_25_ID);
        queue(
            &recorder,
            &[
                &[0x90, 32, 0x7f],
                &[0x80, 32, 0x7f],
                &[0x90, 7, 0x40],
                &[0x90, 7, 0x00],
                &[0x90, 82, 0x7f],
                &[0x90, 71, 0x7f],
                &[0x90, 81, 0x7f],
                &[0x90, 91, 0x7f],
                &[0x90, 93, 0x7f],
                &[0x90, 98, 0x7f],
                // The MK1's piano keys on another channel, an unknown note,
                // pitch bend, aftertouch, a sustain pedal and short input.
                &[0x91, 64, 0x7f],
                &[0x90, 60, 0x7f],
                &[0xe0, 0x00, 0x40],
                &[0xd0, 0x40],
                &[0xb0, 64, 0x7f],
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
                key(40, true),
                key(52, true),
                key(53, true),
                key(54, true),
                key(55, true),
                key(56, true),
            ]
        );
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn mk1_knobs_are_absolute_from_their_first_value() {
        let (mut deck, recorder) = started(APC_KEY_25_ID);
        queue(
            &recorder,
            &[
                &[0xb0, 48, 64],
                &[0xb0, 48, 66],
                &[0xb0, 48, 60],
                &[0xb0, 48, 60],
                &[0xb0, 55, 127],
                &[0xb0, 55, 0],
                // Another channel and controller.
                &[0xb1, 48, 0],
                &[0xb0, 56, 10],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(0, 2), turn(0, -6), turn(7, -127)]
        );
    }

    #[test]
    fn mk2_knobs_are_twos_complement() {
        let (mut deck, recorder) = started(APC_KEY_25_MK2_ID);
        queue(
            &recorder,
            &[
                &[0xb0, 48, 1],
                &[0xb0, 48, 0x7f],
                &[0xb0, 50, 5],
                &[0xb0, 55, 0x41],
                &[0xb0, 49, 0],
                &[0xb0, 49, 64],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(0, 1), turn(0, -1), turn(2, 5), turn(7, -63)]
        );
        assert_eq!(relative(63), Some(63));
        assert_eq!(relative(64), None);
    }

    #[test]
    fn mk1_lights_are_green_red_amber_or_on_off() {
        let (mut deck, recorder) = started(APC_KEY_25_ID);
        deck.set_face(0, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(1, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(2, Some(&lit([255, 160, 0]))).unwrap();
        deck.set_face(40, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(52, Some(&lit([0, 0, 255]))).unwrap();
        // Keys without a light take nothing.
        deck.set_face(54, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x90, 32, 1],
                vec![0x90, 33, 3],
                vec![0x90, 34, 5],
                vec![0x90, 82, 1],
                vec![0x90, 71, 1],
            ]
        );
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 5);
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[5], vec![0x90, 32, 0]);
        // No brightness command.
        deck.set_brightness(10).unwrap();
        assert_eq!(recorder.writes().len(), 6);
    }

    #[test]
    fn mk2_pads_take_the_nearest_palette_colour_and_brightness_channel() {
        let (mut deck, recorder) = started(APC_KEY_25_MK2_ID);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(39, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(44, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![vec![0x96, 32, 5], vec![0x96, 7, 45], vec![0x90, 86, 1]]
        );
        // Half brightness repaints the lit pads on channel 2 only.
        deck.set_brightness(40).unwrap();
        assert_eq!(
            recorder.writes()[3..],
            [vec![0x91, 32, 5], vec![0x91, 7, 45]]
        );
        // Off goes back on the full-brightness channel.
        deck.set_face(0, Some(&lit(BLACK))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[5], vec![0x96, 32, 0]);
        deck.set_brightness(100).unwrap();
        assert_eq!(recorder.writes()[6..], [vec![0x96, 7, 45]]);
    }

    #[test]
    fn letting_go_turns_every_light_off() {
        let (mut deck, recorder) = started(APC_KEY_25_MK2_ID);
        deck.set_face(3, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 53);
        assert!(writes.iter().all(|write| write[2] == 0));
        assert_eq!(writes[3], vec![0x96, 35, 0]);
        let (deck, recorder) = open(APC_KEY_25_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn identity_replies_are_recognised_and_ignored() {
        assert!(is_identity_reply(&[
            0xf0, 0x7e, 0x00, 0x06, 0x02, 0x47, 0x27, 0x00, 0x19, 0xf7
        ]));
        assert!(is_identity_reply(&[
            0xf0, 0x7e, 0x00, 0x06, 0x02, 0x47, 0x4e, 0x00, 0x19, 0xf7
        ]));
        assert!(!is_identity_reply(&[0xf0, 0x7e, 0x00, 0x06, 0x02, 0x47]));
        let (mut deck, recorder) = started(APC_KEY_25_ID);
        queue(
            &recorder,
            &[&[0xf0, 0x7e, 0x00, 0x06, 0x02, 0x47, 0x27, 0x00, 0x19, 0xf7]],
        );
        assert!(deck.poll(0).unwrap().is_empty());
    }

    #[test]
    fn ports_match_by_name_and_the_mk2_by_its_control_port() {
        let mk1 = by_id(APC_KEY_25_ID).unwrap();
        let mk2 = by_id(APC_KEY_25_MK2_ID).unwrap();
        let cases: [(&str, bool, Option<&str>); 13] = [
            ("APC Key 25", false, Some(APC_KEY_25_ID)),
            (
                "APC Key 25:APC Key 25 MIDI 1 24:0",
                false,
                Some(APC_KEY_25_ID),
            ),
            ("apc key 25", true, Some(APC_KEY_25_ID)),
            ("APC Key 25 mk2 Control", false, Some(APC_KEY_25_MK2_ID)),
            (
                "APC Key 25 mk2:APC Key 25 mk2 Contr 24:1",
                false,
                Some(APC_KEY_25_MK2_ID),
            ),
            (
                "APC Key 25 mk2:APC Key 25 mk2 MIDI 2 24:1",
                false,
                Some(APC_KEY_25_MK2_ID),
            ),
            ("MIDIIN2 (APC Key 25 mk2)", true, Some(APC_KEY_25_MK2_ID)),
            ("APC Key 25 MK2 Control", false, Some(APC_KEY_25_MK2_ID)),
            // The MK2's keys port, never the deck.
            ("APC Key 25 mk2 Keys", false, None),
            ("APC Key 25 mk2:APC Key 25 mk2 MIDI 1 24:0", false, None),
            ("APC Key 25 mk2", true, None),
            // Other APCs.
            ("APC mini mk2 Control", false, Some("akai-apc-mini-mk2")),
            ("APC40 mkII", false, Some("akai-apc40-mk2")),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(
                port_matches(mk1, &name),
                expected == Some(APC_KEY_25_ID),
                "{raw}"
            );
            assert_eq!(
                port_matches(mk2, &name),
                expected == Some(APC_KEY_25_MK2_ID),
                "{raw}"
            );
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_views_place_and_name_every_key() {
        for (id, mute, colors, brightness) in [
            (APC_KEY_25_ID, 43, "green_red_yellow", false),
            (APC_KEY_25_MK2_ID, 42, "rgb", true),
        ] {
            let model = by_id(id).unwrap();
            let view = ModelView::from(model);
            let cells = view.key_cells.unwrap();
            assert_eq!(cells.len(), 57);
            let unique: std::collections::HashSet<_> = cells.iter().collect();
            assert_eq!(unique.len(), 57);
            let grid = u16::from(model.rows) * u16::from(model.cols);
            assert!(cells.iter().all(|cell| *cell < grid));
            assert_eq!(
                (cells[0], cells[40], cells[45], cells[53], cells[54], cells[56]),
                (0, 8, 45, 53, 55, 54)
            );
            let names = view.key_names.unwrap();
            assert_eq!(names.len(), 57);
            assert_eq!(
                (names[0], names[39], names[mute], names[45], names[56]),
                ("Clip 1-1", "Clip 5-8", "Mute", "Up", "Shift")
            );
            assert_eq!(view.lit_keys.unwrap(), (0..53).collect::<Vec<u8>>());
            assert_eq!(view.key_light_colors, colors);
            assert_eq!(view.brightness, brightness);
            assert_eq!(view.dial_lights, DialLights::None);
            assert!(view.key_lights);
            assert_eq!((view.deck_lights, view.dials), (0, 8));
        }
    }
}
