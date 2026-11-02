//! Novation's Launch Control XL 3 and Launch Control 3, in DAW mode over
//! class-compliant USB MIDI ([`super::midi`]).
//!
//! Ports: each lists a `MIDI In/Out` interface (custom modes), a `DAW In/Out`
//! interface and a DIN output; only the DAW port is opened. Its names are
//! not printed in Novation's guides, so [`port_matches`] takes a name with
//! the USB product name's `LCXL3` or `LC3` token in it (`LCXL3 1` to
//! `LCXL3 8`, `LC3 1` to `LC3 8`, after the deck's device id) that is
//! Windows' second interface (`MIDIIN2 (...)` / `MIDIOUT2 (...)`), or
//! elsewhere names `DAW` or ends in ` MIDI 2` or ` #2`. The Launch Control
//! XL MK1/MK2 and Launch Control matcher ([`super::launch_control`]) never
//! takes these names, nor this one theirs.
//!
//! Start-up, queued (never written in `new` or waited on): DAW mode on
//! (`F0 00 20 29 02 <dev> 02 7F F7`, `dev` 15 on the XL 3 and 16 on the
//! Launch Control 3), relative output for each encoder row (`B6 45 7F`,
//! `B6 48 7F`, and on the XL 3 `B6 49 7F`), every light, then the screen
//! once a face is set for it. The deck reports its surface mode on `B6 1E`:
//! on DAW Mixer or DAW Control (1, 2) Ryu asks for relative rows again and
//! repaints everything; on a Custom mode (6-9, 18-29), whose controls talk on
//! the other port, Ryu lets go of every held key, reads nothing and sends
//! nothing until a DAW mode comes back. On drop Ryu turns DAW mode off
//! (`02 00`), which hands the deck back to its own modes.
//!
//! In, the DAW port only: buttons are Control Change on channel 1, above 0
//! pressed (XL 3: 37-52, Solo/Arm 65, Mute/Select 66, the logo button 104,
//! Page Up / Down 106 / 107, Track Left / Right 103 / 102, Record 118, Play
//! 116; Launch Control 3: 37-44). Shift is `B6 3F`. Encoders on channel 16
//! are relative (`BF 4D`-`64`, offset binary around 64, above 64 clockwise),
//! with the absolute form (`BF 0D`-`24`) turning by how far it moved from
//! the first value seen; the XL 3's faders (`BF 05`-`0C`) are absolute too.
//! Touch (channel 15, `BE`, which Ableton turns on and the same controller
//! numbers as the encoders) and other feature replies are ignored.
//!
//! Out: every lit button and every encoder LED takes its exact colour by
//! SysEx (`F0 00 20 29 02 <dev> 01 53 <cc> <r> <g> <b> F7`, 7-bit), an
//! encoder LED addressed by its absolute controller number even while its row
//! is relative. Shift and the logo button are never coloured (Shift shares
//! its number with a feature control; the logo button's LED is not known).
//! The brightness controls (`B6 6F` / `B6 70`) are stored in the deck's
//! flash, so they are never sent: Ryu scales the colours. Only changes are
//! sent, from a queue that poll and flush drain a burst at a time, `pace`
//! apart, so the shared poll loop never sleeps.
//!
//! The 128 x 64 OLED is the info screen: the face is packed seven pixels a
//! byte (bit 6 the leftmost, 19 bytes a row, the last five bits spare) into
//! `F0 00 20 29 02 <dev> 09 20 <1216 bytes> F7`. The deck holds one bitmap
//! at a time and answers `F0 00 20 29 02 <dev> 09 7F` when it is ready for
//! the next, so only the latest waits, for that answer or a short timeout.
//! Brightness 0% blanks it.
//!
//! Facts from Novation's "Launch Control XL 3 Programmer's Reference Guide"
//! v1.0 and "Launch Control 3 Programmer's Reference Guide" v1.1 (Focusrite,
//! public, no code) and Ableton Live's Launch_Control_XL_3 and
//! Launch_Control_3 remote scripts as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; USB ids,
//! the start-up messages and mode handling only). No code was copied.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::face;
use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, KeyImage};

pub const XL3_ID: &str = "novation-launch-control-xl-3";
pub const LC3_ID: &str = "novation-launch-control-3";

const CONTROL_CHANGE: u8 = 0xb0;
/// Channels, as the low nibble of a status byte.
const CH1: u8 = 0x00;
const CH7: u8 = 0x06;
const CH16: u8 = 0x0f;

/// Feature controls, on channel 7 (`B6`).
const FEATURE: u8 = CONTROL_CHANGE | CH7;
const SHIFT_CC: u8 = 0x3f;
const MODE_CC: u8 = 0x1e;
/// The relative-output switch of each encoder row, top to bottom.
const XL3_RELATIVE_ROWS: [u8; 3] = [0x45, 0x48, 0x49];
const LC3_RELATIVE_ROWS: [u8; 2] = [0x45, 0x48];
/// Surface modes the deck reports: DAW Mixer and DAW Control keep the DAW
/// port; Custom modes 1-4 and 5-16 move the controls to the other port.
const DAW_MODES: [u8; 2] = [1, 2];
const CUSTOM_MODES: [std::ops::RangeInclusive<u8>; 2] = [6..=9, 18..=29];

/// Encoders, eight a row, on channel 16: absolute, and relative 64 higher.
const ROW: u8 = 8;
const ABSOLUTE_ENCODER: u8 = 0x0d;
const RELATIVE_ENCODER: u8 = 0x4d;
const RELATIVE_CENTRE: i16 = 64;
/// The XL 3's eight faders, absolute on channel 16.
const FIRST_FADER: u8 = 0x05;
/// The XL 3's faders, its last dials; they have no light.
pub const XL3_FADERS: u8 = 8;

/// The SysEx header, then the device byte.
const SYSEX_HEADER: [u8; 5] = [0xf0, 0x00, 0x20, 0x29, 0x02];
const SYSEX_END: u8 = 0xf7;
const XL3_DEVICE: u8 = 0x15;
const LC3_DEVICE: u8 = 0x16;
/// `02 7F` / `02 00`: DAW mode on and off.
const DAW_MODE: u8 = 0x02;
/// `01 53`: one control's exact RGB colour.
const RGB_LED: [u8; 2] = [0x01, 0x53];
/// `09 20`: a bitmap on the stationary display; `09 7F` back means ready.
const BITMAP: u8 = 0x09;
const BITMAP_STATIONARY: u8 = 0x20;
const BITMAP_READY: u8 = 0x7f;

/// The OLED: 128 x 64, seven pixels a byte.
const SCREEN_WIDTH: usize = 128;
const SCREEN_HEIGHT: usize = 64;
/// Bytes a row as the face is encoded (`ImageMode::MonoRows`).
const FACE_ROW_BYTES: usize = SCREEN_WIDTH / 8;
/// Bytes a row on the wire: 19 x 7 = 133 bits, the last five spare.
const WIRE_ROW_BYTES: usize = SCREEN_WIDTH.div_ceil(7);
pub const BITMAP_BYTES: usize = WIRE_ROW_BYTES * SCREEN_HEIGHT;

/// The gap after each burst of messages before the next goes out.
const PACE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(2)
};
/// How long a bitmap waits for the deck's ready answer before the next goes
/// anyway.
const SCREEN_TIMEOUT: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(250)
};
/// Messages written per burst: a full repaint of the XL 3 is about 50.
const BURST: usize = 16;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 2048;
const MAX_MESSAGES_PER_POLL: usize = 256;

/// A button: the controller it sends (and is lit by), whether Ryu lights it,
/// its name and its (row, column) in the model's grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Button {
    cc: u8,
    lit: bool,
    name: &'static str,
    place: (u8, u8),
}

const fn button(cc: u8, lit: bool, name: &'static str, place: (u8, u8)) -> Button {
    Button {
        cc,
        lit,
        name,
        place,
    }
}

/// The XL 3's buttons: keys 0-15 the two button rows, then the side and
/// navigation buttons; Shift (`B6 3F`) is key 25.
const XL3_BUTTONS: [Button; 26] = [
    button(37, true, "Button 1", (2, 2)),
    button(38, true, "Button 2", (2, 3)),
    button(39, true, "Button 3", (2, 4)),
    button(40, true, "Button 4", (2, 5)),
    button(41, true, "Button 5", (2, 6)),
    button(42, true, "Button 6", (2, 7)),
    button(43, true, "Button 7", (2, 8)),
    button(44, true, "Button 8", (2, 9)),
    button(45, true, "Button 9", (3, 2)),
    button(46, true, "Button 10", (3, 3)),
    button(47, true, "Button 11", (3, 4)),
    button(48, true, "Button 12", (3, 5)),
    button(49, true, "Button 13", (3, 6)),
    button(50, true, "Button 14", (3, 7)),
    button(51, true, "Button 15", (3, 8)),
    button(52, true, "Button 16", (3, 9)),
    button(65, true, "Solo/Arm", (2, 0)),
    button(66, true, "Mute/Select", (3, 0)),
    button(104, false, "Function", (2, 1)),
    button(106, true, "Page Up", (0, 0)),
    button(107, true, "Page Down", (0, 1)),
    button(103, true, "Track Left", (1, 0)),
    button(102, true, "Track Right", (1, 1)),
    button(118, true, "Record", (0, 2)),
    button(116, true, "Play", (0, 3)),
    button(SHIFT_CC, false, "Shift", (0, 4)),
];

/// The Launch Control 3's eight buttons (keys 0-7), then Shift (key 8). Its
/// Page, Track, Function and Mode buttons are not in Novation's map.
const LC3_BUTTONS: [Button; 9] = [
    button(37, true, "Button 1", (0, 1)),
    button(38, true, "Button 2", (0, 2)),
    button(39, true, "Button 3", (0, 3)),
    button(40, true, "Button 4", (0, 4)),
    button(41, true, "Button 5", (0, 5)),
    button(42, true, "Button 6", (0, 6)),
    button(43, true, "Button 7", (0, 7)),
    button(44, true, "Button 8", (0, 8)),
    button(SHIFT_CC, false, "Shift", (0, 0)),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Xl3,
    Lc3,
}

impl Kind {
    fn of(model: &DeviceModel) -> Option<Self> {
        match model.id {
            XL3_ID => Some(Self::Xl3),
            LC3_ID => Some(Self::Lc3),
            _ => None,
        }
    }

    const fn buttons(self) -> &'static [Button] {
        match self {
            Self::Xl3 => &XL3_BUTTONS,
            Self::Lc3 => &LC3_BUTTONS,
        }
    }

    const fn device(self) -> u8 {
        match self {
            Self::Xl3 => XL3_DEVICE,
            Self::Lc3 => LC3_DEVICE,
        }
    }

    /// The encoder rows' relative switches.
    const fn relative_rows(self) -> &'static [u8] {
        match self {
            Self::Xl3 => &XL3_RELATIVE_ROWS,
            Self::Lc3 => &LC3_RELATIVE_ROWS,
        }
    }

    /// The encoders, which each have a light: dials 0 onwards.
    fn encoders(self) -> u8 {
        u8::try_from(self.relative_rows().len()).unwrap_or(0) * ROW
    }

    const fn faders(self) -> u8 {
        match self {
            Self::Xl3 => XL3_FADERS,
            Self::Lc3 => 0,
        }
    }

    fn dials(self) -> u8 {
        self.encoders() + self.faders()
    }
}

/// Whether `word` is one of `base`'s alphanumeric tokens.
fn has_token(base: &str, word: &str) -> bool {
    base.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|token| token == word)
}

/// The model a normalised port name is the DAW port of, if any.
fn model_of_port(name: &PortName) -> Option<&'static str> {
    let base = name.base.as_str();
    let daw = match name.interface {
        Some(interface) => interface == 2,
        None => has_token(base, "DAW") || base.ends_with(" MIDI 2") || base.ends_with(" #2"),
    };
    if !daw {
        return None;
    }
    if has_token(base, "LCXL3") {
        Some(XL3_ID)
    } else if has_token(base, "LC3") {
        Some(LC3_ID)
    } else {
        None
    }
}

/// Whether a MIDI port, by its normalised name, is this model's DAW port.
/// The `MIDI` interface (Windows' first, `... MIDI 1`) and the DIN output
/// never match.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    Kind::of(model).is_some() && model_of_port(name) == Some(model.id)
}

/// The legend on each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    let kind = Kind::of(model)?;
    Some(kind.buttons().iter().map(|button| button.name).collect())
}

/// Each key's cell in the model's grid.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    let kind = Kind::of(model)?;
    let cols = u16::from(model.cols);
    Some(
        kind.buttons()
            .iter()
            .map(|button| u16::from(button.place.0) * cols + u16::from(button.place.1))
            .collect(),
    )
}

/// The keys with a light: every button but Shift (and the XL 3's logo
/// button).
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    let kind = Kind::of(model)?;
    Some(
        (0u8..)
            .zip(kind.buttons())
            .filter(|(_, button)| button.lit)
            .map(|(index, _)| index)
            .collect(),
    )
}

/// A colour scaled by `percent`.
fn scaled(color: [u8; 3], percent: u8) -> [u8; 3] {
    let percent = u16::from(percent.min(100));
    color.map(|channel| ((u16::from(channel) * percent + 50) / 100) as u8)
}

/// Repacks a face's rows (16 bytes a row, the leftmost pixel in the top bit)
/// into the OLED's: 19 bytes a row, seven pixels a byte, bit 6 the leftmost.
/// Missing input is dark.
#[must_use]
pub fn pack_bitmap(rows: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; BITMAP_BYTES];
    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            let byte = rows.get(y * FACE_ROW_BYTES + x / 8).copied().unwrap_or(0);
            if byte & (0x80 >> (x % 8)) == 0 {
                continue;
            }
            if let Some(slot) = out.get_mut(y * WIRE_ROW_BYTES + x / 7) {
                *slot |= 1 << (6 - x % 7);
            }
        }
    }
    out
}

/// What a queued message is, so a newer one of the same kind replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Daw,
    Feature(u8),
    /// A key's light (`0..keys`) or an encoder's (`keys..`).
    Light(usize),
}

pub struct LaunchControl3 {
    kind: Kind,
    transport: Box<dyn Transport>,
    /// DAW mode, relative rows and the first repaint are queued.
    started: bool,
    /// The deck is in a DAW mode (not a Custom mode).
    daw: bool,
    outbox: VecDeque<(Slot, Vec<u8>)>,
    pace: Duration,
    /// When the next burst may go out.
    next_send: Instant,
    brightness: u8,
    /// Each light's colour: the keys, then the encoders.
    colors: Vec<[u8; 3]>,
    /// What each light was last queued; `None` until it is.
    sent: Vec<Option<Vec<u8>>>,
    held: Vec<bool>,
    /// Each absolute encoder's or fader's last value; `None` until one
    /// arrives.
    dials: Vec<Option<u8>>,
    /// The info key and its encoding.
    info: Option<(u8, KeyImage)>,
    /// The packed bitmap the info face wants; `None` until a face is set.
    screen: Option<Vec<u8>>,
    /// The bitmap last written.
    screen_sent: Option<Vec<u8>>,
    /// When the last bitmap went out, until the deck says it is ready.
    screen_waiting: Option<Instant>,
    screen_timeout: Duration,
    parser: Parser,
}

impl LaunchControl3 {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model).unwrap_or(Kind::Xl3);
        let keys = kind.buttons().len();
        let lights = keys + usize::from(kind.encoders());
        Self {
            kind,
            transport,
            started: false,
            daw: true,
            outbox: VecDeque::new(),
            pace: PACE,
            next_send: Instant::now(),
            brightness: 100,
            colors: vec![BLACK; lights],
            sent: vec![None; lights],
            held: vec![false; keys],
            dials: vec![None; usize::from(kind.dials())],
            info: model.info_key().zip(model.info_image),
            screen: None,
            screen_sent: None,
            screen_waiting: None,
            screen_timeout: SCREEN_TIMEOUT,
            parser: Parser::default(),
        }
    }

    fn sysex(&self, body: &[u8]) -> Vec<u8> {
        let mut message = SYSEX_HEADER.to_vec();
        message.push(self.kind.device());
        message.extend_from_slice(body);
        message.push(SYSEX_END);
        message
    }

    fn queue(&mut self, slot: Slot, message: Vec<u8>) {
        self.outbox.retain(|(queued, _)| *queued != slot);
        self.outbox.push_back((slot, message));
    }

    fn queue_relative_rows(&mut self) {
        for &row in self.kind.relative_rows() {
            self.queue(Slot::Feature(row), vec![FEATURE, row, 0x7f]);
        }
    }

    /// Queues DAW mode, relative rows and every light, from scratch.
    fn start(&mut self) {
        self.started = true;
        self.daw = true;
        self.outbox.clear();
        let daw_on = self.sysex(&[DAW_MODE, 0x7f]);
        self.queue(Slot::Daw, daw_on);
        self.repaint();
    }

    /// Relative rows, every light and the screen again (after start-up or a
    /// DAW mode report).
    fn repaint(&mut self) {
        self.queue_relative_rows();
        self.sent.fill(None);
        self.dials.fill(None);
        self.screen_sent = None;
        self.screen_waiting = None;
        self.queue_lights();
    }

    fn ensure_started(&mut self) {
        if !self.started {
            self.start();
        }
    }

    /// The controller number a light is set by, if Ryu sets it.
    fn light_cc(&self, light: usize) -> Option<u8> {
        let buttons = self.kind.buttons();
        match light.checked_sub(buttons.len()) {
            None => buttons
                .get(light)
                .filter(|button| button.lit)
                .map(|button| button.cc),
            Some(encoder) => u8::try_from(encoder)
                .ok()
                .filter(|encoder| *encoder < self.kind.encoders())
                .map(|encoder| ABSOLUTE_ENCODER + encoder),
        }
    }

    /// The message that sets light `light` at the current colour and
    /// brightness, if Ryu sets it.
    fn light_message(&self, light: usize) -> Option<Vec<u8>> {
        let cc = self.light_cc(light)?;
        let [red, green, blue] = scaled(*self.colors.get(light)?, self.brightness);
        Some(self.sysex(&[RGB_LED[0], RGB_LED[1], cc, red >> 1, green >> 1, blue >> 1]))
    }

    /// Queues every light whose message differs from the last one queued,
    /// while the deck is in a DAW mode.
    fn queue_lights(&mut self) {
        if !self.daw {
            return;
        }
        for light in 0..self.sent.len() {
            let Some(message) = self.light_message(light) else {
                continue;
            };
            if self.sent.get(light).and_then(Option::as_ref) == Some(&message) {
                continue;
            }
            if let Some(sent) = self.sent.get_mut(light) {
                *sent = Some(message.clone());
            }
            self.queue(Slot::Light(light), message);
        }
    }

    /// The bitmap the screen should show now: the info face, or dark at 0%.
    fn screen_wanted(&self) -> Option<Vec<u8>> {
        let screen = self.screen.as_ref()?;
        Some(if self.brightness == 0 {
            vec![0; BITMAP_BYTES]
        } else {
            screen.clone()
        })
    }

    /// The bitmap message to write now, if the screen changed and the deck
    /// is ready for it (or stopped answering).
    fn screen_due(&self, now: Instant) -> Option<Vec<u8>> {
        if !self.daw {
            return None;
        }
        if let Some(waiting) = self.screen_waiting {
            if now < waiting + self.screen_timeout {
                return None;
            }
        }
        let wanted = self.screen_wanted()?;
        (self.screen_sent.as_ref() != Some(&wanted)).then_some(wanted)
    }

    /// Writes the queued messages whose turn has come, a burst at a time,
    /// then the screen when the lights are out, without ever sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        loop {
            let now = Instant::now();
            if now < self.next_send {
                break;
            }
            if self.outbox.is_empty() {
                let Some(bitmap) = self.screen_due(now) else {
                    break;
                };
                let mut body = vec![BITMAP, BITMAP_STATIONARY];
                body.extend_from_slice(&bitmap);
                let message = self.sysex(&body);
                self.transport.write(&message)?;
                self.screen_sent = Some(bitmap);
                self.screen_waiting = Some(now);
                self.next_send = now + self.pace;
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

    fn release_all(&mut self, events: &mut Vec<Input>) {
        for key in 0..self.held.len() {
            self.press(key, false, events);
        }
    }

    /// An absolute value: the first one seeds the dial, each later one turns
    /// it by how far it moved.
    fn turn_absolute(&mut self, dial: u8, value: u8, events: &mut Vec<Input>) {
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

    /// A surface mode report (`B6 1E`).
    fn mode_report(&mut self, mode: u8, events: &mut Vec<Input>) {
        if DAW_MODES.contains(&mode) {
            self.daw = true;
            self.repaint();
        } else if CUSTOM_MODES.iter().any(|range| range.contains(&mode)) && self.daw {
            self.daw = false;
            self.release_all(events);
            self.outbox.retain(|(slot, _)| *slot == Slot::Daw);
            self.screen_waiting = None;
        }
    }

    /// An encoder or fader on channel 16.
    fn analogue(&mut self, number: u8, value: u8, events: &mut Vec<Input>) {
        let encoders = self.kind.encoders();
        if let Some(encoder) = number
            .checked_sub(RELATIVE_ENCODER)
            .filter(|at| *at < encoders)
        {
            let steps = i16::from(value) - RELATIVE_CENTRE;
            if steps != 0 {
                events.push(Input::DialTurn {
                    index: encoder,
                    delta: i8::try_from(steps.clamp(-127, 127)).unwrap_or(0),
                });
            }
        } else if let Some(encoder) = number
            .checked_sub(ABSOLUTE_ENCODER)
            .filter(|at| *at < encoders)
        {
            self.turn_absolute(encoder, value, events);
        } else if let Some(fader) = number
            .checked_sub(FIRST_FADER)
            .filter(|at| *at < self.kind.faders())
        {
            self.turn_absolute(encoders + fader, value, events);
        }
    }

    /// Handles one message from the deck.
    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let [status, number, value] = match message {
            Message::Channel(bytes) => *bytes,
            Message::SysEx(bytes) => {
                let mut ready = SYSEX_HEADER.to_vec();
                ready.extend_from_slice(&[self.kind.device(), BITMAP, BITMAP_READY]);
                if bytes.starts_with(&ready) {
                    self.screen_waiting = None;
                }
                return;
            }
        };
        let kind = status & 0xf0;
        let channel = status & 0x0f;
        if kind != CONTROL_CHANGE {
            return;
        }
        if channel == CH7 && number == MODE_CC {
            self.mode_report(value, events);
            return;
        }
        if !self.daw {
            return;
        }
        match channel {
            CH7 if number == SHIFT_CC => {
                if let Some(at) = self
                    .kind
                    .buttons()
                    .iter()
                    .position(|button| button.cc == SHIFT_CC)
                {
                    self.press(at, value > 0, events);
                }
            }
            CH1 => {
                if let Some(at) = self
                    .kind
                    .buttons()
                    .iter()
                    .position(|button| button.cc == number && button.cc != SHIFT_CC)
                {
                    self.press(at, value > 0, events);
                }
            }
            CH16 => self.analogue(number, value, events),
            // Touch (channel 15) and other feature replies.
            _ => {}
        }
    }
}

impl Driver for LaunchControl3 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_started();
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
        self.pump()?;
        Ok(events)
    }

    /// The info key draws the OLED; every other key keeps its colour for its
    /// light.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some((info, format)) = self.info {
            if key == info {
                let rows = face::encode(face, &format)?;
                self.screen = Some(pack_bitmap(&rows));
                return Ok(());
            }
        }
        if usize::from(key) < self.held.len() {
            if let Some(color) = self.colors.get_mut(usize::from(key)) {
                *color = face.map_or(BLACK, |face| face.color);
            }
        }
        Ok(())
    }

    /// Queues each light that changed and writes what the pace allows.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started();
        self.queue_lights();
        self.pump()
    }

    /// The brightness controls are stored in flash, so the colours are
    /// scaled instead and every light that changes is sent again; 0% also
    /// blanks the screen.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }

    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if dial < self.kind.encoders() {
            let light = self.held.len() + usize::from(dial);
            if let Some(slot) = self.colors.get_mut(light) {
                *slot = color.unwrap_or(BLACK);
            }
        }
        Ok(())
    }
}

impl Drop for LaunchControl3 {
    /// Turns DAW mode off, which hands the deck back to its own modes; one
    /// direct write, never a wait.
    fn drop(&mut self) {
        if self.started {
            self.outbox.clear();
            let daw_off = self.sysex(&[DAW_MODE, 0x00]);
            let _ = self.transport.write(&daw_off);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{
        by_id, identify, identify_midi, DialLights, ImageMode, ModelView, Protocol, MODELS,
    };
    use image::{Rgba, RgbaImage};

    fn open(id: &str) -> (LaunchControl3, Recorder) {
        let recorder = Recorder::default();
        let deck = LaunchControl3::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        assert!(deck.pace.is_zero() && deck.screen_timeout.is_zero());
        (deck, recorder)
    }

    /// A deck past its start-up.
    fn started(id: &str) -> (LaunchControl3, Recorder) {
        let (mut deck, recorder) = open(id);
        deck.poll(0).unwrap();
        assert!(deck.outbox.is_empty());
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

    fn feed(deck: &mut LaunchControl3, recorder: &Recorder, messages: &[&[u8]]) -> Vec<Input> {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
        deck.poll(0).unwrap()
    }

    fn rgb(device: u8, cc: u8, color: [u8; 3]) -> Vec<u8> {
        vec![
            0xf0, 0x00, 0x20, 0x29, 0x02, device, 0x01, 0x53, cc, color[0], color[1], color[2],
            0xf7,
        ]
    }

    fn picture(pixels: &[(u32, u32)]) -> Face {
        let mut image = RgbaImage::from_pixel(128, 64, Rgba([0, 0, 0, 255]));
        for (x, y) in pixels {
            image.put_pixel(*x, *y, Rgba([255, 255, 255, 255]));
        }
        Face {
            label: String::new(),
            color: BLACK,
            image: Some(image),
        }
    }

    #[test]
    fn start_up_enters_daw_mode_asks_for_relative_rows_and_paints() {
        let (mut deck, recorder) = open(XL3_ID);
        // Nothing is written when the deck is made.
        assert!(recorder.writes().is_empty());
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(22, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(25, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_dial_light(23, Some([0, 255, 0])).unwrap();
        // Faders have no light.
        deck.set_dial_light(24, Some([0, 255, 0])).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[..4],
            [
                vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x15, 0x02, 0x7f, 0xf7],
                vec![0xb6, 0x45, 0x7f],
                vec![0xb6, 0x48, 0x7f],
                vec![0xb6, 0x49, 0x7f],
            ]
        );
        // 24 lit buttons (not Shift or the logo button) and 24 encoders.
        assert_eq!(writes.len(), 4 + 24 + 24);
        assert_eq!(writes[4], rgb(0x15, 37, [0x7f, 0, 0]));
        assert_eq!(writes[5], rgb(0x15, 38, [0, 0, 0]));
        // Track Right is 102.
        assert!(writes.contains(&rgb(0x15, 102, [0, 0, 0x7f])));
        // Encoder 24 is lit by its absolute number, 36.
        assert_eq!(writes.last(), Some(&rgb(0x15, 36, [0, 0x7f, 0])));
        assert!(!writes
            .iter()
            .any(|write| matches!(write.get(8), Some(&(0x3f | 104)))));
        // Nothing changed: nothing sent; one change, one message.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 52);
        deck.set_face(24, Some(&lit([255, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[52..], [rgb(0x15, 116, [0x7f, 0x7f, 0])]);
    }

    #[test]
    fn the_launch_control_3_has_two_rows_and_eight_buttons() {
        let (mut deck, recorder) = open(LC3_ID);
        deck.set_face(7, Some(&lit([254, 2, 128]))).unwrap();
        deck.set_dial_light(15, Some([255, 255, 255])).unwrap();
        deck.set_dial_light(16, Some([255, 255, 255])).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[..3],
            [
                vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x16, 0x02, 0x7f, 0xf7],
                vec![0xb6, 0x45, 0x7f],
                vec![0xb6, 0x48, 0x7f],
            ]
        );
        assert_eq!(writes.len(), 3 + 8 + 16);
        assert_eq!(writes[3 + 7], rgb(0x16, 44, [127, 1, 64]));
        assert_eq!(writes.last(), Some(&rgb(0x16, 28, [127, 127, 127])));
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xb0, 37, 0x7f],
                &[0xb0, 37, 0x00],
                &[0xb0, 44, 0x7f],
                &[0xb6, 0x3f, 0x7f],
                &[0xb6, 0x3f, 0x00],
                // Not on the Launch Control 3.
                &[0xb0, 45, 0x7f],
                &[0xb0, 106, 0x7f],
                &[0xbf, 0x5d, 0x41],
                &[0xbf, 0x05, 0x10],
                &[0xbf, 0x05, 0x20],
                // Encoder 16, relative.
                &[0xbf, 0x5c, 0x3f],
            ],
        );
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(0, false),
                key(7, true),
                key(8, true),
                key(8, false),
                turn(15, -1),
            ]
        );
    }

    #[test]
    fn buttons_encoders_and_faders_map_by_controller_and_channel() {
        let (mut deck, recorder) = started(XL3_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xb0, 52, 0x7f],
                &[0xb0, 52, 0x00],
                &[0xb0, 65, 0x7f],
                &[0xb0, 66, 0x7f],
                &[0xb0, 104, 0x7f],
                &[0xb0, 106, 0x7f],
                &[0xb0, 107, 0x7f],
                // Track Left is 103, Right 102.
                &[0xb0, 103, 0x7f],
                &[0xb0, 102, 0x7f],
                &[0xb0, 118, 0x7f],
                &[0xb0, 116, 0x7f],
                // Shift is only `B6 3F`.
                &[0xb0, 0x3f, 0x7f],
                &[0xb6, 0x3f, 0x7f],
                // Encoder 1 one step clockwise, encoder 24 three back.
                &[0xbf, 0x4d, 0x41],
                &[0xbf, 0x64, 0x3d],
                &[0xbf, 0x4d, 0x40],
                // The absolute form: seeds, then turns.
                &[0xbf, 0x0e, 0x40],
                &[0xbf, 0x0e, 0x46],
                // Faders 1 and 8 are dials 25 and 32.
                &[0xbf, 0x05, 0],
                &[0xbf, 0x05, 127],
                &[0xbf, 0x0c, 100],
                &[0xbf, 0x0c, 99],
                // Touch on channel 15 and buttons on channel 16 are not keys.
                &[0xbe, 0x0d, 0x7f],
                &[0xbe, 0x05, 0x7f],
                &[0xbf, 37, 0x7f],
                // Encoders on channel 1 are not Ryu's.
                &[0xb0, 0x4d, 0x41],
                // Notes are ignored.
                &[0x9f, 0x0c, 0x00],
            ],
        );
        assert_eq!(
            events,
            vec![
                key(15, true),
                key(15, false),
                key(16, true),
                key(17, true),
                key(18, true),
                key(19, true),
                key(20, true),
                key(21, true),
                key(22, true),
                key(23, true),
                key(24, true),
                key(25, true),
                turn(0, 1),
                turn(23, -3),
                turn(1, 6),
                turn(24, 127),
                turn(31, -1),
            ]
        );
    }

    #[test]
    fn custom_modes_pause_ryu_and_daw_modes_repaint() {
        let (mut deck, recorder) = started(XL3_ID);
        deck.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xb0, 40, 0x7f],
                // Custom mode 1: the held button is let go, input ignored.
                &[0xb6, 0x1e, 0x06],
                &[0xb0, 41, 0x7f],
                &[0xbf, 0x4d, 0x41],
            ],
        );
        assert_eq!(events, vec![key(3, true), key(3, false)]);
        // Nothing is painted while it is away.
        deck.set_face(4, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), before);
        // Custom mode 16 too.
        feed(&mut deck, &recorder, &[&[0xb6, 0x1e, 29]]);
        assert_eq!(recorder.writes().len(), before);
        // DAW Control: relative rows again, then every light.
        let events = feed(
            &mut deck,
            &recorder,
            &[&[0xb6, 0x1e, 0x02], &[0xb0, 41, 0x7f]],
        );
        assert_eq!(events, vec![key(4, true)]);
        let writes = &recorder.writes()[before..];
        assert_eq!(
            writes[..3],
            [
                vec![0xb6, 0x45, 0x7f],
                vec![0xb6, 0x48, 0x7f],
                vec![0xb6, 0x49, 0x7f],
            ]
        );
        assert_eq!(writes.len(), 3 + 24 + 24);
        assert!(writes.contains(&rgb(0x15, 40, [0x7f, 0, 0])));
        assert!(writes.contains(&rgb(0x15, 41, [0, 0x7f, 0])));
        // Other reports change nothing.
        feed(
            &mut deck,
            &recorder,
            &[
                &[0xb6, 0x1e, 0x03],
                &[0xb6, 0x45, 0x7f],
                &[0xb6, 0x6f, 0x40],
            ],
        );
        assert_eq!(recorder.writes().len(), before + 51);
    }

    #[test]
    fn the_screen_is_packed_seven_pixels_a_byte() {
        let mut rows = vec![0u8; 16 * 64];
        // (0, 0): byte 0, bit 6.
        rows[0] = 0x80;
        // (7, 0): byte 1, bit 6.
        rows[0] |= 0x01;
        // (127, 63): byte 63 * 19 + 18, bit 6 - 1.
        rows[63 * 16 + 15] = 0x01;
        let packed = pack_bitmap(&rows);
        assert_eq!(packed.len(), 1216);
        assert_eq!(packed[0], 1 << 6);
        assert_eq!(packed[1], 1 << 6);
        assert_eq!(packed[63 * 19 + 18], 1 << 5);
        // Every pixel lit fills 128 bits a row and never sets bit 7.
        let full = pack_bitmap(&[0xff; 16 * 64]);
        let set: u32 = full.iter().map(|byte| byte.count_ones()).sum();
        assert_eq!(set, 128 * 64);
        assert!(full.iter().all(|byte| *byte < 0x80));
        assert_eq!(full[18], 0b110_0000);
        // Short input is dark, never a panic.
        assert!(pack_bitmap(&[]).iter().all(|byte| *byte == 0));
    }

    #[test]
    fn the_info_key_draws_the_screen_one_bitmap_at_a_time() {
        let (mut deck, recorder) = started(LC3_ID);
        deck.screen_timeout = Duration::from_secs(60);
        let before = recorder.writes().len();
        // The info key is key 9, after Shift.
        deck.set_face(9, Some(&picture(&[(0, 0)]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 1);
        let bitmap = &writes[before];
        assert_eq!(bitmap.len(), 1225);
        assert_eq!(
            bitmap[..8],
            [0xf0, 0x00, 0x20, 0x29, 0x02, 0x16, 0x09, 0x20]
        );
        assert_eq!(bitmap[8], 1 << 6);
        assert_eq!(bitmap.last(), Some(&0xf7));
        // The info key is not a button light.
        assert!(deck.colors.iter().all(|color| *color == BLACK));
        // The next waits for the deck's answer; only the latest goes.
        deck.set_face(9, Some(&picture(&[(1, 0)]))).unwrap();
        deck.flush().unwrap();
        deck.set_face(9, Some(&picture(&[(2, 0)]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), before + 1);
        feed(
            &mut deck,
            &recorder,
            &[&[0xf0, 0x00, 0x20, 0x29, 0x02, 0x16, 0x09, 0x7f, 0xf7]],
        );
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 2);
        assert_eq!(writes[before + 1][8], 1 << 4);
        // Another deck's answer does not count.
        deck.set_face(9, None).unwrap();
        feed(
            &mut deck,
            &recorder,
            &[&[0xf0, 0x00, 0x20, 0x29, 0x02, 0x15, 0x09, 0x7f, 0xf7]],
        );
        assert_eq!(recorder.writes().len(), before + 2);
        // Once the wait runs out, it goes anyway.
        deck.screen_timeout = Duration::ZERO;
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 3);
        assert!(writes[before + 2][8..1224].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn brightness_scales_colours_and_zero_blanks_the_screen() {
        let (mut deck, recorder) = started(XL3_ID);
        deck.set_face(0, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_dial_light(0, Some([0, 0, 255])).unwrap();
        deck.set_face(26, Some(&picture(&[(0, 0)]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_brightness(40).unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [rgb(0x15, 37, [51, 51, 51]), rgb(0x15, 13, [0, 0, 51])]
        );
        // No flash-stored brightness control is ever sent.
        assert!(!recorder
            .writes()
            .iter()
            .any(|write| write[..2] == [0xb6, 0x6f] || write[..2] == [0xb6, 0x70]));
        deck.set_brightness(0).unwrap();
        let writes = recorder.writes();
        let bitmap = writes.last().unwrap();
        assert_eq!(bitmap.len(), 1225);
        assert!(bitmap[8..1224].iter().all(|byte| *byte == 0));
        drop(deck);
        assert_eq!(
            recorder.writes().last(),
            Some(&vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x15, 0x02, 0x00, 0xf7])
        );
        // A deck that never started sends nothing.
        let (deck, recorder) = open(LC3_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn writes_are_paced_by_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open(XL3_ID);
        deck.pace = Duration::from_secs(60);
        let begun = Instant::now();
        deck.poll(0).unwrap();
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), BURST);
        // A newer light for the same key replaces the queued one.
        deck.set_face(15, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        deck.set_face(15, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            deck.outbox
                .iter()
                .filter(|(slot, _)| *slot == Slot::Light(15))
                .count(),
            1
        );
        // Dropping writes DAW off at once, without waiting for the queue.
        drop(deck);
        assert!(begun.elapsed() < Duration::from_secs(1));
        let writes = recorder.writes();
        assert_eq!(writes.len(), BURST + 1);
        assert_eq!(
            writes.last(),
            Some(&vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x15, 0x02, 0x00, 0xf7])
        );
    }

    #[test]
    fn short_or_odd_input_is_ignored() {
        let (mut deck, recorder) = started(XL3_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xb0],
                &[0xf8],
                &[0xf0, 0x00, 0x20, 0x29, 0xf7],
                &[0xf0, 0xf7],
                &[0xe0, 0x00, 0x40],
                &[0xbf, 0x7f, 0x7f],
                &[0xb6],
                &[0xb6, 0x1e],
                &[],
            ],
        );
        assert!(events.is_empty());
        // Running status across reads.
        let events = feed(&mut deck, &recorder, &[&[0xb0], &[37, 0x7f, 38, 0x7f]]);
        assert_eq!(events, vec![key(0, true), key(1, true)]);
    }

    #[test]
    fn only_daw_ports_match_and_the_name_picks_the_model() {
        let cases: [(&str, bool, Option<&str>); 16] = [
            // macOS: `DAW In` / `DAW Out`; the `MIDI` pair never.
            ("LCXL3 1 DAW Out", false, Some(XL3_ID)),
            ("LCXL3 1 DAW In", false, Some(XL3_ID)),
            ("LC3 3 DAW Out", false, Some(LC3_ID)),
            ("LCXL3 1 MIDI Out", false, None),
            ("LC3 1 MIDI In", false, None),
            ("LCXL3 1 To DIN Out", false, None),
            // Windows: the second interface only.
            ("MIDIIN2 (LCXL3 1)", true, Some(XL3_ID)),
            ("MIDIOUT2 (LC3 8)", true, Some(LC3_ID)),
            ("LCXL3 1", true, None),
            ("MIDIOUT3 (LCXL3 1)", true, None),
            // Linux.
            ("LCXL3 1:LCXL3 1 MIDI 2 28:1", false, Some(XL3_ID)),
            ("LCXL3 1:LCXL3 1 MIDI 1 28:0", false, None),
            ("LC3 2:LC3 2 LC3 2 #2 32:1", false, Some(LC3_ID)),
            // Neither token hides in another word.
            ("XLC3 DAW", false, None),
            // The MK1/MK2 keep their own models.
            (
                "Launch Control XL:Launch Control XL MIDI 1 24:0",
                false,
                Some(super::super::launch_control::XL_ID),
            ),
            (
                "Launch Control",
                true,
                Some(super::super::launch_control::LC_ID),
            ),
        ];
        for (raw, windows, expected) in cases {
            let found = identify_midi(&normalise(raw), windows).map(|model| model.id);
            assert_eq!(found, expected, "{raw}");
        }
        // The MK1/MK2 matcher never takes these names.
        for raw in ["LCXL3 1 DAW Out", "MIDIIN2 (LC3 1)", "LCXL3 1 MIDI Out"] {
            let name = normalise(raw);
            for model in MODELS.iter().filter(|model| {
                matches!(
                    model.protocol,
                    Protocol::NovationLaunchControlXl | Protocol::NovationLaunchControl
                )
            }) {
                assert!(
                    !super::super::launch_control::port_matches(model, &name),
                    "{raw}"
                );
            }
        }
        assert!(model_of_port(&normalise("Launch Control XL 2 DAW")).is_none());
    }

    #[test]
    fn the_rows_describe_the_keys_the_driver_reads() {
        for id in [XL3_ID, LC3_ID] {
            let model = by_id(id).unwrap();
            let kind = Kind::of(model).unwrap();
            assert!(model.protocol.is_midi() && model.protocol.has_brightness());
            assert!(model.midi_port().is_none(), "{id}");
            assert_eq!(usize::from(model.keys), kind.buttons().len(), "{id}");
            assert_eq!(model.dials, kind.dials(), "{id}");
            assert_eq!(model.info_key(), Some(model.keys), "{id}");
            let info = model.info_image.unwrap();
            assert_eq!((info.width, info.height), (128, 64));
            assert_eq!(info.mode, ImageMode::MonoRows);
            for product in model.product_ids {
                assert!(identify(model.vendor_id, *product, 0).is_none(), "{id}");
            }
            let view = ModelView::from(model);
            assert_eq!(view.dial_lights, DialLights::PerDial);
            // Only the encoders are lit, so only they get a colour picker.
            assert_eq!(model.dial_light_colors(), kind.encoders(), "{id}");
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
        }
        let xl = ModelView::from(by_id(XL3_ID).unwrap());
        let lit = xl.lit_keys.unwrap();
        assert_eq!(lit.len(), 24);
        assert!(!lit.contains(&18) && !lit.contains(&25));
        let lc = ModelView::from(by_id(LC3_ID).unwrap());
        assert_eq!(lc.lit_keys.unwrap(), (0..8).collect::<Vec<u8>>());
    }
}
