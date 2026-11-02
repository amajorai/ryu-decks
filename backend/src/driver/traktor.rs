//! Native Instruments Traktor Kontrol F1 (17cc:1120) in HID mode.
//!
//! The F1 is one HID collection (interface 1) with no handshake or keep-alive.
//! Every input report starts with its report id, and every write is the report
//! id followed by its payload; byte offsets below count the id as byte 0.
//!
//! - Input report `01` (22 bytes): bytes 1 to 4 are a button bitfield. Byte 1
//!   holds grid pads 1 to 8 from bit 7 down to bit 0, byte 2 pads 9 to 16 the
//!   same way; byte 3 bit 2 is the encoder push, then Browse, Size, Type,
//!   Reverse and Shift at bits 3 to 7; byte 4 bits 1 to 7 are Capture, Quant,
//!   Sync, Stop 4, Stop 3, Stop 2 and Stop 1. Byte 5 is the select encoder, an
//!   8-bit counter that wraps. Bytes 6 to 21 are eight little-endian 12-bit
//!   readings: the four knobs, then the four faders.
//! - Output report `80` (80 payload bytes, 0 to 0x7F each) is every light at
//!   once: the 7-segment display at bytes 1 to 16 (left dark), Browse to Sync at
//!   17 to 24, the sixteen pads at 25 + 3 x (pad - 1) as blue, red, green, and
//!   two LEDs for each Stop button at 73 to 80, Stop 4 first.
//!
//! Ryu pairs input grid pad n with LED pad n and takes pad 1 as the top-left
//! pad, row by row; the physical corner is not confirmed on hardware. The
//! encoder and the absolute knobs and faders become dial turns, each seeded
//! from the first report. There is no brightness command, so Ryu scales the
//! light values, and blanks every light when it lets go.
//!
//! Facts per shaduzlabs/cabl (MIT), cross-checked against the Traktor Kontrol
//! F1 mapping in mixxxdj/mixxx (GPL-2.0-or-later); no code was copied.

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const F1_ID: &str = "ni-traktor-kontrol-f1";

const INPUT_REPORT: u8 = 0x01;
const LIGHTS_REPORT: u8 = 0x80;
/// Light bytes after the report id.
const LIGHTS_LEN: usize = 80;
/// Bigger than the 22-byte input report.
const READ_BUFFER: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 64;
/// The shortest report that carries every button and the encoder.
const MIN_INPUT_LEN: usize = 6;
const ENCODER_AT: usize = 5;
/// The first knob reading; the four knobs then the four faders follow, two bytes each.
const ANALOG_AT: usize = 6;
const ANALOG_CONTROLS: usize = 8;
/// Units of a knob or fader (of 4096) per dial step: its full travel is 64
/// steps, and ADC jitter of a few units never adds up to one.
const ANALOG_STEP: i32 = 64;
/// The encoder push, a bit of the bitfield (byte 3, bit 2).
const ENCODER_PUSH_BIT: u8 = 18;
/// The first pad's blue LED.
const PAD_LIGHTS_AT: usize = 25;
/// Browse's light; Size to Sync follow.
const BUTTON_LIGHTS_AT: usize = 17;
/// Stop 4's first LED; each Stop button has two, Stop 3 to Stop 1 after it.
const STOP_LIGHTS_AT: usize = 73;

/// The keys: the sixteen pads, top-left first, then the twelve buttons.
pub const KEY_NAMES: [&str; 28] = [
    "Pad 1", "Pad 2", "Pad 3", "Pad 4", "Pad 5", "Pad 6", "Pad 7", "Pad 8", "Pad 9", "Pad 10",
    "Pad 11", "Pad 12", "Pad 13", "Pad 14", "Pad 15", "Pad 16", "Browse", "Size", "Type",
    "Reverse", "Shift", "Capture", "Quant", "Sync", "Stop 1", "Stop 2", "Stop 3", "Stop 4",
];

/// Each key's row and column on the Companion's 7 x 4 grid: the eight buttons
/// above the pads, the Stop buttons below them.
pub const KEY_PLACES: [(u8, u8); 28] = [
    (2, 0),
    (2, 1),
    (2, 2),
    (2, 3),
    (3, 0),
    (3, 1),
    (3, 2),
    (3, 3),
    (4, 0),
    (4, 1),
    (4, 2),
    (4, 3),
    (5, 0),
    (5, 1),
    (5, 2),
    (5, 3),
    (0, 0),
    (0, 1),
    (0, 2),
    (0, 3),
    (1, 0),
    (1, 1),
    (1, 2),
    (1, 3),
    (6, 0),
    (6, 1),
    (6, 2),
    (6, 3),
];

/// Each key's bit in the bitfield, counting from bit 0 of byte 1.
const fn key_bit(key: u8) -> u8 {
    match key {
        // Pad n is bit 7 - (n - 1) % 8 of byte 1 + (n - 1) / 8.
        0..=15 => 8 * (key / 8) + 7 - key % 8,
        // Browse, Size, Type, Reverse, Shift: byte 3, bits 3 to 7.
        16..=20 => 16 + 3 + (key - 16),
        // Capture, Quant, Sync: byte 4, bits 1 to 3.
        21..=23 => 24 + 1 + (key - 21),
        // Stop 1 to Stop 4: byte 4, bits 7 down to 4.
        _ => 24 + 7 - (key - 24),
    }
}

/// The light bytes of a key, with the id at byte 0: three (blue, red, green)
/// for a pad, one for Browse to Sync, two for a Stop button.
fn key_lights(key: u8) -> &'static [usize] {
    const fn table() -> [[usize; 3]; 28] {
        let mut table = [[0; 3]; 28];
        let mut key = 0;
        while key < 28 {
            table[key] = if key < 16 {
                let at = PAD_LIGHTS_AT + 3 * key;
                [at, at + 1, at + 2]
            } else if key < 24 {
                [BUTTON_LIGHTS_AT + key - 16, 0, 0]
            } else {
                let at = STOP_LIGHTS_AT + 2 * (27 - key);
                [at, at + 1, 0]
            };
            key += 1;
        }
        table
    }
    static TABLE: [[usize; 3]; 28] = table();
    let len = match key {
        0..=15 => 3,
        16..=23 => 1,
        _ => 2,
    };
    TABLE
        .get(usize::from(key))
        .and_then(|bytes| bytes.get(..len))
        .unwrap_or_default()
}

/// An 8-bit colour channel as a 7-bit light value, scaled by a percentage.
fn level(channel: u8, percent: u16) -> u8 {
    u8::try_from(u16::from(channel >> 1) * percent.min(100) / 100).unwrap_or(0x7f)
}

pub struct TraktorF1 {
    transport: Box<dyn Transport>,
    /// Each key's light colour before brightness; `None` for off.
    colors: [Option<[u8; 3]>; 28],
    brightness: u8,
    /// The light report last sent.
    sent: Option<Vec<u8>>,
    /// The bitfield at the last report.
    held: u32,
    encoder: Option<u8>,
    analog: [Option<u16>; ANALOG_CONTROLS],
    /// Knob and fader movement not yet turned into a whole step.
    analog_rest: [i32; ANALOG_CONTROLS],
}

impl TraktorF1 {
    #[must_use]
    pub fn new(_model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            colors: [None; 28],
            brightness: 100,
            sent: None,
            held: 0,
            encoder: None,
            analog: [None; ANALOG_CONTROLS],
            analog_rest: [0; ANALOG_CONTROLS],
        }
    }

    fn parse(&mut self, report: &[u8], events: &mut Vec<Input>) {
        if report.first() != Some(&INPUT_REPORT) || report.len() < MIN_INPUT_LEN {
            return;
        }
        let bits = u32::from_le_bytes([report[1], report[2], report[3], report[4]]);
        let changed = bits ^ self.held;
        for index in 0..u8::try_from(KEY_NAMES.len()).unwrap_or(0) {
            let mask = 1u32 << key_bit(index);
            if changed & mask != 0 {
                events.push(Input::Key {
                    index,
                    pressed: bits & mask != 0,
                });
            }
        }
        let push = 1u32 << ENCODER_PUSH_BIT;
        if changed & push != 0 {
            events.push(Input::DialPress {
                index: 0,
                pressed: bits & push != 0,
            });
        }
        self.held = bits;
        let value = report[ENCODER_AT];
        if let Some(old) = self.encoder.replace(value) {
            let delta = i8::from_ne_bytes([value.wrapping_sub(old)]);
            if delta != 0 {
                events.push(Input::DialTurn { index: 0, delta });
            }
        }
        self.parse_analog(report, events);
    }

    /// The knobs (dial indices 1 to 4) and faders (5 to 8): every
    /// `ANALOG_STEP` units moved is one step.
    fn parse_analog(&mut self, report: &[u8], events: &mut Vec<Input>) {
        for (control, dial) in (0..ANALOG_CONTROLS).zip(1u8..) {
            let at = ANALOG_AT + 2 * control;
            let (Some(low), Some(high)) = (report.get(at), report.get(at + 1)) else {
                return;
            };
            let value = u16::from_le_bytes([*low, *high]) & 0x0fff;
            let Some(old) = self.analog[control].replace(value) else {
                continue;
            };
            let moved = self.analog_rest[control] + i32::from(value) - i32::from(old);
            let steps = moved / ANALOG_STEP;
            self.analog_rest[control] = moved - steps * ANALOG_STEP;
            if steps != 0 {
                events.push(Input::DialTurn {
                    index: dial,
                    delta: i8::try_from(steps.clamp(-127, 127)).unwrap_or(0),
                });
            }
        }
    }

    /// The whole light report, from the key colours and the brightness.
    fn light_report(&self) -> Vec<u8> {
        let mut report = vec![0u8; LIGHTS_LEN + 1];
        report[0] = LIGHTS_REPORT;
        let percent = u16::from(self.brightness);
        for (key, color) in (0u8..).zip(&self.colors) {
            let Some([red, green, blue]) = *color else {
                continue;
            };
            let bytes = key_lights(key);
            let values = if bytes.len() == 3 {
                // Pads take blue, red, green.
                vec![blue, red, green]
            } else {
                // One-colour LEDs glow with the brightest channel.
                vec![red.max(green).max(blue); bytes.len()]
            };
            for (at, value) in bytes.iter().zip(values) {
                if let Some(slot) = report.get_mut(*at) {
                    *slot = level(value, percent);
                }
            }
        }
        report
    }
}

impl Driver for TraktorF1 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
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

    /// Sends the light report, whole, when it changed.
    fn flush(&mut self) -> DriverResult<()> {
        let report = self.light_report();
        if self.sent.as_ref() != Some(&report) {
            self.transport.write(&report)?;
            self.sent = Some(report);
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

impl Drop for TraktorF1 {
    /// Leaves every light off when Ryu lets go.
    fn drop(&mut self) {
        let mut report = vec![0u8; LIGHTS_LEN + 1];
        report[0] = LIGHTS_REPORT;
        let _ = self.transport.write(&report);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn open() -> (TraktorF1, Recorder) {
        let recorder = Recorder::default();
        (
            TraktorF1::new(by_id(F1_ID).unwrap(), Box::new(recorder.clone())),
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

    /// An input report with the bitfield bytes, encoder and the eight readings.
    fn report(buttons: [u8; 4], encoder: u8, analog: [u16; 8]) -> Vec<u8> {
        let mut report = vec![0x01];
        report.extend_from_slice(&buttons);
        report.push(encoder);
        for value in analog {
            report.extend_from_slice(&value.to_le_bytes());
        }
        assert_eq!(report.len(), 22);
        report
    }

    #[test]
    fn pads_and_buttons_follow_the_bitfield() {
        let (mut f1, recorder) = open();
        recorder.queue_read(report([0; 4], 0, [0; 8]));
        // Pad 1 (byte 1 bit 7), pad 8 (bit 0), pad 9 (byte 2 bit 7), pad 16 (bit 0).
        recorder.queue_read(report([0x81, 0x81, 0, 0], 0, [0; 8]));
        // Browse (byte 3 bit 3), Shift (bit 7), Capture (byte 4 bit 1), Sync (bit 3).
        recorder.queue_read(report([0, 0, 0x88, 0x0a], 0, [0; 8]));
        assert_eq!(
            f1.poll(0).unwrap(),
            vec![
                key(0, true),
                key(7, true),
                key(8, true),
                key(15, true),
                key(0, false),
                key(7, false),
                key(8, false),
                key(15, false),
                key(16, true),
                key(20, true),
                key(21, true),
                key(23, true),
            ]
        );
    }

    #[test]
    fn stop_buttons_run_from_bit_seven_down() {
        let (mut f1, recorder) = open();
        // Stop 1 is byte 4 bit 7, Stop 4 bit 4.
        recorder.queue_read(report([0, 0, 0, 0x80], 0, [0; 8]));
        recorder.queue_read(report([0, 0, 0, 0x10], 0, [0; 8]));
        assert_eq!(
            f1.poll(0).unwrap(),
            vec![key(24, true), key(24, false), key(27, true)]
        );
    }

    #[test]
    fn the_encoder_wraps_and_pushes_dial_zero() {
        let (mut f1, recorder) = open();
        recorder.queue_read(report([0; 4], 0xfe, [0; 8]));
        recorder.queue_read(report([0; 4], 0x01, [0; 8]));
        recorder.queue_read(report([0, 0, 0x04, 0], 0xff, [0; 8]));
        recorder.queue_read(report([0; 4], 0xff, [0; 8]));
        assert_eq!(
            f1.poll(0).unwrap(),
            vec![
                turn(0, 3),
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                turn(0, -2),
                Input::DialPress {
                    index: 0,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn knobs_and_faders_step_every_64_units() {
        let (mut f1, recorder) = open();
        let mut analog = [2048u16; 8];
        recorder.queue_read(report([0; 4], 0, analog));
        // Jitter never turns a dial.
        analog[0] = 2050;
        recorder.queue_read(report([0; 4], 0, analog));
        analog[0] = 2046;
        recorder.queue_read(report([0; 4], 0, analog));
        // Knob 1 up two steps; fader 4 (dial 8) down one.
        analog[0] = 2048 + 128;
        analog[7] = 2048 - 70;
        recorder.queue_read(report([0; 4], 0, analog));
        // The top four bits are not part of the reading.
        analog[0] |= 0xf000;
        recorder.queue_read(report([0; 4], 0, analog));
        assert_eq!(f1.poll(0).unwrap(), vec![turn(1, 2), turn(8, -1)]);
    }

    #[test]
    fn short_or_other_reports_are_ignored() {
        let (mut f1, recorder) = open();
        recorder.queue_read(vec![0x01, 0xff, 0xff]);
        recorder.queue_read(vec![0x02, 0xff, 0xff, 0xff, 0xff, 0x10]);
        recorder.queue_read(vec![]);
        assert!(f1.poll(0).unwrap().is_empty());
        // Buttons and the encoder work without the analog bytes.
        recorder.queue_read(vec![0x01, 0, 0, 0, 0, 7]);
        recorder.queue_read(vec![0x01, 0x80, 0, 0, 0, 8]);
        assert_eq!(f1.poll(0).unwrap(), vec![key(0, true), turn(0, 1)]);
    }

    #[test]
    fn lights_are_one_whole_report_sent_when_it_changes() {
        let (mut f1, recorder) = open();
        f1.set_face(0, Some(&color([0xff, 0x80, 0x02]))).unwrap();
        f1.set_face(15, Some(&color([0, 0, 0xff]))).unwrap();
        f1.set_face(16, Some(&color([0x10, 0xfe, 0x20]))).unwrap();
        f1.set_face(24, Some(&color([0xff, 0, 0]))).unwrap();
        f1.set_face(27, Some(&color([0, 0x40, 0]))).unwrap();
        f1.flush().unwrap();
        f1.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 1);
        let lights = &writes[0];
        assert_eq!(lights.len(), 81);
        assert_eq!(lights[0], 0x80);
        // Pad 1: blue, red, green, each halved to 7 bits.
        assert_eq!(&lights[25..28], &[0x01, 0x7f, 0x40]);
        // Pad 16 at 25 + 45.
        assert_eq!(&lights[70..73], &[0x7f, 0, 0]);
        // Browse takes its brightest channel.
        assert_eq!(lights[17], 0x7f);
        // Stop 1 is the last pair, Stop 4 the first.
        assert_eq!(&lights[79..81], &[0x7f, 0x7f]);
        assert_eq!(&lights[73..75], &[0x20, 0x20]);
        // The 7-segment display stays dark.
        assert!(lights[1..17].iter().all(|byte| *byte == 0));
        let lit = lights.iter().skip(1).filter(|byte| **byte != 0).count();
        assert_eq!(lit, 3 + 1 + 1 + 2 + 2);
    }

    #[test]
    fn brightness_scales_and_unbound_keys_go_dark() {
        let (mut f1, recorder) = open();
        f1.set_face(3, Some(&color([0xff, 0xff, 0xff]))).unwrap();
        f1.set_face(18, Some(&color([0xc8, 0, 0]))).unwrap();
        f1.flush().unwrap();
        f1.set_brightness(50).unwrap();
        f1.set_face(18, None).unwrap();
        f1.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 3);
        assert_eq!(&writes[0][34..37], &[0x7f, 0x7f, 0x7f]);
        assert_eq!(writes[0][19], 0x64);
        assert_eq!(&writes[1][34..37], &[0x3f, 0x3f, 0x3f]);
        assert_eq!(writes[1][19], 0x32);
        assert_eq!(writes[2][19], 0);
    }

    #[test]
    fn dropping_the_deck_turns_every_light_off() {
        let (mut f1, recorder) = open();
        f1.set_face(0, Some(&color([0xff, 0xff, 0xff]))).unwrap();
        f1.flush().unwrap();
        drop(f1);
        let writes = recorder.writes();
        let last = writes.last().unwrap();
        assert_eq!(last.len(), 81);
        assert_eq!(last[0], 0x80);
        assert!(last[1..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn every_key_has_its_own_bit_and_lights() {
        let bits: std::collections::HashSet<u8> = (0..28).map(key_bit).collect();
        assert_eq!(bits.len(), 28);
        assert!(!bits.contains(&ENCODER_PUSH_BIT));
        let mut lights: Vec<usize> = (0..28).flat_map(|key| key_lights(key).to_vec()).collect();
        lights.sort_unstable();
        assert_eq!(lights, (17..=80).collect::<Vec<_>>());
        let places: std::collections::HashSet<_> = KEY_PLACES.iter().collect();
        assert_eq!(places.len(), KEY_PLACES.len());
    }
}
