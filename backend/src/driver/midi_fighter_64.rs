//! DJ TechTools' Midi Fighter 64, over class-compliant USB MIDI
//! ([`super::midi`]), on its factory configuration.
//!
//! The Midi Fighter 64 is an 8 x 8 grid of arcade buttons, each with a
//! palette-colour light. Its MIDI port is found by the name fragment
//! `Fighter 64`, in any case; the full name is taken to be `Midi Fighter 64`
//! after its USB product string, like the Twister's and the Spectra's.
//! [`port_matches`] takes any name containing it, on Windows only the first
//! interface.
//!
//! The grid is two 4-wide halves, each numbered from the bottom row up: the
//! left half's bottom row is notes 36-39 and its top row 64-67, the right
//! half's bottom row 68-71 and its top row 96-99. Ryu's key `k` (0 at the top
//! left, row by row) is note `(36 + c or 68 + c - 4) + (7 - r) * 4` with `r =
//! k / 8`, `c = k % 8`.
//!
//! In (factory settings): a press is `92 <note> 7F` on channel 3, or `91
//! <note> 7F` on channel 2 while the upper-right bank is on (holding a corner
//! button for 2 s switches bank; the upper-left bank is channel 3). A release
//! is `82`/`81 <note> <any>` or a Note On with velocity 0. Ryu folds both
//! channels: the same note is the same key. Every other message is ignored.
//!
//! Out, all 3-byte messages; no SysEx, no handshake, no keep-alive:
//!
//! - a button's colour is `92 <note> <palette index>` on channel 3: the
//!   128-entry Launchpad palette, 0 black (off), 3 white, 5 red, 45 blue;
//! - then its mode, `93 <note - 36> <mode>` on channel 4: 18-33 is a steady
//!   brightness from off to full. 34-53 start blinks, pulses and animations,
//!   which Ryu never sends. The brightness setting maps to 18-33.
//!
//! At open every colour and mode is written, with no reply awaited, so the
//! shared poll loop never blocks. When Ryu lets go, every button gets colour 0
//! and mode 33 (full), so no light is left dimmed for the next app.
//!
//! Facts from FMMT666's launchpad.py (`MidiFighter64` class, README and
//! colour chart; CC BY 4.0); no code or tables were copied. The palette table
//! is the repo's own, in [`super::akai_apc40`].

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const MF64_ID: &str = "djtt-midi-fighter-64";

pub const KEYS: u8 = 64;
pub const ROWS: u8 = 8;
pub const COLS: u8 = 8;

/// Note On / Off on channel 3 (the upper-left bank, and every light) and
/// channel 2 (the upper-right bank's input).
const BUTTON_ON: u8 = 0x92;
const BUTTON_OFF: u8 = 0x82;
const BANK2_ON: u8 = 0x91;
const BANK2_OFF: u8 = 0x81;
/// Note On on channel 4: a light's mode.
const MODE: u8 = 0x93;

const FIRST_NOTE: u8 = 36;
const LAST_NOTE: u8 = 99;
/// The right half's bottom-left note.
const RIGHT_NOTE: u8 = 68;

/// Steady brightness modes: 18 is off, 33 (0x21) fully on.
const MODE_OFF: u8 = 18;
const MODE_FULL: u8 = 0x21;
const MODE_STEPS: u16 = 15;

const OFF_INDEX: u8 = 0;
/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// Whether a normalised port name is a Midi Fighter 64's, on Windows its first
/// interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.protocol == Protocol::MidiFighter64Midi
        && name.interface.unwrap_or(1) == 1
        && name.base.to_ascii_lowercase().contains("fighter 64")
}

/// The note of key `key` (0 at the top left, row by row).
#[must_use]
pub const fn note(key: u8) -> u8 {
    let key = key % KEYS;
    let row = key / COLS;
    let col = key % COLS;
    let bottom = if col < 4 {
        FIRST_NOTE + col
    } else {
        RIGHT_NOTE + col - 4
    };
    bottom + (7 - row) * 4
}

/// The key a button note is; `None` outside 36-99.
#[must_use]
pub const fn key_of(note: u8) -> Option<u8> {
    if note < FIRST_NOTE || note > LAST_NOTE {
        return None;
    }
    let offset = note - FIRST_NOTE;
    let col = offset % 4 + if note >= RIGHT_NOTE { 4 } else { 0 };
    let row = 7 - (offset % 32) / 4;
    Some(row * COLS + col)
}

/// The palette index a key's colour is sent as: 0 when dark or unbound.
fn color_value(color: Option<[u8; 3]>) -> u8 {
    match color {
        Some(color) if color.iter().any(|channel| *channel >= DARK) => {
            super::akai_apc40::palette_index(color)
        }
        _ => OFF_INDEX,
    }
}

/// The steady-brightness mode for a brightness percentage: 18-33.
fn mode_value(percent: u8) -> u8 {
    let steps = (u16::from(percent.min(100)) * MODE_STEPS + 50) / 100;
    MODE_OFF
        .saturating_add(u8::try_from(steps).unwrap_or(u8::MAX))
        .min(MODE_FULL)
}

pub struct MidiFighter64 {
    transport: Box<dyn Transport>,
    started: bool,
    percent: u8,
    colors: [Option<[u8; 3]>; KEYS as usize],
    /// The colour index and mode last sent to each key.
    sent_colors: [Option<u8>; KEYS as usize],
    sent_modes: [Option<u8>; KEYS as usize],
    parser: Parser,
}

impl MidiFighter64 {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            percent: 100,
            colors: [None; KEYS as usize],
            sent_colors: [None; KEYS as usize],
            sent_modes: [None; KEYS as usize],
            parser: Parser::default(),
        }
    }

    /// Writes every light: plain writes, no reply awaited.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        self.send_lights()
    }

    /// Sends each colour and mode that changed, the colour first.
    fn send_lights(&mut self) -> DriverResult<()> {
        let mode = mode_value(self.percent);
        for key in 0..KEYS {
            let index = usize::from(key);
            let number = note(key);
            let color = color_value(self.colors.get(index).copied().flatten());
            if let Some(sent) = self.sent_colors.get_mut(index) {
                if *sent != Some(color) {
                    self.transport.write(&[BUTTON_ON, number, color])?;
                    *sent = Some(color);
                }
            }
            if let Some(sent) = self.sent_modes.get_mut(index) {
                if *sent != Some(mode) {
                    self.transport.write(&[MODE, number - FIRST_NOTE, mode])?;
                    *sent = Some(mode);
                }
            }
        }
        Ok(())
    }

    fn receive(message: &Message, events: &mut Vec<Input>) {
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        let pressed = match status {
            BUTTON_ON | BANK2_ON => value > 0,
            BUTTON_OFF | BANK2_OFF => false,
            _ => return,
        };
        if let Some(index) = key_of(number) {
            events.push(Input::Key { index, pressed });
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
}

impl Driver for MidiFighter64 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_started()?;
        let mut events = Vec::new();
        for message in &self.read(timeout_ms)? {
            Self::receive(message, &mut events);
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(slot) = self.colors.get_mut(usize::from(key)) {
            *slot = face.map(|face| face.color);
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        if !self.started {
            return self.ensure_started();
        }
        self.send_lights()
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.percent = percent.min(100);
        self.flush()
    }
}

impl Drop for MidiFighter64 {
    /// Turns every light off and leaves each at full brightness mode.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for key in 0..KEYS {
            let number = note(key);
            let _ = self.transport.write(&[BUTTON_ON, number, OFF_INDEX]);
            let _ = self
                .transport
                .write(&[MODE, number - FIRST_NOTE, MODE_FULL]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    fn open() -> (MidiFighter64, Recorder) {
        let recorder = Recorder::default();
        let deck = MidiFighter64::new(by_id(MF64_ID).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// A started deck, with the start's writes cleared.
    fn ready() -> (MidiFighter64, Recorder) {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        (deck, recorder)
    }

    fn queue(recorder: &Recorder, messages: &[&[u8]]) {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    fn face(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    #[test]
    fn the_model_is_found_by_its_port_name() {
        for raw in [
            "Midi Fighter 64",
            "Midi Fighter 64:Midi Fighter 64 MIDI 1 24:0",
            "2- Midi Fighter 64",
        ] {
            let model = identify_midi(&normalise(raw), false).unwrap();
            assert_eq!(model.id, MF64_ID, "{raw}");
        }
        assert_eq!(
            identify_midi(&normalise("Midi Fighter 64"), true)
                .unwrap()
                .id,
            MF64_ID
        );
        assert!(identify_midi(&normalise("MIDIIN2 (Midi Fighter 64)"), true).is_none());
        assert!(identify_midi(&normalise("Midi Fighter 3D"), false).is_none());
        for other in ["Midi Fighter Spectra", "Midi Fighter Twister"] {
            assert_ne!(
                identify_midi(&normalise(other), false).unwrap().id,
                MF64_ID,
                "{other}"
            );
        }
        let spectra = by_id(super::super::midi_fighter_spectra::SPECTRA_ID).unwrap();
        let twister = by_id(super::super::midi_fighter_twister::TWISTER_ID).unwrap();
        let name = normalise("Midi Fighter 64");
        assert!(!super::super::midi_fighter_spectra::port_matches(
            spectra, &name
        ));
        assert!(!super::super::midi_fighter_twister::port_matches(
            twister, &name
        ));
        let model = by_id(MF64_ID).unwrap();
        assert!(model.protocol.is_midi());
        assert_eq!(model.dial_lights, DialLights::None);
        assert_eq!((model.dials, model.deck_lights), (0, 0));
        assert_eq!(model.dial_light_colors(), 0);
        assert!(model.key_lights && model.image.is_none());
        let view = ModelView::from(model);
        assert!(view.brightness);
        assert!(view.usb_ids.is_empty());
    }

    #[test]
    fn notes_run_bottom_up_in_two_halves() {
        assert_eq!(note(0), 64);
        assert_eq!(note(3), 67);
        assert_eq!(note(4), 96);
        assert_eq!(note(7), 99);
        assert_eq!(note(56), 36);
        assert_eq!(note(59), 39);
        assert_eq!(note(60), 68);
        assert_eq!(note(63), 71);
        let mut notes: Vec<u8> = (0..KEYS).map(note).collect();
        for index in 0..KEYS {
            assert_eq!(key_of(note(index)), Some(index));
        }
        notes.sort_unstable();
        assert_eq!(notes, (36..=99).collect::<Vec<u8>>());
        assert_eq!(key_of(35), None);
        assert_eq!(key_of(100), None);
    }

    #[test]
    fn presses_and_releases_fold_both_channels() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                // Top left on channel 3, then channel 2.
                &[0x92, 64, 0x7f],
                &[0x82, 64, 0x40],
                &[0x91, 64, 0x7f],
                &[0x81, 64, 0x00],
                // Bottom right; a Note On with velocity 0 is a release.
                &[0x92, 71, 0x7f],
                &[0x92, 71, 0x00],
                &[0x91, 99, 0x7f],
                &[0x91, 99, 0x00],
                // Running status, a real-time byte and a split read.
                &[0x92, 36, 0x7f, 0xf8, 36],
                &[0x00],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(0, true),
                key(0, false),
                key(63, true),
                key(63, false),
                key(7, true),
                key(7, false),
                key(56, true),
                key(56, false),
            ]
        );
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn other_messages_are_never_keys() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0x93, 0, 0x7f],
                &[0x83, 0, 0x00],
                &[0xb3, 1, 0x7f],
                &[0xb2, 64, 0x7f],
                &[0x90, 64, 0x7f],
                &[0x92, 35, 0x7f],
                &[0x92, 100, 0x7f],
                &[0xf0, 0x7e, 0x7f, 0x06, 0x02, 0xf7],
            ],
        );
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn opening_writes_every_colour_then_mode() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&face([255, 0, 0]))).unwrap();
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(writes.len(), 128);
        assert_eq!(writes[0], vec![0x92, 64, 5]);
        assert_eq!(writes[1], vec![0x93, 28, 0x21]);
        // A key with no face is black, at full brightness.
        assert_eq!(writes[2], vec![0x92, 65, 0]);
        assert_eq!(writes[3], vec![0x93, 29, 0x21]);
        assert_eq!(writes[127], vec![0x93, 35, 0x21]);
        for pair in writes.chunks(2) {
            assert_eq!((pair[0][0], pair[1][0]), (0x92, 0x93));
            assert_eq!(pair[0][1] - 36, pair[1][1]);
        }
        // Nothing changed: nothing more is sent.
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), writes.len());
    }

    #[test]
    fn only_changed_colours_and_modes_are_sent() {
        let (mut deck, recorder) = ready();
        deck.set_face(63, Some(&face([0, 0, 255]))).unwrap();
        deck.set_face(1, Some(&face([10, 10, 10]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes(), vec![vec![0x92, 71, 45]]);
        recorder.0.lock().unwrap().writes.clear();
        deck.set_brightness(30).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 64);
        assert!(writes
            .iter()
            .all(|write| write[0] == 0x93 && write[2] == 23));
        recorder.0.lock().unwrap().writes.clear();
        deck.set_brightness(0).unwrap();
        assert!(recorder.writes().iter().all(|write| write[2] == 18));
        recorder.0.lock().unwrap().writes.clear();
        deck.set_brightness(0).unwrap();
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn brightness_maps_to_the_steady_modes() {
        assert_eq!(mode_value(0), 18);
        assert_eq!(mode_value(30), 23);
        assert_eq!(mode_value(100), 33);
        assert_eq!(mode_value(255), 33);
        for percent in 0..=100 {
            assert!((18..=33).contains(&mode_value(percent)));
        }
    }

    #[test]
    fn colours_take_the_launchpad_palette() {
        assert_eq!(color_value(Some([255, 0, 0])), 5);
        assert_eq!(color_value(Some([255, 255, 255])), 3);
        assert_eq!(color_value(Some([0, 0, 255])), 45);
        assert_eq!(color_value(Some([0, 255, 0])), 21);
        assert_eq!(color_value(Some([0, 0, 0])), 0);
        assert_eq!(color_value(Some([20, 0, 31])), 0);
        assert_eq!(color_value(None), 0);
    }

    #[test]
    fn letting_go_turns_every_light_off_at_full_mode() {
        let (mut deck, recorder) = ready();
        deck.set_face(0, Some(&face([0, 255, 0]))).unwrap();
        deck.set_brightness(10).unwrap();
        recorder.0.lock().unwrap().writes.clear();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 128);
        let mut notes = Vec::new();
        for pair in writes.chunks(2) {
            assert_eq!((pair[0][0], pair[0][2]), (0x92, 0));
            assert_eq!(pair[1], vec![0x93, pair[0][1] - 36, 0x21]);
            notes.push(pair[0][1]);
        }
        notes.sort_unstable();
        assert_eq!(notes, (36..=99).collect::<Vec<u8>>());
    }

    #[test]
    fn a_deck_never_started_sends_nothing_on_letting_go() {
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }
}
