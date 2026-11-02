//! Akai Professional's MIDImix mixer controller, over class-compliant USB MIDI
//! ([`super::midi`]).
//!
//! The MIDImix lists one MIDI port each way, whose name starts with `MIDI
//! Mix` (the bare name on macOS and in Web MIDI, so the shared `<token> MI`
//! rule cannot take it). [`port_matches`] takes a name containing `midi mix`
//! in any case, and on Windows only the first interface.
//!
//! Everything is on MIDI channel 1, and the deck needs no start-up message.
//! In: its 19 buttons are notes. Each of the eight channel strips has a Mute
//! button (notes 1, 4, ... 22) and a Rec Arm button (notes 3, 6, ... 24);
//! while SOLO (note 27) is held, the Mute buttons send the strip's Solo note
//! instead (2, 5, ... 23). Bank Left is note 25 and Bank Right note 26. Ryu
//! lays these out on a 3 x 9 grid: row 1 Mute 1-8 then Bank Left, row 2 Solo
//! 1-8 then Bank Right, row 3 Rec Arm 1-8 then SOLO, so note `n` in 1-24 is
//! key `((n - 1) % 3) * 9 + (n - 1) / 3`. A Note On above velocity 0 presses;
//! a Note Off (or velocity 0) releases. The 24 knobs and nine faders are
//! absolute Control Change, 0-127: strip `t` counts from controller 16 + 4t
//! (strips 1-4) or 46 + 4(t - 4) (strips 5-8), then its top knob, middle
//! knob, bottom knob and fader; the master fader is `CC 62`. They send
//! nothing until moved, except that the Send All button re-sends every one,
//! so each one's first value only seeds it and later ones turn its dial a step
//! per two units moved (a re-sent unchanged value moves nothing).
//!
//! Out: every light is a Note On on channel 1, `90 <note> 7F` on and `90
//! <note> 00` off, on the Mute, Solo, Rec Arm and Bank notes (1-26); SOLO and
//! Send All have none. There is no RGB, dimming or brightness command. The
//! deck keeps the host's lights and they cannot be read back, so the first
//! flush sends every light, later flushes only the changed ones, and Ryu turns
//! them all off when it lets go. Ableton's identity request and introduction
//! are not sent.
//!
//! Facts from the akai-midimix npm package (MIT, Copyright 2020 Tom Striker)
//! for the port name, note and controller numbers and light values,
//! cross-checked against Ableton's MIDI_Mix Remote Script as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for
//! facts and the USB id only); no code was copied.

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const MIDIMIX_ID: &str = "akai-midimix";

/// Three rows of nine: eight strips and the right-hand column.
const COLS: u8 = 9;
pub const KEYS: u8 = 27;
pub const DIALS: u8 = 33;
/// The SOLO button, bottom right: no light.
const SOLO_KEY: u8 = 26;
const STRIPS: u8 = 8;
const MASTER_DIAL: u8 = 32;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const BANK_LEFT_NOTE: u8 = 25;
const LAST_NOTE: u8 = 27;
const MASTER_CC: u8 = 62;

const LED_ON: u8 = 0x7f;
const LED_OFF: u8 = 0;

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Control units (of 128) per dial step: 64 steps from end to end.
const STEP: i16 = 2;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const NAMES: [&str; KEYS as usize] = [
    "Mute 1",
    "Mute 2",
    "Mute 3",
    "Mute 4",
    "Mute 5",
    "Mute 6",
    "Mute 7",
    "Mute 8",
    "Bank Left",
    "Solo 1",
    "Solo 2",
    "Solo 3",
    "Solo 4",
    "Solo 5",
    "Solo 6",
    "Solo 7",
    "Solo 8",
    "Bank Right",
    "Rec Arm 1",
    "Rec Arm 2",
    "Rec Arm 3",
    "Rec Arm 4",
    "Rec Arm 5",
    "Rec Arm 6",
    "Rec Arm 7",
    "Rec Arm 8",
    "SOLO",
];

/// Whether a normalised port name is a MIDImix port: `MIDI Mix` in any case,
/// and on Windows the first (only) interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.protocol == Protocol::AkaiMidimixMidi
        && name.interface.unwrap_or(1) == 1
        && name.base.to_ascii_lowercase().contains("midi mix")
}

/// The legend on each key: Mute, Solo and Rec Arm by strip, the bank buttons
/// and SOLO.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::AkaiMidimixMidi).then(|| NAMES.to_vec())
}

/// Every key but SOLO has a light.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    (model.protocol == Protocol::AkaiMidimixMidi).then(|| (0..SOLO_KEY).collect())
}

/// The key a note presses.
const fn key_of_note(note: u8) -> Option<u8> {
    match note {
        1..=24 => Some(((note - 1) % 3) * COLS + (note - 1) / 3),
        BANK_LEFT_NOTE..=LAST_NOTE => Some((note - BANK_LEFT_NOTE) * COLS + STRIPS),
        _ => None,
    }
}

/// The note of a key.
const fn note_of_key(key: u8) -> Option<u8> {
    if key >= KEYS {
        return None;
    }
    let (row, col) = (key / COLS, key % COLS);
    if col < STRIPS {
        Some(1 + col * 3 + row)
    } else {
        Some(BANK_LEFT_NOTE + row)
    }
}

/// The dial a controller turns: dials 1-8 the top knobs, 9-16 the middle
/// knobs, 17-24 the bottom knobs, 25-32 the channel faders, 33 the master.
const fn dial_of_cc(cc: u8) -> Option<u8> {
    let (strip, part) = match cc {
        16..=31 => ((cc - 16) / 4, (cc - 16) % 4),
        46..=61 => (4 + (cc - 46) / 4, (cc - 46) % 4),
        MASTER_CC => return Some(MASTER_DIAL),
        _ => return None,
    };
    Some(part * STRIPS + strip)
}

/// A light's state: on for any colour that is not dark.
fn led_state(color: [u8; 3]) -> u8 {
    if color.iter().any(|channel| *channel >= DARK) {
        LED_ON
    } else {
        LED_OFF
    }
}

pub struct Midimix {
    transport: Box<dyn Transport>,
    /// Every light sent once since open.
    started: bool,
    colors: Vec<[u8; 3]>,
    /// The state each lit key was last sent; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    /// Each control's last value; `None` until it first arrives.
    values: Vec<Option<u8>>,
    /// Movement not yet turned into a whole step.
    rest: Vec<i16>,
    parser: Parser,
}

impl Midimix {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            colors: vec![BLACK; usize::from(SOLO_KEY)],
            sent: vec![None; usize::from(SOLO_KEY)],
            held: vec![false; usize::from(KEYS)],
            values: vec![None; usize::from(DIALS)],
            rest: vec![0; usize::from(DIALS)],
            parser: Parser::default(),
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

    /// A knob's or fader's absolute value: the first seeds it, each later
    /// one turns its dial a step per two units moved.
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

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let Message::Channel([status, number, value]) = *message else {
            return;
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
                if let Some(dial) = dial_of_cc(number) {
                    self.control(dial, value, events);
                }
            }
            _ => {}
        }
    }
}

impl Driver for Midimix {
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

    /// One Note On for each light whose state changed (every light the
    /// first time).
    fn flush(&mut self) -> DriverResult<()> {
        self.started = true;
        for key in 0..SOLO_KEY {
            let (Some(note), Some(color)) = (note_of_key(key), self.colors.get(usize::from(key)))
            else {
                continue;
            };
            let state = led_state(*color);
            let Some(sent) = self.sent.get_mut(usize::from(key)) else {
                continue;
            };
            if *sent != Some(state) {
                self.transport.write(&[NOTE_ON, note, state])?;
                *sent = Some(state);
            }
        }
        Ok(())
    }

    /// The MIDImix has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for Midimix {
    /// Turns every light off: the deck would otherwise keep them.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for note in 1..LAST_NOTE {
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

    fn open() -> (Midimix, Recorder) {
        let recorder = Recorder::default();
        let deck = Midimix::new(by_id(MIDIMIX_ID).unwrap(), Box::new(recorder.clone()));
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
    fn notes_map_to_the_three_rows() {
        // Mute, Solo and Rec Arm of strip 1, then of strip 8.
        assert_eq!(key_of_note(1), Some(0));
        assert_eq!(key_of_note(2), Some(9));
        assert_eq!(key_of_note(3), Some(18));
        assert_eq!(key_of_note(22), Some(7));
        assert_eq!(key_of_note(23), Some(16));
        assert_eq!(key_of_note(24), Some(25));
        // Bank Left, Bank Right and SOLO down the right.
        assert_eq!(key_of_note(25), Some(8));
        assert_eq!(key_of_note(26), Some(17));
        assert_eq!(key_of_note(27), Some(26));
        for note in [0, 28, 62, 127] {
            assert_eq!(key_of_note(note), None, "{note}");
        }
        for key in 0..KEYS {
            assert_eq!(note_of_key(key).and_then(key_of_note), Some(key));
        }
        assert_eq!(note_of_key(KEYS), None);
    }

    #[test]
    fn controllers_map_to_dials_by_row() {
        let send_a = [16, 20, 24, 28, 46, 50, 54, 58];
        let send_b = [17, 21, 25, 29, 47, 51, 55, 59];
        let pan = [18, 22, 26, 30, 48, 52, 56, 60];
        let faders = [19, 23, 27, 31, 49, 53, 57, 61];
        for (row, ccs) in [send_a, send_b, pan, faders].iter().enumerate() {
            for (strip, cc) in ccs.iter().enumerate() {
                let dial = u8::try_from(row * 8 + strip).unwrap();
                assert_eq!(dial_of_cc(*cc), Some(dial), "CC {cc}");
            }
        }
        assert_eq!(dial_of_cc(62), Some(32));
        for cc in [0, 15, 32, 45, 63, 127] {
            assert_eq!(dial_of_cc(cc), None, "{cc}");
        }
    }

    #[test]
    fn the_first_flush_sends_every_light_and_later_ones_only_changes() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(9, Some(&lit([0, 0, 40]))).unwrap();
        deck.set_face(17, Some(&lit([20, 20, 20]))).unwrap();
        deck.set_face(25, Some(&lit([0, 200, 0]))).unwrap();
        // SOLO has no light.
        deck.set_face(26, Some(&lit([255, 255, 255]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 26);
        assert_eq!(writes[0], vec![0x90, 1, 0x7f]);
        assert_eq!(writes[1], vec![0x90, 4, 0]);
        assert_eq!(writes[8], vec![0x90, 25, 0]);
        assert_eq!(writes[9], vec![0x90, 2, 0x7f]);
        assert_eq!(writes[17], vec![0x90, 26, 0]);
        assert_eq!(writes[25], vec![0x90, 24, 0x7f]);
        assert!(writes
            .iter()
            .all(|write| write.len() == 3 && write[1] != 27));
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 26);
        deck.set_face(8, Some(&lit([255, 255, 0]))).unwrap();
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[26..],
            [vec![0x90, 1, 0], vec![0x90, 25, 0x7f]]
        );
        // Brightness changes nothing.
        deck.set_brightness(10).unwrap();
        assert_eq!(recorder.writes().len(), 28);
    }

    #[test]
    fn presses_map_and_other_channels_are_dropped() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x90, 1, 0x7f],
                &[0x80, 1, 0x7f],
                // SOLO held, then Mute 3 sends Solo 3.
                &[0x90, 27, 0x7f],
                &[0x90, 8, 0x7f],
                &[0x80, 8, 0x00],
                &[0x80, 27, 0x00],
                &[0x90, 24, 0x7f],
                &[0x90, 24, 0x00],
                &[0x90, 25, 0x7f],
                &[0x90, 26, 0x7f],
                // Unknown notes, another channel, and a SysEx reply.
                &[0x90, 0, 0x7f],
                &[0x90, 28, 0x7f],
                &[0x91, 1, 0x7f],
                &[0xf0, 0x7e, 0x00, 0x06, 0x02, 0x47, 0x31, 0xf7],
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
                key(26, true),
                key(11, true),
                key(11, false),
                key(26, false),
                key(25, true),
                key(25, false),
                key(8, true),
                key(17, true),
            ]
        );
    }

    #[test]
    fn knobs_and_faders_turn_dials_after_their_first_value() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0xb0, 16, 0],
                &[0xb0, 16, 4],
                &[0xb0, 16, 5],
                &[0xb0, 16, 6],
                &[0xb0, 62, 127],
                &[0xb0, 62, 100],
                &[0xb0, 61, 64],
                // Send All re-sends the same values: nothing moves.
                &[0xb0, 16, 6],
                &[0xb0, 62, 100],
                &[0xb0, 61, 64],
                &[0xb0, 61, 70],
                // Not a control; another channel.
                &[0xb0, 63, 3],
                &[0xb1, 16, 100],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(0, 2), turn(0, 1), turn(32, -13), turn(31, 3)]
        );
    }

    #[test]
    fn letting_go_turns_every_light_off() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        drop(deck);
        let writes = &recorder.writes()[26..];
        assert_eq!(writes.len(), 26);
        assert_eq!(writes[0], vec![0x90, 1, 0]);
        assert_eq!(writes[25], vec![0x90, 26, 0]);
        // Never opened: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_midimix_ports_match() {
        let model = by_id(MIDIMIX_ID).unwrap();
        let cases = [
            ("MIDI Mix", false, true),
            ("MIDI Mix:MIDI Mix MIDI 1 24:0", false, true),
            ("MIDI Mix", true, true),
            ("2- MIDI Mix", true, true),
            ("MIDIIN2 (MIDI Mix)", true, false),
            // Other Akai decks and an unrelated port.
            ("APC MINI", false, false),
            ("APC mini mk2 Control", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(MIDIMIX_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_names_every_key() {
        let view = ModelView::from(by_id(MIDIMIX_ID).unwrap());
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), 27);
        assert_eq!(
            (names[0], names[8], names[9], names[17], names[18], names[26]),
            (
                "Mute 1",
                "Bank Left",
                "Solo 1",
                "Bank Right",
                "Rec Arm 1",
                "SOLO"
            )
        );
        assert_eq!(view.lit_keys.unwrap(), (0..26).collect::<Vec<u8>>());
        assert_eq!(view.key_light_colors, "on_off");
        assert_eq!(view.dial_lights, DialLights::None);
        assert!(view.key_lights && !view.brightness);
    }
}
