//! Mackie's MCU Pro (the Mackie Control Universal Pro) and its XT Pro
//! extender, over the MCU Pro's class-compliant USB MIDI ([`super::midi`]),
//! in Mackie Control mode.
//!
//! The MCU Pro lists four MIDI ports each way, all named after its product
//! string (`MCU Pro USB v3.1`; the version may change, so only `MCU PRO
//! USB` is matched, in any case). The first is the MCU Pro itself: on
//! Windows the plain name, on Linux ALSA port 0. The other three are the DIN
//! MIDI jacks an XT Pro (or any other MIDI gear) hangs off: on Windows they
//! are wrapped `MIDIIN<n> (MCU Pro USB v3.1)` (Bitwig numbers them from 1,
//! Cubase from 2, so any wrapped port is an extender candidate), on Linux
//! ALSA ports 1-3. Each extender port is its own deck. No reference gives
//! the XT Pro's own USB id or port name, so a stand-alone XT Pro on USB is
//! not found. On Windows and Linux the three extender ports are listed as
//! XT Pro decks whether or not anything is chained; each only sends the
//! device query until an XT Pro answers. The macOS names of ports 2-4 are
//! not documented, so there every same-named port becomes an MCU Pro deck
//! that queries both the MCU Pro's and the XT Pro's ids: a chained XT Pro
//! that answers drives that deck with the MCU Pro's layout (its strip keys,
//! fader touches, V-Pots and faders), and only its own lights are written.
//!
//! Opening is silent but for Mackie's device query (`F0 00 00 66 <id> 00
//! F7`, id `14` on the MCU Pro, `15` on the XT Pro, and the Logic Control
//! ids `10`/`11` for a unit booted in that emulation), sent again from the
//! poll loop every two seconds until the deck answers `F0 00 00 66 <id> 01
//! ... F7`. In Mackie mode that answer makes the deck live; in Logic Control
//! mode it carries a challenge, answered (`02`, the seven serial bytes and
//! four response bytes) before the deck confirms (`03`, then Ryu sends `13
//! 00`) or refuses (`04`: Ryu stays silent and logs a hint); with no
//! confirmation two seconds later Ryu logs it once and queries again. A
//! ready answer from a live deck (a unit rebooted behind an open port)
//! darkens and greets it again and resends every light. The id of the
//! answer goes in every later SysEx header. On Windows and Linux the MCU
//! Pro's own port may also go live on any note, Control Change or pitch
//! bend; an extender port (a keyboard's pitch wheel would fake it) and any
//! macOS port wait for the SysEx answer, so a synth on the DIN jacks never
//! gets a note from Ryu.
//!
//! In, on MIDI channel 1: every button and fader touch is a Note On,
//! pressed at a velocity over 64 and released at 64 or under (a Note Off
//! releases too). On the MCU Pro a key's index is its note, `0x00`-`0x70`,
//! eight to a row: Rec, Solo, Mute, Select and V-Pot push by strip, the
//! globals in Mackie note order (`0x28` Track to `0x67` User Switch B), the
//! fader touches (row 14) and the master fader touch (row 15). On the XT
//! Pro keys 1-40 are notes `0x00`-`0x27` and keys 41-48 the fader touches
//! `0x68`-`0x6F`. The V-Pots are `CC 0x10+s` (dials 1-8) and the MCU Pro's
//! jog wheel `CC 0x3C` (dial 9), sign-magnitude relative: bit 6 set turns
//! anticlockwise, bits 0-5 count the steps (0 counts as 1; the jog wheel
//! accelerates, at most 15 a message). The motor faders are pitch bend on
//! channels 1-8 (and the MCU Pro's master on 9): the first value of each
//! only seeds it, then every 128 of its 14 bits turn its dial a step (dials
//! 10-18 on the MCU Pro, 9-16 on the XT Pro). The expression pedal (`CC
//! 0x2E`) is not read.
//!
//! Out, once live, on channel 1: a button's LED is `90 <note> 7F` on and
//! `90 <note> 00` off (V-Pot pushes, foot switches and fader touches have
//! none); a V-Pot's LED ring (the dial's light) is `B0 30+s 2B` (every LED
//! in wrap mode) or `00`. Going live turns every LED, the MCU Pro's SMPTE,
//! Beats and Rude Solo lamps (`90 71-73 00`) and the rings off, writes `Ryu`
//! on the 2 x 56 LCD (`F0 00 00 66 <id> 12 00 <112 chars> F7`) and on the
//! MCU Pro blanks the ten timecode and two assignment digits (`B0 40-4B
//! 20`). Every write waits in a queue drained by poll and flush, a
//! millisecond apart and five after a SysEx (a guess; no wait in tests),
//! and Ryu never sleeps in the poll loop. Letting go turns the lights off
//! and blanks the displays at once. Ryu never sends pitch bend (the faders
//! have motors), a reset, a fader recalibration, touch sensitivity, the
//! backlight or meter-mode SysEx, or HUI's heartbeat. There is no
//! brightness command and no keep-alive.
//!
//! Facts from Ardour's Mackie Control surface and its MCU Pro device files
//! (C++, GPL-2.0-or-later), DrivenByMoss's Mackie Control support (Java,
//! LGPL-3.0), Ableton Live's Mackie Control scripts (decompiled, facts
//! only), and the Cubase MIDI Remote (MIT) and Bitwig (MIT) MCU Pro
//! definitions; no code was copied. Untested on hardware.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::icon_qcon::{relative, Fader, NAMES};
use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const MCU_PRO_ID: &str = "mackie-mcu-pro";
pub const XT_PRO_ID: &str = "mackie-xt-pro";

/// Mackie Designs' USB vendor id and the MCU Pro's product id (from
/// Ableton Live's script), shown only: the deck is found by port name.
pub const MCU_PRO_PRODUCT_ID: u16 = 0x0006;

pub const MCU_ROWS: u8 = 15;
pub const MCU_COLS: u8 = 8;
/// Notes `0x00` to `0x70`, each its own key.
pub const MCU_KEYS: u8 = 113;
/// Eight V-Pots, the jog wheel, eight faders and the master fader.
pub const MCU_DIALS: u8 = 18;
pub const XT_ROWS: u8 = 6;
pub const XT_COLS: u8 = 8;
/// Rec, Solo, Mute, Select, V-Pot push and fader touch for eight strips.
pub const XT_KEYS: u8 = 48;
/// Eight V-Pots and eight faders.
pub const XT_DIALS: u8 = 16;
/// The V-Pots' LED rings (dials 1-8); the jog wheel and faders have none.
pub const RINGS: u8 = 8;

const STRIPS: u8 = 8;
const XT_LAST_STRIP_NOTE: u8 = 0x27;
const XT_FIRST_TOUCH_KEY: u8 = 40;
const FADER_TOUCH: u8 = 0x68;
const MASTER_TOUCH: u8 = 0x70;
const LAST_STRIP_LIGHT: u8 = 0x1f;
/// The MCU Pro's global buttons with a light, Track to Scrub.
const GLOBAL_LIGHTS: std::ops::RangeInclusive<u8> = 0x28..=0x65;
/// SMPTE, Beats and Rude Solo.
const LAMPS: std::ops::RangeInclusive<u8> = 0x71..=0x73;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const PITCH_BEND: u8 = 0xe0;
const CHANNEL_PRESSURE: u8 = 0xd0;
const VPOT_CC: u8 = 0x10;
const JOG_CC: u8 = 0x3c;
const EXTERNAL_CC: u8 = 0x2e;
/// HUI's zone and port select: a unit booted in HUI mode.
const HUI_ZONE_CC: u8 = 0x0f;
const HUI_PORT_CC: u8 = 0x2f;
const RING_CC: u8 = 0x30;
/// The ten timecode digits (`0x40` the rightmost) and the two assignment
/// digits (`0x4A` right, `0x4B` left).
const SEGMENT_CCS: std::ops::RangeInclusive<u8> = 0x40..=0x4b;
const BLANK: u8 = 0x20;
const PRESS_OVER: u8 = 64;

const LED_OFF: u8 = 0;
const LED_ON: u8 = 0x7f;
/// Wrap mode, all eleven LEDs.
const RING_ALL: u8 = 0x2b;
const RING_OFF: u8 = 0;

const SYSEX_START: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
const MACKIE: [u8; 3] = [0x00, 0x00, 0x66];
const QUERY: u8 = 0x00;
const READY: u8 = 0x01;
const CHALLENGE_REPLY: u8 = 0x02;
const CONFIRMED: u8 = 0x03;
const DENIED: u8 = 0x04;
const VERSION_REQUEST: u8 = 0x13;
const LCD: u8 = 0x12;
const MCU_DEVICE: u8 = 0x14;
const XT_DEVICE: u8 = 0x15;
const LC_DEVICE: u8 = 0x10;
const LC_XT_DEVICE: u8 = 0x11;
/// A Logic Control challenge: header, command, seven serial bytes, four
/// challenge bytes, end.
const CHALLENGE_LEN: usize = 18;
const SERIAL: std::ops::Range<usize> = 6..13;
const CHALLENGE: std::ops::Range<usize> = 13..17;
/// Both LCD lines, 56 characters each, from offset 0.
const LCD_CHARS: usize = 112;
const GREETING: &[u8] = b"Ryu";

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Fader units (of 16384) per dial step: 128 from end to end.
const FADER_STEP: i32 = 128;
const MAX_FADERS: usize = 9;

/// The gap after a short message and after a SysEx, a guess (none in
/// tests).
const SHORT_GAP: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(1)
};
const SYSEX_GAP: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(5)
};
/// How often the device query goes out until the deck answers (every poll
/// in tests).
const QUERY_EVERY: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(2)
};
const MAX_CREDIT: Duration = Duration::from_millis(10);
const READ_LEN: usize = 256;
const MAX_MESSAGES_PER_POLL: usize = 64;
/// Whether port names say which port is the MCU Pro's own (Windows and
/// Linux do; macOS's names of the extra ports are not documented).
const NAMED_PORTS: bool = !cfg!(target_os = "macos");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Mcu,
    Xt,
}

impl Kind {
    fn of(model: &DeviceModel) -> Option<Self> {
        match (model.protocol, model.id) {
            (Protocol::MackieMcuProMidi, MCU_PRO_ID) => Some(Self::Mcu),
            (Protocol::MackieXtProMidi, XT_PRO_ID) => Some(Self::Xt),
            _ => None,
        }
    }

    const fn keys(self) -> u8 {
        match self {
            Self::Mcu => MCU_KEYS,
            Self::Xt => XT_KEYS,
        }
    }

    /// Its own SysEx id, then its Logic Control emulation's.
    const fn devices(self) -> [u8; 2] {
        match self {
            Self::Mcu => [MCU_DEVICE, LC_DEVICE],
            Self::Xt => [XT_DEVICE, LC_XT_DEVICE],
        }
    }

    fn key(self, note: u8) -> Option<u8> {
        match self {
            Self::Mcu => (note <= MASTER_TOUCH).then_some(note),
            Self::Xt => match note {
                0..=XT_LAST_STRIP_NOTE => Some(note),
                _ if (FADER_TOUCH..FADER_TOUCH + STRIPS).contains(&note) => {
                    Some(note - FADER_TOUCH + XT_FIRST_TOUCH_KEY)
                }
                _ => None,
            },
        }
    }

    fn note(self, key: u8) -> Option<u8> {
        match self {
            Self::Mcu => (key < MCU_KEYS).then_some(key),
            Self::Xt => match key {
                0..=XT_LAST_STRIP_NOTE => Some(key),
                XT_FIRST_TOUCH_KEY..XT_KEYS => Some(key - XT_FIRST_TOUCH_KEY + FADER_TOUCH),
                _ => None,
            },
        }
    }

    fn has_light(self, key: u8) -> bool {
        key <= LAST_STRIP_LIGHT || (self == Self::Mcu && GLOBAL_LIGHTS.contains(&key))
    }

    fn lit_keys(self) -> Vec<u8> {
        (0..self.keys())
            .filter(|key| self.has_light(*key))
            .collect()
    }

    fn names(self) -> Vec<&'static str> {
        (0..self.keys())
            .filter_map(|key| self.note(key))
            .filter_map(|note| NAMES.get(usize::from(note)).copied())
            .collect()
    }

    const fn first_fader_dial(self) -> u8 {
        match self {
            Self::Mcu => 9,
            Self::Xt => 8,
        }
    }

    const fn faders(self) -> u8 {
        match self {
            Self::Mcu => 9,
            Self::Xt => 8,
        }
    }
}

/// Whether a normalised port name is this model's: the uppercased name
/// holds `MCU PRO USB`; the MCU Pro is the unwrapped name on ALSA port 0
/// (or with no ALSA number), the XT Pro any Windows-wrapped port or ALSA
/// ports 1-3.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    let Some(kind) = Kind::of(model) else {
        return false;
    };
    if !name.base.to_ascii_uppercase().contains("MCU PRO USB") {
        return false;
    }
    match kind {
        Kind::Mcu => name.interface.is_none() && matches!(name.alsa_port, None | Some(0)),
        Kind::Xt => matches!(name.interface, Some(1..=4)) || matches!(name.alsa_port, Some(1..=3)),
    }
}

/// The Mackie function of each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    Kind::of(model).map(Kind::names)
}

/// The keys with a light: Rec, Solo, Mute, Select and the MCU Pro's global
/// buttons from Track to Scrub.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    Kind::of(model).map(Kind::lit_keys)
}

fn is_lit(color: Option<[u8; 3]>) -> bool {
    color.is_some_and(|color| color.iter().any(|channel| *channel >= DARK))
}

/// The four response bytes to a Logic Control challenge, computed as C
/// does on promoted bytes: wrapping 32-bit arithmetic, a shift past the
/// width giving zero, the low seven bits kept.
fn challenge_response(challenge: [u8; 4]) -> [u8; 4] {
    let [l0, l1, l2, l3] = challenge.map(i32::from);
    let shift = |value: i32, by: i32| {
        u32::try_from(by)
            .ok()
            .and_then(|by| value.checked_shr(by))
            .unwrap_or(0)
    };
    let shl = |value: i32, by: u32| value.checked_shl(by).unwrap_or(0);
    let r0 = l0.wrapping_add(l1 ^ 0x0a).wrapping_sub(l3);
    let r1 = shift(l2, l3) ^ l0.wrapping_add(l3);
    let r2 = l3.wrapping_sub(shl(l2, 2)) ^ (l0 | l1);
    let r3 = l1.wrapping_sub(l2).wrapping_add(0xf0 ^ shl(l3, 4));
    [r0, r1, r2, r3].map(|value| u8::try_from(value & 0x7f).unwrap_or(0))
}

fn sysex(device: u8, body: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(body.len() + 6);
    message.push(SYSEX_START);
    message.extend_from_slice(&MACKIE);
    message.push(device);
    message.extend_from_slice(body);
    message.push(SYSEX_END);
    message
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Querying, writing nothing else.
    Waiting,
    /// A Logic Control challenge answered; waiting for the confirmation.
    Challenged,
    Live,
    /// The deck refused the handshake: nothing more is written.
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Led(u8),
    Ring(u8),
    Other,
}

#[derive(Debug)]
struct Out {
    slot: Slot,
    bytes: Vec<u8>,
}

pub struct MackieMcu {
    kind: Kind,
    transport: Box<dyn Transport>,
    phase: Phase,
    /// The SysEx id the deck answered with (its own until then).
    device: u8,
    /// When the device query next goes out while waiting.
    query_at: Option<Instant>,
    query_every: Duration,
    outbox: VecDeque<Out>,
    ready_at: Instant,
    short_gap: Duration,
    sysex_gap: Duration,
    leds: Vec<bool>,
    queued: Vec<bool>,
    held: Vec<bool>,
    rings: [bool; RINGS as usize],
    queued_rings: [bool; RINGS as usize],
    faders: [Fader; MAX_FADERS],
    warned: bool,
    /// Whether a Logic Control confirmation that never came was logged.
    unconfirmed_warned: bool,
    /// Whether port names tell the MCU Pro's port from the extenders'.
    named_ports: bool,
    /// The unit that answered: an XT Pro may answer an MCU Pro deck on a
    /// macOS port.
    unit: Kind,
    parser: Parser,
}

impl MackieMcu {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model).unwrap_or(Kind::Xt);
        let keys = usize::from(kind.keys());
        Self {
            kind,
            transport,
            phase: Phase::Waiting,
            device: kind.devices()[0],
            query_at: None,
            query_every: QUERY_EVERY,
            outbox: VecDeque::new(),
            ready_at: Instant::now(),
            short_gap: SHORT_GAP,
            sysex_gap: SYSEX_GAP,
            leds: vec![false; keys],
            queued: vec![false; keys],
            held: vec![false; keys],
            rings: [false; RINGS as usize],
            queued_rings: [false; RINGS as usize],
            faders: [Fader::default(); MAX_FADERS],
            warned: false,
            unconfirmed_warned: false,
            named_ports: NAMED_PORTS,
            unit: kind,
            parser: Parser::default(),
        }
    }

    fn push(&mut self, slot: Slot, bytes: Vec<u8>) {
        if slot != Slot::Other {
            self.outbox.retain(|out| out.slot != slot);
        }
        self.outbox.push_back(Out { slot, bytes });
    }

    /// The SysEx ids this deck queries and accepts answers from: on an
    /// unnamed (macOS) port an MCU Pro deck takes an XT Pro's too.
    fn devices(&self) -> Vec<u8> {
        let mut devices = self.kind.devices().to_vec();
        if self.kind == Kind::Mcu && !self.named_ports {
            devices.extend(Kind::Xt.devices());
        }
        devices
    }

    /// Whether a key has a light on both the model and the unit behind it.
    fn lit(&self, key: u8) -> bool {
        self.kind.has_light(key) && self.unit.has_light(key)
    }

    /// Queues the device query when it is due, while the deck has not
    /// answered or has not confirmed a Logic Control challenge.
    fn query(&mut self) {
        if !matches!(self.phase, Phase::Waiting | Phase::Challenged) {
            return;
        }
        let now = Instant::now();
        if self.query_at.is_some_and(|at| at > now) {
            return;
        }
        if self.phase == Phase::Challenged {
            self.phase = Phase::Waiting;
            if !self.unconfirmed_warned {
                self.unconfirmed_warned = true;
                tracing::warn!(
                    "the Mackie MCU Pro / XT Pro never confirmed Logic Control's handshake; \
                     asking it again"
                );
            }
        }
        self.query_at = Some(now + self.query_every);
        for device in self.devices() {
            self.push(Slot::Other, sysex(device, &[QUERY]));
        }
    }

    fn led_message(&self, key: u8, on: bool) -> Option<Vec<u8>> {
        let note = self.kind.note(key)?;
        Some(vec![NOTE_ON, note, if on { LED_ON } else { LED_OFF }])
    }

    fn lcd_message(&self, text: &[u8]) -> Vec<u8> {
        let mut body = vec![LCD, 0];
        let mut chars = [BLANK; LCD_CHARS];
        for (slot, byte) in chars.iter_mut().zip(text) {
            *slot = *byte;
        }
        body.extend_from_slice(&chars);
        sysex(self.device, &body)
    }

    /// Every light off and the 7-segment digits blank.
    fn dark_messages(&self) -> Vec<(Slot, Vec<u8>)> {
        let mut messages = Vec::new();
        for key in self.kind.lit_keys() {
            if !self.unit.has_light(key) {
                continue;
            }
            if let Some(bytes) = self.led_message(key, false) {
                messages.push((Slot::Led(key), bytes));
            }
        }
        if self.unit == Kind::Mcu {
            for lamp in LAMPS {
                messages.push((Slot::Other, vec![NOTE_ON, lamp, LED_OFF]));
            }
        }
        for strip in 0..RINGS {
            messages.push((
                Slot::Ring(strip),
                vec![CONTROL_CHANGE, RING_CC + strip, RING_OFF],
            ));
        }
        if self.unit == Kind::Mcu {
            for digit in SEGMENT_CCS {
                messages.push((Slot::Other, vec![CONTROL_CHANGE, digit, BLANK]));
            }
        }
        messages
    }

    /// The deck answered: darken it, greet on the LCD, then whatever is
    /// staged.
    fn go_live(&mut self) {
        if self.phase == Phase::Live {
            return;
        }
        self.phase = Phase::Live;
        self.outbox.clear();
        for (slot, bytes) in self.dark_messages() {
            self.push(slot, bytes);
        }
        let greeting = self.lcd_message(GREETING);
        self.push(Slot::Other, greeting);
        self.queued.fill(false);
        self.queued_rings = [false; RINGS as usize];
        self.queue_changes();
    }

    fn queue_changes(&mut self) {
        if self.phase != Phase::Live {
            return;
        }
        for key in 0..self.kind.keys() {
            let index = usize::from(key);
            let on = self.leds.get(index).copied().unwrap_or(false);
            if !self.lit(key) || self.queued.get(index) == Some(&on) {
                continue;
            }
            if let Some(queued) = self.queued.get_mut(index) {
                *queued = on;
            }
            if let Some(bytes) = self.led_message(key, on) {
                self.push(Slot::Led(key), bytes);
            }
        }
        for strip in 0..RINGS {
            let index = usize::from(strip);
            let on = self.rings[index];
            if self.queued_rings[index] == on {
                continue;
            }
            self.queued_rings[index] = on;
            let value = if on { RING_ALL } else { RING_OFF };
            self.push(
                Slot::Ring(strip),
                vec![CONTROL_CHANGE, RING_CC + strip, value],
            );
        }
    }

    fn pump(&mut self) -> DriverResult<()> {
        let now = Instant::now();
        if let Some(floor) = now.checked_sub(MAX_CREDIT) {
            self.ready_at = self.ready_at.max(floor);
        }
        while let Some(out) = self.outbox.front() {
            if self.ready_at > now {
                break;
            }
            self.transport.write(&out.bytes)?;
            let gap = if out.bytes.first() == Some(&SYSEX_START) {
                self.sysex_gap
            } else {
                self.short_gap
            };
            self.outbox.pop_front();
            self.ready_at += gap;
        }
        Ok(())
    }

    fn hint(&mut self) {
        if !self.warned {
            self.warned = true;
            tracing::warn!(
                "the Mackie MCU Pro / XT Pro sent a message Ryu does not expect; boot it in its \
                 Mackie Control (or Logic Control) mode for Ryu to read it"
            );
        }
    }

    fn sysex(&mut self, bytes: &[u8]) {
        if bytes.get(1..4) != Some(&MACKIE[..]) {
            return;
        }
        let Some(&device) = bytes.get(4) else {
            return;
        };
        if !self.devices().contains(&device) {
            return;
        }
        match bytes.get(5).copied() {
            Some(READY) => self.ready(device, bytes),
            Some(CONFIRMED) if self.phase == Phase::Challenged && device == self.device => {
                self.go_live();
                self.outbox.push_front(Out {
                    slot: Slot::Other,
                    bytes: sysex(device, &[VERSION_REQUEST, 0]),
                });
            }
            Some(DENIED) if self.phase != Phase::Live => {
                self.phase = Phase::Denied;
                self.outbox.clear();
                tracing::warn!(
                    "the Mackie MCU Pro / XT Pro refused Logic Control's handshake; boot it in \
                     its Mackie Control mode for Ryu to drive it"
                );
            }
            _ => {}
        }
    }

    /// A device-ready answer: live at once in Mackie mode, a challenge to
    /// answer in Logic Control mode. A live deck that announces itself again
    /// was rebooted: it is darkened, greeted and relit (or challenged) anew.
    fn ready(&mut self, device: u8, bytes: &[u8]) {
        if self.phase == Phase::Denied {
            return;
        }
        let unit = if Kind::Xt.devices().contains(&device) {
            Kind::Xt
        } else {
            Kind::Mcu
        };
        if device == unit.devices()[0] {
            self.device = device;
            self.unit = unit;
            if bytes.len() == CHALLENGE_LEN {
                tracing::debug!(serial = ?bytes.get(SERIAL), "Mackie surface answered");
            }
            self.phase = Phase::Waiting;
            self.go_live();
            return;
        }
        if bytes.len() != CHALLENGE_LEN {
            return;
        }
        let (Some(serial), Some(&[l0, l1, l2, l3])) = (bytes.get(SERIAL), bytes.get(CHALLENGE))
        else {
            return;
        };
        let mut body = vec![CHALLENGE_REPLY];
        body.extend_from_slice(serial);
        body.extend_from_slice(&challenge_response([l0, l1, l2, l3]));
        self.device = device;
        self.unit = unit;
        self.outbox.clear();
        self.push(Slot::Other, sysex(device, &body));
        self.phase = Phase::Challenged;
        self.query_at = Some(Instant::now() + self.query_every);
    }

    fn note(&mut self, note: u8, pressed: bool, events: &mut Vec<Input>) {
        let Some(key) = self.kind.key(note) else {
            self.hint();
            return;
        };
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

    fn control(&mut self, number: u8, value: u8, events: &mut Vec<Input>) {
        let index = match number {
            JOG_CC if self.kind == Kind::Mcu => STRIPS,
            _ if (VPOT_CC..VPOT_CC + STRIPS).contains(&number) => number - VPOT_CC,
            EXTERNAL_CC => return,
            _ => {
                self.hint();
                return;
            }
        };
        events.push(Input::DialTurn {
            index,
            delta: relative(value),
        });
    }

    fn fader(&mut self, channel: u8, lsb: u8, msb: u8, events: &mut Vec<Input>) {
        if channel >= self.kind.faders() {
            return;
        }
        let Some(fader) = self.faders.get_mut(usize::from(channel)) else {
            return;
        };
        let value = u16::from(lsb & 0x7f) | u16::from(msb & 0x7f) << 7;
        let delta = fader.moved(value, FADER_STEP);
        if delta != 0 {
            events.push(Input::DialTurn {
                index: self.kind.first_fader_dial() + channel,
                delta,
            });
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let [status, data1, data2] = match message {
            Message::SysEx(bytes) => return self.sysex(bytes),
            Message::Channel(bytes) => *bytes,
        };
        if status == CONTROL_CHANGE && matches!(data1, HUI_ZONE_CC | HUI_PORT_CC) {
            self.hint();
            return;
        }
        if self.phase != Phase::Live {
            // Only the MCU Pro's own, plainly named port may go live on
            // input; an extender port may carry any MIDI gear.
            let shaped = matches!(status, NOTE_ON | NOTE_OFF | CONTROL_CHANGE)
                || status & 0xf0 == PITCH_BEND;
            if self.kind == Kind::Mcu && self.named_ports && self.phase == Phase::Waiting && shaped
            {
                self.go_live();
            } else {
                return;
            }
        }
        match status {
            NOTE_ON => self.note(data1, data2 > PRESS_OVER, events),
            NOTE_OFF => self.note(data1, false, events),
            CONTROL_CHANGE => self.control(data1, data2, events),
            _ if status & 0xf0 == PITCH_BEND => self.fader(status & 0x0f, data1, data2, events),
            _ if status & 0xf0 == CHANNEL_PRESSURE => {}
            _ => self.hint(),
        }
    }
}

impl Driver for MackieMcu {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.query();
        self.pump()?;
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
        if self.lit(key) {
            if let Some(led) = self.leds.get_mut(usize::from(key)) {
                *led = is_lit(face.map(|face| face.color));
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.query();
        self.queue_changes();
        self.pump()
    }

    /// A V-Pot's LED ring, all on or off; the jog wheel and faders have no
    /// light.
    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if let Some(ring) = self.rings.get_mut(usize::from(dial)) {
            *ring = is_lit(color);
        }
        Ok(())
    }

    /// The MCU Pro has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for MackieMcu {
    /// Turns the lights off and blanks the displays at once, without
    /// pacing; anything still queued is dropped. Nothing is sent to a deck
    /// that never answered.
    fn drop(&mut self) {
        if self.phase != Phase::Live {
            return;
        }
        self.outbox.clear();
        for (_, bytes) in self.dark_messages() {
            let _ = self.transport.write(&bytes);
        }
        let blank = self.lcd_message(&[]);
        let _ = self.transport.write(&blank);
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, ModelView};

    fn open(id: &str) -> (MackieMcu, Recorder) {
        let recorder = Recorder::default();
        let deck = MackieMcu::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    fn face(color: [u8; 3]) -> Face {
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

    fn ready(device: u8) -> Vec<u8> {
        vec![0xf0, 0x00, 0x00, 0x66, device, 0x01, 0xf7]
    }

    fn no_pitch_bend(writes: &[Vec<u8>]) {
        assert!(writes.iter().all(|write| write[0] & 0xf0 != 0xe0));
    }

    /// Opens a deck and makes it live with its own ready answer.
    fn live(id: &str, device: u8) -> (MackieMcu, Recorder) {
        let (mut deck, recorder) = open(id);
        recorder.queue_read(ready(device));
        deck.poll(0).unwrap();
        (deck, recorder)
    }

    #[test]
    fn port_names_pick_the_mcu_pro_and_its_extender_ports() {
        let mcu = by_id(MCU_PRO_ID).unwrap();
        let xt = by_id(XT_PRO_ID).unwrap();
        let found = |raw: &str| identify_midi(&normalise(raw), true).map(|model| model.id);
        assert_eq!(found("MCU Pro USB v3.1"), Some(MCU_PRO_ID));
        assert_eq!(found("mcu pro usb v3.2"), Some(MCU_PRO_ID));
        assert_eq!(found("MIDIIN2 (MCU Pro USB v3.1)"), Some(XT_PRO_ID));
        assert_eq!(found("MIDIOUT4 (MCU Pro USB v3.1)"), Some(XT_PRO_ID));
        // Bitwig numbers the first extender port 1: still an extender.
        assert_eq!(found("MIDIIN1 (MCU Pro USB v3.1)"), Some(XT_PRO_ID));
        let alsa = |port: u16| PortName {
            base: "MCU Pro USB v3.1 MIDI 1".to_owned(),
            interface: None,
            alsa_port: Some(port),
        };
        assert!(port_matches(mcu, &alsa(0)) && !port_matches(xt, &alsa(0)));
        assert!(port_matches(xt, &alsa(2)) && !port_matches(mcu, &alsa(2)));
        assert!(!port_matches(xt, &alsa(4)) && !port_matches(mcu, &alsa(4)));
        assert_eq!(found("MCU Classic"), None);
        assert_eq!(ModelView::from(mcu).vendor_id, "0a73");
    }

    #[test]
    fn opening_sends_only_the_device_query_until_the_deck_answers() {
        assert!(SHORT_GAP.is_zero() && SYSEX_GAP.is_zero() && QUERY_EVERY.is_zero());
        let (mut deck, recorder) = open(MCU_PRO_ID);
        deck.set_face(0x5e, Some(&face([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0xf0, 0x00, 0x00, 0x66, 0x14, 0x00, 0xf7],
                vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x00, 0xf7],
            ]
        );
        // Asked again on the next poll (every two seconds outside tests).
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 4);
    }

    #[test]
    fn a_ready_answer_darkens_the_mcu_pro_and_greets_on_its_lcd() {
        let (mut deck, recorder) = live(MCU_PRO_ID, 0x14);
        let writes = recorder.writes();
        let after: Vec<Vec<u8>> = writes[2..].to_vec();
        let lit: Vec<u8> = (0x00..=0x1f).chain(0x28..=0x65).collect();
        assert_eq!(after.len(), lit.len() + 3 + 8 + 12 + 1);
        for (write, note) in after.iter().zip(&lit) {
            assert_eq!(write, &vec![0x90, *note, 0x00]);
        }
        let rest = &after[lit.len()..];
        assert_eq!(
            rest[..3],
            [
                vec![0x90, 0x71, 0],
                vec![0x90, 0x72, 0],
                vec![0x90, 0x73, 0]
            ]
        );
        for strip in 0..8u8 {
            assert_eq!(rest[3 + usize::from(strip)], vec![0xb0, 0x30 + strip, 0]);
        }
        for (index, cc) in (0x40..=0x4bu8).enumerate() {
            assert_eq!(rest[11 + index], vec![0xb0, cc, 0x20]);
        }
        let mut lcd = vec![0xf0, 0x00, 0x00, 0x66, 0x14, 0x12, 0x00, b'R', b'y', b'u'];
        lcd.extend([0x20; 109]);
        lcd.push(0xf7);
        assert_eq!(rest[23], lcd);
        no_pitch_bend(&writes);
        // No more queries once live.
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), writes.len());
    }

    #[test]
    fn buttons_touches_vpots_and_jog_turn_into_keys_and_dials() {
        let (mut deck, recorder) = live(MCU_PRO_ID, 0x14);
        for message in [
            [0x90, 0x5e, 0x7f],
            [0x90, 0x5e, 0x00],
            [0x90, 0x00, 0x7f],
            [0x80, 0x00, 0x40],
            [0x90, 0x68, 0x7f],
            [0x90, 0x70, 0x7f],
            [0x90, 0x71, 0x7f],
            [0xb0, 0x10, 0x01],
            [0xb0, 0x17, 0x41],
            [0xb0, 0x10, 0x00],
            [0xb0, 0x3c, 0x45],
            [0xb0, 0x2e, 0x40],
        ] {
            recorder.queue_read(message.to_vec());
        }
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0x5e, true),
                key(0x5e, false),
                key(0, true),
                key(0, false),
                key(0x68, true),
                key(112, true),
                turn(0, 1),
                turn(7, -1),
                turn(0, 1),
                turn(8, -5),
            ]
        );
    }

    #[test]
    fn faders_seed_then_step_and_are_never_written() {
        let (mut deck, recorder) = live(MCU_PRO_ID, 0x14);
        for message in [
            [0xe0, 0x00, 0x00],
            [0xe0, 0x7f, 0x00],
            [0xe0, 0x00, 0x01],
            [0xe0, 0x00, 0x03],
            [0xe8, 0x00, 0x40],
            [0xe8, 0x00, 0x3f],
            [0xe9, 0x00, 0x00],
        ] {
            recorder.queue_read(message.to_vec());
        }
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(9, 1), turn(9, 2), turn(17, -1)]
        );
        no_pitch_bend(&recorder.writes());
    }

    #[test]
    fn lights_go_only_to_lit_keys_and_rings() {
        let (mut deck, recorder) = live(MCU_PRO_ID, 0x14);
        let before = recorder.writes().len();
        for key in [0x00, 0x20, 0x5e, 0x66, 0x68] {
            deck.set_face(key, Some(&face([0, 200, 0]))).unwrap();
        }
        deck.set_dial_light(2, Some([255, 255, 255])).unwrap();
        deck.set_dial_light(9, Some([255, 255, 255])).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [
                vec![0x90, 0x00, 0x7f],
                vec![0x90, 0x5e, 0x7f],
                vec![0xb0, 0x32, 0x2b],
            ]
        );
        deck.set_dial_light(2, None).unwrap();
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[before + 3..],
            [vec![0x90, 0x00, 0x00], vec![0xb0, 0x32, 0x00]]
        );
    }

    #[test]
    fn the_logic_control_challenge_is_answered_then_confirmed() {
        assert_eq!(challenge_response([0, 0, 0, 0]), [0x0a, 0x00, 0x00, 0x70]);
        // l3 = 127 shifts past the width; nothing panics and it reads zero.
        assert_eq!(challenge_response([1, 2, 3, 127]), [0x0a, 0x00, 0x70, 0x7f]);
        let (mut deck, recorder) = open(MCU_PRO_ID);
        // The confirmation is not overdue during this test.
        deck.query_every = Duration::from_secs(60);
        let mut challenge = vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x01];
        challenge.extend([1, 2, 3, 4, 5, 6, 7]);
        challenge.extend([0x10, 0x20, 0x30, 0x02]);
        challenge.push(0xf7);
        recorder.queue_read(challenge);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        let mut reply = vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x02, 1, 2, 3, 4, 5, 6, 7];
        reply.extend(challenge_response([0x10, 0x20, 0x30, 0x02]));
        reply.push(0xf7);
        assert_eq!(writes.last(), Some(&reply));
        let count = writes.len();
        // Input before the confirmation neither presses nor lights.
        recorder.queue_read(vec![0x90, 0x00, 0x7f]);
        assert!(deck.poll(0).unwrap().is_empty());
        assert_eq!(recorder.writes().len(), count);
        let mut confirm = vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x03];
        confirm.extend([1, 2, 3, 4, 5, 6, 7]);
        confirm.push(0xf7);
        recorder.queue_read(confirm);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[count],
            vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x13, 0x00, 0xf7]
        );
        // The LCD goes out under the Logic Control id.
        assert_eq!(
            writes.last().unwrap()[..7],
            [0xf0, 0x00, 0x00, 0x66, 0x10, 0x12, 0x00]
        );
        // It wrote 0x7f to the first LED only after the confirmation, never
        // from the early press.
        assert!(writes.iter().all(|write| write != &vec![0x90, 0x00, 0x7f]));
    }

    #[test]
    fn an_unconfirmed_challenge_is_queried_again() {
        let (mut deck, recorder) = open(XT_PRO_ID);
        let mut challenge = vec![0xf0, 0x00, 0x00, 0x66, 0x11, 0x01];
        challenge.extend([0; 11]);
        challenge.push(0xf7);
        recorder.queue_read(challenge.clone());
        deck.poll(0).unwrap();
        assert_eq!(deck.phase, Phase::Challenged);
        let count = recorder.writes().len();
        // No confirmation: the query goes out again (two seconds later
        // outside tests) and a fresh challenge is answered.
        deck.poll(0).unwrap();
        assert_eq!(deck.phase, Phase::Waiting);
        assert!(deck.unconfirmed_warned);
        assert_eq!(
            recorder.writes()[count..],
            [
                vec![0xf0, 0x00, 0x00, 0x66, 0x15, 0x00, 0xf7],
                vec![0xf0, 0x00, 0x00, 0x66, 0x11, 0x00, 0xf7],
            ]
        );
        recorder.queue_read(challenge);
        deck.poll(0).unwrap();
        assert_eq!(deck.phase, Phase::Challenged);
        assert_eq!(recorder.writes().last().unwrap()[4..6], [0x11, 0x02]);
    }

    #[test]
    fn a_rebooted_deck_that_announces_itself_again_is_relit() {
        let (mut deck, recorder) = live(XT_PRO_ID, 0x15);
        deck.set_face(3, Some(&face([255, 255, 255]))).unwrap();
        deck.set_dial_light(1, Some([255, 255, 255])).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        recorder.queue_read(ready(0x15));
        deck.poll(0).unwrap();
        let again = recorder.writes()[before..].to_vec();
        // Darkened and greeted again, its light and ring resent in place of
        // their off messages.
        assert_eq!(again.len(), 32 + 8 + 1);
        assert!(again.contains(&vec![0x90, 0x03, 0x7f]) && again.contains(&vec![0xb0, 0x31, 0x2b]));
        assert!(
            !again.contains(&vec![0x90, 0x03, 0x00]) && !again.contains(&vec![0xb0, 0x31, 0x00])
        );
        assert_eq!(again[0], vec![0x90, 0x00, 0x00]);
        assert!(again
            .iter()
            .any(|write| write.get(4..6) == Some(&[0x15, 0x12][..])));
        // A fresh Logic Control challenge from a live deck is answered.
        let mut challenge = vec![0xf0, 0x00, 0x00, 0x66, 0x11, 0x01];
        challenge.extend([0; 11]);
        challenge.push(0xf7);
        recorder.queue_read(challenge);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().last().unwrap()[4..6], [0x11, 0x02]);
    }

    #[test]
    fn an_unnamed_mcu_port_takes_an_xt_pro_that_answers() {
        let (mut deck, recorder) = open(MCU_PRO_ID);
        deck.named_ports = false;
        deck.poll(0).unwrap();
        let queried: Vec<u8> = recorder.writes().iter().map(|write| write[4]).collect();
        assert_eq!(queried, vec![0x14, 0x10, 0x15, 0x11]);
        recorder.queue_read(ready(0x15));
        deck.poll(0).unwrap();
        assert_eq!(deck.phase, Phase::Live);
        // Past the second round of queries sent before the answer was read.
        let writes = recorder.writes()[8..].to_vec();
        // The XT Pro's strip lights and rings, its LCD; no globals, lamps or
        // digits.
        assert_eq!(writes.len(), 32 + 8 + 1);
        assert_eq!(writes.last().unwrap()[4..6], [0x15, 0x12]);
        let before = recorder.writes().len();
        deck.set_face(0x5e, Some(&face([255, 255, 255]))).unwrap();
        deck.set_face(0x02, Some(&face([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[before..], [vec![0x90, 0x02, 0x7f]]);
        // Its fader touch and first fader read with the MCU Pro's layout.
        for message in [[0x90, 0x68, 0x7f], [0xe0, 0x00, 0x00], [0xe0, 0x00, 0x01]] {
            recorder.queue_read(message.to_vec());
        }
        assert_eq!(deck.poll(0).unwrap(), vec![key(0x68, true), turn(9, 1)]);
        // A named MCU Pro port ignores an XT Pro's answer.
        let (mut deck, recorder) = open(MCU_PRO_ID);
        deck.named_ports = true;
        recorder.queue_read(ready(0x15));
        deck.poll(0).unwrap();
        assert_eq!(deck.phase, Phase::Waiting);
    }

    #[test]
    fn a_denied_handshake_stays_silent() {
        let (mut deck, recorder) = open(MCU_PRO_ID);
        let mut challenge = vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x01];
        challenge.extend([0; 11]);
        challenge.push(0xf7);
        recorder.queue_read(challenge);
        recorder.queue_read(vec![0xf0, 0x00, 0x00, 0x66, 0x10, 0x04, 0xf7]);
        deck.poll(0).unwrap();
        let count = recorder.writes().len();
        deck.set_face(0, Some(&face([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        drop(deck);
        assert_eq!(recorder.writes().len(), count);
    }

    #[test]
    fn the_xt_pro_stays_silent_on_pitch_bend_and_notes_until_it_answers() {
        let (mut deck, recorder) = open(XT_PRO_ID);
        deck.set_face(0, Some(&face([255, 255, 255]))).unwrap();
        for message in [[0xe0, 0x00, 0x40], [0xe0, 0x00, 0x50], [0x90, 0x68, 0x7f]] {
            recorder.queue_read(message.to_vec());
        }
        assert!(deck.poll(0).unwrap().is_empty());
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert!(writes
            .iter()
            .all(|write| write[..5] == [0xf0, 0x00, 0x00, 0x66, write[4]]
                && write[5] == 0x00
                && matches!(write[4], 0x15 | 0x11)));
        drop(deck);
        assert_eq!(recorder.writes(), writes);

        let (mut deck, recorder) = live(XT_PRO_ID, 0x15);
        let writes = recorder.writes();
        // Rec to Select off, eight rings off, the LCD; no lamps or digits.
        assert_eq!(writes.len(), 2 + 32 + 8 + 1);
        assert!(writes
            .iter()
            .all(|write| !(write[0] == 0x90 && write[1] > 0x1f)
                && !(write[0] == 0xb0 && write[1] >= 0x40)));
        assert_eq!(writes.last().unwrap()[4..6], [0x15, 0x12]);
        deck.set_face(0, Some(&face([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().last(), Some(&vec![0x90, 0x00, 0x7f]));
        for message in [
            [0x90, 0x6f, 0x7f],
            [0xe7, 0x00, 0x00],
            [0xe7, 0x00, 0x02],
            [0xe8, 0, 0],
        ] {
            recorder.queue_read(message.to_vec());
        }
        assert_eq!(deck.poll(0).unwrap(), vec![key(47, true), turn(15, 2)]);
    }

    #[test]
    fn the_mcu_pro_port_goes_live_on_input_where_ports_are_named() {
        let (mut deck, recorder) = open(MCU_PRO_ID);
        recorder.queue_read(vec![0x90, 0x5e, 0x7f]);
        let events = deck.poll(0).unwrap();
        if NAMED_PORTS {
            assert_eq!(events, vec![key(0x5e, true)]);
            assert!(recorder.writes().len() > 2);
        } else {
            assert!(events.is_empty());
        }
    }

    #[test]
    fn letting_go_darkens_and_blanks_at_once() {
        let (deck, recorder) = live(MCU_PRO_ID, 0x14);
        let before = recorder.writes().len();
        drop(deck);
        let writes = recorder.writes();
        let closing = &writes[before..];
        assert_eq!(closing.len(), 94 + 3 + 8 + 12 + 1);
        let mut blank = vec![0xf0, 0x00, 0x00, 0x66, 0x14, 0x12, 0x00];
        blank.extend([0x20; 112]);
        blank.push(0xf7);
        assert_eq!(closing.last(), Some(&blank));
        no_pitch_bend(&writes);
    }

    #[test]
    fn short_and_foreign_sysex_is_ignored() {
        let (mut deck, recorder) = open(XT_PRO_ID);
        for message in [
            vec![0xf0, 0xf7],
            vec![0xf0, 0x00, 0x00, 0x66, 0xf7],
            vec![0xf0, 0x00, 0x00, 0x66, 0x14, 0x01, 0xf7],
            vec![0xf0, 0x00, 0x20, 0x29, 0x15, 0x01, 0xf7],
            vec![0xf0, 0x00, 0x00, 0x66, 0x11, 0x01, 0xf7],
            vec![0xf0, 0x00, 0x00, 0x66, 0x15, 0x03, 0xf7],
        ] {
            recorder.queue_read(message);
        }
        deck.poll(0).unwrap();
        assert_eq!(deck.phase, Phase::Waiting);
        assert!(recorder
            .writes()
            .iter()
            .all(|write| write.get(5) == Some(&0x00)));
    }

    #[test]
    fn key_names_and_lights_follow_the_note_map() {
        let mcu = by_id(MCU_PRO_ID).unwrap();
        let xt = by_id(XT_PRO_ID).unwrap();
        let names = key_names(mcu).unwrap();
        assert_eq!(
            (names.len(), names[0x5e], names[112]),
            (113, "Play", "Master Touch")
        );
        let names = key_names(xt).unwrap();
        assert_eq!(
            (names.len(), names[39], names[40]),
            (48, "V-Pot 8", "Fader Touch 1")
        );
        assert_eq!(lit_keys(mcu).unwrap().len(), 94);
        assert_eq!(lit_keys(xt).unwrap(), (0..32).collect::<Vec<u8>>());
        assert_eq!((mcu.dial_light_colors(), xt.dial_light_colors()), (8, 8));
    }
}
