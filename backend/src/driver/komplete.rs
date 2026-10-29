//! Native Instruments Komplete Kontrol S25, S49, S61 and S88 (MK1): the HID
//! control section above the piano keys.
//!
//! The piano keys, the pitch and modulation wheels and the touch strip reach
//! the computer over the keyboard's USB-MIDI interface, which Ryu never opens.
//! The HID side is the 23 buttons, the browse encoder and eight parameter
//! encoders, their LEDs and the RGB light guide over the piano keys. Every input
//! report starts with its report id, and every write is the report id followed
//! by its payload, unpadded; byte offsets below count the id as byte 0.
//!
//! - Start: `A0 00 00`, written once before anything else.
//! - Input report `01` (38 bytes): bytes 1 to 5 are a bitfield, bit n at byte
//!   1 + n / 8, bit n % 8. Bit 0 is the browse encoder's push, bits 1 to 23 the
//!   buttons (Preset Up, Enter, Preset Down, Browse, Instance, Octave Down,
//!   Octave Up, Stop, Rec, Play, Navigate Right, Down, Left, Back, Navigate Up,
//!   Shift, Scale, Arp, Loop, Page Right, Page Left, Rwd, Ffw), bits 24 to 32
//!   capacitive touch on the encoders (not read). Byte 6 is the browse encoder,
//!   a 4-bit counter that wraps; bytes 7 to 22 are the eight parameter
//!   encoders as little-endian counters that wrap at 10 bits.
//! - Output report `80` (25 bytes): one brightness byte per button LED, in an
//!   order of its own (Shift first, Navigate Right at 20), then two LEDs each
//!   for Octave Down and Octave Up, taken to be white then red.
//! - Output report `82`: the light guide, red, green, blue for each piano key
//!   from the lowest up (25, 49, 61 or 88 keys).
//!
//! The nine segment LCDs are not driven: their payload is not documented.
//! There is no brightness command or keep-alive, so Ryu scales the light
//! values, sends a report only when it changes, and blanks every light when it
//! lets go. It writes each value as 7 bits, as the other Native Instruments
//! controllers take them; the device's own range is not documented. Only the
//! octave buttons' white LEDs are lit; their red LEDs stay off.
//!
//! Facts per shaduzlabs/cabl (MIT); no code was copied.

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const S25_ID: &str = "ni-komplete-kontrol-s25";
pub const S49_ID: &str = "ni-komplete-kontrol-s49";
pub const S61_ID: &str = "ni-komplete-kontrol-s61";
pub const S88_ID: &str = "ni-komplete-kontrol-s88";

const START: [u8; 3] = [0xa0, 0x00, 0x00];
const INPUT_REPORT: u8 = 0x01;
const BUTTON_LIGHTS_REPORT: u8 = 0x80;
const BUTTON_LIGHTS_LEN: usize = 25;
const GUIDE_REPORT: u8 = 0x82;
const READ_BUFFER: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 64;
/// The shortest report that carries every button and the browse encoder.
const MIN_INPUT_LEN: usize = 7;
const BROWSE_AT: usize = 6;
/// Parameter encoder 1; encoders 2 to 8 follow, two bytes each.
const ENCODERS_AT: usize = 7;
const ENCODERS: usize = 8;

/// The keys, in input-bit order (key n is bit n + 1).
pub const KEY_NAMES: [&str; 23] = [
    "Preset Up",
    "Enter",
    "Preset Down",
    "Browse",
    "Instance",
    "Octave Down",
    "Octave Up",
    "Stop",
    "Rec",
    "Play",
    "Navigate Right",
    "Navigate Down",
    "Navigate Left",
    "Back",
    "Navigate Up",
    "Shift",
    "Scale",
    "Arp",
    "Loop",
    "Page Right",
    "Page Left",
    "Rwd",
    "Ffw",
];

/// Each key's row and column on the Companion's 5 x 7 grid: Shift to Page Right
/// and Browse on top, the transport and preset buttons below, the octave
/// buttons and the navigation cross under them.
pub const KEY_PLACES: [(u8, u8); 23] = [
    (1, 5), // Preset Up
    (3, 5), // Enter
    (2, 6), // Preset Down
    (0, 6), // Browse
    (1, 6), // Instance
    (2, 0), // Octave Down
    (2, 1), // Octave Up
    (1, 4), // Stop
    (1, 3), // Rec
    (1, 2), // Play
    (3, 6), // Navigate Right
    (4, 5), // Navigate Down
    (3, 4), // Navigate Left
    (2, 4), // Back
    (2, 5), // Navigate Up
    (0, 0), // Shift
    (0, 1), // Scale
    (0, 2), // Arp
    (0, 3), // Loop
    (0, 5), // Page Right
    (0, 4), // Page Left
    (1, 0), // Rwd
    (1, 1), // Ffw
];

/// Each key's LED in the `80` report, counting the first byte after the id
/// as 0. Octave Down and Octave Up take their white LEDs (21 and 23).
const KEY_LIGHTS: [usize; 23] = [
    12, 17, 14, 11, 13, 21, 23, 8, 7, 6, 20, 19, 18, 15, 16, 0, 1, 2, 3, 10, 9, 4, 5,
];

/// The piano keys under the light guide, by model.
#[must_use]
pub fn guide_keys(model: &DeviceModel) -> usize {
    match model.id {
        S25_ID => 25,
        S49_ID => 49,
        S61_ID => 61,
        S88_ID => 88,
        _ => 0,
    }
}

/// An 8-bit colour channel as a 7-bit light value, scaled by a percentage.
fn level(channel: u8, percent: u8) -> u8 {
    u8::try_from(u16::from(channel >> 1) * u16::from(percent.min(100)) / 100).unwrap_or(0x7f)
}

/// The signed difference of two counters that wrap at `modulus` (a power of two).
fn wrapped(new: u16, old: u16, modulus: u16) -> i8 {
    let half = modulus / 2;
    let delta =
        i32::from(new.wrapping_sub(old).wrapping_add(half) & (modulus - 1)) - i32::from(half);
    i8::try_from(delta.clamp(-127, 127)).unwrap_or(0)
}

pub struct KompleteKontrol {
    transport: Box<dyn Transport>,
    guide_keys: usize,
    started: bool,
    /// Each key's light colour before brightness; `None` for off.
    colors: [Option<[u8; 3]>; 23],
    guide: Option<[u8; 3]>,
    brightness: u8,
    sent_buttons: Option<Vec<u8>>,
    sent_guide: Option<Vec<u8>>,
    /// Bits 0 to 23 at the last report.
    held: u32,
    browse: Option<u16>,
    encoders: [Option<u16>; ENCODERS],
}

impl KompleteKontrol {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            guide_keys: guide_keys(model),
            started: false,
            colors: [None; 23],
            guide: None,
            brightness: 100,
            sent_buttons: None,
            sent_guide: None,
            held: 0,
            browse: None,
            encoders: [None; ENCODERS],
        }
    }

    fn ensure_started(&mut self) -> DriverResult<()> {
        if !self.started {
            self.transport.write(&START)?;
            self.started = true;
        }
        Ok(())
    }

    fn parse(&mut self, report: &[u8], events: &mut Vec<Input>) {
        if report.first() != Some(&INPUT_REPORT) || report.len() < MIN_INPUT_LEN {
            return;
        }
        // Bits 0 to 23: the push and the buttons; the touch bits are left out.
        let bits = u32::from_le_bytes([report[1], report[2], report[3], 0]);
        let changed = bits ^ self.held;
        if changed & 1 != 0 {
            events.push(Input::DialPress {
                index: 0,
                pressed: bits & 1 != 0,
            });
        }
        for index in 0..u8::try_from(KEY_NAMES.len()).unwrap_or(0) {
            let mask = 1u32 << (index + 1);
            if changed & mask != 0 {
                events.push(Input::Key {
                    index,
                    pressed: bits & mask != 0,
                });
            }
        }
        self.held = bits;
        let browse = u16::from(report[BROWSE_AT] & 0x0f);
        if let Some(old) = self.browse.replace(browse) {
            let delta = wrapped(browse, old, 16);
            if delta != 0 {
                events.push(Input::DialTurn { index: 0, delta });
            }
        }
        for (encoder, dial) in (0..ENCODERS).zip(1u8..) {
            let at = ENCODERS_AT + 2 * encoder;
            let (Some(low), Some(high)) = (report.get(at), report.get(at + 1)) else {
                return;
            };
            let value = u16::from_le_bytes([*low, *high]) & 0x03ff;
            if let Some(old) = self.encoders[encoder].replace(value) {
                let delta = wrapped(value, old, 1024);
                if delta != 0 {
                    events.push(Input::DialTurn { index: dial, delta });
                }
            }
        }
    }

    fn button_report(&self) -> Vec<u8> {
        let mut report = vec![0u8; BUTTON_LIGHTS_LEN + 1];
        report[0] = BUTTON_LIGHTS_REPORT;
        for (color, led) in self.colors.iter().zip(KEY_LIGHTS) {
            if let (Some([red, green, blue]), Some(slot)) = (color, report.get_mut(led + 1)) {
                *slot = level(*red.max(green).max(blue), self.brightness);
            }
        }
        report
    }

    fn guide_report(&self) -> Vec<u8> {
        let [red, green, blue] = self.guide.unwrap_or_default();
        let rgb = [red, green, blue].map(|channel| level(channel, self.brightness));
        let mut report = Vec::with_capacity(1 + 3 * self.guide_keys);
        report.push(GUIDE_REPORT);
        for _ in 0..self.guide_keys {
            report.extend_from_slice(&rgb);
        }
        report
    }
}

impl Driver for KompleteKontrol {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_started()?;
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.parse(buf.get(..read).unwrap_or_default(), &mut events);
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(slot) = self.colors.get_mut(usize::from(key)) {
            *slot = face.map(|face| face.color);
        }
        Ok(())
    }

    /// The light guide over the piano keys, in one colour.
    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        self.guide = color;
        Ok(())
    }

    /// Sends the button LEDs, then the light guide, each only when it changed.
    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        let buttons = self.button_report();
        if self.sent_buttons.as_ref() != Some(&buttons) {
            self.transport.write(&buttons)?;
            self.sent_buttons = Some(buttons);
        }
        if self.guide_keys > 0 {
            let guide = self.guide_report();
            if self.sent_guide.as_ref() != Some(&guide) {
                self.transport.write(&guide)?;
                self.sent_guide = Some(guide);
            }
        }
        Ok(())
    }

    /// No brightness command: Ryu scales every light value.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        let percent = percent.min(100);
        if percent == self.brightness {
            return Ok(());
        }
        self.brightness = percent;
        self.flush()
    }
}

impl Drop for KompleteKontrol {
    /// Leaves every light off when Ryu lets go.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        let mut buttons = vec![0u8; BUTTON_LIGHTS_LEN + 1];
        buttons[0] = BUTTON_LIGHTS_REPORT;
        let _ = self.transport.write(&buttons);
        if self.guide_keys > 0 {
            let mut guide = vec![0u8; 1 + 3 * self.guide_keys];
            guide[0] = GUIDE_REPORT;
            let _ = self.transport.write(&guide);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn open(id: &str) -> (KompleteKontrol, Recorder) {
        let recorder = Recorder::default();
        (
            KompleteKontrol::new(by_id(id).unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    fn turn(index: u8, delta: i8) -> Input {
        Input::DialTurn { index, delta }
    }

    fn color(rgb: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color: rgb,
            image: None,
        }
    }

    /// A 38-byte input report.
    fn report(buttons: [u8; 5], browse: u8, encoders: [u16; 8]) -> Vec<u8> {
        let mut report = vec![0x01];
        report.extend_from_slice(&buttons);
        report.push(browse);
        for value in encoders {
            report.extend_from_slice(&value.to_le_bytes());
        }
        report.resize(38, 0);
        report
    }

    #[test]
    fn the_first_poll_starts_the_control_section() {
        let (mut kk, recorder) = open(S25_ID);
        kk.poll(0).unwrap();
        kk.poll(0).unwrap();
        assert_eq!(recorder.writes(), vec![vec![0xa0, 0x00, 0x00]]);
    }

    #[test]
    fn buttons_follow_bits_one_to_twenty_three() {
        let (mut kk, recorder) = open(S49_ID);
        recorder.queue_read(report([0; 5], 0, [0; 8]));
        // Preset Up (bit 1), Octave Up (bit 7), Stop (bit 8), Shift (bit 16), Ffw (bit 23).
        recorder.queue_read(report([0x82, 0x01, 0x81, 0, 0], 0, [0; 8]));
        recorder.queue_read(report([0x02, 0, 0, 0, 0], 0, [0; 8]));
        assert_eq!(
            kk.poll(0).unwrap(),
            vec![
                key(0, true),
                key(6, true),
                key(7, true),
                key(15, true),
                key(22, true),
                key(6, false),
                key(7, false),
                key(15, false),
                key(22, false),
            ]
        );
    }

    #[test]
    fn touching_an_encoder_is_not_a_press() {
        let (mut kk, recorder) = open(S25_ID);
        recorder.queue_read(report([0; 5], 0, [0; 8]));
        recorder.queue_read(report([0, 0, 0, 0xff, 0x01], 0, [0; 8]));
        assert!(kk.poll(0).unwrap().is_empty());
    }

    #[test]
    fn the_browse_encoder_is_dial_zero() {
        let (mut kk, recorder) = open(S25_ID);
        recorder.queue_read(report([0; 5], 0x0e, [0; 8]));
        // 0x0e to 0x01 wraps forward three steps; the top bits are not read.
        recorder.queue_read(report([0x01, 0, 0, 0, 0], 0xf1, [0; 8]));
        recorder.queue_read(report([0; 5], 0x0f, [0; 8]));
        assert_eq!(
            kk.poll(0).unwrap(),
            vec![
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                turn(0, 3),
                Input::DialPress {
                    index: 0,
                    pressed: false
                },
                turn(0, -2),
            ]
        );
    }

    #[test]
    fn parameter_encoders_wrap_at_ten_bits() {
        let (mut kk, recorder) = open(S61_ID);
        let mut encoders = [0x0200u16; 8];
        encoders[0] = 0x03fe;
        encoders[7] = 0x0001;
        recorder.queue_read(report([0; 5], 0, encoders));
        // Encoder 1 wraps up by 4, encoder 8 down by 3, encoder 4 up by 5.
        encoders[0] = 0x0002;
        encoders[7] = 0x03fe;
        encoders[3] = 0x0205;
        recorder.queue_read(report([0; 5], 0, encoders));
        // Bits above the tenth are not read.
        encoders[3] |= 0xfc00;
        recorder.queue_read(report([0; 5], 0, encoders));
        assert_eq!(
            kk.poll(0).unwrap(),
            vec![turn(1, 4), turn(4, 5), turn(8, -3)]
        );
    }

    #[test]
    fn short_or_other_reports_are_ignored() {
        let (mut kk, recorder) = open(S25_ID);
        recorder.queue_read(vec![0x01, 0xff, 0xff]);
        recorder.queue_read(vec![0x02, 0xff, 0xff, 0xff, 0xff, 0xff, 0x05]);
        recorder.queue_read(vec![]);
        assert!(kk.poll(0).unwrap().is_empty());
        // Buttons and the browse encoder work without the encoder bytes.
        recorder.queue_read(vec![0x01, 0, 0, 0, 0, 0, 4]);
        recorder.queue_read(vec![0x01, 0x04, 0, 0, 0, 0, 5]);
        assert_eq!(kk.poll(0).unwrap(), vec![key(1, true), turn(0, 1)]);
    }

    #[test]
    fn button_lights_are_one_report_in_their_own_order() {
        let (mut kk, recorder) = open(S25_ID);
        // Enter, Shift, Octave Up and Ffw.
        kk.set_face(1, Some(&color([0x10, 0xfe, 0x20]))).unwrap();
        kk.set_face(15, Some(&color([0xff, 0, 0]))).unwrap();
        kk.set_face(6, Some(&color([0x80, 0x80, 0x80]))).unwrap();
        kk.set_face(22, Some(&color([0, 0, 0x40]))).unwrap();
        kk.flush().unwrap();
        kk.flush().unwrap();
        let writes = recorder.writes();
        // Start, the button LEDs, the (dark) light guide; nothing more.
        assert_eq!(writes.len(), 3);
        assert_eq!(writes[0], vec![0xa0, 0, 0]);
        let lights = &writes[1];
        assert_eq!((lights.len(), lights[0]), (26, 0x80));
        assert_eq!(lights[1 + 17], 0x7f);
        assert_eq!(lights[1], 0x7f);
        // Octave Up's white LED, not its red one.
        assert_eq!((lights[1 + 23], lights[1 + 24]), (0x40, 0));
        assert_eq!(lights[1 + 5], 0x20);
        let lit = lights.iter().skip(1).filter(|byte| **byte != 0).count();
        assert_eq!(lit, 4);
    }

    #[test]
    fn the_deck_light_fills_the_light_guide() {
        for (id, keys) in [(S25_ID, 25), (S49_ID, 49), (S61_ID, 61), (S88_ID, 88)] {
            let (mut kk, recorder) = open(id);
            kk.set_deck_light(Some([0xff, 0x80, 0x02])).unwrap();
            kk.flush().unwrap();
            let writes = recorder.writes();
            let guide = writes.last().unwrap();
            assert_eq!(guide.len(), 1 + 3 * keys, "{id}");
            assert_eq!(guide[0], 0x82);
            assert!(guide[1..].chunks(3).all(|rgb| rgb == [0x7f, 0x40, 0x01]));
        }
    }

    #[test]
    fn brightness_scales_and_unbound_keys_go_dark() {
        let (mut kk, recorder) = open(S25_ID);
        kk.set_face(0, Some(&color([0xc8, 0, 0]))).unwrap();
        kk.set_deck_light(Some([0xff, 0xff, 0xff])).unwrap();
        kk.flush().unwrap();
        kk.set_brightness(50).unwrap();
        kk.set_face(0, None).unwrap();
        kk.set_deck_light(None).unwrap();
        kk.flush().unwrap();
        let writes = recorder.writes();
        // Start, then both reports three times.
        assert_eq!(writes.len(), 7);
        assert_eq!(writes[1][1 + 12], 0x64);
        assert_eq!(&writes[2][1..4], &[0x7f, 0x7f, 0x7f]);
        assert_eq!(writes[3][1 + 12], 0x32);
        assert_eq!(&writes[4][1..4], &[0x3f, 0x3f, 0x3f]);
        assert_eq!(writes[5][1 + 12], 0);
        assert!(writes[6][1..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn dropping_the_deck_turns_every_light_off() {
        let (mut kk, recorder) = open(S88_ID);
        kk.set_face(3, Some(&color([0xff, 0xff, 0xff]))).unwrap();
        kk.set_deck_light(Some([0xff, 0, 0])).unwrap();
        kk.flush().unwrap();
        drop(kk);
        let writes = recorder.writes();
        let [buttons, guide] = &writes[writes.len() - 2..] else {
            panic!("two blanking writes");
        };
        assert_eq!((buttons.len(), buttons[0]), (26, 0x80));
        assert_eq!((guide.len(), guide[0]), (265, 0x82));
        assert!(buttons[1..]
            .iter()
            .chain(&guide[1..])
            .all(|byte| *byte == 0));
    }

    #[test]
    fn every_key_has_its_own_light_and_place() {
        let lights: std::collections::HashSet<_> = KEY_LIGHTS.iter().collect();
        assert_eq!(lights.len(), 23);
        assert!(KEY_LIGHTS.iter().all(|led| *led < BUTTON_LIGHTS_LEN));
        // The octave buttons' red LEDs are the two left over.
        assert!(!lights.contains(&22) && !lights.contains(&24));
        let places: std::collections::HashSet<_> = KEY_PLACES.iter().collect();
        assert_eq!(places.len(), 23);
    }
}
