//! KORG's nanoKONTROL2 and nanoKONTROL Studio, over class-compliant USB MIDI
//! ([`super::midi`]), driven in KORG Native Mode.
//!
//! Each lists one MIDI port pair, found by name ([`port_matches`]): a name
//! with `nanoKONTROL2` in it (any case) is the nanoKONTROL2, and one with
//! `nanoKONTROL Studio` the Studio. The original 2008 nanoKONTROL (whose name
//! has neither) is never matched. Windows' class driver names the ports after
//! the device alone; KORG's driver and CoreMIDI name the input `... SLIDER/KNOB`
//! and the output `... CTRL`, which [`super::midi::normalise`] strips so the
//! two pair. Both decks have one MIDI cable, so a Windows `MIDIIN2 (...)` name
//! is not theirs.
//!
//! Open: Ryu first asks for the deck's Global MIDI channel `g` with a KORG
//! Search Device request (`F0 42 50 00 <echo> F7`; the reply is
//! `F0 42 50 01 0g <echo> <family> ... F7`, family `13 01` on the
//! nanoKONTROL2 and `37 01` on the Studio), since the deck ignores KORG SysEx
//! on another channel; with no reply within half a second it takes channel 1
//! (`g = 0`, the factory setting). The reply is read by the normal poll, so
//! the wait never holds up other decks or drops this deck's input. It then
//! enters Native Mode, which makes every control report the
//! same way whatever scene, channel or LED mode the user has set, and lets the
//! host drive the LEDs:
//!
//! - nanoKONTROL2: `F0 42 4g 00 01 13 00 00 00 01 F7` (acknowledged by
//!   `... 13 00 40 00 03 F7`), and `... 00 00 00 F7` to leave;
//! - Studio: `F0 42 4g 00 01 37 02 00 00 00 01 F7` (acknowledged by
//!   `... 37 02 00 00 01 03 F7`), and `... 00 00 F7` to leave.
//!
//! Every LED is then turned off. When Ryu lets go it leaves Native Mode, so
//! the deck returns to its scene and its own LED behaviour.
//!
//! In (Native Mode), every control on `BF` (Control Change, channel 16):
//! sliders `CC 00-07`, knobs `CC 10-17` (absolute, turned into dial steps of
//! one MIDI unit from the first value seen), and the buttons on their own
//! numbers, `7F` pressed and `00` released: Solo `20-27`, Mute `30-37`, Rec
//! `40-47`, Select `50-57` (Studio), Play `29`, Stop `2A`, Rewind `2B`,
//! Forward `2C`, Record `2D`, Cycle `2E`, Return to Zero `2F` (Studio), Track
//! ◀ ▶ `3A 3B`, Marker Set `3C` and Marker ◀ ▶ `3D 3E`. The Studio's jog wheel
//! is `CC 6E`, relative (`01-07` clockwise, `41-47` counter-clockwise), and
//! its Scene button arrives as a parameter change,
//! `F0 42 4g 00 01 37 05 00 00 41 40 40 <7F|00> 00 F7`. Any other status is
//! ignored.
//!
//! Out, each LED is `BF <cc> 7F` (on) or `BF <cc> 00` (off) on its button's
//! number, sent only when it changes. The nanoKONTROL2's Track and Marker
//! buttons have no LED; on the Studio every button but Scene has one. KORG
//! documents the Studio's Native Mode outputs only, so its LED input is
//! inferred from the nanoKONTROL2's and still to be checked on hardware.
//! There is no brightness command. Every LED is resent every 30 seconds (the
//! deck's power saving may turn its LEDs off) and after a Studio scene change.
//!
//! Facts per KORG's nanoKONTROL2 and nanoKONTROL Studio MIDI Implementation
//! documents and Parameter Guide / Owner's Manual, cross-checked against
//! Mixxx's nanoKONTROL2 mapping (GPL-2.0-or-later), the korg-nano-kontrol2 npm
//! package (MIT), node-korg-nanokontrol-studio (MIT) and Bitwig scripts by
//! benschmaus, olaims and randrej (AGPL-3.0); no code was copied.

use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const NANOKONTROL2_ID: &str = "korg-nanokontrol2";
pub const STUDIO_ID: &str = "korg-nanokontrol-studio";

pub const NK2_KEYS: u8 = 35;
pub const NK2_DIALS: u8 = 16;
pub const STUDIO_KEYS: u8 = 45;
pub const STUDIO_DIALS: u8 = 17;

const CONTROL_CHANGE_16: u8 = 0xbf;
const SYSEX_START: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
const KORG: u8 = 0x42;
/// The Search Device request and reply's second header byte.
const SEARCH: u8 = 0x50;
/// The echo id Ryu puts in its Search Device request.
const ECHO: u8 = 0x52;
const LED_ON: u8 = 0x7f;
const LED_OFF: u8 = 0x00;
/// A light that is only on or off is on for any colour brighter than this.
const ON_OFF_THRESHOLD: u8 = 16;
/// The first knob's and the first slider's controller.
const FIRST_KNOB: u8 = 0x10;
const FIRST_SLIDER: u8 = 0x00;
const STRIPS: u8 = 8;
/// The Studio's jog wheel controller, and its dial.
const JOG: u8 = 0x6e;
const JOG_DIAL: u8 = 16;
/// The Studio's Scene button, which has no controller.
const SCENE_KEY: u8 = 44;
/// The Studio's Scene Change message's function.
const SCENE_CHANGE: u8 = 0x4f;
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;
const SEARCH_WAIT: Duration = Duration::from_millis(500);
/// How often every LED is resent.
const REFRESH: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Nk2,
    Studio,
}

/// Each nanoKONTROL2 key's controller: S, M and R 1-8, then Track ◀ ▶,
/// Cycle, Marker Set, Marker ◀ ▶, Rewind, Forward, Stop, Play and Record.
const NK2_CCS: [u8; NK2_KEYS as usize] = [
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, //
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, //
    0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, //
    0x3a, 0x3b, 0x2e, 0x3c, 0x3d, 0x3e, //
    0x2b, 0x2c, 0x2a, 0x29, 0x2d,
];

/// Each Studio key's controller (the Scene button, key 44, has none): Solo,
/// Mute, Rec and Select 1-8, then Track ◀ ▶, Cycle, Marker Set, Marker ◀ ▶,
/// Return to Zero, Rewind, Forward, Stop, Play and Record.
const STUDIO_CCS: [u8; SCENE_KEY as usize] = [
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, //
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, //
    0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, //
    0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, //
    0x3a, 0x3b, 0x2e, 0x3c, 0x3d, 0x3e, //
    0x2f, 0x2b, 0x2c, 0x2a, 0x29, 0x2d,
];

/// The nanoKONTROL2's keys with no LED: Track ◀ ▶, Marker Set and Marker ◀ ▶.
const NK2_UNLIT: [u8; 5] = [24, 25, 27, 28, 29];

const NK2_NAMES: [&str; NK2_KEYS as usize] = [
    "S1",
    "S2",
    "S3",
    "S4",
    "S5",
    "S6",
    "S7",
    "S8", //
    "M1",
    "M2",
    "M3",
    "M4",
    "M5",
    "M6",
    "M7",
    "M8", //
    "R1",
    "R2",
    "R3",
    "R4",
    "R5",
    "R6",
    "R7",
    "R8", //
    "Track ◀",
    "Track ▶",
    "Cycle",
    "Set",
    "Marker ◀",
    "Marker ▶", //
    "REW",
    "FF",
    "Stop",
    "Play",
    "Rec",
];

const STUDIO_NAMES: [&str; STUDIO_KEYS as usize] = [
    "Solo 1",
    "Solo 2",
    "Solo 3",
    "Solo 4",
    "Solo 5",
    "Solo 6",
    "Solo 7",
    "Solo 8", //
    "Mute 1",
    "Mute 2",
    "Mute 3",
    "Mute 4",
    "Mute 5",
    "Mute 6",
    "Mute 7",
    "Mute 8", //
    "Rec 1",
    "Rec 2",
    "Rec 3",
    "Rec 4",
    "Rec 5",
    "Rec 6",
    "Rec 7",
    "Rec 8", //
    "Select 1",
    "Select 2",
    "Select 3",
    "Select 4",
    "Select 5",
    "Select 6",
    "Select 7",
    "Select 8", //
    "Track ◀",
    "Track ▶",
    "Cycle",
    "Set",
    "Marker ◀",
    "Marker ▶", //
    "Return to Zero",
    "REW",
    "FF",
    "Stop",
    "Play",
    "Rec",
    "Scene",
];

impl Kind {
    const fn of(protocol: Protocol) -> Option<Self> {
        match protocol {
            Protocol::KorgNanoKontrol2 => Some(Self::Nk2),
            Protocol::KorgNanoKontrolStudio => Some(Self::Studio),
            _ => None,
        }
    }

    const fn keys(self) -> u8 {
        match self {
            Self::Nk2 => NK2_KEYS,
            Self::Studio => STUDIO_KEYS,
        }
    }

    const fn dials(self) -> u8 {
        match self {
            Self::Nk2 => NK2_DIALS,
            Self::Studio => STUDIO_DIALS,
        }
    }

    const fn ccs(self) -> &'static [u8] {
        match self {
            Self::Nk2 => &NK2_CCS,
            Self::Studio => &STUDIO_CCS,
        }
    }

    /// The SysEx family code, low byte first, as the Search Device reply
    /// gives it.
    const fn family(self) -> [u8; 2] {
        match self {
            Self::Nk2 => [0x13, 0x01],
            Self::Studio => [0x37, 0x01],
        }
    }

    /// The controller that sets key `key`'s LED, if it has one.
    fn led_cc(self, key: u8) -> Option<u8> {
        if self == Self::Nk2 && NK2_UNLIT.contains(&key) {
            return None;
        }
        self.ccs().get(usize::from(key)).copied()
    }

    /// The Native Mode request: `true` enters it, `false` leaves it.
    fn native_mode(self, channel: u8, enter: bool) -> Vec<u8> {
        let header = [SYSEX_START, KORG, 0x40 | (channel & 0x0f), 0x00, 0x01];
        let body: &[u8] = match self {
            // Sub id 00, Native In/Out request (00) in the two-byte format.
            Self::Nk2 => &[0x13, 0x00, 0x00, 0x00],
            // Length 2 (three 7-bit bytes, low first), Native In/Out request.
            Self::Studio => &[0x37, 0x02, 0x00, 0x00, 0x00],
        };
        let mut message = Vec::with_capacity(header.len() + body.len() + 2);
        message.extend_from_slice(&header);
        message.extend_from_slice(body);
        message.push(u8::from(enter));
        message.push(SYSEX_END);
        message
    }

    /// Each key's (row, column), as the panel lays them out.
    fn places(self) -> Vec<(u8, u8)> {
        match self {
            Self::Nk2 => {
                // Eight strips in columns 5-12, S over M over R; the track
                // buttons top left, Cycle and the marker buttons below, and
                // the transport along the bottom.
                let mut places: Vec<(u8, u8)> = (0..3)
                    .flat_map(|row| (0..STRIPS).map(move |strip| (row, 5 + strip)))
                    .collect();
                places.extend([
                    (0, 0),
                    (0, 1),
                    (1, 0),
                    (1, 2),
                    (1, 3),
                    (1, 4),
                    (2, 0),
                    (2, 1),
                    (2, 2),
                    (2, 3),
                    (2, 4),
                ]);
                places
            }
            Self::Studio => {
                // Eight strips in columns 6-13, Mute over Solo over Rec over
                // Select (as the Owner's Manual numbers them; keys 0-7 are
                // Solo); Scene, the track and marker buttons top left, Cycle
                // below, and the transport along the bottom.
                let mut places: Vec<(u8, u8)> = [1, 0, 2, 3]
                    .into_iter()
                    .flat_map(|row| (0..STRIPS).map(move |strip| (row, 6 + strip)))
                    .collect();
                places.extend([
                    (0, 1),
                    (0, 2),
                    (1, 0),
                    (0, 3),
                    (0, 4),
                    (0, 5),
                    (3, 0),
                    (3, 1),
                    (3, 2),
                    (3, 3),
                    (3, 4),
                    (3, 5),
                    (0, 0),
                ]);
                places
            }
        }
    }
}

/// Whether a normalised port name is this model's: `nanoKONTROL2` or
/// `nanoKONTROL Studio` in any case, never the original nanoKONTROL, and
/// never a Windows second interface (both decks have one).
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    let Some(kind) = Kind::of(model.protocol) else {
        return false;
    };
    if name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let lower = name.base.to_ascii_lowercase();
    match kind {
        Kind::Nk2 => lower.contains("nanokontrol2"),
        Kind::Studio => lower.contains("nanokontrol studio"),
    }
}

/// The legend on each key, on a nanoKONTROL.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    Some(match Kind::of(model.protocol)? {
        Kind::Nk2 => NK2_NAMES.to_vec(),
        Kind::Studio => STUDIO_NAMES.to_vec(),
    })
}

/// Each key's cell in the model's grid, on a nanoKONTROL.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    let cols = u16::from(model.cols);
    Some(
        Kind::of(model.protocol)?
            .places()
            .into_iter()
            .map(|(row, col)| u16::from(row) * cols + u16::from(col))
            .collect(),
    )
}

/// The keys with an LED: all but the nanoKONTROL2's Track and Marker
/// buttons, and all but the Studio's Scene button.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    let kind = Kind::of(model.protocol)?;
    Some(
        (0..kind.keys())
            .filter(|key| kind.led_cc(*key).is_some())
            .collect(),
    )
}

const fn is_on(color: [u8; 3]) -> bool {
    color[0] > ON_OFF_THRESHOLD || color[1] > ON_OFF_THRESHOLD || color[2] > ON_OFF_THRESHOLD
}

/// The Global MIDI channel in a Search Device reply for `kind`.
fn search_reply(kind: Kind, bytes: &[u8]) -> Option<u8> {
    match bytes {
        [SYSEX_START, KORG, SEARCH, 0x01, channel, ECHO, low, high, ..]
            if [*low, *high] == kind.family() && bytes.last() == Some(&SYSEX_END) =>
        {
            Some(channel & 0x0f)
        }
        _ => None,
    }
}

/// The header of a Studio message on any Global channel, and what follows it.
fn studio_body(bytes: &[u8]) -> Option<&[u8]> {
    match bytes {
        [SYSEX_START, KORG, channel, 0x00, 0x01, 0x37, body @ .., SYSEX_END]
            if channel & 0xf0 == 0x40 =>
        {
            Some(body)
        }
        _ => None,
    }
}

/// The Studio's Scene button in Native Mode: whether it is pressed.
fn scene_button(bytes: &[u8]) -> Option<bool> {
    match studio_body(bytes)? {
        [0x05, 0x00, 0x00, 0x41, 0x40, 0x40, state, 0x00] => Some(*state > 0),
        _ => None,
    }
}

/// Whether a Studio message is a Scene Change, which turns its LEDs off.
fn is_scene_change(bytes: &[u8]) -> bool {
    matches!(
        studio_body(bytes),
        Some([0x02, 0x00, 0x00, SCENE_CHANGE, _])
    )
}

pub struct Korg {
    kind: Kind,
    transport: Box<dyn Transport>,
    /// In Native Mode with every LED off, as far as Ryu knows.
    started: bool,
    /// The deck's Global MIDI channel (0 to 15), from its Search Device reply.
    channel: u8,
    /// Whether each key's LED should be on.
    leds: Vec<bool>,
    /// What each key's LED was last sent; `None` until it is (and always for
    /// keys with no LED).
    sent: Vec<Option<bool>>,
    held: Vec<bool>,
    /// Each absolute dial's last value; `None` until one arrives.
    dials: Vec<Option<u8>>,
    parser: Parser,
    /// When every LED was last sent.
    refreshed: Instant,
    /// When the Search Device request went out, while its reply is awaited.
    searching: Option<Instant>,
    /// How long to wait for the Search Device reply.
    search_wait: Duration,
}

impl Korg {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = Kind::of(model.protocol).unwrap_or(Kind::Nk2);
        let keys = usize::from(kind.keys());
        Self {
            kind,
            transport,
            started: false,
            channel: 0,
            leds: vec![false; keys],
            sent: vec![None; keys],
            held: vec![false; keys],
            dials: vec![None; usize::from(kind.dials())],
            parser: Parser::default(),
            refreshed: Instant::now(),
            searching: None,
            search_wait: SEARCH_WAIT,
        }
    }

    /// Sends the Search Device request once, without waiting for its reply:
    /// [`Driver::poll`] picks the reply up, so other decks keep being served.
    /// With no reply within `search_wait` the deck starts on channel 1 (the
    /// factory setting). Returns whether the deck is started.
    fn ensure_started(&mut self) -> DriverResult<bool> {
        if self.started {
            return Ok(true);
        }
        let asked = match self.searching {
            Some(asked) => asked,
            None => {
                self.transport
                    .write(&[SYSEX_START, KORG, SEARCH, 0x00, ECHO, SYSEX_END])?;
                let now = Instant::now();
                self.searching = Some(now);
                now
            }
        };
        if asked.elapsed() >= self.search_wait {
            tracing::debug!(
                "ryu-decks: the nanoKONTROL did not answer its search; using channel 1"
            );
            self.start(0)?;
        }
        Ok(self.started)
    }

    /// Enters Native Mode on Global channel `channel` and turns every LED off.
    fn start(&mut self, channel: u8) -> DriverResult<()> {
        self.searching = None;
        self.channel = channel;
        self.transport
            .write(&self.kind.native_mode(self.channel, true))?;
        self.sent.fill(None);
        for key in 0..self.kind.keys() {
            if let Some(cc) = self.kind.led_cc(key) {
                self.transport.write(&[CONTROL_CHANGE_16, cc, LED_OFF])?;
                if let Some(sent) = self.sent.get_mut(usize::from(key)) {
                    *sent = Some(false);
                }
            }
        }
        self.dials.fill(None);
        self.refreshed = Instant::now();
        self.started = true;
        Ok(())
    }

    /// Sends every LED again on the next flush.
    fn forget_leds(&mut self) {
        self.sent.fill(None);
    }

    /// Handles one message; returns whether every LED must be resent.
    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> bool {
        let [status, number, value] = match message {
            Message::SysEx(bytes) => {
                if self.kind != Kind::Studio {
                    return false;
                }
                if let Some(pressed) = scene_button(bytes) {
                    self.press(SCENE_KEY, pressed, events);
                    return false;
                }
                return is_scene_change(bytes);
            }
            Message::Channel(bytes) => *bytes,
        };
        if status != CONTROL_CHANGE_16 {
            return false;
        }
        if let Some(key) = self.kind.ccs().iter().position(|cc| *cc == number) {
            if let Ok(key) = u8::try_from(key) {
                self.press(key, value > 0, events);
            }
        } else if self.kind == Kind::Studio && number == JOG {
            let steps = i8::try_from(value & 0x3f).unwrap_or(0);
            let delta = if value & 0x40 == 0 { steps } else { -steps };
            if delta != 0 {
                events.push(Input::DialTurn {
                    index: JOG_DIAL,
                    delta,
                });
            }
        } else if let Some(at) = number.checked_sub(FIRST_KNOB).filter(|at| *at < STRIPS) {
            self.turn(at, value, events);
        } else if let Some(at) = number.checked_sub(FIRST_SLIDER).filter(|at| *at < STRIPS) {
            self.turn(STRIPS + at, value, events);
        }
        false
    }

    fn press(&mut self, key: u8, pressed: bool, events: &mut Vec<Input>) {
        let Some(held) = self.held.get_mut(usize::from(key)) else {
            return;
        };
        if *held == pressed {
            return;
        }
        *held = pressed;
        events.push(Input::Key {
            index: key,
            pressed,
        });
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
}

impl Driver for Korg {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let was_started = self.started;
        if !was_started {
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
        let mut resend = false;
        for message in &messages {
            if !self.started {
                if let Message::SysEx(bytes) = message {
                    if let Some(channel) = search_reply(self.kind, bytes) {
                        self.start(channel)?;
                        continue;
                    }
                }
            }
            resend |= self.receive(message, &mut events);
        }
        if resend {
            self.forget_leds();
        }
        // Waits out the search, or sends the LEDs staged before it ended.
        if resend || !was_started {
            self.flush()?;
        }
        Ok(events)
    }

    /// Stages a key's LED: on for any colour that is not dark. Keys with no
    /// LED keep nothing.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if self.kind.led_cc(key).is_none() {
            return Ok(());
        }
        if let Some(led) = self.leds.get_mut(usize::from(key)) {
            *led = face.is_some_and(|face| is_on(face.color));
        }
        Ok(())
    }

    /// One `BF <cc> 7F/00` for each LED that changed.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.ensure_started()? {
            return Ok(());
        }
        for key in 0..self.kind.keys() {
            let Some(cc) = self.kind.led_cc(key) else {
                continue;
            };
            let index = usize::from(key);
            let (Some(&on), Some(sent)) = (self.leds.get(index), self.sent.get_mut(index)) else {
                continue;
            };
            if *sent != Some(on) {
                let value = if on { LED_ON } else { LED_OFF };
                self.transport.write(&[CONTROL_CHANGE_16, cc, value])?;
                *sent = Some(on);
            }
        }
        Ok(())
    }

    /// The nanoKONTROLs have no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }

    /// Resends every LED every 30 seconds, in case the deck's power saving
    /// turned them off.
    fn heartbeat(&mut self) -> DriverResult<()> {
        if self.started && self.refreshed.elapsed() >= REFRESH {
            self.refreshed = Instant::now();
            self.forget_leds();
            self.flush()?;
        }
        Ok(())
    }
}

impl Drop for Korg {
    /// Leaves Native Mode, so the deck returns to its scene and its own LEDs.
    fn drop(&mut self) {
        if self.started {
            let _ = self
                .transport
                .write(&self.kind.native_mode(self.channel, false));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::{normalise, pair_ports};
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, Family, ModelView};

    /// Opens a deck that does not wait for its Search Device reply.
    fn open(id: &str) -> (Korg, Recorder) {
        let recorder = Recorder::default();
        let mut deck = Korg::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        deck.search_wait = Duration::ZERO;
        (deck, recorder)
    }

    /// Opens a deck and clears what opening it sent.
    fn started(id: &str) -> (Korg, Recorder) {
        let (mut deck, recorder) = open(id);
        deck.flush().unwrap();
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

    #[test]
    fn opening_the_nanokontrol2_searches_enters_native_mode_and_darkens() {
        let (mut deck, recorder) = open(NANOKONTROL2_ID);
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[0], [0xf0, 0x42, 0x50, 0x00, 0x52, 0xf7]);
        assert_eq!(
            writes[1],
            [0xf0, 0x42, 0x40, 0x00, 0x01, 0x13, 0x00, 0x00, 0x00, 0x01, 0xf7]
        );
        let leds = &writes[2..];
        assert_eq!(leds.len(), 30);
        assert!(leds.iter().all(|led| led[0] == 0xbf && led[2] == 0x00));
        assert_eq!(leds[0], [0xbf, 0x20, 0x00]);
        assert_eq!(leds[29], [0xbf, 0x2d, 0x00]);
        // No LED for Track or Marker.
        for cc in [0x3a, 0x3b, 0x3c, 0x3d, 0x3e] {
            assert!(leds.iter().all(|led| led[1] != cc));
        }
        // Nothing changed since, so nothing more is sent.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 32);
    }

    #[test]
    fn opening_the_studio_uses_its_own_framing_and_darkens_44_leds() {
        let (mut deck, recorder) = open(STUDIO_ID);
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[0], [0xf0, 0x42, 0x50, 0x00, 0x52, 0xf7]);
        assert_eq!(
            writes[1],
            [0xf0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x02, 0x00, 0x00, 0x00, 0x01, 0xf7]
        );
        assert_eq!(writes.len(), 2 + 44);
        assert!(writes.contains(&vec![0xbf, 0x57, 0x00]));
        assert!(writes.contains(&vec![0xbf, 0x2f, 0x00]));
    }

    #[test]
    fn the_global_channel_from_the_search_reply_frames_later_sysex() {
        let (mut deck, recorder) = open(NANOKONTROL2_ID);
        deck.search_wait = SEARCH_WAIT;
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 1);
        queue(
            &recorder,
            &[
                // Another deck's reply is not this one's.
                &[
                    0xf0, 0x42, 0x50, 0x01, 0x05, 0x52, 0x37, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00,
                    0x00, 0xf7,
                ],
                &[
                    0xf0, 0x42, 0x50, 0x01, 0x03, 0x52, 0x13, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00,
                    0x00, 0xf7,
                ],
            ],
        );
        assert!(deck.poll(0).unwrap().is_empty());
        assert_eq!(
            recorder.writes()[1],
            [0xf0, 0x42, 0x43, 0x00, 0x01, 0x13, 0x00, 0x00, 0x00, 0x01, 0xf7]
        );
        drop(deck);
        assert_eq!(
            recorder.writes().last().unwrap(),
            &[0xf0, 0x42, 0x43, 0x00, 0x01, 0x13, 0x00, 0x00, 0x00, 0x00, 0xf7]
        );
    }

    #[test]
    fn waiting_for_the_search_reply_never_blocks_or_drops_input() {
        let (mut deck, recorder) = open(NANOKONTROL2_ID);
        deck.search_wait = SEARCH_WAIT;
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        let begun = Instant::now();
        deck.flush().unwrap();
        // Only the request goes out, and nothing waits for its reply.
        assert_eq!(
            recorder.writes(),
            vec![vec![0xf0, 0x42, 0x50, 0x00, 0x52, 0xf7]]
        );
        assert!(deck.poll(0).unwrap().is_empty());
        deck.flush().unwrap();
        assert!(begun.elapsed() < SEARCH_WAIT);
        assert_eq!(recorder.writes().len(), 1);
        // A press before the reply still counts.
        queue(&recorder, &[&[0xbf, 0x20, 0x7f]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(0, true)]);
        // With no reply in time the deck starts on channel 1, and the light
        // staged meanwhile is sent.
        deck.search_wait = Duration::ZERO;
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(
            writes[1],
            [0xf0, 0x42, 0x40, 0x00, 0x01, 0x13, 0x00, 0x00, 0x00, 0x01, 0xf7]
        );
        assert_eq!(writes.last().unwrap(), &[0xbf, 0x20, 0x7f]);
        assert_eq!(writes.len(), 2 + 30 + 1);
    }

    #[test]
    fn lights_are_sent_on_change_and_never_for_unlit_keys() {
        let (mut deck, recorder) = started(NANOKONTROL2_ID);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(33, Some(&lit([0, 0, 200]))).unwrap();
        deck.set_face(1, Some(&lit([8, 8, 8]))).unwrap();
        deck.set_face(24, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(27, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![vec![0xbf, 0x20, 0x7f], vec![0xbf, 0x29, 0x7f]]
        );
        deck.set_face(0, None).unwrap();
        deck.set_face(33, Some(&lit([0, 0, 200]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().last().unwrap(), &[0xbf, 0x20, 0x00]);
        assert_eq!(recorder.writes().len(), 3);
        // Out of range is ignored.
        deck.set_face(200, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 3);
    }

    #[test]
    fn nanokontrol2_buttons_and_dials_map_on_channel_16() {
        let (mut deck, recorder) = started(NANOKONTROL2_ID);
        queue(
            &recorder,
            &[
                &[0xbf, 0x20, 0x7f],
                &[0xbf, 0x3a, 0x7f],
                &[0xbf, 0x3a, 0x00],
                &[0xbf, 0x2d, 0x7f],
                // CC mode traffic on channel 1 is not Native Mode's.
                &[0xb0, 0x21, 0x7f],
                &[0x9f, 0x22, 0x7f],
                &[0xbf, 0x10, 0x40],
                &[0xbf, 0x10, 0x45],
                &[0xbf, 0x07, 0x7f],
                &[0xbf, 0x07, 0x00],
                &[0xbf, 0x07, 0x00],
                // Unknown controllers and short messages.
                &[0xbf, 0x6e, 0x01],
                &[0xbf, 0x50, 0x7f],
                &[0xbf],
                &[],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(24, true),
                key(24, false),
                key(34, true),
                turn(0, 5),
                turn(15, -127),
            ]
        );
    }

    #[test]
    fn studio_jog_select_and_scene_button() {
        let (mut deck, recorder) = started(STUDIO_ID);
        queue(
            &recorder,
            &[
                &[0xbf, 0x6e, 0x43],
                &[0xbf, 0x6e, 0x05],
                &[0xbf, 0x6e, 0x00],
                &[0xbf, 0x57, 0x7f],
                &[0xbf, 0x2f, 0x7f],
                &[0xbf, 0x2d, 0x7f],
                &[
                    0xf0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x05, 0x00, 0x00, 0x41, 0x40, 0x40, 0x7f,
                    0x00, 0xf7,
                ],
                &[
                    0xf0, 0x42, 0x45, 0x00, 0x01, 0x37, 0x05, 0x00, 0x00, 0x41, 0x40, 0x40, 0x00,
                    0x00, 0xf7,
                ],
                // Truncated and foreign SysEx.
                &[0xf0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x05, 0xf7],
                &[
                    0xf0, 0x42, 0x40, 0x00, 0x01, 0x13, 0x05, 0x00, 0x00, 0x41, 0x40, 0x40, 0x7f,
                    0x00, 0xf7,
                ],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(16, -3),
                turn(16, 5),
                key(31, true),
                key(38, true),
                key(43, true),
                key(44, true),
                key(44, false),
            ]
        );
    }

    #[test]
    fn a_studio_scene_change_resends_every_led() {
        let (mut deck, recorder) = started(STUDIO_ID);
        deck.set_face(5, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        queue(
            &recorder,
            &[&[
                0xf0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x02, 0x00, 0x00, 0x4f, 0x02, 0xf7,
            ]],
        );
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(writes.len(), 44);
        assert!(writes.contains(&vec![0xbf, 0x25, 0x7f]));
        // The heartbeat waits its 30 seconds.
        deck.heartbeat().unwrap();
        assert_eq!(recorder.writes().len(), 44);
    }

    #[test]
    fn letting_go_leaves_native_mode() {
        let (deck, recorder) = started(STUDIO_ID);
        drop(deck);
        assert_eq!(
            recorder.writes(),
            vec![vec![
                0xf0, 0x42, 0x40, 0x00, 0x01, 0x37, 0x02, 0x00, 0x00, 0x00, 0x00, 0xf7
            ]]
        );
        // A deck never started sends nothing.
        let (deck, recorder) = open(NANOKONTROL2_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn ports_are_found_by_name() {
        let cases = [
            ("nanoKONTROL2", true, Some(NANOKONTROL2_ID)),
            (
                "nanoKONTROL2:nanoKONTROL2 MIDI 1 24:0",
                false,
                Some(NANOKONTROL2_ID),
            ),
            (
                "nanoKONTROL2:nanoKONTROL2 _ CTRL 24:0",
                false,
                Some(NANOKONTROL2_ID),
            ),
            ("nanoKONTROL2 SLIDER/KNOB", false, Some(NANOKONTROL2_ID)),
            ("nanoKONTROL2 _ CTRL", false, Some(NANOKONTROL2_ID)),
            ("2- nanoKONTROL2", true, Some(NANOKONTROL2_ID)),
            ("nanoKONTROL Studio", true, Some(STUDIO_ID)),
            ("nanoKONTROL Studio _ SLIDER/KNO", true, Some(STUDIO_ID)),
            (
                "nanoKONTROL Studio:nanoKONTROL Studio MIDI 1 28:0",
                false,
                Some(STUDIO_ID),
            ),
            // A second interface, which neither deck has.
            ("MIDIIN2 (nanoKONTROL2)", true, None),
            // The original nanoKONTROL is not supported.
            ("nanoKONTROL", true, None),
            ("nanoKONTROL _ CTRL", false, None),
            ("nanoKONTROL:nanoKONTROL MIDI 1 24:0", false, None),
            ("nanoKEY2", true, None),
        ];
        for (raw, windows, expected) in cases {
            assert_eq!(
                identify_midi(&normalise(raw), windows).map(|model| model.id),
                expected,
                "{raw}"
            );
        }
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn korg_driver_ports_pair_by_their_shared_name() {
        let found = pair_ports(
            &strings(&["nanoKONTROL2 SLIDER/KNOB"]),
            &strings(&["nanoKONTROL2 CTRL"]),
            false,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].model.id, NANOKONTROL2_ID);
        let found = pair_ports(
            &strings(&["nanoKONTROL Studio _ SLIDER/KNO"]),
            &strings(&["nanoKONTROL Studio _ CTRL"]),
            true,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].model.id, STUDIO_ID);
        let found = pair_ports(
            &strings(&["nanoKONTROL2:nanoKONTROL2 _ CTRL 24:0"]),
            &strings(&["nanoKONTROL2:nanoKONTROL2 _ CTRL 24:0"]),
            false,
        );
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn rows_name_place_and_light_every_key() {
        for (id, keys, dials, unlit) in [
            (NANOKONTROL2_ID, 35, 16, vec![24, 25, 27, 28, 29]),
            (STUDIO_ID, 45, 17, vec![44]),
        ] {
            let model = by_id(id).unwrap();
            assert_eq!(model.family, Family::Korg);
            assert!(model.protocol.is_midi() && !model.protocol.is_serial());
            assert!(!model.protocol.has_brightness());
            let view = ModelView::from(model);
            assert_eq!((view.keys, view.dials), (keys, dials));
            assert_eq!(view.key_names.as_ref().unwrap().len(), usize::from(keys));
            let cells = view.key_cells.clone().unwrap();
            assert_eq!(cells.len(), usize::from(keys));
            let cell_count = u16::from(view.rows) * u16::from(view.cols);
            assert!(cells.iter().all(|cell| *cell < cell_count), "{id}");
            let mut distinct = cells.clone();
            distinct.sort_unstable();
            distinct.dedup();
            assert_eq!(distinct.len(), cells.len(), "{id}");
            let lit = view.lit_keys.clone().unwrap();
            assert_eq!(lit.len(), usize::from(keys) - unlit.len());
            assert!(unlit.iter().all(|key| !lit.contains(key)));
            assert_eq!(view.key_light_colors, "on_off");
            assert_eq!(view.dial_lights, DialLights::None);
            assert!(view.key_lights && !view.brightness);
        }
        let nk2 = ModelView::from(by_id(NANOKONTROL2_ID).unwrap());
        let cells = nk2.key_cells.unwrap();
        assert_eq!((cells[0], cells[8], cells[16], cells[24]), (5, 18, 31, 0));
        let studio = ModelView::from(by_id(STUDIO_ID).unwrap());
        assert_eq!(studio.key_cells.unwrap()[44], 0);
    }
}
