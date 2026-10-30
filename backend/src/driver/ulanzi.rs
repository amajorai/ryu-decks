//! Ulanzi D200 family (D200, D200H, D200X).
//!
//! Every packet is a fixed 1024-byte frame: `7C 7C`, a big-endian command, a
//! little-endian 32-bit length, then up to 1016 bytes of data. The button grid is
//! not drawn key by key: the host sends one zip archive holding a `manifest.json`
//! and one PNG per key. Behavior follows the MIT-licensed reverse engineering in
//! redphx/strmdck and marcelobrake/ulanzi-linux.
//!
//! Two firmware quirks shape this file:
//! - The deck falls back to its own screensaver when the host is silent for about
//!   five seconds, so [`Driver::heartbeat`] refreshes the status window (a clock).
//! - The firmware parses the archive as it streams in and mis-reads it when a
//!   byte at a frame boundary is `00` or `7C`; the archive is re-padded until no
//!   boundary lands on either.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

use super::{face, Driver, DriverError, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const FRAME: usize = 1024;
const HEADER: usize = 8;
const PAYLOAD: usize = FRAME - HEADER;
const MAGIC: [u8; 2] = [0x7c, 0x7c];

const CMD_SET_BUTTONS: u16 = 0x0001;
const CMD_SMALL_WINDOW: u16 = 0x0006;
const CMD_BRIGHTNESS: u16 = 0x000a;
const IN_BUTTON: u16 = 0x0101;

/// The status window to the right of the last three keys; it takes a 2:1 image.
const STATUS_WINDOW_INDEX: u8 = 13;
const MAX_PADDING_ATTEMPTS: usize = 1024;

pub struct UlanziD200 {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    faces: BTreeMap<u8, Vec<u8>>,
    dirty: bool,
}

impl UlanziD200 {
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            model,
            transport,
            faces: BTreeMap::new(),
            dirty: true,
        }
    }

    fn send(&mut self, frame: &[u8; FRAME]) -> DriverResult<()> {
        // hidapi wants the report id first; this device uses none.
        let mut report = Vec::with_capacity(FRAME + 1);
        report.push(0);
        report.extend_from_slice(frame);
        self.transport.write(&report)
    }

    fn upload_grid(&mut self) -> DriverResult<()> {
        let size = self.model.image.map_or(196, |image| image.width);
        let mut icons = Vec::new();
        for key in 0..self.model.keys {
            let png = match self.faces.get(&key) {
                Some(png) => png.clone(),
                None => blank_png(size, size)?,
            };
            icons.push((key, png));
        }
        icons.push((STATUS_WINDOW_INDEX, blank_png(size * 2, size)?));
        let archive = grid_archive(&icons)?;
        for frame in chunked_frames(CMD_SET_BUTTONS, &archive) {
            self.send(&frame)?;
        }
        Ok(())
    }
}

fn blank_png(width: u32, height: u32) -> DriverResult<Vec<u8>> {
    let format = crate::catalog::KeyImage {
        width,
        height,
        mode: crate::catalog::ImageMode::Png,
        rotation: crate::catalog::Rotation::None,
        mirror: crate::catalog::Mirror::None,
    };
    face::encode(None, &format)
}

/// One frame carrying a short command.
fn frame(command: u16, data: &[u8]) -> [u8; FRAME] {
    let mut out = [0u8; FRAME];
    out[..2].copy_from_slice(&MAGIC);
    out[2..4].copy_from_slice(&command.to_be_bytes());
    out[4..8].copy_from_slice(&(data.len() as u32).to_le_bytes());
    let len = data.len().min(PAYLOAD);
    out[HEADER..HEADER + len].copy_from_slice(&data[..len]);
    out
}

/// A large payload: the first frame has the header and the TOTAL length; every
/// later frame is raw data with no header at all.
fn chunked_frames(command: u16, blob: &[u8]) -> Vec<[u8; FRAME]> {
    let mut frames = Vec::new();
    let mut first = frame(command, &blob[..blob.len().min(PAYLOAD)]);
    first[4..8].copy_from_slice(&(blob.len() as u32).to_le_bytes());
    frames.push(first);
    if blob.len() > PAYLOAD {
        for chunk in blob[PAYLOAD..].chunks(FRAME) {
            let mut raw = [0u8; FRAME];
            raw[..chunk.len()].copy_from_slice(chunk);
            frames.push(raw);
        }
    }
    frames
}

/// The `{col}_{row}` key a button index is filed under in the archive manifest.
fn grid_cell(index: u8, cols: u8) -> String {
    format!("{}_{}", index % cols, index / cols)
}

/// FNV-1a, to give every icon a content-derived name: the deck caches icons by
/// file name, so a changed face under an old name would keep showing the old one.
fn content_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn build_archive(icons: &[(u8, Vec<u8>)], padding: usize) -> DriverResult<Vec<u8>> {
    let zip_error = |error: zip::result::ZipError| DriverError::Image(error.to_string());
    let io_error = |error: std::io::Error| DriverError::Image(error.to_string());
    let mut manifest = serde_json::Map::new();
    let mut files = Vec::new();
    for (index, png) in icons {
        let name = format!(
            "icons/{index}-{:012x}.png",
            content_hash(png) & 0xffff_ffff_ffff
        );
        manifest.insert(
            grid_cell(*index, 5),
            serde_json::json!({ "State": 0, "ViewParam": [{ "Icon": name }] }),
        );
        files.push((name, png));
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let deflated = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(1));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer
        .start_file("manifest.json", deflated)
        .map_err(zip_error)?;
    writer
        .write_all(serde_json::Value::Object(manifest).to_string().as_bytes())
        .map_err(io_error)?;
    // Written before the icons so its length shifts every later byte.
    writer.start_file("dummy.txt", stored).map_err(zip_error)?;
    writer.write_all(&vec![b'A'; padding]).map_err(io_error)?;
    for (name, png) in files {
        writer.start_file(name, deflated).map_err(zip_error)?;
        writer.write_all(png).map_err(io_error)?;
    }
    // The firmware has been sensitive to the final entry; keep it an empty one.
    writer
        .start_file("sentinel.txt", stored)
        .map_err(zip_error)?;
    Ok(writer.finish().map_err(zip_error)?.into_inner())
}

/// The archive bytes at offsets 1016, 2040, 3064, ... (one per frame boundary,
/// counting from the end of the first frame's 1016-byte payload) must be neither
/// `00` nor `7C`. These are the offsets ulanzi-linux checks against hardware.
fn boundaries_are_safe(archive: &[u8]) -> bool {
    (PAYLOAD..archive.len())
        .step_by(FRAME)
        .all(|offset| !matches!(archive[offset], 0x00 | 0x7c))
}

fn grid_archive(icons: &[(u8, Vec<u8>)]) -> DriverResult<Vec<u8>> {
    for padding in 0..MAX_PADDING_ATTEMPTS {
        let archive = build_archive(icons, padding)?;
        if boundaries_are_safe(&archive) {
            return Ok(archive);
        }
    }
    Err(DriverError::Image(
        "could not lay out the button archive around the firmware's frame boundaries".to_owned(),
    ))
}

/// Parse one input report. The leading report-id byte is optional because
/// platforms disagree on whether to keep it for a device without numbered reports.
fn parse_report(report: &[u8]) -> Option<Input> {
    let start = report.windows(2).take(2).position(|pair| pair == MAGIC)?;
    let frame = &report[start..];
    if frame.len() < HEADER + 4 {
        return None;
    }
    let command = u16::from_be_bytes([frame[2], frame[3]]);
    if command != IN_BUTTON {
        return None;
    }
    let data = &frame[HEADER..];
    Some(Input::Key {
        index: data[1],
        pressed: data[3] == 0x01,
    })
}

impl Driver for UlanziD200 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = vec![0u8; FRAME + 1];
        for attempt in 0..8 {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            if let Some(Input::Key { index, pressed }) = parse_report(&buf[..read]) {
                if index < self.model.keys {
                    events.push(Input::Key { index, pressed });
                }
            }
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
        match face {
            Some(face) => {
                self.faces.insert(key, face::encode(Some(face), &format)?);
            }
            None => {
                self.faces.remove(&key);
            }
        }
        self.dirty = true;
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        if !self.dirty {
            return Ok(());
        }
        self.upload_grid()?;
        self.dirty = false;
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        // The firmware reads brightness as ASCII digits.
        let digits = percent.min(100).to_string();
        self.send(&frame(CMD_BRIGHTNESS, digits.as_bytes()))
    }

    fn heartbeat(&mut self) -> DriverResult<()> {
        // Clock mode: `mode|cpu|mem|time|gpu`.
        let now = chrono::Local::now().format("%H:%M:%S");
        let payload = format!("1|0|0|{now}|0");
        self.send(&frame(CMD_SMALL_WINDOW, payload.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::by_id;

    #[test]
    fn short_frames_carry_magic_command_and_le_length() {
        let out = frame(CMD_BRIGHTNESS, b"75");
        assert_eq!(
            &out[..10],
            &[0x7c, 0x7c, 0x00, 0x0a, 2, 0, 0, 0, b'7', b'5']
        );
        assert!(out[10..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn only_the_first_chunk_has_a_header() {
        let blob: Vec<u8> = (0..3000u32).map(|i| (i % 251) as u8).collect();
        let frames = chunked_frames(CMD_SET_BUTTONS, &blob);
        assert_eq!(frames.len(), 3);
        assert_eq!(&frames[0][..8], &[0x7c, 0x7c, 0x00, 0x01, 0xb8, 0x0b, 0, 0]);
        assert_eq!(&frames[0][8..], &blob[..PAYLOAD]);
        assert_eq!(&frames[1][..], &blob[PAYLOAD..PAYLOAD + FRAME]);
        assert_eq!(
            &frames[2][..3000 - PAYLOAD - FRAME],
            &blob[PAYLOAD + FRAME..]
        );
    }

    #[test]
    fn button_reports_parse_with_or_without_a_report_id() {
        let mut report = vec![0x7c, 0x7c, 0x01, 0x01, 4, 0, 0, 0, 0x00, 6, 0x01, 0x01];
        assert_eq!(
            parse_report(&report),
            Some(Input::Key {
                index: 6,
                pressed: true
            })
        );
        report.insert(0, 0x00);
        report[12] = 0x00;
        assert_eq!(
            parse_report(&report),
            Some(Input::Key {
                index: 6,
                pressed: false
            })
        );
        assert_eq!(
            parse_report(&[0x7c, 0x7c, 0x03, 0x03, 0, 0, 0, 0, 0, 0, 0, 0]),
            None
        );
        assert_eq!(parse_report(&[0; 16]), None);
    }

    #[test]
    fn the_grid_archive_avoids_bad_frame_boundaries_and_names_every_cell() {
        let icons: Vec<(u8, Vec<u8>)> = (0..13u8)
            .map(|key| (key, vec![key; 3000 + usize::from(key) * 17]))
            .chain(std::iter::once((STATUS_WINDOW_INDEX, vec![9; 5000])))
            .collect();
        let archive = grid_archive(&icons).unwrap();
        assert!(boundaries_are_safe(&archive));

        let mut zip = zip::ZipArchive::new(Cursor::new(archive)).unwrap();
        let mut manifest = String::new();
        zip.by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut manifest)
            .unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
        assert!(manifest.get("0_0").is_some());
        assert!(manifest.get("2_2").is_some(), "key 12");
        assert!(manifest.get("3_2").is_some(), "status window");
        let icon = manifest["4_1"]["ViewParam"][0]["Icon"].as_str().unwrap();
        assert!(icon.starts_with("icons/9-"));
        assert!(zip.by_name(icon).is_ok());
        let names: Vec<String> = zip
            .file_names()
            .map(|name| name.expect("zip entry name").into_owned())
            .collect();
        assert_eq!(names.last().map(String::as_str), Some("sentinel.txt"));
    }

    #[test]
    fn flush_uploads_the_grid_once_until_a_face_changes() {
        let recorder = Recorder::default();
        let mut deck = UlanziD200::new(by_id("ulanzi-d200").unwrap(), Box::new(recorder.clone()));
        deck.flush().unwrap();
        let first = recorder.writes().len();
        assert!(first > 1);
        assert!(recorder
            .writes()
            .iter()
            .all(|w| w.len() == FRAME + 1 && w[0] == 0));
        assert_eq!(&recorder.writes()[0][1..5], &[0x7c, 0x7c, 0x00, 0x01]);
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), first);
        deck.set_face(
            1,
            Some(&Face {
                label: String::new(),
                color: [255, 0, 0],
                image: None,
            }),
        )
        .unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().len() > first);
    }

    #[test]
    fn heartbeat_sends_a_clock_to_the_status_window() {
        let recorder = Recorder::default();
        let mut deck = UlanziD200::new(by_id("ulanzi-d200").unwrap(), Box::new(recorder.clone()));
        deck.heartbeat().unwrap();
        let sent = &recorder.writes()[0];
        assert_eq!(&sent[1..5], &[0x7c, 0x7c, 0x00, 0x06]);
        let text = String::from_utf8_lossy(&sent[9..9 + 16]);
        assert!(text.starts_with("1|0|0|"), "{text}");
    }
}
