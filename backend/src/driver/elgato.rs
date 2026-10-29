//! Elgato Stream Deck family.
//!
//! Input reports carry the state of every key at once, so presses are found by
//! comparing each report with the previous one. Key images go out as a series of
//! output reports ("pages"), each with a small header naming the key, the page
//! number and whether it is the last page. The Stream Deck +'s touch strip takes
//! its segments as rectangles of one 800 x 100 picture. The + XL's panels are
//! turned a quarter round: its 1200 x 100 strip is addressed as 100 x 1200, so
//! a segment's rectangle is written transposed.
//!
//! The Corsair Galleon 100 SD's panel speaks the + dialect, with its 720 x 384
//! screen drawn as two rectangles like the + strip. It also needs a keep-alive
//! feature report every 500 ms, and its dial rings take one LED per feature
//! report. Its byte layout was taken from @elgato-stream-deck/core (MIT); no
//! code was copied.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::{face, Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol, Rotation};

/// Big enough for any Stream Deck input report.
const READ_BUFFER: usize = 512;
/// Reports drained per poll, so a burst cannot starve the other devices.
const MAX_REPORTS_PER_POLL: usize = 16;
/// Stream Deck + touch-strip input kinds (byte 4 of a `0x02` report).
const STRIP_TAP: u8 = 0x01;
const STRIP_LONG_PRESS: u8 = 0x02;
const STRIP_SWIPE: u8 = 0x03;
/// Stream Deck Studio dial lights: a ring of 24 RGB segments and the knob.
const DIAL_RING: u8 = 0x0f;
const DIAL_KNOB: u8 = 0x10;
const RING_SEGMENTS: usize = 24;
/// Galleon 100 SD: the keep-alive feature report and how often it must go out.
const GALLEON_PING: [u8; 2] = [0x03, 0x27];
const GALLEON_PING_EVERY: Duration = Duration::from_millis(500);
/// How long the Galleon wants after the first ping before it is written to.
const GALLEON_SETTLE: Duration = Duration::from_millis(200);
/// Galleon 100 SD ring lights: feature `03 24 led r g b`, four LEDs per dial,
/// the last dial's LEDs first (dial 0 is LEDs 4 to 7, dial 1 is LEDs 0 to 3).
const GALLEON_RING_LED: u8 = 0x24;
const GALLEON_RING_LEDS: u8 = 4;

pub struct Elgato {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    keys: Vec<bool>,
    dials: Vec<bool>,
    staged: BTreeMap<u8, Vec<u8>>,
    dial_lights: BTreeMap<u8, [u8; 3]>,
    /// When the last keep-alive went out, on decks that need one.
    last_ping: Option<Instant>,
    /// The wait after the first keep-alive (zero in tests).
    settle: Duration,
}

impl Elgato {
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            model,
            transport,
            keys: vec![false; usize::from(model.keys)],
            dials: vec![false; usize::from(model.dials)],
            staged: BTreeMap::new(),
            dial_lights: BTreeMap::new(),
            last_ping: None,
            settle: GALLEON_SETTLE,
        }
    }

    /// Keeps a Galleon awake: a ping before anything else is sent (then a
    /// short wait), and again whenever 500 ms have passed. Other decks need none.
    fn keep_alive(&mut self) -> DriverResult<()> {
        if self.model.protocol != Protocol::ElgatoGalleon {
            return Ok(());
        }
        match self.last_ping {
            Some(at) if at.elapsed() < GALLEON_PING_EVERY => Ok(()),
            Some(_) => {
                self.transport.send_feature_report(&GALLEON_PING)?;
                self.last_ping = Some(Instant::now());
                Ok(())
            }
            None => {
                self.transport.send_feature_report(&GALLEON_PING)?;
                self.last_ping = Some(Instant::now());
                if !self.settle.is_zero() {
                    std::thread::sleep(self.settle);
                }
                Ok(())
            }
        }
    }

    /// The feature reports that colour one Galleon dial's ring of four LEDs.
    fn galleon_ring(&self, dial: u8, [r, g, b]: [u8; 3]) -> Vec<[u8; 6]> {
        let Some(slot) = self.model.dials.checked_sub(dial + 1) else {
            return Vec::new();
        };
        let first = slot * GALLEON_RING_LEDS;
        (first..first + GALLEON_RING_LEDS)
            .map(|led| [0x03, GALLEON_RING_LED, led, r, g, b])
            .collect()
    }

    /// The 2017 model numbers its keys right to left within each row.
    fn flip(&self, key: u8) -> u8 {
        let cols = self.model.cols;
        let col = key % cols;
        key - col + (cols - 1 - col)
    }

    fn parse(&mut self, report: &[u8]) -> Vec<Input> {
        if report.first() != Some(&0x01) {
            return Vec::new();
        }
        match self.model.protocol {
            Protocol::ElgatoOriginal => {
                let states: Vec<bool> = (0..self.model.keys)
                    .map(|key| byte_set(report, 1 + usize::from(self.flip(key))))
                    .collect();
                self.diff_keys(&states)
            }
            Protocol::ElgatoMini => {
                let states = flags(report, 1, self.model.keys);
                self.diff_keys(&states)
            }
            Protocol::ElgatoPlus | Protocol::ElgatoGalleon => match report.get(1) {
                Some(0x00) => {
                    let states = flags(report, 4, self.model.keys);
                    self.diff_keys(&states)
                }
                Some(0x02) => self.parse_strip(report),
                Some(0x03) => self.parse_dials(report),
                _ => Vec::new(),
            },
            _ => {
                let states = flags(report, 4, self.model.keys);
                self.diff_keys(&states)
            }
        }
    }

    fn parse_dials(&mut self, report: &[u8]) -> Vec<Input> {
        let count = self.model.dials;
        match report.get(4) {
            Some(0x00) => {
                let states = flags(report, 5, count);
                let mut events = Vec::new();
                for (index, (now, before)) in states.iter().zip(self.dials.iter_mut()).enumerate() {
                    if now != before {
                        *before = *now;
                        events.push(Input::DialPress {
                            index: index as u8,
                            pressed: *now,
                        });
                    }
                }
                events
            }
            Some(0x01) => (0..count)
                .filter_map(|index| {
                    let raw = *report.get(5 + usize::from(index))?;
                    let delta = i8::from_le_bytes([raw]);
                    (delta != 0).then_some(Input::DialTurn { index, delta })
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// A tap or long press presses the strip key under the finger; the strip
    /// reports no release, so the press is a click. A swipe names both ends.
    fn parse_strip(&self, report: &[u8]) -> Vec<Input> {
        let x_at = |offset: usize| {
            report
                .get(offset..offset + 2)
                .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        };
        let Some(start) = self.along_strip(x_at(6), x_at(8)) else {
            return Vec::new();
        };
        match report.get(4).copied() {
            Some(STRIP_TAP | STRIP_LONG_PRESS) => {
                let Some(index) = self.strip_key_at(start) else {
                    return Vec::new();
                };
                vec![
                    Input::Key {
                        index,
                        pressed: true,
                    },
                    Input::Key {
                        index,
                        pressed: false,
                    },
                ]
            }
            Some(STRIP_SWIPE) => {
                let Some(end) = self.along_strip(x_at(10), x_at(12)) else {
                    return Vec::new();
                };
                match (self.strip_key_at(start), self.strip_key_at(end)) {
                    (Some(from), Some(to)) => vec![Input::StripSwipe {
                        from,
                        to,
                        rightward: end >= start,
                    }],
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// How far along the strip a touch is. On a turned strip (the + XL) the
    /// long axis may arrive as either coordinate, so a value past the strip's
    /// height is the long one; below it both readings land in the first segment.
    fn along_strip(&self, x: Option<u16>, y: Option<u16>) -> Option<u16> {
        let format = self.model.strip_image?;
        let x = x?;
        if format.rotation == Rotation::ThreeQuarter {
            let y = y.unwrap_or(0);
            if u32::from(y) >= format.height {
                return Some(y);
            }
        }
        Some(x)
    }

    /// The strip key whose segment holds `x`.
    fn strip_key_at(&self, x: u16) -> Option<u8> {
        let width = self.model.strip_image?.width;
        let last = self.model.strip_keys.checked_sub(1)?;
        let segment = u8::try_from(u32::from(x) / width.max(1))
            .unwrap_or(u8::MAX)
            .min(last);
        Some(self.model.keys + segment)
    }

    fn diff_keys(&mut self, states: &[bool]) -> Vec<Input> {
        let mut events = Vec::new();
        for (index, (now, before)) in states.iter().zip(self.keys.iter_mut()).enumerate() {
            if now != before {
                *before = *now;
                events.push(Input::Key {
                    index: index as u8,
                    pressed: *now,
                });
            }
        }
        events
    }

    /// The output reports that draw one strip segment: a rectangle of the strip,
    /// paged with a 16-byte header (x, y, width, height, last flag, page, length).
    fn strip_reports(&self, segment: u8, data: &[u8]) -> Vec<Vec<u8>> {
        const REPORT_LEN: usize = 1024;
        const PAYLOAD_LEN: usize = REPORT_LEN - 16;
        let Some(format) = self.model.strip_image else {
            return Vec::new();
        };
        let width = format.width as u16;
        let height = format.height as u16;
        let along = u16::from(segment) * width;
        // A turned strip takes the rectangle transposed.
        let (x, y, width, height) = if format.rotation == Rotation::ThreeQuarter {
            (0, along, height, width)
        } else {
            (along, 0, width, height)
        };
        let pages = data.len().div_ceil(PAYLOAD_LEN).max(1);
        let mut reports = Vec::with_capacity(pages);
        for page in 0..pages {
            let chunk = &data
                [(page * PAYLOAD_LEN).min(data.len())..((page + 1) * PAYLOAD_LEN).min(data.len())];
            let len = chunk.len() as u16;
            let page_number = page as u16;
            let mut report = vec![0x02, 0x0c];
            for value in [x, y, width, height] {
                report.extend_from_slice(&value.to_le_bytes());
            }
            report.push(u8::from(page + 1 == pages));
            report.extend_from_slice(&page_number.to_le_bytes());
            report.extend_from_slice(&len.to_le_bytes());
            report.push(0);
            report.extend_from_slice(chunk);
            report.resize(REPORT_LEN, 0);
            reports.push(report);
        }
        reports
    }

    /// The output reports that carry one encoded key image.
    fn image_reports(&self, key: u8, data: &[u8]) -> Vec<Vec<u8>> {
        let (report_len, payload_len) = match self.model.protocol {
            // Always two pages: the image split in half.
            Protocol::ElgatoOriginal => (8191, data.len().div_ceil(2).max(1)),
            Protocol::ElgatoMini => (1024, 1024 - 16),
            _ => (1024, 1024 - 8),
        };
        let mut reports = Vec::new();
        for (page, chunk) in data.chunks(payload_len).enumerate() {
            let last = u8::from((page + 1) * payload_len >= data.len());
            let mut report = match self.model.protocol {
                Protocol::ElgatoOriginal => {
                    let mut header =
                        vec![0x02, 0x01, (page + 1) as u8, 0, last, self.flip(key) + 1];
                    header.resize(16, 0);
                    header
                }
                Protocol::ElgatoMini => {
                    let mut header = vec![0x02, 0x01, page as u8, 0, last, key + 1];
                    header.resize(16, 0);
                    header
                }
                _ => {
                    let len = chunk.len() as u16;
                    let page = page as u16;
                    vec![
                        0x02,
                        0x07,
                        key,
                        last,
                        (len & 0xff) as u8,
                        (len >> 8) as u8,
                        (page & 0xff) as u8,
                        (page >> 8) as u8,
                    ]
                }
            };
            report.extend_from_slice(chunk);
            report.resize(report_len, 0);
            reports.push(report);
        }
        reports
    }
}

/// The output reports that fill the Neo's info screen (`0x02 0x0b`, then an
/// 8-byte header like a key's, without the key).
fn info_reports(data: &[u8]) -> Vec<Vec<u8>> {
    const REPORT_LEN: usize = 1024;
    const PAYLOAD_LEN: usize = REPORT_LEN - 8;
    let pages = data.len().div_ceil(PAYLOAD_LEN).max(1);
    (0..pages)
        .map(|page| {
            let chunk = &data
                [(page * PAYLOAD_LEN).min(data.len())..((page + 1) * PAYLOAD_LEN).min(data.len())];
            let len = chunk.len() as u16;
            let mut report = vec![0x02, 0x0b, 0x00, u8::from(page + 1 == pages)];
            report.extend_from_slice(&len.to_le_bytes());
            report.extend_from_slice(&(page as u16).to_le_bytes());
            report.extend_from_slice(chunk);
            report.resize(REPORT_LEN, 0);
            report
        })
        .collect()
}

fn byte_set(report: &[u8], at: usize) -> bool {
    report.get(at).is_some_and(|byte| *byte != 0)
}

fn flags(report: &[u8], offset: usize, count: u8) -> Vec<bool> {
    (0..usize::from(count))
        .map(|index| byte_set(report, offset + index))
        .collect()
}

impl Driver for Elgato {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.keep_alive()?;
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
        if self.model.info_key() == Some(key) {
            if let Some(format) = self.model.info_image {
                self.staged.insert(key, face::encode(face, &format)?);
            }
            return Ok(());
        }
        if self.model.strip_segment(key).is_some() {
            if let Some(format) = self.model.strip_image {
                self.staged.insert(key, face::encode(face, &format)?);
            }
            return Ok(());
        }
        let Some(format) = self.model.image else {
            return Ok(());
        };
        // Touch buttons (the Neo's) have no screen to paint.
        if key >= self.model.screen_keys() {
            return Ok(());
        }
        self.staged.insert(key, face::encode(face, &format)?);
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.keep_alive()?;
        let staged = std::mem::take(&mut self.staged);
        for (key, data) in staged {
            let reports = if self.model.info_key() == Some(key) {
                info_reports(&data)
            } else {
                match self.model.strip_segment(key) {
                    Some(segment) => self.strip_reports(segment, &data),
                    None => self.image_reports(key, &data),
                }
            };
            for report in reports {
                self.transport.write(&report)?;
            }
        }
        if self.model.protocol == Protocol::ElgatoGalleon {
            for (dial, color) in std::mem::take(&mut self.dial_lights) {
                for report in self.galleon_ring(dial, color) {
                    self.transport.send_feature_report(&report)?;
                }
            }
            return Ok(());
        }
        // The ring takes one colour per segment; the knob one colour.
        for (dial, [r, g, b]) in std::mem::take(&mut self.dial_lights) {
            let mut ring = vec![0x02, DIAL_RING, dial];
            for _ in 0..RING_SEGMENTS {
                ring.extend_from_slice(&[r, g, b]);
            }
            self.transport.write(&ring)?;
            self.transport.write(&[0x02, DIAL_KNOB, dial, r, g, b])?;
        }
        Ok(())
    }

    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if dial < self.model.dials && self.model.dial_lights != crate::catalog::DialLights::None {
            self.dial_lights.insert(dial, color.unwrap_or([0, 0, 0]));
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.keep_alive()?;
        let percent = percent.min(100);
        let mut report = match self.model.protocol {
            Protocol::ElgatoOriginal | Protocol::ElgatoMini => {
                vec![0x05, 0x55, 0xaa, 0xd1, 0x01, percent]
            }
            _ => vec![0x03, 0x08, percent],
        };
        report.resize(
            match self.model.protocol {
                Protocol::ElgatoOriginal | Protocol::ElgatoMini => 17,
                _ => 32,
            },
            0,
        );
        self.transport.send_feature_report(&report)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::by_id;

    fn driver(model: &str) -> (Elgato, Recorder) {
        let recorder = Recorder::default();
        let driver = Elgato::new(by_id(model).unwrap(), Box::new(recorder.clone()));
        (driver, recorder)
    }

    fn v2_report(keys: &[u8], total: usize) -> Vec<u8> {
        let mut report = vec![0x01, 0x00, total as u8, 0x00];
        report.resize(4 + total, 0);
        for key in keys {
            report[4 + usize::from(*key)] = 1;
        }
        report
    }

    #[test]
    fn v2_reports_become_press_and_release_events() {
        let (mut deck, recorder) = driver("elgato-mk2");
        recorder.queue_read(v2_report(&[3], 15));
        recorder.queue_read(v2_report(&[], 15));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 3,
                    pressed: true
                },
                Input::Key {
                    index: 3,
                    pressed: false
                }
            ]
        );
    }

    #[test]
    fn neo_touch_buttons_follow_the_screen_keys() {
        let (mut deck, recorder) = driver("elgato-neo");
        recorder.queue_read(v2_report(&[9], 10));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 9,
                pressed: true
            }]
        );
        deck.set_face(8, None).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty(), "touch buttons have no screen");
    }

    #[test]
    fn the_neo_info_screen_is_filled_whole() {
        let (mut deck, recorder) = driver("elgato-neo");
        let face = Face {
            label: String::new(),
            color: [0, 120, 60],
            image: None,
        };
        deck.set_face(10, Some(&face)).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(&writes[0][..3], &[0x02, 0x0b, 0x00]);
        assert_eq!(writes.last().unwrap()[3], 1, "last page flagged");
        let jpeg: Vec<u8> = writes
            .iter()
            .flat_map(|report| {
                let len = usize::from(u16::from_le_bytes([report[4], report[5]]));
                report[8..8 + len].to_vec()
            })
            .collect();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (248, 58));
    }

    #[test]
    fn the_original_deck_reports_keys_mirrored() {
        let (mut deck, recorder) = driver("elgato-original");
        // Physical top-left key arrives in the slot of the top-right one.
        let mut report = vec![0x01];
        report.resize(16, 0);
        report[1 + 4] = 1;
        recorder.queue_read(report);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 0,
                pressed: true
            }]
        );
    }

    #[test]
    fn mini_reports_start_right_after_the_report_id() {
        let (mut deck, recorder) = driver("elgato-mini");
        recorder.queue_read(vec![0x01, 0, 0, 0, 0, 0, 1]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 5,
                pressed: true
            }]
        );
    }

    #[test]
    fn plus_dials_report_presses_and_turns() {
        let (mut deck, recorder) = driver("elgato-plus");
        recorder.queue_read(vec![0x01, 0x03, 0x05, 0x00, 0x00, 0, 1, 0, 0]);
        recorder.queue_read(vec![0x01, 0x03, 0x05, 0x00, 0x01, 0, 0, 0xfe, 3]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialPress {
                    index: 1,
                    pressed: true
                },
                Input::DialTurn {
                    index: 2,
                    delta: -2
                },
                Input::DialTurn { index: 3, delta: 3 },
            ]
        );
    }

    fn strip_report(kind: u8, start: u16, end: u16) -> Vec<u8> {
        let mut report = vec![0x01, 0x02, 0x0e, 0x00, kind, 0x00];
        report.extend_from_slice(&start.to_le_bytes());
        report.extend_from_slice(&40u16.to_le_bytes());
        report.extend_from_slice(&end.to_le_bytes());
        report.extend_from_slice(&40u16.to_le_bytes());
        report
    }

    #[test]
    fn plus_strip_taps_press_the_segment_under_the_finger() {
        let (mut deck, recorder) = driver("elgato-plus");
        recorder.queue_read(strip_report(STRIP_TAP, 250, 0));
        recorder.queue_read(strip_report(STRIP_LONG_PRESS, 799, 0));
        // Off the right edge still lands on the last segment.
        recorder.queue_read(strip_report(STRIP_TAP, 900, 0));
        recorder.queue_read(strip_report(STRIP_SWIPE, 620, 30));
        let inputs = deck.poll(0).unwrap();
        let pressed: Vec<u8> = inputs
            .iter()
            .filter_map(|input| match input {
                Input::Key {
                    index,
                    pressed: true,
                } => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(pressed, vec![9, 11, 11]);
        assert_eq!(
            inputs.last(),
            Some(&Input::StripSwipe {
                from: 11,
                to: 8,
                rightward: false
            })
        );
    }

    #[test]
    fn plus_strip_faces_draw_their_segment_of_the_lcd() {
        let (mut deck, recorder) = driver("elgato-plus");
        let face = Face {
            label: String::new(),
            color: [200, 10, 10],
            image: None,
        };
        deck.set_face(10, Some(&face)).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert!(!writes.is_empty());
        let first = &writes[0];
        assert_eq!(first.len(), 1024);
        // Segment 2: x = 400, y = 0, 200 x 100.
        assert_eq!(
            &first[..10],
            &[0x02, 0x0c, 0x90, 0x01, 0, 0, 200, 0, 100, 0]
        );
        assert_eq!(writes.last().unwrap()[10], 1, "last page flagged");
        assert_eq!(&first[16..18], &[0xff, 0xd8], "a JPEG");
        let len = usize::from(u16::from_le_bytes([first[13], first[14]]));
        assert!(len > 0 && len <= 1008);
    }

    #[test]
    fn studio_keys_are_tall_and_its_dials_report_like_the_plus() {
        let (mut deck, recorder) = driver("elgato-studio");
        let mut report = vec![0x01, 0x00, 32, 0x00];
        report.resize(4 + 32, 0);
        report[4 + 20] = 1;
        recorder.queue_read(report);
        recorder.queue_read(vec![0x01, 0x03, 0x05, 0x00, 0x01, 0xff, 0x02]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 20,
                    pressed: true
                },
                Input::DialTurn {
                    index: 0,
                    delta: -1
                },
                Input::DialTurn { index: 1, delta: 2 },
            ]
        );
        let face = Face {
            label: String::new(),
            color: [9, 9, 9],
            image: None,
        };
        deck.set_face(31, Some(&face)).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(&writes[0][..3], &[0x02, 0x07, 31]);
        let jpeg: Vec<u8> = writes
            .iter()
            .flat_map(|report| {
                let len = usize::from(u16::from_le_bytes([report[4], report[5]]));
                report[8..8 + len].to_vec()
            })
            .collect();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (80, 120));
    }

    #[test]
    fn studio_dial_lights_colour_the_ring_and_knob() {
        let (mut deck, recorder) = driver("elgato-studio");
        deck.set_dial_light(1, Some([10, 20, 30])).unwrap();
        deck.set_dial_light(0, None).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 4);
        // Dial 0 off first, then dial 1.
        assert_eq!(&writes[0][..6], &[0x02, DIAL_RING, 0, 0, 0, 0]);
        assert_eq!(writes[2].len(), 3 + 3 * RING_SEGMENTS);
        assert_eq!(&writes[2][3..9], &[10, 20, 30, 10, 20, 30]);
        assert_eq!(writes[3], vec![0x02, DIAL_KNOB, 1, 10, 20, 30]);
        // A deck without dial lights ignores them.
        let (mut plus, recorder) = driver("elgato-plus");
        plus.set_dial_light(0, Some([1, 2, 3])).unwrap();
        plus.flush().unwrap();
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn plus_xl_strip_is_drawn_transposed() {
        let (mut deck, recorder) = driver("elgato-plus-xl");
        let face = Face {
            label: String::new(),
            color: [10, 10, 200],
            image: None,
        };
        // Strip key 39 is the fourth segment (36 grid keys first).
        deck.set_face(39, Some(&face)).unwrap();
        deck.flush().unwrap();
        let first = &recorder.writes()[0];
        // x 0, y 600, 100 wide, 200 tall: the 200 x 100 segment, turned.
        assert_eq!(
            &first[..10],
            &[0x02, 0x0c, 0, 0, 0x58, 0x02, 100, 0, 200, 0]
        );
    }

    #[test]
    fn plus_xl_touches_read_along_either_axis() {
        let (mut deck, recorder) = driver("elgato-plus-xl");
        // Landscape coordinates: x 450 is the third segment.
        recorder.queue_read(strip_report(STRIP_TAP, 450, 0));
        // Portrait coordinates: the long axis arrives as y (y 1050, sixth).
        let mut portrait = vec![0x01, 0x02, 0x0e, 0x00, STRIP_TAP, 0x00];
        portrait.extend_from_slice(&40u16.to_le_bytes());
        portrait.extend_from_slice(&1050u16.to_le_bytes());
        recorder.queue_read(portrait);
        let pressed: Vec<u8> = deck
            .poll(0)
            .unwrap()
            .into_iter()
            .filter_map(|input| match input {
                Input::Key {
                    index,
                    pressed: true,
                } => Some(index),
                _ => None,
            })
            .collect();
        assert_eq!(pressed, vec![38, 41]);
    }

    #[test]
    fn v2_images_are_paged_with_length_and_last_flag() {
        let (deck, _) = driver("elgato-mk2");
        let data = vec![0xab; 2000];
        let reports = deck.image_reports(7, &data);
        assert_eq!(reports.len(), 2);
        assert!(reports.iter().all(|report| report.len() == 1024));
        assert_eq!(&reports[0][..8], &[0x02, 0x07, 7, 0, 0xf8, 0x03, 0, 0]);
        // 2000 - 1016 = 984 = 0x03d8 bytes on the last page.
        assert_eq!(&reports[1][..8], &[0x02, 0x07, 7, 1, 0xd8, 0x03, 1, 0]);
        assert_eq!(reports[1][8 + 983], 0xab);
        assert_eq!(reports[1][8 + 984], 0);
    }

    #[test]
    fn the_original_deck_takes_two_large_pages_and_a_mirrored_key() {
        let (deck, _) = driver("elgato-original");
        let reports = deck.image_reports(0, &vec![1; 15_606]);
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].len(), 8191);
        assert_eq!(&reports[0][..6], &[0x02, 0x01, 1, 0, 0, 5]);
        assert_eq!(&reports[1][..6], &[0x02, 0x01, 2, 0, 1, 5]);
    }

    #[test]
    fn flush_writes_staged_faces_once() {
        let (mut deck, recorder) = driver("elgato-mini");
        deck.set_face(2, None).unwrap();
        deck.flush().unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert!(!writes.is_empty());
        assert!(writes.iter().all(|report| report[5] == 3));
        assert_eq!(writes.last().unwrap()[4], 1, "last page flagged");
    }

    #[test]
    fn pedals_have_no_screens_to_paint() {
        let (mut deck, recorder) = driver("elgato-pedal");
        deck.set_face(0, None).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
    }

    fn galleon() -> (Elgato, Recorder) {
        let (mut deck, recorder) = driver("corsair-galleon-100-sd");
        deck.settle = Duration::ZERO;
        (deck, recorder)
    }

    #[test]
    fn galleon_keys_and_dials_read_like_the_plus() {
        let (mut deck, recorder) = galleon();
        // Key 11 is the bottom-right key, numbered in reading order.
        recorder.queue_read(v2_report(&[0, 11], 12));
        recorder.queue_read(v2_report(&[11], 12));
        recorder.queue_read(vec![0x01, 0x03, 0x05, 0x00, 0x00, 1, 0]);
        recorder.queue_read(vec![0x01, 0x03, 0x05, 0x00, 0x01, 0xfd, 1]);
        // Short and malformed reports are ignored.
        recorder.queue_read(vec![0x01, 0x03]);
        recorder.queue_read(vec![0x01]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 0,
                    pressed: true
                },
                Input::Key {
                    index: 11,
                    pressed: true
                },
                Input::Key {
                    index: 0,
                    pressed: false
                },
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                Input::DialTurn {
                    index: 0,
                    delta: -3
                },
                Input::DialTurn { index: 1, delta: 1 },
            ]
        );
    }

    #[test]
    fn galleon_screen_touches_land_on_the_half_under_the_finger() {
        let (mut deck, recorder) = galleon();
        recorder.queue_read(strip_report(STRIP_TAP, 100, 0));
        recorder.queue_read(strip_report(STRIP_TAP, 360, 0));
        let pressed: Vec<u8> = deck
            .poll(0)
            .unwrap()
            .into_iter()
            .filter_map(|input| match input {
                Input::Key {
                    index,
                    pressed: true,
                } => Some(index),
                _ => None,
            })
            .collect();
        assert_eq!(pressed, vec![12, 13]);
    }

    #[test]
    fn galleon_pings_first_and_every_half_second() {
        let (mut deck, recorder) = galleon();
        deck.poll(0).unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.features(), vec![GALLEON_PING.to_vec()]);
        deck.last_ping = Instant::now().checked_sub(GALLEON_PING_EVERY);
        deck.poll(0).unwrap();
        assert_eq!(recorder.features().len(), 2);
        // Other decks are never pinged.
        let (mut plus, recorder) = driver("elgato-plus");
        plus.poll(0).unwrap();
        assert!(recorder.features().is_empty());
    }

    #[test]
    fn galleon_faces_are_unmirrored_160_px_keys_and_screen_halves() {
        let (mut deck, recorder) = galleon();
        let face = Face {
            label: String::new(),
            color: [0, 0, 200],
            image: None,
        };
        deck.set_face(4, Some(&face)).unwrap();
        deck.set_face(13, Some(&face)).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.features()[0], GALLEON_PING.to_vec());
        let writes = recorder.writes();
        assert!(writes.iter().all(|report| report.len() == 1024));
        let (keys, screen): (Vec<_>, Vec<_>) = writes.iter().partition(|report| report[1] == 0x07);
        assert_eq!(&keys[0][..4], &[0x02, 0x07, 4, u8::from(keys.len() == 1)]);
        let jpeg: Vec<u8> = keys
            .iter()
            .flat_map(|report| {
                let len = usize::from(u16::from_le_bytes([report[4], report[5]]));
                report[8..8 + len].to_vec()
            })
            .collect();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (160, 160));
        // The right half of the screen: x 360, y 0, 360 x 384.
        assert_eq!(
            &screen[0][..10],
            &[0x02, 0x0c, 0x68, 0x01, 0, 0, 0x68, 0x01, 0x80, 0x01]
        );
        assert_eq!(screen.last().unwrap()[10], 1, "last page flagged");
    }

    #[test]
    fn galleon_dial_rings_are_set_one_led_at_a_time() {
        let (mut deck, recorder) = galleon();
        deck.set_dial_light(0, Some([1, 2, 3])).unwrap();
        deck.set_dial_light(1, None).unwrap();
        deck.set_dial_light(2, Some([9, 9, 9])).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty(), "no Studio ring reports");
        let features = recorder.features();
        assert_eq!(features[0], GALLEON_PING.to_vec());
        assert_eq!(
            features[1..].to_vec(),
            vec![
                vec![0x03, 0x24, 4, 1, 2, 3],
                vec![0x03, 0x24, 5, 1, 2, 3],
                vec![0x03, 0x24, 6, 1, 2, 3],
                vec![0x03, 0x24, 7, 1, 2, 3],
                vec![0x03, 0x24, 0, 0, 0, 0],
                vec![0x03, 0x24, 1, 0, 0, 0],
                vec![0x03, 0x24, 2, 0, 0, 0],
                vec![0x03, 0x24, 3, 0, 0, 0],
            ]
        );
    }

    #[test]
    fn galleon_brightness_is_the_gen2_feature_report() {
        let (mut deck, recorder) = galleon();
        deck.set_brightness(55).unwrap();
        let features = recorder.features();
        assert_eq!(features[0], GALLEON_PING.to_vec());
        assert_eq!(features[1].len(), 32);
        assert_eq!(&features[1][..4], &[0x03, 0x08, 55, 0]);
    }

    #[test]
    fn brightness_uses_each_generation_feature_report() {
        let (mut mk2, recorder) = driver("elgato-mk2");
        mk2.set_brightness(150).unwrap();
        assert_eq!(&recorder.features()[0][..3], &[0x03, 0x08, 100]);
        let (mut mini, recorder) = driver("elgato-mini");
        mini.set_brightness(40).unwrap();
        assert_eq!(
            &recorder.features()[0][..6],
            &[0x05, 0x55, 0xaa, 0xd1, 0x01, 40]
        );
    }
}
