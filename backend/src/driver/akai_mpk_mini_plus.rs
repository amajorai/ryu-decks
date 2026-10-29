//! Akai Professional's MPK mini Plus (the first generation), over
//! class-compliant USB MIDI ([`super::midi`]): only its five transport
//! buttons, each a key with a single-colour on/off light.
//!
//! The deck lists one MIDI port each way: `MPK mini Plus` on Windows,
//! `MPK mini Plus Port 1` on macOS (or a localised `Anschluss 1`, `Puerto 1`,
//! `Porto 1`) and `MPK mini Plus MIDI 1` under ALSA. [`port_matches`] takes a
//! name containing `mpk mini plus` in any case, but never one naming
//! `plus ii` (the second-generation MPK mini Plus II, a different deck with a
//! DAW port, driven by [`super::akai_mpk_mk4`]) or `mk` (the MPK mini mkII,
//! mkIII and the like), and on Windows only the first interface.
//!
//! In: Rewind, Fast Forward, Stop, Play and Record are Control Change
//! `0x73`-`0x77` on MIDI channel 1, value 127 pressed and 0 released (any
//! value above 0 presses here). They are keys 0-4 in that order; that this is
//! their order left to right on the deck is assumed from the numbering. Those
//! numbers are fixed in Bitwig's extension, which relies on them whether or
//! not it writes its own program to the deck; that no program changes them
//! is not shown. Everything else is dropped: the keybed's notes, pitch bend,
//! aftertouch and controllers on channel 1, the pads on channel 10, the
//! knobs, Program Change and SysEx. The pads and knobs send whatever the
//! active program says, their layout is not documented and no light output
//! for them is known, so they are not keys or dials.
//!
//! Out: each button's light is `90 <its controller> 7F` on or `90 <its
//! controller> 00` off. There is no colour, dimming or brightness command: a
//! colour that is not dark turns a light on, and brightness 0 turns them all
//! off. Ryu sends no start-up message and never Bitwig's program SysEx (its
//! fields are not documented and it rewrites a stored program). In case the
//! deck's own button handling overrides the host's lights (an assumption: no
//! reference says it does), every light is sent again about every 300 ms
//! while the deck is open (from [`Driver::poll`], which the manager calls
//! every few milliseconds; at once under test), and a button's own light
//! again when it is released. Ryu turns every light off when it lets go.
//!
//! Facts from Bitwig's MPK mini Plus extension (bitwig/bitwig-extensions,
//! MIT; port names, controller numbers and light messages), cross-checked
//! against Ableton's MPK_mini_Plus Remote Script as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for the
//! USB id and identity bytes only); no code was copied.

use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const MPK_MINI_PLUS_ID: &str = "akai-mpk-mini-plus";

pub const KEYS: u8 = 5;
pub const DIALS: u8 = 0;

const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// Rewind's controller; Fast Forward, Stop, Play and Record follow.
const FIRST_CC: u8 = 0x73;

const LED_ON: u8 = 0x7f;
const LED_OFF: u8 = 0x00;

/// How often every light is sent again, in case the deck drops them.
#[cfg(not(test))]
const REFRESH: Duration = Duration::from_millis(300);
#[cfg(test)]
const REFRESH: Duration = Duration::ZERO;
/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const NAMES: [&str; KEYS as usize] = ["Rewind", "Fast Forward", "Stop", "Play", "Record"];

/// Whether a normalised port name is a first MPK mini Plus port: `MPK mini
/// Plus` in any case, never a `Plus II` or an `mk` model, and on Windows the
/// first (only) interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.protocol != Protocol::AkaiMpkMiniPlusMidi || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.to_ascii_lowercase();
    base.contains("mpk mini plus") && !base.contains("plus ii") && !base.contains("mk")
}

/// The legend on each key: the transport button's function.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::AkaiMpkMiniPlusMidi).then(|| NAMES.to_vec())
}

/// The key a controller presses.
const fn key_of_cc(cc: u8) -> Option<u8> {
    if cc >= FIRST_CC && cc < FIRST_CC + KEYS {
        Some(cc - FIRST_CC)
    } else {
        None
    }
}

/// The note that sets a key's light: its controller number.
const fn note_of_key(key: u8) -> Option<u8> {
    if key < KEYS {
        Some(FIRST_CC + key)
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

pub struct MpkMiniPlus {
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
    parser: Parser,
}

impl MpkMiniPlus {
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
        // In case the deck lit the button itself while it was held.
        if pressed {
            Ok(())
        } else {
            self.send(key)
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> DriverResult<()> {
        let Message::Channel([status, number, value]) = *message else {
            return Ok(());
        };
        if status != CONTROL_CHANGE {
            return Ok(());
        }
        match key_of_cc(number) {
            Some(key) => self.press(key, value > 0, events),
            None => Ok(()),
        }
    }
}

impl Driver for MpkMiniPlus {
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

    /// The deck has no brightness command: 0% turns every light off, any
    /// other level shows them as their colours say.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.dark = percent == 0;
        if self.started {
            self.flush()?;
        }
        Ok(())
    }
}

impl Drop for MpkMiniPlus {
    /// Turns every light off: the deck would otherwise keep them.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for key in 0..KEYS {
            if let Some(note) = note_of_key(key) {
                let _ = self.transport.write(&[NOTE_ON, note, LED_OFF]);
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

    fn open() -> (MpkMiniPlus, Recorder) {
        let recorder = Recorder::default();
        let deck = MpkMiniPlus::new(by_id(MPK_MINI_PLUS_ID).unwrap(), Box::new(recorder.clone()));
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

    /// Every write is a light: `90 <73-77> 00|7F`.
    fn only_transport_lights(recorder: &Recorder) {
        for write in recorder.writes() {
            assert!(
                write.len() == 3
                    && write[0] == 0x90
                    && (0x73..=0x77).contains(&write[1])
                    && (write[2] == 0 || write[2] == 0x7f),
                "{write:02x?}"
            );
        }
    }

    #[test]
    fn controllers_map_to_the_transport_keys() {
        assert_eq!(key_of_cc(0x73), Some(0));
        assert_eq!(key_of_cc(0x77), Some(4));
        for cc in [0, 1, 64, 0x46, 0x4d, 0x72, 0x78, 127] {
            assert_eq!(key_of_cc(cc), None, "{cc}");
        }
        let notes: Vec<u8> = (0..KEYS).filter_map(note_of_key).collect();
        assert_eq!(notes, vec![0x73, 0x74, 0x75, 0x76, 0x77]);
        assert_eq!(note_of_key(KEYS), None);
    }

    #[test]
    fn the_first_flush_sends_every_light_then_only_changes() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(3, Some(&lit([0, 0, 40]))).unwrap();
        deck.set_face(4, Some(&lit([20, 20, 20]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x90, 0x73, 0x7f],
                vec![0x90, 0x74, 0x00],
                vec![0x90, 0x75, 0x00],
                vec![0x90, 0x76, 0x7f],
                vec![0x90, 0x77, 0x00],
            ]
        );
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 5);
        deck.set_face(4, Some(&lit([255, 255, 0]))).unwrap();
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[5..],
            [vec![0x90, 0x73, 0x00], vec![0x90, 0x77, 0x7f]]
        );
        // Brightness 0 turns the lit ones off, and back on after.
        deck.set_brightness(0).unwrap();
        assert_eq!(
            recorder.writes()[7..],
            [vec![0x90, 0x76, 0x00], vec![0x90, 0x77, 0x00]]
        );
        deck.set_brightness(40).unwrap();
        assert_eq!(
            recorder.writes()[9..],
            [vec![0x90, 0x76, 0x7f], vec![0x90, 0x77, 0x7f]]
        );
        only_transport_lights(&recorder);
    }

    #[test]
    fn every_poll_sends_the_lights_again_under_test() {
        // The 300 ms refresh is zero under test: each poll re-sends them.
        let (mut deck, recorder) = open();
        deck.set_face(2, Some(&lit([0, 255, 0]))).unwrap();
        deck.poll(0).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 10);
        assert_eq!(writes[5..], writes[..5]);
        assert_eq!(writes[2], vec![0x90, 0x75, 0x7f]);
        only_transport_lights(&recorder);
    }

    #[test]
    fn transport_buttons_press_and_release_and_other_messages_are_dropped() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                // Rewind pressed and released.
                &[0xb0, 0x73, 0x7f],
                &[0xb0, 0x73, 0x00],
                // Record, with an odd value still pressing.
                &[0xb0, 0x77, 0x40],
                // Play pressed, Record released.
                &[0xb0, 0x76, 0x7f],
                &[0xb0, 0x77, 0x00],
                // The same controllers on channel 2 and 10.
                &[0xb1, 0x74, 0x7f],
                &[0xb9, 0x75, 0x7f],
                // Keybed: notes, mod wheel, sustain, pitch bend, aftertouch.
                &[0x90, 0x3c, 0x64],
                &[0x80, 0x3c, 0x00],
                &[0x90, 0x73, 0x7f],
                &[0xb0, 0x01, 0x40],
                &[0xb0, 0x40, 0x7f],
                &[0xe0, 0x00, 0x50],
                &[0xd0, 0x30],
                // Pads on channel 10, knobs, Program Change.
                &[0x99, 0x24, 0x7f],
                &[0x89, 0x24, 0x00],
                &[0xa9, 0x24, 0x20],
                &[0xd9, 0x20],
                &[0xb0, 0x46, 0x01],
                &[0xb0, 0x4d, 0x7f],
                &[0xc0, 0x02],
                // The identity reply and other SysEx.
                &[
                    0xf0, 0x7e, 0x7f, 0x06, 0x02, 0x47, 0x54, 0x00, 0x19, 0x00, 0xf7,
                ],
                // A short message, completed by running status as a
                // controller that is not a key.
                &[0xb0],
                &[0x10, 0x7f],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(4, true),
                key(3, true),
                key(4, false),
            ]
        );
        // Each release puts the host's light back (after poll's refresh).
        assert_eq!(
            recorder.writes()[10..],
            [vec![0x90, 0x73, 0x7f], vec![0x90, 0x77, 0x00]]
        );
        only_transport_lights(&recorder);
    }

    #[test]
    fn letting_go_turns_every_light_off() {
        let (mut deck, recorder) = open();
        deck.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        drop(deck);
        let writes = &recorder.writes()[5..];
        let expected: Vec<Vec<u8>> = (0x73..=0x77).map(|note| vec![0x90, note, 0]).collect();
        assert_eq!(writes, expected.as_slice());
        only_transport_lights(&recorder);
        // Never opened: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_first_generation_ports_match() {
        let model = by_id(MPK_MINI_PLUS_ID).unwrap();
        let cases = [
            ("MPK mini Plus MIDI 1", false, true),
            ("MPK mini Plus:MPK mini Plus MIDI 1 20:0", false, true),
            ("MPK mini Plus", true, true),
            ("2- MPK mini Plus", true, true),
            ("MPK mini Plus Port 1", false, true),
            ("MPK mini Plus Anschluss 1", false, true),
            ("MPK mini Plus Puerto 1", false, true),
            ("MPK MINI PLUS", false, true),
            ("MIDIIN2 (MPK mini Plus)", true, false),
            // The second generation, every way it is named.
            ("MPK mini Plus II", true, false),
            ("MIDIIN2 (MPK mini Plus II)", true, false),
            ("MIDIOUT2 (MPK mini Plus II)", true, false),
            ("MPK mini Plus II DAW Port", false, false),
            ("MPK mini Plus II MIDI Port", false, false),
            // Other MPK minis and an unrelated port.
            ("MPK mini 3", false, false),
            ("MPK Mini Mk II", false, false),
            ("MPK mini Plus mk2", false, false),
            ("MPK mini IV DAW Port", false, false),
            ("LPD8", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(MPK_MINI_PLUS_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_names_every_transport_key() {
        let model = by_id(MPK_MINI_PLUS_ID).unwrap();
        let view = ModelView::from(model);
        assert_eq!(
            view.key_names.unwrap(),
            vec!["Rewind", "Fast Forward", "Stop", "Play", "Record"]
        );
        assert_eq!((view.rows, view.cols, view.keys, view.dials), (1, 5, 5, 0));
        assert_eq!(view.key_cells, None);
        assert_eq!(view.lit_keys, None);
        assert_eq!(view.key_light_colors, "on_off");
        assert_eq!(view.dial_lights, DialLights::None);
        assert!(view.key_lights && !view.brightness);
        assert_eq!(model.dial_light_colors(), 0);
    }
}
