//! KORG's padKONTROL, over class-compliant USB MIDI ([`super::midi`]), driven
//! in KORG Native Mode.
//!
//! The deck lists three MIDI inputs (its MIDI IN jack, PORT A and PORT B) and
//! two outputs (its MIDI OUT jack and CTRL). Native Mode runs on the deck's
//! second port both ways: Ryu reads PORT A and writes CTRL, which KORG's
//! manual and CoreMIDI call `padKONTROL Port 2` in both directions (Ableton
//! looks for `padKONTROL (Port 2)`). [`port_matches`] takes:
//!
//! - a name containing `padKONTROL` and `Port 2` (any case, any spacing or
//!   parentheses), as KORG's manual and Ableton name it;
//! - a name ending `padKONTROL <n>` once [`super::midi::normalise`] has
//!   dropped KORG's ` PORT A` / ` CTRL` jack names, as JACK lists them
//!   (`padKONTROL 1 PORT A` / `padKONTROL 1 CTRL`; under ALSA with a
//!   `padKONTROL:` client prefix);
//! - inferred from other decks and untested: Windows' second interface
//!   (`MIDIIN2 (padKONTROL)` / `MIDIOUT2 (padKONTROL)`) and ALSA sequencer
//!   port 1 of a name containing `padKONTROL`.
//!
//! A bare `padKONTROL` is refused: under Windows' class driver that is the
//! MIDI IN / OUT jacks, and a host that named the ports `padKONTROL PORT A` /
//! `padKONTROL CTRL` with no number (no reference says one does) would be
//! refused with it. PORT B, MIDI IN and MIDI OUT are never taken.
//!
//! Open never blocks the poll loop. Ryu sends the Universal Device Inquiry
//! `F0 7E 7F 06 01 F7` and reads the deck's Global MIDI channel `g` from the
//! reply (`F0 7E 0g 06 02 42 6E 00 08 00 ... F7`), since the deck ignores KORG
//! SysEx on another channel; with no reply within half a second it takes
//! `g = 0`. The reply is read by the normal poll. Then, on the header
//! `F0 42 4g 6E 08`:
//!
//! - Native Mode in, `00 00 01` (acknowledged by `40 00 03`);
//! - packet 1, `3F 2A 00` and 41 data bytes: the Global channel, then every
//!   normal-MIDI transmit bit off (X-Y, pads), every pad channel 0 and the pad
//!   notes 1-16 (acknowledged by `5F 00 00`);
//! - packet 2, `3F 0A 01` and 9 data bytes: every LED off, no decimal point
//!   and three spaces on the display (acknowledged by `5F 01 00`).
//!
//! In, every Native Mode event is `F0 42 4g 6E 08 cc aa bb F7`:
//!
//! - `45`: pad `aa & 0F` (keys 0-15), pressed when `aa & 40` is set; the
//!   velocity `bb` is 64 on release, so it is never used;
//! - `48`: switch `aa`, `7F` pressed and `00` released: `00-12` are the 19
//!   buttons (keys 16-34), `20` the X-Y pad touch (key 35);
//! - `43 00 dd`: the data-entry encoder, `dd` a 7-bit two's-complement step
//!   (dial 0);
//! - `49 0n dd`: Knob 1 or 2, absolute (dials 1 and 2; KORG prints this
//!   header with `02` for `08`, so both are taken);
//! - `4B xx yy`: the X-Y pad, absolute (dials 3 and 4, seeded again on each
//!   touch so a new touch does not jump);
//! - `47 00 dd`: the pedal jack, pressed at 64 and above (key 36);
//! - `40` and `5F 00/01 rr` acknowledgements, logged at debug level when they
//!   report an error; other `5F` functions are ignored.
//!
//! Absolute controls are turned into dial steps of one MIDI unit from the
//! first value seen. Anything else is ignored.
//!
//! Out, each LED is `01 nn et`, `nn` the key's number (pads `00-0F`, buttons
//! `10-22` in switch order) and `et` `20` on or `00` off, sent only when it
//! changes. The 7-segment display is never driven beyond the blank at open.
//! There is no brightness command. Every LED is resent (as one packet 2)
//! every 30 seconds. When Ryu lets go it turns every LED off and leaves Native
//! Mode (`00 00 00`), so the deck returns to its scene.
//!
//! Facts per KORG's padKONTROL MIDI Implementation (Native Mode) and Owner's
//! Manual, stuartkeith's padkontrol (MIT) and Ableton Live's padKONTROL remote
//! script (decompiled, no licence; facts only); no code was copied.

use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const PADKONTROL_ID: &str = "korg-padkontrol";

pub const ROWS: u8 = 6;
pub const COLS: u8 = 8;
pub const KEYS: u8 = 37;
pub const DIALS: u8 = 5;

const SYSEX_START: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
const KORG: u8 = 0x42;
const PADKONTROL: u8 = 0x6e;
/// The Native Mode sub id.
const NATIVE: u8 = 0x08;
/// The sub id KORG prints on the knob message (the KONTROL49's).
const KONTROL49: u8 = 0x02;
const UNIVERSAL: u8 = 0x7e;
const INQUIRY: [u8; 6] = [SYSEX_START, UNIVERSAL, 0x7f, 0x06, 0x01, SYSEX_END];

const NATIVE_MODE: u8 = 0x00;
const LED: u8 = 0x01;
const PACKET: u8 = 0x3f;
const LED_ON: u8 = 0x20;
const LED_OFF: u8 = 0x00;

const ENCODER: u8 = 0x43;
const PAD: u8 = 0x45;
const PEDAL: u8 = 0x47;
const SWITCH: u8 = 0x48;
const KNOB: u8 = 0x49;
const XY: u8 = 0x4b;
const NATIVE_ACK: u8 = 0x40;
const PACKET_ACK: u8 = 0x5f;

/// The first button's key; button `n` (switch `n`) is key `FIRST_BUTTON + n`.
const FIRST_BUTTON: u8 = 16;
const BUTTONS: u8 = 19;
/// The X-Y pad touch switch, its key, and the pedal's key: neither has a light.
const XY_TOUCH: u8 = 0x20;
const XY_TOUCH_KEY: u8 = 35;
const PEDAL_KEY: u8 = 36;
/// Keys 0-34 are lit, each on the LED of its own number.
const LIT: u8 = 35;
const PAD_COUNT: u8 = 16;

const ENCODER_DIAL: u8 = 0;
const FIRST_KNOB_DIAL: u8 = 1;
const X_DIAL: u8 = 3;
const Y_DIAL: u8 = 4;

/// A light that is only on or off is on for any colour brighter than this.
const ON_OFF_THRESHOLD: u8 = 16;
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;
/// How long to wait for the Device Inquiry reply: none under test.
const INQUIRY_WAIT: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(500)
};
/// How often every LED is resent.
const REFRESH: Duration = Duration::from_secs(30);

/// Each key's legend: pads 1-16, the buttons in switch order, the X-Y pad's
/// touch and the pedal jack.
const NAMES: [&str; KEYS as usize] = [
    "Pad 1",
    "Pad 2",
    "Pad 3",
    "Pad 4",
    "Pad 5",
    "Pad 6",
    "Pad 7",
    "Pad 8",
    "Pad 9",
    "Pad 10",
    "Pad 11",
    "Pad 12",
    "Pad 13",
    "Pad 14",
    "Pad 15",
    "Pad 16",
    "Scene",
    "Message / Exit",
    "Setting / Enter",
    "Note / CC#",
    "MIDI Ch",
    "SW Type",
    "Rel. Val.",
    "Velocity",
    "Port",
    "Fixed Velocity",
    "Prog Change",
    "X",
    "Y",
    "Knob 1 Assign",
    "Knob 2 Assign",
    "Pedal",
    "Roll",
    "Flam",
    "Hold",
    "X-Y Touch",
    "Footswitch",
];

/// Each button's (row, column), in switch order, as the panel lays them out
/// (approximately): Setting at the top, the setting buttons in three columns
/// on the left, Scene to Hold in one column beside the pads, Flam and Roll
/// left of Hold, and the knob assign buttons above the pads.
const BUTTON_PLACES: [(u8, u8); BUTTONS as usize] = [
    (1, 3), // Scene
    (2, 3), // Message / Exit
    (0, 2), // Setting / Enter
    (2, 0), // Note / CC#
    (2, 1), // MIDI Ch
    (2, 2), // SW Type
    (3, 0), // Rel. Val.
    (3, 1), // Velocity
    (3, 2), // Port
    (3, 3), // Fixed Velocity
    (4, 3), // Prog Change
    (1, 0), // X
    (1, 1), // Y
    (0, 5), // Knob 1 Assign
    (0, 6), // Knob 2 Assign
    (1, 2), // Pedal
    (5, 1), // Roll
    (5, 2), // Flam
    (5, 3), // Hold
];

/// Whether a normalised port name is the padKONTROL's Native Mode port (PORT
/// A in, CTRL out; `Port 2` both ways).
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.protocol != Protocol::KorgPadKontrol {
        return false;
    }
    let lower = name.base.to_ascii_lowercase();
    if !lower.contains("padkontrol") {
        return false;
    }
    // Windows' class driver: interface 2 is PORT A / CTRL (inferred).
    if let Some(interface) = name.interface {
        return interface == 2;
    }
    // ALSA sequencer port 1 carries PORT A and CTRL (inferred).
    if name.alsa_port == Some(1) {
        return true;
    }
    if lower.contains("port 2") {
        return true;
    }
    // `padKONTROL <n>`, left once the jack name went.
    let head = lower.trim_end_matches(|c: char| c.is_ascii_digit());
    head.len() < lower.len() && head.ends_with("padkontrol ")
}

/// The legend on each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::KorgPadKontrol).then(|| NAMES.to_vec())
}

/// Each key's cell: the pads in a 4 x 4 block on the right (pad 1 top left),
/// the buttons as [`BUTTON_PLACES`], the X-Y touch and the pedal bottom left.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    if model.protocol != Protocol::KorgPadKontrol {
        return None;
    }
    let cols = u16::from(model.cols);
    let pads = (0..PAD_COUNT).map(|pad| (1 + pad / 4, 4 + pad % 4));
    let rest = [(4, 0), (5, 0)];
    Some(
        pads.chain(BUTTON_PLACES)
            .chain(rest)
            .map(|(row, col)| u16::from(row) * cols + u16::from(col))
            .collect(),
    )
}

/// The keys with an LED: the pads and the buttons, never the X-Y touch or
/// the pedal.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    (model.protocol == Protocol::KorgPadKontrol).then(|| (0..LIT).collect())
}

const fn is_on(color: [u8; 3]) -> bool {
    color[0] > ON_OFF_THRESHOLD || color[1] > ON_OFF_THRESHOLD || color[2] > ON_OFF_THRESHOLD
}

/// The Global MIDI channel in a Device Inquiry reply from a padKONTROL.
fn inquiry_reply(bytes: &[u8]) -> Option<u8> {
    match bytes {
        [SYSEX_START, UNIVERSAL, channel, 0x06, 0x02, KORG, PADKONTROL, 0x00, NATIVE, 0x00, .., SYSEX_END]
            if channel & 0xf0 == 0 =>
        {
            Some(channel & 0x0f)
        }
        _ => None,
    }
}

/// A Native Mode event's command and its two data bytes.
fn native_event(bytes: &[u8]) -> Option<(u8, u8, u8)> {
    match bytes {
        [SYSEX_START, KORG, channel, PADKONTROL, sub, command, first, second, SYSEX_END]
            if channel & 0xf0 == 0x40
                && (*sub == NATIVE || (*sub == KONTROL49 && *command == KNOB)) =>
        {
            Some((*command, *first, *second))
        }
        _ => None,
    }
}

/// A 7-bit two's-complement step.
fn signed(value: u8) -> i8 {
    let value = i16::from(value & 0x7f);
    let value = if value >= 0x40 { value - 0x80 } else { value };
    i8::try_from(value).unwrap_or(0)
}

pub struct PadKontrol {
    transport: Box<dyn Transport>,
    /// In Native Mode with every LED off, as far as Ryu knows.
    started: bool,
    /// The deck's Global MIDI channel (0 to 15), from its inquiry reply.
    channel: u8,
    /// Whether each lit key's LED should be on.
    leds: [bool; LIT as usize],
    /// What each LED was last sent; `None` until it is.
    sent: [Option<bool>; LIT as usize],
    held: [bool; KEYS as usize],
    /// Each absolute dial's last value; `None` until one arrives.
    dials: [Option<u8>; DIALS as usize],
    parser: Parser,
    /// When every LED was last sent.
    refreshed: Instant,
    /// When the inquiry went out, while its reply is awaited.
    asking: Option<Instant>,
    /// How long to wait for the inquiry's reply.
    inquiry_wait: Duration,
}

impl PadKontrol {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            channel: 0,
            leds: [false; LIT as usize],
            sent: [None; LIT as usize],
            held: [false; KEYS as usize],
            dials: [None; DIALS as usize],
            parser: Parser::default(),
            refreshed: Instant::now(),
            asking: None,
            inquiry_wait: INQUIRY_WAIT,
        }
    }

    /// A Native Mode message on the deck's Global channel.
    fn native(&self, body: &[u8]) -> Vec<u8> {
        let mut message = Vec::with_capacity(body.len() + 6);
        message.extend_from_slice(&[SYSEX_START, KORG, 0x40 | self.channel, PADKONTROL, NATIVE]);
        message.extend_from_slice(body);
        message.push(SYSEX_END);
        message
    }

    /// Packet 1: every normal-MIDI transmit bit off, pad channels 0 and pad
    /// notes 1-16.
    fn packet_1(&self) -> Vec<u8> {
        let mut body = vec![PACKET, 0x2a, 0x00, self.channel];
        body.extend_from_slice(&[0; 24]);
        body.extend(1..=PAD_COUNT);
        self.native(&body)
    }

    /// Packet 2: every LED as `leds`, no decimal point, three spaces.
    fn packet_2(&self, leds: &[bool; LIT as usize]) -> Vec<u8> {
        let mut bits = [0u8; 5];
        for (index, _) in leds.iter().enumerate().filter(|(_, on)| **on) {
            if let Some(byte) = bits.get_mut(index / 7) {
                *byte |= 1 << (index % 7);
            }
        }
        let mut body = vec![PACKET, 0x0a, 0x01];
        body.extend_from_slice(&bits);
        body.extend_from_slice(&[0x00, b' ', b' ', b' ']);
        self.native(&body)
    }

    /// Sends the inquiry once, without waiting for its reply: [`Driver::poll`]
    /// picks it up, so other decks keep being served. With no reply within
    /// `inquiry_wait` the deck starts on channel 1. Returns whether the deck
    /// is started.
    fn ensure_started(&mut self) -> DriverResult<bool> {
        if self.started {
            return Ok(true);
        }
        let asked = if let Some(asked) = self.asking {
            asked
        } else {
            self.transport.write(&INQUIRY)?;
            let now = Instant::now();
            self.asking = Some(now);
            now
        };
        if asked.elapsed() >= self.inquiry_wait {
            tracing::debug!(
                "ryu-decks: the padKONTROL did not answer its inquiry; using channel 1"
            );
            self.start(0)?;
        }
        Ok(self.started)
    }

    /// Enters Native Mode on Global channel `channel`, stops normal MIDI and
    /// turns every LED off.
    fn start(&mut self, channel: u8) -> DriverResult<()> {
        self.asking = None;
        self.channel = channel & 0x0f;
        self.transport
            .write(&self.native(&[NATIVE_MODE, 0x00, 0x01]))?;
        self.transport.write(&self.packet_1())?;
        self.transport
            .write(&self.packet_2(&[false; LIT as usize]))?;
        self.sent = [Some(false); LIT as usize];
        self.refreshed = Instant::now();
        self.started = true;
        Ok(())
    }

    fn receive(&mut self, bytes: &[u8], events: &mut Vec<Input>) {
        let Some((command, first, second)) = native_event(bytes) else {
            return;
        };
        match command {
            PAD => self.press(first & 0x0f, first & 0x40 != 0, events),
            SWITCH => {
                let pressed = second > 0;
                if first < BUTTONS {
                    self.press(FIRST_BUTTON + first, pressed, events);
                } else if first == XY_TOUCH {
                    if pressed {
                        self.dials[usize::from(X_DIAL)] = None;
                        self.dials[usize::from(Y_DIAL)] = None;
                    }
                    self.press(XY_TOUCH_KEY, pressed, events);
                }
            }
            ENCODER => {
                let delta = signed(second);
                if delta != 0 {
                    events.push(Input::DialTurn {
                        index: ENCODER_DIAL,
                        delta,
                    });
                }
            }
            KNOB if first < 2 => self.turn(FIRST_KNOB_DIAL + first, second, events),
            XY => {
                self.turn(X_DIAL, first, events);
                self.turn(Y_DIAL, second, events);
            }
            PEDAL => self.press(PEDAL_KEY, second >= 0x40, events),
            NATIVE_ACK => {
                tracing::debug!(
                    "ryu-decks: padKONTROL native mode {}",
                    if second == 0x03 { "entered" } else { "left" }
                );
            }
            PACKET_ACK if first <= 1 && second != 0 => {
                tracing::debug!("ryu-decks: padKONTROL refused packet {}", first + 1);
            }
            _ => {}
        }
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
        let Some(old) = last.replace(value & 0x7f) else {
            return;
        };
        let moved = i16::from(value & 0x7f) - i16::from(old);
        if moved != 0 {
            events.push(Input::DialTurn {
                index: dial,
                delta: i8::try_from(moved).unwrap_or(0),
            });
        }
    }
}

impl Driver for PadKontrol {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let was_started = self.started;
        if !was_started {
            self.flush()?;
        }
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
        let mut events = Vec::new();
        for message in &messages {
            let Message::SysEx(bytes) = message else {
                continue;
            };
            if !self.started {
                if let Some(channel) = inquiry_reply(bytes) {
                    self.start(channel)?;
                    continue;
                }
            }
            self.receive(bytes, &mut events);
        }
        // Waits out the inquiry, or sends the LEDs staged before it ended.
        if !was_started {
            self.flush()?;
        }
        Ok(events)
    }

    /// Stages a key's LED: on for any colour that is not dark. The X-Y touch
    /// and the pedal keep nothing.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(led) = self.leds.get_mut(usize::from(key)) {
            *led = face.is_some_and(|face| is_on(face.color));
        }
        Ok(())
    }

    /// One `01 nn 20/00` for each LED that changed.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.ensure_started()? {
            return Ok(());
        }
        for key in 0..LIT {
            let index = usize::from(key);
            let on = self.leds[index];
            if self.sent[index] != Some(on) {
                let state = if on { LED_ON } else { LED_OFF };
                self.transport.write(&self.native(&[LED, key, state]))?;
                self.sent[index] = Some(on);
            }
        }
        Ok(())
    }

    /// The padKONTROL has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }

    /// Resends every LED, as one packet 2, every 30 seconds.
    fn heartbeat(&mut self) -> DriverResult<()> {
        if self.started && self.refreshed.elapsed() >= REFRESH {
            self.refreshed = Instant::now();
            self.transport.write(&self.packet_2(&self.leds))?;
            self.sent = self.leds.map(Some);
        }
        Ok(())
    }
}

impl Drop for PadKontrol {
    /// Turns every LED off and leaves Native Mode, so the deck returns to its
    /// scene.
    fn drop(&mut self) {
        if self.started {
            let _ = self.transport.write(&self.packet_2(&[false; LIT as usize]));
            let _ = self
                .transport
                .write(&self.native(&[NATIVE_MODE, 0x00, 0x00]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::{normalise, pair_ports};
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, Family, ModelView};

    fn open() -> (PadKontrol, Recorder) {
        let recorder = Recorder::default();
        let deck = PadKontrol::new(by_id(PADKONTROL_ID).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// Opens a deck and clears what opening it sent.
    fn started() -> (PadKontrol, Recorder) {
        let (mut deck, recorder) = open();
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

    fn event(channel: u8, command: u8, first: u8, second: u8) -> Vec<u8> {
        vec![
            0xf0,
            0x42,
            0x40 | channel,
            0x6e,
            0x08,
            command,
            first,
            second,
            0xf7,
        ]
    }

    fn queue(recorder: &Recorder, messages: &[Vec<u8>]) {
        for message in messages {
            recorder.queue_read(message.clone());
        }
    }

    fn packet_1(header: u8) -> Vec<u8> {
        let mut expected = vec![
            0xf0,
            0x42,
            header,
            0x6e,
            0x08,
            0x3f,
            0x2a,
            0x00,
            header & 0x0f,
        ];
        expected.extend([0; 24]);
        expected.extend(1..=16);
        expected.push(0xf7);
        expected
    }

    fn dark_packet_2(header: u8) -> Vec<u8> {
        vec![
            0xf0, 0x42, header, 0x6e, 0x08, 0x3f, 0x0a, 0x01, 0, 0, 0, 0, 0, 0, 0x20, 0x20, 0x20,
            0xf7,
        ]
    }

    #[test]
    fn opening_asks_the_channel_enters_native_mode_and_darkens() {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[0], [0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7]);
        assert_eq!(
            writes[1],
            [0xf0, 0x42, 0x40, 0x6e, 0x08, 0x00, 0x00, 0x01, 0xf7]
        );
        assert_eq!(writes[2], packet_1(0x40));
        assert_eq!(writes[2].len(), 50);
        assert_eq!(writes[3], dark_packet_2(0x40));
        assert_eq!(writes[3].len(), 18);
        assert_eq!(writes.len(), 4);
        // Nothing changed since, so nothing more is sent.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 4);
    }

    #[test]
    fn the_inquiry_reply_sets_the_global_channel_without_blocking() {
        let (mut deck, recorder) = open();
        deck.inquiry_wait = Duration::from_secs(60);
        deck.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        let begun = Instant::now();
        deck.flush().unwrap();
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert_eq!(recorder.writes().len(), 1);
        queue(
            &recorder,
            &[
                // A press before the reply still counts.
                event(0, 0x45, 0x40, 0x61),
                // Another maker's reply is not this deck's.
                vec![
                    0xf0, 0x7e, 0x05, 0x06, 0x02, 0x41, 0x6e, 0x00, 0x08, 0x00, 0x01, 0x00, 0x01,
                    0x00, 0xf7,
                ],
                vec![
                    0xf0, 0x7e, 0x03, 0x06, 0x02, 0x42, 0x6e, 0x00, 0x08, 0x00, 0x01, 0x00, 0x01,
                    0x00, 0xf7,
                ],
            ],
        );
        assert_eq!(deck.poll(0).unwrap(), vec![key(0, true)]);
        let writes = recorder.writes();
        assert_eq!(
            writes[1],
            [0xf0, 0x42, 0x43, 0x6e, 0x08, 0x00, 0x00, 0x01, 0xf7]
        );
        assert_eq!(writes[2], packet_1(0x43));
        assert_eq!(writes[3], dark_packet_2(0x43));
        // The light staged meanwhile follows.
        assert_eq!(
            writes[4],
            [0xf0, 0x42, 0x43, 0x6e, 0x08, 0x01, 0x03, 0x20, 0xf7]
        );
        assert_eq!(writes.len(), 5);
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes[5], dark_packet_2(0x43));
        assert_eq!(
            writes[6],
            [0xf0, 0x42, 0x43, 0x6e, 0x08, 0x00, 0x00, 0x00, 0xf7]
        );
    }

    #[test]
    fn pads_buttons_touch_and_pedal_are_keys() {
        let (mut deck, recorder) = started();
        queue(
            &recorder,
            &[
                event(0, 0x45, 0x40, 0x61),
                // Release velocity is 64: the bit decides.
                event(0, 0x45, 0x00, 0x40),
                event(0, 0x45, 0x4f, 0x01),
                event(5, 0x48, 0x0a, 0x7f),
                event(0, 0x48, 0x0a, 0x00),
                event(0, 0x48, 0x00, 0x7f),
                event(0, 0x48, 0x12, 0x7f),
                event(0, 0x48, 0x20, 0x7f),
                event(0, 0x48, 0x20, 0x00),
                // An unknown switch.
                event(0, 0x48, 0x13, 0x7f),
                event(0, 0x47, 0x00, 0x30),
                event(0, 0x47, 0x00, 0x50),
                event(0, 0x47, 0x00, 0x7f),
                event(0, 0x47, 0x00, 0x10),
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(15, true),
                key(26, true),
                key(26, false),
                key(16, true),
                key(34, true),
                key(35, true),
                key(35, false),
                key(36, true),
                key(36, false),
            ]
        );
    }

    #[test]
    fn encoder_knobs_and_xy_pad_are_dials() {
        let (mut deck, recorder) = started();
        let mut knob = event(0, 0x49, 0x00, 0x40);
        // KORG's printed knob header.
        knob[4] = 0x02;
        queue(
            &recorder,
            &[
                event(0, 0x43, 0x00, 0x7f),
                event(0, 0x43, 0x00, 0x01),
                event(0, 0x43, 0x00, 0x40),
                event(0, 0x43, 0x00, 0x3f),
                event(0, 0x43, 0x00, 0x00),
                knob,
                event(0, 0x49, 0x00, 0x45),
                event(0, 0x49, 0x01, 0x10),
                event(0, 0x49, 0x01, 0x00),
                event(0, 0x49, 0x02, 0x10),
                event(0, 0x4b, 0x40, 0x40),
                event(0, 0x4b, 0x42, 0x3c),
                // A new touch seeds again.
                event(0, 0x48, 0x20, 0x7f),
                event(0, 0x4b, 0x10, 0x10),
                event(0, 0x4b, 0x11, 0x10),
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, -1),
                turn(0, 1),
                turn(0, -64),
                turn(0, 63),
                turn(1, 5),
                turn(2, -16),
                turn(3, 2),
                turn(4, -4),
                key(35, true),
                turn(3, 1),
            ]
        );
    }

    #[test]
    fn malformed_and_foreign_messages_are_ignored() {
        let (mut deck, recorder) = started();
        let mut wrong_sub = event(0, 0x45, 0x40, 0x61);
        wrong_sub[4] = 0x02;
        let mut wrong_device = event(0, 0x45, 0x40, 0x61);
        wrong_device[3] = 0x13;
        queue(
            &recorder,
            &[
                wrong_sub,
                wrong_device,
                vec![0xf0, 0x42, 0x40, 0x6e, 0x08, 0x45, 0xf7],
                vec![0xf0, 0x42, 0x40, 0x6e, 0x08, 0x45, 0x40, 0x61, 0x00, 0xf7],
                vec![0x90, 0x24, 0x7f],
                vec![0xb0, 0x01, 0x7f],
                event(0, 0x40, 0x00, 0x03),
                event(0, 0x5f, 0x00, 0x01),
                event(0, 0x5f, 0x4f, 0x02),
                vec![0xf0],
                vec![],
            ],
        );
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn lights_are_sent_on_change_and_never_for_unlit_keys() {
        let (mut deck, recorder) = started();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(34, Some(&lit([0, 0, 200]))).unwrap();
        deck.set_face(1, Some(&lit([8, 8, 8]))).unwrap();
        deck.set_face(35, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(36, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(200, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0xf0, 0x42, 0x40, 0x6e, 0x08, 0x01, 0x00, 0x20, 0xf7],
                vec![0xf0, 0x42, 0x40, 0x6e, 0x08, 0x01, 0x22, 0x20, 0xf7],
            ]
        );
        deck.set_face(0, None).unwrap();
        deck.set_face(34, Some(&lit([0, 0, 200]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes().last().unwrap(),
            &[0xf0, 0x42, 0x40, 0x6e, 0x08, 0x01, 0x00, 0x00, 0xf7]
        );
        assert_eq!(recorder.writes().len(), 3);
        // The heartbeat waits its 30 seconds, then sends one packet 2.
        deck.heartbeat().unwrap();
        assert_eq!(recorder.writes().len(), 3);
        deck.set_face(16, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(7, Some(&lit([255, 255, 255]))).unwrap();
        deck.refreshed = Instant::now().checked_sub(REFRESH).unwrap();
        deck.heartbeat().unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[3],
            [
                0xf0, 0x42, 0x40, 0x6e, 0x08, 0x3f, 0x0a, 0x01, 0x00, 0x01, 0x04, 0x00, 0x40, 0x00,
                0x20, 0x20, 0x20, 0xf7
            ]
        );
        // The packet carried them, so the flush sends nothing.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 4);
    }

    #[test]
    fn letting_go_darkens_and_leaves_native_mode() {
        let (deck, recorder) = started();
        drop(deck);
        assert_eq!(
            recorder.writes(),
            vec![
                dark_packet_2(0x40),
                vec![0xf0, 0x42, 0x40, 0x6e, 0x08, 0x00, 0x00, 0x00, 0xf7],
            ]
        );
        // A deck never started sends nothing.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_the_native_mode_port_is_found() {
        let cases = [
            ("padKONTROL Port 2", false, true),
            ("padKONTROL (Port 2)", false, true),
            ("padKONTROL 1 PORT A", false, true),
            ("padKONTROL 1 CTRL", false, true),
            ("padKONTROL:padKONTROL 1 PORT A 24:1", false, true),
            ("padKONTROL:padKONTROL MIDI 2 24:1", false, true),
            ("MIDIIN2 (padKONTROL)", true, true),
            ("MIDIOUT2 (padKONTROL)", true, true),
            ("padKONTROL Port 1", false, false),
            ("padKONTROL Port 3", false, false),
            ("padKONTROL 1 PORT B", false, false),
            ("padKONTROL _ PORT B", false, false),
            ("padKONTROL 1 MIDI IN", false, false),
            ("padKONTROL 1 MIDI OUT", false, false),
            ("padKONTROL:padKONTROL MIDI 1 24:0", false, false),
            ("padKONTROL:padKONTROL 1 PORT B 24:2", false, false),
            ("padKONTROL", true, false),
            ("padKONTROL", false, false),
            ("MIDIIN3 (padKONTROL)", true, false),
            ("2- padKONTROL", true, false),
        ];
        for (raw, windows, expected) in cases {
            assert_eq!(
                identify_midi(&normalise(raw), windows).map(|model| model.id),
                expected.then_some(PADKONTROL_ID),
                "{raw}"
            );
        }
    }

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn port_a_pairs_with_ctrl() {
        let found = pair_ports(
            &strings(&[
                "padKONTROL 1 MIDI IN",
                "padKONTROL 1 PORT A",
                "padKONTROL 1 PORT B",
            ]),
            &strings(&["padKONTROL 1 MIDI OUT", "padKONTROL 1 CTRL"]),
            false,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].model.id, PADKONTROL_ID);
        assert_eq!(found[0].ports.input, "padKONTROL 1 PORT A");
        assert_eq!(found[0].ports.output, "padKONTROL 1 CTRL");
        let found = pair_ports(
            &strings(&[
                "padKONTROL Port 1",
                "padKONTROL Port 2",
                "padKONTROL Port 3",
            ]),
            &strings(&["padKONTROL Port 1", "padKONTROL Port 2"]),
            false,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ports.output, "padKONTROL Port 2");
    }

    #[test]
    fn the_row_names_places_and_lights_every_key() {
        let model = by_id(PADKONTROL_ID).unwrap();
        assert_eq!(model.family, Family::Korg);
        assert!(model.protocol.is_midi() && !model.protocol.has_brightness());
        assert_eq!(model.dial_light_colors(), 0);
        let view = ModelView::from(model);
        assert_eq!((view.keys, view.dials), (37, 5));
        assert_eq!(view.key_names.as_ref().unwrap().len(), 37);
        let cells = view.key_cells.clone().unwrap();
        assert_eq!(cells.len(), 37);
        assert!(cells
            .iter()
            .all(|cell| *cell < u16::from(view.rows) * u16::from(view.cols)));
        let mut distinct = cells.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), cells.len());
        // Pad 1 top left of the pads, pad 16 bottom right.
        assert_eq!((cells[0], cells[15]), (12, 39));
        let lit = view.lit_keys.clone().unwrap();
        assert_eq!(lit, (0..35).collect::<Vec<u8>>());
        assert_eq!(view.key_light_colors, "on_off");
        assert_eq!(view.dial_lights, DialLights::None);
        assert!(view.key_lights && !view.brightness);
    }
}
