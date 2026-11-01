//! Novation's Launchkey MK2 keyboards (25, 49 and 61) in InControl
//! ("extended") mode, over class-compliant USB MIDI ([`super::midi`]).
//!
//! Each Launchkey MK2 lists two MIDI interfaces: the first carries the piano
//! keys and is never opened; the second (its `InControl` port) carries the
//! pads, buttons, knobs, faders and lights. The exact port names are not
//! verified on hardware: Bitwig lists `Launchkey MIDI` and `MIDIIN2 (Launchkey
//! MIDI)` / `MIDIOUT2 (Launchkey MIDI)` on Windows and `Launchkey MIDI MIDI 1`
//! / `... MIDI 2` on Linux, and its macOS names are copied from the Launchkey
//! Mini's. So [`port_matches`] is a heuristic: a name with `Launchkey` and no
//! `MK3`, `MK4`, `LKMK3`, `Mini` or `88`, which is Windows interface 2, ends in
//! `InControl`, ` MIDI 2` or ` #2`, or is ALSA port 1 (not ending ` MIDI 1`).
//! The size is rarely in the name: a whole-word `25`, `49` or `61` picks that
//! model, anything else the 25 (whose controls the 49 and 61 also have). A
//! first-generation Launchkey (`Launchkey 25`) has a name of the same shape
//! and is taken for an MK2; the MK2's messages do nothing on it.
//!
//! Everything is on MIDI channel 16. Start-up is queued and sent on the first
//! poll or flush, never waited on: `9F 0C 7F` (InControl mode on), then
//! `9F 0D 7F`, `9F 0E 7F` and `9F 0F 7F` (the knobs, faders and pads in
//! InControl), then every light. When the keyboard reports a section leaving
//! InControl (`9F 0D` to `9F 10` at 0) or InControl mode itself (`9F 0C 00`),
//! Ryu sends those again and repaints; reports in the next 150 ms (which the
//! keyboard may send in answer) are ignored, compared against a deadline and
//! never slept on. On drop Ryu sends `9F 0C 00`, which hands the keyboard back.
//!
//! In: the pads are notes 96-103 (top row) and 112-119 (bottom row), Scene
//! Launch note 104 and Stop All Clips note 120, pressed at any velocity above
//! 0 and released at 0 or by Note Off; the track, scene and transport buttons
//! and (49 and 61) the fader buttons are Control Change, above 0 pressed. The
//! knobs (`CC 21-28`), and on the 49 and 61 the faders (`CC 41-48`) and master
//! fader (`CC 7`), are absolute, turned into dial steps of one MIDI unit from
//! the first value seen. Every other channel and message is dropped.
//!
//! Out: the pads and the two round buttons take an index into Novation's
//! 128-colour palette (the MK3's, [`super::launchkey::PALETTE`]) by Note On;
//! the fader buttons are on or off by Control Change (`7F` / `00`). The track,
//! scene and transport buttons are left unlit. There is no brightness
//! command: brightness scales the colour before the palette lookup.
//!
//! Facts from Bitwig's Launchkey MK2 extension (bitwig-extensions, MIT) and
//! Ableton Live's Launchkey MK2 remote script (no licence; read for note and
//! controller numbers, channels and the palette only); no code was copied.

use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::launchkey::palette_index;
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const K25_ID: &str = "novation-launchkey-25-mk2";
pub const K49_ID: &str = "novation-launchkey-49-mk2";
pub const K61_ID: &str = "novation-launchkey-61-mk2";

/// The 16 pads, in two rows of eight; they are keys 0 to 15.
pub const PADS: usize = 16;
const ROW: u8 = 8;
/// The first note of each pad row.
const PAD_ROWS: [u8; 2] = [96, 112];

const NOTE_OFF: u8 = 0x8f;
const NOTE_ON: u8 = 0x9f;
const CONTROL_CHANGE: u8 = 0xbf;

/// Note 12 turns InControl ("extended") mode on (127) or off (0).
const EXTENDED_NOTE: u8 = 0x0c;
/// Note 11 reports the InControl status; it is only read, never acted on.
const STATUS_NOTE: u8 = 0x0b;
/// Notes 13 to 15 put the knobs, faders and pads in InControl; note 16 is
/// the knobs' momentary InControl button. At 0 from the keyboard, a section
/// left InControl (or the button was let go), and Ryu asks again.
const SECTION_NOTES: std::ops::RangeInclusive<u8> = 0x0d..=0x10;
const ON: u8 = 0x7f;
const OFF: u8 = 0x00;
const EXTENDED_ON: [u8; 3] = [NOTE_ON, EXTENDED_NOTE, ON];
const EXTENDED_OFF: [u8; 3] = [NOTE_ON, EXTENDED_NOTE, OFF];
const SECTIONS_ON: [[u8; 3]; 3] = [
    [NOTE_ON, 0x0d, ON],
    [NOTE_ON, 0x0e, ON],
    [NOTE_ON, 0x0f, ON],
];

/// The knobs, then (49 and 61) the faders and master fader.
const KNOBS_FIRST_CC: u8 = 21;
const FADERS_FIRST_CC: u8 = 41;
const MASTER_FADER_CC: u8 = 7;
const KNOBS: u8 = 8;
const FADERS: u8 = 8;

/// How long after asking for InControl again the keyboard's reports are
/// ignored. Compared against, never slept on; none under test.
#[cfg(not(test))]
const REASSERT_GUARD: Duration = Duration::from_millis(150);
#[cfg(test)]
const REASSERT_GUARD: Duration = Duration::ZERO;

const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The 25.
    Small,
    /// The 49 and 61, with faders and fader buttons.
    Large,
}

impl Kind {
    fn of(model: &DeviceModel) -> Option<Self> {
        match model.id {
            K25_ID => Some(Self::Small),
            K49_ID | K61_ID => Some(Self::Large),
            _ => None,
        }
    }

    const fn buttons(self) -> &'static [Button] {
        match self {
            Self::Small => SMALL_BUTTONS,
            Self::Large => &LARGE_BUTTONS,
        }
    }

    const fn dials(self) -> u8 {
        match self {
            Self::Small => KNOBS,
            Self::Large => KNOBS + FADERS + 1,
        }
    }

    /// The dial a knob or fader controller turns.
    fn dial_of_cc(self, cc: u8) -> Option<u8> {
        if let Some(at) = cc.checked_sub(KNOBS_FIRST_CC).filter(|at| *at < KNOBS) {
            return Some(at);
        }
        if self != Self::Large {
            return None;
        }
        if cc == MASTER_FADER_CC {
            return Some(KNOBS + FADERS);
        }
        cc.checked_sub(FADERS_FIRST_CC)
            .filter(|at| *at < FADERS)
            .map(|at| KNOBS + at)
    }
}

/// What a button sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sends {
    Note(u8),
    Cc(u8),
}

/// How a button's light is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Light {
    None,
    /// A palette index by Note On.
    Palette,
    /// On or off by Control Change.
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Button {
    sends: Sends,
    light: Light,
    name: &'static str,
    /// (row, column) in the model's grid.
    place: (u8, u8),
}

const fn note(number: u8, name: &'static str, place: (u8, u8)) -> Button {
    Button {
        sends: Sends::Note(number),
        light: Light::Palette,
        name,
        place,
    }
}

const fn cc(number: u8, light: Light, name: &'static str, place: (u8, u8)) -> Button {
    Button {
        sends: Sends::Cc(number),
        light,
        name,
        place,
    }
}

/// Every button: keys 16 to 25 on the 25, and 16 to 34 on the 49 and 61.
const LARGE_BUTTONS: [Button; 19] = [
    note(104, "Scene Launch", (0, 8)),
    note(120, "Stop All Clips", (1, 8)),
    cc(102, Light::None, "Track Left", (2, 0)),
    cc(103, Light::None, "Track Right", (2, 1)),
    cc(112, Light::None, "Scene Up", (2, 2)),
    cc(113, Light::None, "Scene Down", (2, 3)),
    cc(114, Light::None, "Stop", (2, 4)),
    cc(115, Light::None, "Play", (2, 5)),
    cc(116, Light::None, "Loop", (2, 6)),
    cc(117, Light::None, "Record", (2, 7)),
    // The 49 and 61 only.
    cc(51, Light::Mono, "Fader Button 1", (3, 0)),
    cc(52, Light::Mono, "Fader Button 2", (3, 1)),
    cc(53, Light::Mono, "Fader Button 3", (3, 2)),
    cc(54, Light::Mono, "Fader Button 4", (3, 3)),
    cc(55, Light::Mono, "Fader Button 5", (3, 4)),
    cc(56, Light::Mono, "Fader Button 6", (3, 5)),
    cc(57, Light::Mono, "Fader Button 7", (3, 6)),
    cc(58, Light::Mono, "Fader Button 8", (3, 7)),
    cc(59, Light::Mono, "Master Button", (3, 8)),
];

/// The buttons the 25 has.
const SMALL_BUTTONS: &[Button] = LARGE_BUTTONS.split_at(10).0;

const PAD_NAMES: [&str; PADS] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16",
];

/// Names of other Launchkeys (and the Launchkey Mini's, which Bitwig's macOS
/// MK2 names were copied from), never an MK2.
const NOT_MK2: [&str; 5] = ["mk3", "mk4", "lkmk3", "mini", "88"];

/// The model a port's normalised name picks, if it is a Launchkey MK2's
/// InControl port. The first interface (the piano keys) never matches.
fn model_of_port(name: &PortName) -> Option<&'static str> {
    let lower = name.base.to_ascii_lowercase();
    if !lower.contains("launchkey") || NOT_MK2.iter().any(|token| lower.contains(token)) {
        return None;
    }
    // Windows names the second interface `MIDIIN2 (...)`, the first bare.
    let incontrol = match name.interface {
        Some(interface) => interface == 2,
        None => {
            lower.ends_with("incontrol")
                || lower.ends_with(" midi 2")
                || lower.ends_with(" #2")
                || (name.alsa_port == Some(1) && !lower.ends_with(" midi 1"))
        }
    };
    if !incontrol {
        return None;
    }
    let size = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find_map(|word| match word {
            "25" => Some(K25_ID),
            "49" => Some(K49_ID),
            "61" => Some(K61_ID),
            _ => None,
        });
    Some(size.unwrap_or(K25_ID))
}

/// Whether a MIDI port, by its normalised name, is this model's InControl
/// port.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    Kind::of(model).is_some() && model_of_port(name) == Some(model.id)
}

/// The legend on each key, on a Launchkey MK2.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    let kind = Kind::of(model)?;
    Some(
        PAD_NAMES
            .iter()
            .copied()
            .chain(kind.buttons().iter().map(|button| button.name))
            .collect(),
    )
}

/// Each key's cell in the model's grid: the pads in rows 1 and 2, the round
/// buttons to their right, the track, scene and transport buttons in row 3
/// and (49 and 61) the fader buttons in row 4.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    let kind = Kind::of(model)?;
    let cols = u16::from(model.cols);
    let row = u16::from(ROW);
    let pads = (0..PADS as u16).map(|pad| (pad / row) * cols + pad % row);
    let buttons = kind
        .buttons()
        .iter()
        .map(|button| u16::from(button.place.0) * cols + u16::from(button.place.1));
    Some(pads.chain(buttons).collect())
}

/// The keys with a light: the pads, the round buttons and the fader buttons.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    let kind = Kind::of(model)?;
    let pads = 0..PADS as u8;
    let buttons = (PADS as u8..)
        .zip(kind.buttons())
        .filter(|(_, button)| button.light != Light::None)
        .map(|(index, _)| index);
    Some(pads.chain(buttons).collect())
}

/// A colour scaled by `percent`.
fn scaled(color: [u8; 3], percent: u8) -> [u8; 3] {
    let percent = u16::from(percent.min(100));
    color.map(|channel| ((u16::from(channel) * percent + 50) / 100) as u8)
}

pub struct LaunchkeyMk2 {
    kind: Kind,
    transport: Box<dyn Transport>,
    /// Whether the start-up messages went out, so drop hands it back.
    started: bool,
    /// Messages to send on the next flush, before the lights.
    queued: Vec<[u8; 3]>,
    /// Until when the keyboard's InControl reports are ignored after Ryu
    /// asked for InControl again.
    quiet_until: Option<Instant>,
    guard: Duration,
    colors: Vec<[u8; 3]>,
    /// What each key's light was last sent; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    /// Each dial's last absolute value; `None` until one arrives.
    dials: Vec<Option<u8>>,
    brightness: u8,
    parser: Parser,
}

impl LaunchkeyMk2 {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model).unwrap_or(Kind::Small);
        let keys = PADS + kind.buttons().len();
        Self {
            kind,
            transport,
            started: false,
            queued: Vec::new(),
            quiet_until: None,
            guard: REASSERT_GUARD,
            colors: vec![BLACK; keys],
            sent: vec![None; keys],
            held: vec![false; keys],
            dials: vec![None; usize::from(kind.dials())],
            brightness: 100,
            parser: Parser::default(),
        }
    }

    /// Queues InControl mode and the three sections on, and forgets the
    /// lights so the next flush paints them all.
    fn queue_incontrol(&mut self, extended: bool) {
        if extended {
            self.queued.push(EXTENDED_ON);
        }
        self.queued.extend(SECTIONS_ON);
        self.sent.fill(None);
    }

    /// Whether the keyboard's InControl reports are ignored for now.
    fn quiet(&self) -> bool {
        self.quiet_until.is_some_and(|until| Instant::now() < until)
    }

    /// Asks for InControl again after the keyboard left it, letting go of
    /// everything held; returns whether it did.
    fn reassert(&mut self, extended: bool, events: &mut Vec<Input>) -> bool {
        if self.quiet() {
            return false;
        }
        self.queue_incontrol(extended);
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
            NOTE_ON | NOTE_OFF if number == EXTENDED_NOTE => {
                status == NOTE_ON && value == OFF && self.reassert(true, events)
            }
            NOTE_ON | NOTE_OFF if SECTION_NOTES.contains(&number) => {
                status == NOTE_ON && value == OFF && self.reassert(false, events)
            }
            NOTE_ON | NOTE_OFF if number == STATUS_NOTE => false,
            NOTE_ON | NOTE_OFF => {
                if let Some(key) = self.key_of(Sends::Note(number)) {
                    self.press(key, status == NOTE_ON && value > 0, events);
                }
                false
            }
            CONTROL_CHANGE => {
                if let Some(key) = self.key_of(Sends::Cc(number)) {
                    self.press(key, value > 0, events);
                } else if let Some(dial) = self.kind.dial_of_cc(number) {
                    self.turn(dial, value, events);
                }
                false
            }
            _ => false,
        }
    }

    /// The key a note or controller is.
    fn key_of(&self, sends: Sends) -> Option<usize> {
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
        self.kind
            .buttons()
            .iter()
            .position(|button| button.sends == sends)
            .map(|at| PADS + at)
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

    /// An absolute value: the first one seeds the dial, each later one turns
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

    /// The message that sets key `key`'s light at the current colour and
    /// brightness, if it has one.
    fn light_message(&self, key: usize) -> Option<[u8; 3]> {
        let color = scaled(*self.colors.get(key)?, self.brightness);
        if key < PADS {
            let row = key / usize::from(ROW);
            let col = (key % usize::from(ROW)) as u8;
            let note = PAD_ROWS.get(row)? + col;
            return Some([NOTE_ON, note, palette_index(color)]);
        }
        let button = self.kind.buttons().get(key - PADS)?;
        match (button.light, button.sends) {
            (Light::Palette, Sends::Note(number)) => Some([NOTE_ON, number, palette_index(color)]),
            (Light::Mono, Sends::Cc(number)) => Some([
                CONTROL_CHANGE,
                number,
                if color == BLACK { OFF } else { ON },
            ]),
            _ => None,
        }
    }
}

impl Driver for LaunchkeyMk2 {
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

    /// The queued start-up or InControl messages, then one message per light
    /// that changed.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.started {
            self.started = true;
            self.queue_incontrol(true);
        }
        for message in std::mem::take(&mut self.queued) {
            self.transport.write(&message)?;
        }
        for key in 0..self.sent.len() {
            let Some(message) = self.light_message(key) else {
                continue;
            };
            if self.sent.get(key).copied().flatten() != Some(message[2]) {
                self.transport.write(&message)?;
                if let Some(sent) = self.sent.get_mut(key) {
                    *sent = Some(message[2]);
                }
            }
        }
        Ok(())
    }

    /// No brightness command: the colours are scaled before the palette
    /// lookup and the changed lights are sent again.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }
}

impl Drop for LaunchkeyMk2 {
    /// Turns InControl mode off, which hands the keyboard back. No delay.
    fn drop(&mut self) {
        if self.started {
            let _ = self.transport.write(&EXTENDED_OFF);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify, identify_midi, ModelView, Protocol, MODELS};

    fn open(id: &str) -> (LaunchkeyMk2, Recorder) {
        let recorder = Recorder::default();
        let deck = LaunchkeyMk2::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// Opens a deck and lets it send its start-up.
    fn ready(id: &str) -> (LaunchkeyMk2, Recorder) {
        let (mut deck, recorder) = open(id);
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

    fn feed(deck: &mut LaunchkeyMk2, recorder: &Recorder, messages: &[&[u8]]) -> Vec<Input> {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
        deck.poll(0).unwrap()
    }

    const START: [[u8; 3]; 4] = [
        [0x9f, 0x0c, 0x7f],
        [0x9f, 0x0d, 0x7f],
        [0x9f, 0x0e, 0x7f],
        [0x9f, 0x0f, 0x7f],
    ];

    #[test]
    fn nothing_is_sent_before_the_first_poll_or_flush() {
        let (deck, recorder) = open(K25_ID);
        assert!(recorder.writes().is_empty());
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn start_up_turns_incontrol_on_and_paints_every_light() {
        let (mut deck, recorder) = open(K25_ID);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(15, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(16, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(18, Some(&lit([255, 255, 255]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[..4], START.map(|message| message.to_vec()));
        // 16 pads and the two round buttons; the CC buttons have no light.
        assert_eq!(writes.len(), 4 + 18);
        assert_eq!(writes[4], vec![0x9f, 0x60, 5]);
        assert_eq!(writes[5], vec![0x9f, 0x61, 0]);
        assert_eq!(writes[4 + 8], vec![0x9f, 0x70, 0]);
        assert_eq!(writes[4 + 15], vec![0x9f, 0x77, 45]);
        assert_eq!(writes[20], vec![0x9f, 104, 21]);
        assert_eq!(writes[21], vec![0x9f, 120, 0]);
        // Nothing changed: nothing sent; one change, one message.
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 22);
        deck.set_face(1, Some(&lit([255, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[22..], [vec![0x9f, 0x61, 13]]);
    }

    #[test]
    fn the_49_lights_its_fader_buttons_on_or_off() {
        let (mut deck, recorder) = open(K49_ID);
        deck.set_face(26, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(34, Some(&lit([1, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 4 + 18 + 9);
        assert_eq!(writes[22], vec![0xbf, 51, 0x7f]);
        assert_eq!(writes[23], vec![0xbf, 52, 0x00]);
        assert_eq!(writes[30], vec![0xbf, 59, 0x7f]);
        deck.set_face(26, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[31..], [vec![0xbf, 51, 0x00]]);
    }

    #[test]
    fn pads_round_buttons_and_cc_buttons_map_by_message_type() {
        let (mut deck, recorder) = ready(K25_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x9f, 0x60, 0x7f],
                &[0x9f, 0x60, 0x00],
                &[0x9f, 0x77, 0x10],
                &[0x8f, 0x77, 0x40],
                &[0x9f, 104, 0x7f],
                &[0x8f, 104, 0x00],
                &[0x9f, 120, 0x01],
                &[0x9f, 120, 0x00],
                // Scene Up and Down are controllers 112 and 113; the
                // bottom-left pads are notes 112 and 113.
                &[0xbf, 112, 0x7f],
                &[0xbf, 113, 0x7f],
                &[0xbf, 112, 0x00],
                &[0x9f, 112, 0x7f],
                &[0x9f, 113, 0x7f],
                &[0xbf, 102, 0x7f],
                &[0xbf, 117, 0x7f],
                // Fader buttons are not on a 25.
                &[0xbf, 51, 0x7f],
                &[0xbf, 59, 0x7f],
            ],
        );
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(0, false),
                key(15, true),
                key(15, false),
                key(16, true),
                key(16, false),
                key(17, true),
                key(17, false),
                key(20, true),
                key(21, true),
                key(20, false),
                key(8, true),
                key(9, true),
                key(18, true),
                key(25, true),
            ]
        );
    }

    #[test]
    fn knobs_seed_then_turn_and_faders_only_on_the_49_and_61() {
        let (mut deck, recorder) = ready(K25_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xbf, 21, 64],
                &[0xbf, 21, 70],
                &[0xbf, 21, 60],
                &[0xbf, 28, 0],
                &[0xbf, 41, 10],
                &[0xbf, 41, 20],
                &[0xbf, 7, 10],
                &[0xbf, 7, 20],
            ],
        );
        assert_eq!(
            events,
            vec![
                Input::DialTurn { index: 0, delta: 6 },
                Input::DialTurn {
                    index: 0,
                    delta: -10
                },
            ]
        );
        let (mut deck, recorder) = ready(K61_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xbf, 41, 0],
                &[0xbf, 41, 127],
                &[0xbf, 48, 5],
                &[0xbf, 48, 4],
                &[0xbf, 7, 100],
                &[0xbf, 7, 99],
                &[0xbf, 51, 0x7f],
                &[0xbf, 59, 0x7f],
                &[0xbf, 59, 0x00],
            ],
        );
        assert_eq!(
            events,
            vec![
                Input::DialTurn {
                    index: 8,
                    delta: 127
                },
                Input::DialTurn {
                    index: 15,
                    delta: -1
                },
                Input::DialTurn {
                    index: 16,
                    delta: -1
                },
                key(26, true),
                key(34, true),
                key(34, false),
            ]
        );
    }

    #[test]
    fn other_channels_and_messages_are_dropped() {
        let (mut deck, recorder) = ready(K61_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x90, 0x60, 0x7f],
                &[0xb0, 7, 10],
                &[0xb0, 7, 20],
                &[0xb0, 21, 10],
                &[0xb0, 21, 20],
                &[0xbe, 115, 0x7f],
                &[0x99, 40, 100],
                &[0xaf, 0x60, 0x30],
                &[0xdf, 0x30],
                &[0xef, 0x00, 0x40],
                &[0xcf, 0x01],
                &[0xf8],
                &[0x9f],
                &[
                    0xf0, 0x7e, 0x00, 0x06, 0x02, 0x00, 0x20, 0x29, 0x7c, 0x00, 0xf7,
                ],
                &[0x9f, 0x0b, 0x7f],
                &[0x9f, 0x0b, 0x00],
                &[],
            ],
        );
        assert!(events.is_empty());
    }

    #[test]
    fn leaving_incontrol_is_undone_and_repainted() {
        let (mut deck, recorder) = ready(K25_ID);
        deck.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        let events = feed(
            &mut deck,
            &recorder,
            &[&[0x9f, 0x61, 0x7f], &[0x9f, 0x0f, 0x00]],
        );
        // The held pad is let go.
        assert_eq!(events, vec![key(1, true), key(1, false)]);
        let writes = &recorder.writes()[before..];
        assert_eq!(
            writes[..3],
            START[1..]
                .iter()
                .map(|message| message.to_vec())
                .collect::<Vec<_>>()
        );
        // Every light again.
        assert_eq!(writes.len(), 3 + 18);
        assert_eq!(writes[3 + 3], vec![0x9f, 0x63, 5]);
        // Ryu's own requests echoed back at 7F change nothing.
        let before = recorder.writes().len();
        feed(
            &mut deck,
            &recorder,
            &[
                &[0x9f, 0x0d, 0x7f],
                &[0x9f, 0x0f, 0x7f],
                &[0x9f, 0x10, 0x7f],
            ],
        );
        assert_eq!(recorder.writes().len(), before);
        // The momentary knob button let go is handled the same way.
        feed(&mut deck, &recorder, &[&[0x9f, 0x10, 0x00]]);
        assert_eq!(recorder.writes().len(), before + 3 + 18);
    }

    #[test]
    fn leaving_extended_mode_turns_it_on_again() {
        let (mut deck, recorder) = ready(K49_ID);
        let before = recorder.writes().len();
        feed(&mut deck, &recorder, &[&[0x9f, 0x0c, 0x00]]);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes[..4], START.map(|message| message.to_vec()));
        assert_eq!(writes.len(), 4 + 27);
        // Its echo at 7F is ignored.
        feed(&mut deck, &recorder, &[&[0x9f, 0x0c, 0x7f]]);
        assert_eq!(recorder.writes().len(), before + 4 + 27);
    }

    #[test]
    fn reports_soon_after_a_reassert_are_ignored() {
        let (mut deck, recorder) = ready(K25_ID);
        deck.guard = Duration::from_secs(3600);
        let before = recorder.writes().len();
        feed(
            &mut deck,
            &recorder,
            &[
                &[0x9f, 0x0d, 0x00],
                &[0x9f, 0x0e, 0x00],
                &[0x9f, 0x0c, 0x00],
            ],
        );
        // One reassert, not three.
        assert_eq!(recorder.writes().len(), before + 3 + 18);
        // The deadline passed: the next report is acted on.
        deck.quiet_until = Some(Instant::now());
        feed(&mut deck, &recorder, &[&[0x9f, 0x0f, 0x00]]);
        assert_eq!(recorder.writes().len(), before + 2 * (3 + 18));
    }

    #[test]
    fn brightness_scales_colours_and_drop_leaves_extended_mode() {
        let (mut deck, recorder) = ready(K49_ID);
        deck.set_face(0, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(26, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_brightness(40).unwrap();
        // White at 40% is the palette's 757575; the mono light stays on.
        assert_eq!(recorder.writes()[before..], [vec![0x9f, 0x60, 118]]);
        deck.set_brightness(0).unwrap();
        assert_eq!(
            recorder.writes()[before + 1..],
            [vec![0x9f, 0x60, 0], vec![0xbf, 51, 0]]
        );
        drop(deck);
        assert_eq!(recorder.writes().last(), Some(&vec![0x9f, 0x0c, 0x00]));
    }

    #[test]
    fn only_incontrol_ports_match_and_the_size_picks_the_model() {
        let cases: [(&str, bool, Option<&str>); 24] = [
            // Bitwig's Windows names.
            ("MIDIIN2 (Launchkey MIDI)", true, Some(K25_ID)),
            ("MIDIOUT2 (Launchkey MIDI)", true, Some(K25_ID)),
            ("Launchkey MIDI", true, None),
            // Bitwig's Linux names, and the ALSA form.
            ("Launchkey MIDI MIDI 2", false, Some(K25_ID)),
            ("Launchkey MIDI MIDI 1", false, None),
            (
                "Launchkey MIDI:Launchkey MIDI MIDI 2 20:1",
                false,
                Some(K25_ID),
            ),
            ("Launchkey MIDI:Launchkey MIDI MIDI 1 20:0", false, None),
            ("Launchkey MIDI #2", false, Some(K25_ID)),
            // Bitwig's macOS names, copied from the Launchkey Mini's.
            ("Launchkey MIDI LK Mini MIDI", false, None),
            ("Launchkey MIDI LK Mini InControl", false, None),
            // A size in the name picks the model.
            ("Launchkey MK2 49 InControl", false, Some(K49_ID)),
            ("Launchkey 61 InControl", false, Some(K61_ID)),
            ("MIDIIN2 (Launchkey 49)", true, Some(K49_ID)),
            ("Launchkey 61:Launchkey 61 MIDI 2 24:1", false, Some(K61_ID)),
            // Never an MK3, MK4 or Mini.
            ("Launchkey Mini MK3 DAW Port", false, None),
            ("MIDIIN2 (Launchkey Mini MK3 MID", true, None),
            ("MIDIIN2 (LKMK3 MIDI)", true, None),
            ("Launchkey MK3 61:Launchkey MK3 61 MIDI 2 28:1", false, None),
            ("Launchkey MK3 88 LKMK3 DAW Out", false, None),
            (
                "Launchkey MK4 25:Launchkey MK4 25 Launchkey #2 28:1",
                false,
                None,
            ),
            ("Launchkey MK4 49 DAW Out", false, None),
            ("MIDIIN2 (Launchkey MK4 61 MIDI)", true, None),
            ("MIDIIN2 (Launchkey Mini)", true, None),
            ("Launchkey Mini:Launchkey Mini MIDI 2 24:1", false, None),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            let found = MODELS
                .iter()
                .find(|model| port_matches(model, &name))
                .map(|model| model.id);
            assert_eq!(found, expected, "{raw}");
            if let Some(id) = expected {
                assert_eq!(
                    identify_midi(&name, windows).map(|m| m.id),
                    Some(id),
                    "{raw}"
                );
            }
        }
    }

    #[test]
    fn the_rows_describe_the_keys_the_driver_reads() {
        for id in [K25_ID, K49_ID, K61_ID] {
            let model = by_id(id).unwrap();
            let kind = Kind::of(model).unwrap();
            assert_eq!(model.protocol, Protocol::LaunchkeyMk2Midi, "{id}");
            assert!(model.protocol.is_midi(), "{id}");
            assert_eq!(usize::from(model.keys), PADS + kind.buttons().len(), "{id}");
            assert_eq!(model.dials, kind.dials(), "{id}");
            // No dial has a light.
            assert_eq!(model.dial_light_colors(), 0, "{id}");
            for product in model.product_ids {
                assert!(
                    identify(model.vendor_id, *product, 0x0001).is_none(),
                    "{id}"
                );
            }
            let view = ModelView::from(model);
            let cells = view.key_cells.unwrap();
            assert_eq!(cells.len(), usize::from(model.keys), "{id}");
            let mut unique = cells.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), cells.len(), "{id}");
            assert!(cells
                .iter()
                .all(|cell| *cell < u16::from(model.rows) * u16::from(model.cols)));
            assert_eq!(view.key_names.unwrap().len(), usize::from(model.keys));
            let lit = view.lit_keys.unwrap();
            assert!((0..18).all(|index| lit.contains(&index)), "{id}");
            assert!((18..26).all(|index| !lit.contains(&index)), "{id}");
            assert_eq!(lit.len(), if kind == Kind::Large { 27 } else { 18 });
        }
        assert_eq!(
            MODELS
                .iter()
                .filter(|model| model.protocol == Protocol::LaunchkeyMk2Midi)
                .count(),
            3
        );
    }
}
