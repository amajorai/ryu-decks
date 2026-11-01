//! Behringer's B-Control BCR2000 (32 push encoders and 24 plain encoders)
//! and BCF2000 (8 push encoders and 8 motor faders), over class-compliant USB
//! MIDI ([`super::midi`]).
//!
//! Both list their MIDI ports under the deck's name (`BCR2000` or `BCF2000`),
//! often as several ports (the USB port, then the two DIN outputs), so the
//! shared `<token> MI` rule cannot take them. [`port_matches`] takes a name
//! containing the model's name in any case that is not its second or third
//! port: on Windows the first interface, on ALSA port 0, and never a name
//! ending in `Port 2`, `MIDI 3` and so on. Both report USB product id `00BC`,
//! so the USB id never tells them apart; the port name does.
//!
//! The deck keeps whatever preset the user last chose, so Ryu writes its own
//! into the deck's edit buffer, about a second after the port opens: a BCL
//! text preset, one SysEx per line, `F0 00 20 32 7F 7F 20 <hi> <lo> <ASCII>
//! F7` (Behringer's id, any device, any model, command 20 BCL text, then the
//! line's running number from 0 in two 7-bit halves). The lines (with no
//! indent) are `$rev R1` (`$rev F1` on the BCF), the `$preset` block (a
//! 24-character name, snapshot and request off, 4 encoder groups, `.init`),
//! then:
//!
//! - `$encoder 1`-`32`, the push encoders in their four groups: absolute CC
//!   1-32 on channel 1, 0-127, value shown on the ring;
//! - BCR only, `$encoder 33`-`56`, the three rows of plain encoders: CC
//!   81-104;
//! - `$button 1`-`32`, the encoder pushes: CC 33-64, 127 pressed and 0
//!   released (`toggleoff`, momentary);
//! - `$button 33`-`48`, the two button rows: CC 65-80, the same way;
//! - `$button 49`-`52`, the four buttons at the lower right: CC 105-108 on the
//!   BCR, CC 89-92 on the BCF;
//! - BCF only, `$fader 1`-`8`: CC 81-88, 0-127, with the motors off;
//! - `$end`.
//!
//! Each line goes out [`PACE`] after the one before, from a queue that each
//! poll and flush drains as its turn comes, so the shared poll loop never
//! sleeps for the deck. `.store` is never sent: the user's stored presets are
//! untouched, and picking one on the deck brings it back.
//!
//! In, all Control Change on channel 1: CC 1-32 turn dials 0-7 (CC `c` is dial
//! `(c - 1) % 8`, so the encoder group chosen on the deck does not matter), CC
//! 33-64 press them; CC 65-72 are keys 0-7 (the top button row), 73-80 keys
//! 8-15 and the box keys 16-19 (top left, top right, bottom left, bottom
//! right, an order inferred from Mixxx). BCR: CC 81-104 are dials 8-31, row by
//! row from the top (assumed). BCF: CC 81-88 are the faders, dials 8-15.
//! Everything else, SysEx replies included, is ignored.
//!
//! Encoders send their absolute position, which stops at 0 and 127, so Ryu
//! turns a dial by the difference from the last value and, once the encoder
//! has been still for [`IDLE`], puts it back to 64 (`B0 c 40`); a turn that
//! beat that re-centre counts from the old value. All encoders are set to 64
//! right after the preset. The motor faders are never written: the first
//! value only seeds, and later values step the dial every two units.
//!
//! Out: each of the 20 keys has a single-colour LED that follows its own CC
//! (`.showvalue on`): `B0 <cc> 7F` on, `00` off; any colour under [`DARK`] in
//! every channel is off. The deck clears a momentary button's LED when it is
//! released, so Ryu sends that key's light again after each release. The
//! encoder rings only show values. No brightness, dial lights or keep-alive.
//! When Ryu lets go, every key light that may be on goes dark.
//!
//! None of this is tested on hardware: that the LEDs follow incoming CCs,
//! that the encoders take a re-centre, whether the deck echoes what it is sent
//! and the order of the BCR's lower encoder rows are assumptions.
//!
//! Facts from Ableton's BCR2000 and BCF2000 remote scripts and their
//! Preset.syx files as published in gluon/AbletonLive12_MIDIRemoteScripts
//! (decompiled, no licence; read for facts only), Mixxx's BCR2000 mapping
//! (GPL-2.0-or-later) and linux-usb.org's usb.ids; no code was copied.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const BCR2000_ID: &str = "behringer-bcr2000";
pub const BCF2000_ID: &str = "behringer-bcf2000";

pub const KEYS: u8 = 20;
pub const BCR_DIALS: u8 = 32;
pub const BCF_DIALS: u8 = 16;
/// The four box buttons, after the two button rows.
pub const EXTRA_KEYS: u8 = 4;
/// Two rows of eight buttons, with the 2 x 2 button box at the right.
pub const ROWS: u8 = 2;
pub const COLS: u8 = 10;
const ROW_KEYS: u8 = 16;
const ROW_COLS: u8 = 8;
const PUSH_DIALS: u8 = 8;

const CONTROL_CHANGE: u8 = 0xb0;
const SYSEX_END: u8 = 0xf7;
/// Behringer's id, any device, any model, BCL text.
const BCL_HEAD: [u8; 7] = [0xf0, 0x00, 0x20, 0x32, 0x7f, 0x7f, 0x20];

const FIRST_PUSH_ENCODER_CC: u8 = 1;
const PUSH_ENCODERS: u8 = 32;
const FIRST_PUSH_CC: u8 = 33;
const FIRST_KEY_CC: u8 = 65;
/// The lower encoders (BCR) or faders (BCF).
const FIRST_LOWER_CC: u8 = 81;
const BCR_LOWER_ENCODERS: u8 = 24;
const BCF_FADERS: u8 = 8;
const BCR_BOX_CC: u8 = 105;
const BCF_BOX_CC: u8 = 89;
const BOX_KEYS: u8 = 4;

const CENTRE: u8 = 0x40;
const ON: u8 = 0x7f;
const OFF: u8 = 0x00;

/// How long after the port opens Ryu writes its preset.
#[cfg(not(test))]
const SETTLE: Duration = Duration::from_secs(1);
#[cfg(test)]
const SETTLE: Duration = Duration::ZERO;
/// The gap between two messages; no reference gives one, so it is generous.
#[cfg(not(test))]
const PACE: Duration = Duration::from_millis(4);
#[cfg(test)]
const PACE: Duration = Duration::ZERO;
/// How long an encoder must be still before it is put back to 64.
const IDLE: Duration = Duration::from_millis(200);
/// How long a value Ryu just sent is taken as the deck's echo of it.
const ECHO: Duration = Duration::from_millis(50);
/// Fader units per dial step.
const FADER_STEP: i16 = 2;

/// A colour is dark (its key off) while every channel is under this.
pub const DARK: u8 = 32;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const NAMES: [&str; KEYS as usize] = [
    "Button 1",
    "Button 2",
    "Button 3",
    "Button 4",
    "Button 5",
    "Button 6",
    "Button 7",
    "Button 8",
    "Button 9",
    "Button 10",
    "Button 11",
    "Button 12",
    "Button 13",
    "Button 14",
    "Button 15",
    "Button 16",
    "Box top left",
    "Box top right",
    "Box bottom left",
    "Box bottom right",
];

const fn is_bcontrol(model: &DeviceModel) -> bool {
    matches!(model.protocol, Protocol::BehringerBControlMidi)
}

/// Whether a normalised port name is this model's first (USB) port: its name
/// in any case, on Windows the first interface, on ALSA port 0, and never a
/// name that ends in port 2 or 3.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if !is_bcontrol(model) || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    if name.alsa_port.is_some_and(|port| port != 0) {
        return false;
    }
    let token = if model.id == BCF2000_ID {
        "BCF2000"
    } else {
        "BCR2000"
    };
    let base = name.base.to_ascii_uppercase();
    if !base.contains(token) {
        return false;
    }
    let mut words = base.rsplit(' ');
    let last = words.next().unwrap_or_default();
    let before = words.next().unwrap_or_default();
    !(matches!(last, "2" | "3") && matches!(before, "PORT" | "MIDI"))
}

/// The legend on each key: the two button rows, then the box.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    is_bcontrol(model).then(|| NAMES.to_vec())
}

/// Places on the 2 x 10 grid: the button rows in columns 1-8, the box in
/// columns 9 and 10.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    is_bcontrol(model).then(|| {
        (0..KEYS)
            .map(|key| {
                let (row, col) = if key < ROW_KEYS {
                    (key / ROW_COLS, key % ROW_COLS)
                } else {
                    let place = key - ROW_KEYS;
                    (place / 2, ROW_COLS + place % 2)
                };
                u16::from(row) * u16::from(COLS) + u16::from(col)
            })
            .collect()
    })
}

/// The BCL text of Ryu's preset, line by line.
#[must_use]
pub fn preset_lines(bcf: bool) -> Vec<String> {
    let mut lines = vec![
        if bcf { "$rev F1" } else { "$rev R1" }.to_owned(),
        "$preset".to_owned(),
        format!(".name '{:<24}'", "Ryu"),
        ".snapshot off".to_owned(),
        ".request off".to_owned(),
        ".egroups 4".to_owned(),
        ".fkeys on".to_owned(),
        ".lock off".to_owned(),
        ".init".to_owned(),
    ];
    let encoder = |lines: &mut Vec<String>, n: u8, cc: u8, mode: &str| {
        lines.push(format!("$encoder {n}"));
        lines.push(format!(".easypar CC 1 {cc} 0 127 absolute"));
        lines.push(".showvalue on".to_owned());
        lines.push(format!(".mode {mode}"));
    };
    for n in 1..=PUSH_ENCODERS {
        encoder(&mut lines, n, FIRST_PUSH_ENCODER_CC + n - 1, "12dot");
    }
    if !bcf {
        for n in 0..BCR_LOWER_ENCODERS {
            encoder(
                &mut lines,
                PUSH_ENCODERS + 1 + n,
                FIRST_LOWER_CC + n,
                "1dot",
            );
        }
    }
    let box_cc = if bcf { BCF_BOX_CC } else { BCR_BOX_CC };
    let buttons = (1..=48u8)
        .map(|n| (n, FIRST_PUSH_CC + n - 1))
        .chain((0..BOX_KEYS).map(|b| (49 + b, box_cc + b)));
    for (n, cc) in buttons {
        lines.push(format!("$button {n}"));
        lines.push(format!(".easypar CC 1 {cc} 127 0 toggleoff"));
        lines.push(".showvalue on".to_owned());
    }
    if bcf {
        for n in 0..BCF_FADERS {
            lines.push(format!("$fader {}", n + 1));
            lines.push(format!(
                ".easypar CC 1 {} 0 127 absolute",
                FIRST_LOWER_CC + n
            ));
            lines.push(".showvalue on".to_owned());
            lines.push(".motor off".to_owned());
            lines.push(".override move".to_owned());
            lines.push(".keyoverride off".to_owned());
        }
    }
    lines.push("$end".to_owned());
    lines
}

/// One BCL line as SysEx, `index` its running number.
#[must_use]
pub fn bcl_sysex(index: u16, line: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(BCL_HEAD.len() + 3 + line.len());
    out.extend_from_slice(&BCL_HEAD);
    out.push(u8::try_from((index >> 7) & 0x7f).unwrap_or(0));
    out.push(u8::try_from(index & 0x7f).unwrap_or(0));
    out.extend(line.bytes().filter(u8::is_ascii));
    out.push(SYSEX_END);
    out
}

/// Whether a key's light is on for this colour.
#[must_use]
pub fn is_lit(color: [u8; 3]) -> bool {
    color.iter().any(|channel| *channel >= DARK)
}

/// What a Control Change number is, on this model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    /// An encoder Ryu re-centres, and its dial.
    Encoder(u8),
    /// A BCF motor fader (0-7); its dial is 8 + fader.
    Fader(u8),
    /// An encoder push, and its dial.
    Push(u8),
    Key(u8),
}

#[derive(Debug, Clone, Copy)]
struct Knob {
    /// The value the deck last sent or was set to.
    last: u8,
    /// When it last turned; `None` once it is back at the centre.
    moved: Option<Instant>,
    /// The value before the last re-centre, while no turn has come since.
    before_centre: Option<u8>,
}

impl Default for Knob {
    fn default() -> Self {
        Self {
            last: CENTRE,
            moved: None,
            before_centre: None,
        }
    }
}

pub struct BControl {
    transport: Box<dyn Transport>,
    bcf: bool,
    opened: Instant,
    settle: Duration,
    pace: Duration,
    idle: Duration,
    echo: Duration,
    /// Ryu's preset and the first lights are queued.
    set_up: bool,
    /// Messages waiting their turn, `pace` apart.
    outbox: VecDeque<Vec<u8>>,
    next_send: Instant,
    colors: [[u8; 3]; KEYS as usize],
    /// The light each key was last sent; `None` until it is.
    sent: [Option<bool>; KEYS as usize],
    /// The value and time of each key's last light write, to spot an echo.
    last_light: [Option<(u8, Instant)>; KEYS as usize],
    held: [bool; KEYS as usize],
    /// Encoders by Control Change number.
    knobs: [Knob; 128],
    faders: [Option<u8>; BCF_FADERS as usize],
    fader_rest: [i16; BCF_FADERS as usize],
    parser: Parser,
}

impl BControl {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            bcf: model.id == BCF2000_ID,
            opened: Instant::now(),
            settle: SETTLE,
            pace: PACE,
            idle: IDLE,
            echo: ECHO,
            set_up: false,
            outbox: VecDeque::new(),
            next_send: Instant::now(),
            colors: [BLACK; KEYS as usize],
            sent: [None; KEYS as usize],
            last_light: [None; KEYS as usize],
            held: [false; KEYS as usize],
            knobs: [Knob::default(); 128],
            faders: [None; BCF_FADERS as usize],
            fader_rest: [0; BCF_FADERS as usize],
            parser: Parser::default(),
        }
    }

    const fn box_cc(&self) -> u8 {
        if self.bcf {
            BCF_BOX_CC
        } else {
            BCR_BOX_CC
        }
    }

    fn key_cc(&self, key: u8) -> u8 {
        if key < ROW_KEYS {
            FIRST_KEY_CC + key
        } else {
            self.box_cc() + key - ROW_KEYS
        }
    }

    /// Every encoder Ryu re-centres, by Control Change number.
    fn encoder_ccs(&self) -> Vec<u8> {
        let lower = if self.bcf { 0 } else { BCR_LOWER_ENCODERS };
        (FIRST_PUSH_ENCODER_CC..FIRST_PUSH_ENCODER_CC + PUSH_ENCODERS)
            .chain(FIRST_LOWER_CC..FIRST_LOWER_CC + lower)
            .collect()
    }

    fn control(&self, cc: u8) -> Option<Control> {
        let box_cc = self.box_cc();
        match cc {
            1..=32 => Some(Control::Encoder((cc - FIRST_PUSH_ENCODER_CC) % PUSH_DIALS)),
            33..=64 => Some(Control::Push((cc - FIRST_PUSH_CC) % PUSH_DIALS)),
            65..=80 => Some(Control::Key(cc - FIRST_KEY_CC)),
            _ if (box_cc..box_cc + BOX_KEYS).contains(&cc) => {
                Some(Control::Key(ROW_KEYS + cc - box_cc))
            }
            _ if self.bcf && (FIRST_LOWER_CC..FIRST_LOWER_CC + BCF_FADERS).contains(&cc) => {
                Some(Control::Fader(cc - FIRST_LOWER_CC))
            }
            _ if !self.bcf
                && (FIRST_LOWER_CC..FIRST_LOWER_CC + BCR_LOWER_ENCODERS).contains(&cc) =>
            {
                Some(Control::Encoder(PUSH_DIALS + cc - FIRST_LOWER_CC))
            }
            _ => None,
        }
    }

    /// Queues one message; a Control Change replaces one to the same number
    /// still waiting.
    fn send(&mut self, message: Vec<u8>) {
        if message.first() == Some(&CONTROL_CHANGE) {
            let number = message.get(1).copied();
            self.outbox.retain(|queued| {
                queued.first() != Some(&CONTROL_CHANGE) || queued.get(1).copied() != number
            });
        }
        self.outbox.push_back(message);
    }

    /// Writes the queued messages whose turn has come, without sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        while !self.outbox.is_empty() {
            let now = Instant::now();
            if now < self.next_send {
                break;
            }
            let Some(message) = self.outbox.pop_front() else {
                break;
            };
            if let Err(error) = self.transport.write(&message) {
                self.outbox.push_front(message);
                return Err(error);
            }
            if let [CONTROL_CHANGE, cc, value] = message[..] {
                self.note_light(cc, value, now);
            }
            self.next_send = now + self.pace;
        }
        Ok(())
    }

    /// Remembers a key light just written, to tell its echo from a press.
    fn note_light(&mut self, cc: u8, value: u8, now: Instant) {
        if let Some(Control::Key(key)) = self.control(cc) {
            if let Some(slot) = self.last_light.get_mut(usize::from(key)) {
                *slot = Some((value, now));
            }
        }
    }

    /// Whether some of Ryu's preset is still waiting to be written.
    fn preset_pending(&self) -> bool {
        self.outbox
            .iter()
            .any(|queued| queued.first() == Some(&BCL_HEAD[0]))
    }

    /// A key's light, sent whether or not it changed.
    fn send_key(&mut self, key: u8) {
        let Some(color) = self.colors.get(usize::from(key)) else {
            return;
        };
        let lit = is_lit(*color);
        let message = vec![CONTROL_CHANGE, self.key_cc(key), if lit { ON } else { OFF }];
        self.send(message);
        if let Some(sent) = self.sent.get_mut(usize::from(key)) {
            *sent = Some(lit);
        }
    }

    /// Queues the preset, every encoder back to 64, then every light.
    fn set_up(&mut self) -> DriverResult<()> {
        for (index, line) in preset_lines(self.bcf).iter().enumerate() {
            let index = u16::try_from(index).unwrap_or(u16::MAX);
            self.send(bcl_sysex(index, line));
        }
        for cc in self.encoder_ccs() {
            self.send(vec![CONTROL_CHANGE, cc, CENTRE]);
            if let Some(knob) = self.knobs.get_mut(usize::from(cc)) {
                *knob = Knob::default();
            }
        }
        self.set_up = true;
        for key in 0..KEYS {
            self.send_key(key);
        }
        self.pump()
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

    fn press(&mut self, key: u8, value: u8, events: &mut Vec<Input>) {
        // The deck repeating a light Ryu just sent is no press.
        let echo = self
            .last_light
            .get(usize::from(key))
            .copied()
            .flatten()
            .is_some_and(|(sent, at)| sent == value && at.elapsed() < self.echo);
        if echo {
            return;
        }
        let Some(held) = self.held.get_mut(usize::from(key)) else {
            return;
        };
        let pressed = value > 0;
        if *held != pressed {
            *held = pressed;
            events.push(Input::Key {
                index: key,
                pressed,
            });
        }
    }

    fn turn(&mut self, cc: u8, dial: u8, value: u8, events: &mut Vec<Input>) {
        let Some(knob) = self.knobs.get_mut(usize::from(cc)) else {
            return;
        };
        let value = value.min(127);
        let step = |from: u8| i16::from(value) - i16::from(from);
        let mut delta = step(knob.last);
        // Right after a re-centre the deck may have kept its old value.
        if let Some(before) = knob.before_centre.take() {
            if step(before).abs() < delta.abs() {
                delta = step(before);
            }
        }
        knob.last = value;
        if delta != 0 {
            knob.moved = Some(Instant::now());
        }
        if let Ok(delta) = i8::try_from(delta) {
            if delta != 0 {
                events.push(Input::DialTurn { index: dial, delta });
            }
        }
    }

    fn move_fader(&mut self, fader: u8, value: u8, events: &mut Vec<Input>) {
        let index = usize::from(fader);
        let (Some(slot), Some(rest)) = (self.faders.get_mut(index), self.fader_rest.get_mut(index))
        else {
            return;
        };
        let Some(old) = slot.replace(value.min(127)) else {
            return;
        };
        let moved = *rest + i16::from(value.min(127)) - i16::from(old);
        let steps = moved / FADER_STEP;
        *rest = moved - steps * FADER_STEP;
        if steps != 0 {
            events.push(Input::DialTurn {
                index: PUSH_DIALS + fader,
                delta: i8::try_from(steps.clamp(-127, 127)).unwrap_or(0),
            });
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let Message::Channel([status, number, value]) = *message else {
            return;
        };
        if status != CONTROL_CHANGE {
            return;
        }
        match self.control(number) {
            Some(Control::Encoder(dial)) => self.turn(number, dial, value, events),
            Some(Control::Fader(fader)) => self.move_fader(fader, value, events),
            Some(Control::Push(dial)) => events.push(Input::DialPress {
                index: dial,
                pressed: value > 0,
            }),
            Some(Control::Key(key)) => self.press(key, value, events),
            None => {}
        }
    }

    /// Puts every encoder that has been still long enough back to 64.
    fn recentre(&mut self) {
        for cc in self.encoder_ccs() {
            let Some(knob) = self.knobs.get_mut(usize::from(cc)) else {
                continue;
            };
            let Some(moved) = knob.moved else {
                continue;
            };
            if moved.elapsed() < self.idle {
                continue;
            }
            knob.moved = None;
            if knob.last != CENTRE {
                knob.before_centre = Some(knob.last);
                knob.last = CENTRE;
                self.send(vec![CONTROL_CHANGE, cc, CENTRE]);
            }
        }
    }
}

impl Driver for BControl {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.set_up {
            let left = self.settle.saturating_sub(self.opened.elapsed());
            if !left.is_zero() {
                // Anything arriving under the user's own preset is dropped.
                let wait = i32::try_from(left.as_millis())
                    .unwrap_or(i32::MAX)
                    .min(timeout_ms);
                self.read(wait)?;
                return Ok(Vec::new());
            }
            self.parser = Parser::default();
            self.set_up()?;
        }
        if self.preset_pending() {
            // Input under a half-written preset is dropped.
            self.pump()?;
            self.read(timeout_ms)?;
            self.parser = Parser::default();
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        for message in &self.read(timeout_ms)? {
            self.receive(message, &mut events);
        }
        // The deck darkens a momentary button's LED on release: put it back.
        for event in &events {
            if let Input::Key {
                index,
                pressed: false,
            } = *event
            {
                self.send_key(index);
            }
        }
        self.recentre();
        self.pump()?;
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// One Control Change for each key light that changed (every one the
    /// first time); nothing until Ryu's preset is queued.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.set_up {
            return Ok(());
        }
        for key in 0..KEYS {
            let index = usize::from(key);
            let (Some(color), Some(sent)) = (self.colors.get(index), self.sent.get(index)) else {
                continue;
            };
            if *sent != Some(is_lit(*color)) {
                self.send_key(key);
            }
        }
        self.pump()
    }

    /// The B-Controls have no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for BControl {
    /// Turns off every key light that may be on.
    fn drop(&mut self) {
        if !self.set_up {
            return;
        }
        let unsent = !self.outbox.is_empty();
        self.outbox.clear();
        let mut first = true;
        for key in 0..KEYS {
            let index = usize::from(key);
            let dark =
                self.sent.get(index) == Some(&Some(false)) && self.held.get(index) == Some(&false);
            if dark && !unsent {
                continue;
            }
            if !first && !self.pace.is_zero() {
                std::thread::sleep(self.pace);
            }
            first = false;
            let cc = self.key_cc(key);
            // An unplugged deck fails its first write: stop there.
            if self.transport.write(&[CONTROL_CHANGE, cc, OFF]).is_err() {
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

    fn open(id: &str) -> (BControl, Recorder) {
        let recorder = Recorder::default();
        let mut deck = BControl::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        deck.settle = Duration::ZERO;
        deck.pace = Duration::ZERO;
        deck.echo = Duration::ZERO;
        (deck, recorder)
    }

    /// A deck past its setup, with the setup's writes cleared.
    fn ready(id: &str) -> (BControl, Recorder) {
        let (mut deck, recorder) = open(id);
        deck.poll(0).unwrap();
        recorder.0.lock().unwrap().writes.clear();
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

    fn text(message: &[u8]) -> String {
        String::from_utf8(message[9..message.len() - 1].to_vec()).unwrap()
    }

    #[test]
    fn both_models_are_found_by_port_name_and_share_one_usb_id() {
        let bcr = by_id(BCR2000_ID).unwrap();
        let bcf = by_id(BCF2000_ID).unwrap();
        for model in [bcr, bcf] {
            assert_eq!(model.protocol, Protocol::BehringerBControlMidi);
            assert!(model.protocol.is_midi() && !model.protocol.has_brightness());
            assert_eq!(model.dial_lights, DialLights::None);
            assert!(model.key_lights && model.deck_lights == 0);
            assert_eq!(ModelView::from(model).vendor_id, "1397");
        }
        assert_eq!((bcr.dials, bcf.dials), (32, 16));
        // Both report 1397:00bc; only one row lists it, so ids stay unique.
        assert_eq!(bcr.product_ids, &[0x00bc]);
        assert!(bcf.product_ids.is_empty());
        for (raw, windows, id) in [
            ("BCR2000", false, Some(BCR2000_ID)),
            ("BCR2000 Port 1", false, Some(BCR2000_ID)),
            ("BCR2000:BCR2000 MIDI 1 24:0", false, Some(BCR2000_ID)),
            ("bcf2000", false, Some(BCF2000_ID)),
            ("BCF2000 MIDI 1", false, Some(BCF2000_ID)),
            ("MIDIIN2 (BCR2000)", true, None),
            ("BCR2000 Port 2", false, None),
            ("BCF2000 MIDI 3", false, None),
            ("BCR2000:BCR2000 MIDI 2 24:1", false, None),
            ("BCN44", false, None),
        ] {
            assert_eq!(
                identify_midi(&normalise(raw), windows).map(|model| model.id),
                id,
                "{raw}"
            );
        }
        let view = ModelView::from(bcr);
        assert_eq!(view.key_names.unwrap()[16], "Box top left");
        assert_eq!(
            view.key_cells.unwrap(),
            (0..8)
                .chain(10..18)
                .chain([8, 9, 18, 19])
                .collect::<Vec<u16>>()
        );
    }

    #[test]
    fn the_preset_is_one_sysex_per_bcl_line() {
        let bcr = preset_lines(false);
        let bcf = preset_lines(true);
        assert_eq!((bcr.len(), bcf.len()), (390, 342));
        assert_eq!(
            bcl_sysex(0, &bcr[0]),
            vec![
                0xf0, 0x00, 0x20, 0x32, 0x7f, 0x7f, 0x20, 0x00, 0x00, 0x24, 0x72, 0x65, 0x76, 0x20,
                0x52, 0x31, 0xf7
            ]
        );
        assert_eq!(bcf[0], "$rev F1");
        assert_eq!(bcr[2], format!(".name 'Ryu{}'", " ".repeat(21)));
        assert_eq!(
            bcr[9..13],
            [
                "$encoder 1",
                ".easypar CC 1 1 0 127 absolute",
                ".showvalue on",
                ".mode 12dot"
            ]
        );
        // The BCR's lower encoders, then the buttons and the box.
        assert_eq!(bcr[137], "$encoder 33");
        assert_eq!(bcr[138], ".easypar CC 1 81 0 127 absolute");
        assert_eq!(bcr[229], "$encoder 56");
        assert_eq!(bcr[230], ".easypar CC 1 104 0 127 absolute");
        assert_eq!(
            bcr[233..236],
            [
                "$button 1",
                ".easypar CC 1 33 127 0 toggleoff",
                ".showvalue on"
            ]
        );
        assert_eq!(bcr[386], "$button 52");
        assert_eq!(bcr[387], ".easypar CC 1 108 127 0 toggleoff");
        assert_eq!(bcr[389], "$end");
        // The BCF: box on 89-92, faders after the buttons, motors off.
        assert_eq!(bcf[290], "$button 52");
        assert_eq!(bcf[291], ".easypar CC 1 92 127 0 toggleoff");
        assert_eq!(
            bcf[293..299],
            [
                "$fader 1",
                ".easypar CC 1 81 0 127 absolute",
                ".showvalue on",
                ".motor off",
                ".override move",
                ".keyoverride off"
            ]
        );
        assert_eq!(bcf[336], ".easypar CC 1 88 0 127 absolute");
        assert_eq!(bcf[341], "$end");
        for lines in [&bcr, &bcf] {
            assert!(lines.iter().all(|line| !line.starts_with(' ')));
            assert!(!lines.iter().any(|line| line.contains(".store")));
        }
        // The running number rolls over into the high byte at line 128.
        assert_eq!(bcl_sysex(128, "$end")[7..9], [0x01, 0x00]);
        assert_eq!(bcl_sysex(389, "$end")[7..9], [0x03, 0x05]);
    }

    #[test]
    fn opening_writes_the_preset_then_centres_and_lights() {
        let (mut deck, recorder) = open(BCR2000_ID);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        // Nothing is written before the setup.
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
        // Input under the user's preset or a half-written one is dropped.
        deck.settle = Duration::from_secs(60);
        queue(&recorder, &[&[0xb0, 0x41, 0x7f]]);
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(recorder.writes().is_empty());
        deck.settle = Duration::ZERO;
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 390 + 56 + 20);
        assert_eq!(text(&writes[0]), "$rev R1");
        assert_eq!(writes[0].len(), 10 + 7);
        assert_eq!(writes[128][7..9], [0x01, 0x00]);
        assert_eq!(text(&writes[389]), "$end");
        assert_eq!(writes[390], vec![0xb0, 1, 0x40]);
        assert_eq!(writes[390 + 32], vec![0xb0, 81, 0x40]);
        assert_eq!(writes[445], vec![0xb0, 104, 0x40]);
        assert_eq!(writes[446], vec![0xb0, 0x41, 0x7f]);
        assert_eq!(writes[447], vec![0xb0, 0x42, 0x00]);
        assert_eq!(writes[462], vec![0xb0, 0x69, 0x00]);
        assert_eq!(writes[465], vec![0xb0, 0x6c, 0x00]);
    }

    #[test]
    fn the_bcf_centres_only_its_encoders() {
        let (mut deck, recorder) = open(BCF2000_ID);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 342 + 32 + 20);
        assert_eq!(text(&writes[0]), "$rev F1");
        assert_eq!(writes[373], vec![0xb0, 32, 0x40]);
        assert_eq!(writes[374], vec![0xb0, 0x41, 0x00]);
        assert_eq!(writes[393], vec![0xb0, 0x5c, 0x00]);
        // The faders are never written.
        assert!(!writes
            .iter()
            .any(|write| write[0] == 0xb0 && (81..=88).contains(&write[1])));
    }

    #[test]
    fn writes_wait_their_turn_without_sleeping() {
        let (mut deck, recorder) = open(BCR2000_ID);
        deck.pace = Duration::from_secs(60);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 1);
        // Input waits for the preset.
        queue(&recorder, &[&[0xb0, 0x41, 0x7f]]);
        assert!(deck.poll(0).unwrap().is_empty());
        assert_eq!(recorder.writes().len(), 1);
        deck.pace = Duration::ZERO;
        deck.next_send = Instant::now();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 466);
    }

    #[test]
    fn keys_and_encoder_pushes() {
        let (mut deck, recorder) = ready(BCR2000_ID);
        queue(
            &recorder,
            &[
                &[0xb0, 0x41, 0x7f],
                &[0xb0, 0x50, 0x7f],
                &[0xb0, 0x69, 0x7f],
                &[0xb0, 0x6c, 0x00],
                &[0xb0, 0x2a, 0x7f],
                &[0xb0, 0x2a, 0x00],
                // Other channels, statuses and numbers are ignored.
                &[0xb1, 0x41, 0x7f],
                &[0x90, 0x41, 0x7f],
                &[0xb0, 0x6d, 0x7f],
                &[0xf0, 0x00, 0x20, 0x32, 0x7f, 0x7f, 0x20, 0xf7],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(15, true),
                key(16, true),
                Input::DialPress {
                    index: 1,
                    pressed: true
                },
                Input::DialPress {
                    index: 1,
                    pressed: false
                },
            ]
        );
        queue(&recorder, &[&[0xb0, 0x41, 0x00]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(0, false)]);
        // The release's light is sent again.
        assert_eq!(recorder.writes(), vec![vec![0xb0, 0x41, 0x00]]);
    }

    #[test]
    fn the_bcf_box_is_on_its_own_numbers() {
        let (mut deck, recorder) = ready(BCF2000_ID);
        queue(&recorder, &[&[0xb0, 0x59, 0x7f], &[0xb0, 0x69, 0x7f]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(16, true)]);
    }

    #[test]
    fn a_key_light_is_on_or_off_and_sent_again_after_release() {
        let (mut deck, recorder) = ready(BCR2000_ID);
        deck.set_face(0, Some(&lit([200, 0, 0]))).unwrap();
        deck.set_face(1, Some(&lit([10, 20, 31]))).unwrap();
        deck.set_face(19, Some(&lit([0, 0, 40]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![vec![0xb0, 0x41, 0x7f], vec![0xb0, 0x6c, 0x7f]]
        );
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 2);
        queue(&recorder, &[&[0xb0, 0x41, 0x7f], &[0xb0, 0x41, 0x00]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(0, true), key(0, false)]);
        assert_eq!(recorder.writes()[2], vec![0xb0, 0x41, 0x7f]);
        deck.set_brightness(0).unwrap();
        assert_eq!(recorder.writes().len(), 3);
    }

    #[test]
    fn an_echo_of_a_light_is_no_press() {
        let (mut deck, recorder) = ready(BCR2000_ID);
        deck.echo = Duration::from_secs(60);
        deck.set_face(2, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        queue(&recorder, &[&[0xb0, 0x43, 0x7f]]);
        assert!(deck.poll(0).unwrap().is_empty());
        // Another key's press is no echo.
        queue(&recorder, &[&[0xb0, 0x44, 0x7f]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(3, true)]);
    }

    #[test]
    fn every_encoder_group_turns_the_same_dial() {
        let (mut deck, recorder) = ready(BCR2000_ID);
        queue(
            &recorder,
            &[
                &[0xb0, 0x02, 0x43],
                &[0xb0, 0x0a, 0x3e],
                &[0xb0, 0x0a, 0x3f],
                &[0xb0, 0x51, 0x41],
                &[0xb0, 0x68, 0x30],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(1, 3),
                turn(1, -2),
                turn(1, 1),
                turn(8, 1),
                turn(31, -16)
            ]
        );
        // Not still long enough to be re-centred.
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn a_still_encoder_is_put_back_to_the_centre() {
        let (mut deck, recorder) = ready(BCR2000_ID);
        deck.idle = Duration::ZERO;
        queue(&recorder, &[&[0xb0, 0x03, 70]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(2, 6)]);
        assert_eq!(recorder.writes(), vec![vec![0xb0, 0x03, 0x40]]);
        // The deck's echo of the centre is no turn.
        queue(&recorder, &[&[0xb0, 0x03, 0x40]]);
        assert!(deck.poll(0).unwrap().is_empty());
        queue(&recorder, &[&[0xb0, 0x03, 65]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(2, 1)]);
    }

    #[test]
    fn a_turn_that_beat_the_recentre_counts_from_the_old_value() {
        let (mut deck, recorder) = ready(BCR2000_ID);
        deck.idle = Duration::ZERO;
        queue(&recorder, &[&[0xb0, 0x07, 120]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(6, 56)]);
        queue(&recorder, &[&[0xb0, 0x07, 121]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(6, 1)]);
    }

    #[test]
    fn bcf_faders_seed_then_step_and_are_never_written() {
        let (mut deck, recorder) = ready(BCF2000_ID);
        deck.idle = Duration::ZERO;
        queue(
            &recorder,
            &[
                &[0xb0, 0x51, 10],
                &[0xb0, 0x51, 13],
                &[0xb0, 0x51, 14],
                &[0xb0, 0x58, 100],
                &[0xb0, 0x58, 90],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(8, 1), turn(8, 1), turn(15, -5)]
        );
        assert!(recorder.writes().is_empty());
        // CC 89-104 are no encoders on the BCF.
        queue(&recorder, &[&[0xb0, 0x60, 10]]);
        assert!(deck.poll(0).unwrap().is_empty());
    }

    #[test]
    fn letting_go_darkens_every_light_that_may_be_on() {
        let (mut deck, recorder) = ready(BCR2000_ID);
        deck.set_face(5, Some(&lit([255, 255, 0]))).unwrap();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        drop(deck);
        assert_eq!(recorder.writes(), vec![vec![0xb0, 0x46, 0x00]]);
        // A deck never set up sends nothing.
        let (deck, recorder) = open(BCF2000_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }
}
