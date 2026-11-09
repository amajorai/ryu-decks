//! Sony Buzz! quiz buzzers: four handsets on one USB hub or wireless receiver,
//! each with a big red Buzz! dome and four coloured answer buttons. No screens;
//! each dome has one red LED that is either on or off.
//!
//! Input is a plain HID report with no report id, at least five bytes long:
//! bytes 2, 3 and 4 hold a little-endian bitmask of all twenty buttons, five
//! bits per handset in the order red dome, yellow, green, orange, blue. Bits 20
//! to 23 are not buttons and are masked off, and shorter reads are dropped. Each
//! report is the whole state, so Ryu raises a press or a release only when a
//! button's bit changes, starting with every button up.
//!
//! Ryu numbers the keys by handset (one row each) and, within a handset, from
//! the top of the handset down: the dome, then blue, orange, green and yellow.
//!
//! The only output is the four dome LEDs, all in one report with no report id:
//! seven bytes `00 L1 L2 L3 L4 00 00`, each `Ln` 0xFF for on and 0x00 for off.
//! The LEDs go off when Ryu lets go of the buzzers.

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const READ_BUFFER: usize = 16;
const MAX_REPORTS_PER_POLL: usize = 16;
/// The shortest report that carries every button.
const MIN_REPORT_LEN: usize = 5;
/// The twenty button bits; the top nibble of byte 4 is something else.
const BUTTON_MASK: u32 = 0x000F_FFFF;
const HANDSETS: usize = 4;
const BUTTONS_PER_HANDSET: usize = 5;
const KEYS: usize = HANDSETS * BUTTONS_PER_HANDSET;
/// A dome LED is lit when its colour's brightest channel is above this.
const ON_OFF_THRESHOLD: u8 = 16;
const LED_ON: u8 = 0xFF;

/// The column (top to bottom on the handset) of each wire bit within a
/// handset's group: red dome, yellow, green, orange, blue on the wire.
const WIRE_TO_COL: [u8; BUTTONS_PER_HANDSET] = [0, 4, 3, 2, 1];

/// The red Buzz! domes, the only keys with a light.
pub const LIT_KEYS: [u8; HANDSETS] = [0, 5, 10, 15];

/// The key names, in Ryu's key order.
pub const KEY_NAMES: [&str; KEYS] = [
    "P1 Buzz",
    "P1 Blue",
    "P1 Orange",
    "P1 Green",
    "P1 Yellow",
    "P2 Buzz",
    "P2 Blue",
    "P2 Orange",
    "P2 Green",
    "P2 Yellow",
    "P3 Buzz",
    "P3 Blue",
    "P3 Orange",
    "P3 Green",
    "P3 Yellow",
    "P4 Buzz",
    "P4 Blue",
    "P4 Orange",
    "P4 Green",
    "P4 Yellow",
];

/// Ryu's key for wire bit `bit` (0 to 19).
fn key_for_bit(bit: usize) -> u8 {
    let handset = bit / BUTTONS_PER_HANDSET;
    let col = WIRE_TO_COL[bit % BUTTONS_PER_HANDSET];
    // At most 3 * 5 + 4 = 19.
    (handset * BUTTONS_PER_HANDSET) as u8 + col
}

/// The hidapi write for these dome LEDs (bit n is handset n + 1): the report
/// id byte hidapi expects, then the seven-byte report.
fn led_report(leds: u8) -> [u8; 8] {
    let lit = |handset: u8| {
        if leds & (1 << handset) != 0 {
            LED_ON
        } else {
            0
        }
    };
    [0x00, 0x00, lit(0), lit(1), lit(2), lit(3), 0x00, 0x00]
}

pub struct Buzz {
    transport: Box<dyn Transport>,
    held: [bool; KEYS],
    /// Staged dome LEDs, bit n for handset n + 1.
    leds: u8,
    sent_leds: Option<u8>,
}

impl Buzz {
    #[must_use]
    pub fn new(_model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            held: [false; KEYS],
            leds: 0,
            sent_leds: None,
        }
    }

    fn parse(&mut self, report: &[u8]) -> Vec<Input> {
        if report.len() < MIN_REPORT_LEN {
            return Vec::new();
        }
        let byte = |index: usize| u32::from(report.get(index).copied().unwrap_or(0));
        let buttons = (byte(2) | byte(3) << 8 | byte(4) << 16) & BUTTON_MASK;
        let mut events = Vec::new();
        for bit in 0..KEYS {
            let pressed = buttons & (1 << bit) != 0;
            let index = key_for_bit(bit);
            let Some(held) = self.held.get_mut(usize::from(index)) else {
                continue;
            };
            if pressed != *held {
                *held = pressed;
                events.push(Input::Key { index, pressed });
            }
        }
        events
    }

    /// Writes the LEDs; a receiver that refuses the full report is sent the
    /// shorter form without the two trailing zeros.
    fn write_leds(&mut self, leds: u8) -> DriverResult<()> {
        let report = led_report(leds);
        if self.transport.write(&report).is_ok() {
            return Ok(());
        }
        self.transport.write(report.get(..6).unwrap_or(&report))
    }
}

impl Driver for Buzz {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            events.extend(self.parse(buf.get(..read).unwrap_or_default()));
        }
        Ok(events)
    }

    /// The red domes take an on or off LED from the colour; the answer buttons
    /// have no light.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let Some(handset) = LIT_KEYS.iter().position(|lit| *lit == key) else {
            return Ok(());
        };
        let [r, g, b] = face.map_or([0, 0, 0], |face| face.color);
        let mask = 1u8 << handset;
        if r.max(g).max(b) > ON_OFF_THRESHOLD {
            self.leds |= mask;
        } else {
            self.leds &= !mask;
        }
        Ok(())
    }

    /// Sends the four LEDs together, only when one changed.
    fn flush(&mut self) -> DriverResult<()> {
        if self.sent_leds == Some(self.leds) {
            return Ok(());
        }
        self.write_leds(self.leds)?;
        self.sent_leds = Some(self.leds);
        Ok(())
    }

    /// The LEDs are only on or off.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for Buzz {
    /// Turns every dome off when Ryu lets go of the buzzers.
    fn drop(&mut self) {
        let _ = self.write_leds(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn buzzers() -> (Buzz, Recorder) {
        let recorder = Recorder::default();
        (
            Buzz::new(by_id("sony-buzz").unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    fn face(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    #[test]
    fn each_handset_is_a_row_in_top_to_bottom_order() {
        let (mut deck, recorder) = buzzers();
        // Handset 1's red dome.
        recorder.queue_read(vec![0x00, 0x00, 0x01, 0x00, 0x00]);
        // Same state again: nothing new.
        recorder.queue_read(vec![0x7f, 0x7f, 0x01, 0x00, 0x00]);
        // Dome up; handset 1's yellow (bit 1) and blue (bit 4) down.
        recorder.queue_read(vec![0x00, 0x00, 0x12, 0x00, 0x00]);
        // All up, then handset 4's blue (bit 19).
        recorder.queue_read(vec![0x00, 0x00, 0x00, 0x00, 0x00]);
        recorder.queue_read(vec![0x00, 0x00, 0x00, 0x00, 0x08]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(4, true),
                key(1, true),
                key(4, false),
                key(1, false),
                key(16, true),
            ]
        );
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn every_wire_bit_lands_on_its_named_key() {
        // Wire order within a handset: red, yellow, green, orange, blue.
        let colours = ["Buzz", "Yellow", "Green", "Orange", "Blue"];
        for bit in 0..KEYS {
            let (mut deck, recorder) = buzzers();
            let [b2, b3, b4, _] = (1u32 << bit).to_le_bytes();
            recorder.queue_read(vec![0, 0, b2, b3, b4]);
            let events = deck.poll(0).unwrap();
            let [Input::Key {
                index,
                pressed: true,
            }] = events.as_slice()
            else {
                panic!("bit {bit}: {events:?}");
            };
            let expected = format!("P{} {}", bit / 5 + 1, colours[bit % 5]);
            assert_eq!(KEY_NAMES[usize::from(*index)], expected, "bit {bit}");
        }
        // Handset 3's dome and handset 2's green.
        assert_eq!((key_for_bit(10), key_for_bit(7)), (10, 8));
    }

    #[test]
    fn the_top_nibble_and_short_reads_are_ignored() {
        let (mut deck, recorder) = buzzers();
        recorder.queue_read(vec![0x00, 0x00, 0x00, 0x00, 0xf0]);
        recorder.queue_read(vec![0x00, 0x00, 0xff, 0xff]);
        recorder.queue_read(vec![0x00]);
        recorder.queue_read(vec![0x00, 0x00, 0x20, 0x00, 0xf0, 0x00]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(5, true)]);
    }

    #[test]
    fn only_the_domes_light_and_only_changes_are_sent() {
        let (mut deck, recorder) = buzzers();
        deck.set_face(0, Some(&face([255, 0, 0]))).unwrap();
        // Answer buttons have no light; a dim colour is off.
        deck.set_face(1, Some(&face([255, 255, 255]))).unwrap();
        deck.set_face(5, Some(&face([16, 16, 16]))).unwrap();
        deck.flush().unwrap();
        deck.flush().unwrap();
        deck.set_face(15, Some(&face([0, 0, 200]))).unwrap();
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        deck.set_brightness(10).unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x00, 0x00, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00],
                vec![0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x00, 0x00],
            ]
        );
    }

    #[test]
    fn the_first_flush_sends_even_all_off_and_drop_turns_them_off() {
        let (mut deck, recorder) = buzzers();
        deck.flush().unwrap();
        deck.set_face(10, Some(&face([0, 40, 0]))).unwrap();
        deck.flush().unwrap();
        drop(deck);
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x00; 8],
                vec![0x00, 0x00, 0x00, 0x00, 0xff, 0x00, 0x00, 0x00],
                vec![0x00; 8],
            ]
        );
    }
}
