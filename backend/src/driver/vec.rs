//! VEC footpedal (made by P.I. Engineering): three pedals, no screens or lights.
//!
//! Every input report is the pedals' whole state: two bytes with no report id,
//! read as a little-endian bitmask where bit 0 is key 0, bit 1 key 1 and bit 2
//! key 2. Ryu raises a press or a release only when a pedal's bit changes, and
//! starts with every pedal up. Reads of any other length are dropped, and
//! nothing is ever written to the pedal.

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const READ_BUFFER: usize = 16;
const MAX_REPORTS_PER_POLL: usize = 16;
const REPORT_LEN: usize = 2;
const PEDALS: usize = 3;

pub struct Footpedal {
    transport: Box<dyn Transport>,
    held: [bool; PEDALS],
}

impl Footpedal {
    #[must_use]
    pub fn new(_model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            held: [false; PEDALS],
        }
    }

    fn parse(&mut self, report: &[u8]) -> Vec<Input> {
        // The reference drops every read that is not exactly the 2-byte report.
        let (REPORT_LEN, Some(&low), Some(&high)) = (report.len(), report.first(), report.get(1))
        else {
            return Vec::new();
        };
        let pedals = u16::from_le_bytes([low, high]);
        let mut events = Vec::new();
        for (index, held) in self.held.iter_mut().enumerate() {
            let pressed = pedals & (1 << index) != 0;
            if pressed != *held {
                *held = pressed;
                events.push(Input::Key {
                    index: index as u8,
                    pressed,
                });
            }
        }
        events
    }
}

impl Driver for Footpedal {
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

    /// No screens and no lights: a layout only marks the pedals in the Companion.
    fn set_face(&mut self, _key: u8, _face: Option<&Face>) -> DriverResult<()> {
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        Ok(())
    }

    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn pedal() -> (Footpedal, Recorder) {
        let recorder = Recorder::default();
        (
            Footpedal::new(by_id("vec-footpedal").unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    #[test]
    fn each_bit_is_a_pedal_and_only_changes_are_raised() {
        let (mut deck, recorder) = pedal();
        // Left pedal (bit 0) down.
        recorder.queue_read(vec![0x01, 0x00]);
        // Repeated state: nothing new.
        recorder.queue_read(vec![0x01, 0x00]);
        // Left up, middle and right down; bits above 2 are ignored.
        recorder.queue_read(vec![0b1111_0110, 0x80]);
        // All up.
        recorder.queue_read(vec![0x00, 0x00]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(1, true),
                key(2, true),
                key(1, false),
                key(2, false),
            ]
        );
        assert!(
            recorder.writes().is_empty(),
            "nothing is written to the pedal"
        );
    }

    #[test]
    fn reads_of_any_other_length_are_dropped() {
        let (mut deck, recorder) = pedal();
        recorder.queue_read(vec![0x07]);
        recorder.queue_read(vec![0x07, 0x00, 0x00]);
        recorder.queue_read(vec![0x04, 0x00]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(2, true)]);
    }

    #[test]
    fn faces_lights_and_brightness_write_nothing() {
        let (mut deck, recorder) = pedal();
        deck.set_face(0, None).unwrap();
        deck.set_brightness(40).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
    }
}
