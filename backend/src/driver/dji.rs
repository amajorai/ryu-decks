//! DJI drone remote controllers (the RC-N1) over USB serial: two sticks and the
//! gimbal wheel, with no screens, lights or brightness.
//!
//! The remote enumerates as two USB CDC-ACM serial ports with one USB serial
//! number. Only one of them speaks DJI's DUML framing, and which one is not
//! fixed, so the device loop claims a port only once it answers [`probe`]; the
//! silent sibling is closed again and never written after that.
//!
//! A DUML frame (multi-byte fields little-endian) is `55`, a u16 holding the
//! whole frame's length in its low 10 bits and the version (1) above them, a
//! CRC-8 of those three bytes, the source and target addresses, a u16 sequence
//! number, the command type, set and id, the payload, and a CRC-16 of everything
//! before it. Both CRCs are reflected: CRC-8 polynomial 0x8C seeded 0x77,
//! CRC-16 polynomial 0x8408 seeded 0x3692. The host (address 0x0A) asks the
//! remote (0x06) with requests (type 0x40) in the remote's command set (0x06).
//!
//! The remote says nothing until asked. Ryu turns its simulator mode on (id
//! 0x24, payload `01`), without which it ignores channel requests, then asks
//! for the channels (id 0x01, no payload) every 20 ms. With no answer for a
//! second (the remote drops simulator mode when it is switched off and on while
//! plugged in) the simulator request goes out again. Replies come back in the
//! same command set and id among unsolicited status frames, which are ignored.
//!
//! A channel reply's payload is a status byte, then 3-byte slots with a u16 one
//! byte in: slot `i`'s value is at `2 + 3i`. The value is centred on 1024 with
//! 660 of travel each way, up and right positive. Slots 2 and 3 (the left
//! stick's Y and X) are confirmed on a remote; slots 0 and 1 (the right stick's
//! X and Y) and 4 (the gimbal wheel, which springs back to centre) follow
//! community documentation that has not been checked on hardware, and are kept
//! in [`SLOTS`] so they can be corrected in one place.
//!
//! Ryu has no analog input, so each axis is read two ways, like the Xbox
//! controllers' sticks: as keys, one per direction, held once the axis passes
//! half its travel (after a 15% dead zone) and let go below 35%; and as five
//! dials that keep turning while the axis is off centre, faster the further it
//! is pushed (2 to 15 steps a second). Nothing clicks, so there are no dial
//! presses. The remote's buttons (return to home, shutter, record, Fn, C1, C2
//! and the flight-mode switch) are not in the channel reply and their message
//! is not known, so they are not offered.

use std::time::{Duration, Instant};

use super::xbox::{dead_zone, repeat_period, rotary_level, PRESS_AT, RELEASE_RATIO};
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const MODEL_ID: &str = "dji-rc-n1";

const SOF: u8 = 0x55;
/// The protocol version, in the bits above the 10-bit length.
const VERSION: u16 = 1 << 10;
const LENGTH_MASK: u16 = 0x3ff;
/// The header up to and including the command id.
const HEADER_LEN: usize = 11;
/// The header and the CRC-16: a frame with no payload.
const MIN_FRAME: usize = HEADER_LEN + 2;
const HOST: u8 = 0x0a;
const REMOTE: u8 = 0x06;
const REQUEST: u8 = 0x40;
/// The remote controller's command set.
const SET_RC: u8 = 0x06;
const ID_CHANNELS: u8 = 0x01;
const ID_SIMULATOR: u8 = 0x24;

const CRC8_SEED: u8 = 0x77;
const CRC8_POLY: u8 = 0x8c;
const CRC16_SEED: u16 = 0x3692;
const CRC16_POLY: u16 = 0x8408;

/// How often the channels are asked for.
const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// With no channel reply for longer than this, simulator mode is turned on again.
const REPLY_TIMEOUT: Duration = Duration::from_millis(1000);
/// How long a port is given to answer the probe, and how often it is asked
/// meanwhile: the remote can take over a second after another program let go
/// of the port abruptly.
const PROBE_WINDOW: Duration = Duration::from_millis(1500);
const PROBE_RESEND: Duration = Duration::from_millis(100);

const READ_BUFFER: usize = 256;
const MAX_READS_PER_POLL: usize = 16;
/// More buffered than any frame can be long means the stream is lost: start over.
const MAX_BUFFER: usize = 4 * LENGTH_MASK as usize;

/// The raw channel value at rest, and its travel each way.
const CENTRE: f32 = 1024.0;
const TRAVEL: f32 = 660.0;

/// The axes, which are also the dials, in dial order.
const AXES: usize = 5;
const LEFT_X: usize = 0;
const LEFT_Y: usize = 1;
const RIGHT_X: usize = 2;
const RIGHT_Y: usize = 3;
const WHEEL: usize = 4;

/// The reply slot each axis is read from, in axis order. Only the left stick's
/// (3 and 2) are confirmed on hardware; the right stick's (0 and 1) and the
/// wheel's (4) come from community documentation.
pub const SLOTS: [usize; AXES] = [3, 2, 0, 1, 4];
/// The slots a reply must carry.
const SLOTS_NEEDED: usize = 5;

/// The key names, in key order.
pub const KEY_NAMES: [&str; 10] = [
    "LS up",
    "LS down",
    "LS left",
    "LS right",
    "RS up",
    "RS down",
    "RS left",
    "RS right",
    "Wheel up",
    "Wheel down",
];

/// Each key's axis and the direction along it that presses the key.
const KEY_DIRECTIONS: [(usize, f32); 10] = [
    (LEFT_Y, 1.0),
    (LEFT_Y, -1.0),
    (LEFT_X, -1.0),
    (LEFT_X, 1.0),
    (RIGHT_Y, 1.0),
    (RIGHT_Y, -1.0),
    (RIGHT_X, -1.0),
    (RIGHT_X, 1.0),
    (WHEEL, 1.0),
    (WHEEL, -1.0),
];

/// DUML's header CRC: reflected, polynomial 0x8C, seeded 0x77.
#[must_use]
pub fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = CRC8_SEED;
    for byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ CRC8_POLY
            };
        }
    }
    crc
}

/// DUML's frame CRC: reflected, polynomial 0x8408, seeded 0x3692.
#[must_use]
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = CRC16_SEED;
    for byte in bytes {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ CRC16_POLY
            };
        }
    }
    crc
}

/// A request from the host to the remote in the remote's command set.
#[must_use]
pub fn request(seq: u16, id: u8, payload: &[u8]) -> Vec<u8> {
    let len = MIN_FRAME + payload.len();
    // Ryu's requests are a few bytes; the 10-bit field cannot be overrun.
    let field = (u16::try_from(len).unwrap_or(LENGTH_MASK) & LENGTH_MASK) | VERSION;
    let mut frame = Vec::with_capacity(len);
    frame.push(SOF);
    frame.extend_from_slice(&field.to_le_bytes());
    frame.push(crc8(&frame));
    frame.extend_from_slice(&[HOST, REMOTE]);
    frame.extend_from_slice(&seq.to_le_bytes());
    frame.extend_from_slice(&[REQUEST, SET_RC, id]);
    frame.extend_from_slice(payload);
    let crc = crc16(&frame);
    frame.extend_from_slice(&crc.to_le_bytes());
    frame
}

/// Turns simulator mode on, without which the remote ignores channel requests.
#[must_use]
pub fn enable_simulator(seq: u16) -> Vec<u8> {
    request(seq, ID_SIMULATOR, &[0x01])
}

/// Asks for the stick and wheel positions.
#[must_use]
pub fn get_channels(seq: u16) -> Vec<u8> {
    request(seq, ID_CHANNELS, &[])
}

/// One checked frame's command and payload.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    set: u8,
    id: u8,
    payload: Vec<u8>,
}

impl Frame {
    fn is_channels(&self) -> bool {
        self.set == SET_RC && self.id == ID_CHANNELS
    }
}

/// Splits a serial byte stream, which splits and merges frames anywhere, into
/// checked frames.
#[derive(Debug, Default)]
struct Parser {
    buffer: Vec<u8>,
}

impl Parser {
    fn push(&mut self, bytes: &[u8]) {
        if self.buffer.len() + bytes.len() > MAX_BUFFER {
            self.buffer.clear();
        }
        self.buffer.extend_from_slice(bytes);
    }

    /// The next whole frame with good CRCs, if one has arrived. A bad header
    /// CRC or an impossible length skips one byte and looks for the next
    /// start; a bad frame CRC drops the whole frame.
    fn next_frame(&mut self) -> Option<Frame> {
        loop {
            let Some(start) = self.buffer.iter().position(|byte| *byte == SOF) else {
                self.buffer.clear();
                return None;
            };
            self.buffer.drain(..start);
            let (Some(head), Some(&check)) = (self.buffer.get(..3), self.buffer.get(3)) else {
                return None;
            };
            let len = usize::from(u16::from_le_bytes([head[1], head[2]]) & LENGTH_MASK);
            if crc8(head) != check || len < MIN_FRAME {
                self.buffer.drain(..1);
                continue;
            }
            if self.buffer.len() < len {
                return None;
            }
            let frame: Vec<u8> = self.buffer.drain(..len).collect();
            let (Some(body), Some(tail)) = (frame.get(..len - 2), frame.get(len - 2..)) else {
                continue;
            };
            if crc16(body) != u16::from_le_bytes([tail[0], tail[1]]) {
                continue;
            }
            let (Some(&set), Some(&id)) = (body.get(9), body.get(10)) else {
                continue;
            };
            return Some(Frame {
                set,
                id,
                payload: body.get(HEADER_LEN..).unwrap_or_default().to_vec(),
            });
        }
    }
}

/// Each axis's position, -1 to 1 (up and right positive), from a channel
/// reply's payload; `None` when it holds fewer than five slots.
fn axes(payload: &[u8]) -> Option<[f32; AXES]> {
    let mut slots = [0u16; SLOTS_NEEDED];
    for (index, slot) in slots.iter_mut().enumerate() {
        let at = 2 + 3 * index;
        let pair = payload.get(at..at + 2)?;
        *slot = u16::from_le_bytes([pair[0], pair[1]]);
    }
    let mut axes = [0.0; AXES];
    for (axis, slot) in axes.iter_mut().zip(SLOTS) {
        let raw = f32::from(*slots.get(slot)?);
        *axis = ((raw - CENTRE) / TRAVEL).clamp(-1.0, 1.0);
    }
    Some(axes)
}

/// Sends simulator mode and a channel request every 100 ms for up to 1.5 s and
/// reports whether a channel reply came back: whether this port is the
/// remote's DUML port rather than its silent sibling.
pub fn probe(transport: &mut dyn Transport) -> DriverResult<bool> {
    probe_within(transport, PROBE_WINDOW)
}

fn probe_within(transport: &mut dyn Transport, window: Duration) -> DriverResult<bool> {
    let deadline = Instant::now() + window;
    let mut parser = Parser::default();
    let mut seq: u16 = 0;
    let mut next_send = Instant::now();
    let mut buf = [0u8; READ_BUFFER];
    loop {
        let now = Instant::now();
        if now >= next_send {
            transport.write(&enable_simulator(seq))?;
            transport.write(&get_channels(seq.wrapping_add(1)))?;
            seq = seq.wrapping_add(2);
            next_send = now + PROBE_RESEND;
        }
        let wait = deadline
            .saturating_duration_since(now)
            .min(next_send.saturating_duration_since(now));
        let wait = i32::try_from(wait.as_millis()).unwrap_or(i32::MAX).max(1);
        let read = transport.read_timeout(&mut buf, wait)?;
        parser.push(buf.get(..read).unwrap_or_default());
        while let Some(frame) = parser.next_frame() {
            if frame.is_channels() {
                return Ok(true);
            }
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
    }
}

/// One axis's repeating dial turn.
#[derive(Debug, Clone, Copy, Default)]
struct Rotary {
    /// -8 to 8: how far the axis is pushed, in steps; 0 is centred.
    level: i8,
    next: Option<Instant>,
}

pub struct Remote {
    transport: Box<dyn Transport>,
    parser: Parser,
    seq: u16,
    started: bool,
    last_request: Option<Instant>,
    last_reply: Option<Instant>,
    keys: [bool; KEY_NAMES.len()],
    rotaries: [Rotary; AXES],
}

impl Remote {
    #[must_use]
    pub fn new(_model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            parser: Parser::default(),
            seq: 0,
            started: false,
            last_request: None,
            last_reply: None,
            keys: [false; KEY_NAMES.len()],
            rotaries: [Rotary::default(); AXES],
        }
    }

    fn send(&mut self, frame: fn(u16) -> Vec<u8>) -> DriverResult<()> {
        let bytes = frame(self.seq);
        self.seq = self.seq.wrapping_add(1);
        self.transport.write(&bytes)
    }

    /// Every 20 ms: simulator mode first when the remote has gone quiet for a
    /// second (or on the first tick), then a channel request.
    fn tick(&mut self, now: Instant) -> DriverResult<()> {
        if self
            .last_request
            .is_some_and(|sent| now.saturating_duration_since(sent) < POLL_INTERVAL)
        {
            return Ok(());
        }
        let quiet = self
            .last_reply
            .is_none_or(|reply| now.saturating_duration_since(reply) > REPLY_TIMEOUT);
        if !self.started || quiet {
            self.started = true;
            self.send(enable_simulator)?;
            self.last_reply = Some(now);
        }
        self.send(get_channels)?;
        self.last_request = Some(now);
        Ok(())
    }

    /// The key and dial events new axis positions raise at `now`.
    fn apply(&mut self, positions: [f32; AXES], now: Instant, events: &mut Vec<Input>) {
        let positions = positions.map(dead_zone);
        for (index, (axis, toward)) in KEY_DIRECTIONS.into_iter().enumerate() {
            let travel = (positions[axis] * toward).max(0.0);
            let held = self.keys[index];
            let pressed = if held {
                travel > PRESS_AT * RELEASE_RATIO
            } else {
                travel >= PRESS_AT
            };
            if pressed != held {
                self.keys[index] = pressed;
                events.push(Input::Key {
                    index: index as u8,
                    pressed,
                });
            }
        }
        for (axis, position) in positions.into_iter().enumerate() {
            let level = rotary_level(position);
            let rotary = &mut self.rotaries[axis];
            let previous = rotary.level;
            if level == previous {
                continue;
            }
            rotary.level = level;
            if level == 0 {
                rotary.next = None;
                continue;
            }
            // A change of level restarts the repeat at the new rate.
            rotary.next = Some(now + repeat_period(level));
            if previous == 0 || previous.signum() != level.signum() {
                events.push(turn(axis, level));
            }
        }
    }

    /// The repeat turns due by `now`, at most one per axis.
    fn due_turns(&mut self, now: Instant, events: &mut Vec<Input>) {
        for (axis, rotary) in self.rotaries.iter_mut().enumerate() {
            let Some(due) = rotary.next.filter(|due| *due <= now) else {
                continue;
            };
            let period = repeat_period(rotary.level);
            let next = due + period;
            // After a stall the repeat picks up from now rather than catching up.
            rotary.next = Some(if next <= now { now + period } else { next });
            events.push(turn(axis, rotary.level));
        }
    }

    fn poll_at(&mut self, timeout_ms: i32, now: impl Fn() -> Instant) -> DriverResult<Vec<Input>> {
        self.tick(now())?;
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        // Never wait past the next request.
        let until_request = self.last_request.map_or(Duration::ZERO, |sent| {
            (sent + POLL_INTERVAL).saturating_duration_since(now())
        });
        let first_wait = i32::try_from(until_request.as_millis())
            .unwrap_or(i32::MAX)
            .min(timeout_ms.max(0));
        for attempt in 0..MAX_READS_PER_POLL {
            let wait = if attempt == 0 { first_wait } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.parser.push(buf.get(..read).unwrap_or_default());
            while let Some(frame) = self.parser.next_frame() {
                if !frame.is_channels() {
                    continue;
                }
                let Some(positions) = axes(&frame.payload) else {
                    continue;
                };
                let at = now();
                self.last_reply = Some(at);
                self.apply(positions, at, &mut events);
            }
        }
        self.due_turns(now(), &mut events);
        Ok(events)
    }
}

fn turn(axis: usize, level: i8) -> Input {
    Input::DialTurn {
        index: axis as u8,
        delta: level.signum(),
    }
}

impl Driver for Remote {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.poll_at(timeout_ms, Instant::now)
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
    use std::cell::Cell;

    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn hex(text: &str) -> Vec<u8> {
        text.split_whitespace()
            .map(|pair| u8::from_str_radix(pair, 16).unwrap())
            .collect()
    }

    /// A frame from the remote with any set, id and payload.
    fn reply(set: u8, id: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = request(0x1234, id, payload);
        frame[4] = REMOTE;
        frame[5] = HOST;
        frame[8] = 0x80;
        frame[9] = set;
        let body = frame.len() - 2;
        let crc = crc16(&frame[..body]);
        frame[body..].copy_from_slice(&crc.to_le_bytes());
        frame
    }

    /// A channel reply with these raw slot values (slot 0 first).
    fn channels(slots: [u16; 5]) -> Vec<u8> {
        let mut payload = vec![0x00];
        for value in slots {
            payload.push(0x00);
            payload.extend_from_slice(&value.to_le_bytes());
        }
        reply(SET_RC, ID_CHANNELS, &payload)
    }

    const CENTRED: [u16; 5] = [1024; 5];

    fn driver() -> (Remote, Recorder) {
        let recorder = Recorder::default();
        let remote = Remote::new(by_id(MODEL_ID).unwrap(), Box::new(recorder.clone()));
        (remote, recorder)
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    #[test]
    fn requests_match_the_documented_bytes() {
        assert_eq!(
            enable_simulator(0),
            hex("55 0e 04 66 0a 06 00 00 40 06 24 01 2e ba")
        );
        assert_eq!(
            get_channels(1),
            hex("55 0d 04 33 0a 06 01 00 40 06 01 24 7d")
        );
        assert_eq!(get_channels(0xffff)[6..8], [0xff, 0xff]);
    }

    #[test]
    fn the_parser_resyncs_on_noise_and_drops_bad_frames() {
        let good = channels([1, 2, 3, 4, 5]);
        let mut corrupt = channels(CENTRED);
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0xff;
        // A start byte with a bad header CRC, a short length, a status push,
        // a frame with a bad CRC-16, then the good frame split in three.
        let mut stream = vec![0x00, 0x55, 0x01, 0x02, 0x03];
        stream.extend_from_slice(&[0x55, 0x05, 0x04, crc8(&[0x55, 0x05, 0x04])]);
        stream.extend(reply(0x00, 0x0e, &[1, 2, 3]));
        stream.extend(corrupt);
        let mut parser = Parser::default();
        parser.push(&stream);
        parser.push(&good[..3]);
        let status = parser.next_frame().unwrap();
        assert_eq!((status.set, status.id), (0x00, 0x0e));
        assert!(parser.next_frame().is_none());
        parser.push(&good[3..9]);
        assert!(parser.next_frame().is_none());
        parser.push(&good[9..]);
        let frame = parser.next_frame().unwrap();
        assert!(frame.is_channels());
        assert_eq!(frame.payload.len(), 16);
        assert!(parser.next_frame().is_none());
        assert!(parser.buffer.is_empty());
    }

    #[test]
    fn the_slots_are_read_into_axes() {
        let payload = &channels([1684, 364, 1354, 694, 1024])[HEADER_LEN..];
        let payload = &payload[..payload.len() - 2];
        // Right X full right, right Y full down, left Y half up, left X half
        // left, the wheel centred.
        assert_eq!(axes(payload), Some([-0.5, 0.5, 1.0, -1.0, 0.0]));
        // Out of range clamps; fewer than five slots is no reading.
        let payload = &channels([0, 2047, 1024, 1024, 1024])[HEADER_LEN..];
        assert_eq!(axes(payload).unwrap()[2..4], [-1.0, 1.0]);
        assert_eq!(axes(&[0u8; 15]), None);
        assert_eq!(SLOTS, [3, 2, 0, 1, 4], "only 2 and 3 are confirmed");
    }

    #[test]
    fn start_enables_the_simulator_then_polls_every_20_ms() {
        let (mut remote, recorder) = driver();
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        assert!(remote.poll_at(5, || start).unwrap().is_empty());
        assert_eq!(
            recorder.writes(),
            vec![enable_simulator(0), get_channels(1)]
        );
        // Not yet due.
        remote.poll_at(5, || at(10)).unwrap();
        assert_eq!(recorder.writes().len(), 2);
        recorder.queue_read(channels(CENTRED));
        remote.poll_at(5, || at(20)).unwrap();
        assert_eq!(recorder.writes()[2], get_channels(2));
        // Replies keep the simulator on: no re-enable within a second of one.
        for ms in (40..=1000).step_by(20) {
            remote.poll_at(5, || at(ms)).unwrap();
        }
        assert!(recorder
            .writes()
            .iter()
            .skip(1)
            .all(|frame| frame[10] == ID_CHANNELS));
        // Silent for over a second since the last reply: enable, then poll.
        recorder.0.lock().unwrap().writes.clear();
        remote.poll_at(5, || at(1040)).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 2);
        assert_eq!((writes[0][10], writes[1][10]), (ID_SIMULATOR, ID_CHANNELS));
        assert_eq!(
            u16::from_le_bytes([writes[1][6], writes[1][7]]),
            u16::from_le_bytes([writes[0][6], writes[0][7]]) + 1
        );
        // The timer restarts with the enable.
        recorder.0.lock().unwrap().writes.clear();
        remote.poll_at(5, || at(1060)).unwrap();
        assert_eq!(recorder.writes().len(), 1);
    }

    #[test]
    fn the_sequence_number_wraps() {
        let (mut remote, recorder) = driver();
        remote.seq = 0xffff;
        remote.poll_at(0, Instant::now).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[0], enable_simulator(0xffff));
        assert_eq!(writes[1], get_channels(0));
    }

    #[test]
    fn stick_directions_are_keys_with_hysteresis() {
        let (mut remote, recorder) = driver();
        let start = Instant::now();
        let clock = Cell::new(start);
        let mut events = Vec::new();
        let mut feed = |slots: [u16; 5], ms: u64| {
            clock.set(start + Duration::from_millis(ms));
            recorder.queue_read(channels(slots));
            remote
                .poll_at(0, || clock.get())
                .unwrap()
                .into_iter()
                .filter(|event| matches!(event, Input::Key { .. }))
                .collect::<Vec<_>>()
        };
        // Every direction at full travel: left stick up and right, right stick
        // down and left, wheel up.
        events.extend(feed([364, 364, 1684, 1684, 1684], 0));
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(3, true),
                key(5, true),
                key(6, true),
                key(8, true)
            ]
        );
        // The other way round.
        assert_eq!(
            feed([1684, 1684, 364, 364, 364], 20),
            vec![
                key(0, false),
                key(1, true),
                key(2, true),
                key(3, false),
                key(4, true),
                key(5, false),
                key(6, false),
                key(7, true),
                key(8, false),
                key(9, true),
            ]
        );
        assert_eq!(feed(CENTRED, 40).len(), 5);
        // Half way past the dead zone presses; 40% of it (above 35%) still holds.
        let half = 1024 + (660.0 * (0.15 + 0.85 * 0.5)) as u16 + 1;
        let forty = 1024 + (660.0 * (0.15 + 0.85 * 0.4)) as u16;
        let thirty = 1024 + (660.0 * (0.15 + 0.85 * 0.3)) as u16;
        assert_eq!(feed([1024, 1024, half, 1024, 1024], 60), vec![key(0, true)]);
        assert!(feed([1024, 1024, forty, 1024, 1024], 80).is_empty());
        assert_eq!(
            feed([1024, 1024, thirty, 1024, 1024], 100),
            vec![key(0, false)]
        );
        assert!(feed([1024, 1024, forty, 1024, 1024], 120).is_empty());
    }

    #[test]
    fn axes_turn_dials_at_a_rate_set_by_how_far_they_are_pushed() {
        let (mut remote, recorder) = driver();
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        // Left stick full right, wheel full down.
        recorder.queue_read(channels([1024, 1024, 1024, 1684, 364]));
        let turns: Vec<Input> = remote
            .poll_at(0, || start)
            .unwrap()
            .into_iter()
            .filter(|event| matches!(event, Input::DialTurn { .. }))
            .collect();
        assert_eq!(
            turns,
            vec![
                Input::DialTurn { index: 0, delta: 1 },
                Input::DialTurn {
                    index: 4,
                    delta: -1
                },
            ]
        );
        // Full travel repeats 15 times a second.
        assert!(remote.poll_at(0, || at(50)).unwrap().is_empty());
        assert_eq!(
            remote.poll_at(0, || at(67)).unwrap(),
            vec![
                Input::DialTurn { index: 0, delta: 1 },
                Input::DialTurn {
                    index: 4,
                    delta: -1
                },
            ]
        );
        // Easing off (same direction) restarts the repeat without a turn;
        // reversing turns at once.
        recorder.queue_read(channels([1024, 1024, 1024, 1300, 1684]));
        assert_eq!(
            remote.poll_at(0, || at(70)).unwrap(),
            vec![
                Input::Key {
                    index: 3,
                    pressed: false
                },
                Input::Key {
                    index: 8,
                    pressed: true
                },
                Input::Key {
                    index: 9,
                    pressed: false
                },
                Input::DialTurn { index: 4, delta: 1 },
            ]
        );
        // Back to centre: the repeats stop.
        recorder.queue_read(channels(CENTRED));
        assert_eq!(
            remote.poll_at(0, || at(90)).unwrap(),
            vec![Input::Key {
                index: 8,
                pressed: false
            }]
        );
        assert!(remote.poll_at(0, || at(3000)).unwrap().is_empty());
        assert!(recorder
            .writes()
            .iter()
            .all(|frame| frame[10] == ID_CHANNELS || frame[10] == ID_SIMULATOR));
    }

    #[test]
    fn other_frames_and_short_replies_change_nothing() {
        let (mut remote, recorder) = driver();
        let start = Instant::now();
        remote.poll_at(0, || start).unwrap();
        // A status push, a reply with four slots, garbage.
        recorder.queue_read(reply(0x00, 0x0e, &[0xff; 20]));
        recorder.queue_read(reply(SET_RC, ID_CHANNELS, &[0x00; 13]));
        recorder.queue_read(vec![0x55, 0xff, 0xff, 0x00, 0x55]);
        assert!(remote.poll_at(0, || start).unwrap().is_empty());
        // None of them counted as a reply: the simulator is enabled again.
        recorder.0.lock().unwrap().writes.clear();
        remote
            .poll_at(0, || start + Duration::from_millis(1001))
            .unwrap();
        assert_eq!(recorder.writes()[0][10], ID_SIMULATOR);
    }

    #[test]
    fn nothing_is_drawn_or_lit() {
        let (mut remote, recorder) = driver();
        remote.set_brightness(50).unwrap();
        remote.set_face(0, None).unwrap();
        remote.flush().unwrap();
        assert!(recorder.writes().is_empty());
        let model = by_id(MODEL_ID).unwrap();
        assert!(!model.protocol.has_brightness());
        assert_eq!(usize::from(model.keys), KEY_NAMES.len());
        assert_eq!(usize::from(model.dials), AXES);
    }

    #[test]
    fn the_probe_resends_until_a_channel_reply_comes_back() {
        let mut silent = Recorder::default();
        assert!(!probe_within(&mut silent, Duration::from_millis(250)).unwrap());
        let writes = silent.writes();
        assert!(writes.len() >= 4, "resent every 100 ms");
        assert_eq!(
            writes[..4],
            [
                enable_simulator(0),
                get_channels(1),
                enable_simulator(2),
                get_channels(3)
            ]
        );

        let mut answering = Recorder::default();
        answering.queue_read(reply(0x00, 0x0e, &[1]));
        answering.queue_read(reply(SET_RC, ID_CHANNELS, &[0x00]));
        assert!(probe_within(&mut answering, Duration::from_millis(250)).unwrap());

        // Only the channel reply counts.
        let mut chatty = Recorder::default();
        chatty.queue_read(reply(0x00, 0x0e, &[1]));
        assert!(!probe_within(&mut chatty, Duration::ZERO).unwrap());
    }
}
