//! Akai Professional's LPD8 (the original, MK1) pad controller, over
//! class-compliant USB MIDI ([`super::midi`]).
//!
//! The LPD8 lists one MIDI port each way named `LPD8` (the bare name on macOS,
//! Windows and in Web MIDI, so the shared `<token> MI` rule cannot take it).
//! [`port_matches`] takes a name containing `lpd8` in any case, but never one
//! naming an `mk2` (a different deck, not covered here), and on Windows only
//! the first interface.
//!
//! Ryu expects the factory settings of program 1 (PROG 1) with the pads in
//! PAD mode, all on MIDI channel 1. The PROG, PAD, CC, PROG CHNG, NOTE REPEAT
//! and FULL LEVEL buttons send nothing and are not keys.
//!
//! In: the eight velocity-sensitive pads are notes 36-43 (PAD1-PAD8). PAD5-8
//! sit above PAD1-4, so in reading order key `k` is note `40 + k` for the top
//! row (keys 0-3) and `32 + k` for the bottom row (keys 4-7). A Note On above
//! velocity 0 presses; a Note Off, or velocity 0, releases. The knobs K1-K8
//! are absolute Control Change 1-8 (0-127), dials 1-8; they send nothing until
//! moved, so each one's first value only seeds it and later ones turn its dial
//! a step per two units. Other channels, notes and controllers, Program
//! Change (the pads in PROG CHNG mode) and SysEx are dropped.
//!
//! Out: each pad's single-colour light is `90 <note> 01` on or `90 <note> 00`
//! off; nothing else is ever sent, since unknown messages are reported to make
//! the deck stop responding. There is no colour, dimming or brightness
//! command: a colour that is not dark turns a light on, and brightness 0 turns
//! them all off. The deck lights a pad itself while it is struck, and it shows
//! the host's lights only for the program and mode it is on, losing them when
//! the user switches; it does not report the switch. So every light is sent
//! again about every 300 ms while the deck is open (from [`Driver::poll`],
//! which the manager calls every few milliseconds), and a pad's own light
//! again when it is released. Ryu turns every light off when it lets go.
//!
//! Facts from the akai-lpd8 npm package (ISC) for the layout, port name and
//! note and controller numbers, cross-checked against Ableton's LPD8 Remote
//! Script as published in gluon/AbletonLive12_MIDIRemoteScripts (decompiled,
//! no licence; read for facts and the USB id only) and Mixxx's
//! Akai-LPD8-RK mapping (GPL-2.0-or-later; read for the light values and the
//! refresh only); no code was copied.

use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const LPD8_ID: &str = "akai-lpd8";

pub const KEYS: u8 = 8;
pub const DIALS: u8 = 8;
/// Keys per row: PAD5-8 on top, PAD1-4 below.
const COLS: u8 = 4;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// PAD1, bottom left.
const FIRST_NOTE: u8 = 36;
/// PAD5, top left.
const TOP_NOTE: u8 = 40;
const LAST_NOTE: u8 = 43;
/// K1's controller.
const FIRST_CC: u8 = 1;

const LED_ON: u8 = 1;
const LED_OFF: u8 = 0;

/// How often every light is sent again, since the deck drops them when its
/// program or mode is switched.
const REFRESH: Duration = Duration::from_millis(300);
/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Control units (of 128) per dial step: 64 steps from end to end.
const STEP: i16 = 2;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const NAMES: [&str; KEYS as usize] = [
    "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 1", "Pad 2", "Pad 3", "Pad 4",
];

/// Whether a normalised port name is an LPD8 port: `LPD8` in any case, not an
/// `mk2`, and on Windows the first (only) interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.protocol != Protocol::AkaiLpd8Midi || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.to_ascii_lowercase();
    base.contains("lpd8") && !base.contains("mk2")
}

/// The legend on each key: the pad number printed on the deck.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::AkaiLpd8Midi).then(|| NAMES.to_vec())
}

/// The key a note presses.
const fn key_of_note(note: u8) -> Option<u8> {
    match note {
        TOP_NOTE..=LAST_NOTE => Some(note - TOP_NOTE),
        FIRST_NOTE..TOP_NOTE => Some(note - FIRST_NOTE + COLS),
        _ => None,
    }
}

/// The note of a key.
const fn note_of_key(key: u8) -> Option<u8> {
    match key {
        0..COLS => Some(TOP_NOTE + key),
        COLS..KEYS => Some(FIRST_NOTE + key - COLS),
        _ => None,
    }
}

/// The dial a controller turns: K1-K8 are dials 1-8.
const fn dial_of_cc(cc: u8) -> Option<u8> {
    if cc >= FIRST_CC && cc < FIRST_CC + DIALS {
        Some(cc - FIRST_CC)
    } else {
        None
    }
}

/// A light's state: on for any colour that is not dark.
fn led_state(color: [u8; 3]) -> u8 {
    if color.iter().any(|channel| *channel >= DARK) {
        LED_ON
    } else {
        LED_OFF
    }
}

pub struct Lpd8 {
    transport: Box<dyn Transport>,
    /// Every light sent once since open.
    started: bool,
    colors: [[u8; 3]; KEYS as usize],
    /// Brightness 0 turns every light off.
    dark: bool,
    /// The state each key was last sent; `None` until it is.
    sent: [Option<u8>; KEYS as usize],
    /// When every light is next sent again.
    refresh_at: Instant,
    held: [bool; KEYS as usize],
    /// Each knob's last value; `None` until it first arrives.
    values: [Option<u8>; DIALS as usize],
    /// Movement not yet turned into a whole step.
    rest: [i16; DIALS as usize],
    parser: Parser,
}

impl Lpd8 {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            colors: [BLACK; KEYS as usize],
            dark: false,
            sent: [None; KEYS as usize],
            refresh_at: Instant::now() + REFRESH,
            held: [false; KEYS as usize],
            values: [None; DIALS as usize],
            rest: [0; DIALS as usize],
            parser: Parser::default(),
        }
    }

    /// The state a key's light should show.
    fn state(&self, key: u8) -> u8 {
        if self.dark {
            return LED_OFF;
        }
        self.colors
            .get(usize::from(key))
            .map_or(LED_OFF, |color| led_state(*color))
    }

    /// Sends one key's light, whether or not it changed.
    fn send(&mut self, key: u8) -> DriverResult<()> {
        let Some(note) = note_of_key(key) else {
            return Ok(());
        };
        let state = self.state(key);
        self.transport.write(&[NOTE_ON, note, state])?;
        if let Some(sent) = self.sent.get_mut(usize::from(key)) {
            *sent = Some(state);
        }
        Ok(())
    }

    /// Sends every light and schedules the next refresh.
    fn send_all(&mut self) -> DriverResult<()> {
        self.started = true;
        self.refresh_at = Instant::now() + REFRESH;
        for key in 0..KEYS {
            self.send(key)?;
        }
        Ok(())
    }

    fn press(&mut self, key: u8, pressed: bool, events: &mut Vec<Input>) -> DriverResult<()> {
        let Some(held) = self.held.get_mut(usize::from(key)) else {
            return Ok(());
        };
        if *held == pressed {
            return Ok(());
        }
        *held = pressed;
        events.push(Input::Key {
            index: key,
            pressed,
        });
        // The deck lit the pad itself while it was struck: put Ryu's back.
        if pressed {
            Ok(())
        } else {
            self.send(key)
        }
    }

    /// A knob's absolute value: the first seeds it, each later one turns its
    /// dial a step per two units moved.
    fn control(&mut self, dial: u8, value: u8, events: &mut Vec<Input>) {
        let at = usize::from(dial);
        let (Some(last), Some(rest)) = (self.values.get_mut(at), self.rest.get_mut(at)) else {
            return;
        };
        let Some(old) = last.replace(value) else {
            return;
        };
        let moved = *rest + i16::from(value) - i16::from(old);
        let steps = moved / STEP;
        *rest = moved - steps * STEP;
        if steps != 0 {
            events.push(Input::DialTurn {
                index: dial,
                delta: i8::try_from(steps.clamp(-127, 127)).unwrap_or(0),
            });
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> DriverResult<()> {
        let Message::Channel([status, number, value]) = *message else {
            return Ok(());
        };
        if status & 0x0f != 0 {
            return Ok(());
        }
        match status & 0xf0 {
            NOTE_ON | NOTE_OFF => {
                if let Some(key) = key_of_note(number) {
                    self.press(key, status & 0xf0 == NOTE_ON && value > 0, events)?;
                }
            }
            CONTROL_CHANGE => {
                if let Some(dial) = dial_of_cc(number) {
                    self.control(dial, value, events);
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl Driver for Lpd8 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started || Instant::now() >= self.refresh_at {
            self.send_all()?;
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
            self.receive(message, &mut events)?;
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// Every light the first time, then one Note On for each light whose
    /// state changed; the periodic refresh re-sends the rest.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.started {
            return self.send_all();
        }
        for key in 0..KEYS {
            let state = self.state(key);
            if self.sent.get(usize::from(key)).copied().flatten() != Some(state) {
                self.send(key)?;
            }
        }
        Ok(())
    }

    /// The LPD8 has no brightness command: 0% turns every light off, any
    /// other level shows them as their colours say.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.dark = percent == 0;
        if self.started {
            self.flush()?;
        }
        Ok(())
    }
}

impl Drop for Lpd8 {
    /// Turns every light off: the deck would otherwise keep them.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for note in FIRST_NOTE..=LAST_NOTE {
            let _ = self.transport.write(&[NOTE_ON, note, LED_OFF]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    fn open() -> (Lpd8, Recorder) {
        let recorder = Recorder::default();
        let deck = Lpd8::new(by_id(LPD8_ID).unwrap(), Box::new(recorder.clone()));
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

    /// Every write is a light: `90 <36-43> 00|01`.
    fn only_pad_lights(recorder: &Recorder) {
        for write in recorder.writes() {
            assert!(
                write.len() == 3
                    && write[0] == 0x90
                    && (36..=43).contains(&write[1])
                    && write[2] <= 1,
                "{write:02x?}"
            );
        }
    }

    #[test]
    fn notes_map_to_pads_in_reading_order() {
        // PAD5-8 on top, PAD1-4 below.
        assert_eq!(key_of_note(40), Some(0));
        assert_eq!(key_of_note(43), Some(3));
        assert_eq!(key_of_note(36), Some(4));
        assert_eq!(key_of_note(39), Some(7));
        for note in [0, 35, 44, 60, 127] {
            assert_eq!(key_of_note(note), None, "{note}");
        }
        let notes: Vec<u8> = (0..KEYS).filter_map(note_of_key).collect();
        assert_eq!(notes, vec![40, 41, 42, 43, 36, 37, 38, 39]);
        for key in 0..KEYS {
            assert_eq!(note_of_key(key).and_then(key_of_note), Some(key));
        }
        assert_eq!(note_of_key(KEYS), None);
        assert_eq!(dial_of_cc(1), Some(0));
        assert_eq!(dial_of_cc(8), Some(7));
        for cc in [0, 9, 64, 127] {
            assert_eq!(dial_of_cc(cc), None, "{cc}");
        }
    }

    #[test]
    fn the_first_poll_sends_every_light_and_flushes_only_changes() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(5, Some(&lit([0, 0, 40]))).unwrap();
        deck.set_face(7, Some(&lit([20, 20, 20]))).unwrap();
        deck.poll(0).unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x90, 40, 1],
                vec![0x90, 41, 0],
                vec![0x90, 42, 0],
                vec![0x90, 43, 0],
                vec![0x90, 36, 0],
                vec![0x90, 37, 1],
                vec![0x90, 38, 0],
                vec![0x90, 39, 0],
            ]
        );
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 8);
        deck.set_face(4, Some(&lit([255, 255, 0]))).unwrap();
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[8..],
            [vec![0x90, 40, 0], vec![0x90, 36, 1]]
        );
        // Brightness 0 turns the lit ones off, and back on after.
        deck.set_brightness(0).unwrap();
        assert_eq!(
            recorder.writes()[10..],
            [vec![0x90, 36, 0], vec![0x90, 37, 0]]
        );
        deck.set_brightness(40).unwrap();
        assert_eq!(
            recorder.writes()[12..],
            [vec![0x90, 36, 1], vec![0x90, 37, 1]]
        );
        only_pad_lights(&recorder);
    }

    #[test]
    fn every_light_is_sent_again_after_the_refresh_interval() {
        let (mut deck, recorder) = open();
        deck.set_face(2, Some(&lit([0, 255, 0]))).unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 8);
        // Not due yet: nothing more.
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 8);
        deck.refresh_at = Instant::now() - Duration::from_millis(1);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 16);
        assert_eq!(writes[8..], writes[..8]);
        assert_eq!(writes[10], vec![0x90, 42, 1]);
        assert!(deck.refresh_at > Instant::now());
        only_pad_lights(&recorder);
    }

    #[test]
    fn pads_press_and_release_and_other_messages_are_dropped() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                // PAD5 struck softly, released by Note Off.
                &[0x90, 40, 0x05],
                &[0x80, 40, 0x00],
                // PAD1 released by velocity 0.
                &[0x90, 36, 0x7f],
                &[0x90, 36, 0x00],
                &[0x90, 39, 0x40],
                // Other notes, another channel, Program Change, SysEx.
                &[0x90, 35, 0x7f],
                &[0x90, 44, 0x7f],
                &[0x91, 37, 0x7f],
                &[0xc0, 0x02],
                &[0xf0, 0x7e, 0x00, 0x06, 0x02, 0x47, 0x75, 0xf7],
                // Short and stray input.
                &[0x90],
                &[0x47, 0x00],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(4, true),
                key(4, false),
                key(7, true),
            ]
        );
        // Each release puts the host's light back.
        assert_eq!(
            recorder.writes()[8..],
            [vec![0x90, 40, 1], vec![0x90, 36, 0]]
        );
        only_pad_lights(&recorder);
    }

    #[test]
    fn knobs_turn_dials_after_their_first_value() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 1, 0],
                &[0xb0, 1, 4],
                &[0xb0, 1, 5],
                &[0xb0, 1, 6],
                &[0xb0, 8, 127],
                &[0xb0, 8, 100],
                &[0xb0, 8, 100],
                // Not a knob; another channel.
                &[0xb0, 9, 3],
                &[0xb1, 1, 100],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(0, 2), turn(0, 1), turn(7, -13)]
        );
    }

    #[test]
    fn letting_go_turns_every_light_off() {
        let (mut deck, recorder) = open();
        deck.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        drop(deck);
        let writes = &recorder.writes()[8..];
        let expected: Vec<Vec<u8>> = (36..=43).map(|note| vec![0x90, note, 0]).collect();
        assert_eq!(writes, expected.as_slice());
        only_pad_lights(&recorder);
        // Never opened: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_lpd8_ports_match() {
        let model = by_id(LPD8_ID).unwrap();
        let cases = [
            ("LPD8", false, true),
            ("LPD8", true, true),
            ("LPD8:LPD8 MIDI 1 24:0", false, true),
            ("MIDIIN2 (LPD8)", true, false),
            ("LPD8 mk2", false, false),
            ("LPD8 MK2:LPD8 MK2 MIDI 1 24:0", false, false),
            // Other Akai decks and an unrelated port.
            ("APC MINI", false, false),
            ("MIDI Mix", false, false),
            ("MPK mini 3", false, false),
            ("MPD218 Port A", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(LPD8_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_names_every_pad() {
        let view = ModelView::from(by_id(LPD8_ID).unwrap());
        assert_eq!(
            view.key_names.unwrap(),
            vec!["Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 1", "Pad 2", "Pad 3", "Pad 4"]
        );
        assert_eq!((view.rows, view.cols, view.keys, view.dials), (2, 4, 8, 8));
        assert_eq!(view.key_cells, None);
        assert_eq!(view.lit_keys, None);
        assert_eq!(view.key_light_colors, "on_off");
        assert_eq!(view.dial_lights, DialLights::None);
        assert!(view.key_lights && !view.brightness);
    }
}
