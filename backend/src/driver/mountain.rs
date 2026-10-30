//! Mountain DisplayPad: twelve keys, each with its own 102 x 102 screen.
//!
//! The deck is one USB device with two HID interfaces that are used together.
//! Interface 3 (control) carries every input report and the 64-byte commands;
//! interface 1 (display) only takes key-face pixels and is never read.
//!
//! The host first sends an init command and waits for its acknowledgement
//! (report type `0x11`). A face is then one acknowledged transfer: an image-start
//! command naming the key, the deck's echo (`0x21 00 00`), a 31 KiB block of raw
//! BGR pixels in 1 KiB writes on the display interface, and the deck's completion
//! (`0x21 00 ff`). Only one key may be in flight, so `flush` runs each transfer to
//! the end and queues key reports that arrive meanwhile for the next poll.
//!
//! A key report (type `0x01`) is the full set of held keys, one bit each across
//! bytes 42 and 47. There is no known brightness command.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use super::{face, Driver, DriverError, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const KEY_REPORT: u8 = 0x01;
const INIT_ACK: u8 = 0x11;
const IMAGE_STATUS: u8 = 0x21;
const IMAGE_STARTED: u8 = 0x00;
const IMAGE_DONE: u8 = 0xff;

/// Commands are 64-byte reports, written behind report id 0.
const COMMAND_LEN: usize = 65;

/// Where each key's bit sits in a key report, in key order (row-major from the
/// top left).
const KEY_BITS: [(usize, u8); 12] = [
    (42, 0x02),
    (42, 0x04),
    (42, 0x08),
    (42, 0x10),
    (42, 0x20),
    (42, 0x40),
    (42, 0x80),
    (47, 0x01),
    (47, 0x02),
    (47, 0x04),
    (47, 0x08),
    (47, 0x10),
];

const FACE_SIDE: usize = 102;
/// One face: 102 rows of 102 BGR pixels, top to bottom, left to right.
const PIXELS_LEN: usize = FACE_SIDE * FACE_SIDE * 3;
/// Zeros ahead of the pixels.
const BLOCK_HEADER: usize = 306;
const CHUNK_LEN: usize = 1024;
const CHUNKS: usize = 31;
/// The pixel block, zero-padded after the pixels to 31 whole chunks.
const BLOCK_LEN: usize = CHUNK_LEN * CHUNKS;

/// How long the deck has to answer one step of a transfer, or the init command.
const STEP_TIMEOUT: Duration = Duration::from_millis(1000);
/// How long a deck that stopped answering is left alone before its unsent
/// faces are tried again. Each attempt blocks the device loop (and every other
/// deck's input) for seconds, so it is not repeated on every repaint.
const STALL_RETRY: Duration = Duration::from_secs(30);
const READ_BUFFER: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 16;

/// An answer the deck gives on the control interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reply {
    InitAcknowledged,
    ImageStarted,
    ImageDone,
}

pub struct DisplayPad {
    model: &'static DeviceModel,
    control: Box<dyn Transport>,
    /// The display interface; without it, faces cannot be sent.
    display: Option<Box<dyn Transport>>,
    init_sent: bool,
    ready: bool,
    keys: Vec<bool>,
    /// Key changes that arrived while a transfer waited for an answer.
    pending: VecDeque<Input>,
    staged: BTreeMap<u8, Vec<u8>>,
    /// The pixels each key shows now, so unchanged faces are not sent again.
    shown: BTreeMap<u8, Vec<u8>>,
    /// After a stalled flush, when the faces it could not send are tried again.
    retry_at: Option<Instant>,
}

fn init_command() -> Vec<u8> {
    let mut command = vec![0u8; COMMAND_LEN];
    command[1] = INIT_ACK;
    command[2] = 0x80;
    command[5] = 0x01;
    command
}

fn image_start_command(key: u8) -> Vec<u8> {
    let mut command = vec![0u8; COMMAND_LEN];
    command[1] = IMAGE_STATUS;
    command[5] = key;
    command[6] = 0x3d;
    // 101: the last pixel column and row.
    command[9] = 0x65;
    command[10] = 0x65;
    command
}

/// The display-interface writes for one face: the block in 1 KiB pieces, each
/// behind report id 0.
fn pixel_writes(pixels: &[u8]) -> Vec<Vec<u8>> {
    let mut block = vec![0u8; BLOCK_LEN];
    let take = pixels.len().min(PIXELS_LEN);
    block[BLOCK_HEADER..BLOCK_HEADER + take].copy_from_slice(&pixels[..take]);
    block
        .chunks(CHUNK_LEN)
        .map(|chunk| {
            let mut write = Vec::with_capacity(CHUNK_LEN + 1);
            write.push(0x00);
            write.extend_from_slice(chunk);
            write
        })
        .collect()
}

impl DisplayPad {
    #[must_use]
    pub fn new(
        model: &'static DeviceModel,
        control: Box<dyn Transport>,
        display: Option<Box<dyn Transport>>,
    ) -> Self {
        Self {
            model,
            control,
            display,
            init_sent: false,
            ready: false,
            keys: vec![false; usize::from(model.keys)],
            pending: VecDeque::new(),
            staged: BTreeMap::new(),
            shown: BTreeMap::new(),
            retry_at: None,
        }
    }

    fn send_init(&mut self) -> DriverResult<()> {
        self.init_sent = true;
        self.ready = false;
        self.control.write(&init_command())
    }

    /// Reads one control report: key changes go to `pending`, an answer is returned.
    fn handle(&mut self, report: &[u8]) -> Option<Reply> {
        match report.first().copied()? {
            KEY_REPORT => {
                let changes = self.key_changes(report);
                self.pending.extend(changes);
                None
            }
            INIT_ACK => {
                self.ready = true;
                Some(Reply::InitAcknowledged)
            }
            IMAGE_STATUS if report.get(1) == Some(&0x00) => match report.get(2).copied()? {
                IMAGE_STARTED => Some(Reply::ImageStarted),
                IMAGE_DONE => Some(Reply::ImageDone),
                _ => None,
            },
            _ => None,
        }
    }

    /// The keys whose state differs from the last report. A report too short to
    /// hold both key bytes is ignored.
    fn key_changes(&mut self, report: &[u8]) -> Vec<Input> {
        let (Some(&first), Some(&second)) = (report.get(42), report.get(47)) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        for (index, (held, (byte, bit))) in self.keys.iter_mut().zip(KEY_BITS).enumerate() {
            let value = if byte == 42 { first } else { second };
            let pressed = value & bit != 0;
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

    /// Reads the control interface until `wanted` arrives or the step times out.
    fn await_reply(&mut self, wanted: Reply) -> DriverResult<bool> {
        let deadline = Instant::now() + STEP_TIMEOUT;
        let mut buf = [0u8; READ_BUFFER];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(false);
            }
            let wait = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
            let read = self.control.read_timeout(&mut buf, wait)?;
            if read == 0 {
                return Ok(false);
            }
            if self.handle(&buf[..read]) == Some(wanted) {
                return Ok(true);
            }
        }
    }

    /// Sends init (once) and waits for its acknowledgement.
    fn ensure_ready(&mut self) -> DriverResult<bool> {
        if self.ready {
            return Ok(true);
        }
        if !self.init_sent {
            self.send_init()?;
        }
        self.await_reply(Reply::InitAcknowledged)
    }

    /// One face, start to finish. `Ok(false)` when the deck stopped answering.
    fn transfer(&mut self, key: u8, pixels: &[u8]) -> DriverResult<bool> {
        self.control.write(&image_start_command(key))?;
        if !self.await_reply(Reply::ImageStarted)? {
            return Ok(false);
        }
        let Some(display) = self.display.as_mut() else {
            return Err(DriverError::Io(
                "the DisplayPad's display interface is not open".to_owned(),
            ));
        };
        for write in pixel_writes(pixels) {
            display.write(&write)?;
        }
        self.await_reply(Reply::ImageDone)
    }
}

impl Driver for DisplayPad {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.init_sent {
            self.send_init()?;
        }
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 && self.pending.is_empty() {
                timeout_ms
            } else {
                0
            };
            let read = self.control.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.handle(&buf[..read]);
        }
        Ok(self.pending.drain(..).collect())
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let Some(format) = self.model.image else {
            return Ok(());
        };
        if key >= self.model.keys {
            return Ok(());
        }
        // An unbound key is sent black: `encode` paints it so.
        self.staged.insert(key, face::encode(face, &format)?);
        Ok(())
    }

    /// Sends the changed faces. When the deck stops answering, the faces not
    /// yet sent stay staged and the heartbeat tries them again after
    /// [`STALL_RETRY`]; until then a flush only stages.
    fn flush(&mut self) -> DriverResult<()> {
        if self.retry_at.is_some_and(|at| Instant::now() < at) {
            return Ok(());
        }
        let staged = std::mem::take(&mut self.staged);
        let mut changed: BTreeMap<u8, Vec<u8>> = staged
            .into_iter()
            .filter(|(key, pixels)| self.shown.get(key) != Some(pixels))
            .collect();
        if changed.is_empty() {
            return Ok(());
        }
        let result = self.send_faces(&mut changed);
        if result.is_err() {
            for (key, pixels) in changed {
                self.staged.entry(key).or_insert(pixels);
            }
            self.retry_at = Some(Instant::now() + STALL_RETRY);
        } else {
            self.retry_at = None;
        }
        result
    }

    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        // No brightness command is known for the DisplayPad.
        Ok(())
    }

    /// Retries the faces a stalled flush could not send, once the pause is over.
    fn heartbeat(&mut self) -> DriverResult<()> {
        let due = self.retry_at.is_some_and(|at| Instant::now() >= at);
        if !due || self.staged.is_empty() {
            return Ok(());
        }
        self.retry_at = None;
        if let Err(error) = self.flush() {
            tracing::debug!(%error, "ryu-decks: the DisplayPad still does not take its faces");
        }
        Ok(())
    }
}

impl DisplayPad {
    /// Sends `changed` in key order, removing each face once the deck has it;
    /// what is left on an error was not sent.
    fn send_faces(&mut self, changed: &mut BTreeMap<u8, Vec<u8>>) -> DriverResult<()> {
        if !self.ensure_ready()? {
            // Init is sometimes taken without effect; send it once more.
            self.send_init()?;
            if !self.await_reply(Reply::InitAcknowledged)? {
                return Err(DriverError::Io(
                    "the DisplayPad did not answer its init command".to_owned(),
                ));
            }
        }
        while let Some((key, pixels)) = changed.pop_first() {
            match self.send_face(key, &pixels) {
                Ok(()) => {
                    self.shown.insert(key, pixels);
                }
                Err(error) => {
                    changed.insert(key, pixels);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// One face; a transfer that stalls is retried once after a fresh init.
    fn send_face(&mut self, key: u8, pixels: &[u8]) -> DriverResult<()> {
        let mut sent = self.transfer(key, pixels)?;
        if !sent {
            self.send_init()?;
            sent = self.await_reply(Reply::InitAcknowledged)? && self.transfer(key, pixels)?;
        }
        if sent {
            Ok(())
        } else {
            Err(DriverError::Io(format!(
                "the DisplayPad did not take the face for key {}",
                key + 1
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn driver() -> (DisplayPad, Recorder, Recorder) {
        let control = Recorder::default();
        let display = Recorder::default();
        let model = by_id("mountain-displaypad").unwrap();
        (
            DisplayPad::new(
                model,
                Box::new(control.clone()),
                Some(Box::new(display.clone())),
            ),
            control,
            display,
        )
    }

    fn key_report(byte42: u8, byte47: u8) -> Vec<u8> {
        let mut report = vec![0u8; 64];
        report[0] = KEY_REPORT;
        report[42] = byte42;
        report[47] = byte47;
        report
    }

    fn status(code: u8) -> Vec<u8> {
        let mut report = vec![0u8; 64];
        report[0] = IMAGE_STATUS;
        report[2] = code;
        report
    }

    fn ack() -> Vec<u8> {
        let mut report = vec![0u8; 64];
        report[0] = INIT_ACK;
        report
    }

    fn keys(events: Vec<Input>) -> Vec<(u8, bool)> {
        events
            .into_iter()
            .map(|input| match input {
                Input::Key { index, pressed } => (index, pressed),
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[test]
    fn key_reports_are_diffed_bitmaps_across_bytes_42_and_47() {
        let (mut deck, control, _) = driver();
        // Key 1 (bit 1 of byte 42), key 7 (bit 7) and key 12 (bit 4 of byte 47).
        control.queue_read(key_report(0x82, 0x10));
        control.queue_read(key_report(0x80, 0x10));
        control.queue_read(key_report(0x00, 0x00));
        // Unused bits and short reports change nothing.
        control.queue_read(key_report(0x01, 0xe0));
        control.queue_read(vec![KEY_REPORT, 0xff, 0xff]);
        assert_eq!(
            keys(deck.poll(0).unwrap()),
            vec![
                (0, true),
                (6, true),
                (11, true),
                (0, false),
                (6, false),
                (11, false)
            ]
        );
        // The first poll sends init.
        assert_eq!(control.writes(), vec![init_command()]);
        assert_eq!(
            &control.writes()[0][..6],
            &[0x00, 0x11, 0x80, 0x00, 0x00, 0x01]
        );
        // Every key has its own bit.
        let (mut deck, control, _) = driver();
        control.queue_read(key_report(0x7e, 0x00));
        control.queue_read(key_report(0x00, 0x00));
        control.queue_read(key_report(0x00, 0x1f));
        let pressed: Vec<u8> = keys(deck.poll(0).unwrap())
            .into_iter()
            .filter(|(_, pressed)| *pressed)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(pressed, vec![0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 11]);
    }

    #[test]
    fn a_face_is_an_acknowledged_transfer_of_raw_bgr_in_31_chunks() {
        let (mut deck, control, display) = driver();
        let mut picture = image::RgbaImage::from_pixel(102, 102, image::Rgba([0, 0, 0, 255]));
        picture.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        picture.put_pixel(101, 0, image::Rgba([0, 0, 255, 255]));
        picture.put_pixel(0, 1, image::Rgba([0, 255, 0, 255]));
        let face = Face {
            label: String::new(),
            color: [0, 0, 0],
            image: Some(picture),
        };
        deck.set_face(4, Some(&face)).unwrap();
        control.queue_read(ack());
        control.queue_read(status(IMAGE_STARTED));
        // A press during the transfer is kept for the next poll.
        control.queue_read(key_report(0x04, 0x00));
        control.queue_read(status(IMAGE_DONE));
        deck.flush().unwrap();

        let commands = control.writes();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0], init_command());
        let mut start = vec![0u8; 65];
        start[..11].copy_from_slice(&[
            0x00, 0x21, 0x00, 0x00, 0x00, 0x04, 0x3d, 0x00, 0x00, 0x65, 0x65,
        ]);
        assert_eq!(commands[1], start);

        let chunks = display.writes();
        assert_eq!(chunks.len(), 31);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.len() == 1025 && chunk[0] == 0x00));
        let block: Vec<u8> = chunks
            .iter()
            .flat_map(|chunk| chunk[1..].to_vec())
            .collect();
        assert_eq!(block.len(), 31_744);
        assert!(block[..306].iter().all(|byte| *byte == 0));
        let pixels = &block[306..306 + 31_212];
        assert_eq!(&pixels[..3], &[0, 0, 255], "red, in BGR order");
        assert_eq!(&pixels[101 * 3..102 * 3], &[255, 0, 0], "not mirrored");
        assert_eq!(&pixels[306..309], &[0, 255, 0], "row two after 306 bytes");
        assert!(block[306 + 31_212..].iter().all(|byte| *byte == 0));

        assert_eq!(keys(deck.poll(0).unwrap()), vec![(1, true)]);

        // The same face again sends nothing; an unbound key is sent black.
        deck.set_face(4, Some(&face)).unwrap();
        deck.flush().unwrap();
        assert_eq!(control.writes().len(), 2);
        deck.set_face(4, None).unwrap();
        control.queue_read(status(IMAGE_STARTED));
        control.queue_read(status(IMAGE_DONE));
        deck.flush().unwrap();
        assert_eq!(control.writes().len(), 3);
        let chunks = display.writes();
        assert_eq!(chunks.len(), 62);
        assert!(chunks[31..]
            .iter()
            .all(|chunk| chunk.iter().all(|b| *b == 0)));
        deck.set_brightness(50).unwrap();
    }

    #[test]
    fn a_stalled_transfer_is_retried_once_after_a_fresh_init() {
        let (mut deck, control, display) = driver();
        deck.set_face(0, None).unwrap();
        control.queue_read(ack());
        control.queue_read(status(IMAGE_STARTED));
        // An empty read is a timeout: the finish never comes the first time.
        control.queue_read(Vec::new());
        control.queue_read(ack());
        control.queue_read(status(IMAGE_STARTED));
        control.queue_read(status(IMAGE_DONE));
        deck.flush().unwrap();
        let commands = control.writes();
        assert_eq!(
            commands,
            vec![
                init_command(),
                image_start_command(0),
                init_command(),
                image_start_command(0)
            ]
        );
        assert_eq!(display.writes().len(), 62);

        // A deck that stays silent fails the flush instead of hanging.
        let (mut deck, control, display) = driver();
        deck.set_face(0, None).unwrap();
        control.queue_read(ack());
        assert!(deck.flush().is_err());
        assert_eq!(
            control.writes(),
            vec![init_command(), image_start_command(0), init_command()]
        );
        assert!(display.writes().is_empty());
    }

    #[test]
    fn faces_a_stall_left_unsent_are_kept_and_retried_after_a_pause() {
        let (mut deck, control, display) = driver();
        for key in 0..3 {
            deck.set_face(key, None).unwrap();
        }
        // Key 1 goes through; key 2 never starts, not even after a re-init.
        control.queue_read(ack());
        control.queue_read(status(IMAGE_STARTED));
        control.queue_read(status(IMAGE_DONE));
        assert!(deck.flush().is_err());
        assert_eq!(display.writes().len(), 31);
        assert_eq!(deck.staged.keys().copied().collect::<Vec<_>>(), vec![1, 2]);
        // During the pause a repaint only stages: nothing is sent, nothing blocks.
        let sent = control.writes().len();
        deck.set_face(2, None).unwrap();
        deck.flush().unwrap();
        deck.heartbeat().unwrap();
        assert_eq!(control.writes().len(), sent);
        // After it the heartbeat sends keys 2 and 3.
        deck.retry_at = Some(Instant::now() - Duration::from_secs(1));
        control.queue_read(ack());
        for _ in 0..2 {
            control.queue_read(status(IMAGE_STARTED));
            control.queue_read(status(IMAGE_DONE));
        }
        deck.heartbeat().unwrap();
        assert!(deck.staged.is_empty());
        assert_eq!(deck.retry_at, None);
        assert_eq!(display.writes().len(), 93);
    }

    #[test]
    fn malformed_reports_are_ignored() {
        let (mut deck, control, _) = driver();
        for report in [
            vec![IMAGE_STATUS],
            vec![IMAGE_STATUS, 0x01, 0x00],
            vec![0x55; 64],
            vec![],
        ] {
            control.queue_read(report);
        }
        assert!(deck.poll(0).unwrap().is_empty());
        // Without a display interface, a face is an error, not a panic.
        let model = by_id("mountain-displaypad").unwrap();
        let control = Recorder::default();
        let mut deck = DisplayPad::new(model, Box::new(control.clone()), None);
        deck.set_face(13, None).unwrap();
        deck.set_face(0, None).unwrap();
        control.queue_read(ack());
        control.queue_read(status(IMAGE_STARTED));
        assert!(deck.flush().is_err());
    }
}
