//! Contour ShuttleXpress and ShuttlePro: buttons, a jog wheel and a shuttle ring,
//! with no screens or lights.
//!
//! An input report is `[shuttle, jog, _, buttons (u16 LE)]`. The shuttle is its
//! ring's position (-7 to 7, springing back to 0); the jog is a counter that wraps
//! at 256. Both are raised as dial turns: the jog by the counter's change, the
//! shuttle by how far the ring moved. The first report only sets the baseline, so
//! plugging the controller in does not read as a turn.

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const READ_BUFFER: usize = 16;
const MAX_REPORTS_PER_POLL: usize = 16;

pub struct Contour {
    transport: Box<dyn Transport>,
    /// The bit of each key, in key order.
    bits: &'static [u8],
    keys: Vec<bool>,
    jog: Option<u8>,
    shuttle: i8,
}

impl Contour {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let bits: &'static [u8] = match model.id {
            "contour-shuttlexpress" => &[4, 5, 6, 7, 8],
            "contour-shuttlepro-v1" => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
            _ => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
        };
        Self {
            transport,
            bits,
            keys: vec![false; usize::from(model.keys)],
            jog: None,
            shuttle: 0,
        }
    }

    fn parse(&mut self, report: &[u8]) -> Vec<Input> {
        let (Some(&shuttle), Some(&jog), Some(&low), Some(&high)) =
            (report.first(), report.get(1), report.get(3), report.get(4))
        else {
            return Vec::new();
        };
        let mut events = Vec::new();
        let buttons = u16::from_le_bytes([low, high]);
        for (index, (bit, held)) in self.bits.iter().zip(self.keys.iter_mut()).enumerate() {
            let pressed = buttons & (1 << bit) != 0;
            if pressed != *held {
                *held = pressed;
                events.push(Input::Key {
                    index: index as u8,
                    pressed,
                });
            }
        }
        if let Some(previous) = self.jog.replace(jog) {
            // The counter wraps, so the shortest way round is the turn.
            let delta = i8::from_le_bytes([jog.wrapping_sub(previous)]);
            if delta != 0 {
                events.push(Input::DialTurn { index: 0, delta });
            }
        }
        let position = i8::from_le_bytes([shuttle]);
        let moved = position.saturating_sub(self.shuttle);
        self.shuttle = position;
        if moved != 0 {
            events.push(Input::DialTurn {
                index: 1,
                delta: moved,
            });
        }
        events
    }
}

impl Driver for Contour {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            events.extend(self.parse(&buf[..read]));
        }
        Ok(events)
    }

    /// No screens and no lights: a layout only marks the keys in the Companion.
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

    fn driver(model: &str) -> (Contour, Recorder) {
        let recorder = Recorder::default();
        (
            Contour::new(by_id(model).unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    #[test]
    fn jog_wraps_shuttle_moves_and_buttons_follow_their_bits() {
        let (mut deck, recorder) = driver("contour-shuttlexpress");
        // The first report sets the jog baseline (250) and presses button 2 (bit 6).
        recorder.queue_read(vec![0, 250, 0, 0b0100_0000, 0]);
        // Jog 250 -> 2 is eight steps clockwise across the wrap; shuttle to -3.
        recorder.queue_read(vec![0xfd, 2, 0, 0, 0]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 2,
                    pressed: true
                },
                Input::Key {
                    index: 2,
                    pressed: false
                },
                Input::DialTurn { index: 0, delta: 8 },
                Input::DialTurn {
                    index: 1,
                    delta: -3
                },
            ]
        );
        assert!(recorder.writes().is_empty(), "nothing to draw or light");
    }

    #[test]
    fn the_shuttlepro_has_fifteen_buttons() {
        let (mut deck, recorder) = driver("contour-shuttlepro-v2");
        recorder.queue_read(vec![0, 0, 0, 0, 0b0100_0000]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 14,
                pressed: true
            }]
        );
    }
}
