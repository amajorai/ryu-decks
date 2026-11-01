//! Akai Professional's APC mini MK2, over class-compliant USB MIDI
//! ([`super::midi`]).
//!
//! The APC mini MK2 lists two MIDI ports: the first (its control port,
//! `APC mini mk2 Control` on macOS, `... APC mini mk2 Contr` on Linux, the bare
//! `APC mini mk2` on Windows) carries every button, fader and light; the
//! second (`Notes`) carries only note-mode pads and is never opened.
//! [`port_matches`] finds the first by name.
//!
//! Everything is on MIDI channel 1 in the default session pad mode. In: the
//! 8 x 8 pads are notes 0-63 with note 0 the bottom-left pad, running left to
//! right and then upwards, so a pad's key (reading order from the top left)
//! is `(7 - note / 8) * 8 + note % 8`; the scene column is notes 112-119 top
//! to bottom (keys 64-71), the track row notes 100-107 left to right (keys
//! 72-79) and Shift note 122 (key 80). Any Note On above velocity 0 presses; a
//! Note Off or velocity 0 releases. Messages on other channels (drum-mode pads
//! on channel 10) are dropped. The nine faders are `CC 48-56`, absolute 0-127,
//! turned into dial steps of four units from the first value seen.
//!
//! SysEx is `F0 47 7F 4F <id> <len MSB> <len LSB> <data> F7`. On open Ryu
//! sends the session pad mode (`62`, data `00`), the introduction (`60`, data
//! `00 01 00 00`), whose `61` answer carries the nine fader positions that seed
//! the dials, and a velocity-0 Note On to every pad (which clears a blink or
//! pulse another app left, which would hide the RGB colours). The deck
//! reports a pad mode change on `62`; any mode but session (note, drum, note
//! edit) makes Ryu send session mode again and repaint.
//!
//! Out: the pads take 8-bit RGB by `24`, in groups of `start end R>>7 R&7F
//! G>>7 G&7F B>>7 B&7F` (a range of notes in one colour), at most 32 groups a
//! message. The scene (green) and track (red) LEDs are on or off by Note On
//! velocity 1 or 0; Shift has no light Ryu drives (unverified). There is no
//! brightness command: Ryu scales the RGB, and a single-colour LED is on while
//! its scaled colour has a channel at 32 or more. Only changed lights are
//! sent. When Ryu lets go it blanks the pads and LEDs and leaves session mode
//! set.
//!
//! Facts from Ableton Live's APC mini MK2 remote script (no licence; read for
//! note and controller numbers, SysEx ids and the USB id only), cross-checked
//! against tstriker's akai-apc-mini-mk2 (MIT) and DJj123dj's apc-mini-mk2
//! (MIT); no code was copied.

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const APC_MINI_MK2_ID: &str = "akai-apc-mini-mk2";

/// The 8 x 8 pads, keys 0 to 63.
pub const PADS: u8 = 64;
/// Eight scene keys (64-71), eight track keys (72-79), then Shift (80).
pub const KEYS: u8 = 81;
const SCENE_KEY: u8 = 64;
const TRACK_KEY: u8 = 72;
const SHIFT_KEY: u8 = 80;
/// The keys with a single-colour LED (scene and track).
const LEDS: usize = 16;
pub const FADERS: u8 = 9;
const ROW: u8 = 8;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const SCENE_NOTE: u8 = 0x70;
const TRACK_NOTE: u8 = 0x64;
const SHIFT_NOTE: u8 = 0x7a;
const FADER_CC: u8 = 0x30;

const SYSEX_END: u8 = 0xf7;
/// Akai's manufacturer id, the "any device" byte and the APC mini MK2's model.
const HEADER: [u8; 4] = [0xf0, 0x47, 0x7f, 0x4f];
const RGB: u8 = 0x24;
const INTRODUCTION: u8 = 0x60;
const FADER_DUMP: u8 = 0x61;
const PAD_MODE: u8 = 0x62;
const SESSION_MODE: u8 = 0x00;
/// Pad groups per `24` message.
const MAX_GROUPS: usize = 32;
const GROUP_LEN: usize = 8;

/// Fader units (of 128) per dial step: 32 steps from end to end.
const FADER_STEP: i16 = 4;
/// A single-colour LED is on while a scaled channel reaches this.
const ON_THRESHOLD: u8 = 32;
const LED_ON: u8 = 1;
const LED_OFF: u8 = 0;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const NAMES: [&str; KEYS as usize] = [
    "Pad 1",
    "Pad 2",
    "Pad 3",
    "Pad 4",
    "Pad 5",
    "Pad 6",
    "Pad 7",
    "Pad 8",
    "Pad 9",
    "Pad 10",
    "Pad 11",
    "Pad 12",
    "Pad 13",
    "Pad 14",
    "Pad 15",
    "Pad 16",
    "Pad 17",
    "Pad 18",
    "Pad 19",
    "Pad 20",
    "Pad 21",
    "Pad 22",
    "Pad 23",
    "Pad 24",
    "Pad 25",
    "Pad 26",
    "Pad 27",
    "Pad 28",
    "Pad 29",
    "Pad 30",
    "Pad 31",
    "Pad 32",
    "Pad 33",
    "Pad 34",
    "Pad 35",
    "Pad 36",
    "Pad 37",
    "Pad 38",
    "Pad 39",
    "Pad 40",
    "Pad 41",
    "Pad 42",
    "Pad 43",
    "Pad 44",
    "Pad 45",
    "Pad 46",
    "Pad 47",
    "Pad 48",
    "Pad 49",
    "Pad 50",
    "Pad 51",
    "Pad 52",
    "Pad 53",
    "Pad 54",
    "Pad 55",
    "Pad 56",
    "Pad 57",
    "Pad 58",
    "Pad 59",
    "Pad 60",
    "Pad 61",
    "Pad 62",
    "Pad 63",
    "Pad 64",
    "Clip Stop",
    "Solo",
    "Mute",
    "Rec Arm",
    "Select",
    "Drum",
    "Note",
    "Stop All Clips",
    "Volume",
    "Pan",
    "Send",
    "Device",
    "Up",
    "Down",
    "Left",
    "Right",
    "Shift",
];

/// Whether a normalised port name is an APC mini MK2's control port: it
/// names the deck and either reads as the control port (`Contr`, `MIDI 1`, or
/// the bare name) or carries none of the second port's marks (`Notes`,
/// `MIDI 2`, `MIDIIN2`). The marks are heuristics; Windows' second interface
/// (`MIDIIN2 (...)`) never matches.
fn is_control_port(base: &str) -> bool {
    const NAME: &str = "apc mini mk2";
    let lower = base.to_ascii_lowercase();
    let Some(at) = lower.rfind(NAME) else {
        return false;
    };
    let rest = lower.get(at + NAME.len()..).unwrap_or("").trim();
    let control = rest.starts_with("contr") || rest.starts_with("midi 1") || rest.is_empty();
    let second = ["notes", "midi 2", "midiin2"]
        .iter()
        .any(|mark| lower.contains(mark));
    control || !second
}

/// Whether a MIDI port, by its normalised name, is this model's control port.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.protocol == Protocol::AkaiApcMiniMk2Midi
        && name.interface.unwrap_or(1) == 1
        && is_control_port(&name.base)
}

const fn is_apc(model: &DeviceModel) -> bool {
    matches!(model.protocol, Protocol::AkaiApcMiniMk2Midi)
}

/// Both APC minis place their keys the same way.
const fn has_apc_layout(model: &DeviceModel) -> bool {
    matches!(
        model.protocol,
        Protocol::AkaiApcMiniMk2Midi | Protocol::AkaiApcMiniMidi
    )
}

/// The legend on each key: the pads, then the scene and track buttons by
/// their Shift functions, then Shift.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    is_apc(model).then(|| NAMES.to_vec())
}

/// Each key's (row, column) in the 9 x 9 grid: the pads top left, the scene
/// column at the right, the track row underneath, Shift bottom right.
const fn cell(key: u8) -> (u8, u8) {
    if key < PADS {
        (key / ROW, key % ROW)
    } else if key < TRACK_KEY {
        (key - SCENE_KEY, ROW)
    } else if key < SHIFT_KEY {
        (ROW, key - TRACK_KEY)
    } else {
        (ROW, ROW)
    }
}

/// Each key's cell in the model's grid.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    if !has_apc_layout(model) {
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

/// Every key but Shift has a light (on both APC minis).
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    has_apc_layout(model).then(|| (0..SHIFT_KEY).collect())
}

/// The key a note presses.
const fn key_of_note(note: u8) -> Option<u8> {
    match note {
        0..=63 => Some((ROW - 1 - note / ROW) * ROW + note % ROW),
        0x70..=0x77 => Some(SCENE_KEY + note - SCENE_NOTE),
        0x64..=0x6b => Some(TRACK_KEY + note - TRACK_NOTE),
        SHIFT_NOTE => Some(SHIFT_KEY),
        _ => None,
    }
}

/// The note of a key: a pad's note counts from the bottom-left pad.
const fn note_of_key(key: u8) -> Option<u8> {
    if key < PADS {
        Some((ROW - 1 - key / ROW) * ROW + key % ROW)
    } else if key < TRACK_KEY {
        Some(SCENE_NOTE + key - SCENE_KEY)
    } else if key < SHIFT_KEY {
        Some(TRACK_NOTE + key - TRACK_KEY)
    } else if key == SHIFT_KEY {
        Some(SHIFT_NOTE)
    } else {
        None
    }
}

/// A whole SysEx message with its 14-bit data length.
fn sysex(id: u8, data: &[u8]) -> Vec<u8> {
    let len = data.len().min(0x3fff);
    let mut out = Vec::with_capacity(HEADER.len() + 4 + len);
    out.extend_from_slice(&HEADER);
    out.push(id);
    out.push(((len >> 7) & 0x7f) as u8);
    out.push((len & 0x7f) as u8);
    out.extend_from_slice(&data[..len]);
    out.push(SYSEX_END);
    out
}

/// Sets session pad mode (also what Ableton leaves the deck in).
fn session_mode() -> Vec<u8> {
    sysex(PAD_MODE, &[SESSION_MODE])
}

/// Asks for the fader positions (`61`).
fn introduction() -> Vec<u8> {
    sysex(INTRODUCTION, &[0x00, 0x01, 0x00, 0x00])
}

/// A colour scaled by `percent`.
fn scaled(color: [u8; 3], percent: u8) -> [u8; 3] {
    let percent = u16::from(percent.min(100));
    color.map(|channel| ((u16::from(channel) * percent + 50) / 100) as u8)
}

/// One pad group: notes `start..=end` in one colour, each channel split into
/// its top bit and low seven bits.
fn group(start: u8, end: u8, [red, green, blue]: [u8; 3]) -> [u8; GROUP_LEN] {
    [
        start,
        end,
        red >> 7,
        red & 0x7f,
        green >> 7,
        green & 0x7f,
        blue >> 7,
        blue & 0x7f,
    ]
}

/// `24` messages for the given (note, colour) pairs, in note order, adjacent
/// notes of one colour joined into a range; at most 32 groups a message.
fn rgb_messages(pads: &[(u8, [u8; 3])]) -> Vec<Vec<u8>> {
    let mut groups: Vec<(u8, u8, [u8; 3])> = Vec::new();
    for &(note, color) in pads {
        match groups.last_mut() {
            Some((_, end, last)) if *end + 1 == note && *last == color => *end = note,
            _ => groups.push((note, note, color)),
        }
    }
    groups
        .chunks(MAX_GROUPS)
        .map(|chunk| {
            let data: Vec<u8> = chunk
                .iter()
                .flat_map(|(start, end, color)| group(*start, *end, *color))
                .collect();
            sysex(RGB, &data)
        })
        .collect()
}

fn is_on(color: [u8; 3]) -> bool {
    color.iter().any(|channel| *channel >= ON_THRESHOLD)
}

pub struct ApcMiniMk2 {
    transport: Box<dyn Transport>,
    /// Session mode set and the pads cleared, as far as Ryu knows.
    started: bool,
    colors: Vec<[u8; 3]>,
    /// The scaled colour each pad was last sent, by note; `None` until it is.
    sent_pads: Vec<Option<[u8; 3]>>,
    /// The velocity each scene, then track, LED was last sent.
    sent_leds: Vec<Option<u8>>,
    held: Vec<bool>,
    /// Each fader's last position; `None` until one arrives.
    faders: Vec<Option<u8>>,
    /// Fader movement not yet turned into a whole step.
    rest: Vec<i16>,
    brightness: u8,
    parser: Parser,
}

impl ApcMiniMk2 {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            colors: vec![BLACK; usize::from(KEYS)],
            sent_pads: vec![None; usize::from(PADS)],
            sent_leds: vec![None; LEDS],
            held: vec![false; usize::from(KEYS)],
            faders: vec![None; usize::from(FADERS)],
            rest: vec![0; usize::from(FADERS)],
            brightness: 100,
            parser: Parser::default(),
        }
    }

    /// Session mode, the introduction, and every pad's animation cleared.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if self.started {
            return Ok(());
        }
        self.transport.write(&session_mode())?;
        self.transport.write(&introduction())?;
        for note in 0..PADS {
            self.transport.write(&[NOTE_ON, note, 0x00])?;
        }
        self.forget();
        self.started = true;
        Ok(())
    }

    fn forget(&mut self) {
        self.sent_pads.fill(None);
        self.sent_leds.fill(None);
    }

    fn release_all(&mut self, events: &mut Vec<Input>) {
        for (index, held) in (0u8..).zip(self.held.iter_mut()) {
            if *held {
                *held = false;
                events.push(Input::Key {
                    index,
                    pressed: false,
                });
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

    /// A fader's absolute position: the first seeds it, each later one
    /// turns its dial a step per four units moved.
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
        if steps != 0 {
            events.push(Input::DialTurn {
                index: dial,
                delta: i8::try_from(steps.clamp(-127, 127)).unwrap_or(0),
            });
        }
    }

    /// Seeds every fader from the `61` dump, without turning a dial.
    fn seed(&mut self, data: &[u8]) {
        for ((last, rest), value) in self.faders.iter_mut().zip(&mut self.rest).zip(data) {
            if *value <= 0x7f {
                *last = Some(*value);
                *rest = 0;
            }
        }
    }

    /// Handles one message; returns whether to send session mode again and
    /// repaint.
    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> bool {
        let [status, number, value] = match message {
            Message::SysEx(bytes) => return self.receive_sysex(bytes, events),
            Message::Channel(bytes) => *bytes,
        };
        if status & 0x0f != 0 {
            return false;
        }
        match status & 0xf0 {
            NOTE_ON | NOTE_OFF => {
                if let Some(key) = key_of_note(number) {
                    self.press(key, status & 0xf0 == NOTE_ON && value > 0, events);
                }
            }
            CONTROL_CHANGE => {
                if let Some(dial) = number.checked_sub(FADER_CC).filter(|dial| *dial < FADERS) {
                    self.fader(dial, value, events);
                }
            }
            _ => {}
        }
        false
    }

    fn receive_sysex(&mut self, bytes: &[u8], events: &mut Vec<Input>) -> bool {
        if bytes.get(..HEADER.len()) != Some(&HEADER[..]) {
            return false;
        }
        let (Some(id), Some(high), Some(low)) = (bytes.get(4), bytes.get(5), bytes.get(6)) else {
            return false;
        };
        let len = (usize::from(*high) << 7) | usize::from(*low);
        let data = bytes.get(7..).unwrap_or_default();
        let data = data.get(..len).unwrap_or(data);
        match *id {
            FADER_DUMP => {
                self.seed(data);
                false
            }
            PAD_MODE => match data.first() {
                Some(&SESSION_MODE) | None => false,
                Some(_) => {
                    self.release_all(events);
                    true
                }
            },
            _ => false,
        }
    }
}

impl Driver for ApcMiniMk2 {
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
        let mut reset = false;
        for message in &messages {
            reset |= self.receive(message, &mut events);
        }
        if reset {
            self.transport.write(&session_mode())?;
            self.forget();
            self.flush()?;
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if key == SHIFT_KEY {
            return Ok(());
        }
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// The changed pads in `24` messages, then each changed LED.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        let mut pads = Vec::new();
        for note in 0..PADS {
            let Some(key) = key_of_note(note) else {
                continue;
            };
            let color = scaled(self.colors[usize::from(key)], self.brightness);
            if self.sent_pads[usize::from(note)] != Some(color) {
                pads.push((note, color));
            }
        }
        for message in rgb_messages(&pads) {
            self.transport.write(&message)?;
        }
        for (note, color) in pads {
            self.sent_pads[usize::from(note)] = Some(color);
        }
        for (slot, key) in (SCENE_KEY..SHIFT_KEY).enumerate() {
            let Some(note) = note_of_key(key) else {
                continue;
            };
            let color = scaled(self.colors[usize::from(key)], self.brightness);
            let velocity = if is_on(color) { LED_ON } else { LED_OFF };
            if self.sent_leds[slot] != Some(velocity) {
                self.transport.write(&[NOTE_ON, note, velocity])?;
                self.sent_leds[slot] = Some(velocity);
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }
}

impl Drop for ApcMiniMk2 {
    /// Blanks the pads and LEDs, then leaves session mode set.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        let _ = self
            .transport
            .write(&sysex(RGB, &group(0, PADS - 1, BLACK)));
        for note in (TRACK_NOTE..TRACK_NOTE + ROW).chain(SCENE_NOTE..SCENE_NOTE + ROW) {
            let _ = self.transport.write(&[NOTE_ON, note, LED_OFF]);
        }
        let _ = self.transport.write(&session_mode());
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    /// Session mode, the introduction and 64 pad clears.
    const START: usize = 2 + 64;

    fn open() -> (ApcMiniMk2, Recorder) {
        let recorder = Recorder::default();
        let deck = ApcMiniMk2::new(by_id(APC_MINI_MK2_ID).unwrap(), Box::new(recorder.clone()));
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
    fn notes_map_to_keys_in_reading_order() {
        assert_eq!(key_of_note(0), Some(56));
        assert_eq!(key_of_note(7), Some(63));
        assert_eq!(key_of_note(56), Some(0));
        assert_eq!(key_of_note(63), Some(7));
        assert_eq!(key_of_note(112), Some(64));
        assert_eq!(key_of_note(119), Some(71));
        assert_eq!(key_of_note(100), Some(72));
        assert_eq!(key_of_note(107), Some(79));
        assert_eq!(key_of_note(122), Some(80));
        assert_eq!(key_of_note(64), None);
        assert_eq!(key_of_note(108), None);
        assert_eq!(key_of_note(127), None);
        for key in 0..KEYS {
            assert_eq!(note_of_key(key).and_then(key_of_note), Some(key));
        }
    }

    #[test]
    fn opening_sets_session_mode_asks_for_the_faders_and_paints() {
        let (mut deck, recorder) = open();
        deck.set_face(56, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(64, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(79, Some(&lit([200, 0, 0]))).unwrap();
        deck.set_face(80, Some(&lit([255, 255, 255]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[0],
            vec![0xf0, 0x47, 0x7f, 0x4f, 0x62, 0x00, 0x01, 0x00, 0xf7]
        );
        assert_eq!(
            writes[1],
            vec![0xf0, 0x47, 0x7f, 0x4f, 0x60, 0x00, 0x04, 0x00, 0x01, 0x00, 0x00, 0xf7]
        );
        for note in 0..64u8 {
            assert_eq!(writes[2 + usize::from(note)], vec![0x90, note, 0x00]);
        }
        // Pad 0 red, then notes 1-63 black in one range.
        assert_eq!(
            writes[START],
            vec![
                0xf0, 0x47, 0x7f, 0x4f, 0x24, 0x00, 0x10, 0x00, 0x00, 0x01, 0x7f, 0x00, 0x00, 0x00,
                0x00, 0x01, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf7
            ]
        );
        // Scene LEDs, then track LEDs; Shift has none.
        let leds = &writes[START + 1..];
        assert_eq!(leds.len(), 16);
        assert_eq!(leds[0], vec![0x90, 112, 1]);
        assert_eq!(leds[1], vec![0x90, 113, 0]);
        assert_eq!(leds[8], vec![0x90, 100, 0]);
        assert_eq!(leds[15], vec![0x90, 107, 1]);
        // Nothing changed: nothing sent.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), START + 17);
    }

    #[test]
    fn one_pad_is_one_exact_group() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_face(56, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [vec![
                0xf0, 0x47, 0x7f, 0x4f, 0x24, 0x00, 0x08, 0x00, 0x00, 0x01, 0x7f, 0x00, 0x00, 0x00,
                0x00, 0xf7
            ]]
        );
        // Mid values split into their top bit and low seven bits.
        assert_eq!(
            group(5, 9, [0x80, 0x7f, 0xc8]),
            [5, 9, 1, 0x00, 0, 0x7f, 1, 0x48]
        );
    }

    #[test]
    fn thirty_three_pads_take_two_messages() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        // Alternating colours, so no two neighbours join into a range.
        for note in 0..33u8 {
            let shade = if note % 2 == 0 { 10 } else { 20 };
            deck.set_face(key_of_note(note).unwrap(), Some(&lit([shade, 0, 0])))
                .unwrap();
        }
        deck.flush().unwrap();
        let writes = &recorder.writes()[before..];
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].len(), 4 + 3 + 32 * 8 + 1);
        assert_eq!(writes[0][4..7], [0x24, 0x02, 0x00]);
        assert_eq!(writes[0][7..15], [0, 0, 0, 10, 0, 0, 0, 0]);
        assert_eq!(
            writes[1],
            vec![0xf0, 0x47, 0x7f, 0x4f, 0x24, 0x00, 0x08, 32, 32, 0, 10, 0, 0, 0, 0, 0xf7]
        );
    }

    #[test]
    fn brightness_scales_the_pads_and_can_turn_leds_off() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([200, 100, 0]))).unwrap();
        deck.set_face(64, Some(&lit([40, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_brightness(50).unwrap();
        let writes = &recorder.writes()[before..];
        // Pad 1 is note 56; 100 and 50 are under 128.
        assert_eq!(
            writes[0],
            vec![0xf0, 0x47, 0x7f, 0x4f, 0x24, 0x00, 0x08, 56, 56, 0, 100, 0, 50, 0, 0, 0xf7]
        );
        // 40 at half is under the threshold: the scene LED goes off.
        assert_eq!(writes[1..], [vec![0x90, 112, 0]]);
    }

    #[test]
    fn presses_map_and_other_channels_are_dropped() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 0, 0x7f],
                &[0x80, 0, 0x7f],
                &[0x90, 63, 0x40],
                &[0x90, 63, 0x00],
                &[0x90, 112, 0x7f],
                &[0x90, 100, 0x7f],
                &[0x90, 122, 0x7f],
                &[0x80, 122, 0x00],
                // Drum-mode pads on channel 10, and another channel's note.
                &[0x99, 64, 0x7f],
                &[0x89, 64, 0x00],
                &[0x91, 0, 0x7f],
                // Short and stray input.
                &[0x90],
                &[0x47, 0x00],
            ],
        );
        let events = deck.poll(0).unwrap();
        assert_eq!(
            events,
            vec![
                key(56, true),
                key(56, false),
                key(7, true),
                key(7, false),
                key(64, true),
                key(72, true),
                key(80, true),
                key(80, false),
            ]
        );
    }

    #[test]
    fn faders_turn_dials_after_their_first_value() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 48, 0],
                &[0xb0, 48, 8],
                &[0xb0, 48, 9],
                &[0xb0, 48, 12],
                &[0xb0, 56, 127],
                &[0xb0, 56, 100],
                // Not a fader; another channel.
                &[0xb0, 57, 3],
                &[0xb1, 48, 100],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialTurn { index: 0, delta: 2 },
                Input::DialTurn { index: 0, delta: 1 },
                Input::DialTurn {
                    index: 8,
                    delta: -6
                },
            ]
        );
    }

    #[test]
    fn the_fader_dump_seeds_the_dials() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[
                    0xf0, 0x47, 0x7f, 0x4f, 0x61, 0x00, 0x09, 0, 10, 20, 30, 40, 50, 60, 70, 127,
                    0xf7,
                ],
                &[0xb0, 49, 18],
                &[0xb0, 56, 119],
                // A truncated dump is ignored safely.
                &[0xf0, 0x47, 0x7f, 0x4f, 0x61],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialTurn { index: 1, delta: 2 },
                Input::DialTurn {
                    index: 8,
                    delta: -2
                },
            ]
        );
    }

    #[test]
    fn any_mode_but_session_is_undone() {
        for mode in [0x01u8, 0x02, 0x03] {
            let (mut deck, recorder) = open();
            deck.set_face(0, Some(&lit([255, 255, 255]))).unwrap();
            deck.flush().unwrap();
            queue(&recorder, &[&[0x90, 56, 0x7f]]);
            assert_eq!(deck.poll(0).unwrap(), vec![key(0, true)]);
            let before = recorder.writes().len();
            queue(
                &recorder,
                &[&[0xf0, 0x47, 0x7f, 0x4f, 0x62, 0x00, 0x01, mode, 0xf7]],
            );
            // The held pad is let go; session mode is set again and repainted.
            assert_eq!(deck.poll(0).unwrap(), vec![key(0, false)]);
            let writes = &recorder.writes()[before..];
            assert_eq!(
                writes[0],
                vec![0xf0, 0x47, 0x7f, 0x4f, 0x62, 0x00, 0x01, 0x00, 0xf7]
            );
            assert_eq!(writes.len(), 1 + 1 + 16, "mode {mode}");
        }
        // Session mode reported: nothing sent.
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        queue(
            &recorder,
            &[&[0xf0, 0x47, 0x7f, 0x4f, 0x62, 0x00, 0x01, 0x00, 0xf7]],
        );
        assert!(deck.poll(0).unwrap().is_empty());
        assert_eq!(recorder.writes().len(), before);
    }

    #[test]
    fn letting_go_blanks_everything_and_leaves_session_mode() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes.len(), 1 + 16 + 1);
        assert_eq!(
            writes[0],
            vec![
                0xf0, 0x47, 0x7f, 0x4f, 0x24, 0x00, 0x08, 0x00, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0xf7
            ]
        );
        assert_eq!(writes[1], vec![0x90, 100, 0]);
        assert_eq!(writes[9], vec![0x90, 112, 0]);
        assert_eq!(
            writes[17],
            vec![0xf0, 0x47, 0x7f, 0x4f, 0x62, 0x00, 0x01, 0x00, 0xf7]
        );
        // Never opened: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_the_control_port_matches() {
        let model = by_id(APC_MINI_MK2_ID).unwrap();
        let cases = [
            ("APC mini mk2 Control", false, true),
            ("APC mini mk2 Notes", false, false),
            ("APC mini mk2:APC mini mk2 Contr 24:0", false, true),
            ("APC mini mk2:APC mini mk2 Notes 24:1", false, false),
            ("APC mini mk2:APC mini mk2 MIDI 1 24:0", false, true),
            ("APC mini mk2:APC mini mk2 MIDI 2 24:1", false, false),
            ("APC mini mk2", true, true),
            ("2- APC mini mk2", true, true),
            ("MIDIIN2 (APC mini mk2)", true, false),
            ("MIDIOUT2 (APC mini mk2)", true, false),
            // The first APC mini (its own model), and an unrelated port.
            ("APC MINI", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(APC_MINI_MK2_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_places_every_key() {
        let view = ModelView::from(by_id(APC_MINI_MK2_ID).unwrap());
        let cells = view.key_cells.unwrap();
        assert_eq!(cells.len(), 81);
        assert_eq!(cells[0], 0);
        assert_eq!(cells[8], 9);
        assert_eq!(cells[63], 7 * 9 + 7);
        assert_eq!(cells[64], 8);
        assert_eq!(cells[71], 7 * 9 + 8);
        assert_eq!(cells[72], 8 * 9);
        assert_eq!(cells[80], 80);
        let mut sorted = cells.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 81);
        let names = view.key_names.unwrap();
        assert_eq!(
            (names[0], names[64], names[71], names[72], names[80]),
            ("Pad 1", "Clip Stop", "Stop All Clips", "Volume", "Shift")
        );
        assert_eq!(view.lit_keys.unwrap(), (0..80).collect::<Vec<u8>>());
        assert_eq!(view.key_light_colors, "rgb");
        assert_eq!(view.dial_lights, DialLights::None);
        assert!(view.brightness);
    }
}
