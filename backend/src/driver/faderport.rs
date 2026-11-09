//! PreSonus' original FaderPort (2007, one motor fader), over class-compliant
//! USB MIDI ([`super::midi`]), in its native mode.
//!
//! The deck lists one MIDI port each way whose name contains `FaderPort`
//! (`FaderPort MIDI 1` on Linux). [`port_matches`] takes `FADERPORT` in any
//! case unless a model number follows it (`FaderPort 2`, `FaderPort8`, ...)
//! or the name carries the newer decks' `FP2` / `FP8` / `FP16`: those are
//! [`super::faderport8`]'s.
//!
//! Start-up, queued from the first poll and never waited on: Ryu sends the
//! universal Device Inquiry (`F0 7E 7F 06 01 F7`). PreSonus' Identity Reply
//! (at least 17 bytes, bytes 2-11 `7F 06 02 00 01 06 02 00 01 00`) is
//! answered with the native-mode switch, a Note On on channel 2
//! (`91 00 64`), then every light. If no reply has come after
//! [`HANDSHAKE_POLLS`] polls, Ryu sends the switch anyway (an assumption: the
//! reference only sends it on the reply).
//!
//! In, on any channel: each button is Polyphonic Aftertouch `An <id> <v>`,
//! pressed while `v` is not zero. Ids 0-23 are the 24 buttons (keys 0-23),
//! 126 the footswitch (key 24) and 127 touching the fader (key 25). The
//! encoder is a pitch bend per detent, one step each: under 8192 clockwise,
//! otherwise anticlockwise. It chatters, so (like the reference) a detent
//! under [`MIN_GAP`] after the last one is dropped (measured between polls:
//! detents read in one poll share a timestamp and are all taken), and within [`HOLD`] the
//! direction only flips once three detents in a row agree. The motor fader
//! is `CC 0` (high seven bits) and `CC 32` (low seven): Ryu reads the high
//! byte as an absolute position, seeded by its first value, a dial step per
//! two units.
//!
//! Out: a button's LED is Polyphonic Aftertouch on channel 1, `A0 <led> <01
//! or 00>`, where the LED number is not the button's input id (see
//! [`LED_OF_KEY`]). The footswitch and the fader touch have no light. There
//! is no brightness command and no keep-alive; Ryu never moves the motor
//! fader. Letting go turns every LED off.
//!
//! Facts from Ardour's FaderPort control surface (C++, GPL-2.0-or-later); no
//! code was copied. No USB id is given there, so the catalog shows none.

use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const FADERPORT_ID: &str = "presonus-faderport";

/// The 24 buttons, the footswitch and the fader touch.
pub const KEYS: u8 = 26;
/// The encoder, then the fader.
pub const DIALS: u8 = 2;
pub const ROWS: u8 = 4;
pub const COLS: u8 = 7;
/// The buttons with a light: keys 0-23.
const LIT: u8 = 24;
const FOOTSWITCH_KEY: u8 = 24;
const FADER_TOUCH_KEY: u8 = 25;
const ENCODER_DIAL: u8 = 0;
const FADER_DIAL: u8 = 1;

const NAMES: [&str; KEYS as usize] = [
    "User",
    "Punch",
    "Shift",
    "Rewind",
    "FFwd",
    "Stop",
    "Play",
    "Rec Enable",
    "Touch",
    "Write",
    "Read",
    "Mix",
    "Proj",
    "Trns",
    "Undo",
    "Loop",
    "Rec",
    "Solo",
    "Mute",
    "Left",
    "Bank",
    "Right",
    "Output",
    "Off",
    "Footswitch",
    "Fader Touch",
];

/// The LED number of each lit key (its button's input id).
pub const LED_OF_KEY: [u8; LIT as usize] = [
    7, 6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8, 23, 22, 21, 20, 19, 18, 17, 16,
];

const POLY_PRESSURE: u8 = 0xa0;
const CONTROL_CHANGE: u8 = 0xb0;
const PITCH_BEND: u8 = 0xe0;
const FOOTSWITCH: u8 = 126;
const FADER_TOUCH: u8 = 127;
const FADER_MSB_CC: u8 = 0;

/// The universal Device Inquiry.
const INQUIRY: [u8; 6] = [0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7];
/// Bytes 2-11 of the FaderPort's Identity Reply: all devices, identity
/// reply, PreSonus (`00 01 06`), family 2, member 1.
const IDENTITY: [u8; 10] = [0x7f, 0x06, 0x02, 0x00, 0x01, 0x06, 0x02, 0x00, 0x01, 0x00];
const IDENTITY_MIN_LEN: usize = 17;
/// Note On, channel 2, note 0, velocity 100: native mode.
const NATIVE_MODE: [u8; 3] = [0x91, 0x00, 0x64];
/// Polls to wait for the Identity Reply before switching anyway.
pub const HANDSHAKE_POLLS: u8 = 20;

const LED_ON: u8 = 0x01;
const LED_OFF: u8 = 0x00;
/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Fader units (of 128) per dial step: 64 steps from end to end.
const FADER_STEP: i16 = 2;
/// The bend value from which a detent is anticlockwise.
const BEND_CENTRE: u16 = 8192;
/// A detent closer than this to the last one taken is chatter.
const MIN_GAP: Duration = Duration::from_millis(10);
/// Within this of the last detent taken, the direction holds unless three
/// detents in a row disagree with it.
const HOLD: Duration = Duration::from_millis(100);
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// Whether a normalised port name is a classic FaderPort's: `FADERPORT` in
/// any case with no model number after it, not one of the newer decks'
/// `FP2` / `FP8` / `FP16` names, and on Windows the first interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.protocol != Protocol::PresonusFaderPortMidi || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.to_ascii_uppercase();
    let newer = base
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| matches!(word, "FP2" | "FP8" | "FP16"));
    if newer {
        return false;
    }
    let mut found = false;
    for (at, _) in base.match_indices("FADERPORT") {
        let rest = base.get(at + "FADERPORT".len()..).unwrap_or_default();
        if rest.trim_start().starts_with(|c: char| c.is_ascii_digit()) {
            return false;
        }
        found = true;
    }
    found
}

/// The legend on each key: the buttons' printed names, the footswitch and
/// the fader touch.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::PresonusFaderPortMidi).then(|| NAMES.to_vec())
}

/// The keys with a light: the 24 buttons.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    (model.protocol == Protocol::PresonusFaderPortMidi).then(|| (0..LIT).collect())
}

/// The key a button id presses.
const fn key_of(id: u8) -> Option<u8> {
    match id {
        FOOTSWITCH => Some(FOOTSWITCH_KEY),
        FADER_TOUCH => Some(FADER_TOUCH_KEY),
        _ if id < LIT => Some(id),
        _ => None,
    }
}

/// Whether a SysEx message is the FaderPort's Identity Reply.
fn is_identity_reply(sysex: &[u8]) -> bool {
    sysex.len() >= IDENTITY_MIN_LEN && sysex.get(2..12) == Some(&IDENTITY[..])
}

/// An absolute 7-bit control read as a dial: its first value seeds it, then
/// every `step` units moved turn the dial one step.
#[derive(Debug, Default)]
struct Absolute {
    last: Option<u8>,
    rest: i16,
}

impl Absolute {
    fn moved(&mut self, value: u8, step: i16) -> i8 {
        let Some(old) = self.last.replace(value) else {
            return 0;
        };
        let moved = self.rest + i16::from(value) - i16::from(old);
        let steps = moved / step;
        self.rest = moved - steps * step;
        i8::try_from(steps.clamp(-127, 127)).unwrap_or(0)
    }
}

/// The encoder's chatter filter.
#[derive(Debug, Default)]
struct Encoder {
    /// When the last detent was taken.
    last: Option<Instant>,
    /// The direction taken last.
    direction: i8,
    /// The raw directions of the last two detents seen.
    recent: [i8; 2],
}

impl Encoder {
    /// The step to take for a detent read as `raw` (+1 or -1) at `now`.
    fn detent(&mut self, raw: i8, now: Instant) -> Option<i8> {
        let since = self.last.map(|last| now.saturating_duration_since(last));
        // Messages read in one poll share one `now`, so a zero gap says nothing
        // about how far apart they arrived: only a measured gap is chatter.
        if since.is_some_and(|since| !since.is_zero() && since < MIN_GAP) {
            return None;
        }
        let agreed = self.recent == [raw, raw];
        self.recent = [self.recent[1], raw];
        let step = if since.is_some_and(|since| since < HOLD)
            && self.direction != 0
            && raw != self.direction
            && !agreed
        {
            self.direction
        } else {
            raw
        };
        self.direction = step;
        self.last = Some(now);
        Some(step)
    }
}

pub struct FaderPort {
    transport: Box<dyn Transport>,
    /// The Device Inquiry has gone out.
    asked: bool,
    /// Native mode is on and every light has gone out.
    native: bool,
    /// Polls since the inquiry, waiting for its reply.
    polls: u8,
    /// Each lit key's LED as staged, then as last sent.
    leds: [u8; LIT as usize],
    sent: [Option<u8>; LIT as usize],
    held: [bool; KEYS as usize],
    encoder: Encoder,
    fader: Absolute,
    parser: Parser,
}

impl FaderPort {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            asked: false,
            native: false,
            polls: 0,
            leds: [LED_OFF; LIT as usize],
            sent: [None; LIT as usize],
            held: [false; KEYS as usize],
            encoder: Encoder::default(),
            fader: Absolute::default(),
            parser: Parser::default(),
        }
    }

    fn ask(&mut self) -> DriverResult<()> {
        if !self.asked {
            self.asked = true;
            self.transport.write(&INQUIRY)?;
        }
        Ok(())
    }

    /// Native mode on, then every light as staged.
    fn go_native(&mut self) -> DriverResult<()> {
        if self.native {
            return Ok(());
        }
        self.native = true;
        self.transport.write(&NATIVE_MODE)?;
        self.sent = [None; LIT as usize];
        self.send_changes()
    }

    /// One message for each LED that differs from what was sent.
    fn send_changes(&mut self) -> DriverResult<()> {
        if !self.native {
            return Ok(());
        }
        for ((led, value), sent) in LED_OF_KEY.iter().zip(self.leds).zip(&mut self.sent) {
            if *sent != Some(value) {
                self.transport.write(&[POLY_PRESSURE, *led, value])?;
                *sent = Some(value);
            }
        }
        Ok(())
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

    fn receive(
        &mut self,
        message: &Message,
        now: Instant,
        events: &mut Vec<Input>,
    ) -> DriverResult<()> {
        let [status, first, second] = match message {
            Message::SysEx(sysex) => {
                if is_identity_reply(sysex) {
                    self.go_native()?;
                }
                return Ok(());
            }
            Message::Channel(bytes) => *bytes,
        };
        match status & 0xf0 {
            POLY_PRESSURE => {
                if let Some(key) = key_of(first) {
                    self.press(key, second != 0, events);
                }
            }
            PITCH_BEND => {
                let bend = u16::from(first & 0x7f) | (u16::from(second & 0x7f) << 7);
                let raw = if bend < BEND_CENTRE { 1 } else { -1 };
                if let Some(delta) = self.encoder.detent(raw, now) {
                    events.push(Input::DialTurn {
                        index: ENCODER_DIAL,
                        delta,
                    });
                }
            }
            CONTROL_CHANGE if first == FADER_MSB_CC => {
                let delta = self.fader.moved(second & 0x7f, FADER_STEP);
                if delta != 0 {
                    events.push(Input::DialTurn {
                        index: FADER_DIAL,
                        delta,
                    });
                }
            }
            // The fader's low byte, and the deck's own Note On echo.
            _ => {}
        }
        Ok(())
    }
}

impl Driver for FaderPort {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ask()?;
        if !self.native {
            self.polls = self.polls.saturating_add(1);
            if self.polls > HANDSHAKE_POLLS {
                tracing::debug!(
                    "the FaderPort did not answer the device inquiry; switching anyway"
                );
                self.go_native()?;
            }
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
        let now = Instant::now();
        for message in &messages {
            self.receive(message, now, &mut events)?;
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        // The footswitch and the fader touch have no light.
        if let Some(led) = self.leds.get_mut(usize::from(key)) {
            let lit = face.is_some_and(|face| face.color.iter().any(|channel| *channel >= DARK));
            *led = if lit { LED_ON } else { LED_OFF };
        }
        Ok(())
    }

    /// The inquiry the first time; once native mode is on, the LEDs that
    /// changed.
    fn flush(&mut self) -> DriverResult<()> {
        self.ask()?;
        self.send_changes()
    }

    /// The FaderPort has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for FaderPort {
    /// Turns every LED off.
    fn drop(&mut self) {
        if !self.native {
            return;
        }
        for led in LED_OF_KEY {
            let _ = self.transport.write(&[POLY_PRESSURE, led, LED_OFF]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    const REPLY: [u8; 17] = [
        0xf0, 0x7e, 0x7f, 0x06, 0x02, 0x00, 0x01, 0x06, 0x02, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00,
        0x00, 0xf7,
    ];

    fn open() -> (FaderPort, Recorder) {
        let recorder = Recorder::default();
        let deck = FaderPort::new(by_id(FADERPORT_ID).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// Opened and in native mode, with every LED sent off.
    fn native() -> (FaderPort, Recorder) {
        let (mut deck, recorder) = open();
        recorder.queue_read(REPLY.to_vec());
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 2 + 24);
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
    fn the_identity_reply_switches_to_native_mode_then_sends_the_lights() {
        let (mut deck, recorder) = open();
        deck.set_face(6, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        // Only the inquiry until the deck answers.
        assert_eq!(recorder.writes(), vec![INQUIRY.to_vec()]);
        deck.poll(0).unwrap();
        // A reply from another device, or one cut short, changes nothing.
        let mut other = REPLY;
        other[7] = 0x07;
        recorder.queue_read(other.to_vec());
        let mut short = REPLY[..15].to_vec();
        short.push(0xf7);
        recorder.queue_read(short);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 1);
        recorder.queue_read(REPLY.to_vec());
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 2 + 24);
        assert_eq!(writes[1], vec![0x91, 0x00, 0x64]);
        // Key 6 (Play) is LED 1; the others go off.
        for (key, write) in writes[2..].iter().enumerate() {
            let value = u8::from(key == 6);
            assert_eq!(*write, vec![0xa0, LED_OF_KEY[key], value], "key {key}");
        }
        assert_eq!(writes[2 + 6], vec![0xa0, 0x01, 0x01]);
        // A second reply does nothing more.
        recorder.queue_read(REPLY.to_vec());
        deck.poll(0).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 2 + 24);
    }

    #[test]
    fn with_no_reply_native_mode_goes_out_anyway() {
        let (mut deck, recorder) = open();
        for _ in 0..HANDSHAKE_POLLS {
            deck.poll(0).unwrap();
        }
        assert_eq!(recorder.writes().len(), 1);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 2 + 24);
        assert_eq!(writes[1], NATIVE_MODE.to_vec());
    }

    #[test]
    fn leds_use_their_own_numbers() {
        let (mut deck, recorder) = native();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(7, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(16, Some(&lit([0, 0, 200]))).unwrap();
        deck.set_face(23, Some(&lit([255, 255, 255]))).unwrap();
        // Dark, unbound, unlit and past the end.
        deck.set_face(1, Some(&lit([20, 20, 20]))).unwrap();
        deck.set_face(2, None).unwrap();
        deck.set_face(24, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(25, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(26, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[26..],
            [
                vec![0xa0, 7, 1],
                vec![0xa0, 0, 1],
                vec![0xa0, 23, 1],
                vec![0xa0, 16, 1],
            ]
        );
        deck.set_face(7, None).unwrap();
        deck.set_brightness(0).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[30..], [vec![0xa0, 0, 0]]);
    }

    #[test]
    fn buttons_the_footswitch_and_the_fader_touch_are_keys() {
        let (mut deck, recorder) = native();
        queue(
            &recorder,
            &[
                &[0xa0, 0x06, 0x7f],
                &[0xa0, 0x06, 0x00],
                &[0xa0, 0x17, 0x01],
                // Any channel.
                &[0xa3, 0x17, 0x00],
                &[0xa0, 0x7e, 0x7f],
                &[0xa0, 0x7e, 0x00],
                &[0xa0, 0x7f, 0x7f],
                &[0xa0, 0x7f, 0x00],
                // Unknown ids, a repeated release, a Note On, the fader's
                // low byte, short and stray input.
                &[0xa0, 0x18, 0x7f],
                &[0xa0, 0x50, 0x7f],
                &[0xa0, 0x06, 0x00],
                &[0x90, 0x06, 0x7f],
                &[0xb0, 0x20, 0x55],
                &[0xa0, 0x11],
                &[0xf6],
                &[0x11, 0x22],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(6, true),
                key(6, false),
                key(23, true),
                key(23, false),
                key(24, true),
                key(24, false),
                key(25, true),
                key(25, false),
            ]
        );
    }

    #[test]
    fn the_fader_turns_dial_2_after_its_first_value() {
        let (mut deck, recorder) = native();
        queue(
            &recorder,
            &[
                &[0xb0, 0x00, 0x40],
                &[0xb0, 0x20, 0x10],
                &[0xb0, 0x00, 0x44],
                &[0xb0, 0x00, 0x45],
                &[0xb0, 0x00, 0x46],
                &[0xb0, 0x00, 0x00],
                &[0xb0, 0x00, 0x7f],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(1, 2), turn(1, 1), turn(1, -35), turn(1, 63)]
        );
    }

    #[test]
    fn each_encoder_bend_is_one_step() {
        let (mut deck, recorder) = native();
        queue(&recorder, &[&[0xe0, 0x00, 0x00], &[0xe0, 0x7f, 0x3f]]);
        // Both arrive in one poll, whose single timestamp cannot tell how far
        // apart they were: both are taken, not dropped as chatter.
        assert_eq!(deck.poll(0).unwrap(), vec![turn(0, 1), turn(0, 1)]);
    }

    #[test]
    fn the_encoder_filter_drops_chatter_and_holds_its_direction() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut encoder = Encoder::default();
        assert_eq!(encoder.detent(1, at(0)), Some(1));
        // Too soon after the last one.
        assert_eq!(encoder.detent(1, at(5)), None);
        assert_eq!(encoder.detent(1, at(20)), Some(1));
        // One stray reversal inside the hold keeps the direction.
        assert_eq!(encoder.detent(-1, at(40)), Some(1));
        assert_eq!(encoder.detent(-1, at(60)), Some(1));
        // The third in a row agreeing turns it.
        assert_eq!(encoder.detent(-1, at(80)), Some(-1));
        assert_eq!(encoder.detent(-1, at(100)), Some(-1));
        // After a pause, the first detent sets the direction.
        assert_eq!(encoder.detent(1, at(500)), Some(1));
        // Detents sharing one timestamp (one poll) are not chatter.
        assert_eq!(encoder.detent(1, at(600)), Some(1));
        assert_eq!(encoder.detent(1, at(600)), Some(1));
    }

    #[test]
    fn letting_go_turns_every_led_off() {
        let (mut deck, recorder) = native();
        deck.set_face(3, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        let writes = &recorder.writes()[before..];
        assert_eq!(writes.len(), 24);
        for (write, led) in writes.iter().zip(LED_OF_KEY) {
            assert_eq!(*write, vec![0xa0, led, 0]);
        }
        // Never in native mode: nothing sent.
        let (mut deck, recorder) = open();
        deck.poll(0).unwrap();
        drop(deck);
        assert_eq!(recorder.writes(), vec![INQUIRY.to_vec()]);
    }

    #[test]
    fn only_the_classic_faderports_port_matches() {
        let model = by_id(FADERPORT_ID).unwrap();
        let cases = [
            ("FaderPort", false, true),
            ("FaderPort", true, true),
            ("FaderPort:FaderPort MIDI 1 20:0", false, true),
            ("FADERPORT", false, true),
            ("PreSonus FaderPort", false, true),
            ("MIDIIN2 (FaderPort)", true, false),
            // The newer FaderPorts and unrelated ports.
            ("FaderPort 8", false, false),
            ("FaderPort8", false, false),
            ("FaderPort 16", false, false),
            ("FaderPort2", false, false),
            ("PreSonus FP2", false, false),
            ("PreSonus FP8 Port 1", false, false),
            ("FaderPort FP16", false, false),
            ("ATOM", false, false),
            ("Midi Through:Midi Through Port-0 14:0", false, false),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(port_matches(model, &name), expected, "{raw}");
            assert_eq!(
                identify_midi(&name, windows).map(|found| found.id) == Some(FADERPORT_ID),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_view_names_every_key_and_lights_only_the_buttons() {
        let model = by_id(FADERPORT_ID).unwrap();
        assert_eq!(model.dial_lights, DialLights::None);
        let view = ModelView::from(model);
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), 26);
        assert_eq!(
            (names[0], names[6], names[23], names[24], names[25]),
            ("User", "Play", "Off", "Footswitch", "Fader Touch")
        );
        assert_eq!(view.lit_keys.unwrap(), (0..=23).collect::<Vec<u8>>());
        assert_eq!((view.keys, view.extra_keys, view.dials), (26, 0, 2));
        assert_eq!((view.rows, view.cols), (4, 7));
        assert_eq!(view.deck_lights, 0);
        assert_eq!(view.key_light_colors, "on_off");
        assert!(view.key_lights && !view.brightness);
    }
}
