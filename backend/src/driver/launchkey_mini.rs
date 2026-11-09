//! Novation's first Launchkey Mini (USB id `1235:0035`) in InControl mode,
//! over class-compliant USB MIDI ([`super::midi`]).
//!
//! The Launchkey Mini lists two MIDI interfaces: the first carries the piano
//! keys and is never opened; the second (its InControl port) carries the
//! pads, round buttons, arrow buttons, knobs and lights. Bitwig names that
//! port `Launchkey Mini LK Mini InControl` on macOS, `MIDIIN2 (Launchkey
//! Mini)` / `MIDIOUT2 (Launchkey Mini)` on Windows and `Launchkey Mini MIDI
//! 2` on Linux; Ableton suggests a port containing `LK Mini InControl`. So
//! [`port_matches`] takes a name ending `LK Mini InControl`, or one with
//! `Launchkey Mini` and no `MK2`, `MK3`, `MK4` or `LKMK3` that is Windows
//! interface 2, ends `InControl`, ` MIDI 2` or ` #2`, or is ALSA port 1 (not
//! ending ` MIDI 1`). No reference gives a later Launchkey Mini's names
//! besides the MK3's and MK4's (other drivers); one that shares these names
//! would be taken for this one.
//!
//! Everything is on MIDI channel 1. Start-up is queued and sent on the first
//! poll or flush, never waited on: `90 0C 7F` (Bitwig: Extended mode; Ableton:
//! Live mode on), then `90 0F 7F` (Bitwig: InControl on), then every light.
//! When the deck reports `90 0C 00` or `90 0F 00` (an inference carried over
//! from the Launchkey MK2), Ryu sends both again and repaints; reports in the
//! next 150 ms are ignored, compared against a deadline and never slept on.
//! On drop Ryu turns the lit LEDs off and sends `90 0C 00`, which hands the
//! deck back.
//!
//! In: the pads are notes 96-103 (top row) and 112-119 (bottom row), the
//! upper round button note 104 and the lower note 120, pressed above 0 and
//! released at 0 or by Note Off. The arrows are Control Change: Scene Up 104,
//! Scene Down 105, Track Left 106 and Track Right 107, above 0 pressed. Note
//! 104 and controller 104 are different keys. The knobs (`CC 21-28`) are
//! absolute, turned into dial steps of one MIDI unit from the first value
//! seen. InControl status notes 10-15 are never keys; every other channel and
//! message is dropped.
//!
//! Out: a lit key takes `90 <note> <colour>`, the colour byte `(green << 4) |
//! 0x0C | red` with each level 0 to 3 (as the first Launchpads,
//! [`super::launchpad_legacy::level`]); blue is dropped. The arrows have no
//! light. There is no brightness command: brightness scales the levels.
//!
//! Facts from Bitwig's Launchkey Mini extension (bitwig-extensions, MIT),
//! FMMT666's launchpad.py (`LaunchKeyMini`, CC BY 4.0) and Ableton Live's
//! Launchkey Mini and Launchkey remote scripts (no licence; read for ids,
//! port names, note and controller numbers only); no code was copied.

use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::launchpad_legacy::level;
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const LAUNCHKEY_MINI_ID: &str = "novation-launchkey-mini";

pub const ROWS: u8 = 2;
pub const COLS: u8 = 13;
/// The 16 pads, the two round buttons and the four arrows.
pub const KEYS: u8 = 22;
pub const DIALS: u8 = 8;

/// The 16 pads, in two rows of eight; they are keys 0 to 15.
const PADS: usize = 16;
const ROW: u8 = 8;
/// The first note of each pad row.
const PAD_ROWS: [u8; 2] = [96, 112];
/// The column of the first pad.
const PAD_COL: u8 = 3;
/// Keys 0 to 17 (the pads and round buttons) have a light.
const LIT: usize = 18;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;

/// Notes 10 to 15 are InControl status reports, never keys.
const STATUS_NOTES: std::ops::RangeInclusive<u8> = 0x0a..=0x0f;
/// Note 12: Extended (Live) mode on at 127, off at 0.
const EXTENDED_NOTE: u8 = 0x0c;
/// Note 15: InControl on at 127.
const INCONTROL_NOTE: u8 = 0x0f;
const ON: u8 = 0x7f;
const OFF: u8 = 0x00;
const START: [[u8; 3]; 2] = [[NOTE_ON, EXTENDED_NOTE, ON], [NOTE_ON, INCONTROL_NOTE, ON]];
const EXTENDED_OFF: [u8; 3] = [NOTE_ON, EXTENDED_NOTE, OFF];

/// The colour byte's Copy and Clear flags; with both levels 0 it is off.
const FLAGS: u8 = 0x0c;
const DARK: u8 = FLAGS;

const KNOBS_FIRST_CC: u8 = 21;

/// How long after asking for InControl again the deck's reports are ignored.
/// Compared against, never slept on; none under test.
#[cfg(not(test))]
const REASSERT_GUARD: Duration = Duration::from_millis(150);
#[cfg(test)]
const REASSERT_GUARD: Duration = Duration::ZERO;

const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// What a button sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sends {
    /// A lit button, by Note On.
    Note(u8),
    /// An unlit arrow, by Control Change.
    Cc(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Button {
    sends: Sends,
    name: &'static str,
    /// (row, column) in the grid.
    place: (u8, u8),
}

/// Keys 16 to 21. The grid approximates launchpad.py's drawing of the deck:
/// Track Left and Right at the top left (its InControl button, which sends
/// nothing, would be the next cell, and the octave buttons under them), the
/// pads, the round buttons, then Scene Up and Down.
const BUTTONS: [Button; 6] = [
    Button {
        sends: Sends::Note(104),
        name: "Scene Launch",
        place: (0, 11),
    },
    Button {
        sends: Sends::Note(120),
        name: "Stop All Clips",
        place: (1, 11),
    },
    Button {
        sends: Sends::Cc(106),
        name: "Track Left",
        place: (0, 0),
    },
    Button {
        sends: Sends::Cc(107),
        name: "Track Right",
        place: (0, 1),
    },
    Button {
        sends: Sends::Cc(104),
        name: "Scene Up",
        place: (0, 12),
    },
    Button {
        sends: Sends::Cc(105),
        name: "Scene Down",
        place: (1, 12),
    },
];

const PAD_NAMES: [&str; PADS] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16",
];

/// Names of other Launchkey Minis, which other drivers take.
const NOT_FIRST: [&str; 4] = ["mk2", "mk3", "mk4", "lkmk3"];

/// Whether a port's normalised name is the first Launchkey Mini's InControl
/// port. The first interface (the piano keys) never matches.
fn is_incontrol_port(name: &PortName) -> bool {
    let lower = name.base.to_ascii_lowercase();
    if NOT_FIRST.iter().any(|token| lower.contains(token)) {
        return false;
    }
    // Ableton's suggested name has no `Launchkey`.
    if lower.ends_with("lk mini incontrol") {
        return true;
    }
    if !lower.contains("launchkey mini") {
        return false;
    }
    // Windows names the second interface `MIDIIN2 (...)`, the first bare.
    match name.interface {
        Some(interface) => interface == 2,
        None => {
            lower.ends_with("incontrol")
                || lower.ends_with(" midi 2")
                || lower.ends_with(" #2")
                || (name.alsa_port == Some(1) && !lower.ends_with(" midi 1"))
        }
    }
}

/// Whether a MIDI port, by its normalised name, is this model's InControl
/// port.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.id == LAUNCHKEY_MINI_ID && is_incontrol_port(name)
}

/// The legend on each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.id == LAUNCHKEY_MINI_ID).then(|| {
        PAD_NAMES
            .iter()
            .copied()
            .chain(BUTTONS.iter().map(|button| button.name))
            .collect()
    })
}

/// Each key's cell in the 2 x 13 grid.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    if model.id != LAUNCHKEY_MINI_ID {
        return None;
    }
    let cols = u16::from(model.cols);
    let row = u16::from(ROW);
    let pads = (0..PADS as u16).map(|pad| (pad / row) * cols + u16::from(PAD_COL) + pad % row);
    let buttons = BUTTONS
        .iter()
        .map(|button| u16::from(button.place.0) * cols + u16::from(button.place.1));
    Some(pads.chain(buttons).collect())
}

/// The keys with a light: the pads and the round buttons.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    (model.id == LAUNCHKEY_MINI_ID).then(|| (0..LIT as u8).collect())
}

/// The colour byte for an RGB colour: red and green at four levels each,
/// scaled by `percent`, with the Copy and Clear flags; no blue.
fn color_byte(color: [u8; 3], percent: u8) -> u8 {
    let red = level(color[0], percent).min(3);
    let green = level(color[1], percent).min(3);
    (green << 4) | FLAGS | red
}

/// The note a lit key sends and is lit by.
fn note_of(key: usize) -> Option<u8> {
    if key < PADS {
        let row = key / usize::from(ROW);
        let col = (key % usize::from(ROW)) as u8;
        return PAD_ROWS.get(row).map(|first| first + col);
    }
    match BUTTONS.get(key - PADS)?.sends {
        Sends::Note(number) => Some(number),
        Sends::Cc(_) => None,
    }
}

pub struct LaunchkeyMini {
    transport: Box<dyn Transport>,
    /// Whether the start-up messages went out, so drop hands it back.
    started: bool,
    /// Messages to send on the next flush, before the lights.
    queued: Vec<[u8; 3]>,
    /// Until when the deck's InControl reports are ignored after Ryu asked
    /// for InControl again.
    quiet_until: Option<Instant>,
    guard: Duration,
    colors: Vec<[u8; 3]>,
    /// What each lit key's LED was last sent; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    /// Each knob's last absolute value; `None` until one arrives.
    dials: Vec<Option<u8>>,
    brightness: u8,
    parser: Parser,
}

impl LaunchkeyMini {
    #[must_use]
    pub fn new(transport: Box<dyn Transport>) -> Self {
        let keys = usize::from(KEYS);
        Self {
            transport,
            started: false,
            queued: Vec::new(),
            quiet_until: None,
            guard: REASSERT_GUARD,
            colors: vec![BLACK; keys],
            sent: vec![None; LIT],
            held: vec![false; keys],
            dials: vec![None; usize::from(DIALS)],
            brightness: 100,
            parser: Parser::default(),
        }
    }

    /// Queues the start-up messages and forgets the lights so the next flush
    /// paints them all.
    fn queue_start(&mut self) {
        self.queued.extend(START);
        self.sent.fill(None);
    }

    /// Asks for InControl again after the deck left it, letting go of
    /// everything held; returns whether it did.
    fn reassert(&mut self, events: &mut Vec<Input>) -> bool {
        if self.quiet_until.is_some_and(|until| Instant::now() < until) {
            return false;
        }
        self.queue_start();
        self.quiet_until = Some(Instant::now() + self.guard);
        for key in 0..self.held.len() {
            self.press(key, false, events);
        }
        self.dials.fill(None);
        true
    }

    /// Handles one message; returns whether something must be sent.
    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> bool {
        let Message::Channel([status, number, value]) = *message else {
            return false;
        };
        match status {
            NOTE_ON | NOTE_OFF if STATUS_NOTES.contains(&number) => {
                status == NOTE_ON
                    && value == OFF
                    && (number == EXTENDED_NOTE || number == INCONTROL_NOTE)
                    && self.reassert(events)
            }
            NOTE_ON | NOTE_OFF => {
                if let Some(key) = key_of(Sends::Note(number)) {
                    self.press(key, status == NOTE_ON && value > 0, events);
                }
                false
            }
            CONTROL_CHANGE => {
                if let Some(key) = key_of(Sends::Cc(number)) {
                    self.press(key, value > 0, events);
                } else if let Some(dial) = number
                    .checked_sub(KNOBS_FIRST_CC)
                    .filter(|dial| *dial < DIALS)
                {
                    self.turn(dial, value, events);
                }
                false
            }
            _ => false,
        }
    }

    fn press(&mut self, key: usize, pressed: bool, events: &mut Vec<Input>) {
        let Some(held) = self.held.get_mut(key) else {
            return;
        };
        if *held == pressed {
            return;
        }
        *held = pressed;
        if let Ok(index) = u8::try_from(key) {
            events.push(Input::Key { index, pressed });
        }
    }

    /// An absolute value: the first one seeds the knob, each later one turns
    /// it by how far it moved.
    fn turn(&mut self, dial: u8, value: u8, events: &mut Vec<Input>) {
        let Some(last) = self.dials.get_mut(usize::from(dial)) else {
            return;
        };
        let Some(old) = last.replace(value) else {
            return;
        };
        let moved = i16::from(value) - i16::from(old);
        if moved != 0 {
            events.push(Input::DialTurn {
                index: dial,
                delta: i8::try_from(moved.clamp(-127, 127)).unwrap_or(0),
            });
        }
    }
}

/// The key a note or controller is.
fn key_of(sends: Sends) -> Option<usize> {
    if let Sends::Note(number) = sends {
        let pad = PAD_ROWS.iter().enumerate().find_map(|(row, first)| {
            number
                .checked_sub(*first)
                .filter(|col| *col < ROW)
                .map(|col| row * usize::from(ROW) + usize::from(col))
        });
        if pad.is_some() {
            return pad;
        }
    }
    BUTTONS
        .iter()
        .position(|button| button.sends == sends)
        .map(|at| PADS + at)
}

impl Driver for LaunchkeyMini {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
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
        let mut send = !self.started;
        for message in &messages {
            send |= self.receive(message, &mut events);
        }
        if send {
            self.flush()?;
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// The queued start-up messages, then one Note On per light that changed.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.started {
            self.started = true;
            self.queue_start();
        }
        for message in std::mem::take(&mut self.queued) {
            self.transport.write(&message)?;
        }
        for key in 0..LIT {
            let (Some(note), Some(color)) = (note_of(key), self.colors.get(key)) else {
                continue;
            };
            let value = color_byte(*color, self.brightness);
            if self.sent.get(key).copied().flatten() != Some(value) {
                self.transport.write(&[NOTE_ON, note, value])?;
                if let Some(sent) = self.sent.get_mut(key) {
                    *sent = Some(value);
                }
            }
        }
        Ok(())
    }

    /// No brightness command: the levels are scaled and the changed lights
    /// sent again.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }
}

impl Drop for LaunchkeyMini {
    /// Turns the lit LEDs off and leaves Extended mode, which hands the deck
    /// back. Fire and forget: no delay.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for key in 0..LIT {
            let lit = self
                .sent
                .get(key)
                .copied()
                .flatten()
                .is_some_and(|value| value != DARK);
            if let (true, Some(note)) = (lit, note_of(key)) {
                let _ = self.transport.write(&[NOTE_ON, note, DARK]);
            }
        }
        let _ = self.transport.write(&EXTENDED_OFF);
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify, identify_midi, ModelView, Protocol, MODELS};

    fn open() -> (LaunchkeyMini, Recorder) {
        let recorder = Recorder::default();
        let deck = LaunchkeyMini::new(Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// Opens a deck and lets it send its start-up.
    fn ready() -> (LaunchkeyMini, Recorder) {
        let (mut deck, recorder) = open();
        deck.poll(0).unwrap();
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

    fn feed(deck: &mut LaunchkeyMini, recorder: &Recorder, messages: &[&[u8]]) -> Vec<Input> {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
        deck.poll(0).unwrap()
    }

    #[test]
    fn nothing_is_sent_before_the_first_poll_or_flush() {
        let (deck, recorder) = open();
        assert!(recorder.writes().is_empty());
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn start_up_turns_incontrol_on_and_paints_every_light() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(1, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(2, Some(&lit([255, 255, 0]))).unwrap();
        deck.set_face(15, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(16, Some(&lit([255, 255, 255]))).unwrap();
        // An arrow has no light.
        deck.set_face(20, Some(&lit([255, 0, 0]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[..2],
            [vec![0x90, 0x0c, 0x7f], vec![0x90, 0x0f, 0x7f]]
        );
        assert_eq!(writes.len(), 2 + 18);
        assert_eq!(writes[2], vec![0x90, 0x60, 0x0f]);
        assert_eq!(writes[3], vec![0x90, 0x61, 0x3c]);
        assert_eq!(writes[4], vec![0x90, 0x62, 0x3f]);
        assert_eq!(writes[2 + 8], vec![0x90, 0x70, 0x0c]);
        // Blue is dropped.
        assert_eq!(writes[2 + 15], vec![0x90, 0x77, 0x0c]);
        assert_eq!(writes[18], vec![0x90, 104, 0x3f]);
        assert_eq!(writes[19], vec![0x90, 120, 0x0c]);
        // Nothing changed: nothing sent; one change, one message.
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 20);
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[20..], [vec![0x90, 0x60, 0x0c]]);
        deck.set_face(0, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[21..], [vec![0x90, 0x60, 0x3c]]);
    }

    #[test]
    fn brightness_scales_the_levels() {
        let (mut deck, recorder) = ready();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(1, Some(&lit([255, 255, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_brightness(50).unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [vec![0x90, 0x60, 0x0e], vec![0x90, 0x61, 0x2e]]
        );
        deck.set_brightness(0).unwrap();
        assert_eq!(
            recorder.writes()[before + 2..],
            [vec![0x90, 0x60, 0x0c], vec![0x90, 0x61, 0x0c]]
        );
    }

    #[test]
    fn pads_round_buttons_and_arrows_map_by_message_type() {
        let (mut deck, recorder) = ready();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x90, 0x60, 0x40],
                &[0x80, 0x60, 0x00],
                &[0x90, 0x60, 0x01],
                &[0x90, 0x60, 0x00],
                &[0x90, 0x77, 0x7f],
                &[0x80, 0x77, 0x40],
                // Note 104 is the round button, controller 104 Scene Up.
                &[0x90, 0x68, 0x7f],
                &[0xb0, 0x68, 0x7f],
                &[0x90, 0x68, 0x00],
                &[0xb0, 0x68, 0x00],
                &[0x90, 0x78, 0x7f],
                &[0xb0, 0x69, 0x7f],
                &[0xb0, 0x6a, 0x7f],
                &[0xb0, 0x6b, 0x7f],
                &[0xb0, 0x6b, 0x00],
                // Not keys: notes between the rows, unused controllers.
                &[0x90, 0x69, 0x7f],
                &[0x90, 0x79, 0x7f],
                &[0xb0, 0x6c, 0x7f],
                &[0xb0, 0x78, 0x7f],
            ],
        );
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(0, false),
                key(0, true),
                key(0, false),
                key(15, true),
                key(15, false),
                key(16, true),
                key(20, true),
                key(16, false),
                key(20, false),
                key(17, true),
                key(21, true),
                key(18, true),
                key(19, true),
                key(19, false),
            ]
        );
    }

    #[test]
    fn knobs_seed_then_turn() {
        let (mut deck, recorder) = ready();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xb0, 0x15, 0x40],
                &[0xb0, 0x15, 0x43],
                &[0xb0, 0x15, 0x3f],
                &[0xb0, 0x1c, 0x00],
                &[0xb0, 0x1c, 0x7f],
                &[0xb0, 0x1d, 0x00],
                &[0xb0, 0x1d, 0x10],
            ],
        );
        assert_eq!(
            events,
            vec![
                Input::DialTurn { index: 0, delta: 3 },
                Input::DialTurn {
                    index: 0,
                    delta: -4
                },
                Input::DialTurn {
                    index: 7,
                    delta: 127
                },
            ]
        );
    }

    #[test]
    fn other_channels_and_messages_are_dropped() {
        let (mut deck, recorder) = ready();
        let before = recorder.writes().len();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x91, 0x60, 0x7f],
                &[0xb1, 0x68, 0x7f],
                &[0xb1, 0x15, 0x10],
                &[0xb1, 0x15, 0x20],
                &[0x99, 0x24, 0x7f],
                &[0xa0, 0x60, 0x30],
                &[0xd0, 0x30],
                &[0xe0, 0x00, 0x40],
                &[0xc0, 0x01],
                &[0xf8],
                &[0x90],
                &[0x90, 0x60],
                &[
                    0xf0, 0x7e, 0x7f, 0x06, 0x02, 0x00, 0x20, 0x29, 0x35, 0x00, 0x00, 0xf7,
                ],
                // InControl status notes are never keys.
                &[0x90, 0x0a, 0x7f],
                &[0x90, 0x0a, 0x00],
                &[0x90, 0x0d, 0x00],
                &[0x90, 0x0c, 0x7f],
                &[0x80, 0x0c, 0x00],
                &[],
            ],
        );
        assert!(events.is_empty());
        assert_eq!(recorder.writes().len(), before);
    }

    #[test]
    fn leaving_incontrol_is_undone_and_repainted() {
        let (mut deck, recorder) = ready();
        deck.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        let events = feed(
            &mut deck,
            &recorder,
            &[&[0x90, 0x61, 0x7f], &[0x90, 0x0f, 0x00]],
        );
        assert_eq!(events, vec![key(1, true), key(1, false)]);
        let writes = &recorder.writes()[before..];
        assert_eq!(
            writes[..2],
            [vec![0x90, 0x0c, 0x7f], vec![0x90, 0x0f, 0x7f]]
        );
        assert_eq!(writes.len(), 2 + 18);
        assert_eq!(writes[2 + 3], vec![0x90, 0x63, 0x0f]);
        // The echo at 7F changes nothing; Extended mode off does the same.
        let before = recorder.writes().len();
        feed(&mut deck, &recorder, &[&[0x90, 0x0c, 0x7f]]);
        assert_eq!(recorder.writes().len(), before);
        feed(&mut deck, &recorder, &[&[0x90, 0x0c, 0x00]]);
        assert_eq!(recorder.writes().len(), before + 2 + 18);
    }

    #[test]
    fn reports_soon_after_a_reassert_are_ignored() {
        let (mut deck, recorder) = ready();
        deck.guard = Duration::from_secs(3600);
        let before = recorder.writes().len();
        feed(
            &mut deck,
            &recorder,
            &[&[0x90, 0x0c, 0x00], &[0x90, 0x0f, 0x00]],
        );
        assert_eq!(recorder.writes().len(), before + 2 + 18);
        deck.quiet_until = Some(Instant::now());
        feed(&mut deck, &recorder, &[&[0x90, 0x0f, 0x00]]);
        assert_eq!(recorder.writes().len(), before + 2 * (2 + 18));
    }

    #[test]
    fn drop_turns_the_lights_off_and_leaves_extended_mode() {
        let (mut deck, recorder) = ready();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(17, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        assert_eq!(
            recorder.writes()[before..],
            [
                vec![0x90, 0x60, 0x0c],
                vec![0x90, 120, 0x0c],
                vec![0x90, 0x0c, 0x00]
            ]
        );
    }

    #[test]
    fn only_the_first_minis_incontrol_port_matches() {
        let cases: [(&str, bool, bool); 22] = [
            // Bitwig's names.
            ("Launchkey Mini LK Mini InControl", false, true),
            ("Launchkey Mini LK Mini MIDI", false, false),
            ("MIDIIN2 (Launchkey Mini)", true, true),
            ("MIDIOUT2 (Launchkey Mini)", true, true),
            ("Launchkey Mini", true, false),
            ("Launchkey Mini MIDI 2", false, true),
            ("Launchkey Mini MIDI 1", false, false),
            // ALSA forms.
            ("Launchkey Mini:Launchkey Mini MIDI 2 20:1", false, true),
            ("Launchkey Mini:Launchkey Mini MIDI 1 20:0", false, false),
            ("Launchkey Mini #2", false, true),
            // Ableton's suggested name.
            ("LK Mini InControl", false, true),
            ("LK Mini MIDI", false, false),
            // Never a Mini MK3 or MK4, or a full-size Launchkey.
            ("Launchkey Mini MK3 DAW Port", false, false),
            ("Launchkey Mini MK3 MIDI 2", false, false),
            ("MIDIIN2 (Launchkey Mini MK3 MID", true, false),
            ("MIDIIN2 (LKMK3 MIDI)", true, false),
            ("Launchkey Mini MK4 25 DAW Out", false, false),
            (
                "Launchkey Mini MK4 25:Launchkey Mini MK4 25 MIDI 2 28:1",
                false,
                false,
            ),
            ("MIDIIN2 (Launchkey Mini MK4 37 MIDI)", true, false),
            ("MIDIIN2 (Launchkey MIDI)", true, false),
            ("Launchkey MIDI LK Mini MK2 InControl", false, false),
            ("Launchkey MK2 49 InControl", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            let found: Vec<_> = MODELS
                .iter()
                .filter(|model| port_matches(model, &name))
                .map(|model| model.id)
                .collect();
            let want: &[&str] = if expected { &[LAUNCHKEY_MINI_ID] } else { &[] };
            assert_eq!(found, want, "{raw}");
            let identified = identify_midi(&name, windows).map(|model| model.id);
            if expected {
                assert_eq!(identified, Some(LAUNCHKEY_MINI_ID), "{raw}");
            } else {
                assert_ne!(identified, Some(LAUNCHKEY_MINI_ID), "{raw}");
            }
        }
    }

    #[test]
    fn the_row_describes_the_keys_the_driver_reads() {
        let model = by_id(LAUNCHKEY_MINI_ID).unwrap();
        assert_eq!(model.protocol, Protocol::LaunchkeyMiniMidi);
        assert!(model.protocol.is_midi() && model.protocol.has_brightness());
        assert_eq!(usize::from(model.keys), PADS + BUTTONS.len());
        assert_eq!(model.dials, DIALS);
        assert_eq!(model.dial_light_colors(), 0);
        assert_eq!(model.product_ids, &[0x0035]);
        // Shown only: the USB id never identifies it.
        assert!(identify(model.vendor_id, 0x0035, 0x0001).is_none());
        assert!(identify(model.vendor_id, 0x0035, 0).is_none());
        let view = ModelView::from(model);
        assert_eq!(view.usb_ids, vec!["1235:0035".to_owned()]);
        assert_eq!(view.key_light_colors, "red_green");
        let cells = view.key_cells.unwrap();
        assert_eq!(cells.len(), usize::from(model.keys));
        let mut unique = cells.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), cells.len());
        assert!(cells
            .iter()
            .all(|cell| *cell < u16::from(model.rows) * u16::from(model.cols)));
        assert_eq!(cells[0], 3);
        assert_eq!(cells[8], 13 + 3);
        assert_eq!(view.key_names.unwrap().len(), usize::from(model.keys));
        assert_eq!(view.lit_keys.unwrap(), (0..18).collect::<Vec<u8>>());
    }
}
