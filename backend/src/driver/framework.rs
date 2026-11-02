//! Framework Laptop 16 RGB Macropad, flashed with the Companion raw-HID firmware.
//!
//! The macropad is a 4 x 6 grid of keys, each with an RGB light and no screen.
//! Stock Framework firmware types keystrokes and speaks VIA; Ryu only drives the
//! custom QMK build made for Bitfocus Companion, which exposes a raw-HID
//! collection on usage page 0xFFDD. Its reports are 32 bytes with no report id,
//! so a write is a 0x00 placeholder followed by the 32-byte payload.
//!
//! Out: `0f c r R G B` lights the key at column `c` (1 to 4) and row `r` (1 to 6),
//! `0b R G B` lights every key, `0c` asks whether the firmware is there. Each
//! write takes effect on its own. In: `50 c r p` is the key at column `c`, row
//! `r` going down (`p` = 1) or up (`p` = 0); `0c 01` answers the probe. Columns
//! and rows count from 1 at the top left; Ryu's key is `(r - 1) * 4 + (c - 1)`.
//! The firmware has no brightness, so Ryu scales the colours itself.
//!
//! Facts per the MIT-licensed Bitfocus companion-surface-framework-macropad
//! module and Julusian's framework_qmk_firmware macropad keymap; no code was
//! copied.

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

/// The raw-HID payload length, both ways (QMK's `RAW_EPSIZE`).
const PAYLOAD_LEN: usize = 32;
/// A write: the report-id placeholder hidapi expects, then the payload.
const WRITE_LEN: usize = PAYLOAD_LEN + 1;
const COLUMNS: u8 = 4;
const ROWS: u8 = 6;
const KEYS: usize = (COLUMNS as usize) * (ROWS as usize);
const MAX_REPORTS_PER_POLL: usize = 32;
/// Heartbeats to wait for the probe's answer before warning about the firmware.
const PROBE_PATIENCE: u8 = 3;

const SET_ALL: u8 = 0x0b;
const PROBE: u8 = 0x0c;
const SET_KEY: u8 = 0x0f;
const KEY_EVENT: u8 = 0x50;

const BLACK: [u8; 3] = [0, 0, 0];

/// One write: the 0x00 placeholder, the command bytes, then zeros.
fn report(command: &[u8]) -> [u8; WRITE_LEN] {
    let mut out = [0u8; WRITE_LEN];
    for (slot, byte) in out[1..].iter_mut().zip(command) {
        *slot = *byte;
    }
    out
}

/// `colour * percent / 100`, per channel.
fn dimmed(color: [u8; 3], percent: u8) -> [u8; 3] {
    color.map(|channel| (u16::from(channel) * u16::from(percent) / 100) as u8)
}

/// A key event payload -> the key it names and whether it went down.
fn key_event(payload: &[u8]) -> Option<Input> {
    if payload.first() != Some(&KEY_EVENT) {
        return None;
    }
    let (&column, &row, &state) = (payload.get(1)?, payload.get(2)?, payload.get(3)?);
    if !(1..=COLUMNS).contains(&column) || !(1..=ROWS).contains(&row) {
        return None;
    }
    Some(Input::Key {
        index: (row - 1) * COLUMNS + (column - 1),
        pressed: state != 0,
    })
}

pub struct Macropad {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    /// The colour each key should show, at full brightness.
    colors: [[u8; 3]; KEYS],
    dirty: [bool; KEYS],
    brightness: u8,
    /// The firmware answered the probe.
    confirmed: bool,
    unanswered_heartbeats: u8,
}

impl Macropad {
    /// Blanks the keys (the firmware boots with them black, but a previous host
    /// may have lit them) and asks whether the Companion firmware is running.
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let mut pad = Self {
            model,
            transport,
            colors: [BLACK; KEYS],
            dirty: [false; KEYS],
            brightness: 100,
            confirmed: false,
            unanswered_heartbeats: 0,
        };
        for command in [report(&[SET_ALL, 0, 0, 0]), report(&[PROBE])] {
            if let Err(error) = pad.transport.write(&command) {
                tracing::debug!(device = model.id, %error, "ryu-decks: macropad setup write failed");
            }
        }
        pad
    }

    fn send_key(&mut self, key: usize) -> DriverResult<()> {
        let [r, g, b] = dimmed(self.colors[key], self.brightness);
        let column = (key % usize::from(COLUMNS)) as u8 + 1;
        let row = (key / usize::from(COLUMNS)) as u8 + 1;
        self.transport
            .write(&report(&[SET_KEY, column, row, r, g, b]))
    }

    fn receive(&mut self, payload: &[u8]) -> Option<Input> {
        if payload.first() == Some(&PROBE) {
            if payload.get(1) == Some(&1) {
                self.confirmed = true;
            }
            return None;
        }
        key_event(payload)
    }
}

impl Driver for Macropad {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; 64];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            events.extend(self.receive(&buf[..read]));
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let key = usize::from(key);
        if key >= KEYS {
            return Ok(());
        }
        let color = face.map_or(BLACK, |face| face.color);
        if self.colors[key] != color {
            self.colors[key] = color;
            self.dirty[key] = true;
        }
        Ok(())
    }

    /// One `0b` when every key changed to the same colour, else one `0f` per
    /// changed key.
    fn flush(&mut self) -> DriverResult<()> {
        let first = self.colors[0];
        if self.dirty.iter().all(|dirty| *dirty) && self.colors.iter().all(|c| *c == first) {
            self.dirty = [false; KEYS];
            let [r, g, b] = dimmed(first, self.brightness);
            return self.transport.write(&report(&[SET_ALL, r, g, b]));
        }
        for key in 0..KEYS {
            if self.dirty[key] {
                self.dirty[key] = false;
                self.send_key(key)?;
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        let percent = percent.min(100);
        if percent == self.brightness {
            return Ok(());
        }
        self.brightness = percent;
        self.dirty = [true; KEYS];
        self.flush()
    }

    /// Warns once if the probe went unanswered: the deck is likely running
    /// firmware other than the Companion build, and its lights will not take.
    fn heartbeat(&mut self) -> DriverResult<()> {
        if !self.confirmed && self.unanswered_heartbeats < PROBE_PATIENCE {
            self.unanswered_heartbeats += 1;
            if self.unanswered_heartbeats == PROBE_PATIENCE {
                tracing::warn!(
                    device = self.model.id,
                    "ryu-decks: the macropad did not answer the firmware probe; it needs the Companion raw-HID firmware"
                );
            }
        }
        Ok(())
    }
}

impl Drop for Macropad {
    /// Leaves the keys dark when Ryu lets go of the deck.
    fn drop(&mut self) {
        let _ = self.transport.write(&report(&[SET_ALL, 0, 0, 0]));
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::by_id;

    fn pad() -> (Macropad, Recorder) {
        let recorder = Recorder::default();
        let pad = Macropad::new(
            by_id("framework-16-macropad").unwrap(),
            Box::new(recorder.clone()),
        );
        recorder.0.lock().unwrap().writes.clear();
        (pad, recorder)
    }

    fn face(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    fn payload(bytes: &[u8]) -> Vec<u8> {
        let mut out = bytes.to_vec();
        out.resize(PAYLOAD_LEN, 0);
        out
    }

    #[test]
    fn opening_blanks_the_keys_and_probes_the_firmware() {
        let recorder = Recorder::default();
        let _pad = Macropad::new(
            by_id("framework-16-macropad").unwrap(),
            Box::new(recorder.clone()),
        );
        let writes = recorder.writes();
        assert_eq!(writes.len(), 2);
        assert!(writes.iter().all(|w| w.len() == WRITE_LEN && w[0] == 0));
        assert_eq!(&writes[0][..5], &[0, 0x0b, 0, 0, 0]);
        assert_eq!(&writes[1][..2], &[0, 0x0c]);
        assert!(writes[1][2..].iter().all(|b| *b == 0));
    }

    #[test]
    fn key_events_map_columns_and_rows_to_row_major_keys() {
        let (mut pad, recorder) = pad();
        recorder.queue_read(payload(&[0x50, 1, 1, 1])); // top left down
        recorder.queue_read(payload(&[0x50, 4, 6, 1])); // bottom right down
        recorder.queue_read(payload(&[0x50, 2, 3, 0])); // column 2, row 3 up
        recorder.queue_read(payload(&[0x0c, 1])); // probe answer
        recorder.queue_read(payload(&[0x50, 5, 1, 1])); // no column 5
        recorder.queue_read(payload(&[0x50, 1, 0, 1])); // no row 0
        recorder.queue_read(payload(&[0x50, 1, 7, 1])); // no row 7
        recorder.queue_read(vec![0x50, 1]); // short
        recorder.queue_read(payload(&[0x51, 1, 1, 1])); // unknown
        assert_eq!(
            pad.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 0,
                    pressed: true
                },
                Input::Key {
                    index: 23,
                    pressed: true
                },
                Input::Key {
                    index: 9,
                    pressed: false
                },
            ]
        );
        assert!(pad.confirmed);
    }

    #[test]
    fn faces_light_one_key_at_a_time_with_a_leading_zero() {
        let (mut pad, recorder) = pad();
        pad.set_face(5, Some(&face([0x2d, 0x7f, 0xf9]))).unwrap();
        pad.set_face(23, Some(&face([1, 2, 3]))).unwrap();
        pad.set_face(24, Some(&face([9, 9, 9]))).unwrap(); // no such key
        pad.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 2);
        // Key 5 is column 2, row 2; key 23 is column 4, row 6.
        assert_eq!(writes[0], report(&[0x0f, 2, 2, 0x2d, 0x7f, 0xf9]).to_vec());
        assert_eq!(writes[0].len(), 33);
        assert_eq!(&writes[1][..7], &[0, 0x0f, 4, 6, 1, 2, 3]);
        // An unbound key goes dark; an unchanged one is not resent.
        pad.set_face(5, None).unwrap();
        pad.set_face(23, Some(&face([1, 2, 3]))).unwrap();
        pad.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 3);
        assert_eq!(&writes[2][..7], &[0, 0x0f, 2, 2, 0, 0, 0]);
    }

    #[test]
    fn one_colour_on_every_key_goes_out_as_one_report() {
        let (mut pad, recorder) = pad();
        for key in 0..24 {
            pad.set_face(key, Some(&face([10, 20, 30]))).unwrap();
        }
        pad.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes, vec![report(&[0x0b, 10, 20, 30]).to_vec()]);
    }

    #[test]
    fn brightness_scales_and_resends_every_key() {
        let (mut pad, recorder) = pad();
        pad.set_face(0, Some(&face([200, 100, 255]))).unwrap();
        pad.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        pad.set_brightness(50).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 24);
        assert_eq!(&writes[0][..7], &[0, 0x0f, 1, 1, 100, 50, 127]);
        assert_eq!(&writes[1][..7], &[0, 0x0f, 2, 1, 0, 0, 0]);
        // Zero is off, not a fallback level.
        pad.set_brightness(0).unwrap();
        assert_eq!(&recorder.writes()[24][..7], &[0, 0x0f, 1, 1, 0, 0, 0]);
        // The same level again sends nothing.
        pad.set_brightness(0).unwrap();
        assert_eq!(recorder.writes().len(), 48);
    }

    #[test]
    fn letting_go_blanks_the_keys_and_never_enters_the_bootloader() {
        let (pad, recorder) = pad();
        drop(pad);
        assert_eq!(recorder.writes(), vec![report(&[0x0b, 0, 0, 0]).to_vec()]);
        assert!(recorder.writes().iter().all(|w| w.get(1) != Some(&0xff)));
    }
}
