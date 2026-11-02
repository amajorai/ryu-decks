//! Novation's Launchkey MK4 keyboards (Launchkey 25 / 37 / 49 / 61 MK4 and
//! Launchkey Mini 25 / 37 MK4) in DAW mode, over class-compliant USB MIDI
//! ([`super::midi`]).
//!
//! Ports: each Launchkey MK4 lists two MIDI interfaces; the first carries the
//! piano keys and is never opened, the second (its DAW port) carries the
//! pads, buttons, knobs, faders and lights. [`port_matches`] takes a name with
//! `Launchkey MK4 <25|37|49|61>` or `Launchkey Mini MK4 <25|37>` in it (the 88
//! is not matched) that is the DAW port: on Windows the second interface
//! (`MIDIIN2 (...)` / `MIDIOUT2 (...)`, cut to 31 characters, so the size may
//! be the last thing left), on macOS a name ending `DAW` once ` In` / ` Out`
//! are stripped (CoreMIDI lists the input Ryu reads as `... DAW Out` and the
//! output as `... DAW In`), and on Linux a name ending ` MIDI 2` or the ALSA
//! port numbered 1. Linux names are not keyed on `#2`: Bitwig's own Linux
//! detection takes the first `... Launchkey` port as DAW, which contradicts
//! Novation's guide, so ALSA's port number (or a `DAW` / `MIDI 2` name)
//! decides.
//!
//! Start-up, queued (never written in `new` or waited on): DAW mode on (Note
//! On `9F 0C 7F`), the DAW pad layout (`B6 1D 02`), the Plugin encoder mode
//! (`B6 1E 02`), on the 49 and 61 the Volume fader mode (`B6 1F 01`), relative
//! encoder output (`B6 45 7F`), then every light. There is no echo to wait
//! for and no keep-alive. The keyboard reports a user's mode change on
//! `B6 1D` / `B6 1E` / `B6 1F`: Ryu picks its pad layout and fader mode back
//! (and repaints the pads), takes its encoder mode back from a Custom Mode
//! (6-9, whose knobs talk on the other port), and after any encoder mode
//! report asks for relative output again. `9F 0C 00` from the keyboard (DAW
//! mode lost) starts it all over. On drop Ryu turns DAW mode off, which hands
//! the keyboard back to its standalone modes.
//!
//! In, the DAW port only: pads are notes 0x60-0x67 (top row, keys 0-7) and
//! 0x70-0x77 (bottom row, keys 8-15) on channel 1, velocity 0 or Note Off
//! releasing; drum-layout notes on channel 10 (Novation's guide misprints the
//! status as `9Ah`) press the same pads. Buttons are Control Change on
//! channel 1 (or 16, as in standalone mode), above 0 pressed; Shift is
//! `B6 3F`. Knobs are relative (`BF 55`-`5C`, offset binary around 64, above
//! 64 clockwise), with the absolute form (`BF 15`-`1C`) turning by how far
//! it moved from the first value seen; the 49's and 61's faders (`BF 05`-`0C`)
//! and master fader (`BF 0D`) are absolute too. Touch (`BE`), other `B6`
//! reports, aftertouch and SysEx are ignored.
//!
//! Out: a pad takes its exact colour by SysEx (`F0 00 20 29 02 <sku> 01 43
//! <note> <r> <g> <b> F7`, 7-bit, SKU `14` for the full-size ones and `13`
//! for the Minis); Scene Launch, Function, the fader buttons and Arm/Select
//! take the nearest of Novation's 128 palette colours by Control Change on
//! channel 1; the other lit buttons have one colour and take a brightness
//! (0-127) by Control Change on channel 4 ([`MONO_LED`] switches them to the
//! palette's white levels on channel 1, which Bitwig sends). Shift has no
//! light. The brightness feature controls (`B6 6F` / `B6 70`) are stored in
//! the keyboard's flash, so they are never sent: Ryu scales the colours.
//! Only changes are sent; everything waits in a queue that poll and flush
//! drain a burst at a time, `pace` apart, so the shared poll loop never
//! sleeps.
//!
//! The 128 x 64 OLED is left to the keyboard (it shows its own parameter
//! pop-ups).
//!
//! Facts from Novation's "Launchkey MK4 Programmer's Reference Guide" v2.0
//! (Focusrite, public, no code), Bitwig's Launchkey MK4 extension
//! (bitwig/bitwig-extensions, MIT; channels, port names and which buttons are
//! coloured), launchkey-sdk (Borna Cvitanic, MIT; the channel-4 brightness
//! form) and Ableton Live's Launchkey MK4 remote scripts as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; USB ids
//! only). No code was copied.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::launchkey::palette_index;
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const MINI_25_ID: &str = "novation-launchkey-mini-25-mk4";
pub const MINI_37_ID: &str = "novation-launchkey-mini-37-mk4";
pub const K25_ID: &str = "novation-launchkey-25-mk4";
pub const K37_ID: &str = "novation-launchkey-37-mk4";
pub const K49_ID: &str = "novation-launchkey-49-mk4";
pub const K61_ID: &str = "novation-launchkey-61-mk4";

/// The 16 pads, in two rows of eight; they are keys 0 to 15.
pub const PADS: usize = 16;
const ROW: usize = 8;
/// The DAW pad layout's first note on each row.
const PAD_ROWS: [u8; 2] = [0x60, 0x70];
/// The drum layout's notes (channel 10), top row then bottom row.
const DRUM_NOTES: [u8; PADS] = [
    0x28, 0x29, 0x2a, 0x2b, 0x30, 0x31, 0x32, 0x33, 0x24, 0x25, 0x26, 0x27, 0x2c, 0x2d, 0x2e, 0x2f,
];

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// Channels, as the low nibble of a status byte.
const CH1: u8 = 0x00;
const CH4: u8 = 0x03;
const CH7: u8 = 0x06;
const DRUM: u8 = 0x09;
const CH16: u8 = 0x0f;

/// Note 12 on channel 16 turns DAW mode on (127) or off (0).
const DAW_NOTE: u8 = 0x0c;
const DAW_ON: [u8; 3] = [NOTE_ON | CH16, DAW_NOTE, 0x7f];
const DAW_OFF: [u8; 3] = [NOTE_ON | CH16, DAW_NOTE, 0x00];
/// Feature and mode controllers, on channel 7 (`B6`).
const FEATURE: u8 = CONTROL_CHANGE | CH7;
const PAD_MODE: u8 = 0x1d;
const ENCODER_MODE: u8 = 0x1e;
const FADER_MODE: u8 = 0x1f;
const SHIFT_CC: u8 = 0x3f;
const RELATIVE_OUTPUT: u8 = 0x45;
/// The DAW pad layout, the Plugin encoder mode and the Volume fader mode.
const PADS_DAW: u8 = 2;
const ENCODERS_PLUGIN: u8 = 2;
const FADERS_VOLUME: u8 = 1;
/// Encoder Custom Modes, whose knobs send on the piano's port.
const ENCODER_CUSTOM: std::ops::RangeInclusive<u8> = 6..=9;

/// Knobs: relative (after `B6 45 7F`) and absolute controllers on channel 16.
const KNOBS: u8 = 8;
const RELATIVE_KNOB: u8 = 0x55;
const ABSOLUTE_KNOB: u8 = 0x15;
/// Faders 1-8 and the master fader (49 and 61), absolute on channel 16.
const FADERS: u8 = 9;
const FIRST_FADER: u8 = 0x05;
const RELATIVE_CENTRE: i16 = 64;

/// The SysEx header, then the SKU byte.
const SYSEX_HEADER: [u8; 5] = [0xf0, 0x00, 0x20, 0x29, 0x02];
const SKU_FULL: u8 = 0x14;
const SKU_MINI: u8 = 0x13;
const SYSEX_END: u8 = 0xf7;
/// `01 43`: a pad's exact RGB colour.
const PAD_RGB: [u8; 2] = [0x01, 0x43];

/// How the single-colour button lights are set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MonoLed {
    /// A brightness 0-127 by Control Change on channel 4 (Novation's guide,
    /// launchkey-sdk).
    Brightness,
    /// The palette's white levels 0-3 by Control Change on channel 1 (what
    /// Bitwig's extension sends).
    #[allow(dead_code)]
    PaletteWhite,
}

/// The single-colour lights' form: switch here if the channel-4 brightness
/// misbehaves on hardware.
const MONO_LED: MonoLed = MonoLed::Brightness;

/// The gap after each burst of messages before the next goes out.
const PACE: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(2)
};
/// Messages written per burst: a full repaint is about 50.
const BURST: usize = 16;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 256;
const MAX_MESSAGES_PER_POLL: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Mini,
    /// The 25 and 37.
    Small,
    /// The 49 and 61, with faders and fader buttons.
    Large,
}

impl Kind {
    fn of(model: &DeviceModel) -> Option<Self> {
        match model.id {
            MINI_25_ID | MINI_37_ID => Some(Self::Mini),
            K25_ID | K37_ID => Some(Self::Small),
            K49_ID | K61_ID => Some(Self::Large),
            _ => None,
        }
    }

    const fn buttons(self) -> &'static [Button] {
        match self {
            Self::Mini => &MINI_BUTTONS,
            Self::Small => SMALL_BUTTONS,
            Self::Large => &LARGE_BUTTONS,
        }
    }

    const fn dials(self) -> u8 {
        match self {
            Self::Mini | Self::Small => KNOBS,
            Self::Large => KNOBS + FADERS,
        }
    }

    const fn sku(self) -> u8 {
        match self {
            Self::Mini => SKU_MINI,
            Self::Small | Self::Large => SKU_FULL,
        }
    }
}

/// How a button's light is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Light {
    None,
    /// A palette index by Control Change on channel 1.
    Rgb,
    /// One colour, at a level ([`MONO_LED`]).
    Mono,
}

/// A button: the controller it sends (and is lit by), its light and name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Button {
    cc: u8,
    light: Light,
    name: &'static str,
    /// (row, column) in the model's grid.
    place: (u8, u8),
}

const fn button(cc: u8, light: Light, name: &'static str, place: (u8, u8)) -> Button {
    Button {
        cc,
        light,
        name,
        place,
    }
}

/// The Minis' buttons: keys 16 to 24. Shift sends `B6 3F`.
const MINI_BUTTONS: [Button; 9] = [
    button(0x6a, Light::Mono, "Pad Up", (1, 0)),
    button(0x6b, Light::Mono, "Pad Down", (2, 0)),
    button(0x68, Light::Rgb, "Scene Launch", (1, 9)),
    button(0x69, Light::Rgb, "Function", (2, 9)),
    button(0x33, Light::Mono, "Encoder Up", (0, 8)),
    button(0x34, Light::Mono, "Encoder Down", (0, 9)),
    button(SHIFT_CC, Light::None, "Shift", (0, 0)),
    button(0x73, Light::Mono, "Play", (0, 1)),
    button(0x75, Light::Mono, "Record", (0, 2)),
];

/// The full-size buttons: keys 16 to 32 on every size, 33 to 41 the 49's and
/// 61's fader buttons and Arm/Select.
const LARGE_BUTTONS: [Button; 26] = [
    button(0x6a, Light::Mono, "Pad Up", (1, 0)),
    button(0x6b, Light::Mono, "Pad Down", (2, 0)),
    button(0x68, Light::Rgb, "Scene Launch", (1, 9)),
    button(0x69, Light::Rgb, "Function", (2, 9)),
    button(0x33, Light::Mono, "Encoder Up", (0, 8)),
    button(0x34, Light::Mono, "Encoder Down", (0, 9)),
    button(SHIFT_CC, Light::None, "Shift", (0, 0)),
    button(0x67, Light::Mono, "Track Left", (0, 1)),
    button(0x66, Light::Mono, "Track Right", (0, 2)),
    button(0x4a, Light::Mono, "Capture MIDI", (3, 0)),
    button(0x4d, Light::Mono, "Undo", (3, 1)),
    button(0x4b, Light::Mono, "Quantise", (3, 2)),
    button(0x4c, Light::Mono, "Metronome", (3, 3)),
    button(0x74, Light::Mono, "Stop", (3, 6)),
    button(0x76, Light::Mono, "Loop", (3, 7)),
    button(0x73, Light::Mono, "Play", (3, 8)),
    button(0x75, Light::Mono, "Record", (3, 9)),
    button(0x25, Light::Rgb, "Fader Button 1", (4, 0)),
    button(0x26, Light::Rgb, "Fader Button 2", (4, 1)),
    button(0x27, Light::Rgb, "Fader Button 3", (4, 2)),
    button(0x28, Light::Rgb, "Fader Button 4", (4, 3)),
    button(0x29, Light::Rgb, "Fader Button 5", (4, 4)),
    button(0x2a, Light::Rgb, "Fader Button 6", (4, 5)),
    button(0x2b, Light::Rgb, "Fader Button 7", (4, 6)),
    button(0x2c, Light::Rgb, "Fader Button 8", (4, 7)),
    button(0x2d, Light::Rgb, "Arm/Select", (4, 8)),
];

/// The buttons the 25 and 37 have.
const SMALL_BUTTONS: &[Button] = LARGE_BUTTONS.split_at(17).0;

const PAD_NAMES: [&str; PADS] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16",
];

/// The two-digit size after `prefix` in `base`, when a non-digit (or the end)
/// follows it.
fn size_after<'a>(base: &'a str, prefix: &str) -> Option<&'a str> {
    let at = base.find(prefix)? + prefix.len();
    let size = base.get(at..at + 2)?;
    let next_is_digit = base.as_bytes().get(at + 2).is_some_and(u8::is_ascii_digit);
    (size.bytes().all(|byte| byte.is_ascii_digit()) && !next_is_digit).then_some(size)
}

/// The model a normalised port name is the DAW port of, if any.
fn model_of_port(name: &PortName) -> Option<&'static str> {
    let base = name.base.as_str();
    let daw = match name.interface {
        Some(interface) => interface == 2,
        None => {
            base.ends_with("DAW")
                || base.ends_with(" MIDI 2")
                || (name.alsa_port == Some(1) && !base.ends_with(" MIDI 1"))
        }
    };
    if !daw {
        return None;
    }
    if let Some(size) = size_after(base, "Launchkey Mini MK4 ") {
        return match size {
            "25" => Some(MINI_25_ID),
            "37" => Some(MINI_37_ID),
            _ => None,
        };
    }
    match size_after(base, "Launchkey MK4 ")? {
        "25" => Some(K25_ID),
        "37" => Some(K37_ID),
        "49" => Some(K49_ID),
        "61" => Some(K61_ID),
        _ => None,
    }
}

/// Whether a MIDI port, by its normalised name, is this model's DAW port.
/// The keys' port (Windows' first interface, macOS `... MIDI`, ALSA port 0 or
/// `MIDI 1`) never matches, and neither does the 88 or any MK3.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    Kind::of(model).is_some() && model_of_port(name) == Some(model.id)
}

/// The legend on each key: the pads, then the buttons.
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

/// Each key's cell: the pads in rows 1 and 2, columns 1-8, the buttons
/// around them.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    let kind = Kind::of(model)?;
    let cols = u16::from(model.cols);
    let pads = (0..PADS as u16).map(|pad| (1 + pad / 8) * cols + 1 + pad % 8);
    let buttons = kind
        .buttons()
        .iter()
        .map(|button| u16::from(button.place.0) * cols + u16::from(button.place.1));
    Some(pads.chain(buttons).collect())
}

/// The keys with a light: every pad and every button but Shift.
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

/// A single-colour light's level from its scaled colour's brightest channel
/// (so a full red or blue is full, not dim): 0-127, or palette white 0-3.
fn mono_level(color: [u8; 3]) -> u8 {
    let peak = u16::from(color.into_iter().max().unwrap_or(0));
    match MONO_LED {
        MonoLed::Brightness => ((peak * 127 + 127) / 255) as u8,
        MonoLed::PaletteWhite => match peak {
            0 => 0,
            1..=84 => 1,
            85..=169 => 2,
            _ => 3,
        },
    }
}

/// The DAW pad note of pad `pad`.
fn pad_note(pad: usize) -> Option<u8> {
    let first = PAD_ROWS.get(pad / ROW)?;
    Some(first + (pad % ROW) as u8)
}

/// The pad a note is, on `channel`.
fn pad_of_note(channel: u8, note: u8) -> Option<usize> {
    match channel {
        CH1 => PAD_ROWS.iter().enumerate().find_map(|(row, first)| {
            note.checked_sub(*first)
                .filter(|col| usize::from(*col) < ROW)
                .map(|col| row * ROW + usize::from(col))
        }),
        DRUM => DRUM_NOTES.iter().position(|drum| *drum == note),
        _ => None,
    }
}

/// What a queued message is, so a newer one of the same kind replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Daw,
    Feature(u8),
    Light(usize),
}

pub struct LaunchkeyMk4 {
    kind: Kind,
    transport: Box<dyn Transport>,
    /// DAW mode, the modes and the first repaint are queued.
    started: bool,
    outbox: VecDeque<(Slot, Vec<u8>)>,
    pace: Duration,
    /// When the next burst may go out.
    next_send: Instant,
    brightness: u8,
    colors: Vec<[u8; 3]>,
    /// What each key's light was last queued; `None` until it is.
    sent: Vec<Option<Vec<u8>>>,
    held: Vec<bool>,
    /// Each absolute knob's or fader's last value; `None` until one arrives.
    dials: Vec<Option<u8>>,
    parser: Parser,
}

impl LaunchkeyMk4 {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model).unwrap_or(Kind::Small);
        let keys = PADS + kind.buttons().len();
        Self {
            kind,
            transport,
            started: false,
            outbox: VecDeque::new(),
            pace: PACE,
            next_send: Instant::now(),
            brightness: 100,
            colors: vec![BLACK; keys],
            sent: vec![None; keys],
            held: vec![false; keys],
            dials: vec![None; usize::from(kind.dials())],
            parser: Parser::default(),
        }
    }

    fn queue(&mut self, slot: Slot, message: Vec<u8>) {
        self.outbox.retain(|(queued, _)| *queued != slot);
        self.outbox.push_back((slot, message));
    }

    fn queue_feature(&mut self, number: u8, value: u8) {
        self.queue(Slot::Feature(number), vec![FEATURE, number, value]);
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

    /// Queues DAW mode, Ryu's modes and every light, from scratch.
    fn start(&mut self) {
        self.started = true;
        self.outbox.clear();
        self.queue(Slot::Daw, DAW_ON.to_vec());
        self.queue_feature(PAD_MODE, PADS_DAW);
        self.queue_feature(ENCODER_MODE, ENCODERS_PLUGIN);
        if self.kind == Kind::Large {
            self.queue_feature(FADER_MODE, FADERS_VOLUME);
        }
        self.queue_feature(RELATIVE_OUTPUT, 0x7f);
        self.sent.fill(None);
        self.dials.fill(None);
        self.queue_lights();
    }

    fn ensure_started(&mut self) {
        if !self.started {
            self.start();
        }
    }

    /// Queues every light whose message differs from the last one queued.
    fn queue_lights(&mut self) {
        for key in 0..self.sent.len() {
            let Some(message) = self.light_message(key) else {
                continue;
            };
            if self.sent.get(key).and_then(Option::as_ref) == Some(&message) {
                continue;
            }
            if let Some(sent) = self.sent.get_mut(key) {
                *sent = Some(message.clone());
            }
            self.queue(Slot::Light(key), message);
        }
    }

    /// The message that sets key `key`'s light at the current colour and
    /// brightness, if it has one.
    fn light_message(&self, key: usize) -> Option<Vec<u8>> {
        let color = scaled(*self.colors.get(key)?, self.brightness);
        if key < PADS {
            let mut message = SYSEX_HEADER.to_vec();
            message.push(self.kind.sku());
            message.extend_from_slice(&PAD_RGB);
            message.push(pad_note(key)?);
            message.extend(color.map(|channel| channel >> 1));
            message.push(SYSEX_END);
            return Some(message);
        }
        let button = self.kind.buttons().get(key - PADS)?;
        match (button.light, MONO_LED) {
            (Light::None, _) => None,
            (Light::Rgb, _) | (Light::Mono, MonoLed::PaletteWhite) => {
                let level = if button.light == Light::Rgb {
                    palette_index(color)
                } else {
                    mono_level(color)
                };
                Some(vec![CONTROL_CHANGE | CH1, button.cc, level])
            }
            (Light::Mono, MonoLed::Brightness) => {
                Some(vec![CONTROL_CHANGE | CH4, button.cc, mono_level(color)])
            }
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

    fn release_all(&mut self, keys: std::ops::Range<usize>, events: &mut Vec<Input>) {
        for key in keys {
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

    /// A mode or feature report on `B6`.
    fn feature_report(&mut self, number: u8, value: u8, events: &mut Vec<Input>) {
        match number {
            SHIFT_CC => {
                if let Some(at) = self
                    .kind
                    .buttons()
                    .iter()
                    .position(|button| button.cc == SHIFT_CC)
                {
                    self.press(PADS + at, value > 0, events);
                }
            }
            PAD_MODE if value != PADS_DAW => {
                self.release_all(0..PADS, events);
                self.queue_feature(PAD_MODE, PADS_DAW);
                self.sent
                    .iter_mut()
                    .take(PADS)
                    .for_each(|sent| *sent = None);
                self.queue_lights();
            }
            ENCODER_MODE => {
                if ENCODER_CUSTOM.contains(&value) {
                    self.queue_feature(ENCODER_MODE, ENCODERS_PLUGIN);
                }
                self.queue_feature(RELATIVE_OUTPUT, 0x7f);
                self.dials
                    .iter_mut()
                    .take(usize::from(KNOBS))
                    .for_each(|dial| *dial = None);
            }
            FADER_MODE if self.kind == Kind::Large && value != FADERS_VOLUME => {
                self.queue_feature(FADER_MODE, FADERS_VOLUME);
                self.dials
                    .iter_mut()
                    .skip(usize::from(KNOBS))
                    .for_each(|dial| *dial = None);
            }
            _ => {}
        }
    }

    /// A knob or fader on channel 16.
    fn analogue(&mut self, number: u8, value: u8, events: &mut Vec<Input>) {
        if let Some(knob) = number.checked_sub(RELATIVE_KNOB).filter(|at| *at < KNOBS) {
            let steps = i16::from(value) - RELATIVE_CENTRE;
            if steps != 0 {
                events.push(Input::DialTurn {
                    index: knob,
                    delta: i8::try_from(steps.clamp(-127, 127)).unwrap_or(0),
                });
            }
        } else if let Some(knob) = number.checked_sub(ABSOLUTE_KNOB).filter(|at| *at < KNOBS) {
            self.turn_absolute(knob, value, events);
        } else if self.kind == Kind::Large {
            if let Some(fader) = number.checked_sub(FIRST_FADER).filter(|at| *at < FADERS) {
                self.turn_absolute(KNOBS + fader, value, events);
            }
        }
    }

    /// Handles one message from the keyboard.
    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        let kind = status & 0xf0;
        let channel = status & 0x0f;
        match (kind, channel) {
            (NOTE_ON, CH16) if number == DAW_NOTE => {
                if value == 0 {
                    // DAW mode lost: let go of everything and start over.
                    self.release_all(0..self.held.len(), events);
                    self.start();
                }
            }
            (NOTE_ON | NOTE_OFF, _) => {
                if let Some(pad) = pad_of_note(channel, number) {
                    self.press(pad, kind == NOTE_ON && value > 0, events);
                }
            }
            (CONTROL_CHANGE, CH7) => self.feature_report(number, value, events),
            (CONTROL_CHANGE, CH1 | CH16) => {
                let found = self
                    .kind
                    .buttons()
                    .iter()
                    .position(|button| button.cc == number && button.cc != SHIFT_CC);
                if let Some(at) = found {
                    self.press(PADS + at, value > 0, events);
                } else if channel == CH16 {
                    self.analogue(number, value, events);
                }
            }
            _ => {}
        }
    }
}

impl Driver for LaunchkeyMk4 {
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

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// Queues each light that changed and writes what the pace allows.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started();
        self.queue_lights();
        self.pump()
    }

    /// No volatile brightness command: the colours are scaled and every
    /// light that changes is sent again.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }
}

impl Drop for LaunchkeyMk4 {
    /// Turns DAW mode off, which hands the keyboard back to its standalone
    /// modes and lights; one direct write, never a wait.
    fn drop(&mut self) {
        if self.started {
            self.outbox.clear();
            let _ = self.transport.write(&DAW_OFF);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify, identify_midi, DialLights, ModelView, Protocol, MODELS};

    const IDS: [&str; 6] = [MINI_25_ID, MINI_37_ID, K25_ID, K37_ID, K49_ID, K61_ID];

    fn open(id: &str) -> (LaunchkeyMk4, Recorder) {
        let recorder = Recorder::default();
        let deck = LaunchkeyMk4::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        assert!(deck.pace.is_zero());
        (deck, recorder)
    }

    /// A deck past its start-up.
    fn started(id: &str) -> (LaunchkeyMk4, Recorder) {
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

    fn feed(deck: &mut LaunchkeyMk4, recorder: &Recorder, messages: &[&[u8]]) -> Vec<Input> {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
        deck.poll(0).unwrap()
    }

    fn pad_bytes(sku: u8, note: u8, rgb: [u8; 3]) -> Vec<u8> {
        vec![
            0xf0, 0x00, 0x20, 0x29, 0x02, sku, 0x01, 0x43, note, rgb[0], rgb[1], rgb[2], 0xf7,
        ]
    }

    #[test]
    fn start_up_enters_daw_mode_picks_the_modes_and_paints() {
        let (mut deck, recorder) = open(K25_ID);
        // Nothing is written when the deck is made.
        assert!(recorder.writes().is_empty());
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(15, Some(&lit([0, 0, 255]))).unwrap();
        deck.set_face(18, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(31, Some(&lit([255, 255, 255]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[..4],
            [
                vec![0x9f, 0x0c, 0x7f],
                vec![0xb6, 0x1d, 0x02],
                vec![0xb6, 0x1e, 0x02],
                vec![0xb6, 0x45, 0x7f],
            ]
        );
        // 16 pads and 16 lit buttons (Shift has none).
        assert_eq!(writes.len(), 4 + 16 + 16);
        assert_eq!(writes[4], pad_bytes(0x14, 0x60, [0x7f, 0, 0]));
        assert_eq!(writes[5], pad_bytes(0x14, 0x61, [0, 0, 0]));
        assert_eq!(writes[4 + 8], pad_bytes(0x14, 0x70, [0, 0, 0]));
        assert_eq!(writes[4 + 15], pad_bytes(0x14, 0x77, [0, 0, 0x7f]));
        // Pad Up and Down are single-colour, on channel 4.
        assert_eq!(writes[20], vec![0xb3, 0x6a, 0]);
        // Scene Launch takes a palette index on channel 1 (green is 21).
        assert_eq!(writes[22], vec![0xb0, 0x68, 21]);
        assert_eq!(writes[23], vec![0xb0, 0x69, 0]);
        // Key 31 is Play, full white.
        assert!(writes.contains(&vec![0xb3, 0x73, 127]));
        assert!(!writes.iter().any(|write| write.get(1) == Some(&0x3f)));
        // Nothing changed: nothing sent; one change, one message.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 36);
        deck.set_face(1, Some(&lit([255, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[36..],
            [pad_bytes(0x14, 0x61, [0x7f, 0x7f, 0])]
        );
    }

    #[test]
    fn the_49_and_61_pick_the_volume_fader_mode_and_light_the_fader_buttons() {
        let (mut deck, recorder) = open(K61_ID);
        deck.set_face(33, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(41, Some(&lit([255, 255, 255]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[..5],
            [
                vec![0x9f, 0x0c, 0x7f],
                vec![0xb6, 0x1d, 0x02],
                vec![0xb6, 0x1e, 0x02],
                vec![0xb6, 0x1f, 0x01],
                vec![0xb6, 0x45, 0x7f],
            ]
        );
        assert!(writes.contains(&vec![0xb0, 0x25, 5]));
        assert!(writes.contains(&vec![0xb0, 0x2d, 3]));
        assert_eq!(writes.len(), 5 + 16 + 16 + 9);
    }

    #[test]
    fn the_mini_uses_its_own_sku_and_nine_buttons() {
        let (mut deck, recorder) = open(MINI_37_ID);
        deck.set_face(3, Some(&lit([254, 2, 128]))).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 4 + 16 + 8);
        assert_eq!(writes[4 + 3], pad_bytes(0x13, 0x63, [127, 1, 64]));
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xb0, 0x6a, 0x7f],
                &[0xb0, 0x6b, 0x7f],
                &[0xb0, 0x34, 0x7f],
                &[0xb6, 0x3f, 0x7f],
                &[0xb6, 0x3f, 0x00],
                &[0xb0, 0x75, 0x7f],
                // Not on the Mini.
                &[0xb0, 0x67, 0x7f],
                &[0xb0, 0x74, 0x7f],
                &[0xbf, 0x05, 0x10],
                &[0xbf, 0x05, 0x20],
            ],
        );
        assert_eq!(
            events,
            vec![
                key(16, true),
                key(17, true),
                key(21, true),
                key(22, true),
                key(22, false),
                key(24, true),
            ]
        );
    }

    #[test]
    fn pads_buttons_and_knobs_map_by_note_controller_and_channel() {
        let (mut deck, recorder) = started(K37_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x90, 0x60, 0x7f],
                &[0x90, 0x60, 0x00],
                &[0x90, 0x77, 0x40],
                &[0x80, 0x77, 0x00],
                // Aftertouch and touch are ignored.
                &[0xa0, 0x70, 0x30],
                &[0xbe, 0x55, 0x7f],
                // Scene Launch on channel 1, then Stop as standalone sends it.
                &[0xb0, 0x68, 0x7f],
                &[0xb0, 0x68, 0x00],
                &[0xbf, 0x74, 0x7f],
                // Shift is only `B6 3F`.
                &[0xb0, 0x3f, 0x7f],
                &[0xb6, 0x3f, 0x7f],
                // Knob 1 one step clockwise, knob 8 three back.
                &[0xbf, 0x55, 0x41],
                &[0xbf, 0x5c, 0x3d],
                &[0xbf, 0x55, 0x40],
                // The absolute form: seeds, then turns.
                &[0xbf, 0x16, 0x40],
                &[0xbf, 0x16, 0x46],
                // Faders are not on a 37.
                &[0xbf, 0x05, 0x10],
                &[0xbf, 0x05, 0x20],
                // Knobs on channel 1 are not Ryu's.
                &[0xb0, 0x55, 0x41],
            ],
        );
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(0, false),
                key(15, true),
                key(15, false),
                key(18, true),
                key(18, false),
                key(29, true),
                key(22, true),
                turn(0, 1),
                turn(7, -3),
                turn(1, 6),
            ]
        );
    }

    #[test]
    fn drum_notes_press_the_same_pads() {
        let (mut deck, recorder) = started(K25_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x99, 0x28, 100],
                &[0x89, 0x28, 0],
                &[0x99, 0x2f, 100],
                &[0x99, 0x24, 100],
                &[0x99, 0x60, 100],
            ],
        );
        assert_eq!(
            events,
            vec![key(0, true), key(0, false), key(15, true), key(8, true)]
        );
    }

    #[test]
    fn faders_are_dials_9_to_17_on_the_49_and_61() {
        let (mut deck, recorder) = started(K49_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xbf, 0x05, 0],
                &[0xbf, 0x05, 127],
                &[0xbf, 0x0d, 100],
                &[0xbf, 0x0d, 99],
                &[0xb0, 0x25, 0x7f],
                &[0xb0, 0x2d, 0x7f],
            ],
        );
        assert_eq!(
            events,
            vec![turn(8, 127), turn(16, -1), key(33, true), key(41, true)]
        );
    }

    #[test]
    fn mode_changes_on_the_keyboard_are_undone() {
        let (mut deck, recorder) = started(K61_ID);
        deck.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x90, 0x60, 0x7f],
                // The user picks the drum layout: the held pad is let go.
                &[0xb6, 0x1d, 0x01],
                &[0xb6, 0x1f, 0x06],
                // Ryu's own choices reported back change nothing.
                &[0xb6, 0x1d, 0x02],
                &[0xb6, 0x1f, 0x01],
            ],
        );
        assert_eq!(events, vec![key(0, true), key(0, false)]);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes[0], vec![0xb6, 0x1d, 0x02]);
        // Then the pads are repainted, and only the pads.
        assert_eq!(writes[1 + 3], pad_bytes(0x14, 0x63, [0x7f, 0, 0]));
        assert_eq!(writes[17], vec![0xb6, 0x1f, 0x01]);
        assert_eq!(writes.len(), 1 + 16 + 1);
    }

    #[test]
    fn encoder_mode_reports_ask_for_relative_output_again() {
        let (mut deck, recorder) = started(K25_ID);
        let before = recorder.writes().len();
        feed(&mut deck, &recorder, &[&[0xb6, 0x1e, 0x04]]);
        assert_eq!(recorder.writes()[before..], [vec![0xb6, 0x45, 0x7f]]);
        // A Custom Mode talks on the other port: Plugin is picked back.
        feed(&mut deck, &recorder, &[&[0xb6, 0x1e, 0x07]]);
        assert_eq!(
            recorder.writes()[before + 1..],
            [vec![0xb6, 0x1e, 0x02], vec![0xb6, 0x45, 0x7f]]
        );
        // Other feature replies are ignored.
        feed(
            &mut deck,
            &recorder,
            &[&[0xb6, 0x45, 0x7f], &[0xb6, 0x6f, 0x40]],
        );
        assert_eq!(recorder.writes().len(), before + 3);
    }

    #[test]
    fn losing_daw_mode_starts_over() {
        let (mut deck, recorder) = started(MINI_25_ID);
        let before = recorder.writes().len();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x90, 0x61, 0x7f],
                &[0x9f, 0x0c, 0x7f],
                &[0x9f, 0x0c, 0x00],
            ],
        );
        assert_eq!(events, vec![key(1, true), key(1, false)]);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes[0], vec![0x9f, 0x0c, 0x7f]);
        assert_eq!(writes.len(), 4 + 16 + 8);
    }

    #[test]
    fn brightness_scales_colours_and_drop_leaves_daw_mode() {
        let (mut deck, recorder) = started(K25_ID);
        deck.set_face(0, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(18, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(31, Some(&lit([0, 0, 255]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_brightness(40).unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [
                pad_bytes(0x14, 0x60, [51, 51, 51]),
                // White at 40% is the palette's 757575.
                vec![0xb0, 0x68, 118],
                vec![0xb3, 0x73, 51],
            ]
        );
        drop(deck);
        assert_eq!(recorder.writes().last(), Some(&vec![0x9f, 0x0c, 0x00]));
        // A deck that never started sends nothing.
        let (deck, recorder) = open(K25_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn writes_are_paced_by_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open(K49_ID);
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
        assert_eq!(writes.last(), Some(&vec![0x9f, 0x0c, 0x00]));
    }

    #[test]
    fn short_or_odd_input_is_ignored() {
        let (mut deck, recorder) = started(K61_ID);
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x90],
                &[0xf8],
                &[0xf0, 0x00, 0x20, 0x29, 0x02, 0x14, 0x09, 0x7f, 0xf7],
                &[0xe0, 0x00, 0x40],
                &[0xbf, 0x7f, 0x7f],
                &[0xb6],
                &[],
            ],
        );
        assert!(events.is_empty());
        // Running status across reads.
        let events = feed(&mut deck, &recorder, &[&[0x90], &[0x61, 0x7f, 0x62, 0x7f]]);
        assert_eq!(events, vec![key(1, true), key(2, true)]);
    }

    #[test]
    fn only_daw_ports_match_and_the_size_picks_the_model() {
        let cases: [(&str, bool, Option<&str>); 22] = [
            // macOS: the input is `DAW Out`, the output `DAW In`.
            ("Launchkey MK4 25 DAW Out", false, Some(K25_ID)),
            ("Launchkey MK4 25 DAW In", false, Some(K25_ID)),
            ("Launchkey MK4 61 DAW Out", false, Some(K61_ID)),
            ("Launchkey Mini MK4 37 DAW In", false, Some(MINI_37_ID)),
            ("Launchkey MK4 25 MIDI Out", false, None),
            ("Launchkey Mini MK4 25 MIDI In", false, None),
            // Windows, whole and cut to 31 characters.
            ("MIDIIN2 (Launchkey MK4 37 MIDI)", true, Some(K37_ID)),
            ("MIDIOUT2 (Launchkey MK4 61 MIDI", true, Some(K61_ID)),
            ("MIDIIN2 (Launchkey Mini MK4 25 ", true, Some(MINI_25_ID)),
            ("MIDIOUT2 (Launchkey Mini MK4 37", true, Some(MINI_37_ID)),
            ("Launchkey MK4 49 MIDI", true, None),
            ("Launchkey Mini MK4 25 MIDI", true, None),
            // Linux: ALSA port 1 or `MIDI 2`, never port 0 or `MIDI 1`.
            (
                "Launchkey MK4 49:Launchkey MK4 49 MIDI 2 24:1",
                false,
                Some(K49_ID),
            ),
            ("Launchkey MK4 49:Launchkey MK4 49 MIDI 1 24:0", false, None),
            (
                "Launchkey MK4 25:Launchkey MK4 25 Launchkey #2 28:1",
                false,
                Some(K25_ID),
            ),
            (
                "Launchkey MK4 25:Launchkey MK4 25 Launchkey 28:0",
                false,
                None,
            ),
            (
                "Launchkey Mini MK4 37:Launchkey Mini MK4 37 MIDI 2 24:1",
                false,
                Some(MINI_37_ID),
            ),
            // The 88 is not driven.
            ("Launchkey MK4 88 DAW Out", false, None),
            ("MIDIIN2 (Launchkey MK4 88 MIDI)", true, None),
            // The MK3s keep their own models.
            (
                "Launchkey Mini MK3:Launchkey Mini MK3 MIDI 2 24:1",
                false,
                Some(super::super::launchkey::MINI_ID),
            ),
            (
                "Launchkey MK3 49 LKMK3 DAW Out",
                false,
                Some(super::super::launchkey::K49_ID),
            ),
            (
                "MIDIIN2 (LKMK3 MIDI)",
                true,
                Some(super::super::launchkey::K25_ID),
            ),
        ];
        for (raw, windows, expected) in cases {
            let found = identify_midi(&normalise(raw), windows).map(|model| model.id);
            assert_eq!(found, expected, "{raw}");
        }
        // The MK3 matcher never takes an MK4 name, nor the MK4 one an MK3's.
        for raw in [
            "Launchkey MK4 25 DAW Out",
            "MIDIIN2 (Launchkey MK4 61 MIDI)",
            "MIDIIN2 (Launchkey Mini MK4 25 ",
        ] {
            let name = normalise(raw);
            for model in MODELS
                .iter()
                .filter(|model| model.protocol == Protocol::LaunchkeyMk3Midi)
            {
                assert!(
                    !super::super::launchkey::port_matches(model, &name),
                    "{raw}"
                );
            }
        }
        for raw in ["Launchkey MK3 25 LKMK3 DAW Out", "MIDIIN2 (LKMK3 MIDI)"] {
            assert!(model_of_port(&normalise(raw)).is_none(), "{raw}");
        }
    }

    #[test]
    fn the_rows_describe_the_keys_the_driver_reads() {
        for id in IDS {
            let model = by_id(id).unwrap();
            let kind = Kind::of(model).unwrap();
            assert_eq!(model.protocol, Protocol::LaunchkeyMk4Midi, "{id}");
            assert!(model.protocol.is_midi() && model.protocol.has_brightness());
            assert!(model.midi_port().is_none(), "{id}");
            assert_eq!(usize::from(model.keys), PADS + kind.buttons().len(), "{id}");
            assert_eq!(model.dials, kind.dials(), "{id}");
            // Found by port name only: the USB id never identifies it.
            for product in model.product_ids {
                assert!(identify(model.vendor_id, *product, 0).is_none(), "{id}");
            }
            let view = ModelView::from(model);
            assert_eq!(view.dial_lights, DialLights::None);
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
            // Every key but Shift (key 22) is lit.
            let lit = view.lit_keys.unwrap();
            assert_eq!(lit.len(), usize::from(model.keys) - 1, "{id}");
            assert!(!lit.contains(&22), "{id}");
        }
        assert_eq!(
            MODELS
                .iter()
                .filter(|model| model.protocol == Protocol::LaunchkeyMk4Midi)
                .count(),
            6
        );
    }
}
