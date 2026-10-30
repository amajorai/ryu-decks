//! 203 Systems Mystrix: an 8 x 8 grid of RGB-lit pads running Matrix OS.
//!
//! Ryu talks to the Matrix OS "Companion" app over the vendor HID collection
//! (usage page 0xFF00). Every message is a numbered report with id 0xFF and 63
//! data bytes, so a write is exactly 64 bytes with the id first, and a read
//! starts with the id too. The same interface also carries the keyboard, mouse,
//! consumer and gamepad reports (ids 1 to 4) and Matrix OS system replies (id
//! 0xCB), which Ryu ignores.
//!
//! Out: `01` asks whether the Companion app is running, `21` clears every pad,
//! `20 x y r g b` lights one pad at once (no commit), `30 p` is a brightness
//! percent the current firmware ignores. In: `01 s` says the app is running
//! with its menu closed (`s` = 1) or not (anything else); `10 x y v` is a pad
//! going down (`v` non-zero) or up. `x` is the column and `y` the row, both from
//! the top left of the grid as Matrix OS has rotated it; the FN key and the
//! touch bars are never reported.
//!
//! Lights only take while the app is active, so Ryu keeps every colour and sends
//! the whole grid again whenever the app becomes active. The firmware does not
//! dim on `30`, so Ryu also scales the colours itself. Facts per the
//! MIT-licensed Bitfocus companion-surface-203-systems-mystrix plugin and the
//! 203-Systems/MatrixOS firmware; no code was copied.

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const REPORT_ID: u8 = 0xff;
/// The report id plus 63 data bytes, both ways.
pub const REPORT_LEN: usize = 64;
const SIDE: u8 = 8;
const PADS: usize = (SIDE as usize) * (SIDE as usize);
const MAX_REPORTS_PER_POLL: usize = 32;

const STATUS: u8 = 0x01;
const KEY: u8 = 0x10;
const SET_COLOR: u8 = 0x20;
const CLEAR: u8 = 0x21;
const BRIGHTNESS: u8 = 0x30;
const ACTIVE: u8 = 0x01;

const BLACK: [u8; 3] = [0, 0, 0];

/// One output report: the id, the command bytes, then zeros.
fn report(command: &[u8]) -> [u8; REPORT_LEN] {
    let mut out = [0u8; REPORT_LEN];
    out[0] = REPORT_ID;
    for (slot, byte) in out[1..].iter_mut().zip(command) {
        *slot = *byte;
    }
    out
}

const fn inquiry() -> [u8; REPORT_LEN] {
    let mut out = [0u8; REPORT_LEN];
    out[0] = REPORT_ID;
    out[1] = STATUS;
    out
}

/// The "is the Companion app running?" report. Only the collection that owns
/// report 0xFF takes it, which is how the manager picks between the two vendor
/// collections Windows lists for one Mystrix.
pub const INQUIRY: [u8; REPORT_LEN] = inquiry();

/// `colour * percent / 100`, per channel.
fn dimmed(color: [u8; 3], percent: u8) -> [u8; 3] {
    color.map(|channel| (u16::from(channel) * u16::from(percent) / 100) as u8)
}

pub struct Mystrix {
    transport: Box<dyn Transport>,
    inquired: bool,
    /// The Companion app is running with its menu closed, so lights take.
    active: bool,
    /// The colour each pad should show, at full brightness; pad = row * 8 + column.
    colors: [[u8; 3]; PADS],
    dirty: [bool; PADS],
    held: [bool; PADS],
    brightness: u8,
}

impl Mystrix {
    #[must_use]
    pub fn new(_model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            inquired: false,
            active: false,
            colors: [BLACK; PADS],
            dirty: [false; PADS],
            held: [false; PADS],
            brightness: 100,
        }
    }

    fn send_color(&mut self, pad: usize) -> DriverResult<()> {
        let [r, g, b] = dimmed(self.colors[pad], self.brightness);
        let (x, y) = (pad % usize::from(SIDE), pad / usize::from(SIDE));
        self.transport
            .write(&report(&[SET_COLOR, x as u8, y as u8, r, g, b]))
    }

    /// Clear the grid, then light every pad that is not black.
    fn refresh(&mut self) -> DriverResult<()> {
        self.dirty = [false; PADS];
        self.transport.write(&report(&[CLEAR]))?;
        for pad in 0..PADS {
            if dimmed(self.colors[pad], self.brightness) != BLACK {
                self.send_color(pad)?;
            }
        }
        Ok(())
    }

    fn receive(&mut self, report: &[u8]) -> DriverResult<Option<Input>> {
        if report.first() != Some(&REPORT_ID) {
            return Ok(None);
        }
        match report.get(1) {
            Some(&STATUS) => {
                let active = report.get(2) == Some(&ACTIVE);
                if active && !self.active {
                    self.active = true;
                    self.refresh()?;
                } else if !active {
                    self.active = false;
                }
                Ok(None)
            }
            Some(&KEY) => {
                let (Some(&x), Some(&y), Some(&state)) =
                    (report.get(2), report.get(3), report.get(4))
                else {
                    return Ok(None);
                };
                if x >= SIDE || y >= SIDE {
                    return Ok(None);
                }
                let index = y * SIDE + x;
                let pressed = state != 0;
                let held = &mut self.held[usize::from(index)];
                if *held == pressed {
                    return Ok(None);
                }
                *held = pressed;
                Ok(Some(Input::Key { index, pressed }))
            }
            _ => Ok(None),
        }
    }
}

impl Driver for Mystrix {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.inquired {
            self.transport.write(&INQUIRY)?;
            self.inquired = true;
        }
        let mut events = Vec::new();
        let mut buf = [0u8; REPORT_LEN];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            events.extend(self.receive(&buf[..read])?);
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let pad = usize::from(key);
        if pad >= PADS {
            return Ok(());
        }
        let color = face.map_or(BLACK, |face| face.color);
        if self.colors[pad] != color {
            self.colors[pad] = color;
            self.dirty[pad] = true;
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        if !self.active {
            return Ok(());
        }
        for pad in 0..PADS {
            if self.dirty[pad] {
                self.dirty[pad] = false;
                self.send_color(pad)?;
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        let percent = percent.min(100);
        self.transport.write(&report(&[BRIGHTNESS, percent]))?;
        if percent == self.brightness {
            return Ok(());
        }
        self.brightness = percent;
        for pad in 0..PADS {
            if self.colors[pad] != BLACK {
                self.dirty[pad] = true;
            }
        }
        self.flush()
    }

    /// Asks again whether the app is running, in case a notice was missed.
    fn heartbeat(&mut self) -> DriverResult<()> {
        self.transport.write(&INQUIRY)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::by_id;

    fn pad() -> (Mystrix, Recorder) {
        let recorder = Recorder::default();
        let deck = Mystrix::new(
            by_id("203systems-mystrix").unwrap(),
            Box::new(recorder.clone()),
        );
        (deck, recorder)
    }

    fn input(bytes: &[u8]) -> Vec<u8> {
        let mut report = vec![0u8; REPORT_LEN];
        report[..bytes.len()].copy_from_slice(bytes);
        report
    }

    fn lit(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    #[test]
    fn every_command_is_a_64_byte_report_0xff() {
        assert_eq!(INQUIRY.len(), 64);
        assert_eq!(&INQUIRY[..3], &[0xff, 0x01, 0x00]);
        let set = report(&[0x20, 3, 5, 1, 2, 3]);
        assert_eq!(&set[..8], &[0xff, 0x20, 3, 5, 1, 2, 3, 0]);
        assert!(set[8..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn pads_map_row_major_and_only_edges_count() {
        let (mut deck, recorder) = pad();
        recorder.queue_read(input(&[0xff, 0x10, 2, 3, 0xff]));
        recorder.queue_read(input(&[0xff, 0x10, 2, 3, 0x7f]));
        recorder.queue_read(input(&[0xff, 0x10, 2, 3, 0x00]));
        recorder.queue_read(input(&[0xff, 0x10, 7, 7, 0xff]));
        // Off the grid, short, another report id, an unknown command.
        recorder.queue_read(input(&[0xff, 0x10, 8, 0, 0xff]));
        recorder.queue_read(vec![0xff, 0x10, 1]);
        recorder.queue_read(input(&[0x01, 0x10, 1, 1, 0xff]));
        recorder.queue_read(input(&[0xcb, 0x10, 1, 1, 0xff]));
        recorder.queue_read(input(&[0xff, 0x42, 1, 1, 0xff]));
        recorder.queue_read(Vec::new());
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 26,
                    pressed: true
                },
                Input::Key {
                    index: 26,
                    pressed: false
                },
                Input::Key {
                    index: 63,
                    pressed: true
                },
            ]
        );
        // The first poll asks whether the app is running.
        assert_eq!(recorder.writes(), vec![INQUIRY.to_vec()]);
    }

    #[test]
    fn colours_wait_for_the_app_then_go_out_after_a_clear() {
        let (mut deck, recorder) = pad();
        deck.set_face(0, Some(&lit([1, 2, 3]))).unwrap();
        deck.set_face(9, Some(&lit([0x2d, 0x7f, 0xf9]))).unwrap();
        deck.set_face(10, None).unwrap();
        deck.set_face(64, Some(&lit([9, 9, 9]))).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty(), "inactive: nothing is sent");

        recorder.queue_read(input(&[0xff, 0x01, 0x01]));
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 4);
        assert_eq!(writes[0], INQUIRY.to_vec());
        assert_eq!(&writes[1][..2], &[0xff, 0x21]);
        assert_eq!(&writes[2][..7], &[0xff, 0x20, 0, 0, 1, 2, 3]);
        // Pad 9 is column 1 of row 1.
        assert_eq!(&writes[3][..7], &[0xff, 0x20, 1, 1, 0x2d, 0x7f, 0xf9]);
        assert!(writes.iter().all(|write| write.len() == 64));

        // Active: a change goes out on flush, unchanged faces do not.
        deck.set_face(9, Some(&lit([0x2d, 0x7f, 0xf9]))).unwrap();
        deck.set_face(63, Some(&lit([4, 5, 6]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 5);
        assert_eq!(&writes[4][..7], &[0xff, 0x20, 7, 7, 4, 5, 6]);

        // The action menu opens: lights wait again, and a reopened app gets them all.
        recorder.queue_read(input(&[0xff, 0x01, 0x00]));
        deck.poll(0).unwrap();
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 5);
        recorder.queue_read(input(&[0xff, 0x01, 0x01]));
        recorder.queue_read(input(&[0xff, 0x01, 0x01]));
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 8, "one refresh, not two");
        assert_eq!(&writes[5][..2], &[0xff, 0x21]);
        assert_eq!(&writes[6][..7], &[0xff, 0x20, 1, 1, 0x2d, 0x7f, 0xf9]);
        assert_eq!(&writes[7][..7], &[0xff, 0x20, 7, 7, 4, 5, 6]);
    }

    #[test]
    fn brightness_is_sent_as_a_percent_and_dims_the_colours() {
        let (mut deck, recorder) = pad();
        recorder.queue_read(input(&[0xff, 0x01, 0x01]));
        deck.poll(0).unwrap();
        deck.set_face(1, Some(&lit([200, 100, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_brightness(150).unwrap();
        deck.set_brightness(50).unwrap();
        let writes = recorder.writes();
        assert_eq!(&writes[before][..3], &[0xff, 0x30, 100]);
        assert_eq!(&writes[before + 1][..3], &[0xff, 0x30, 50]);
        assert_eq!(&writes[before + 2][..7], &[0xff, 0x20, 1, 0, 100, 50, 0]);
        assert_eq!(writes.len(), before + 3);
    }

    #[test]
    fn the_heartbeat_asks_again() {
        let (mut deck, recorder) = pad();
        deck.heartbeat().unwrap();
        assert_eq!(recorder.writes(), vec![INQUIRY.to_vec()]);
    }
}
