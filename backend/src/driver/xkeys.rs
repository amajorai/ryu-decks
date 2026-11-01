//! P.I. Engineering X-keys keypads: keys with backlights, no screens.
//!
//! An input report is `[unit id, status, button bytes..]`: each button byte is a
//! column and each bit a row, from byte 2. Status values above 3 are special
//! replies, not key states. Output reports are 36 bytes led by report id 0. The
//! backlight command depends on the model: legacy panels have a blue and a red
//! bank (on or off per key), the XK-24M-RGB takes green-red-blue per key, and the
//! X-blox modules take full RGB for each of a key's two LEDs. A jog wheel reports a
//! signed step per report; a shuttle ring reports its position (-7 to 7, springing
//! back to 0), turned here into the change since the last report.

use std::collections::BTreeMap;

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

const REPORT_LEN: usize = 36;
const READ_BUFFER: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 16;
/// A legacy LED is lit when its channel passes this level.
const LEGACY_THRESHOLD: u8 = 64;

const SET_LED: u8 = 181;
const SET_RGB_BANK_LED: u8 = 165;
const SET_INTENSITY: u8 = 187;

pub struct XKeys {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    keys: Vec<bool>,
    lights: BTreeMap<u8, [u8; 3]>,
    /// The input bytes of the jog wheel and shuttle ring, on models with them.
    jog_shuttle: Option<(usize, usize)>,
    shuttle: i8,
}

impl XKeys {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let jog_shuttle = match model.id {
            "xkeys-xk12-jog" => Some((6, 7)),
            "xkeys-xba-4x3-jog" => Some((12, 13)),
            _ => None,
        };
        Self {
            model,
            transport,
            keys: vec![false; usize::from(model.keys)],
            lights: BTreeMap::new(),
            jog_shuttle,
            shuttle: 0,
        }
    }

    /// A key in reading order -> its column (button byte) and row (bit).
    fn column_row(&self, key: u8) -> (u8, u8) {
        (key % self.model.cols, key / self.model.cols)
    }

    /// The red bank's offset from the blue one on legacy panels.
    fn red_offset(&self) -> u8 {
        self.model.cols.saturating_mul(8)
    }

    fn parse(&mut self, report: &[u8]) -> Vec<Input> {
        if report.get(1).is_none_or(|status| *status > 3) {
            return Vec::new();
        }
        let mut events = Vec::new();
        for key in 0..self.model.keys {
            let (column, row) = self.column_row(key);
            let Some(byte) = report.get(2 + usize::from(column)) else {
                continue;
            };
            let pressed = byte & (1 << row) != 0;
            let held = &mut self.keys[usize::from(key)];
            if pressed != *held {
                *held = pressed;
                events.push(Input::Key {
                    index: key,
                    pressed,
                });
            }
        }
        if let Some((jog, shuttle)) = self.jog_shuttle {
            let signed = |at: usize| report.get(at).map(|byte| i8::from_le_bytes([*byte]));
            if let Some(step) = signed(jog).filter(|step| *step != 0) {
                events.push(Input::DialTurn {
                    index: 0,
                    delta: step,
                });
            }
            if let Some(position) = signed(shuttle) {
                let moved = position.saturating_sub(self.shuttle);
                self.shuttle = position;
                if moved != 0 {
                    events.push(Input::DialTurn {
                        index: 1,
                        delta: moved,
                    });
                }
            }
        }
        events
    }

    fn send(&mut self, message: &[u8]) -> DriverResult<()> {
        let mut report = message.to_vec();
        report.resize(REPORT_LEN, 0);
        self.transport.write(&report)
    }

    fn light(&mut self, key: u8, [r, g, b]: [u8; 3]) -> DriverResult<()> {
        let (column, row) = self.column_row(key);
        let led = column * 8 + row;
        match self.model.protocol {
            Protocol::XKeysRgb24 => self.send(&[0, SET_LED, led, g, r, b, 0]),
            Protocol::XKeysRgbBanks => {
                // The LEDs are numbered like the keys: down each column.
                let index = column * self.model.rows + row;
                self.send(&[0, SET_RGB_BANK_LED, index, 0, r, g, b, 0])?;
                self.send(&[0, SET_RGB_BANK_LED, index, 1, r, g, b, 0])
            }
            _ => {
                let blue = u8::from(b >= LEGACY_THRESHOLD);
                let red = u8::from(r >= LEGACY_THRESHOLD || g >= LEGACY_THRESHOLD);
                self.send(&[0, SET_LED, led, blue, 1])?;
                self.send(&[0, SET_LED, led + self.red_offset(), red, 1])
            }
        }
    }
}

impl Driver for XKeys {
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

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if key < self.model.keys {
            self.lights
                .insert(key, face.map_or([0, 0, 0], |face| face.color));
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        for (key, color) in std::mem::take(&mut self.lights) {
            self.light(key, color)?;
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        let level = u8::try_from(u16::from(percent.min(100)) * 255 / 100).unwrap_or(u8::MAX);
        if self.model.protocol == Protocol::XKeysLegacy {
            self.send(&[0, SET_INTENSITY, level, level])
        } else {
            self.send(&[0, SET_INTENSITY, level])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn driver(model: &str) -> (XKeys, Recorder) {
        let recorder = Recorder::default();
        (
            XKeys::new(by_id(model).unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    fn face(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    #[test]
    fn button_bytes_are_columns_and_bits_rows() {
        let (mut deck, recorder) = driver("xkeys-xk24");
        // Column 2 (byte 4), row 3 (bit 3): key 3 * 4 + 2 = 14.
        recorder.queue_read(vec![0, 0, 0, 0, 0b1000, 0, 0, 0]);
        // A special reply (status 214) is not a key state.
        recorder.queue_read(vec![0, 214, 0xff, 0xff, 0xff, 0xff]);
        recorder.queue_read(vec![0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 14,
                    pressed: true
                },
                Input::Key {
                    index: 14,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn legacy_backlights_split_a_colour_into_blue_and_red() {
        let (mut deck, recorder) = driver("xkeys-xk24");
        // Key 5 is column 1, row 1: LED 9; its red twin is 9 + 32.
        deck.set_face(5, Some(&face([200, 0, 220]))).unwrap();
        deck.set_face(6, None).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert!(writes.iter().all(|report| report.len() == REPORT_LEN));
        assert_eq!(&writes[0][..5], &[0, SET_LED, 9, 1, 1]);
        assert_eq!(&writes[1][..5], &[0, SET_LED, 41, 1, 1]);
        // Unbound: both off.
        // Key 6 is column 2, row 1: LED 17, red twin 49.
        assert_eq!(&writes[2][..5], &[0, SET_LED, 17, 0, 1]);
        assert_eq!(&writes[3][..5], &[0, SET_LED, 49, 0, 1]);
    }

    #[test]
    fn rgb_panels_take_full_colours() {
        let (mut deck, recorder) = driver("xkeys-xk24-rgb");
        deck.set_face(1, Some(&face([10, 20, 30]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(&recorder.writes()[0][..7], &[0, SET_LED, 8, 20, 10, 30, 0]);

        let (mut deck, recorder) = driver("xkeys-xbk-4x6");
        // Key 4 is column 0, row 1: LED 1, both banks.
        deck.set_face(4, Some(&face([1, 2, 3]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(&writes[0][..8], &[0, SET_RGB_BANK_LED, 1, 0, 1, 2, 3, 0]);
        assert_eq!(&writes[1][..8], &[0, SET_RGB_BANK_LED, 1, 1, 1, 2, 3, 0]);
    }

    #[test]
    fn jog_steps_and_shuttle_moves_are_dial_turns() {
        let (mut deck, recorder) = driver("xkeys-xk12-jog");
        let report = |jog: u8, shuttle: u8| vec![0, 0, 0, 0, 0, 0, jog, shuttle];
        recorder.queue_read(report(0x01, 0));
        recorder.queue_read(report(0xff, 3));
        recorder.queue_read(report(0, 3));
        recorder.queue_read(report(0, 0));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialTurn { index: 0, delta: 1 },
                Input::DialTurn {
                    index: 0,
                    delta: -1
                },
                Input::DialTurn { index: 1, delta: 3 },
                Input::DialTurn {
                    index: 1,
                    delta: -3
                },
            ]
        );
    }

    #[test]
    fn brightness_sets_both_legacy_banks() {
        let (mut deck, recorder) = driver("xkeys-xk80");
        deck.set_brightness(100).unwrap();
        assert_eq!(&recorder.writes()[0][..4], &[0, SET_INTENSITY, 255, 255]);
    }
}
