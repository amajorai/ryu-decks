//! Infinitton iDisplay: an early 15-key Stream Deck look-alike.
//!
//! A key face is 72 x 72 raw BGR pixels, each row mirrored, sent as two 8017-byte
//! output reports with fixed headers (the first carries a BMP header), then
//! committed with a feature report naming the key. Input reports carry one bit
//! per key in bytes 1 and 2.

use std::collections::BTreeMap;

use super::{face, Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const PAGE_LEN: usize = 8017;
const READ_BUFFER: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 16;

/// The first page's header: report id 2, then the page's offset and length and a
/// BMP header for a 72 x 72, 24-bit picture.
const PAGE_ONE_HEADER: [u8; 71] = [
    0x02, 0x00, 0x00, 0x00, 0x00, 0x40, 0x1f, 0x00, 0x00, 0x55, 0xaa, 0xaa, 0x55, 0x11, 0x22, 0x33,
    0x44, 0x42, 0x4d, 0xf6, 0x3c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x36, 0x00, 0x00, 0x00, 0x28,
    0x00, 0x00, 0x00, 0x48, 0x00, 0x00, 0x00, 0x48, 0x00, 0x00, 0x00, 0x01, 0x00, 0x18, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xc0, 0x3c, 0x00, 0x00, 0x13, 0x0b, 0x00, 0x00, 0x13, 0x0b, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
const PAGE_TWO_HEADER: [u8; 17] = [
    0x02, 0x40, 0x1f, 0x00, 0x00, 0xb6, 0x1d, 0x00, 0x00, 0x55, 0xaa, 0xaa, 0x55, 0x11, 0x22, 0x33,
    0x44,
];

pub struct Infinitton {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    keys: Vec<bool>,
    staged: BTreeMap<u8, Vec<u8>>,
}

impl Infinitton {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            model,
            transport,
            keys: vec![false; usize::from(model.keys)],
            staged: BTreeMap::new(),
        }
    }

    fn parse(&mut self, report: &[u8]) -> Vec<Input> {
        let (Some(&low), Some(&high)) = (report.get(1), report.get(2)) else {
            return Vec::new();
        };
        let bits = u16::from_le_bytes([low, high]);
        let mut events = Vec::new();
        for (index, held) in self.keys.iter_mut().enumerate() {
            let pressed = bits & (1 << index) != 0;
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

    /// The two pages and the commit for one face.
    fn reports(key: u8, pixels: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let split = (PAGE_LEN - PAGE_ONE_HEADER.len()).min(pixels.len());
        let mut first = PAGE_ONE_HEADER.to_vec();
        first.extend_from_slice(&pixels[..split]);
        first.resize(PAGE_LEN, 0);
        let mut second = PAGE_TWO_HEADER.to_vec();
        second.extend_from_slice(&pixels[split..]);
        second.resize(PAGE_LEN, 0);
        let mut commit = vec![0u8; 34];
        commit[1] = 0x12;
        commit[2] = 0x01;
        commit[5] = key + 1;
        commit[17] = 0xf6;
        commit[18] = 0x3c;
        (first, second, commit)
    }
}

impl Driver for Infinitton {
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
        let Some(format) = self.model.image else {
            return Ok(());
        };
        if key >= self.model.keys {
            return Ok(());
        }
        self.staged.insert(key, face::encode(face, &format)?);
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        for (key, pixels) in std::mem::take(&mut self.staged) {
            let (first, second, commit) = Self::reports(key, &pixels);
            self.transport.write(&first)?;
            self.transport.write(&second)?;
            self.transport.send_feature_report(&commit)?;
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.transport
            .send_feature_report(&[0x00, 0x11, percent.min(100)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn driver() -> (Infinitton, Recorder) {
        let recorder = Recorder::default();
        let model = by_id("infinitton-idisplay").unwrap();
        (Infinitton::new(model, Box::new(recorder.clone())), recorder)
    }

    #[test]
    fn one_bit_per_key_across_two_bytes() {
        let (mut deck, recorder) = driver();
        // Keys 4, 8 and 14 down, then all up.
        recorder.queue_read(vec![0x00, 0x10, 0x41]);
        recorder.queue_read(vec![0x00, 0x00, 0x00]);
        let pressed: Vec<(u8, bool)> = deck
            .poll(0)
            .unwrap()
            .into_iter()
            .map(|input| match input {
                Input::Key { index, pressed } => (index, pressed),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            pressed,
            vec![
                (4, true),
                (8, true),
                (14, true),
                (4, false),
                (8, false),
                (14, false)
            ]
        );
    }

    #[test]
    fn a_face_is_two_pages_of_mirrored_bgr_then_a_commit() {
        let (mut deck, recorder) = driver();
        // A face whose left half is red: mirrored, the first pixel of a row is
        // from the right half (black background with no image would be uniform).
        let mut picture = image::RgbaImage::from_pixel(72, 72, image::Rgba([0, 0, 0, 255]));
        for y in 0..72 {
            for x in 0..36 {
                picture.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        let face = Face {
            label: String::new(),
            color: [0, 0, 0],
            image: Some(picture),
        };
        deck.set_face(6, Some(&face)).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 2);
        assert!(writes.iter().all(|page| page.len() == PAGE_LEN));
        assert_eq!(&writes[0][..PAGE_ONE_HEADER.len()], &PAGE_ONE_HEADER);
        assert_eq!(&writes[1][..PAGE_TWO_HEADER.len()], &PAGE_TWO_HEADER);
        let first_pixel = &writes[0][PAGE_ONE_HEADER.len()..PAGE_ONE_HEADER.len() + 3];
        let last_of_row =
            &writes[0][PAGE_ONE_HEADER.len() + 71 * 3..PAGE_ONE_HEADER.len() + 72 * 3];
        assert_eq!(first_pixel, &[0, 0, 0], "row mirrored: right half first");
        assert_eq!(last_of_row, &[0, 0, 255], "BGR: red is the third byte");
        let commit = &recorder.features()[0];
        assert_eq!((commit[1], commit[5]), (0x12, 7));
        deck.set_brightness(130).unwrap();
        assert_eq!(recorder.features()[1], vec![0x00, 0x11, 100]);
    }
}
