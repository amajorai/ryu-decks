//! Novation's SL MkIII keyboards (49 and 61 keys): their pads, buttons, knobs
//! and faders in InControl view, over class-compliant USB MIDI
//! ([`super::midi`]).
//!
//! Ports: the SL MkIII lists the piano keys' port first, then its InControl
//! port, then its MIDI DIN ports. Ryu opens only the InControl port, found by
//! [`port_matches`]: a name containing `SL MkIII` (any case) that names
//! `InControl` (macOS: `Novation SL MkIII SL MkIII InControl`) or `MIDI 2`
//! (newer macOS `SL MkIII MIDI 2`, ALSA `Novation SL MkIII MIDI 2`), or on
//! Windows the bare `Novation SL MkIII` as the second interface
//! (`MIDIIN2 (...)` / `MIDIOUT2 (...)`). `MIDI 1` (the keys), `MIDI 3` and the
//! DIN ports never match. The 49 and the 61 share their USB id (1235:0101)
//! and their port names, so one model stands for both; the piano, the only
//! difference, is never driven.
//!
//! The deck answers only while the user has pressed its InControl button
//! (InControl view); no message switches to it. There is no keep-alive.
//!
//! In, on channel 16 only: the 16 pads are Note On (velocity 0, or a Note Off,
//! releases), the top row notes 0x60-0x67 (keys 0-7) and the bottom row
//! 0x70-0x77 (keys 8-15). The buttons are Control Change, above 0 pressed and
//! 0 released: Scene Launch top and bottom 83-84 (keys 16-17), Soft buttons
//! 1-24 51-74 (keys 18-41), then Screen Up/Down 81-82, Pads Up/Down 85-86,
//! the right soft buttons' Up/Down 87-88, Grid 89, Options 90, Shift 91,
//! Duplicate 92, Clear 93, Track Left/Right 102-103 and the transport
//! (Rewind, Fast Forward, Stop, Play, Loop, Record) 112-117 (keys 42-60, see
//! [`BUTTONS`]). The eight knobs (CC 21-28, dials 0-7) are relative, two's
//! complement (1-63 up, 64-127 down); the eight faders (CC 41-48, dials 8-15)
//! are absolute, turned into dial steps of one MIDI unit from the first value
//! seen. The piano keys use the other port and are never seen.
//!
//! Out, all SysEx with the header `F0 00 20 29 02 0A 01`: each light is
//! `03 <led> 01 <r> <g> <b>` (7-bit components, scaled by Ryu's brightness:
//! the deck has no brightness command). The LED numbers are their own space,
//! not the controller numbers ([`BUTTONS`], the pads 0x26-0x35, the fader
//! LEDs 0x36-0x3D, which light dials 8-15). Shift has no light. The screens
//! go to the Knob layout (`01 01`) first; then each knob's light (dials 0-7)
//! is its screen column's knob icon colour (`02 <col> 04 01 <r> <g> <b>`),
//! and Soft buttons 1-8 show their label, up to nine characters, on the
//! lowest text line of the column above them (`02 <col> 01 03 <text> 00`).
//! Only changes are sent. Every message waits in a queue that poll and flush
//! drain a burst at a time, `pace` apart, so the shared poll loop never
//! sleeps. When Ryu lets go every light it may have lit goes off and the
//! screens go to the Empty layout (`01 00`).
//!
//! The piano's light guide (notes 0-60, whose LED numbers in Novation's table
//! overlap the fader and button LEDs) is left alone.
//!
//! Facts from Novation's "SL MkIII Programmer's Reference Guide" (Focusrite,
//! public, no code), Ableton Live's SL_MkIII remote script as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for
//! the USB id, port order, controller numbers and SysEx header only) and
//! DrivenByMoss (LGPL-3.0, Jürgen Moßgraber) for the port names and where the
//! buttons sit. No code was copied.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const SL_MKIII_ID: &str = "novation-sl-mkiii";

pub const KEYS: u8 = 61;
pub const DIALS: u8 = 16;
pub const ROWS: u8 = 6;
pub const COLS: u8 = 11;
const PADS: u8 = 16;
const KNOBS: u8 = 8;
const FADERS: u8 = 8;
/// Soft buttons 1-8, under the screens, are keys 18-25.
const FIRST_SOFT_KEY: u8 = 18;
const SCREEN_COLUMNS: u8 = 8;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// MIDI channel 16, from 0.
const CHANNEL: u8 = 15;
const TOP_PAD_NOTE: u8 = 0x60;
const BOTTOM_PAD_NOTE: u8 = 0x70;
const FIRST_KNOB_CC: u8 = 21;
const FIRST_FADER_CC: u8 = 41;
const FIRST_PAD_LED: u8 = 0x26;
const FIRST_FADER_LED: u8 = 0x36;

const SYSEX_HEAD: [u8; 7] = [0xf0, 0x00, 0x20, 0x29, 0x02, 0x0a, 0x01];
const SYSEX_END: u8 = 0xf7;
const SET_LAYOUT: u8 = 0x01;
const SET_PROPERTY: u8 = 0x02;
const SET_LED: u8 = 0x03;
const LAYOUT_EMPTY: u8 = 0x00;
const LAYOUT_KNOB: u8 = 0x01;
const SOLID: u8 = 0x01;
const PROPERTY_TEXT: u8 = 0x01;
const PROPERTY_RGB: u8 = 0x04;
/// The Knob layout's knob icon line colour.
const KNOB_ICON: u8 = 0x01;
/// The Knob layout's lowest text line, over the soft button.
const SOFT_LABEL: u8 = 0x03;
/// The longest text a screen property takes.
const LABEL_CHARS: usize = 9;

/// The gap after each burst of messages before the next goes out.
const PACE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(2)
};
/// Messages written per burst: a full repaint is about 80.
const BURST: usize = 16;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 256;

/// A button: its controller number, its LED number (`None`: Shift, unlit)
/// and its name. Keys 16-60 in order. Every LED number but the fader LEDs'
/// rests on Novation's guide alone (neither reference sends one), and Scene
/// Launch top and bottom take the guide's decimal 2 and 3: its hex column's
/// 0x03 and 0x04 would clash with Soft button 1.
pub const BUTTONS: [(u8, Option<u8>, &str); 45] = [
    (83, Some(0x02), "Scene Top"),
    (84, Some(0x03), "Scene Bottom"),
    (51, Some(0x04), "Soft 1"),
    (52, Some(0x05), "Soft 2"),
    (53, Some(0x06), "Soft 3"),
    (54, Some(0x07), "Soft 4"),
    (55, Some(0x08), "Soft 5"),
    (56, Some(0x09), "Soft 6"),
    (57, Some(0x0a), "Soft 7"),
    (58, Some(0x0b), "Soft 8"),
    (59, Some(0x0c), "Soft 9"),
    (60, Some(0x0d), "Soft 10"),
    (61, Some(0x0e), "Soft 11"),
    (62, Some(0x0f), "Soft 12"),
    (63, Some(0x10), "Soft 13"),
    (64, Some(0x11), "Soft 14"),
    (65, Some(0x12), "Soft 15"),
    (66, Some(0x13), "Soft 16"),
    (67, Some(0x14), "Soft 17"),
    (68, Some(0x15), "Soft 18"),
    (69, Some(0x16), "Soft 19"),
    (70, Some(0x17), "Soft 20"),
    (71, Some(0x18), "Soft 21"),
    (72, Some(0x19), "Soft 22"),
    (73, Some(0x1a), "Soft 23"),
    (74, Some(0x1b), "Soft 24"),
    (81, Some(0x3e), "Screen Up"),
    (82, Some(0x3f), "Screen Down"),
    (85, Some(0x00), "Pads Up"),
    (86, Some(0x01), "Pads Down"),
    (87, Some(0x1c), "Right Up"),
    (88, Some(0x1d), "Right Down"),
    (89, Some(0x40), "Grid"),
    (90, Some(0x41), "Options"),
    (91, None, "Shift"),
    (92, Some(0x42), "Duplicate"),
    (93, Some(0x43), "Clear"),
    (102, Some(0x1e), "Track Left"),
    (103, Some(0x1f), "Track Right"),
    (112, Some(0x21), "Rewind"),
    (113, Some(0x22), "Fast Forward"),
    (114, Some(0x23), "Stop"),
    (115, Some(0x24), "Play"),
    (116, Some(0x25), "Loop"),
    (117, Some(0x20), "Record"),
];

const PAD_NAMES: [&str; PADS as usize] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16",
];

/// Each key's (row, column) on the 6 x 11 grid: folded to fit, not exactly
/// where it sits (Soft 9-24 are really right of the screens, above the
/// faders). Shift, Duplicate, Clear and the track arrows down the left; the
/// screen, grid and pad arrows next to them; the three soft button rows and
/// the two pad rows in the middle; Options, the right soft arrows and Scene
/// Launch on the right; the transport along the bottom.
const PLACES: [(u8, u8); KEYS as usize] = {
    let mut places = [(0, 0); KEYS as usize];
    let mut pad = 0;
    while pad < PADS as usize {
        places[pad] = (3 + (pad / 8) as u8, 2 + (pad % 8) as u8);
        pad += 1;
    }
    places[16] = (3, 10);
    places[17] = (4, 10);
    let mut soft = 0;
    while soft < 24 {
        // Soft 1-8 on row 2, 9-16 on row 0, 17-24 on row 1.
        let row = match soft / 8 {
            0 => 2,
            1 => 0,
            _ => 1,
        };
        places[FIRST_SOFT_KEY as usize + soft] = (row, 2 + (soft % 8) as u8);
        soft += 1;
    }
    places[42] = (0, 1); // Screen Up
    places[43] = (1, 1); // Screen Down
    places[44] = (3, 1); // Pads Up
    places[45] = (4, 1); // Pads Down
    places[46] = (0, 10); // Right Up
    places[47] = (1, 10); // Right Down
    places[48] = (2, 1); // Grid
    places[49] = (2, 10); // Options
    places[50] = (0, 0); // Shift
    places[51] = (1, 0); // Duplicate
    places[52] = (2, 0); // Clear
    places[53] = (3, 0); // Track Left
    places[54] = (4, 0); // Track Right
    let mut transport = 0;
    while transport < 6 {
        places[55 + transport] = (5, 4 + transport as u8);
        transport += 1;
    }
    places
};

/// Whether a normalised port name is an SL MkIII's InControl port.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.protocol != Protocol::NovationSlMk3Midi {
        return false;
    }
    let base = name.base.to_ascii_lowercase();
    if !base.contains("sl mkiii") {
        return false;
    }
    let second_port = base.match_indices("mkiii midi 2").any(|(at, found)| {
        !base
            .as_bytes()
            .get(at + found.len())
            .is_some_and(u8::is_ascii_digit)
    });
    base.contains("incontrol")
        || second_port
        || (base == "novation sl mkiii" && name.interface == Some(2))
}

/// The legend on each key: the pads, then [`BUTTONS`].
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::NovationSlMk3Midi).then(|| {
        PAD_NAMES
            .iter()
            .copied()
            .chain(BUTTONS.iter().map(|(_, _, name)| *name))
            .collect()
    })
}

/// Every key but Shift has a light.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    (model.protocol == Protocol::NovationSlMk3Midi)
        .then(|| (0..KEYS).filter(|key| led(*key).is_some()).collect())
}

/// Each key's cell in the 6 x 11 grid.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    (model.protocol == Protocol::NovationSlMk3Midi).then(|| {
        PLACES
            .iter()
            .map(|(row, col)| u16::from(*row) * u16::from(COLS) + u16::from(*col))
            .collect()
    })
}

/// A key's LED number, if it has a light.
fn led(key: u8) -> Option<u8> {
    if key < PADS {
        return Some(FIRST_PAD_LED + key);
    }
    BUTTONS.get(usize::from(key - PADS))?.1
}

/// The key a button's controller number presses.
fn button_key(cc: u8) -> Option<u8> {
    let index = BUTTONS.iter().position(|(number, _, _)| *number == cc)?;
    u8::try_from(index).ok().map(|index| index + PADS)
}

/// The key a pad's note presses.
fn pad_key(note: u8) -> Option<u8> {
    match note {
        TOP_PAD_NOTE..=0x67 => Some(note - TOP_PAD_NOTE),
        BOTTOM_PAD_NOTE..=0x77 => Some(note - BOTTOM_PAD_NOTE + 8),
        _ => None,
    }
}

/// A colour at a brightness, as the deck's 7-bit components.
fn rgb7(color: [u8; 3], percent: u8) -> [u8; 3] {
    let percent = u16::from(percent.min(100));
    color.map(|channel| {
        u8::try_from((u16::from(channel) * percent + 50) / 100).unwrap_or(u8::MAX) >> 1
    })
}

/// A SysEx message with the deck's header.
fn sysex(body: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(SYSEX_HEAD.len() + body.len() + 1);
    message.extend_from_slice(&SYSEX_HEAD);
    message.extend_from_slice(body);
    message.push(SYSEX_END);
    message
}

fn led_message(led: u8, rgb: [u8; 3]) -> Vec<u8> {
    let [red, green, blue] = rgb;
    sysex(&[SET_LED, led, SOLID, red, green, blue])
}

fn layout_message(layout: u8) -> Vec<u8> {
    sysex(&[SET_LAYOUT, layout])
}

fn knob_colour_message(column: u8, rgb: [u8; 3]) -> Vec<u8> {
    let [red, green, blue] = rgb;
    sysex(&[
        SET_PROPERTY,
        column,
        PROPERTY_RGB,
        KNOB_ICON,
        red,
        green,
        blue,
    ])
}

/// Up to nine characters of a label, anything outside printable ASCII as a
/// space.
fn label_text(label: &str) -> Vec<u8> {
    label
        .chars()
        .take(LABEL_CHARS)
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c as u8
            } else {
                b' '
            }
        })
        .collect()
}

fn label_message(column: u8, text: &[u8]) -> Vec<u8> {
    let mut body = vec![SET_PROPERTY, column, PROPERTY_TEXT, SOFT_LABEL];
    body.extend_from_slice(text);
    body.push(0x00);
    sysex(&body)
}

/// What a queued message sets, so a newer one for the same thing replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Layout,
    Led(u8),
    KnobColour(u8),
    Label(u8),
}

/// A relative two's-complement step: 1-63 up, 64-127 down.
fn knob_delta(value: u8) -> Option<i8> {
    match value {
        1..=63 => i8::try_from(value).ok(),
        64..=127 => i8::try_from(i16::from(value) - 128).ok(),
        _ => None,
    }
}

pub struct SlMk3 {
    transport: Box<dyn Transport>,
    /// The layout and first repaint are queued.
    started: bool,
    outbox: VecDeque<(Slot, Vec<u8>)>,
    pace: Duration,
    /// When the next burst may go out.
    next_send: Instant,
    brightness: u8,
    colors: Vec<[u8; 3]>,
    dial_colors: Vec<[u8; 3]>,
    /// Soft buttons 1-8's labels.
    labels: Vec<Vec<u8>>,
    /// What each key's LED was last queued; `None` until it is.
    sent: Vec<Option<[u8; 3]>>,
    /// What each dial's light was last queued.
    dial_sent: Vec<Option<[u8; 3]>>,
    labels_sent: Vec<Option<Vec<u8>>>,
    held: Vec<bool>,
    /// Each fader's last absolute value; `None` until one arrives.
    faders: Vec<Option<u8>>,
    parser: Parser,
}

impl SlMk3 {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            outbox: VecDeque::new(),
            pace: PACE,
            next_send: Instant::now(),
            brightness: 100,
            colors: vec![BLACK; usize::from(KEYS)],
            dial_colors: vec![BLACK; usize::from(DIALS)],
            labels: vec![Vec::new(); usize::from(SCREEN_COLUMNS)],
            sent: vec![None; usize::from(KEYS)],
            dial_sent: vec![None; usize::from(DIALS)],
            labels_sent: vec![None; usize::from(SCREEN_COLUMNS)],
            held: vec![false; usize::from(KEYS)],
            faders: vec![None; usize::from(FADERS)],
            parser: Parser::default(),
        }
    }

    fn queue(&mut self, slot: Slot, message: Vec<u8>) {
        self.outbox.retain(|(queued, _)| *queued != slot);
        self.outbox.push_back((slot, message));
    }

    /// Writes the queued messages whose turn has come, a burst at a time,
    /// without ever sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        while !self.outbox.is_empty() {
            let now = Instant::now();
            if now < self.next_send {
                break;
            }
            for _ in 0..BURST {
                let Some((_, message)) = self.outbox.front() else {
                    break;
                };
                self.transport.write(message)?;
                self.outbox.pop_front();
            }
            self.next_send = now + self.pace;
        }
        Ok(())
    }

    /// Queues the Knob layout (which clears the column properties), then
    /// everything from scratch.
    fn ensure_started(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.queue(Slot::Layout, layout_message(LAYOUT_KNOB));
        self.queue_changes();
    }

    /// Queues every light and label that differs from what was last queued.
    fn queue_changes(&mut self) {
        for key in 0..KEYS {
            let Some(led) = led(key) else {
                continue;
            };
            let rgb = rgb7(
                self.colors.get(usize::from(key)).copied().unwrap_or(BLACK),
                self.brightness,
            );
            if let Some(sent) = self.sent.get_mut(usize::from(key)) {
                if *sent != Some(rgb) {
                    *sent = Some(rgb);
                    self.queue(Slot::Led(led), led_message(led, rgb));
                }
            }
        }
        for dial in 0..DIALS {
            let rgb = rgb7(
                self.dial_colors
                    .get(usize::from(dial))
                    .copied()
                    .unwrap_or(BLACK),
                self.brightness,
            );
            let Some(sent) = self.dial_sent.get_mut(usize::from(dial)) else {
                continue;
            };
            if *sent == Some(rgb) {
                continue;
            }
            *sent = Some(rgb);
            if dial < KNOBS {
                self.queue(Slot::KnobColour(dial), knob_colour_message(dial, rgb));
            } else {
                let led = FIRST_FADER_LED + dial - KNOBS;
                self.queue(Slot::Led(led), led_message(led, rgb));
            }
        }
        for column in 0..SCREEN_COLUMNS {
            let index = usize::from(column);
            let Some(text) = self.labels.get(index).cloned() else {
                continue;
            };
            let Some(sent) = self.labels_sent.get_mut(index) else {
                continue;
            };
            if sent.as_ref() != Some(&text) {
                *sent = Some(text.clone());
                self.queue(Slot::Label(column), label_message(column, &text));
            }
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

    /// A fader's absolute value: the first one seeds the dial, each later
    /// one turns it by how far it moved.
    fn fader(&mut self, fader: u8, value: u8, events: &mut Vec<Input>) {
        let Some(last) = self.faders.get_mut(usize::from(fader)) else {
            return;
        };
        let Some(old) = last.replace(value) else {
            return;
        };
        let moved = i16::from(value) - i16::from(old);
        if moved != 0 {
            events.push(Input::DialTurn {
                index: KNOBS + fader,
                delta: i8::try_from(moved.clamp(-127, 127)).unwrap_or(0),
            });
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        if status & 0x0f != CHANNEL {
            return;
        }
        match status & 0xf0 {
            kind @ (NOTE_ON | NOTE_OFF) => {
                if let Some(key) = pad_key(number) {
                    self.press(key, kind == NOTE_ON && value > 0, events);
                }
            }
            CONTROL_CHANGE => match number {
                cc if (FIRST_KNOB_CC..FIRST_KNOB_CC + KNOBS).contains(&cc) => {
                    if let Some(delta) = knob_delta(value) {
                        events.push(Input::DialTurn {
                            index: cc - FIRST_KNOB_CC,
                            delta,
                        });
                    }
                }
                cc if (FIRST_FADER_CC..FIRST_FADER_CC + FADERS).contains(&cc) => {
                    self.fader(cc - FIRST_FADER_CC, value, events);
                }
                cc => {
                    if let Some(key) = button_key(cc) {
                        self.press(key, value > 0, events);
                    }
                }
            },
            _ => {}
        }
    }
}

impl Driver for SlMk3 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_started();
        self.pump()?;
        let mut events = Vec::new();
        for message in &self.read(timeout_ms)? {
            self.receive(message, &mut events);
        }
        self.pump()?;
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        if let Some(column) = key
            .checked_sub(FIRST_SOFT_KEY)
            .filter(|column| *column < SCREEN_COLUMNS)
        {
            if let Some(label) = self.labels.get_mut(usize::from(column)) {
                *label = face.map(|face| label_text(&face.label)).unwrap_or_default();
            }
        }
        Ok(())
    }

    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if let Some(slot) = self.dial_colors.get_mut(usize::from(dial)) {
            *slot = color.unwrap_or(BLACK);
        }
        Ok(())
    }

    /// Queues what changed and writes what may go out now.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started();
        self.queue_changes();
        self.pump()
    }

    /// The deck has no brightness command: Ryu scales the colours it sends.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }
}

impl Drop for SlMk3 {
    /// Turns off every light that may be on and empties the screens, writing
    /// directly (a queue would never drain now), `pace` apart only when it is
    /// not zero, and stopping at the first failed write (an unplugged deck).
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        let unsent = !self.outbox.is_empty();
        self.outbox.clear();
        let dark = Some(BLACK);
        let keys = (0..KEYS).filter_map(|key| {
            let index = usize::from(key);
            let lit = self.sent.get(index) != Some(&dark);
            let held = self.held.get(index) == Some(&true);
            led(key).filter(|_| unsent || lit || held)
        });
        let faders = (KNOBS..DIALS)
            .filter(|dial| unsent || self.dial_sent.get(usize::from(*dial)) != Some(&dark))
            .map(|dial| FIRST_FADER_LED + dial - KNOBS);
        let mut messages: Vec<Vec<u8>> = keys
            .chain(faders)
            .map(|led| led_message(led, BLACK))
            .collect();
        messages.push(layout_message(LAYOUT_EMPTY));
        for (index, message) in messages.iter().enumerate() {
            if index > 0 && !self.pace.is_zero() {
                std::thread::sleep(self.pace);
            }
            if self.transport.write(message).is_err() {
                break;
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

    fn open() -> (SlMk3, Recorder) {
        let recorder = Recorder::default();
        let deck = SlMk3::new(by_id(SL_MKIII_ID).unwrap(), Box::new(recorder.clone()));
        assert!(deck.pace.is_zero());
        (deck, recorder)
    }

    /// A deck past its start-up: Knob layout, 60 key LEDs, 8 knob colours,
    /// 8 fader LEDs and 8 labels.
    fn started() -> (SlMk3, Recorder) {
        let (mut deck, recorder) = open();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 1 + 60 + 16 + 8);
        (deck, recorder)
    }

    fn lit(color: [u8; 3], label: &str) -> Face {
        Face {
            label: label.to_owned(),
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

    fn led_bytes(led: u8, rgb: [u8; 3]) -> Vec<u8> {
        vec![
            0xf0, 0x00, 0x20, 0x29, 0x02, 0x0a, 0x01, 0x03, led, 0x01, rgb[0], rgb[1], rgb[2], 0xf7,
        ]
    }

    #[test]
    fn the_start_up_sets_the_knob_layout_then_everything() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&lit([255, 0, 0], ""))).unwrap();
        deck.set_face(18, Some(&lit([0, 0, 255], "Mute all tracks")))
            .unwrap();
        deck.set_dial_light(0, Some([0, 255, 0])).unwrap();
        deck.set_dial_light(8, Some([255, 255, 255])).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 85);
        assert_eq!(
            writes[0],
            [0xf0, 0x00, 0x20, 0x29, 0x02, 0x0a, 0x01, 0x01, 0x01, 0xf7]
        );
        // Pad 1, then pad 2 dark: 13 bytes, 7-bit components.
        assert_eq!(writes[1], led_bytes(0x26, [127, 0, 0]));
        assert_eq!(writes[1].len(), 14);
        assert_eq!(writes[2], led_bytes(0x27, [0, 0, 0]));
        assert_eq!(writes[16], led_bytes(0x35, [0, 0, 0]));
        // Scene top and bottom, Soft 1 and the last soft button.
        assert_eq!(writes[17], led_bytes(0x02, [0, 0, 0]));
        assert_eq!(writes[18], led_bytes(0x03, [0, 0, 0]));
        assert_eq!(writes[19], led_bytes(0x04, [0, 0, 127]));
        assert_eq!(writes[42], led_bytes(0x1b, [0, 0, 0]));
        // Record (Shift has no LED, so 60 key messages).
        assert_eq!(writes[60], led_bytes(0x20, [0, 0, 0]));
        // Knob 1's icon colour, then fader 1's LED.
        assert_eq!(
            writes[61],
            [
                0xf0, 0x00, 0x20, 0x29, 0x02, 0x0a, 0x01, 0x02, 0x00, 0x04, 0x01, 0x00, 0x7f, 0x00,
                0xf7
            ]
        );
        assert_eq!(writes[69], led_bytes(0x36, [127, 127, 127]));
        assert_eq!(writes[76], led_bytes(0x3d, [0, 0, 0]));
        // Soft 1's label: nine characters, then 00.
        let mut label = vec![
            0xf0, 0x00, 0x20, 0x29, 0x02, 0x0a, 0x01, 0x02, 0x00, 0x01, 0x03,
        ];
        label.extend_from_slice(b"Mute all ");
        label.extend_from_slice(&[0x00, 0xf7]);
        assert_eq!(writes[77], label);
        assert_eq!(
            writes[84],
            [0xf0, 0x00, 0x20, 0x29, 0x02, 0x0a, 0x01, 0x02, 0x07, 0x01, 0x03, 0x00, 0xf7]
        );
        assert!(writes.iter().all(|write| write.len() <= 256));
        // Only once.
        deck.poll(0).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 85);
    }

    #[test]
    fn every_led_number_is_used_once() {
        let mut leds: Vec<u8> = (0..KEYS).filter_map(led).collect();
        leds.extend((0..FADERS).map(|fader| FIRST_FADER_LED + fader));
        assert_eq!(leds.len(), 68);
        leds.sort_unstable();
        leds.dedup();
        assert_eq!(leds, (0..=0x43).collect::<Vec<u8>>());
        assert_eq!(led(50), None);
    }

    #[test]
    fn pads_and_buttons_map_to_keys() {
        let (mut deck, recorder) = started();
        queue(
            &recorder,
            &[
                &[0x9f, 0x60, 0x40],
                &[0x9f, 0x60, 0x00],
                &[0x9f, 0x67, 0x7f],
                &[0x8f, 0x67, 0x00],
                &[0x9f, 0x70, 0x01],
                &[0x9f, 0x77, 0x7f],
                &[0xbf, 83, 0x7f],
                &[0xbf, 84, 0x7f],
                &[0xbf, 51, 0x7f],
                &[0xbf, 74, 0x7f],
                &[0xbf, 81, 0x7f],
                &[0xbf, 91, 0x7f],
                &[0xbf, 93, 0x7f],
                &[0xbf, 102, 0x7f],
                &[0xbf, 112, 0x7f],
                &[0xbf, 117, 0x7f],
                &[0xbf, 117, 0x00],
                // Other channels, notes and controllers outside the deck,
                // the piano port's channel 1, aftertouch, SysEx.
                &[0x90, 0x60, 0x7f],
                &[0xb0, 83, 0x7f],
                &[0x9f, 0x68, 0x7f],
                &[0x9f, 0x5f, 0x7f],
                &[0xbf, 75, 0x7f],
                &[0xbf, 118, 0x7f],
                &[0xaf, 0x60, 0x40],
                &[0xf0, 0x7e, 0x7f, 0x06, 0x02, 0xf7],
                &[0xf8],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(7, true),
                key(7, false),
                key(8, true),
                key(15, true),
                key(16, true),
                key(17, true),
                key(18, true),
                key(41, true),
                key(42, true),
                key(50, true),
                key(52, true),
                key(53, true),
                key(55, true),
                key(60, true),
                key(60, false),
            ]
        );
        // Short or broken messages are dropped.
        queue(&recorder, &[&[0xbf], &[0x9f, 0x61]]);
        assert!(deck.poll(0).unwrap().is_empty());
    }

    #[test]
    fn knobs_step_and_faders_move_from_their_first_value() {
        let (mut deck, recorder) = started();
        queue(
            &recorder,
            &[
                &[0xbf, 21, 0x01],
                &[0xbf, 21, 0x7f],
                &[0xbf, 28, 0x3f],
                &[0xbf, 28, 0x40],
                &[0xbf, 22, 0x00],
                // Fader 1: the first value only seeds it.
                &[0xbf, 41, 100],
                &[0xbf, 41, 103],
                &[0xbf, 41, 103],
                &[0xbf, 41, 90],
                &[0xbf, 48, 0],
                &[0xbf, 48, 127],
                &[0xbf, 48, 0],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, 1),
                turn(0, -1),
                turn(7, 63),
                turn(7, -64),
                turn(8, 3),
                turn(8, -13),
                turn(15, 127),
                turn(15, -127),
            ]
        );
    }

    #[test]
    fn later_flushes_send_only_changes_and_brightness_scales() {
        let (mut deck, recorder) = started();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 85);
        deck.set_face(60, Some(&lit([255, 0, 0], ""))).unwrap();
        // Shift has no light; a label change on Soft 9 is not shown.
        deck.set_face(50, Some(&lit([255, 0, 0], ""))).unwrap();
        deck.set_face(26, Some(&lit([0, 0, 0], "Hi"))).unwrap();
        deck.set_dial_light(15, Some([0, 0, 255])).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[85..],
            [led_bytes(0x20, [127, 0, 0]), led_bytes(0x3d, [0, 0, 127])]
        );
        deck.set_brightness(50).unwrap();
        let writes = &recorder.writes()[87..];
        // Only the lit ones change: (255 * 50 + 50) / 100 = 128, >> 1 = 64.
        assert_eq!(
            writes,
            [led_bytes(0x20, [64, 0, 0]), led_bytes(0x3d, [0, 0, 64])]
        );
    }

    #[test]
    fn writes_are_paced_by_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open();
        deck.pace = Duration::from_secs(60);
        let begun = Instant::now();
        deck.poll(0).unwrap();
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert_eq!(recorder.writes().len(), BURST);
        // A newer light for the same LED replaces the queued one.
        deck.set_face(15, Some(&lit([255, 255, 255], ""))).unwrap();
        deck.flush().unwrap();
        deck.set_face(15, Some(&lit([0, 255, 0], ""))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            deck.outbox
                .iter()
                .filter(|(slot, _)| *slot == Slot::Led(0x35))
                .count(),
            1
        );
        deck.pace = Duration::ZERO;
        deck.next_send = Instant::now();
        deck.poll(0).unwrap();
        assert!(deck.outbox.is_empty());
        let writes = recorder.writes();
        assert_eq!(writes.len(), 85);
        assert!(writes.contains(&led_bytes(0x35, [0, 127, 0])));
    }

    #[test]
    fn letting_go_turns_lit_leds_off_and_empties_the_screens() {
        let (mut deck, recorder) = started();
        deck.set_face(3, Some(&lit([255, 0, 0], ""))).unwrap();
        deck.set_dial_light(9, Some([255, 0, 0])).unwrap();
        deck.flush().unwrap();
        // Pad 6 held: the deck may light it itself.
        queue(&recorder, &[&[0x9f, 0x65, 0x7f]]);
        deck.poll(0).unwrap();
        drop(deck);
        assert_eq!(
            recorder.writes()[87..],
            [
                led_bytes(0x29, [0, 0, 0]),
                led_bytes(0x2b, [0, 0, 0]),
                led_bytes(0x37, [0, 0, 0]),
                vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x0a, 0x01, 0x01, 0x00, 0xf7],
            ]
        );
        // Queued lights may not have gone out: every LED goes dark.
        let (mut deck, recorder) = open();
        deck.pace = Duration::from_secs(60);
        deck.poll(0).unwrap();
        deck.pace = Duration::ZERO;
        drop(deck);
        assert_eq!(recorder.writes().len(), BURST + 68 + 1);
        // Never started: nothing sent.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_the_incontrol_port_matches() {
        let model = by_id(SL_MKIII_ID).unwrap();
        let cases = [
            ("Novation SL MkIII SL MkIII InControl", false, true),
            ("SL MkIII MIDI 2", false, true),
            (
                "Novation SL MkIII:Novation SL MkIII MIDI 2 24:1",
                false,
                true,
            ),
            ("MIDIIN2 (Novation SL MkIII)", true, true),
            ("MIDIOUT2 (Novation SL MkIII)", true, true),
            // The keys, the DIN ports and other devices.
            ("Novation SL MkIII", true, false),
            ("MIDIIN3 (Novation SL MkIII)", true, false),
            ("SL MkIII MIDI 1", false, false),
            ("SL MkIII MIDI 3", false, false),
            ("Novation SL MkIII SL MkIII MIDI", false, false),
            (
                "Novation SL MkIII:Novation SL MkIII MIDI 1 24:0",
                false,
                false,
            ),
            (
                "Novation SL MkIII:Novation SL MkIII MIDI 3 24:2",
                false,
                false,
            ),
            ("Launchkey MK3 49 LKMK3 DAW In", false, false),
            ("MIDIIN2 (Launchkey MK3 49)", true, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(SL_MKIII_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_names_and_places_every_key() {
        let model = by_id(SL_MKIII_ID).unwrap();
        let view = ModelView::from(model);
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), 61);
        assert_eq!(
            (names[0], names[15], names[16], names[18], names[41], names[50], names[60]),
            (
                "Pad 1",
                "Pad 16",
                "Scene Top",
                "Soft 1",
                "Soft 24",
                "Shift",
                "Record"
            )
        );
        let lit = view.lit_keys.unwrap();
        assert_eq!(lit.len(), 60);
        assert!(!lit.contains(&50));
        let mut cells = view.key_cells.unwrap();
        assert_eq!((cells[0], cells[8], cells[16], cells[50]), (35, 46, 43, 0));
        assert_eq!((cells[18], cells[26], cells[34]), (24, 2, 13));
        assert_eq!((cells[55], cells[60]), (59, 64));
        cells.sort_unstable();
        cells.dedup();
        assert_eq!(cells.len(), 61);
        assert!(cells.iter().all(|cell| *cell < 66));
        assert_eq!(view.key_light_colors, "rgb");
        assert_eq!(view.dial_lights, DialLights::PerDial);
        assert!(view.key_lights && view.brightness);
        assert_eq!(
            (view.keys, view.dials, view.rows, view.cols),
            (61, 16, 6, 11)
        );
    }
}
