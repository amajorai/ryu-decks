//! Novation's Launch Control XL (MK1 and MK2) and Launch Control (MK1), over
//! class-compliant USB MIDI ([`super::midi`]).
//!
//! Each lists one MIDI port pair, found by name ([`port_matches`]): a name
//! with `Launch Control XL` in it is the XL, and one with `Launch Control`
//! alone is the Launch Control (an XL's name contains both, so the XL is
//! tried first). The Launch Control XL 3 and Launch Control 3 name their
//! ports otherwise (`LCXL3 n` / `LC3 n`) and are never matched here; they
//! have their own driver, [`super::launch_control3`].
//!
//! Both speak plain MIDI on the channel of their active template: user
//! templates 1 to 8 on channels 1 to 8 (`0` to `7` on the wire), factory
//! templates 1 to 8 on channels 9 to 16 (`8` to `15`). Ryu selects factory
//! template 1 on open (`F0 00 20 29 02 <dev> 77 08 F7`, `dev` 11 on the XL and
//! 0A on the Launch Control), resets it (`B8 00 00`) and paints every light.
//! The device echoes the selection and reports the user switching templates
//! the same way; on a switch to another factory template Ryu repaints there,
//! and while a user template is active (it may be remapped) Ryu ignores input
//! and sends nothing.
//!
//! In, on the XL: the three knob rows (`CC 13-20`, `29-36`, `49-56`) and the
//! faders (`CC 77-84`) are absolute, turned into dial steps of one MIDI unit
//! from the first value seen; Track Focus (`notes 41-44, 57-60`), Track
//! Control (`73-76, 89-92`) and Device, Mute, Solo and Record Arm (`105-108`)
//! press with a Note On and release with a Note Off or zero velocity; Up,
//! Down, Left and Right are `CC 104-107`, 127 then 0. On the Launch Control:
//! the pads are notes 9-12 and 25-28, the knobs `CC 21-28` and `41-48`, the
//! arrows `CC 114-117`; a pad or arrow held across a template switch is let go
//! by a bare `80 00 00` or `B0 00 00`, which releases every held pad or arrow.
//!
//! Out, a bi-colour LED takes a colour byte `(green << 4) | 0C | red`, each
//! colour at levels 0 to 3: on the track buttons and pads (by their own note)
//! and the XL's 24 knob LEDs (note `13 + row + 16 * column`). The XL's arrows
//! (red) and Device to Record Arm (yellow) are on or off (127 or 0); the
//! Launch Control's arrows take `0F` or `0C`. Only changed LEDs are sent.
//! There is no brightness command: Ryu scales the levels. When Ryu lets go it
//! resets every channel (`Bn 00 00`), which turns every LED off.
//!
//! Facts per launch-control-xl (lazurite, MIT), the launchy crate (MIT),
//! @mohayonao/launch-control (MIT), DrivenByMoss (LGPL-3.0) and Ableton
//! Live's remote scripts; no code was copied.

use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const XL_ID: &str = "novation-launch-control-xl";
pub const LC_ID: &str = "novation-launch-control";

pub const XL_KEYS: usize = 24;
pub const XL_DIALS: usize = 32;
/// The XL's knobs (dials 0 to 23) each have a light; its faders do not.
pub const XL_DIAL_LIGHTS: usize = 24;
pub const LC_KEYS: usize = 12;
pub const LC_DIALS: usize = 16;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const SYSEX_START: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
/// Novation's manufacturer id and the deck's family byte, before its device byte.
const SYSEX_HEADER: [u8; 5] = [SYSEX_START, 0x00, 0x20, 0x29, 0x02];
const TEMPLATE_COMMAND: u8 = 0x77;
const XL_DEVICE: u8 = 0x11;
const LC_DEVICE: u8 = 0x0a;
/// Factory template 1, on MIDI channel 9 (`8` on the wire).
const FIRST_FACTORY: u8 = 8;
const CHANNELS: u8 = 16;
/// Steady LED (Copy and Clear both set).
const STEADY: u8 = 0x0c;
/// A light that is only on or off is on for any colour brighter than this.
const ON_OFF_THRESHOLD: u8 = 16;
const BLACK: [u8; 3] = [0, 0, 0];
/// The longest SysEx kept. The template message is 9 bytes; a universal
/// identity reply (the APC40 MKII's is 35) must also fit.
const SYSEX_MAX: usize = 128;
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Xl,
    Lc,
}

impl Kind {
    const fn of(protocol: Protocol) -> Option<Self> {
        match protocol {
            Protocol::NovationLaunchControlXl => Some(Self::Xl),
            Protocol::NovationLaunchControl => Some(Self::Lc),
            _ => None,
        }
    }

    const fn device(self) -> u8 {
        match self {
            Self::Xl => XL_DEVICE,
            Self::Lc => LC_DEVICE,
        }
    }

    const fn keys(self) -> &'static [KeyLight] {
        match self {
            Self::Xl => &XL_KEY_LIGHTS,
            Self::Lc => &LC_KEY_LIGHTS,
        }
    }

    const fn dials(self) -> usize {
        match self {
            Self::Xl => XL_DIALS,
            Self::Lc => LC_DIALS,
        }
    }

    const fn dial_lights(self) -> usize {
        match self {
            Self::Xl => XL_DIAL_LIGHTS,
            Self::Lc => 0,
        }
    }

    /// The dial an absolute knob or fader controller turns.
    fn dial_of_cc(self, cc: u8) -> Option<u8> {
        let ranges: &[(u8, u8)] = match self {
            Self::Xl => &[(13, 0), (29, 8), (49, 16), (77, 24)],
            Self::Lc => &[(21, 0), (41, 8)],
        };
        ranges.iter().find_map(|(first, dial)| {
            cc.checked_sub(*first)
                .filter(|at| *at < 8)
                .map(|at| dial + at)
        })
    }
}

/// How one key's LED is set, and the control its presses come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyLight {
    /// `NOTE_ON` or `CONTROL_CHANGE`, without the channel.
    status: u8,
    number: u8,
    /// Mixes red and green at four levels; otherwise only on or off.
    bi_colour: bool,
    on: u8,
    off: u8,
}

const fn bi(note: u8) -> KeyLight {
    KeyLight {
        status: NOTE_ON,
        number: note,
        bi_colour: true,
        on: 0,
        off: 0,
    }
}

const fn switch(status: u8, number: u8, on: u8, off: u8) -> KeyLight {
    KeyLight {
        status,
        number,
        bi_colour: false,
        on,
        off,
    }
}

/// Track Focus 1-8, Track Control 1-8, Up, Down, Left, Right, then Device,
/// Mute, Solo and Record Arm.
const XL_KEY_LIGHTS: [KeyLight; XL_KEYS] = [
    bi(41),
    bi(42),
    bi(43),
    bi(44),
    bi(57),
    bi(58),
    bi(59),
    bi(60),
    bi(73),
    bi(74),
    bi(75),
    bi(76),
    bi(89),
    bi(90),
    bi(91),
    bi(92),
    switch(CONTROL_CHANGE, 104, 127, 0),
    switch(CONTROL_CHANGE, 105, 127, 0),
    switch(CONTROL_CHANGE, 106, 127, 0),
    switch(CONTROL_CHANGE, 107, 127, 0),
    switch(NOTE_ON, 105, 127, 0),
    switch(NOTE_ON, 106, 127, 0),
    switch(NOTE_ON, 107, 127, 0),
    switch(NOTE_ON, 108, 127, 0),
];

/// Pads 1-8, then Up, Down, Left and Right.
const LC_KEY_LIGHTS: [KeyLight; LC_KEYS] = [
    bi(9),
    bi(10),
    bi(11),
    bi(12),
    bi(25),
    bi(26),
    bi(27),
    bi(28),
    switch(CONTROL_CHANGE, 114, 0x0f, STEADY),
    switch(CONTROL_CHANGE, 115, 0x0f, STEADY),
    switch(CONTROL_CHANGE, 116, 0x0f, STEADY),
    switch(CONTROL_CHANGE, 117, 0x0f, STEADY),
];

/// The Launch Control's first arrow key; the keys below it are pads.
const LC_FIRST_ARROW: usize = 8;

const XL_NAMES: [&str; XL_KEYS] = [
    "Track Focus 1",
    "Track Focus 2",
    "Track Focus 3",
    "Track Focus 4",
    "Track Focus 5",
    "Track Focus 6",
    "Track Focus 7",
    "Track Focus 8",
    "Track Control 1",
    "Track Control 2",
    "Track Control 3",
    "Track Control 4",
    "Track Control 5",
    "Track Control 6",
    "Track Control 7",
    "Track Control 8",
    "Up",
    "Down",
    "Left",
    "Right",
    "Device",
    "Mute",
    "Solo",
    "Record Arm",
];

const LC_NAMES: [&str; LC_KEYS] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Up", "Down", "Left",
    "Right",
];

/// The XL's keys as (row, column) in its 4 x 10 grid: the track buttons in
/// rows 3 and 4, the arrows top right, the four side buttons under them.
const XL_PLACES: [(u8, u8); XL_KEYS] = [
    (2, 0),
    (2, 1),
    (2, 2),
    (2, 3),
    (2, 4),
    (2, 5),
    (2, 6),
    (2, 7),
    (3, 0),
    (3, 1),
    (3, 2),
    (3, 3),
    (3, 4),
    (3, 5),
    (3, 6),
    (3, 7),
    (0, 8),
    (0, 9),
    (1, 8),
    (1, 9),
    (2, 8),
    (2, 9),
    (3, 8),
    (3, 9),
];

/// The Launch Control's keys in its 2 x 10 grid: the pads along the bottom,
/// the arrows at the right.
const LC_PLACES: [(u8, u8); LC_KEYS] = [
    (1, 0),
    (1, 1),
    (1, 2),
    (1, 3),
    (1, 4),
    (1, 5),
    (1, 6),
    (1, 7),
    (0, 8),
    (0, 9),
    (1, 8),
    (1, 9),
];

/// Which Launch Control a port's name says it is, if any.
fn kind_of_port(base: &str) -> Option<Kind> {
    let lower = base.to_ascii_lowercase();
    let newer = |rest: &str| {
        let rest = rest.trim_start();
        rest.starts_with('3') || rest.starts_with("mk3")
    };
    if let Some(at) = lower.find("launch control xl") {
        let rest = lower.get(at + "launch control xl".len()..).unwrap_or("");
        return (!newer(rest)).then_some(Kind::Xl);
    }
    let at = lower.find("launch control")?;
    let rest = lower.get(at + "launch control".len()..).unwrap_or("");
    let next = rest.chars().next();
    // "Launch Controller" or a newer model's name is not this one.
    if next.is_some_and(char::is_alphanumeric) || newer(rest) {
        return None;
    }
    Some(Kind::Lc)
}

/// Whether a MIDI port, by its normalised name, is this model's. Both decks
/// have one MIDI interface, so a Windows `MIDIIN2 (...)` name is not theirs.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    let Some(kind) = Kind::of(model.protocol) else {
        return false;
    };
    name.interface.unwrap_or(1) == 1 && kind_of_port(&name.base) == Some(kind)
}

/// The legend on each key, on a Launch Control.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    Some(match Kind::of(model.protocol)? {
        Kind::Xl => XL_NAMES.to_vec(),
        Kind::Lc => LC_NAMES.to_vec(),
    })
}

/// Each key's cell in the model's grid, on a Launch Control.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    let places: &[(u8, u8)] = match Kind::of(model.protocol)? {
        Kind::Xl => &XL_PLACES,
        Kind::Lc => &LC_PLACES,
    };
    let cols = u16::from(model.cols);
    Some(
        places
            .iter()
            .map(|(row, col)| u16::from(*row) * cols + u16::from(*col))
            .collect(),
    )
}

/// The XL note that sets a knob's LED: `13 + row + 16 * column`.
fn dial_light_note(dial: usize) -> u8 {
    let row = (dial / 8) as u8;
    let col = (dial % 8) as u8;
    13 + row + 16 * col
}

const fn is_on(color: [u8; 3]) -> bool {
    color[0] > ON_OFF_THRESHOLD || color[1] > ON_OFF_THRESHOLD || color[2] > ON_OFF_THRESHOLD
}

/// A colour as red and green levels (0 to 3), scaled by `percent`. A lit
/// colour with neither (blue) shows as low amber rather than off.
fn levels(color: [u8; 3], percent: u8) -> (u8, u8) {
    let level = |channel: u8| ((u16::from(channel) * 3 + 127) / 255) as u8;
    let (red, green) = match (level(color[0]), level(color[1])) {
        (0, 0) if is_on(color) => (1, 1),
        pair => pair,
    };
    let scale = |value: u8| {
        if value == 0 || percent == 0 {
            0
        } else {
            (((u16::from(value) * u16::from(percent) + 50) / 100) as u8).clamp(1, 3)
        }
    };
    (scale(red), scale(green))
}

/// The colour byte for a bi-colour LED.
fn color_byte(color: [u8; 3], percent: u8) -> u8 {
    let (red, green) = levels(color, percent);
    (green << 4) | STEADY | red
}

/// One whole message from the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Message {
    /// A note or controller message: status, number, value.
    Channel([u8; 3]),
    SysEx(Vec<u8>),
}

/// Splits whatever bytes arrive into messages: running status, real-time
/// bytes anywhere, and SysEx across reads.
#[derive(Debug, Default)]
pub(super) struct Parser {
    status: Option<u8>,
    data: Vec<u8>,
    sysex: Option<Vec<u8>>,
}

impl Parser {
    pub(super) fn feed(&mut self, bytes: &[u8], out: &mut Vec<Message>) {
        for &byte in bytes {
            match byte {
                0xf8..=0xff => {}
                SYSEX_START => {
                    self.status = None;
                    self.data.clear();
                    self.sysex = Some(vec![SYSEX_START]);
                }
                SYSEX_END => {
                    if let Some(mut sysex) = self.sysex.take() {
                        sysex.push(SYSEX_END);
                        out.push(Message::SysEx(sysex));
                    }
                }
                0x80..=0xef => {
                    self.sysex = None;
                    self.status = Some(byte);
                    self.data.clear();
                }
                0xf1..=0xf6 => {
                    self.sysex = None;
                    self.status = None;
                    self.data.clear();
                }
                _ => self.data_byte(byte, out),
            }
        }
    }

    fn data_byte(&mut self, byte: u8, out: &mut Vec<Message>) {
        if let Some(sysex) = &mut self.sysex {
            if sysex.len() < SYSEX_MAX {
                sysex.push(byte);
            } else {
                self.sysex = None;
            }
            return;
        }
        let Some(status) = self.status else {
            return;
        };
        self.data.push(byte);
        let needed = if matches!(status & 0xf0, 0xc0 | 0xd0) {
            1
        } else {
            2
        };
        if self.data.len() == needed {
            let second = self.data.get(1).copied().unwrap_or(0);
            out.push(Message::Channel([status, self.data[0], second]));
            self.data.clear();
        }
    }
}

pub struct LaunchControl {
    kind: Kind,
    transport: Box<dyn Transport>,
    /// Factory template 1 selected and reset, as far as Ryu knows.
    started: bool,
    /// The active template (its MIDI channel, 0 to 15).
    template: u8,
    colors: Vec<[u8; 3]>,
    dial_colors: Vec<[u8; 3]>,
    /// What each key LED, then each dial LED, was last sent; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    /// Each dial's last absolute value; `None` until one arrives.
    dials: Vec<Option<u8>>,
    brightness: u8,
    parser: Parser,
}

impl LaunchControl {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model.protocol).unwrap_or(Kind::Xl);
        let keys = kind.keys().len();
        Self {
            kind,
            transport,
            started: false,
            template: FIRST_FACTORY,
            colors: vec![BLACK; keys],
            dial_colors: vec![BLACK; kind.dial_lights()],
            sent: vec![None; keys + kind.dial_lights()],
            held: vec![false; keys],
            dials: vec![None; kind.dials()],
            brightness: 100,
            parser: Parser::default(),
        }
    }

    fn template_message(&self, template: u8) -> [u8; 9] {
        [
            SYSEX_START,
            0x00,
            0x20,
            0x29,
            0x02,
            self.kind.device(),
            TEMPLATE_COMMAND,
            template,
            SYSEX_END,
        ]
    }

    /// Selects factory template 1, resets it (all LEDs off) and forgets what
    /// the LEDs show.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if !self.started {
            self.template = FIRST_FACTORY;
            let select = self.template_message(FIRST_FACTORY);
            self.transport.write(&select)?;
            self.transport
                .write(&[CONTROL_CHANGE | FIRST_FACTORY, 0x00, 0x00])?;
            self.forget();
            self.started = true;
        }
        Ok(())
    }

    /// Forgets the LEDs and the dial positions, so both start afresh.
    fn forget(&mut self) {
        self.sent.fill(None);
        self.dials.fill(None);
    }

    /// Input is read, and LEDs sent, only on a factory template.
    const fn on_factory_template(&self) -> bool {
        self.template >= FIRST_FACTORY
    }

    fn release(&mut self, keys: std::ops::Range<usize>, events: &mut Vec<Input>) {
        for index in keys {
            if let Some(held) = self.held.get_mut(index) {
                if *held {
                    *held = false;
                    if let Ok(index) = u8::try_from(index) {
                        events.push(Input::Key {
                            index,
                            pressed: false,
                        });
                    }
                }
            }
        }
    }

    /// A template switch: held keys are let go and, on another template,
    /// the dials reseed and the LEDs are repainted. Returns whether to repaint.
    fn template_changed(&mut self, template: u8, events: &mut Vec<Input>) -> bool {
        if template == self.template || template >= CHANNELS {
            return false;
        }
        self.release(0..self.held.len(), events);
        self.template = template;
        self.forget();
        self.on_factory_template()
    }

    fn key_of(&self, status: u8, number: u8) -> Option<usize> {
        let status = if status == NOTE_OFF { NOTE_ON } else { status };
        self.kind
            .keys()
            .iter()
            .position(|light| light.status == status && light.number == number)
    }

    /// Handles one message; returns whether the LEDs must be repainted.
    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> bool {
        let [status, number, value] = match message {
            Message::SysEx(bytes) => {
                let Some(template) = self.template_of(bytes) else {
                    return false;
                };
                return self.template_changed(template, events);
            }
            Message::Channel(bytes) => *bytes,
        };
        let kind = status & 0xf0;
        let channel = status & 0x0f;
        // A Launch Control lets go of a pad or arrow held across a template
        // switch with a bare message on channel 1, naming no control.
        if self.kind == Kind::Lc && number == 0 && value == 0 {
            match kind {
                NOTE_OFF => self.release(0..LC_FIRST_ARROW, events),
                CONTROL_CHANGE => self.release(LC_FIRST_ARROW..LC_KEYS, events),
                _ => {}
            }
            return false;
        }
        if !self.on_factory_template() || channel != self.template {
            return false;
        }
        let pressed = match kind {
            NOTE_ON | CONTROL_CHANGE => value > 0,
            NOTE_OFF => false,
            _ => return false,
        };
        if let Some(key) = self.key_of(kind, number) {
            self.press(key, pressed, events);
        } else if kind == CONTROL_CHANGE {
            if let Some(dial) = self.kind.dial_of_cc(number) {
                self.turn(dial, value, events);
            }
        }
        false
    }

    fn template_of(&self, bytes: &[u8]) -> Option<u8> {
        let header = bytes.get(..SYSEX_HEADER.len())?;
        let rest = bytes.get(SYSEX_HEADER.len()..)?;
        match rest {
            [device, TEMPLATE_COMMAND, template, SYSEX_END]
                if header == SYSEX_HEADER && *device == self.kind.device() =>
            {
                Some(*template)
            }
            _ => None,
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

    /// The message for LED `slot` (keys first, then the XL's knob LEDs) at
    /// the current colours and brightness.
    fn led_message(&self, slot: usize) -> Option<[u8; 3]> {
        let channel = self.template & 0x0f;
        let keys = self.kind.keys();
        if let Some(light) = keys.get(slot) {
            let color = *self.colors.get(slot)?;
            let value = if light.bi_colour {
                color_byte(color, self.brightness)
            } else if is_on(color) && self.brightness > 0 {
                light.on
            } else {
                light.off
            };
            return Some([light.status | channel, light.number, value]);
        }
        let dial = slot - keys.len();
        let color = *self.dial_colors.get(dial)?;
        Some([
            NOTE_ON | channel,
            dial_light_note(dial),
            color_byte(color, self.brightness),
        ])
    }
}

impl Driver for LaunchControl {
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
        let mut repaint = false;
        for message in &messages {
            repaint |= self.receive(message, &mut events);
        }
        if repaint {
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

    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if let Some(slot) = self.dial_colors.get_mut(usize::from(dial)) {
            *slot = color.unwrap_or(BLACK);
        }
        Ok(())
    }

    /// One message per LED that changed, on the active factory template;
    /// nothing while a user template is active.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        if !self.on_factory_template() {
            return Ok(());
        }
        for slot in 0..self.sent.len() {
            let Some(message) = self.led_message(slot) else {
                continue;
            };
            if self.sent[slot] != Some(message[2]) {
                self.transport.write(&message)?;
                self.sent[slot] = Some(message[2]);
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }
}

impl Drop for LaunchControl {
    /// Resets every template's channel, which turns every LED off.
    fn drop(&mut self) {
        if self.started {
            for channel in 0..CHANNELS {
                let _ = self
                    .transport
                    .write(&[CONTROL_CHANGE | channel, 0x00, 0x00]);
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

    fn open(id: &str) -> (LaunchControl, Recorder) {
        let recorder = Recorder::default();
        let deck = LaunchControl::new(by_id(id).unwrap(), Box::new(recorder.clone()));
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
    fn colours_become_red_and_green_levels() {
        assert_eq!(color_byte([0, 0, 0], 100), 0x0c);
        assert_eq!(color_byte([255, 0, 0], 100), 0x0f);
        assert_eq!(color_byte([85, 0, 0], 100), 0x0d);
        assert_eq!(color_byte([0, 255, 0], 100), 0x3c);
        assert_eq!(color_byte([0, 85, 0], 100), 0x1c);
        assert_eq!(color_byte([255, 255, 0], 100), 0x3f);
        assert_eq!(color_byte([170, 255, 0], 100), 0x3e);
        assert_eq!(color_byte([85, 85, 0], 100), 0x1d);
        // Blue alone glows low amber; near-black stays off.
        assert_eq!(color_byte([0, 0, 255], 100), 0x1d);
        assert_eq!(color_byte([0, 0, 10], 100), 0x0c);
        // Brightness keeps a lit level at least at 1 until it reaches 0.
        assert_eq!(color_byte([255, 85, 0], 50), 0x1e);
        assert_eq!(color_byte([255, 85, 0], 1), 0x1d);
        assert_eq!(color_byte([255, 255, 0], 0), 0x0c);
    }

    #[test]
    fn opening_the_xl_selects_factory_template_one_and_paints_it() {
        let (mut deck, recorder) = open(XL_ID);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(15, Some(&lit([0, 255, 0]))).unwrap();
        deck.set_face(16, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(23, Some(&lit([0, 0, 200]))).unwrap();
        deck.set_dial_light(0, Some([255, 255, 0])).unwrap();
        deck.set_dial_light(9, Some([0, 255, 0])).unwrap();
        deck.set_dial_light(23, Some([255, 0, 0])).unwrap();
        // The faders have no lights.
        deck.set_dial_light(24, Some([255, 0, 0])).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[0],
            vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x11, 0x77, 0x08, 0xf7]
        );
        assert_eq!(writes[1], vec![0xb8, 0x00, 0x00]);
        assert_eq!(writes.len(), 2 + 24 + 24);
        assert_eq!(writes[2], vec![0x98, 41, 0x0f]);
        assert_eq!(writes[3], vec![0x98, 42, 0x0c]);
        assert_eq!(writes[2 + 15], vec![0x98, 92, 0x3c]);
        assert_eq!(writes[2 + 16], vec![0xb8, 104, 127]);
        assert_eq!(writes[2 + 17], vec![0xb8, 105, 0]);
        assert_eq!(writes[2 + 20], vec![0x98, 105, 0]);
        assert_eq!(writes[2 + 23], vec![0x98, 108, 127]);
        // Knob LEDs: top row 13, 29, ...; middle 14, 30, ...; bottom 15, ...
        assert_eq!(writes[26], vec![0x98, 13, 0x3f]);
        assert_eq!(writes[27], vec![0x98, 29, 0x0c]);
        assert_eq!(writes[26 + 9], vec![0x98, 30, 0x3c]);
        assert_eq!(writes[26 + 16], vec![0x98, 15, 0x0c]);
        assert_eq!(writes[26 + 23], vec![0x98, 127, 0x0f]);
        // Nothing changed: nothing sent; one change: one message.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 50);
        deck.set_face(1, Some(&lit([255, 255, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[50..], [vec![0x98, 42, 0x3f]]);
    }

    #[test]
    fn opening_the_launch_control_paints_pads_and_arrows() {
        let (mut deck, recorder) = open(LC_ID);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(7, Some(&lit([255, 255, 0]))).unwrap();
        deck.set_face(8, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[0],
            vec![0xf0, 0x00, 0x20, 0x29, 0x02, 0x0a, 0x77, 0x08, 0xf7]
        );
        assert_eq!(writes[1], vec![0xb8, 0x00, 0x00]);
        assert_eq!(
            writes[2..],
            [
                vec![0x98, 9, 0x0f],
                vec![0x98, 10, 0x0c],
                vec![0x98, 11, 0x0c],
                vec![0x98, 12, 0x0c],
                vec![0x98, 25, 0x0c],
                vec![0x98, 26, 0x0c],
                vec![0x98, 27, 0x0c],
                vec![0x98, 28, 0x3f],
                vec![0xb8, 114, 0x0f],
                vec![0xb8, 115, 0x0c],
                vec![0xb8, 116, 0x0c],
                vec![0xb8, 117, 0x0c],
            ]
        );
    }

    #[test]
    fn xl_buttons_and_dials_map_on_the_template_channel() {
        let (mut deck, recorder) = open(XL_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                // The device echoes the template Ryu selected: no repaint.
                &[0xf0, 0x00, 0x20, 0x29, 0x02, 0x11, 0x77, 0x08, 0xf7],
                &[0x98, 41, 127],
                &[0x88, 41, 0],
                &[0x98, 92, 127],
                &[0x98, 92, 0],
                // Note 105 is Device; CC 105 is Down.
                &[0x98, 105, 127],
                &[0xb8, 105, 127],
                &[0xb8, 105, 0],
                &[0x88, 108, 0],
                &[0x98, 108, 127],
                // Knobs and faders: the first value seeds.
                &[0xb8, 13, 64],
                &[0xb8, 13, 66],
                &[0xb8, 13, 61],
                &[0xb8, 36, 10],
                &[0xb8, 36, 0],
                &[0xb8, 56, 0],
                &[0xb8, 56, 127],
                &[0xb8, 84, 100],
                &[0xb8, 84, 101],
                // Knob LED notes, another channel, CC 0 and unknown numbers.
                &[0x98, 13, 127],
                &[0x99, 41, 127],
                &[0xb0, 104, 127],
                &[0xb8, 0, 0],
                &[0xb8, 108, 127],
                &[0x98, 61, 127],
                // A repeated press, and a short message.
                &[0x98, 108, 127],
                &[0x98, 41],
            ],
        );
        let before = recorder.writes().len();
        let events = deck.poll(0).unwrap();
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(0, false),
                key(15, true),
                key(15, false),
                key(20, true),
                key(17, true),
                key(17, false),
                key(23, true),
                turn(0, 2),
                turn(0, -5),
                turn(15, -10),
                turn(23, 127),
                turn(31, 1),
            ]
        );
        assert_eq!(recorder.writes().len(), before);
    }

    #[test]
    fn a_split_stream_with_running_status_and_realtime_bytes_decodes() {
        let (mut deck, recorder) = open(XL_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x98, 41],
                &[0xf8, 127, 41, 0xfe],
                &[0],
                &[0xb8, 77, 10, 77, 0xfa, 12],
                &[0xf0, 0x00, 0x20, 0x29],
                &[0x02, 0x11, 0x77, 0xf8, 0x09, 0xf7],
            ],
        );
        let before = recorder.writes().len();
        let events = deck.poll(0).unwrap();
        assert_eq!(events, vec![key(0, true), key(0, false), turn(24, 2)]);
        // Factory template 2 (channel 10): every LED repainted there.
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 48);
        assert_eq!(writes[before], vec![0x99, 41, 0x0c]);
        assert!(writes[before..].iter().all(|w| w[0] & 0x0f == 9));
    }

    #[test]
    fn template_switches_release_keys_and_reseed_dials() {
        let (mut deck, recorder) = open(XL_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x98, 57, 127],
                &[0xb8, 49, 40],
                // User template 1: input is ignored and nothing is sent.
                &[0xf0, 0x00, 0x20, 0x29, 0x02, 0x11, 0x77, 0x00, 0xf7],
                &[0x90, 57, 127],
                &[0xb0, 49, 50],
                // The Launch Control's template message is not the XL's.
                &[0xf0, 0x00, 0x20, 0x29, 0x02, 0x0a, 0x77, 0x08, 0xf7],
            ],
        );
        let before = recorder.writes().len();
        assert_eq!(deck.poll(0).unwrap(), vec![key(4, true), key(4, false)]);
        assert_eq!(recorder.writes().len(), before);
        deck.set_face(4, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), before);
        // Back to factory template 1: repainted, and the dial seeds afresh.
        queue(
            &recorder,
            &[
                &[0xf0, 0x00, 0x20, 0x29, 0x02, 0x11, 0x77, 0x08, 0xf7],
                &[0xb8, 49, 100],
                &[0xb8, 49, 98],
            ],
        );
        assert_eq!(deck.poll(0).unwrap(), vec![turn(16, -2)]);
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 48);
        assert!(writes.contains(&vec![0x98, 57, 0x0f]));
    }

    #[test]
    fn launch_control_inputs_and_stale_releases() {
        let (mut deck, recorder) = open(LC_ID);
        deck.flush().unwrap();
        queue(
            &recorder,
            &[
                &[0x98, 9, 127],
                &[0x98, 28, 127],
                &[0x88, 28, 0],
                &[0x98, 25, 127],
                &[0x98, 25, 0],
                &[0xb8, 114, 127],
                &[0xb8, 117, 127],
                &[0xb8, 117, 0],
                &[0xb8, 116, 127],
                &[0xb8, 21, 0],
                &[0xb8, 21, 3],
                &[0xb8, 48, 70],
                &[0xb8, 48, 69],
                // The XL's numbers mean nothing here.
                &[0x98, 41, 127],
                &[0xb8, 13, 1],
                // Stale releases on channel 1: every held pad, then arrow.
                &[0x80, 0x00, 0x00],
                &[0xb0, 0x00, 0x00],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(7, true),
                key(7, false),
                key(4, true),
                key(4, false),
                key(8, true),
                key(11, true),
                key(11, false),
                key(10, true),
                turn(0, 3),
                turn(15, -1),
                key(0, false),
                key(8, false),
                key(10, false),
            ]
        );
    }

    #[test]
    fn brightness_scales_the_levels_and_switches_the_single_colour_lights() {
        let (mut deck, recorder) = open(XL_ID);
        deck.set_face(0, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(16, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_brightness(50).unwrap();
        assert_eq!(recorder.writes()[before..], [vec![0x98, 41, 0x2e]]);
        let before = recorder.writes().len();
        deck.set_brightness(0).unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [vec![0x98, 41, 0x0c], vec![0xb8, 104, 0]]
        );
    }

    #[test]
    fn letting_go_resets_every_channel() {
        let (mut deck, recorder) = open(LC_ID);
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), before + 16);
        for (channel, write) in (0u8..).zip(&writes[before..]) {
            assert_eq!(*write, vec![0xb0 | channel, 0x00, 0x00]);
        }
        let (deck, recorder) = open(XL_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn ports_are_told_apart_by_name() {
        let cases = [
            ("Launch Control XL", Some(XL_ID)),
            ("2- Launch Control XL", Some(XL_ID)),
            (
                "Launch Control XL:Launch Control XL MIDI 1 24:0",
                Some(XL_ID),
            ),
            ("Launch Control XL Launch Contro", Some(XL_ID)),
            ("Launch Control", Some(LC_ID)),
            ("3- Launch Control", Some(LC_ID)),
            ("Launch Control:Launch Control MIDI 1 28:0", Some(LC_ID)),
            ("launch control", Some(LC_ID)),
            // Newer models, a second Windows interface and other names.
            ("LCXL3 1 MIDI", None),
            ("LC3 1 MIDI", None),
            ("Launch Control XL 3", None),
            ("Launch Control 3", None),
            ("MIDIIN2 (Launch Control XL)", None),
            ("Launch Controller", None),
            ("Launchpad", None),
            ("Launchpad Mini", None),
        ];
        for (raw, expected) in cases {
            let name = normalise(raw);
            let found = [XL_ID, LC_ID]
                .into_iter()
                .find(|id| port_matches(by_id(id).unwrap(), &name));
            assert_eq!(found, expected, "{raw}");
            // The catalog agrees (a Launchpad's name is still a Launchpad's).
            let listed = identify_midi(&name, false)
                .map(|model| model.id)
                .filter(|id| [XL_ID, LC_ID].contains(id));
            assert_eq!(listed, expected, "{raw}");
        }
    }

    #[test]
    fn rows_name_and_place_every_key() {
        for (id, keys, dials, dial_lights) in [
            (XL_ID, XL_KEYS, XL_DIALS, DialLights::PerDial),
            (LC_ID, LC_KEYS, LC_DIALS, DialLights::None),
        ] {
            let model = by_id(id).unwrap();
            assert!(model.protocol.is_midi(), "{id}");
            assert_eq!(usize::from(model.keys), keys);
            assert_eq!(usize::from(model.dials), dials);
            let view = ModelView::from(model);
            assert!(view.brightness && view.key_lights, "{id}");
            assert_eq!(view.key_light_colors, "red_green");
            assert_eq!(view.dial_lights, dial_lights);
            // The XL's faders have no light, so they get no colour picker.
            let lit = if id == XL_ID { XL_DIAL_LIGHTS } else { 0 };
            assert_eq!(usize::from(model.dial_light_colors()), lit, "{id}");
            assert_eq!(view.deck_lights, 0);
            let names = view.key_names.unwrap();
            let cells = view.key_cells.unwrap();
            assert_eq!(names.len(), keys);
            assert_eq!(cells.len(), keys);
            let unique: std::collections::HashSet<u16> = cells.iter().copied().collect();
            assert_eq!(unique.len(), keys);
            let grid = u16::from(model.rows) * u16::from(model.cols);
            assert!(cells.iter().all(|cell| *cell < grid));
        }
        // Every key and knob LED has its own message, and every input
        // control maps back to its key.
        for kind in [Kind::Xl, Kind::Lc] {
            let numbers: std::collections::HashSet<(u8, u8)> = kind
                .keys()
                .iter()
                .map(|light| (light.status, light.number))
                .collect();
            assert_eq!(numbers.len(), kind.keys().len());
        }
        let notes: std::collections::HashSet<u8> =
            (0..XL_DIAL_LIGHTS).map(dial_light_note).collect();
        assert_eq!(notes.len(), XL_DIAL_LIGHTS);
        assert_eq!(
            (dial_light_note(7), dial_light_note(8), dial_light_note(23)),
            (125, 14, 127)
        );
    }
}
