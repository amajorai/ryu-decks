//! Logitech MX Creative Console: the Keypad (nine screen keys behind one 480 x 480
//! panel, plus two page buttons) and the Dialpad (four buttons, a small roller and
//! a large dial).
//!
//! Both speak HID++ 2.0 on a vendor collection. A request is a 20-byte long
//! report `[0x11, 0xff, feature, function << 4 | software id, params...]`, and the
//! device takes one at a time: anything sent before the previous request is
//! answered is dropped. So every write here waits for its answer, and reports
//! that arrive meanwhile (key presses, dial turns) are queued for the next poll.
//! An answer echoes the device index, the feature and the function byte; a
//! notification has 0 in the function byte.
//!
//! The controls report nothing until they are diverted to the host, which the
//! driver does once, 100 ms after the device opens. Key faces are JPEG rectangles
//! of the panel in panel pixels, sent as 4095-byte reports `0x14` and answered on
//! report `0x13` (the one the screen keys report presses on, told apart by the
//! function byte). Feature indexes are the ones current firmware uses; they are
//! not looked up through the HID++ root feature.

use std::collections::{BTreeMap, VecDeque};
use std::ops::Range;
use std::time::{Duration, Instant};

use super::{face, Driver, DriverError, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const LONG_REPORT: u8 = 0x11;
const LONG_LEN: usize = 20;
const KEY_REPORT: u8 = 0x13;
const IMAGE_REPORT: u8 = 0x14;
/// The device index of a directly attached (not receiver-paired) device.
const DEVICE_INDEX: u8 = 0xff;
const SOFTWARE_ID: u8 = 0x0b;

/// Keypad feature indexes.
const KEYPAD_IMAGE: u8 = 0x02;
const KEYPAD_CONTROLS: u8 = 0x0b;
const KEYPAD_BRIGHTNESS: u8 = 0x0f;
/// Dialpad feature indexes.
const DIALPAD_CONTROLS: u8 = 0x0a;
const DIALPAD_DIALS: u8 = 0x0d;

/// Function 3 of the controls feature diverts a control to the host; function 3
/// of the dial feature turns a dial's reports on.
const DIVERT: u8 = 3;
const SET_BRIGHTNESS: u8 = 2;
const WRITE_IMAGE: u8 = 2;

/// The keypad's page buttons, extra keys 9 and 10.
const PAGE_LEFT: u16 = 0x01a1;
const PAGE_RIGHT: u16 = 0x01a2;
/// The dialpad's buttons in key order: top left, top middle, bottom left, bottom right.
const DIALPAD_BUTTONS: [u16; 4] = [0x0053, 0x0056, 0x0059, 0x005a];

const IMAGE_PACKET_LEN: usize = 4095;
const FIRST_HEADER_LEN: usize = 20;
const LATER_HEADER_LEN: usize = 5;
/// The image length field is 16 bits.
const MAX_JPEG_LEN: usize = 0xffff;
const IMAGE_ACCEPTED: u8 = 0x00;
/// Too many images queued on the device: send the whole image again.
const IMAGE_BUSY: u8 = 0xfb;

/// Key faces sit on the panel in a 3 x 3 grid: 118 px keys, 40 px apart.
const PANEL_COLS: u8 = 3;
const KEY_ORIGIN: (u16, u16) = (23, 6);
const KEY_PITCH: u16 = 158;

const SETTLE: Duration = Duration::from_millis(100);
const INIT_TIMEOUT: Duration = Duration::from_millis(250);
const INIT_ATTEMPTS: usize = 3;
const REQUEST_TIMEOUT: Duration = Duration::from_millis(500);
const BUSY_RETRY: Duration = Duration::from_millis(1);
const BUSY_LIMIT: Duration = Duration::from_secs(1);
/// How long a keypad that failed an image is left alone before the faces it
/// did not take are sent again. Each attempt can block the device loop (and
/// every other deck's input), so it is not repeated on every repaint.
const STALL_RETRY: Duration = Duration::from_secs(30);

const READ_BUFFER: usize = 512;
const MAX_REPORTS_PER_POLL: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Keypad,
    Dialpad,
}

/// How a request ended.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    /// The answer's bytes after the report id.
    Accepted(Vec<u8>),
    /// A HID++ error answer, with its error code.
    Refused(u8),
    TimedOut,
}

pub struct Logitech {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    kind: Kind,
    opened: Instant,
    ready: bool,
    keys: Vec<bool>,
    /// Input that arrived while a request waited for its answer.
    pending: VecDeque<Input>,
    staged: BTreeMap<u8, Vec<u8>>,
    /// After a failed flush, when the faces it could not send are tried again.
    retry_at: Option<Instant>,
}

/// A 20-byte HID++ long request.
fn long_request(feature: u8, function: u8, params: &[u8]) -> Vec<u8> {
    let mut packet = vec![
        LONG_REPORT,
        DEVICE_INDEX,
        feature,
        (function << 4) | SOFTWARE_ID,
    ];
    packet.extend_from_slice(params);
    packet.resize(LONG_LEN, 0);
    packet
}

fn divert(feature: u8, control: u16) -> Vec<u8> {
    let [high, low] = control.to_be_bytes();
    long_request(feature, DIVERT, &[high, low, 0x03])
}

/// The control ids in a controls notification: big-endian pairs from byte 3 of
/// the data, up to the first zero.
fn control_ids(data: &[u8]) -> Vec<u16> {
    data.get(3..)
        .unwrap_or_default()
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .take_while(|id| *id != 0)
        .collect()
}

/// The panel origin of a screen key.
fn key_origin(key: u8) -> (u16, u16) {
    let col = u16::from(key % PANEL_COLS);
    let row = u16::from(key / PANEL_COLS);
    (
        KEY_ORIGIN.0 + col * KEY_PITCH,
        KEY_ORIGIN.1 + row * KEY_PITCH,
    )
}

/// The reports that draw `jpeg` at `(x, y)`, `width` x `height` panel pixels.
fn image_packets(x: u16, y: u16, width: u16, height: u16, jpeg: &[u8]) -> Vec<Vec<u8>> {
    let first_room = IMAGE_PACKET_LEN - FIRST_HEADER_LEN;
    let later_room = IMAGE_PACKET_LEN - LATER_HEADER_LEN;
    let split = first_room.min(jpeg.len());
    let mut chunks = vec![&jpeg[..split]];
    chunks.extend(jpeg[split..].chunks(later_room));
    let last = chunks.len() - 1;
    let length = u16::try_from(jpeg.len()).unwrap_or(u16::MAX).to_be_bytes();
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            // 1-based part number, 0x20 always, 0x80 on the first and 0x40 on the last.
            let mut part = ((index + 1) as u8 & 0x1f) | 0x20;
            if index == 0 {
                part |= 0x80;
            }
            if index == last {
                part |= 0x40;
            }
            let mut packet = vec![
                IMAGE_REPORT,
                DEVICE_INDEX,
                KEYPAD_IMAGE,
                (WRITE_IMAGE << 4) | SOFTWARE_ID,
                part,
            ];
            if index == 0 {
                packet.extend_from_slice(&[0x01, 0x00, 0x01, 0x00]);
                for value in [x, y, width, height] {
                    packet.extend_from_slice(&value.to_be_bytes());
                }
                packet.push(0x00);
                packet.extend_from_slice(&length);
            }
            packet.extend_from_slice(chunk);
            packet.resize(IMAGE_PACKET_LEN, 0);
            packet
        })
        .collect()
}

impl Logitech {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = if model.image.is_some() {
            Kind::Keypad
        } else {
            Kind::Dialpad
        };
        Self {
            model,
            transport,
            kind,
            opened: Instant::now(),
            ready: false,
            keys: vec![false; usize::from(model.keys)],
            pending: VecDeque::new(),
            staged: BTreeMap::new(),
            retry_at: None,
        }
    }

    /// The writes that divert the controls (and, on the dialpad, turn the dials on).
    fn init_requests(&self) -> Vec<Vec<u8>> {
        match self.kind {
            Kind::Keypad => vec![
                divert(KEYPAD_CONTROLS, PAGE_LEFT),
                divert(KEYPAD_CONTROLS, PAGE_RIGHT),
            ],
            Kind::Dialpad => {
                let mut requests: Vec<Vec<u8>> = DIALPAD_BUTTONS
                    .iter()
                    .map(|control| divert(DIALPAD_CONTROLS, *control))
                    .collect();
                for dial in [0, 1] {
                    requests.push(long_request(DIALPAD_DIALS, DIVERT, &[dial, 0x01]));
                }
                requests
            }
        }
    }

    /// Diverts the controls once, after the device has settled. A write the
    /// device never answers is retried, then given up on; once one goes
    /// unanswered the rest are sent only once, so a silent device holds up the
    /// device loop for about two seconds, not five. Only an I/O failure is an
    /// error.
    fn ensure_ready(&mut self) -> DriverResult<()> {
        if self.ready {
            return Ok(());
        }
        self.ready = true;
        let settle = SETTLE.saturating_sub(self.opened.elapsed());
        if !settle.is_zero() {
            std::thread::sleep(settle);
        }
        let mut attempts = INIT_ATTEMPTS;
        for request in self.init_requests() {
            let answer = self.request(&request, INIT_TIMEOUT, attempts)?;
            if answer != Answer::TimedOut {
                continue;
            }
            attempts = 1;
            tracing::debug!(
                model = self.model.id,
                "ryu-decks: an MX Creative Console set-up write went unanswered"
            );
        }
        Ok(())
    }

    /// Sends a request and waits for its answer, sending it again on a timeout.
    fn request(
        &mut self,
        packet: &[u8],
        timeout: Duration,
        attempts: usize,
    ) -> DriverResult<Answer> {
        let (Some(&feature), Some(&function)) = (packet.get(2), packet.get(3)) else {
            return Ok(Answer::TimedOut);
        };
        for _ in 0..attempts {
            self.transport.write(packet)?;
            let answer = self.await_answer(feature, function, timeout)?;
            if answer != Answer::TimedOut {
                return Ok(answer);
            }
        }
        Ok(Answer::TimedOut)
    }

    /// Reads until the answer to `(feature, function)` arrives, queueing any
    /// input that comes first.
    fn await_answer(
        &mut self,
        feature: u8,
        function: u8,
        timeout: Duration,
    ) -> DriverResult<Answer> {
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; READ_BUFFER];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(Answer::TimedOut);
            }
            let wait = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                return Ok(Answer::TimedOut);
            }
            let report = &buf[..read];
            let data = report.get(1..).unwrap_or_default();
            match data {
                [DEVICE_INDEX, 0xff, failed, echo, code, ..]
                    if *failed == feature && *echo == function =>
                {
                    return Ok(Answer::Refused(*code));
                }
                [DEVICE_INDEX, answered, echo, ..] if *answered == feature && *echo == function => {
                    return Ok(Answer::Accepted(data.to_vec()));
                }
                _ => {
                    let inputs = self.parse(report);
                    self.pending.extend(inputs);
                }
            }
        }
    }

    fn parse(&mut self, report: &[u8]) -> Vec<Input> {
        let Some(&report_id) = report.first() else {
            return Vec::new();
        };
        let data = report.get(1..).unwrap_or_default();
        // Only a zero function byte is a notification; the rest are answers.
        let (Some(&DEVICE_INDEX), Some(&feature), Some(&0)) =
            (data.first(), data.get(1), data.get(2))
        else {
            return Vec::new();
        };
        let mut events = Vec::new();
        match (self.kind, report_id, feature) {
            (Kind::Keypad, KEY_REPORT, KEYPAD_IMAGE) if data.get(4) == Some(&0x01) => {
                // The screen keys held now, as 1-based codes, up to the first zero.
                let held: Vec<u8> = data
                    .get(5..)
                    .unwrap_or_default()
                    .iter()
                    .copied()
                    .take_while(|code| *code != 0)
                    .collect();
                let screens = usize::from(self.model.screen_keys());
                self.diff(
                    0..screens,
                    |key| held.contains(&(key as u8 + 1)),
                    &mut events,
                );
            }
            (Kind::Keypad, LONG_REPORT, KEYPAD_CONTROLS) => {
                let held = control_ids(data);
                let first = usize::from(self.model.screen_keys());
                let pages = [PAGE_LEFT, PAGE_RIGHT];
                self.diff(
                    first..first + pages.len(),
                    |key| held.contains(&pages[key - first]),
                    &mut events,
                );
            }
            (Kind::Dialpad, LONG_REPORT, DIALPAD_CONTROLS) => {
                let held = control_ids(data);
                self.diff(
                    0..DIALPAD_BUTTONS.len(),
                    |key| held.contains(&DIALPAD_BUTTONS[key]),
                    &mut events,
                );
            }
            (Kind::Dialpad, LONG_REPORT, DIALPAD_DIALS) => {
                let (Some(&dial), Some(&raw)) = (data.get(3), data.get(4)) else {
                    return events;
                };
                let delta = i8::from_le_bytes([raw]);
                // The small roller (dial 0) reports the opposite way round.
                let delta = if dial == 0 {
                    delta.saturating_neg()
                } else {
                    delta
                };
                if dial < self.model.dials && delta != 0 {
                    events.push(Input::DialTurn { index: dial, delta });
                }
            }
            _ => {}
        }
        events
    }

    /// Raises a key event for each key in `range` whose state changed.
    fn diff(&mut self, range: Range<usize>, held: impl Fn(usize) -> bool, events: &mut Vec<Input>) {
        for key in range {
            let pressed = held(key);
            let Some(state) = self.keys.get_mut(key) else {
                continue;
            };
            if *state != pressed {
                *state = pressed;
                events.push(Input::Key {
                    index: key as u8,
                    pressed,
                });
            }
        }
    }

    /// Draws one JPEG rectangle, waiting for each packet's answer. A busy device
    /// gets the whole image again, for up to a second.
    fn write_image(&mut self, x: u16, y: u16, side: u16, jpeg: &[u8]) -> DriverResult<()> {
        let packets = image_packets(x, y, side, side, jpeg);
        let function = (WRITE_IMAGE << 4) | SOFTWARE_ID;
        let mut busy_since: Option<Instant> = None;
        'image: loop {
            for packet in &packets {
                self.transport.write(packet)?;
                let status = match self.await_answer(KEYPAD_IMAGE, function, REQUEST_TIMEOUT)? {
                    Answer::Accepted(data) => data.get(4).copied().unwrap_or(IMAGE_ACCEPTED),
                    Answer::Refused(code) => {
                        return Err(DriverError::Io(format!(
                            "the keypad refused an image (HID++ error {code:#04x})"
                        )));
                    }
                    Answer::TimedOut => {
                        return Err(DriverError::Io(
                            "the keypad did not answer an image write".to_owned(),
                        ));
                    }
                };
                match status {
                    IMAGE_ACCEPTED => {}
                    IMAGE_BUSY => {
                        let since = *busy_since.get_or_insert_with(Instant::now);
                        if since.elapsed() >= BUSY_LIMIT {
                            return Err(DriverError::Io(
                                "the keypad stayed busy for a second".to_owned(),
                            ));
                        }
                        std::thread::sleep(BUSY_RETRY);
                        continue 'image;
                    }
                    other => {
                        return Err(DriverError::Io(format!(
                            "the keypad rejected an image (status {other:#04x})"
                        )));
                    }
                }
            }
            return Ok(());
        }
    }
}

impl Driver for Logitech {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_ready()?;
        let mut events: Vec<Input> = self.pending.drain(..).collect();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 && events.is_empty() {
                timeout_ms
            } else {
                0
            };
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
        if key >= self.model.screen_keys() {
            return Ok(());
        }
        let jpeg = face::encode(face, &format)?;
        if jpeg.len() > MAX_JPEG_LEN {
            return Err(DriverError::Image(format!(
                "a {} byte JPEG is over the keypad's 64 KiB limit",
                jpeg.len()
            )));
        }
        self.staged.insert(key, jpeg);
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_ready()?;
        let Some(format) = self.model.image else {
            return Ok(());
        };
        if self.retry_at.is_some_and(|at| Instant::now() < at) {
            return Ok(());
        }
        let side = u16::try_from(format.width).unwrap_or(u16::MAX);
        let mut staged = std::mem::take(&mut self.staged);
        while let Some((key, jpeg)) = staged.pop_first() {
            let (x, y) = key_origin(key);
            if let Err(error) = self.write_image(x, y, side, &jpeg) {
                // Keep this face and the rest for the heartbeat to send.
                staged.insert(key, jpeg);
                for (key, jpeg) in staged {
                    self.staged.entry(key).or_insert(jpeg);
                }
                self.retry_at = Some(Instant::now() + STALL_RETRY);
                return Err(error);
            }
        }
        self.retry_at = None;
        Ok(())
    }

    /// Retries the faces a failed flush could not send, once the pause is over.
    fn heartbeat(&mut self) -> DriverResult<()> {
        let due = self.retry_at.is_some_and(|at| Instant::now() >= at);
        if !due || self.staged.is_empty() {
            return Ok(());
        }
        self.retry_at = None;
        if let Err(error) = self.flush() {
            tracing::debug!(
                model = self.model.id,
                %error,
                "ryu-decks: the MX Creative Keypad still does not take its faces"
            );
        }
        Ok(())
    }

    /// The keypad's panel, 1 to 100 percent: 0 would reset the device. The
    /// dialpad has nothing to dim.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        if self.kind != Kind::Keypad {
            return Ok(());
        }
        self.ensure_ready()?;
        let packet = long_request(
            KEYPAD_BRIGHTNESS,
            SET_BRIGHTNESS,
            &[0x00, percent.clamp(1, 100)],
        );
        match self.request(&packet, REQUEST_TIMEOUT, 1)? {
            Answer::Accepted(_) => Ok(()),
            Answer::Refused(code) => Err(DriverError::Io(format!(
                "the keypad refused the brightness (HID++ error {code:#04x})"
            ))),
            Answer::TimedOut => Err(DriverError::Io(
                "the keypad did not answer the brightness".to_owned(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn driver(model: &str) -> (Logitech, Recorder) {
        let recorder = Recorder::default();
        let deck = Logitech::new(by_id(model).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// The device's answer to a long request: report 0x11, its first bytes echoed.
    fn answer(request: &[u8]) -> Vec<u8> {
        let mut report = request[..4].to_vec();
        report.resize(LONG_LEN, 0);
        report
    }

    fn padded(prefix: &[u8], len: usize) -> Vec<u8> {
        let mut packet = prefix.to_vec();
        packet.resize(len, 0);
        packet
    }

    #[test]
    fn the_keypad_diverts_its_page_buttons_and_queues_input_that_arrives_first() {
        let (mut deck, recorder) = driver("logitech-mx-keypad");
        let left = padded(&[0x11, 0xff, 0x0b, 0x3b, 0x01, 0xa1, 0x03], LONG_LEN);
        let right = padded(&[0x11, 0xff, 0x0b, 0x3b, 0x01, 0xa2, 0x03], LONG_LEN);
        // Keys 1 and 5 go down while the first divert waits for its answer.
        recorder.queue_read(vec![0x13, 0xff, 0x02, 0x00, 0x00, 0x01, 1, 5, 0]);
        recorder.queue_read(answer(&left));
        recorder.queue_read(answer(&right));
        // Then page right goes down, key 1 comes up, and an image answer is ignored.
        recorder.queue_read(vec![0x11, 0xff, 0x0b, 0x00, 0x01, 0xa2, 0x00, 0x00]);
        recorder.queue_read(vec![0x13, 0xff, 0x02, 0x00, 0x00, 0x01, 5, 0]);
        recorder.queue_read(vec![0x13, 0xff, 0x02, 0x2b, 0xe1, 0x00]);
        recorder.queue_read(vec![0x11, 0xff, 0x0b, 0x00, 0x00, 0x00]);
        recorder.queue_read(vec![0x13]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 0,
                    pressed: true
                },
                Input::Key {
                    index: 4,
                    pressed: true
                },
                Input::Key {
                    index: 10,
                    pressed: true
                },
                Input::Key {
                    index: 0,
                    pressed: false
                },
                Input::Key {
                    index: 10,
                    pressed: false
                },
            ]
        );
        assert_eq!(recorder.writes(), vec![left, right], "answered: sent once");
    }

    #[test]
    fn unanswered_set_up_writes_are_tried_three_times() {
        let (mut deck, recorder) = driver("logitech-mx-dialpad");
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        // The first is tried three times; once it goes unanswered, the rest once.
        assert_eq!(writes.len(), INIT_ATTEMPTS + 5);
        assert_eq!(writes[0], writes[INIT_ATTEMPTS - 1]);
        let sent: Vec<&[u8]> = writes[INIT_ATTEMPTS - 1..]
            .iter()
            .map(|packet| &packet[..7])
            .collect();
        assert_eq!(
            sent,
            vec![
                &[0x11, 0xff, 0x0a, 0x3b, 0x00, 0x53, 0x03][..],
                &[0x11, 0xff, 0x0a, 0x3b, 0x00, 0x56, 0x03],
                &[0x11, 0xff, 0x0a, 0x3b, 0x00, 0x59, 0x03],
                &[0x11, 0xff, 0x0a, 0x3b, 0x00, 0x5a, 0x03],
                &[0x11, 0xff, 0x0d, 0x3b, 0x00, 0x01, 0x00],
                &[0x11, 0xff, 0x0d, 0x3b, 0x01, 0x01, 0x00],
            ]
        );
        assert!(writes.iter().all(|packet| packet.len() == LONG_LEN));
        // The dialpad has no panel to dim.
        deck.set_brightness(50).unwrap();
        assert_eq!(recorder.writes().len(), INIT_ATTEMPTS + 5);
    }

    #[test]
    fn dialpad_buttons_and_dials() {
        let (mut deck, recorder) = driver("logitech-mx-dialpad");
        for request in deck.init_requests() {
            recorder.queue_read(answer(&request));
        }
        // Top middle and bottom right down.
        recorder.queue_read(vec![0x11, 0xff, 0x0a, 0x00, 0x00, 0x56, 0x00, 0x5a, 0, 0]);
        // The roller two detents (reported inverted), the big dial three back.
        recorder.queue_read(vec![0x11, 0xff, 0x0d, 0x00, 0x00, 0x02]);
        recorder.queue_read(vec![0x11, 0xff, 0x0d, 0x00, 0x01, 0xfd]);
        recorder.queue_read(vec![0x11, 0xff, 0x0d, 0x00, 0x01, 0x00]);
        recorder.queue_read(vec![0x11, 0xff, 0x0d, 0x00, 0x07, 0x01]);
        // An answer is not input, nor is a short report.
        recorder.queue_read(vec![0x11, 0xff, 0x0d, 0x3b, 0x00, 0x05]);
        recorder.queue_read(vec![0x11, 0xff, 0x0d, 0x00]);
        recorder.queue_read(vec![0x11, 0xff, 0x0a, 0x00, 0x00, 0x00]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 1,
                    pressed: true
                },
                Input::Key {
                    index: 3,
                    pressed: true
                },
                Input::DialTurn {
                    index: 0,
                    delta: -2
                },
                Input::DialTurn {
                    index: 1,
                    delta: -3
                },
                Input::Key {
                    index: 1,
                    pressed: false
                },
                Input::Key {
                    index: 3,
                    pressed: false
                },
            ]
        );
        assert_eq!(
            recorder.writes().len(),
            6,
            "each set-up write answered once"
        );
    }

    #[test]
    fn image_packets_split_and_flag_their_parts() {
        let single = image_packets(23, 6, 118, 118, &[0xaa; 10]);
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].len(), IMAGE_PACKET_LEN);
        assert_eq!(
            &single[0][..20],
            &[
                0x14, 0xff, 0x02, 0x2b, 0xe1, 0x01, 0x00, 0x01, 0x00, 0x00, 0x17, 0x00, 0x06, 0x00,
                0x76, 0x00, 0x76, 0x00, 0x00, 0x0a
            ]
        );
        assert_eq!(&single[0][20..30], &[0xaa; 10]);
        assert_eq!(single[0][30], 0, "zero padded");

        let jpeg: Vec<u8> = (0..4075 + 4090 + 1).map(|i| (i % 251) as u8).collect();
        let parts = image_packets(0, 0, 480, 480, &jpeg);
        assert_eq!(parts.len(), 3);
        assert_eq!(
            parts.iter().map(|packet| packet[4]).collect::<Vec<_>>(),
            vec![0xa1, 0x22, 0x63]
        );
        assert_eq!(&parts[0][18..20], &[0x1f, 0xe6], "total length 8166");
        assert_eq!(&parts[1][..4], &[0x14, 0xff, 0x02, 0x2b]);
        assert_eq!(parts[1][5], jpeg[4075]);
        assert_eq!(parts[2][5], jpeg[4075 + 4090]);
        assert!(parts.iter().all(|packet| packet.len() == IMAGE_PACKET_LEN));
    }

    #[test]
    fn a_face_is_a_jpeg_at_its_key_resent_when_the_keypad_is_busy() {
        let (mut deck, recorder) = driver("logitech-mx-keypad");
        for request in deck.init_requests() {
            recorder.queue_read(answer(&request));
        }
        let face = Face {
            label: String::new(),
            color: [200, 30, 30],
            image: None,
        };
        deck.set_face(4, Some(&face)).unwrap();
        // The page buttons have no screen.
        deck.set_face(9, Some(&face)).unwrap();
        recorder.queue_read(vec![0x13, 0xff, 0x02, 0x2b, 0xe1, IMAGE_BUSY]);
        recorder.queue_read(vec![0x13, 0xff, 0x02, 0x2b, 0xe1, IMAGE_ACCEPTED]);
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 2 + 2, "two diverts, then the image twice");
        assert_eq!(writes[2], writes[3]);
        let packet = &writes[2];
        // Key 4 is the middle of the grid: (23 + 158, 6 + 158).
        assert_eq!(
            &packet[..17],
            &[
                0x14, 0xff, 0x02, 0x2b, 0xe1, 0x01, 0x00, 0x01, 0x00, 0x00, 0xb5, 0x00, 0xa4, 0x00,
                0x76, 0x00, 0x76
            ]
        );
        let len = usize::from(u16::from_be_bytes([packet[18], packet[19]]));
        let decoded = image::load_from_memory(&packet[20..20 + len]).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (118, 118));

        recorder.queue_read(padded(&[0x11, 0xff, 0x0f, 0x2b], LONG_LEN));
        deck.set_brightness(0).unwrap();
        recorder.queue_read(padded(&[0x11, 0xff, 0x0f, 0x2b], LONG_LEN));
        deck.set_brightness(130).unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[4],
            padded(&[0x11, 0xff, 0x0f, 0x2b, 0x00, 0x01], LONG_LEN),
            "0 would reset the keypad"
        );
        assert_eq!(
            writes[5],
            padded(&[0x11, 0xff, 0x0f, 0x2b, 0x00, 100], LONG_LEN)
        );
    }

    #[test]
    fn a_rejected_or_unanswered_image_is_an_error() {
        let (mut deck, recorder) = driver("logitech-mx-keypad");
        for request in deck.init_requests() {
            recorder.queue_read(answer(&request));
        }
        let paused = |deck: &mut Logitech| {
            deck.retry_at = Some(Instant::now() - Duration::from_secs(1));
        };
        deck.set_face(0, None).unwrap();
        recorder.queue_read(vec![0x13, 0xff, 0x02, 0x2b, 0xe1, 0x05]);
        assert!(deck.flush().is_err());
        paused(&mut deck);
        recorder.queue_read(vec![0x13, 0xff, 0xff, 0x02, 0x2b, 0x09]);
        assert!(deck.flush().is_err());
        paused(&mut deck);
        assert!(deck.flush().is_err(), "no answer at all");
        assert_eq!(deck.staged.keys().copied().collect::<Vec<_>>(), vec![0]);
    }

    #[test]
    fn faces_a_failed_flush_left_unsent_are_kept_and_retried_after_a_pause() {
        let (mut deck, recorder) = driver("logitech-mx-keypad");
        for request in deck.init_requests() {
            recorder.queue_read(answer(&request));
        }
        for key in 0..3 {
            deck.set_face(key, None).unwrap();
        }
        // Key 1 is taken; key 2 times out.
        recorder.queue_read(vec![0x13, 0xff, 0x02, 0x2b, 0xe1, IMAGE_ACCEPTED]);
        assert!(deck.flush().is_err());
        assert_eq!(deck.staged.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
        let sent = recorder.writes().len();
        // During the pause a repaint only stages and the heartbeat waits.
        deck.set_face(1, None).unwrap();
        deck.flush().unwrap();
        deck.heartbeat().unwrap();
        assert_eq!(recorder.writes().len(), sent);
        // After it the heartbeat sends keys 2 and 3.
        deck.retry_at = Some(Instant::now() - Duration::from_secs(1));
        for _ in 0..2 {
            recorder.queue_read(vec![0x13, 0xff, 0x02, 0x2b, 0xe1, IMAGE_ACCEPTED]);
        }
        deck.heartbeat().unwrap();
        assert!(deck.staged.is_empty());
        assert_eq!(deck.retry_at, None);
        assert_eq!(recorder.writes().len(), sent + 2);
    }
}
