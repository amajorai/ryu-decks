//! monome grids and arcs over USB serial (8N1, raw, no keep-alive or commit).
//!
//! A monome has no USB id of its own: older units sit behind a stock FTDI
//! bridge and 2022 and later ones are CDC-ACM. It is found by its USB serial
//! number, which also names the protocol ([`classify_serial`]), or, for a 2022
//! and later grid, by the USB strings "monome" and "grid".
//!
//! - mext (serial `m` and digits; grids from 2011, every arc): one header byte,
//!   `section << 4 | command`, and a payload whose length the header fixes.
//!   The host asks `00` (which sections: `00 05 n` is an arc with `n`
//!   encoders) and `05` (grid size, answered `03 cols rows`). In: `21 x y` key
//!   down, `20 x y` key up, `50 n d` an encoder turned by the signed `d`, `52 n`
//!   / `51 n` an encoder pushed / let go (2011 arcs only). Out: `1A x y` and 32
//!   bytes of 4-bit levels for one 8 x 8 block (row-major, the even index in
//!   the high nibble), `17 i` grid intensity, `91 n level` a whole ring, `94 i
//!   00` ring intensity.
//! - series (serial `m64-`, `m128-`; 2007 to 2010): two bytes a message. In:
//!   `00 xy` key down, `10 xy` key up (x the high nibble). Out: `8q` and eight
//!   row bytes for one 8 x 8 quadrant `q` (bit i the column, LSB first), `Ai`
//!   intensity. LEDs are on or off.
//! - 40h (serial `m40h`, and the arduinome clone `a40h-` at 57600 baud): two
//!   bytes a message. In: `01 xy` key down, `00 xy` key up. Out: `7y bits` one
//!   row, `30 i` intensity. LEDs are on or off.
//!
//! Keys are numbered row by row from the top left, `y * cols + x`, at the
//! grid's own rotation. A 16 x 16 grid (256 keys) is more than a Ryu deck
//! holds, so it is not driven. Every light is dark when Ryu takes the device and
//! again when it lets go.
//!
//! Facts per monome's libmonome and serialosc (ISC) and the monome docs (MIT);
//! no code was copied.

use std::time::{Duration, Instant};

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{self, DeviceModel, Protocol};

pub const GRID_64_ID: &str = "monome-grid-64";
pub const GRID_128_ID: &str = "monome-grid-128";
pub const ARC_2_ID: &str = "monome-arc-2";
pub const ARC_4_ID: &str = "monome-arc-4";
pub const SERIES_64_ID: &str = "monome-series-64";
pub const SERIES_128_ID: &str = "monome-series-128";
pub const FORTY_H_ID: &str = "monome-40h";
pub const ARDUINOME_ID: &str = "monome-arduinome";

/// The arduinome's baud rate; every other monome runs at 115200.
pub const ARDUINOME_BAUD: u32 = 57_600;

/// The USB strings a 2022 and later grid reports, whatever its serial.
const CDC_MANUFACTURER: &str = "monome";
const CDC_PRODUCT: &str = "grid";

const MEXT_QUERY: u8 = 0x00;
const MEXT_GET_SIZE: u8 = 0x05;
const MEXT_QUERY_REPLY: u8 = 0x00;
const MEXT_SIZE_REPLY: u8 = 0x03;
const MEXT_KEY_UP: u8 = 0x20;
const MEXT_KEY_DOWN: u8 = 0x21;
const MEXT_ENCODER_DELTA: u8 = 0x50;
const MEXT_ENCODER_UP: u8 = 0x51;
const MEXT_ENCODER_DOWN: u8 = 0x52;
const MEXT_GRID_INTENSITY: u8 = 0x17;
const MEXT_LEVEL_MAP: u8 = 0x1a;
const MEXT_RING_ALL: u8 = 0x91;
const MEXT_RING_INTENSITY: u8 = 0x94;
/// The query reply's section for encoders.
const MEXT_ENCODER_SECTION: u8 = 0x05;

const SERIES_KEY_DOWN: u8 = 0x00;
const SERIES_KEY_UP: u8 = 0x10;
const SERIES_FRAME: u8 = 0x80;
const SERIES_INTENSITY: u8 = 0xa0;

const FORTY_H_KEY_UP: u8 = 0x00;
const FORTY_H_KEY_DOWN: u8 = 0x01;
const FORTY_H_ROW: u8 = 0x70;
const FORTY_H_INTENSITY: u8 = 0x30;

/// The brightest level a light or intensity takes.
const MAX_LEVEL: u8 = 15;
/// A light that is only on or off is on for any colour brighter than this.
const ON_OFF_THRESHOLD: u8 = 16;
/// Encoder counts per dial step: 1024 a turn on 2012 and later arcs, so a
/// step is one of the ring's 64 LEDs (a quarter turn's sixteenth on 2011 arcs).
const COUNTS_PER_STEP: i32 = 16;
/// How long the probe waits for answers before asking again, and how often.
const PROBE_ROUND: Duration = Duration::from_millis(250);
const PROBE_ROUNDS: usize = 4;
const READ_BUFFER: usize = 256;
const MAX_READS_PER_POLL: usize = 32;
/// Unparsed input kept at most; more means the stream is not what Ryu expects.
const MAX_PENDING: usize = 1024;

/// The protocol a monome's USB serial number names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerialKind {
    /// Grids from 2011 and arcs: the size is asked for.
    Mext,
    /// 2007 to 2010 grids and kits, sized by the serial.
    Series { cols: u8, rows: u8 },
    /// The 40h and its kits, or the arduinome clone (57600 baud).
    FortyH { arduinome: bool },
}

/// One or more ASCII digits, then nothing or the `A` Windows adds to an FTDI
/// serial.
fn numbered(rest: &str) -> bool {
    let digits = rest.strip_suffix('A').unwrap_or(rest);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// `-` or `_`, then [`numbered`].
fn dashed(rest: &str) -> bool {
    rest.strip_prefix(['-', '_']).is_some_and(numbered)
}

/// What a USB serial number says a monome speaks, checked in libmonome's
/// order (`m64-0001` would also pass as mext's `m` and digits otherwise).
#[must_use]
pub fn classify_serial(serial: &str) -> Option<SerialKind> {
    let serial = serial.trim();
    let after = |prefix: &str| serial.strip_prefix(prefix);
    if after("m64").is_some_and(dashed) {
        return Some(SerialKind::Series { cols: 8, rows: 8 });
    }
    if after("m128").is_some_and(dashed) {
        return Some(SerialKind::Series { cols: 16, rows: 8 });
    }
    if after("m256").is_some_and(dashed) || after("mk").is_some_and(numbered) {
        return Some(SerialKind::Series { cols: 16, rows: 16 });
    }
    if after("m40h").is_some_and(numbered) {
        return Some(SerialKind::FortyH { arduinome: false });
    }
    if after("a40h").is_some_and(dashed) {
        return Some(SerialKind::FortyH { arduinome: true });
    }
    if after("m").or_else(|| after("M")).is_some_and(numbered) {
        return Some(SerialKind::Mext);
    }
    None
}

/// The model a USB serial port is, judged by its strings alone. A mext device
/// gets the 128 grid's model until [`probe`] has asked what it is; a 16 x 16
/// series grid or kit gets none.
#[must_use]
pub fn identify(
    serial: Option<&str>,
    manufacturer: Option<&str>,
    product: Option<&str>,
) -> Option<&'static DeviceModel> {
    let kind = serial.and_then(classify_serial).or_else(|| {
        (manufacturer.map(str::trim) == Some(CDC_MANUFACTURER)
            && product.map(str::trim) == Some(CDC_PRODUCT))
        .then_some(SerialKind::Mext)
    })?;
    let id = match kind {
        SerialKind::Mext => GRID_128_ID,
        SerialKind::Series { cols: 8, rows: 8 } => SERIES_64_ID,
        SerialKind::Series { cols: 16, rows: 8 } => SERIES_128_ID,
        SerialKind::Series { .. } => return None,
        SerialKind::FortyH { arduinome: false } => FORTY_H_ID,
        SerialKind::FortyH { arduinome: true } => ARDUINOME_ID,
    };
    catalog::by_id(id)
}

/// How many payload bytes follow a mext header from the device.
const fn mext_payload(header: u8) -> usize {
    match header {
        0x00 | 0x03 | 0x04 | 0x20 | 0x21 | 0x50 => 2,
        0x01 => 32,
        0x02 => 3,
        0x0f => 8,
        0x51 | 0x52 | 0x80 => 1,
        0x81 => 7,
        _ => 0,
    }
}

/// Splits whole mext messages off the front of `pending`, leaving a partial one.
fn take_mext(pending: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut messages = Vec::new();
    let mut at = 0;
    while let Some(&header) = pending.get(at) {
        let end = at + 1 + mext_payload(header);
        let Some(message) = pending.get(at..end) else {
            break;
        };
        messages.push(message.to_vec());
        at = end;
    }
    pending.drain(..at);
    messages
}

/// What the device said while it was asked what it is.
#[derive(Debug, Default)]
struct Answers {
    queried: bool,
    encoders: Option<u8>,
    size: Option<(u8, u8)>,
}

impl Answers {
    fn note(&mut self, message: &[u8]) {
        match message {
            [MEXT_QUERY_REPLY, section, count] => {
                self.queried = true;
                if *section == MEXT_ENCODER_SECTION && *count > 0 {
                    self.encoders = Some(*count);
                }
            }
            [MEXT_SIZE_REPLY, cols, rows] => self.size = Some((*cols, *rows)),
            _ => {}
        }
    }

    fn complete(&self) -> bool {
        self.queried && self.size.is_some()
    }

    /// An arc by its encoder count, else a grid by its size.
    fn model(&self) -> Option<&'static DeviceModel> {
        let id = match (self.encoders, self.size) {
            (Some(1 | 2), _) => ARC_2_ID,
            (Some(3 | 4), _) => ARC_4_ID,
            (Some(_), _) => return None,
            (None, Some((8, 8))) => GRID_64_ID,
            (None, Some((16, 8))) => GRID_128_ID,
            (None, _) => return None,
        };
        catalog::by_id(id)
    }
}

/// What a mext device said to [`probe`].
#[derive(Clone, Copy, Debug)]
pub enum MextAnswer {
    /// It did not answer the query.
    Silent,
    /// A grid or arc Ryu does not drive (a 16 x 16 grid).
    Undriven,
    /// The grid or arc it is.
    Model(&'static DeviceModel),
}

/// Asks a mext device what it is: the query (`00`) and grid size (`05`), asked
/// again each [`PROBE_ROUND`] while either is unanswered, for at most
/// [`PROBE_ROUNDS`].
pub fn probe(link: &mut dyn Transport) -> DriverResult<MextAnswer> {
    let mut answers = Answers::default();
    let mut pending = Vec::new();
    let mut buf = [0u8; READ_BUFFER];
    for _ in 0..PROBE_ROUNDS {
        if !answers.queried {
            link.write(&[MEXT_QUERY])?;
        }
        if answers.size.is_none() {
            link.write(&[MEXT_GET_SIZE])?;
        }
        let deadline = Instant::now() + PROBE_ROUND;
        while !answers.complete() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let wait = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
            let read = link.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            pending.extend_from_slice(buf.get(..read).unwrap_or_default());
            for message in take_mext(&mut pending) {
                answers.note(&message);
            }
            if pending.len() > MAX_PENDING {
                pending.clear();
            }
        }
        if answers.complete() {
            break;
        }
    }
    if let Some(model) = answers.model() {
        return Ok(MextAnswer::Model(model));
    }
    if !answers.queried {
        return Ok(MextAnswer::Silent);
    }
    tracing::info!(
        encoders = ?answers.encoders,
        size = ?answers.size,
        "ryu-decks: a monome answered, but not as a grid or arc Ryu drives"
    );
    Ok(MextAnswer::Undriven)
}

/// A light level, 0 to 15, from a colour's brightest channel.
fn level_of(color: [u8; 3]) -> u8 {
    let [red, green, blue] = color;
    let brightest = u16::from(red.max(green).max(blue));
    u8::try_from((brightest * u16::from(MAX_LEVEL) + 127) / 255).unwrap_or(MAX_LEVEL)
}

/// On or off, for the series and 40h, whose LEDs have no levels.
const fn is_on(color: [u8; 3]) -> bool {
    let [red, green, blue] = color;
    red > ON_OFF_THRESHOLD || green > ON_OFF_THRESHOLD || blue > ON_OFF_THRESHOLD
}

/// A brightness percentage as an intensity, 0 to 15.
fn intensity_of(percent: u8) -> u8 {
    u8::try_from((u16::from(percent.min(100)) * u16::from(MAX_LEVEL) + 50) / 100)
        .unwrap_or(MAX_LEVEL)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Mext,
    Series,
    FortyH,
}

pub struct Monome {
    transport: Box<dyn Transport>,
    dialect: Dialect,
    cols: u8,
    rows: u8,
    rings: u8,
    started: bool,
    /// Each key's light colour; `None` for off.
    colors: Vec<Option<[u8; 3]>>,
    /// Each ring's colour; `None` for off.
    ring_colors: Vec<Option<[u8; 3]>>,
    /// The last message sent for each block, row or ring, in [`Monome::messages`] order.
    sent: Vec<Vec<u8>>,
    pending: Vec<u8>,
    /// Encoder counts not yet a whole dial step.
    turns: Vec<i32>,
}

impl Monome {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let dialect = match model.protocol {
            Protocol::MonomeSeries => Dialect::Series,
            Protocol::Monome40h => Dialect::FortyH,
            _ => Dialect::Mext,
        };
        Self {
            transport,
            dialect,
            cols: model.cols,
            rows: model.rows,
            rings: model.dials,
            started: false,
            colors: vec![None; usize::from(model.keys)],
            ring_colors: vec![None; usize::from(model.dials)],
            sent: Vec::new(),
            pending: Vec::new(),
            turns: vec![0; usize::from(model.dials)],
        }
    }

    fn color_at(colors: &[Option<[u8; 3]>], cols: u8, x: u8, y: u8) -> Option<[u8; 3]> {
        colors
            .get(usize::from(y) * usize::from(cols) + usize::from(x))
            .copied()
            .flatten()
    }

    /// Eight row bytes of one 8 x 8 block, bit i the column `x0 + i`.
    fn block_bits(colors: &[Option<[u8; 3]>], cols: u8, x0: u8, y0: u8) -> [u8; 8] {
        let mut rows = [0u8; 8];
        for (dy, row) in (0u8..).zip(rows.iter_mut()) {
            for dx in 0..8u8 {
                if Self::color_at(colors, cols, x0 + dx, y0 + dy).is_some_and(is_on) {
                    *row |= 1 << dx;
                }
            }
        }
        rows
    }

    /// Every message that sets the device's lights to `colors` and `rings`:
    /// one per 8 x 8 block (mext, series) or row (40h), then one per ring.
    fn messages(&self, colors: &[Option<[u8; 3]>], rings: &[Option<[u8; 3]>]) -> Vec<Vec<u8>> {
        let mut messages = Vec::new();
        let blocks_x = self.cols / 8;
        let blocks_y = self.rows / 8;
        match self.dialect {
            Dialect::Mext => {
                for by in 0..blocks_y {
                    for bx in 0..blocks_x {
                        let (x0, y0) = (bx * 8, by * 8);
                        let mut message = vec![MEXT_LEVEL_MAP, x0, y0];
                        for dy in 0..8u8 {
                            for pair in 0..4u8 {
                                let level = |dx: u8| {
                                    Self::color_at(colors, self.cols, x0 + dx, y0 + dy)
                                        .map_or(0, level_of)
                                        .min(MAX_LEVEL)
                                };
                                message.push(level(pair * 2) << 4 | level(pair * 2 + 1));
                            }
                        }
                        messages.push(message);
                    }
                }
            }
            Dialect::Series => {
                for by in 0..blocks_y {
                    for bx in 0..blocks_x {
                        let quadrant = bx + by * 2;
                        let mut message = vec![SERIES_FRAME | (quadrant & 0x03)];
                        message.extend_from_slice(&Self::block_bits(
                            colors,
                            self.cols,
                            bx * 8,
                            by * 8,
                        ));
                        messages.push(message);
                    }
                }
            }
            Dialect::FortyH => {
                let rows = Self::block_bits(colors, self.cols, 0, 0);
                for (y, bits) in (0u8..).zip(rows) {
                    messages.push(vec![FORTY_H_ROW | (y & 0x07), bits]);
                }
            }
        }
        for (ring, color) in (0..self.rings).zip(rings) {
            messages.push(vec![MEXT_RING_ALL, ring, color.map_or(0, level_of)]);
        }
        messages
    }

    fn ensure_started(&mut self) -> DriverResult<()> {
        if !self.started {
            self.started = true;
            self.flush()?;
        }
        Ok(())
    }

    fn parse(&mut self, events: &mut Vec<Input>) {
        match self.dialect {
            Dialect::Mext => {
                for message in take_mext(&mut self.pending) {
                    self.mext_message(&message, events);
                }
            }
            Dialect::Series | Dialect::FortyH => self.pairs(events),
        }
        if self.pending.len() > MAX_PENDING {
            self.pending.clear();
        }
    }

    fn key(&self, x: u8, y: u8, pressed: bool, events: &mut Vec<Input>) {
        if x < self.cols && y < self.rows {
            if let Ok(index) = u8::try_from(u16::from(y) * u16::from(self.cols) + u16::from(x)) {
                events.push(Input::Key { index, pressed });
            }
        }
    }

    fn mext_message(&mut self, message: &[u8], events: &mut Vec<Input>) {
        match message {
            [header @ (MEXT_KEY_UP | MEXT_KEY_DOWN), x, y] => {
                self.key(*x, *y, *header == MEXT_KEY_DOWN, events);
            }
            [MEXT_ENCODER_DELTA, ring, delta] => {
                let Some(turn) = self.turns.get_mut(usize::from(*ring)) else {
                    return;
                };
                *turn += i32::from(i8::from_le_bytes([*delta]));
                let steps = *turn / COUNTS_PER_STEP;
                if steps != 0 {
                    *turn -= steps * COUNTS_PER_STEP;
                    let delta = i8::try_from(steps.clamp(-127, 127)).unwrap_or(0);
                    events.push(Input::DialTurn {
                        index: *ring,
                        delta,
                    });
                }
            }
            [header @ (MEXT_ENCODER_UP | MEXT_ENCODER_DOWN), ring] if *ring < self.rings => {
                events.push(Input::DialPress {
                    index: *ring,
                    pressed: *header == MEXT_ENCODER_DOWN,
                });
            }
            _ => {}
        }
    }

    /// Series and 40h input: two bytes a message. A first byte that starts no
    /// message is skipped, so a dropped byte costs at most one message.
    fn pairs(&mut self, events: &mut Vec<Input>) {
        let mut at = 0;
        while let (Some(&command), Some(&place)) = (self.pending.get(at), self.pending.get(at + 1))
        {
            let (x, y) = (place >> 4, place & 0x0f);
            let known = match (self.dialect, command) {
                (Dialect::Series, SERIES_KEY_DOWN | SERIES_KEY_UP) => {
                    self.key(x, y, command == SERIES_KEY_DOWN, events);
                    true
                }
                // Tilt and aux input.
                (Dialect::Series, 0xd0 | 0xd1 | 0xe0..=0xef) => true,
                (Dialect::FortyH, FORTY_H_KEY_DOWN | FORTY_H_KEY_UP) => {
                    self.key(x, y, command == FORTY_H_KEY_DOWN, events);
                    true
                }
                // The ADC inputs (tilt).
                (Dialect::FortyH, 0x10..=0x17) => true,
                _ => false,
            };
            at += if known { 2 } else { 1 };
        }
        self.pending.drain(..at.min(self.pending.len()));
    }
}

impl Driver for Monome {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_started()?;
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_READS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.pending
                .extend_from_slice(buf.get(..read).unwrap_or_default());
            self.parse(&mut events);
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(slot) = self.colors.get_mut(usize::from(key)) {
            *slot = face.map(|face| face.color);
        }
        Ok(())
    }

    /// The whole ring, at the colour's brightness.
    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if let Some(slot) = self.ring_colors.get_mut(usize::from(dial)) {
            *slot = color;
        }
        Ok(())
    }

    /// Sends each block, row or ring whose message changed.
    fn flush(&mut self) -> DriverResult<()> {
        self.started = true;
        let messages = self.messages(&self.colors, &self.ring_colors);
        for (index, message) in messages.into_iter().enumerate() {
            if self.sent.get(index) == Some(&message) {
                continue;
            }
            self.transport.write(&message)?;
            match self.sent.get_mut(index) {
                Some(slot) => *slot = message,
                None => self.sent.push(message),
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        let intensity = intensity_of(percent);
        let message = match self.dialect {
            Dialect::Mext if self.rings > 0 => vec![MEXT_RING_INTENSITY, intensity, 0x00],
            Dialect::Mext => vec![MEXT_GRID_INTENSITY, intensity],
            Dialect::Series => vec![SERIES_INTENSITY | intensity],
            Dialect::FortyH => vec![FORTY_H_INTENSITY, intensity],
        };
        self.transport.write(&message)
    }
}

impl Drop for Monome {
    /// Leaves every light off when Ryu lets go.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        let dark_keys = vec![None; self.colors.len()];
        let dark_rings = vec![None; self.ring_colors.len()];
        for message in self.messages(&dark_keys, &dark_rings) {
            let _ = self.transport.write(&message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn open(id: &str) -> (Monome, Recorder) {
        let recorder = Recorder::default();
        (
            Monome::new(by_id(id).unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    fn face(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    /// A 0x1A level map of one dark 8 x 8 block.
    fn dark_map(x: u8, y: u8) -> Vec<u8> {
        let mut message = vec![0x1a, x, y];
        message.extend([0u8; 32]);
        message
    }

    #[test]
    fn serial_numbers_name_the_protocol_in_libmonome_s_order() {
        let series = |cols, rows| Some(SerialKind::Series { cols, rows });
        assert_eq!(classify_serial("m64-0001"), series(8, 8));
        assert_eq!(classify_serial("m64_0001"), series(8, 8));
        assert_eq!(classify_serial("m128-0042"), series(16, 8));
        assert_eq!(classify_serial("m256_0007"), series(16, 16));
        assert_eq!(classify_serial("mk12"), series(16, 16));
        assert_eq!(
            classify_serial("m40h0042"),
            Some(SerialKind::FortyH { arduinome: false })
        );
        assert_eq!(
            classify_serial("a40h-3"),
            Some(SerialKind::FortyH { arduinome: true })
        );
        assert_eq!(classify_serial("m1000123"), Some(SerialKind::Mext));
        assert_eq!(classify_serial("M1000123"), Some(SerialKind::Mext));
        // Windows adds an A to an FTDI serial.
        assert_eq!(classify_serial("m1000123A"), Some(SerialKind::Mext));
        for other in ["", "m", "mA", "monome", "A6008dLd", "m12x", "a40h7", "m64-"] {
            assert_eq!(classify_serial(other), None, "{other:?}");
        }
    }

    #[test]
    fn identify_never_claims_a_plain_ftdi_port_or_a_sixteen_by_sixteen_grid() {
        let id = |serial, manufacturer, product| {
            identify(serial, manufacturer, product).map(|model| model.id)
        };
        assert_eq!(id(Some("m64-0001"), None, None), Some(SERIES_64_ID));
        assert_eq!(id(Some("m128-0001"), None, None), Some(SERIES_128_ID));
        assert_eq!(id(Some("m40h0001"), None, None), Some(FORTY_H_ID));
        assert_eq!(id(Some("a40h_0001"), None, None), Some(ARDUINOME_ID));
        // mext gets a placeholder until its probe names the grid or arc.
        assert_eq!(id(Some("m1000123"), None, None), Some(GRID_128_ID));
        assert_eq!(
            id(Some("E6614103E7"), Some("monome"), Some("grid")),
            Some(GRID_128_ID)
        );
        assert_eq!(id(Some("m256-0001"), None, None), None);
        assert_eq!(id(Some("mk7"), None, None), None);
        assert_eq!(
            id(Some("A6008dLd"), Some("FTDI"), Some("FT232R USB UART")),
            None
        );
        assert_eq!(id(None, Some("monome"), Some("arc")), None);
        assert_eq!(by_id(ARDUINOME_ID).unwrap().serial_baud(), 57_600);
        assert_eq!(by_id(FORTY_H_ID).unwrap().serial_baud(), 115_200);
    }

    #[test]
    fn the_probe_names_a_grid_by_its_size() {
        let mut recorder = Recorder::default();
        // Replies split anywhere across reads.
        recorder.queue_read(vec![0x00, 0x01]);
        recorder.queue_read(vec![0x01, 0x00, 0x02, 0x01, 0x03]);
        recorder.queue_read(vec![0x10, 0x08]);
        let MextAnswer::Model(model) = probe(&mut recorder).unwrap() else {
            panic!("the grid was not named");
        };
        assert_eq!(model.id, GRID_128_ID);
        assert_eq!(recorder.writes(), vec![vec![0x00], vec![0x05]]);

        let mut recorder = Recorder::default();
        recorder.queue_read(vec![0x00, 0x01, 0x01, 0x03, 0x08, 0x08]);
        assert_eq!(probe_model(&mut recorder).id, GRID_64_ID);
    }

    #[test]
    fn the_probe_names_an_arc_by_its_encoders() {
        let mut recorder = Recorder::default();
        recorder.queue_read(vec![0x00, 0x05, 0x04, 0x03, 0x00, 0x00]);
        assert_eq!(probe_model(&mut recorder).id, ARC_4_ID);

        // An arc that never gives its size is still asked only a few times.
        let mut recorder = Recorder::default();
        recorder.queue_read(vec![0x00, 0x05, 0x02]);
        assert_eq!(probe_model(&mut recorder).id, ARC_2_ID);
        assert_eq!(
            recorder.writes(),
            vec![vec![0x00], vec![0x05], vec![0x05], vec![0x05], vec![0x05]]
        );
    }

    #[test]
    fn the_probe_tells_silence_from_sixteen_by_sixteen_grids() {
        let mut recorder = Recorder::default();
        assert!(matches!(probe(&mut recorder).unwrap(), MextAnswer::Silent));
        assert_eq!(recorder.writes().len(), 2 * PROBE_ROUNDS);

        // A grid that answered is not asked again, even though Ryu does not
        // drive it.
        let mut recorder = Recorder::default();
        recorder.queue_read(vec![0x00, 0x01, 0x04, 0x03, 0x10, 0x10]);
        assert!(matches!(
            probe(&mut recorder).unwrap(),
            MextAnswer::Undriven
        ));
    }

    fn probe_model(recorder: &mut Recorder) -> &'static DeviceModel {
        match probe(recorder).unwrap() {
            MextAnswer::Model(model) => model,
            answer => panic!("no model: {answer:?}"),
        }
    }

    #[test]
    fn the_first_poll_darkens_every_block() {
        let (mut grid, recorder) = open(GRID_128_ID);
        assert!(grid.poll(0).unwrap().is_empty());
        assert_eq!(recorder.writes(), vec![dark_map(0, 0), dark_map(8, 0)]);
        // Nothing changed, so nothing more is sent.
        grid.flush().unwrap();
        assert_eq!(recorder.writes().len(), 2);
    }

    #[test]
    fn mext_keys_are_numbered_row_by_row() {
        let (mut grid, recorder) = open(GRID_128_ID);
        recorder.queue_read(vec![0x21, 3, 1, 0x20]);
        recorder.queue_read(vec![15, 7]);
        // Off the grid, then an unknown byte, then a key.
        recorder.queue_read(vec![0x21, 16, 0, 0xff, 0x21, 0, 0]);
        assert_eq!(
            grid.poll(0).unwrap(),
            vec![key(19, true), key(127, false), key(0, true)]
        );
    }

    #[test]
    fn mext_lights_are_sixteen_levels_packed_high_nibble_first() {
        let (mut grid, recorder) = open(GRID_128_ID);
        grid.flush().unwrap();
        grid.set_face(0, Some(&face([255, 255, 255]))).unwrap();
        grid.set_face(1, Some(&face([0x80, 0, 0]))).unwrap();
        grid.set_face(16, Some(&face([0, 0, 255]))).unwrap();
        grid.set_face(17, Some(&face([0, 0, 0]))).unwrap();
        grid.flush().unwrap();
        let mut expected = vec![0x1a, 0, 0, 0xf8, 0, 0, 0, 0xf0];
        expected.extend([0u8; 27]);
        // Only the left block changed.
        assert_eq!(recorder.writes().get(2..), Some(&[expected][..]));

        grid.set_face(9, Some(&face([0, 255, 0]))).unwrap();
        grid.flush().unwrap();
        let mut right = vec![0x1a, 8, 0, 0x0f];
        right.extend([0u8; 31]);
        assert_eq!(recorder.writes().last(), Some(&right));
    }

    #[test]
    fn brightness_is_the_grid_or_ring_intensity() {
        let cases = [
            (GRID_64_ID, vec![0x17, 8]),
            (ARC_4_ID, vec![0x94, 8, 0x00]),
            (SERIES_64_ID, vec![0xa8]),
            (FORTY_H_ID, vec![0x30, 8]),
        ];
        for (id, message) in cases {
            let (mut deck, recorder) = open(id);
            deck.set_brightness(50).unwrap();
            assert_eq!(recorder.writes(), vec![message], "{id}");
        }
        let (mut grid, recorder) = open(GRID_64_ID);
        grid.set_brightness(100).unwrap();
        grid.set_brightness(0).unwrap();
        assert_eq!(recorder.writes(), vec![vec![0x17, 15], vec![0x17, 0]]);
    }

    #[test]
    fn arc_encoders_turn_a_dial_step_every_sixteen_counts() {
        let (mut arc, recorder) = open(ARC_4_ID);
        recorder.queue_read(vec![0x50, 0, 10, 0x50, 0, 6]);
        recorder.queue_read(vec![0x50, 1, 0xe0, 0x52, 0, 0x51, 0]);
        // A fifth encoder does not exist.
        recorder.queue_read(vec![0x50, 4, 0x7f, 0x52, 4]);
        assert_eq!(
            arc.poll(0).unwrap(),
            vec![
                Input::DialTurn { index: 0, delta: 1 },
                Input::DialTurn {
                    index: 1,
                    delta: -2
                },
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                Input::DialPress {
                    index: 0,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn an_arc_lights_whole_rings() {
        let (mut arc, recorder) = open(ARC_2_ID);
        arc.flush().unwrap();
        assert_eq!(recorder.writes(), vec![vec![0x91, 0, 0], vec![0x91, 1, 0]]);
        arc.set_dial_light(1, Some([255, 128, 0])).unwrap();
        arc.set_dial_light(7, Some([255, 255, 255])).unwrap();
        arc.flush().unwrap();
        assert_eq!(recorder.writes().last(), Some(&vec![0x91, 1, 15]));
        assert_eq!(recorder.writes().len(), 3);
        drop(arc);
        assert_eq!(recorder.writes().last(), Some(&vec![0x91, 1, 0]));
    }

    #[test]
    fn series_keys_and_frames() {
        let (mut grid, recorder) = open(SERIES_64_ID);
        // x in the high nibble; a stray byte is skipped; tilt is ignored.
        recorder.queue_read(vec![0x00, 0x37, 0x10, 0x37, 0x55, 0xd0, 0x12, 0x00, 0x00]);
        assert_eq!(
            grid.poll(0).unwrap(),
            vec![key(59, true), key(59, false), key(0, true)]
        );
        // The first poll sent the dark frame.
        assert_eq!(recorder.writes(), vec![vec![0x80, 0, 0, 0, 0, 0, 0, 0, 0]]);
        grid.set_face(0, Some(&face([255, 0, 0]))).unwrap();
        grid.set_face(9, Some(&face([10, 10, 10]))).unwrap();
        grid.set_face(63, Some(&face([0, 40, 0]))).unwrap();
        grid.flush().unwrap();
        assert_eq!(
            recorder.writes().last(),
            Some(&vec![0x80, 0x01, 0, 0, 0, 0, 0, 0, 0x80])
        );

        let (mut wide, recorder) = open(SERIES_128_ID);
        wide.set_face(8, Some(&face([255, 255, 255]))).unwrap();
        wide.set_face(127, Some(&face([255, 255, 255]))).unwrap();
        wide.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x80, 0, 0, 0, 0, 0, 0, 0, 0],
                vec![0x81, 0x01, 0, 0, 0, 0, 0, 0, 0x80],
            ]
        );
    }

    #[test]
    fn forty_h_keys_and_rows() {
        let (mut grid, recorder) = open(FORTY_H_ID);
        recorder.queue_read(vec![0x01, 0x21, 0x12, 0x34, 0x00, 0x21]);
        assert_eq!(grid.poll(0).unwrap(), vec![key(10, true), key(10, false)]);
        let dark: Vec<Vec<u8>> = (0..8u8).map(|y| vec![0x70 | y, 0]).collect();
        assert_eq!(recorder.writes(), dark);
        grid.set_face(10, Some(&face([0, 0, 200]))).unwrap();
        grid.flush().unwrap();
        assert_eq!(recorder.writes().get(8..), Some(&[vec![0x71, 0x04]][..]));
    }

    #[test]
    fn dropping_a_lit_grid_darkens_it() {
        let (mut grid, recorder) = open(GRID_64_ID);
        grid.set_face(5, Some(&face([255, 255, 255]))).unwrap();
        grid.flush().unwrap();
        drop(grid);
        assert_eq!(recorder.writes().last(), Some(&dark_map(0, 0)));
        // A grid that was never used is left alone.
        let (grid, recorder) = open(GRID_64_ID);
        drop(grid);
        assert!(recorder.writes().is_empty());
    }
}
