//! SteelSeries Apex keyboards' 128 x 40 monochrome OLED, the only part of the
//! keyboard Ryu drives. It is the deck's info screen; the keys stay an
//! ordinary keyboard and are never read.
//!
//! Every frame is 640 bytes, one bit per pixel, a set bit a lit pixel, sent
//! as HID feature reports on the OLED's own interface:
//!
//! - The earlier boards (Apex Pro, Apex 7, Apex Pro TKL, Apex 7 TKL, Apex 5):
//!   one 642-byte report, `61`, the frame row by row (16 bytes a row, the
//!   leftmost pixel in the top bit), then `00`.
//! - The Apex Pro Gen 3: the same report, but the frame in the panel's own
//!   order, five 8-row pages of one byte per column with the top row in bit 0.
//! - The Apex Pro TKL Wireless Gen 3: that paged frame in eight 641-byte
//!   reports of 80 bytes each: the frame command (`0C` by cable, `4C`
//!   through the 2.4 GHz dongle), `01`, the chunk's offset in the frame as a
//!   little-endian 16-bit number, `50` (its length), `00`, the 80 bytes, then
//!   zeros. Nothing follows the eighth.
//!
//! The model's info image says which bit order the frame takes, and its
//! protocol whether it goes whole or in chunks. There is no handshake,
//! keep-alive, brightness or shutdown command: a frame stays until the next
//! one, so the frame is sent only when it changes, and blanked when Ryu lets
//! go of the keyboard. SteelSeries GG may draw over it.
//!
//! Device facts come from not-jan's apex-tux (Unlicense); no code was copied.

use std::time::Duration;

use super::{face, Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, KeyImage, Protocol};

/// The OLED's size in pixels.
pub const WIDTH: u32 = 128;
pub const HEIGHT: u32 = 40;
/// One frame at one bit per pixel.
pub const FRAME_BYTES: usize = (WIDTH * HEIGHT / 8) as usize;
/// The model id of the Apex Pro TKL Wireless Gen 3 through its dongle, whose
/// frame command differs from the cabled board's.
pub const DONGLE_ID: &str = "steelseries-apex-pro-tkl-wireless-gen3-dongle";

/// The report id of a whole frame.
const FRAME_REPORT_ID: u8 = 0x61;
/// `61`, the frame and a trailing `00`.
const FRAME_REPORT_LEN: usize = FRAME_BYTES + 2;
/// The chunked frame command by cable and through the dongle.
const CABLE_CHUNK_COMMAND: u8 = 0x0c;
const DONGLE_CHUNK_COMMAND: u8 = 0x4c;
/// The OLED sub-command after the chunk command.
const OLED_SUBCOMMAND: u8 = 0x01;
/// Frame bytes in one chunk.
const CHUNK_BYTES: usize = 80;
/// The header before a chunk's bytes: command, sub-command, offset (2),
/// length, and a zero.
const CHUNK_HEADER: usize = 6;
/// Every chunk report, header and padding included.
const CHUNK_REPORT_LEN: usize = 641;

/// How a frame reaches the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wire {
    /// One report headed `61`.
    Whole,
    /// Eight 80-byte chunks headed by this command.
    Chunked(u8),
}

pub struct ApexOled {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    wire: Wire,
    /// The frame staged for the next flush.
    frame: Vec<u8>,
    /// The frame the panel shows, once one has been sent.
    sent: Option<Vec<u8>>,
}

impl ApexOled {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let wire = match model.protocol {
            Protocol::SteelSeriesOledChunked if model.id == DONGLE_ID => {
                Wire::Chunked(DONGLE_CHUNK_COMMAND)
            }
            Protocol::SteelSeriesOledChunked => Wire::Chunked(CABLE_CHUNK_COMMAND),
            _ => Wire::Whole,
        };
        Self {
            model,
            transport,
            wire,
            frame: vec![0; FRAME_BYTES],
            sent: None,
        }
    }

    fn info_image(&self) -> Option<(u8, KeyImage)> {
        self.model.info_key().zip(self.model.info_image)
    }

    /// The reports that put `frame` on the panel.
    fn reports(&self, frame: &[u8]) -> Vec<Vec<u8>> {
        match self.wire {
            Wire::Whole => vec![frame_report(frame)],
            Wire::Chunked(command) => chunk_reports(command, frame),
        }
    }

    fn send(&mut self, frame: &[u8]) -> DriverResult<()> {
        for report in self.reports(frame) {
            self.transport.send_feature_report(&report)?;
        }
        Ok(())
    }
}

/// A whole frame's report: `61`, the frame (cut or padded to 640 bytes), `00`.
fn frame_report(frame: &[u8]) -> Vec<u8> {
    let mut report = vec![0u8; FRAME_REPORT_LEN];
    report[0] = FRAME_REPORT_ID;
    for (slot, byte) in report[1..=FRAME_BYTES].iter_mut().zip(frame) {
        *slot = *byte;
    }
    report
}

/// A paged frame as eight chunk reports headed by `command`.
fn chunk_reports(command: u8, frame: &[u8]) -> Vec<Vec<u8>> {
    (0..FRAME_BYTES / CHUNK_BYTES)
        .map(|chunk| {
            let offset = chunk * CHUNK_BYTES;
            let [low, high] = u16::try_from(offset).unwrap_or(0).to_le_bytes();
            let mut report = vec![0u8; CHUNK_REPORT_LEN];
            report[..CHUNK_HEADER].copy_from_slice(&[
                command,
                OLED_SUBCOMMAND,
                low,
                high,
                CHUNK_BYTES as u8,
                0x00,
            ]);
            let bytes = frame.get(offset..).unwrap_or_default();
            for (slot, byte) in report[CHUNK_HEADER..CHUNK_HEADER + CHUNK_BYTES]
                .iter_mut()
                .zip(bytes)
            {
                *slot = *byte;
            }
            report
        })
        .collect()
}

impl Driver for ApexOled {
    /// Nothing is ever read: the OLED interface is write-only, and the keys
    /// belong to the keyboard. Waits out the timeout so the device loop paces.
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let wait = u64::try_from(timeout_ms).unwrap_or(0);
        if wait > 0 {
            std::thread::sleep(Duration::from_millis(wait));
        }
        Ok(Vec::new())
    }

    /// The info key draws the OLED; an unbound one leaves it dark.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let Some((info, format)) = self.info_image() else {
            return Ok(());
        };
        if key != info {
            return Ok(());
        }
        let mut frame = face::encode(face, &format)?;
        frame.resize(FRAME_BYTES, 0);
        self.frame = frame;
        Ok(())
    }

    /// Sends the frame when it differs from the one the panel shows.
    fn flush(&mut self) -> DriverResult<()> {
        if self.sent.as_ref() == Some(&self.frame) {
            return Ok(());
        }
        let frame = self.frame.clone();
        self.send(&frame)?;
        self.sent = Some(frame);
        Ok(())
    }

    /// The OLED has no known brightness command.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for ApexOled {
    /// Blanks the OLED if Ryu drew anything on it.
    fn drop(&mut self) {
        let lit = self
            .sent
            .as_ref()
            .is_some_and(|frame| frame.iter().any(|byte| *byte != 0));
        if lit {
            let _ = self.send(&[0; FRAME_BYTES]);
        }
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn open(id: &str) -> (ApexOled, Recorder) {
        let recorder = Recorder::default();
        let model = by_id(id).unwrap();
        (ApexOled::new(model, Box::new(recorder.clone())), recorder)
    }

    /// A black face with white pixels at the given places.
    fn face_with(pixels: &[(u32, u32)]) -> Face {
        let mut picture = RgbaImage::from_pixel(WIDTH, HEIGHT, Rgba([0, 0, 0, 255]));
        for (x, y) in pixels {
            picture.put_pixel(*x, *y, Rgba([255, 255, 255, 255]));
        }
        Face {
            label: "OLED".to_owned(),
            color: [0, 0, 0],
            image: Some(picture),
        }
    }

    const MARKS: [(u32, u32); 4] = [(0, 0), (9, 1), (127, 8), (3, 39)];

    #[test]
    fn the_earlier_boards_take_one_report_row_by_row() {
        for id in [
            "steelseries-apex-pro",
            "steelseries-apex-7",
            "steelseries-apex-pro-tkl",
            "steelseries-apex-7-tkl",
            "steelseries-apex-5",
        ] {
            let (mut oled, recorder) = open(id);
            oled.set_face(0, Some(&face_with(&MARKS))).unwrap();
            oled.flush().unwrap();
            let features = recorder.features();
            assert_eq!(features.len(), 1, "{id}");
            let report = &features[0];
            assert_eq!(report.len(), 642);
            assert_eq!((report[0], report[641]), (0x61, 0x00));
            // Byte 1 + 16 y + x / 8, the leftmost pixel in the top bit.
            assert_eq!(report[1], 0x80);
            assert_eq!(report[1 + 16 + 1], 0x40);
            assert_eq!(report[1 + 16 * 8 + 15], 0x01);
            assert_eq!(report[1 + 16 * 39], 0x10);
            assert_eq!(report[1..641].iter().filter(|byte| **byte != 0).count(), 4);
        }
    }

    #[test]
    fn the_apex_pro_gen3_takes_one_report_in_pages() {
        let (mut oled, recorder) = open("steelseries-apex-pro-gen3");
        oled.set_face(0, Some(&face_with(&MARKS))).unwrap();
        oled.flush().unwrap();
        let features = recorder.features();
        assert_eq!(features.len(), 1);
        let report = &features[0];
        assert_eq!(report.len(), 642);
        assert_eq!((report[0], report[641]), (0x61, 0x00));
        // Byte 1 + 128 (y / 8) + x, the top row of each page in bit 0.
        assert_eq!(report[1], 0x01);
        assert_eq!(report[1 + 9], 0x02);
        assert_eq!(report[1 + 128 + 127], 0x01);
        assert_eq!(report[1 + 128 * 4 + 3], 0x80);
        assert_eq!(report[1..641].iter().filter(|byte| **byte != 0).count(), 4);
    }

    #[test]
    fn the_wireless_gen3_takes_eight_chunks_headed_by_its_command() {
        for (id, command) in [
            ("steelseries-apex-pro-tkl-wireless-gen3", 0x0c),
            (DONGLE_ID, 0x4c),
        ] {
            let (mut oled, recorder) = open(id);
            oled.set_face(0, Some(&face_with(&MARKS))).unwrap();
            oled.flush().unwrap();
            let features = recorder.features();
            assert_eq!(features.len(), 8, "{id}");
            let offsets: Vec<u16> = features
                .iter()
                .map(|report| u16::from_le_bytes([report[2], report[3]]))
                .collect();
            assert_eq!(offsets, [0, 80, 160, 240, 320, 400, 480, 560]);
            for report in &features {
                assert_eq!(report.len(), 641);
                assert_eq!(
                    (report[0], report[1], report[4], report[5]),
                    (command, 0x01, 0x50, 0x00)
                );
                assert!(report[86..].iter().all(|byte| *byte == 0));
            }
            // The paged frame, 80 bytes a chunk from byte 6.
            let frame: Vec<u8> = features
                .iter()
                .flat_map(|report| report[6..86].to_vec())
                .collect();
            assert_eq!(frame[0], 0x01);
            assert_eq!(frame[9], 0x02);
            assert_eq!(frame[128 + 127], 0x01);
            assert_eq!(frame[128 * 4 + 3], 0x80);
            assert_eq!(features[3][2..4], [0xf0, 0x00]);
            assert_eq!(features[7][2..4], [0x30, 0x02]);
        }
    }

    #[test]
    fn frames_go_only_when_they_change_and_unbound_is_dark() {
        let (mut oled, recorder) = open("steelseries-apex-pro-tkl");
        // The first flush always sends, so the panel matches Ryu on connect.
        oled.flush().unwrap();
        assert_eq!(recorder.features().len(), 1);
        assert!(recorder.features()[0][1..641].iter().all(|byte| *byte == 0));
        oled.set_face(0, Some(&face_with(&[(1, 1)]))).unwrap();
        oled.flush().unwrap();
        oled.flush().unwrap();
        assert_eq!(recorder.features().len(), 2);
        // Keys that are not the OLED change nothing.
        oled.set_face(1, Some(&face_with(&[(5, 5)]))).unwrap();
        oled.flush().unwrap();
        assert_eq!(recorder.features().len(), 2);
        oled.set_face(0, None).unwrap();
        oled.flush().unwrap();
        let features = recorder.features();
        assert_eq!(features.len(), 3);
        assert!(features[2][1..].iter().all(|byte| *byte == 0));
        assert_eq!(features[2][0], 0x61);
    }

    #[test]
    fn a_coloured_background_lights_the_panel_by_brightness() {
        let (mut oled, recorder) = open("steelseries-apex-5");
        let bright = Face {
            label: String::new(),
            color: [255, 255, 255],
            image: None,
        };
        oled.set_face(0, Some(&bright)).unwrap();
        oled.flush().unwrap();
        assert!(recorder.features()[0][1..641]
            .iter()
            .all(|byte| *byte == 0xff));
    }

    #[test]
    fn nothing_is_read_and_brightness_is_ignored() {
        let (mut oled, recorder) = open("steelseries-apex-pro");
        recorder.queue_read(vec![0x01, 0x02, 0x03]);
        assert!(oled.poll(0).unwrap().is_empty());
        oled.set_brightness(0).unwrap();
        assert!(recorder.features().is_empty() && recorder.writes().is_empty());
    }

    #[test]
    fn dropping_blanks_a_lit_panel_only() {
        let (mut oled, recorder) = open("steelseries-apex-pro-gen3");
        oled.set_face(0, Some(&face_with(&[(2, 2)]))).unwrap();
        oled.flush().unwrap();
        drop(oled);
        let features = recorder.features();
        assert_eq!(features.len(), 2);
        assert!(features[1][1..].iter().all(|byte| *byte == 0));
        let (oled, recorder) = open(DONGLE_ID);
        drop(oled);
        assert!(recorder.features().is_empty());
    }

    #[test]
    fn short_frames_are_padded_not_panicked_on() {
        assert_eq!(frame_report(&[0xaa]).len(), 642);
        assert_eq!(frame_report(&[0xaa])[1..3], [0xaa, 0x00]);
        let chunks = chunk_reports(0x0c, &[0x11; 90]);
        assert_eq!(chunks.len(), 8);
        assert_eq!(chunks[1][6..16], [0x11; 10][..]);
        assert_eq!(chunks[1][16], 0);
        assert!(chunks[2][6..].iter().all(|byte| *byte == 0));
    }
}
