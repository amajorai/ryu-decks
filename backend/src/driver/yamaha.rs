//! Yamaha CC121MK2 ("CC1"): twelve keys behind one 480 x 272 colour screen,
//! seventeen plain buttons (fifteen with a light), five rotary controls, a jog
//! wheel and a motorised fader, on a USB serial (CDC-ACM) port.
//!
//! Both directions carry framed messages: `C0`, the escaped body, `C1`. Inside
//! a body `DB DC` stands for `C0`, `DB DD` for `DB` and `DB DE` for `C1` (the
//! end byte is not SLIP's). The unescaped body is a length (u16 LE), that many
//! payload bytes and a checksum that brings the payload's byte sum to zero.
//! A payload is a routing word, a sequence number and an opcode (each u16 LE),
//! then the data. The host routes on 0 (handshake), 1 (commands), 2 (fader
//! touch mode) or 4 (keep-alive) only: anything else silently mutes the
//! device. The device sets bit 15 on the routing it answers with, and on some
//! opcodes.
//!
//! The device says nothing until a host asks for its info (opcode 0 on routing
//! 0). About 300 ms after the answer the host sets the fader's touch mode, the
//! panel mode and the screen's backlight, in that order (the panel mode
//! re-initialises the screen, undoing an earlier backlight); 400 ms later it
//! clears the whole panel, since the gaps between the key apertures are never
//! painted again. Without a keep-alive (opcode 0 on routing 4) at least every
//! second it drops the host, so one goes out every 300 ms from [`Driver::poll`].
//! Ryu treats a handshake with no answer within 500 ms as a refusal, so the
//! deck is tried again only after a pause (reopening quickly can wedge the
//! CDC driver until a replug); the reference driver sends the rest anyway.
//!
//! The screen takes rectangles of RGB565 pixels of at most 40 x 20 (larger ones
//! stall it), each followed by a 4 ms pause, and shows them only on a commit
//! carrying one zero byte. Every paint is acknowledged, so tiles sent with no
//! answer for 2.5 s mean the session stalled: the deck is dropped and reopened.
//!
//! Switches 21 to 32 are the screen keys, numbered down each column; 12 to 17
//! are the encoder pushes; the plain buttons are scattered over the rest.
//! Ryu numbers the buttons after the screen keys in the order of their printed
//! labels (1, 2, then 15 to 29). Dials 0 to 5 are the panel's RC1 to RC6
//! (encoders 2, 3, 4, 5, 1 and 0; encoder 0 is the jog wheel, which sends
//! several ticks at once when spun fast). Ryu has no fader, so the fader is
//! dial 6: touching it presses the dial, and moving it while touched turns it
//! by one step per 16 of its 1024 positions. Its motor parks it at the bottom
//! unless told to hold a position, so on release Ryu tells it to hold where it
//! was left. The lights take one of ten colours each, not RGB.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::{face, Driver, DriverError, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, ImageMode, KeyImage, Mirror, Rotation};

pub const MODEL_ID: &str = "yamaha-cc121mk2";

const START: u8 = 0xc0;
const END: u8 = 0xc1;
const ESCAPE: u8 = 0xdb;
const ESCAPED_START: u8 = 0xdc;
const ESCAPED_ESCAPE: u8 = 0xdd;
const ESCAPED_END: u8 = 0xde;

/// Set on the routing (and some opcodes) of everything the device sends.
const FROM_DEVICE: u16 = 0x8000;

const OP_DEVICE_INFO: u16 = 0x0000;
const OP_SWITCH: u16 = 0x0001;
const OP_ENCODER: u16 = 0x0002;
/// Fader reports in; the motor's target position out.
const OP_FADER: u16 = 0x0004;
const OP_BACKLIGHT: u16 = 0x0082;
const OP_PANEL_MODE: u16 = 0x0104;
const OP_LCD_PAINT: u16 = 0x0182;
const OP_FADER_TOUCH_MODE: u16 = 0x0204;
const OP_LED_BRIGHTNESS: u16 = 0x0281;
const OP_LCD_COMMIT: u16 = 0x0282;
const OP_LED: u16 = 0x0381;

/// The channels a host may send on. Typed, because a frame on any other
/// routing is well formed and silently ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Routing {
    Handshake = 0,
    Command = 1,
    FaderMode = 2,
    KeepAlive = 4,
}

/// The backlight and LED level ControlCenter uses: the highest proven one.
const MAX_LEVEL: u8 = 0xae;
const LED_COUNT: u8 = 15;
/// The colour byte sent with an LED that is turning off.
const OFF_COLOUR: u8 = 0x01;

pub const PANEL_WIDTH: u16 = 480;
pub const PANEL_HEIGHT: u16 = 272;
const TILE_WIDTH: u16 = 40;
const TILE_HEIGHT: u16 = 20;
pub const KEY_SIZE: u16 = 72;
/// The left edge of each column's key aperture.
const KEY_X: [u16; 4] = [15, 141, 267, 393];
/// The top edge of each row's key aperture.
const KEY_Y: [u16; 3] = [3, 98, 194];
const SCREEN_KEYS: u8 = 12;
/// How a screen key's face is encoded: RGB565, low byte first, upright.
pub const KEY_IMAGE: KeyImage = KeyImage {
    width: KEY_SIZE as u32,
    height: KEY_SIZE as u32,
    mode: ImageMode::Rgb565,
    rotation: Rotation::None,
    mirror: Mirror::None,
};

/// The switch of each extra key, in the order of their printed labels.
const EXTRA_SWITCHES: [u8; 17] = [33, 34, 11, 20, 19, 9, 10, 7, 8, 6, 5, 4, 18, 3, 2, 1, 0];
/// Each extra key's LED: the buttons printed 1 and 2 have none.
const EXTRA_LEDS: [Option<u8>; 17] = [
    None,
    None,
    Some(0x03),
    Some(0x06),
    Some(0x05),
    Some(0x02),
    Some(0x01),
    Some(0x00),
    Some(0x0e),
    Some(0x0d),
    Some(0x0c),
    Some(0x0b),
    Some(0x04),
    Some(0x0a),
    Some(0x09),
    Some(0x08),
    Some(0x07),
];
/// The keys with a light: the buttons printed 15 to 29.
pub const LIT_KEYS: [u8; 15] = [14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28];
/// Each key's name: the screen keys, then the buttons by their printed numbers.
pub const KEY_NAMES: [&str; 29] = [
    "Key 1",
    "Key 2",
    "Key 3",
    "Key 4",
    "Key 5",
    "Key 6",
    "Key 7",
    "Key 8",
    "Key 9",
    "Key 10",
    "Key 11",
    "Key 12",
    "Button 1",
    "Button 2",
    "Button 15",
    "Button 16",
    "Button 17",
    "Button 18",
    "Button 19",
    "Button 20",
    "Button 21",
    "Button 22",
    "Button 23",
    "Button 24",
    "Button 25",
    "Button 26",
    "Button 27",
    "Button 28",
    "Button 29",
];

const FIRST_SCREEN_SWITCH: u8 = 21;
const FIRST_PUSH_SWITCH: u8 = 12;
/// The encoder behind each dial, RC1 to RC6; encoder 0 is the jog wheel.
const DIAL_ENCODERS: [u8; 6] = [2, 3, 4, 5, 1, 0];
/// The fader's dial.
pub const FADER_DIAL: u8 = 6;
const FADER_MAX: u16 = 0x3ff;
const FADER_TOUCHED: u8 = 0x80;
/// Fader positions per dial step: 64 steps end to end.
const FADER_STEP: i32 = 16;

/// The ten LED colours, by value, as roughly how each looks on the panel.
const PALETTE: [[u8; 3]; 10] = [
    [0, 0, 255],     // blue
    [255, 120, 0],   // orange
    [255, 255, 0],   // yellow
    [150, 0, 255],   // purple
    [110, 190, 255], // sky blue
    [255, 80, 170],  // pink
    [255, 0, 0],     // red
    [0, 255, 0],     // green
    [170, 255, 0],   // yellow-green
    [255, 255, 255], // white
];
/// A colour whose brightest channel is under this is off.
const DARK: u8 = 24;

/// Frames this long or longer, and every tile, are followed by a pause.
const PACED_LEN: usize = 1024;
const READ_BUFFER: usize = 1024;
const MAX_READS_PER_POLL: usize = 16;
/// More unframed bytes than this are dropped: no frame is that long.
const MAX_PENDING: usize = 64 * 1024;

/// The pauses the device needs, kept apart so tests can run without them.
#[derive(Debug, Clone, Copy)]
struct Timing {
    handshake_wait: Duration,
    after_handshake: Duration,
    after_init: Duration,
    tile_pause: Duration,
    keep_alive: Duration,
    stall: Duration,
}

const DEVICE_TIMING: Timing = Timing {
    handshake_wait: Duration::from_millis(500),
    after_handshake: Duration::from_millis(300),
    after_init: Duration::from_millis(400),
    tile_pause: Duration::from_millis(4),
    keep_alive: Duration::from_millis(300),
    stall: Duration::from_millis(2500),
};

/// One frame on the wire: routing, sequence number, opcode and data, framed
/// and escaped.
#[must_use]
pub fn frame(routing: Routing, seq: u16, opcode: u16, data: &[u8]) -> Vec<u8> {
    raw_frame(routing as u16, seq, opcode, data)
}

fn raw_frame(routing: u16, seq: u16, opcode: u16, data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(6 + data.len());
    payload.extend_from_slice(&routing.to_le_bytes());
    payload.extend_from_slice(&seq.to_le_bytes());
    payload.extend_from_slice(&opcode.to_le_bytes());
    payload.extend_from_slice(data);
    let length = u16::try_from(payload.len()).unwrap_or(u16::MAX);
    let sum = payload
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    let mut body = Vec::with_capacity(payload.len() + 3);
    body.extend_from_slice(&length.to_le_bytes());
    body.extend_from_slice(&payload);
    body.push(0u8.wrapping_sub(sum));
    let mut wire = Vec::with_capacity(body.len() + body.len() / 8 + 2);
    wire.push(START);
    for byte in body {
        match byte {
            START => wire.extend_from_slice(&[ESCAPE, ESCAPED_START]),
            ESCAPE => wire.extend_from_slice(&[ESCAPE, ESCAPED_ESCAPE]),
            END => wire.extend_from_slice(&[ESCAPE, ESCAPED_END]),
            other => wire.push(other),
        }
    }
    wire.push(END);
    wire
}

/// One message from the device.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Message {
    routing: u16,
    opcode: u16,
    data: Vec<u8>,
}

/// The message in one frame's escaped body (between `C0` and `C1`), if its
/// length and checksum hold.
fn unframe(escaped: &[u8]) -> Option<Message> {
    let mut body = Vec::with_capacity(escaped.len());
    let mut bytes = escaped.iter();
    while let Some(byte) = bytes.next() {
        if *byte == ESCAPE {
            let Some(next) = bytes.next() else {
                break;
            };
            body.push(match *next {
                ESCAPED_START => START,
                ESCAPED_ESCAPE => ESCAPE,
                ESCAPED_END => END,
                other => other,
            });
        } else {
            body.push(*byte);
        }
    }
    let length = usize::from(u16::from_le_bytes([*body.first()?, *body.get(1)?]));
    if length + 3 != body.len() {
        return None;
    }
    let payload = body.get(2..2 + length)?;
    let sum = payload
        .iter()
        .chain(body.last())
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    if sum != 0 || payload.len() < 6 {
        return None;
    }
    let word = |at: usize| u16::from_le_bytes([payload[at], payload[at + 1]]);
    Some(Message {
        routing: word(0),
        opcode: word(4),
        data: payload[6..].to_vec(),
    })
}

/// Bytes read so far, split into frames as each one completes. A frame split
/// across reads waits for the rest; bytes outside a frame are dropped.
#[derive(Default)]
struct Receiver {
    pending: Vec<u8>,
}

impl Receiver {
    /// The valid messages completed by `bytes`.
    fn push(&mut self, bytes: &[u8]) -> Vec<Message> {
        self.pending.extend_from_slice(bytes);
        let mut messages = Vec::new();
        let mut consumed = 0;
        loop {
            let rest = self.pending.get(consumed..).unwrap_or_default();
            let Some(start) = rest.iter().position(|byte| *byte == START) else {
                consumed = self.pending.len();
                break;
            };
            let after = rest.get(start + 1..).unwrap_or_default();
            let Some(end) = after.iter().position(|byte| *byte == END) else {
                consumed += start;
                break;
            };
            // A start inside means the earlier frame was cut short; the last
            // start before the end begins this one.
            let body = after.get(..end).unwrap_or_default();
            let body = body
                .iter()
                .rposition(|byte| *byte == START)
                .map_or(body, |restart| body.get(restart + 1..).unwrap_or_default());
            if let Some(message) = unframe(body) {
                messages.push(message);
            }
            consumed += start + 1 + end + 1;
        }
        self.pending.drain(..consumed.min(self.pending.len()));
        if self.pending.len() > MAX_PENDING {
            self.pending.clear();
        }
        messages
    }
}

/// The nearest of the ten LED colours, or `None` for a colour that reads as
/// off. Brightness is taken out first, since the LEDs have none of their own,
/// and the distance is weighted the "redmean" way, closer to how eyes judge it.
#[must_use]
pub fn palette_colour([r, g, b]: [u8; 3]) -> Option<u8> {
    let max = r.max(g).max(b);
    if max < DARK {
        return None;
    }
    let scale = 255.0 / f64::from(max);
    let [r, g, b] = [r, g, b].map(|channel| f64::from(channel) * scale);
    let distance = |[pr, pg, pb]: [u8; 3]| {
        let (pr, pg, pb) = (f64::from(pr), f64::from(pg), f64::from(pb));
        let mean = (r + pr) / 2.0;
        let (dr, dg, db) = (r - pr, g - pg, b - pb);
        (2.0 + mean / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - mean) / 256.0) * db * db
    };
    let mut best = 0u8;
    let mut best_distance = f64::INFINITY;
    for (value, colour) in (0u8..).zip(PALETTE) {
        let d = distance(colour);
        if d < best_distance {
            best_distance = d;
            best = value;
        }
    }
    Some(best)
}

/// A brightness percentage as the device's level, 0 to 0xAE.
#[must_use]
pub fn level(percent: u8) -> u8 {
    let scaled = (u32::from(percent.min(100)) * u32::from(MAX_LEVEL) + 50) / 100;
    u8::try_from(scaled).unwrap_or(MAX_LEVEL)
}

/// The Ryu key of a screen key's switch (21 to 32, numbered down each column).
fn screen_key(switch: u8) -> Option<u8> {
    let offset = switch.checked_sub(FIRST_SCREEN_SWITCH)?;
    (offset < SCREEN_KEYS).then(|| (offset % 3) * 4 + offset / 3)
}

/// The switch of a screen key, the other way round.
#[must_use]
pub fn screen_switch(key: u8) -> u8 {
    FIRST_SCREEN_SWITCH + 3 * (key % 4) + key / 4
}

/// What a switch is in Ryu's terms.
fn switch_input(switch: u8, pressed: bool) -> Option<Input> {
    if let Some(index) = screen_key(switch) {
        return Some(Input::Key { index, pressed });
    }
    if let Some(encoder) = switch.checked_sub(FIRST_PUSH_SWITCH) {
        if let Some(index) = DIAL_ENCODERS.iter().position(|id| *id == encoder) {
            return Some(Input::DialPress {
                index: u8::try_from(index).ok()?,
                pressed,
            });
        }
    }
    let position = EXTRA_SWITCHES.iter().position(|id| *id == switch)?;
    Some(Input::Key {
        index: SCREEN_KEYS + u8::try_from(position).ok()?,
        pressed,
    })
}

/// The pieces (x, y, width, height) a `width` x `height` rectangle at `x`, `y`
/// is painted in: rows of at most 40 x 20, the last column and row narrower.
fn tiles(x: u16, y: u16, width: u16, height: u16) -> Vec<(u16, u16, u16, u16)> {
    let mut pieces = Vec::new();
    let mut top = 0;
    while top < height {
        let tall = TILE_HEIGHT.min(height - top);
        let mut left = 0;
        while left < width {
            let wide = TILE_WIDTH.min(width - left);
            pieces.push((x + left, y + top, wide, tall));
            left += wide;
        }
        top += tall;
    }
    pieces
}

/// One rectangle's paint data: two zero bytes, the inclusive corners, pixels.
fn paint_data(x: u16, y: u16, width: u16, height: u16, pixels: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(10 + pixels.len());
    data.extend_from_slice(&[0, 0]);
    for value in [x, y, x + width - 1, y + height - 1] {
        data.extend_from_slice(&value.to_le_bytes());
    }
    data.extend_from_slice(pixels);
    data
}

pub struct Cc1 {
    transport: Box<dyn Transport>,
    timing: Timing,
    seq: u16,
    started: bool,
    receiver: Receiver,
    /// Screen-key pixels waiting for [`Driver::flush`].
    staged: BTreeMap<u8, Vec<u8>>,
    /// What each screen key shows now.
    shown: BTreeMap<u8, Vec<u8>>,
    /// Each LED's wanted colour (`None` is off).
    lights: [Option<u8>; LED_COUNT as usize],
    /// Each LED's colour as last sent; `None` before it ever was.
    lit: [Option<Option<u8>>; LED_COUNT as usize],
    keys: BTreeMap<u8, bool>,
    dial_presses: [bool; DIAL_ENCODERS.len()],
    fader_touched: bool,
    fader_position: Option<u16>,
    /// Fader positions moved but not yet a whole dial step.
    fader_travel: i32,
    last_keep_alive: Instant,
    last_received: Instant,
    last_tile: Option<Instant>,
}

impl Cc1 {
    #[must_use]
    pub fn new(_model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self::with_timing(transport, DEVICE_TIMING)
    }

    fn with_timing(transport: Box<dyn Transport>, timing: Timing) -> Self {
        let now = Instant::now();
        Self {
            transport,
            timing,
            seq: 0,
            started: false,
            receiver: Receiver::default(),
            staged: BTreeMap::new(),
            shown: BTreeMap::new(),
            lights: [None; LED_COUNT as usize],
            lit: [None; LED_COUNT as usize],
            keys: BTreeMap::new(),
            dial_presses: [false; DIAL_ENCODERS.len()],
            fader_touched: false,
            fader_position: None,
            fader_travel: 0,
            last_keep_alive: now,
            last_received: now,
            last_tile: None,
        }
    }

    fn next_seq(&mut self) -> u16 {
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        seq
    }

    /// Sends one frame, pausing after a large one so the device keeps up.
    fn send(&mut self, routing: Routing, opcode: u16, data: &[u8]) -> DriverResult<()> {
        let seq = self.next_seq();
        let wire = frame(routing, seq, opcode, data);
        self.transport.write(&wire)?;
        // Every tile is paced, the short ones along the bottom edges too.
        if wire.len() >= PACED_LEN || opcode == OP_LCD_PAINT {
            pause(self.timing.tile_pause);
        }
        if opcode == OP_LCD_PAINT {
            self.last_tile = Some(Instant::now());
        }
        Ok(())
    }

    /// Handshake, then the setup frames, then a black panel. A device that
    /// never answers the handshake is refused.
    fn start(&mut self) -> DriverResult<()> {
        if self.started {
            return Ok(());
        }
        self.send(Routing::Handshake, OP_DEVICE_INFO, &[])?;
        if !self.await_handshake()? {
            return Err(DriverError::Refused(
                "The CC121 did not answer. Quit Yamaha's ControlCenter or any other app using it, then unplug and replug it.".to_owned(),
            ));
        }
        pause(self.timing.after_handshake);
        self.send(Routing::FaderMode, OP_FADER_TOUCH_MODE, &[0x00])?;
        self.send(Routing::Command, OP_PANEL_MODE, &[0x00, 0x07])?;
        // Last: the panel mode re-initialises the screen.
        self.send(Routing::Command, OP_BACKLIGHT, &[0x00, MAX_LEVEL])?;
        pause(self.timing.after_init);
        let black = vec![0u8; usize::from(TILE_WIDTH) * usize::from(TILE_HEIGHT) * 2];
        for (x, y, width, height) in tiles(0, 0, PANEL_WIDTH, PANEL_HEIGHT) {
            let len = usize::from(width) * usize::from(height) * 2;
            let data = paint_data(x, y, width, height, black.get(..len).unwrap_or_default());
            self.send(Routing::Command, OP_LCD_PAINT, &data)?;
        }
        self.send(Routing::Command, OP_LCD_COMMIT, &[0x00])?;
        self.started = true;
        self.last_keep_alive = Instant::now();
        self.last_received = Instant::now();
        Ok(())
    }

    /// Waits for the device's info, keeping anything after it for `poll`.
    fn await_handshake(&mut self) -> DriverResult<bool> {
        let deadline = Instant::now() + self.timing.handshake_wait;
        let mut buf = [0u8; READ_BUFFER];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let wait = i32::try_from(left.as_millis()).unwrap_or(i32::MAX);
            let read = self.transport.read_timeout(&mut buf, wait)?;
            let answered = self
                .receiver
                .push(buf.get(..read).unwrap_or_default())
                .iter()
                .any(|message| {
                    message.routing & FROM_DEVICE != 0
                        && message.opcode & !FROM_DEVICE == OP_DEVICE_INFO
                });
            if answered {
                return Ok(true);
            }
            if left.is_zero() {
                return Ok(false);
            }
        }
    }

    fn handle(&mut self, message: &Message, events: &mut Vec<Input>) -> DriverResult<()> {
        let data = &message.data;
        match message.opcode & !FROM_DEVICE {
            OP_SWITCH => {
                let (Some(switch), Some(state)) = (data.first(), data.get(1)) else {
                    return Ok(());
                };
                let pressed = *state != 0;
                match switch_input(*switch, pressed) {
                    Some(Input::Key { index, pressed }) => {
                        let held = self.keys.entry(index).or_insert(false);
                        if *held != pressed {
                            *held = pressed;
                            events.push(Input::Key { index, pressed });
                        }
                    }
                    Some(Input::DialPress { index, pressed }) => {
                        if let Some(held) = self.dial_presses.get_mut(usize::from(index)) {
                            if *held != pressed {
                                *held = pressed;
                                events.push(Input::DialPress { index, pressed });
                            }
                        }
                    }
                    _ => {}
                }
            }
            OP_ENCODER => {
                let (Some(encoder), Some(delta)) = (data.first(), data.get(1)) else {
                    return Ok(());
                };
                let delta = i8::from_le_bytes([*delta]);
                let dial = DIAL_ENCODERS
                    .iter()
                    .position(|id| id == encoder)
                    .and_then(|dial| u8::try_from(dial).ok());
                if let Some(index) = dial.filter(|_| delta != 0) {
                    events.push(Input::DialTurn { index, delta });
                }
            }
            OP_FADER => {
                let (Some(low), Some(high), Some(flags)) = (data.get(1), data.get(2), data.get(3))
                else {
                    return Ok(());
                };
                let position = u16::from_le_bytes([*low, *high]) & FADER_MAX;
                let touched = flags & FADER_TOUCHED != 0;
                self.fader(position, touched, events)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// A fader report: a touch press or release, and steps for the distance
    /// moved by hand. The motor's own moves are not input.
    fn fader(&mut self, position: u16, touched: bool, events: &mut Vec<Input>) -> DriverResult<()> {
        let by_hand = touched || self.fader_touched;
        if let (Some(last), true) = (self.fader_position, by_hand) {
            self.fader_travel += i32::from(position) - i32::from(last);
            let steps = self.fader_travel / FADER_STEP;
            if steps != 0 {
                self.fader_travel -= steps * FADER_STEP;
                let delta = i8::try_from(steps.clamp(-127, 127)).unwrap_or(0);
                events.push(Input::DialTurn {
                    index: FADER_DIAL,
                    delta,
                });
            }
        }
        self.fader_position = Some(position);
        if touched != self.fader_touched {
            self.fader_touched = touched;
            self.fader_travel = 0;
            events.push(Input::DialPress {
                index: FADER_DIAL,
                pressed: touched,
            });
            if !touched {
                // Otherwise the motor slides it back to the bottom.
                self.hold_fader(position)?;
            }
        }
        Ok(())
    }

    /// Tells the motor to hold the fader at `position`.
    fn hold_fader(&mut self, position: u16) -> DriverResult<()> {
        let [low, high] = (position & FADER_MAX).to_le_bytes();
        self.send(Routing::Command, OP_FADER, &[0x00, 0x01, low, high & 0x03])
    }

    fn keep_alive(&mut self) -> DriverResult<()> {
        if self.last_keep_alive.elapsed() >= self.timing.keep_alive {
            self.last_keep_alive = Instant::now();
            self.send(Routing::KeepAlive, OP_DEVICE_INFO, &[])?;
        }
        Ok(())
    }

    /// Paints one screen key's 72 x 72 pixels in tiles at its aperture.
    fn paint_key(&mut self, key: u8, pixels: &[u8]) -> DriverResult<()> {
        let (Some(x), Some(y)) = (
            KEY_X.get(usize::from(key % 4)),
            KEY_Y.get(usize::from(key / 4)),
        ) else {
            return Ok(());
        };
        let row_bytes = usize::from(KEY_SIZE) * 2;
        for (left, top, width, height) in tiles(0, 0, KEY_SIZE, KEY_SIZE) {
            let mut tile = Vec::with_capacity(usize::from(width) * usize::from(height) * 2);
            for row in top..top + height {
                let from = usize::from(row) * row_bytes + usize::from(left) * 2;
                let Some(line) = pixels.get(from..from + usize::from(width) * 2) else {
                    return Ok(());
                };
                tile.extend_from_slice(line);
            }
            let data = paint_data(x + left, y + top, width, height, &tile);
            self.send(Routing::Command, OP_LCD_PAINT, &data)?;
        }
        Ok(())
    }
}

/// Whether paints went out after the device last spoke, and long enough ago
/// that their acknowledgements should have come.
fn stalled(
    last_tile: Option<Instant>,
    last_received: Instant,
    now: Instant,
    after: Duration,
) -> bool {
    last_tile.is_some_and(|tile| tile > last_received && now.duration_since(tile) > after)
}

fn pause(duration: Duration) {
    if !duration.is_zero() {
        std::thread::sleep(duration);
    }
}

impl Driver for Cc1 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.start()?;
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_READS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            let messages = self.receiver.push(buf.get(..read).unwrap_or_default());
            if !messages.is_empty() {
                self.last_received = Instant::now();
            }
            for message in &messages {
                self.handle(message, &mut events)?;
            }
        }
        self.keep_alive()?;
        if stalled(
            self.last_tile,
            self.last_received,
            Instant::now(),
            self.timing.stall,
        ) {
            return Err(DriverError::Io(
                "the CC121 stopped acknowledging its screen".to_owned(),
            ));
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if key < SCREEN_KEYS {
            self.staged.insert(key, face::encode(face, &KEY_IMAGE)?);
            return Ok(());
        }
        let led = key
            .checked_sub(SCREEN_KEYS)
            .and_then(|extra| EXTRA_LEDS.get(usize::from(extra)))
            .copied()
            .flatten();
        if let Some(slot) = led.and_then(|led| self.lights.get_mut(usize::from(led))) {
            *slot = face.and_then(|face| palette_colour(face.color));
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.start()?;
        let staged = std::mem::take(&mut self.staged);
        let mut painted = false;
        for (key, pixels) in staged {
            if self.shown.get(&key) == Some(&pixels) {
                continue;
            }
            self.paint_key(key, &pixels)?;
            self.shown.insert(key, pixels);
            painted = true;
        }
        if painted {
            self.send(Routing::Command, OP_LCD_COMMIT, &[0x00])?;
        }
        for led in 0..LED_COUNT {
            let index = usize::from(led);
            let wanted = self.lights.get(index).copied().flatten();
            if self.lit.get(index).copied().flatten() == Some(wanted) {
                continue;
            }
            let data = match wanted {
                Some(colour) => [led, colour, 0x01],
                None => [led, OFF_COLOUR, 0x00],
            };
            self.send(Routing::Command, OP_LED, &data)?;
            if let Some(slot) = self.lit.get_mut(index) {
                *slot = Some(wanted);
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.start()?;
        let level = level(percent);
        let data: Vec<u8> = (0..LED_COUNT).flat_map(|led| [led, level]).collect();
        self.send(Routing::Command, OP_LED_BRIGHTNESS, &data)?;
        self.send(Routing::Command, OP_BACKLIGHT, &[0x00, level])
    }
}

impl Drop for Cc1 {
    /// Leaves the lights off and the screen dark when Ryu lets go of the deck.
    /// The port closes normally; DTR stays up (see the manager).
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for led in 0..LED_COUNT {
            if self
                .send(Routing::Command, OP_LED, &[led, OFF_COLOUR, 0x00])
                .is_err()
            {
                return;
            }
        }
        let _ = self.send(Routing::Command, OP_BACKLIGHT, &[0x00, 0x00]);
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::Recorder;
    use super::*;

    const NO_WAIT: Timing = Timing {
        handshake_wait: Duration::ZERO,
        after_handshake: Duration::ZERO,
        after_init: Duration::ZERO,
        tile_pause: Duration::ZERO,
        keep_alive: Duration::from_secs(3600),
        stall: Duration::from_secs(3600),
    };

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// What the device sends: routing and opcode with bit 15 set.
    fn from_device(opcode: u16, data: &[u8]) -> Vec<u8> {
        raw_frame(0x8001, 0, opcode | FROM_DEVICE, data)
    }

    fn device_info() -> Vec<u8> {
        raw_frame(
            FROM_DEVICE,
            0,
            OP_DEVICE_INFO,
            &[0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01],
        )
    }

    /// A started CC1, with the writes so far cleared.
    fn deck() -> (Cc1, Recorder) {
        let recorder = Recorder::default();
        recorder.queue_read(device_info());
        let mut deck = Cc1::with_timing(Box::new(recorder.clone()), NO_WAIT);
        deck.start().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        (deck, recorder)
    }

    fn payloads(recorder: &Recorder) -> Vec<Message> {
        let mut receiver = Receiver::default();
        recorder
            .writes()
            .iter()
            .flat_map(|wire| receiver.push(wire))
            .collect()
    }

    #[test]
    fn frames_match_the_capture() {
        assert_eq!(
            hex(&frame(Routing::Handshake, 0, OP_DEVICE_INFO, &[])),
            "c0060000000000000000c1"
        );
        assert_eq!(
            hex(&frame(Routing::FaderMode, 1, OP_FADER_TOUCH_MODE, &[0x00])),
            "c0070002000100040200f7c1"
        );
        assert_eq!(
            hex(&frame(Routing::Command, 2, OP_PANEL_MODE, &[0x00, 0x07])),
            "c008000100020004010007f1c1"
        );
        assert_eq!(
            hex(&frame(Routing::Command, 3, OP_BACKLIGHT, &[0x00, 0xae])),
            "c0080001000300820000aeccc1"
        );
        // The commit carries one zero byte.
        let commit = frame(Routing::Command, 9, OP_LCD_COMMIT, &[0x00]);
        assert_eq!(hex(&commit), "c007000100090082020072c1");
    }

    #[test]
    fn reserved_bytes_are_escaped_and_survive() {
        let wire = frame(Routing::Command, 0, 0x0001, &[0xc0, 0xc1, 0xdb]);
        let inner = &wire[1..wire.len() - 1];
        assert!(!inner.contains(&START) && !inner.contains(&END));
        assert!(hex(&wire).contains("dbdcdbdedbdd"));
        let message = Receiver::default().push(&wire).pop().unwrap();
        assert_eq!(message.data, vec![0xc0, 0xc1, 0xdb]);
    }

    #[test]
    fn frames_split_across_reads_and_bad_ones_are_dropped() {
        let mut receiver = Receiver::default();
        let one = from_device(OP_SWITCH, &[21, 1]);
        let two = from_device(OP_SWITCH, &[21, 0]);
        let mut stream = vec![0x55, 0x66];
        stream.extend_from_slice(&one);
        stream.extend_from_slice(&two[..4]);
        assert_eq!(receiver.push(&stream).len(), 1);
        assert_eq!(receiver.push(&two[4..]).len(), 1);
        // A wrong checksum, a wrong length and a short payload are all dropped.
        let mut corrupt = one.clone();
        let at = corrupt.len() - 2;
        corrupt[at] ^= 0x01;
        assert!(receiver.push(&corrupt).is_empty());
        assert!(receiver.push(&[0xc0, 0x05, 0x00, 0x00, 0xc1]).is_empty());
        assert!(receiver
            .push(&[0xc0, 0x01, 0x00, 0x05, 0xfb, 0xc1])
            .is_empty());
        assert!(receiver.push(&[0xc0]).is_empty());
        assert!(receiver.push(&[0xc1, 0xc1, 0xdb]).is_empty());
        assert_eq!(receiver.push(&one).len(), 1);
    }

    #[test]
    fn start_shakes_hands_sets_up_in_order_and_clears_the_panel() {
        let recorder = Recorder::default();
        recorder.queue_read(device_info());
        let mut deck = Cc1::with_timing(Box::new(recorder.clone()), NO_WAIT);
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(hex(&writes[0]), "c0060000000000000000c1");
        assert_eq!(hex(&writes[1]), "c0070002000100040200f7c1");
        assert_eq!(hex(&writes[2]), "c008000100020004010007f1c1");
        assert_eq!(hex(&writes[3]), "c0080001000300820000aeccc1");
        let messages = payloads(&recorder);
        let paints: Vec<&Message> = messages
            .iter()
            .filter(|message| message.opcode == OP_LCD_PAINT)
            .collect();
        assert_eq!(paints.len(), 168);
        // The last tile is the bottom-right 40 x 12, corners inclusive.
        let last = paints.last().unwrap();
        assert_eq!(
            &last.data[..10],
            &[0, 0, 0xb8, 0x01, 0x04, 0x01, 0xdf, 0x01, 0x0f, 0x01]
        );
        assert_eq!(last.data.len(), 10 + 40 * 12 * 2);
        let commit = &messages[4 + 168];
        assert_eq!(
            (commit.opcode, commit.data.as_slice()),
            (OP_LCD_COMMIT, &[0][..])
        );
        // Sequence numbers count up from the handshake's 0.
        assert_eq!(&writes[4][3..7], &[0x01, 0x00, 0x04, 0x00]);
    }

    #[test]
    fn a_silent_device_is_refused() {
        let recorder = Recorder::default();
        let mut deck = Cc1::with_timing(Box::new(recorder.clone()), NO_WAIT);
        assert!(matches!(deck.poll(0), Err(DriverError::Refused(_))));
        assert_eq!(recorder.writes().len(), 1);
    }

    #[test]
    fn switches_become_keys_and_dial_presses() {
        let (mut deck, recorder) = deck();
        for (switch, pressed) in [
            (21, 1),
            (24, 1),
            (22, 1),
            (32, 1),
            (33, 1),
            (0, 1),
            (12, 1),
            (14, 1),
            (12, 0),
            (21, 0),
            (21, 0),
            (99, 1),
        ] {
            recorder.queue_read(from_device(OP_SWITCH, &[switch, pressed]));
        }
        recorder.queue_read(from_device(OP_SWITCH, &[21]));
        let events = deck.poll(0).unwrap();
        assert_eq!(
            events,
            vec![
                Input::Key {
                    index: 0,
                    pressed: true
                },
                Input::Key {
                    index: 1,
                    pressed: true
                },
                Input::Key {
                    index: 4,
                    pressed: true
                },
                Input::Key {
                    index: 11,
                    pressed: true
                },
                Input::Key {
                    index: 12,
                    pressed: true
                },
                Input::Key {
                    index: 28,
                    pressed: true
                },
                // Encoder 0 is the jog wheel (RC6); encoder 2 is RC1.
                Input::DialPress {
                    index: 5,
                    pressed: true
                },
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                Input::DialPress {
                    index: 5,
                    pressed: false
                },
                Input::Key {
                    index: 0,
                    pressed: false
                },
            ]
        );
        for key in 0..12 {
            assert_eq!(screen_key(screen_switch(key)), Some(key));
        }
    }

    #[test]
    fn encoders_turn_dials_by_their_signed_delta() {
        let (mut deck, recorder) = deck();
        recorder.queue_read(from_device(OP_ENCODER, &[2, 0x01]));
        recorder.queue_read(from_device(OP_ENCODER, &[0, 0xfc]));
        recorder.queue_read(from_device(OP_ENCODER, &[1, 0xff]));
        recorder.queue_read(from_device(OP_ENCODER, &[9, 0x01]));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialTurn { index: 0, delta: 1 },
                Input::DialTurn {
                    index: 5,
                    delta: -4
                },
                Input::DialTurn {
                    index: 4,
                    delta: -1
                },
            ]
        );
    }

    #[test]
    fn the_fader_is_a_dial_moved_by_hand_and_held_on_release() {
        let (mut deck, recorder) = deck();
        // The motor parking it untouched is not input.
        recorder.queue_read(from_device(OP_FADER, &[0x00, 0x00, 0x01, 0x00]));
        recorder.queue_read(from_device(OP_FADER, &[0x00, 0x00, 0x00, 0x00]));
        // Touch, move up 40 positions (two steps, 8 left over), then let go.
        recorder.queue_read(from_device(OP_FADER, &[0x00, 0x00, 0x00, 0x80]));
        recorder.queue_read(from_device(OP_FADER, &[0x00, 0x28, 0x00, 0x80]));
        recorder.queue_read(from_device(OP_FADER, &[0x00, 0x28, 0xfc, 0x00]));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialPress {
                    index: 6,
                    pressed: true
                },
                Input::DialTurn { index: 6, delta: 2 },
                Input::DialPress {
                    index: 6,
                    pressed: false
                },
            ]
        );
        let hold = payloads(&recorder).pop().unwrap();
        assert_eq!(hold.opcode, OP_FADER);
        assert_eq!(hold.routing, 1);
        assert_eq!(hold.data, vec![0x00, 0x01, 0x28, 0x00]);
        // Full scale masks to ten bits.
        recorder.queue_read(from_device(OP_FADER, &[0x00, 0xff, 0xff, 0x80]));
        deck.poll(0).unwrap();
        assert_eq!(deck.fader_position, Some(1023));
    }

    #[test]
    fn a_screen_key_is_painted_in_eight_tiles_then_committed_once() {
        let (mut deck, recorder) = deck();
        let face = Face {
            label: String::new(),
            color: [255, 255, 255],
            image: None,
        };
        deck.set_face(11, Some(&face)).unwrap();
        deck.flush().unwrap();
        let messages = payloads(&recorder);
        assert_eq!(messages.len(), 9 + 15);
        let corners: Vec<&[u8]> = messages[..8].iter().map(|m| &m.data[2..10]).collect();
        // Key 11 sits at 393, 194: tiles of 40 and 32 wide, 20, 20, 20 and 12 tall.
        assert_eq!(
            corners[0],
            &[0x89, 0x01, 0xc2, 0x00, 0xb0, 0x01, 0xd5, 0x00]
        );
        assert_eq!(
            corners[1],
            &[0xb1, 0x01, 0xc2, 0x00, 0xd0, 0x01, 0xd5, 0x00]
        );
        assert_eq!(
            corners[7],
            &[0xb1, 0x01, 0xfe, 0x00, 0xd0, 0x01, 0x09, 0x01]
        );
        assert_eq!(messages[1].data.len(), 10 + 32 * 20 * 2);
        assert!(messages[0].data[10..].iter().all(|byte| *byte == 0xff));
        assert_eq!(
            (messages[8].opcode, messages[8].data.as_slice()),
            (OP_LCD_COMMIT, &[0][..])
        );
        // Unchanged, it is not sent again.
        recorder.0.lock().unwrap().writes.clear();
        deck.set_face(11, Some(&face)).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn lights_take_the_nearest_palette_colour() {
        let (mut deck, recorder) = deck();
        let face = |color| Face {
            label: String::new(),
            color,
            image: None,
        };
        // Button 18 (switch 9) has LED 2; buttons 1 and 2 have none.
        deck.set_face(17, Some(&face([255, 120, 0]))).unwrap();
        deck.set_face(12, Some(&face([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let leds: Vec<Vec<u8>> = payloads(&recorder).into_iter().map(|m| m.data).collect();
        assert_eq!(leds.len(), 15);
        assert_eq!(leds[2], vec![0x02, 0x01, 0x01]);
        assert_eq!(leds[0], vec![0x00, 0x01, 0x00]);
        let wire = &recorder.writes()[2];
        assert_eq!(hex(&wire[7..12]), "8103020101");
        // Turning it off keeps a valid colour byte.
        recorder.0.lock().unwrap().writes.clear();
        deck.set_face(17, None).unwrap();
        deck.flush().unwrap();
        let off: Vec<Vec<u8>> = payloads(&recorder).into_iter().map(|m| m.data).collect();
        assert_eq!(off, vec![vec![0x02, 0x01, 0x00]]);

        assert_eq!(palette_colour([255, 0, 0]), Some(6));
        assert_eq!(palette_colour([0, 255, 0]), Some(7));
        assert_eq!(palette_colour([0, 0, 255]), Some(0));
        assert_eq!(palette_colour([255, 255, 255]), Some(9));
        assert_eq!(palette_colour([255, 255, 0]), Some(2));
        assert_eq!(palette_colour([128, 255, 0]), Some(8));
        assert_eq!(palette_colour([0, 255, 255]), Some(4));
        assert_eq!(palette_colour([64, 0, 0]), Some(6));
        assert_eq!(palette_colour([120, 120, 120]), Some(9));
        assert_eq!(palette_colour([0, 0, 0]), None);
        assert_eq!(palette_colour([23, 10, 5]), None);
    }

    #[test]
    fn brightness_sets_every_led_and_the_backlight() {
        let (mut deck, recorder) = deck();
        deck.set_brightness(100).unwrap();
        let messages = payloads(&recorder);
        assert_eq!(messages[0].opcode, OP_LED_BRIGHTNESS);
        assert_eq!(messages[0].data.len(), 30);
        assert_eq!(&messages[0].data[28..], &[14, 0xae]);
        assert_eq!(messages[1].data, vec![0x00, 0xae]);
        assert_eq!(level(0), 0);
        assert_eq!(level(50), 0x57);
        assert_eq!(level(200), 0xae);
    }

    #[test]
    fn keep_alives_go_out_on_routing_four() {
        let recorder = Recorder::default();
        recorder.queue_read(device_info());
        let timing = Timing {
            keep_alive: Duration::ZERO,
            ..NO_WAIT
        };
        let mut deck = Cc1::with_timing(Box::new(recorder.clone()), timing);
        deck.start().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][..3], &[0xc0, 0x06, 0x00]);
        assert_eq!(&writes[0][3..5], &[0x04, 0x00]);
        assert_eq!(&writes[0][7..9], &[0x00, 0x00]);
    }

    #[test]
    fn unanswered_paints_mean_a_stall() {
        let now = Instant::now();
        let after = Duration::from_millis(2500);
        let earlier = now.checked_sub(Duration::from_secs(3)).unwrap();
        let before = now.checked_sub(Duration::from_secs(4)).unwrap();
        assert!(stalled(Some(earlier), before, now, after));
        assert!(!stalled(Some(earlier), earlier, now, after));
        assert!(!stalled(Some(now), before, now, after));
        assert!(!stalled(None, before, now, after));
    }

    #[test]
    fn letting_go_turns_the_lights_off_and_the_backlight_down() {
        let (deck, recorder) = deck();
        drop(deck);
        let messages = payloads(&recorder);
        assert_eq!(messages.len(), 16);
        assert_eq!(messages[15].data, vec![0x00, 0x00]);
    }
}
