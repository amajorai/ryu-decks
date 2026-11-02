//! DJ TechTools' Midi Fighter Spectra, over class-compliant USB MIDI
//! ([`super::midi`]), on its factory configuration.
//!
//! The Spectra is a 4 x 4 grid of arcade buttons, each with an RGB light, six
//! side buttons and four white ground-effect lights under the case. It lists
//! one MIDI port each way, taken to be named after its USB product string like
//! the Twister's: `Midi Fighter Spectra`. [`port_matches`] takes any name
//! containing it, on Windows only the first interface.
//!
//! The firmware keeps four banks of 16 notes and the side buttons switch
//! between them (left previous, right next) without telling the host anything
//! Ryu relies on; they are never keys. Bank `b` (0-3) is notes `36 + 16 * b`
//! to `51 + 16 * b`, numbered from the bottom row up, left to right. Ryu's key
//! `k` (0 at the top left, row by row) is note `36 + 16 * b + (3 - k / 4) * 4 +
//! k % 4` in every bank, so Ryu folds the banks: whichever bank is on show,
//! the same button is the same key.
//!
//! In (factory channels: buttons on 3, system on 4):
//!
//! - a press is `92 <note> <velocity>` with a velocity above 0; a release is
//!   `82 <note> <any velocity>` or `92 <note> 00`. Velocity means nothing
//!   else (the buttons are not velocity-sensitive);
//! - a bank change may be reported as `93 <bank> <nonzero>` or `B3 <bank>
//!   <nonzero>` (bank 0-3); no reference states its status byte, so Ryu only
//!   notes the bank and never takes a channel-4 message as a key.
//!
//! Out, all 3-byte messages; no SysEx, no keep-alive:
//!
//! - a button's light is `92 <note> <palette index>`, written to its note in
//!   all four banks so it is right whichever bank the side buttons show. 1 is
//!   off; each hue has a bright index and, one above it, a dim one: 18 red, 30
//!   orange, 42 yellow, 54 lime, 66 green, 78 celeste, 90 blue, 102 purple,
//!   114 pink. White is 121, with no dim twin that shows white (122 is
//!   reported to show purple). 127 hands the light to the user's own Utility
//!   colour and 0 is a reset; Ryu sends neither. [`palette_index`] picks the
//!   nearest hue;
//! - there is no brightness command: under 50% the keys take the dim index,
//!   and 0% turns every light off;
//! - the ground effect is `93 <10..13> <level>`, one level for all four
//!   lights: 18 is off, 19-33 a steady brightness from the deck light's
//!   luminance and the brightness setting. 34-49 start strobe and pulse
//!   animations, so Ryu clamps to 33.
//!
//! At open Ryu shows bank 1 with `93 00 7F` and writes every light; it waits
//! for no reply, so the shared poll loop never blocks. When Ryu lets go, every
//! button note gets `92 <note> 01` and the ground effect `93 <10..13> 12`.
//!
//! Facts from Mixxx's Midi Fighter Spectra mapping (mixxxdj/mixxx,
//! GPL-2.0-or-later: notes per layer, light and ground-effect values, bank
//! select) and its manual (mixxxdj/manual: the button numbering and side
//! buttons), with the bank-1 notes and palette cross-checked against bbonora's
//! FL Studio script for the Spectra (no formal licence) and the 36-99 note
//! range and note on/off format of the Midi Fighter 3D against KYBIFighter
//! (GPL-3.0; same DJTT family); no code or tables were copied.

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const SPECTRA_ID: &str = "djtt-midi-fighter-spectra";

pub const KEYS: u8 = 16;
pub const ROWS: u8 = 4;
pub const COLS: u8 = 4;
/// The four white ground-effect lights, which take one level.
pub const DECK_LIGHTS: u8 = 4;

/// Note On / Note Off on channel 3 (the buttons), Note On, Note Off and
/// Control Change on channel 4 (the system channel).
const BUTTON_ON: u8 = 0x92;
const BUTTON_OFF: u8 = 0x82;
const SYSTEM_ON: u8 = 0x93;
const SYSTEM_CC: u8 = 0xb3;

/// The first note of bank 1, and the banks the firmware keeps.
const FIRST_NOTE: u8 = 36;
const BANKS: u8 = 4;
/// The system-channel value that selects a bank.
const SELECT: u8 = 0x7f;

/// Palette indices: off, and white (bright and dim alike).
const OFF_INDEX: u8 = 1;
const WHITE_INDEX: u8 = 121;

/// Ground-effect notes and levels: 18 off, 19-33 steady brightness.
const GROUND_NOTES: [u8; DECK_LIGHTS as usize] = [0x10, 0x11, 0x12, 0x13];
const GROUND_OFF: u8 = 18;
const GROUND_MIN: u8 = 19;
const GROUND_MAX: u8 = 33;

/// Under this brightness the keys take their dim index.
const DIM_BELOW: u8 = 50;
/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// Whether a normalised port name is a Spectra's, on Windows its first
/// interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.protocol == Protocol::MidiFighterSpectraMidi
        && name.interface.unwrap_or(1) == 1
        && name.base.contains("Midi Fighter Spectra")
}

/// The note of key `key` (0 at the top left, row by row) in bank `bank`.
#[must_use]
pub const fn note(bank: u8, key: u8) -> u8 {
    FIRST_NOTE + 16 * (bank % BANKS) + (3 - (key % KEYS) / 4) * 4 + key % 4
}

/// The key a button note is, in whichever bank; `None` outside 36-99.
#[must_use]
pub const fn key_of(note: u8) -> Option<u8> {
    if note < FIRST_NOTE || note >= FIRST_NOTE + 16 * BANKS {
        return None;
    }
    let offset = (note - FIRST_NOTE) % 16;
    Some((3 - offset / 4) * 4 + offset % 4)
}

/// Hue (degrees) of each palette colour and its bright index; the dim index
/// is one above.
const HUES: [(i32, u8); 9] = [
    (0, 18),
    (30, 30),
    (60, 42),
    (90, 54),
    (120, 66),
    (195, 78),
    (240, 90),
    (275, 102),
    (320, 114),
];

/// The bright palette index nearest a colour; `None` when it is dark.
#[must_use]
pub fn palette_index(color: [u8; 3]) -> Option<u8> {
    let [red, green, blue] = color.map(i32::from);
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    if max < i32::from(DARK) {
        return None;
    }
    let chroma = max - min;
    // Under a quarter saturated: white or grey.
    if chroma * 4 < max {
        return Some(WHITE_INDEX);
    }
    let hue = if max == red {
        (60 * (green - blue) / chroma).rem_euclid(360)
    } else if max == green {
        120 + 60 * (blue - red) / chroma
    } else {
        240 + 60 * (red - green) / chroma
    };
    let distance = |to: i32| {
        let apart = (hue - to).rem_euclid(360);
        apart.min(360 - apart)
    };
    HUES.iter()
        .min_by_key(|(to, _)| distance(*to))
        .map(|(_, index)| *index)
}

/// The palette index one key's light is sent as.
fn key_value(color: Option<[u8; 3]>, percent: u8) -> u8 {
    if percent == 0 {
        return OFF_INDEX;
    }
    match color.and_then(palette_index) {
        None => OFF_INDEX,
        Some(WHITE_INDEX) => WHITE_INDEX,
        Some(bright) if percent < DIM_BELOW => bright + 1,
        Some(bright) => bright,
    }
}

/// The ground-effect level for the deck light's colour.
fn ground_level(color: Option<[u8; 3]>, percent: u8) -> u8 {
    let Some([red, green, blue]) = color else {
        return GROUND_OFF;
    };
    if red.max(green).max(blue) < DARK || percent == 0 {
        return GROUND_OFF;
    }
    let luminance = (299 * u32::from(red) + 587 * u32::from(green) + 114 * u32::from(blue)) / 1000;
    let scaled = luminance * u32::from(percent.min(100));
    let span = u32::from(GROUND_MAX - GROUND_MIN);
    // Rounded: 255 * 100 is full.
    let above = (scaled * span + 255 * 50) / (255 * 100);
    GROUND_MIN
        .saturating_add(u8::try_from(above).unwrap_or(u8::MAX))
        .min(GROUND_MAX)
}

pub struct Spectra {
    transport: Box<dyn Transport>,
    started: bool,
    /// The bank the deck last reported, for diagnostics only.
    bank: u8,
    percent: u8,
    colors: [Option<[u8; 3]>; KEYS as usize],
    /// The index last sent to each key, in every bank.
    sent: [Option<u8>; KEYS as usize],
    ground: Option<[u8; 3]>,
    ground_sent: Option<u8>,
    parser: Parser,
}

impl Spectra {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            bank: 0,
            percent: 100,
            colors: [None; KEYS as usize],
            sent: [None; KEYS as usize],
            ground: None,
            ground_sent: None,
            parser: Parser::default(),
        }
    }

    /// Shows bank 1 and writes every light: plain writes, no reply awaited.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        self.transport.write(&[SYSTEM_ON, 0x00, SELECT])?;
        self.send_lights()
    }

    /// Sends each light that changed: a key's to its note in all four banks.
    fn send_lights(&mut self) -> DriverResult<()> {
        for key in 0..KEYS {
            let index = usize::from(key);
            let wanted = key_value(self.colors.get(index).copied().flatten(), self.percent);
            if self.sent.get(index).copied().flatten() == Some(wanted) {
                continue;
            }
            for bank in 0..BANKS {
                self.transport
                    .write(&[BUTTON_ON, note(bank, key), wanted])?;
            }
            if let Some(sent) = self.sent.get_mut(index) {
                *sent = Some(wanted);
            }
        }
        let level = ground_level(self.ground, self.percent);
        if self.ground_sent != Some(level) {
            for number in GROUND_NOTES {
                self.transport.write(&[SYSTEM_ON, number, level])?;
            }
            self.ground_sent = Some(level);
        }
        Ok(())
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        match status {
            BUTTON_ON | BUTTON_OFF => {
                if let Some(index) = key_of(number) {
                    events.push(Input::Key {
                        index,
                        pressed: status == BUTTON_ON && value > 0,
                    });
                }
            }
            SYSTEM_ON | SYSTEM_CC if number < BANKS && value > 0 => self.bank = number,
            _ => {}
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

impl Driver for Spectra {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_started()?;
        let mut events = Vec::new();
        for message in &self.read(timeout_ms)? {
            self.receive(message, &mut events);
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

    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        self.ground = color;
        Ok(())
    }
}

impl Drop for Spectra {
    /// Turns every button light off in all four banks, and the ground effect.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for bank in 0..BANKS {
            for key in 0..KEYS {
                let _ = self
                    .transport
                    .write(&[BUTTON_ON, note(bank, key), OFF_INDEX]);
            }
        }
        for number in GROUND_NOTES {
            let _ = self.transport.write(&[SYSTEM_ON, number, GROUND_OFF]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    fn open() -> (Spectra, Recorder) {
        let recorder = Recorder::default();
        let deck = Spectra::new(by_id(SPECTRA_ID).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// A started deck, with the start's writes cleared.
    fn ready() -> (Spectra, Recorder) {
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
            "Midi Fighter Spectra:Midi Fighter Spectra MIDI 1 24:0",
            "Midi Fighter Spectra",
            "2- Midi Fighter Spectra",
        ] {
            let model = identify_midi(&normalise(raw), false).unwrap();
            assert_eq!(model.id, SPECTRA_ID, "{raw}");
        }
        assert_eq!(
            identify_midi(&normalise("Midi Fighter Spectra"), true)
                .unwrap()
                .id,
            SPECTRA_ID
        );
        assert!(identify_midi(&normalise("MIDIIN2 (Midi Fighter Spectra)"), true).is_none());
        assert!(identify_midi(&normalise("Midi Fighter 3D"), false).is_none());
        assert_ne!(
            identify_midi(&normalise("Midi Fighter Twister"), false)
                .unwrap()
                .id,
            SPECTRA_ID
        );
        let model = by_id(SPECTRA_ID).unwrap();
        assert!(model.protocol.is_midi());
        assert_eq!(model.dial_lights, DialLights::None);
        assert_eq!((model.dials, model.deck_lights), (0, DECK_LIGHTS));
        assert!(model.key_lights && model.image.is_none());
        let view = ModelView::from(model);
        assert!(view.brightness);
        assert_eq!(view.usb_ids, vec!["2580:0006".to_owned()]);
    }

    #[test]
    fn notes_run_bottom_up_in_four_banks() {
        // Bank 1: the bottom row is 36-39, the top row 48-51.
        assert_eq!(note(0, 0), 48);
        assert_eq!(note(0, 3), 51);
        assert_eq!(note(0, 12), 36);
        assert_eq!(note(0, 15), 39);
        // Bank 3 (Mixxx's sampler layer): top row 0x50-0x53, bottom 0x44-0x47.
        assert_eq!(note(2, 0), 0x50);
        assert_eq!(note(2, 12), 0x44);
        assert_eq!(note(3, 15), 87);
        for bank in 0..BANKS {
            for index in 0..KEYS {
                assert_eq!(key_of(note(bank, index)), Some(index));
            }
        }
        assert_eq!(key_of(35), None);
        assert_eq!(key_of(100), None);
    }

    #[test]
    fn presses_and_releases_fold_the_banks() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                // Top left in bank 1 and bank 4.
                &[0x92, 48, 0x7f],
                &[0x82, 48, 0x40],
                &[0x92, 96, 0x7f],
                &[0x92, 96, 0x00],
                // Bottom right in bank 2, bank 3.
                &[0x92, 55, 0x7f],
                &[0x82, 71, 0x00],
                // Running status, a real-time byte and a short read.
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
                key(15, true),
                key(15, false),
                key(12, true),
                key(12, false),
            ]
        );
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn bank_reports_and_other_messages_are_never_keys() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0x93, 2, 0x7f],
                &[0x83, 2, 0x00],
                &[0xb3, 1, 0x7f],
                &[0xb3, 0, 0x00],
                &[0x93, 0x10, 0x7f],
                // Out of range notes and other channels.
                &[0x92, 35, 0x7f],
                &[0x92, 100, 0x7f],
                &[0x90, 48, 0x7f],
                &[0xb2, 20, 0x7f],
                &[0xf0, 0x7e, 0x7f, 0x06, 0x02, 0xf7],
            ],
        );
        assert!(deck.poll(0).unwrap().is_empty());
        assert_eq!(deck.bank, 1);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn opening_selects_bank_one_and_writes_every_light() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&face([255, 0, 0]))).unwrap();
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(writes[0], vec![0x93, 0x00, 0x7f]);
        assert_eq!(writes.len(), 1 + 64 + 4);
        assert_eq!(
            writes[1..5],
            [
                vec![0x92, 48, 18],
                vec![0x92, 64, 18],
                vec![0x92, 80, 18],
                vec![0x92, 96, 18]
            ]
        );
        // A key with no face is dark.
        assert_eq!(writes[5], vec![0x92, 49, 1]);
        assert_eq!(
            writes[65..],
            [
                vec![0x93, 0x10, 18],
                vec![0x93, 0x11, 18],
                vec![0x93, 0x12, 18],
                vec![0x93, 0x13, 18]
            ]
        );
        // Nothing changed: nothing more is sent.
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), writes.len());
    }

    #[test]
    fn a_light_goes_to_all_four_banks_and_dims_below_half() {
        let (mut deck, recorder) = ready();
        deck.set_face(15, Some(&face([0, 0, 255]))).unwrap();
        deck.set_face(1, Some(&face([10, 10, 10]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x92, 39, 90],
                vec![0x92, 55, 90],
                vec![0x92, 71, 90],
                vec![0x92, 87, 90]
            ]
        );
        recorder.0.lock().unwrap().writes.clear();
        deck.set_brightness(30).unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x92, 39, 91],
                vec![0x92, 55, 91],
                vec![0x92, 71, 91],
                vec![0x92, 87, 91]
            ]
        );
        recorder.0.lock().unwrap().writes.clear();
        deck.set_brightness(0).unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x92, 39, 1],
                vec![0x92, 55, 1],
                vec![0x92, 71, 1],
                vec![0x92, 87, 1]
            ]
        );
    }

    #[test]
    fn colours_take_the_nearest_palette_hue() {
        assert_eq!(palette_index([255, 0, 0]), Some(18));
        assert_eq!(palette_index([255, 128, 0]), Some(30));
        assert_eq!(palette_index([255, 255, 0]), Some(42));
        assert_eq!(palette_index([128, 255, 0]), Some(54));
        assert_eq!(palette_index([0, 255, 0]), Some(66));
        assert_eq!(palette_index([0, 200, 255]), Some(78));
        assert_eq!(palette_index([0, 0, 255]), Some(90));
        assert_eq!(palette_index([128, 0, 255]), Some(102));
        assert_eq!(palette_index([255, 0, 160]), Some(114));
        assert_eq!(palette_index([255, 0, 20]), Some(18));
        assert_eq!(palette_index([255, 255, 255]), Some(121));
        assert_eq!(palette_index([128, 128, 128]), Some(121));
        assert_eq!(palette_index([20, 0, 31]), None);
        // White has no dim twin; dark and unbound keys are off.
        assert_eq!(key_value(Some([255, 255, 255]), 20), 121);
        assert_eq!(key_value(Some([0, 0, 0]), 100), 1);
        assert_eq!(key_value(None, 100), 1);
        assert_eq!(key_value(Some([255, 0, 0]), 49), 19);
        assert_eq!(key_value(Some([255, 0, 0]), 50), 18);
    }

    #[test]
    fn the_ground_effect_is_off_at_18_and_lit_at_19_to_33() {
        let (mut deck, recorder) = ready();
        let ground = |level: u8| -> Vec<Vec<u8>> {
            GROUND_NOTES
                .iter()
                .map(|&number| vec![0x93, number, level])
                .collect()
        };
        deck.set_deck_light(Some([255, 255, 255])).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes(), ground(33));
        recorder.0.lock().unwrap().writes.clear();
        deck.set_brightness(50).unwrap();
        assert_eq!(recorder.writes(), ground(26));
        recorder.0.lock().unwrap().writes.clear();
        deck.set_deck_light(Some([0, 0, 255])).unwrap();
        deck.set_brightness(100).unwrap();
        assert_eq!(recorder.writes(), ground(21));
        recorder.0.lock().unwrap().writes.clear();
        deck.set_deck_light(Some([10, 10, 10])).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes(), ground(18));
        recorder.0.lock().unwrap().writes.clear();
        deck.set_deck_light(None).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
        for percent in 0..=100 {
            for value in (0..=255u8).step_by(5) {
                let level = ground_level(Some([value, value, value]), percent);
                assert!(level == 18 || (19..=33).contains(&level));
            }
        }
    }

    #[test]
    fn letting_go_turns_every_light_off() {
        let (mut deck, recorder) = ready();
        deck.set_face(0, Some(&face([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 64 + 4);
        let mut notes: Vec<u8> = writes[..64]
            .iter()
            .map(|write| {
                assert_eq!((write[0], write[2]), (0x92, 1));
                write[1]
            })
            .collect();
        notes.sort_unstable();
        assert_eq!(notes, (36..=99).collect::<Vec<u8>>());
        assert_eq!(writes[64], vec![0x93, 0x10, 18]);
        assert_eq!(writes[67], vec![0x93, 0x13, 18]);
    }

    #[test]
    fn a_deck_never_started_sends_nothing_on_letting_go() {
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }
}
