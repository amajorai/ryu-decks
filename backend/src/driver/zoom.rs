//! The Zoom R16 and R24 recorders as DAW control surfaces, over class-compliant
//! USB MIDI ([`super::midi`]), in their Mackie Control mode.
//!
//! The recorder lists a MIDI port only after it is put in its audio interface
//! mode on the device (USB > AUDIO I/F > EXECUTE) and its control surface is
//! set to Mackie Control; in storage mode there is none. The R16 and R24 share
//! one USB id (1686:00dd) and one port name, `ZOOM R16_R24` (or `ZOOM R16_R24
//! Audio Interface`), so they are one model here. [`port_matches`] takes any
//! name containing `R16_R24` in any case, on Windows the first interface only.
//!
//! In, on MIDI channel 1: every key is a note, pressed by a Note On above
//! velocity 0 and released by velocity 0 or a Note Off. The eight track
//! status keys send the Mackie Rec arm (`00`-`07`), Solo (`08`-`0F`) or Mute
//! (`10`-`17`) note of their track, whichever the recorder's MASTER status key
//! has chosen on the device, so all three ranges press the same key. The
//! AUTO PUNCH I/O, A-B REPEAT, MARKER |<<, MARKER >>| and MARK keys are F1-F5
//! (`36`-`3A`), the 1-8Tr and 9-16Tr keys Bank < and > (`2E`, `2F`), the
//! transport `5B`-`5F` and the cursor keys `60`-`63`. Faders 1-8 are pitch bend
//! on channels 1-8 and the master fader on channel 9; they have no motor and
//! are absolute, so each one's first value only seeds it and each later one
//! turns its dial a step per two units of the upper seven bits. The jog dial
//! is `CC 3C`, sign and magnitude: 1-63 clockwise by that many, 65-127
//! anticlockwise by the value less 64.
//!
//! Out, on channel 1: a status key's light may be lit by a Note On at velocity
//! 127 (0 off) on its Rec, Solo and Mute notes together; whether the recorder
//! shows it is not confirmed. There is no handshake, SysEx, display,
//! brightness command or keep-alive, and Ryu never sends pitch bend.
//!
//! Facts from DrivenByMoss's Mackie Control support and its Zoom R16 profile
//! (LGPL-3.0), nattheriddle1's FL-Studio-Zoom-R16 script (MIT), bebejane's
//! ZoomR16XL Ableton script (no licence; facts only), the Linux kernel's
//! USB audio quirks table (GPL-2.0) and Zoom's R16 and R24 manuals; no code
//! was copied. Untested on hardware: the jog direction and the status lights
//! most of all.

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const ZOOM_R16_R24_ID: &str = "zoom-r16-r24";

/// The eight track status keys (0-7), F1-F5 (8-12), the two bank keys
/// (13-14), the transport (15-19) and the cursor keys (20-23).
pub const KEYS: u8 = 24;
/// Faders 1-8 (0-7), the master fader (8) and the jog dial (9).
pub const DIALS: u8 = 10;
pub const ROWS: u8 = 3;
pub const COLS: u8 = 9;

/// The status keys, the only ones with a light.
const STATUS_KEYS: u8 = 8;
/// The channel faders and the master, one pitch bend channel each.
const FADERS: u8 = 9;
const JOG_DIAL: u8 = 9;
/// Where each grid row starts, by key: status keys, F1-F5 and bank, then the
/// transport and cursor keys.
const ROW_STARTS: [u8; ROWS as usize] = [0, 8, 15];

/// The note of each key from key 8 on, in key order: F1-F5, Bank < and >,
/// REW, FF, STOP, PLAY, REC, then up, down, left and right.
const OTHER_NOTES: [u8; (KEYS - STATUS_KEYS) as usize] = [
    0x36, 0x37, 0x38, 0x39, 0x3a, 0x2e, 0x2f, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60, 0x61, 0x62, 0x63,
];

const NAMES: [&str; KEYS as usize] = [
    "Status 1",
    "Status 2",
    "Status 3",
    "Status 4",
    "Status 5",
    "Status 6",
    "Status 7",
    "Status 8",
    "Auto Punch I/O",
    "A-B Repeat",
    "Marker |<<",
    "Marker >>|",
    "Mark",
    "1-8Tr",
    "9-16Tr",
    "REW",
    "FF",
    "STOP",
    "PLAY",
    "REC",
    "Up",
    "Down",
    "Left",
    "Right",
];

/// The three Mackie notes (Rec arm, Solo, Mute) a status key may send.
const STATUS_RANGES: u8 = 3;
/// The transport notes, cleared with the status notes as the deck is taken
/// and let go (their lights are not driven otherwise).
const TRANSPORT_NOTES: std::ops::RangeInclusive<u8> = 0x5b..=0x5f;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const PITCH_BEND: u8 = 0xe0;
const JOG_CC: u8 = 0x3c;
/// A jog value: the low six bits count, bit 6 says anticlockwise.
const TURN_BACK: u8 = 0x40;

const LED_OFF: u8 = 0;
const LED_ON: u8 = 0x7f;

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Fader units (of 128) per dial step: 64 steps from end to end.
const FADER_STEP: i16 = 2;
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

fn is_zoom(model: &DeviceModel) -> bool {
    model.protocol == Protocol::ZoomR16Midi
}

/// Whether a normalised port name is the recorder's: `R16_R24` in any case
/// (`ZOOM R16_R24`, `ZOOM R16_R24 Audio Interface`), and on Windows the
/// first interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    is_zoom(model)
        && name.interface.unwrap_or(1) == 1
        && name.base.to_ascii_uppercase().contains("R16_R24")
}

/// The legend on each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    is_zoom(model).then(|| NAMES.to_vec())
}

/// Only the eight status keys may have a light.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    is_zoom(model).then(|| (0..STATUS_KEYS).collect())
}

/// Where each key sits on the 3 x 9 grid: the status keys on top, F1-F5 and
/// the bank keys below them, then the transport and cursor keys.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    is_zoom(model).then(|| {
        (0..KEYS)
            .map(|key| {
                let row = ROW_STARTS
                    .iter()
                    .rposition(|start| key >= *start)
                    .unwrap_or(0);
                let start = ROW_STARTS.get(row).copied().unwrap_or(0);
                u16::try_from(row).unwrap_or(0) * u16::from(COLS) + u16::from(key - start)
            })
            .collect()
    })
}

/// The key a note presses: any of a track's three status notes is its
/// status key.
fn key_of_note(note: u8) -> Option<u8> {
    if note < STATUS_RANGES * STATUS_KEYS {
        return Some(note % STATUS_KEYS);
    }
    (STATUS_KEYS..)
        .zip(OTHER_NOTES)
        .find(|(_, candidate)| *candidate == note)
        .map(|(key, _)| key)
}

/// How far one jog message turns: sign and magnitude, 0 and 64 none.
fn jog_delta(value: u8) -> i8 {
    let count = i8::try_from(value & (TURN_BACK - 1)).unwrap_or(0);
    if value & TURN_BACK == 0 {
        count
    } else {
        -count
    }
}

/// On for any colour that is not dark.
fn is_lit(color: Option<[u8; 3]>) -> bool {
    color.is_some_and(|color| color.iter().any(|channel| *channel >= DARK))
}

/// Every note Ryu clears: the status keys' three ranges and the transport.
fn clear_notes() -> impl Iterator<Item = u8> {
    (0..STATUS_RANGES * STATUS_KEYS).chain(TRANSPORT_NOTES)
}

pub struct ZoomR16 {
    transport: Box<dyn Transport>,
    /// Every light cleared since open.
    started: bool,
    /// Each status key's light as staged, then as last sent.
    leds: [bool; STATUS_KEYS as usize],
    sent_leds: [bool; STATUS_KEYS as usize],
    held: [bool; KEYS as usize],
    /// Each fader's last position (upper seven bits); `None` until it moves.
    faders: [Option<u8>; FADERS as usize],
    /// Each fader's movement not yet turned into a whole step.
    rests: [i16; FADERS as usize],
    parser: Parser,
}

impl ZoomR16 {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            leds: [false; STATUS_KEYS as usize],
            sent_leds: [false; STATUS_KEYS as usize],
            held: [false; KEYS as usize],
            faders: [None; FADERS as usize],
            rests: [0; FADERS as usize],
            parser: Parser::default(),
        }
    }

    /// Every light off, once, from the first poll or flush.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if self.started {
            return Ok(());
        }
        for note in clear_notes() {
            self.transport.write(&[NOTE_ON, note, LED_OFF])?;
        }
        self.sent_leds = [false; STATUS_KEYS as usize];
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

    /// A fader's absolute position: its first seeds it, each later one turns
    /// its dial a step per two units moved.
    fn move_fader(&mut self, fader: u8, value: u8, events: &mut Vec<Input>) {
        let (Some(position), Some(rest)) = (
            self.faders.get_mut(usize::from(fader)),
            self.rests.get_mut(usize::from(fader)),
        ) else {
            return;
        };
        let Some(old) = position.replace(value) else {
            return;
        };
        let moved = *rest + i16::from(value) - i16::from(old);
        let steps = moved / FADER_STEP;
        *rest = moved - steps * FADER_STEP;
        if steps != 0 {
            events.push(Input::DialTurn {
                index: fader,
                delta: i8::try_from(steps.clamp(-127, 127)).unwrap_or(0),
            });
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        // The recorder sends no SysEx.
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        let channel = status & 0x0f;
        if status & 0xf0 == PITCH_BEND {
            // `number` is the low seven bits; the upper seven are enough.
            if channel < FADERS && value <= 0x7f {
                self.move_fader(channel, value, events);
            }
            return;
        }
        if channel != 0 {
            return;
        }
        match status & 0xf0 {
            NOTE_ON | NOTE_OFF => {
                let pressed = status & 0xf0 == NOTE_ON && value > 0;
                if let Some(key) = key_of_note(number) {
                    self.press(key, pressed, events);
                }
            }
            CONTROL_CHANGE if number == JOG_CC => {
                let delta = jog_delta(value);
                if delta != 0 {
                    events.push(Input::DialTurn {
                        index: JOG_DIAL,
                        delta,
                    });
                }
            }
            _ => {}
        }
    }
}

impl Driver for ZoomR16 {
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

    /// Only a status key's light is staged; the other keys have none.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(led) = self.leds.get_mut(usize::from(key)) {
            *led = is_lit(face.map(|face| face.color));
        }
        Ok(())
    }

    /// Every light off the first time, then the three notes of each status
    /// key whose light changed.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        for ((key, lit), sent) in (0u8..).zip(self.leds).zip(&mut self.sent_leds) {
            if *sent == lit {
                continue;
            }
            let velocity = if lit { LED_ON } else { LED_OFF };
            for range in 0..STATUS_RANGES {
                self.transport
                    .write(&[NOTE_ON, range * STATUS_KEYS + key, velocity])?;
            }
            *sent = lit;
        }
        Ok(())
    }

    /// The recorder has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for ZoomR16 {
    /// Turns every light off.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for note in clear_notes() {
            let _ = self.transport.write(&[NOTE_ON, note, LED_OFF]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, ModelView};

    /// What the first flush sends: notes 00-17 and 5B-5F off.
    const INIT: usize = 24 + 5;

    fn open() -> (ZoomR16, Recorder) {
        let recorder = Recorder::default();
        let deck = ZoomR16::new(by_id(ZOOM_R16_R24_ID).unwrap(), Box::new(recorder.clone()));
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
    fn the_first_flush_clears_every_light_and_nothing_is_sent_at_open() {
        let (mut deck, recorder) = open();
        assert!(recorder.writes().is_empty());
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), INIT);
        let notes: Vec<u8> = writes
            .iter()
            .map(|write| {
                assert_eq!((write.len(), write[0], write[2]), (3, 0x90, 0));
                write[1]
            })
            .collect();
        let expected: Vec<u8> = (0x00..=0x17).chain(0x5b..=0x5f).collect();
        assert_eq!(notes, expected);
        // Nothing more until something changes.
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), INIT);
    }

    #[test]
    fn the_first_poll_clears_too() {
        let (mut deck, recorder) = open();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), INIT);
    }

    #[test]
    fn a_status_light_writes_its_three_notes_and_other_faces_write_nothing() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        // No light on these, and out of range.
        deck.set_face(18, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(8, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(24, Some(&lit([0, 255, 0]))).unwrap();
        // Black, near-black and no face stay off.
        deck.set_face(1, Some(&lit([0, 0, 0]))).unwrap();
        deck.set_face(2, Some(&lit([20, 20, 20]))).unwrap();
        deck.set_face(3, None).unwrap();
        assert_eq!(recorder.writes().len(), INIT);
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[INIT..],
            [
                vec![0x90, 0x00, 0x7f],
                vec![0x90, 0x08, 0x7f],
                vec![0x90, 0x10, 0x7f],
            ]
        );
        deck.set_face(0, None).unwrap();
        deck.set_face(7, Some(&lit([0, 0, 200]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[INIT + 3..],
            [
                vec![0x90, 0x00, 0x00],
                vec![0x90, 0x08, 0x00],
                vec![0x90, 0x10, 0x00],
                vec![0x90, 0x07, 0x7f],
                vec![0x90, 0x0f, 0x7f],
                vec![0x90, 0x17, 0x7f],
            ]
        );
        // Brightness changes nothing.
        deck.set_brightness(10).unwrap();
        assert_eq!(recorder.writes().len(), INIT + 9);
    }

    #[test]
    fn notes_map_to_keys() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 0x5e, 0x7f],
                &[0x90, 0x5e, 0x00],
                // Rec, Solo and Mute notes of track 1 are one key.
                &[0x90, 0x08, 0x7f],
                &[0x90, 0x10, 0x7f],
                &[0x80, 0x00, 0x40],
                &[0x90, 0x17, 0x7f],
                &[0x90, 0x36, 0x7f],
                &[0x90, 0x3a, 0x7f],
                &[0x90, 0x2e, 0x7f],
                &[0x90, 0x2f, 0x7f],
                &[0x90, 0x5b, 0x7f],
                &[0x90, 0x5f, 0x7f],
                &[0x90, 0x60, 0x7f],
                &[0x90, 0x63, 0x7f],
                &[0x80, 0x63, 0x00],
                // Unknown notes (Select, fader touch), another channel, SysEx.
                &[0x90, 0x18, 0x7f],
                &[0x90, 0x68, 0x7f],
                &[0x91, 0x5e, 0x7f],
                &[0xf0, 0x00, 0x00, 0x66, 0xf7],
                // Short and stray input.
                &[0x90],
                &[0x47, 0x00],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(18, true),
                key(18, false),
                key(0, true),
                key(0, false),
                key(7, true),
                key(8, true),
                key(12, true),
                key(13, true),
                key(14, true),
                key(15, true),
                key(19, true),
                key(20, true),
                key(23, true),
                key(23, false),
            ]
        );
    }

    #[test]
    fn each_fader_seeds_and_turns_its_own_dial() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xe0, 0x00, 0x40],
                &[0xe0, 0x00, 0x44],
                // Fader 2 seeds on its own.
                &[0xe1, 0x00, 0x10],
                &[0xe1, 0x00, 0x13],
                &[0xe1, 0x00, 0x14],
                &[0xe0, 0x7f, 0x45],
                // The master fader.
                &[0xe8, 0x00, 0x00],
                &[0xe8, 0x00, 0x7f],
                // Channels 10-16 and bad data are ignored.
                &[0xe9, 0x00, 0x00],
                &[0xe9, 0x00, 0x40],
                &[0xef, 0x00, 0x40],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(0, 2), turn(1, 1), turn(1, 1), turn(8, 63)]
        );
        // Ryu never sends pitch bend back.
        assert_eq!(recorder.writes().len(), INIT);
    }

    #[test]
    fn the_jog_dial_is_sign_and_magnitude() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 0x3c, 0x01],
                &[0xb0, 0x3c, 0x41],
                &[0xb0, 0x3c, 0x05],
                &[0xb0, 0x3c, 0x7f],
                // No movement, another CC, another channel.
                &[0xb0, 0x3c, 0x00],
                &[0xb0, 0x3c, 0x40],
                &[0xb0, 0x10, 0x01],
                &[0xb1, 0x3c, 0x01],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(9, 1), turn(9, -1), turn(9, 5), turn(9, -63)]
        );
    }

    #[test]
    fn letting_go_clears_the_lights() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes.len(), INIT);
        assert_eq!(writes[0], vec![0x90, 0x00, 0x00]);
        assert_eq!(writes[INIT - 1], vec![0x90, 0x5f, 0x00]);
        // Never started: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_the_recorders_port_matches() {
        let model = by_id(ZOOM_R16_R24_ID).unwrap();
        let cases = [
            ("ZOOM R16_R24", false, true),
            ("ZOOM R16_R24 Audio Interface", false, true),
            ("Zoom R16_R24", true, true),
            ("ZOOM R16_R24:ZOOM R16_R24 MIDI 1 24:0", false, true),
            ("MIDIIN2 (ZOOM R16_R24)", true, false),
            ("ZOOM R8", false, false),
            ("ZOOM R16", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(ZOOM_R16_R24_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_places_names_and_lights_every_key() {
        let view = ModelView::from(by_id(ZOOM_R16_R24_ID).unwrap());
        let cells = view.key_cells.unwrap();
        let expected: Vec<u16> = (0..8).chain(9..16).chain(18..27).collect();
        assert_eq!(cells, expected);
        let names = view.key_names.unwrap();
        assert_eq!(
            (names[0], names[8], names[13], names[18], names[23]),
            ("Status 1", "Auto Punch I/O", "1-8Tr", "PLAY", "Right")
        );
        assert_eq!((view.keys, view.extra_keys, view.dials), (24, 0, 10));
        assert_eq!(view.lit_keys.unwrap(), (0..8).collect::<Vec<u8>>());
        assert_eq!(by_id(ZOOM_R16_R24_ID).unwrap().dial_light_colors(), 0);
    }

    #[test]
    fn every_key_but_the_status_keys_has_one_note() {
        for (key, note) in (STATUS_KEYS..).zip(OTHER_NOTES) {
            assert_eq!(key_of_note(note), Some(key));
        }
        for note in 0..0x18 {
            assert_eq!(key_of_note(note), Some(note % 8));
        }
        assert_eq!(key_of_note(0x18), None);
        assert_eq!(key_of_note(0x68), None);
    }
}
