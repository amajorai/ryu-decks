//! TourBox Neo and Elite / Elite Plus: eleven buttons and three rotary
//! controls on a USB serial port, with no screens or lights.
//!
//! The TourBox stays silent until it is unlocked: the host sends an 8-byte
//! unlock, reads back a reply of about 26 bytes, then sends a 94-byte config
//! that also sets the Elite's haptics (Ryu leaves them all off). The reply and
//! anything else the device says during setup is drained and dropped, since its
//! runs of zero bytes would otherwise read as presses.
//!
//! After that every input is one byte, and a read may hold several. A button
//! press is its id with bit 7 clear and its release the same id with bit 7 set.
//! A rotary control sends two bytes per detent: one with bit 7 clear, then the
//! same with bit 7 set. Ryu counts the first and ignores the second; bit 6 of
//! either is the direction (set for clockwise, or up on the scroll wheel). The
//! knob, scroll wheel and dial are dials 0, 1 and 2, and pushing one is that
//! dial's press. Nothing is ever drawn and no keep-alive is needed.

use std::time::{Duration, Instant};

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

/// Sent first; without it the TourBox reports nothing.
pub const UNLOCK: [u8; 8] = [0x55, 0x00, 0x07, 0x88, 0x94, 0x00, 0x1a, 0xfe];

/// The config's slot ids, one at each odd offset from 3 to 91. The byte after
/// each is that slot's haptic strength and detent speed; 0 is off.
const CONFIG_SLOTS: [u8; 45] = [
    0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x26, 0x27, 0x28, 0x29, 0x3b,
    0x3c, 0x3d, 0x3e, 0x3f, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x4b,
    0x4c, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x52, 0x53, 0x54, 0xa8, 0xa9, 0xaa, 0xab,
];

/// The config with every haptic off: a 3-byte header (0x5d is the length that
/// follows), the slot pairs, then 0xfe.
pub const CONFIG: [u8; 94] = config();

const fn config() -> [u8; 94] {
    let mut message = [0u8; 94];
    message[0] = 0xb5;
    message[1] = 0x00;
    message[2] = 0x5d;
    let mut slot = 0;
    while slot < CONFIG_SLOTS.len() {
        message[3 + slot * 2] = CONFIG_SLOTS[slot];
        slot += 1;
    }
    message[93] = 0xfe;
    message
}

/// A TourBox answers the unlock with at least this many bytes.
const MIN_UNLOCK_REPLY: usize = 20;
/// How long the unlock reply is given to arrive.
const UNLOCK_WAIT: Duration = Duration::from_millis(300);
/// The pause after the config before input is read.
const CONFIG_SETTLE: Duration = Duration::from_millis(50);
/// Anything still arriving this soon after the config is dropped too.
const CONFIG_DRAIN: Duration = Duration::from_millis(100);
/// A read this long holding a byte that is never input is a reply, not input.
const REPLY_LEN: usize = 20;
const READ_BUFFER: usize = 64;
const MAX_READS_PER_POLL: usize = 16;

/// The printed name of each key, in key order.
pub const KEY_NAMES: [&str; 11] = [
    "Tall", "Side", "Top", "Short", "C1", "C2", "Tour", "Up", "Down", "Left", "Right",
];

/// The press id of each key, in key order.
const KEY_IDS: [u8; 11] = [
    0x00, 0x01, 0x02, 0x03, 0x22, 0x23, 0x2a, 0x10, 0x11, 0x12, 0x13,
];
/// The press id of the knob, scroll wheel and dial.
const DIAL_PRESS_IDS: [u8; 3] = [0x37, 0x0a, 0x38];
/// The turn id (bits 0 to 5) of the knob, scroll wheel and dial.
const DIAL_TURN_IDS: [u8; 3] = [0x04, 0x09, 0x0f];

const RELEASE: u8 = 0x80;
const CLOCKWISE: u8 = 0x40;
const ID: u8 = 0x3f;

/// What one input byte means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Code {
    Key {
        index: u8,
        pressed: bool,
    },
    DialPress {
        index: u8,
        pressed: bool,
    },
    /// The first byte of a detent.
    Turn {
        index: u8,
        clockwise: bool,
    },
    /// The second byte of a detent: counted already.
    TurnEnd,
}

fn decode(byte: u8) -> Option<Code> {
    let id = byte & ID;
    if let Some(index) = DIAL_TURN_IDS.iter().position(|turn| *turn == id) {
        let index = index as u8;
        if byte & RELEASE != 0 {
            return Some(Code::TurnEnd);
        }
        return Some(Code::Turn {
            index,
            clockwise: byte & CLOCKWISE != 0,
        });
    }
    // Only the rotary controls set bit 6.
    if byte & CLOCKWISE != 0 {
        return None;
    }
    let pressed = byte & RELEASE == 0;
    if let Some(index) = KEY_IDS.iter().position(|key| *key == id) {
        return Some(Code::Key {
            index: index as u8,
            pressed,
        });
    }
    DIAL_PRESS_IDS
        .iter()
        .position(|dial| *dial == id)
        .map(|index| Code::DialPress {
            index: index as u8,
            pressed,
        })
}

/// Reads and drops whatever arrives within `window`, returning how many bytes
/// that was. Stops early once a read times out with nothing.
fn drain(transport: &mut dyn Transport, window: Duration) -> DriverResult<usize> {
    let deadline = Instant::now() + window;
    let mut buf = [0u8; READ_BUFFER];
    let mut total = 0;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(total);
        }
        let wait = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
        let read = transport.read_timeout(&mut buf, wait)?;
        if read == 0 {
            return Ok(total);
        }
        total += read;
    }
}

/// Sends the unlock and reports whether a TourBox answered it. Used before
/// claiming a port on a generic USB id that other boards share.
pub fn probe(transport: &mut dyn Transport) -> DriverResult<bool> {
    transport.write(&UNLOCK)?;
    Ok(drain(transport, UNLOCK_WAIT)? >= MIN_UNLOCK_REPLY)
}

pub struct TourBox {
    transport: Box<dyn Transport>,
    started: bool,
    keys: [bool; KEY_IDS.len()],
    dials: [bool; DIAL_PRESS_IDS.len()],
}

impl TourBox {
    #[must_use]
    pub fn new(_model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            keys: [false; KEY_IDS.len()],
            dials: [false; DIAL_PRESS_IDS.len()],
        }
    }

    /// Unlock, drop the reply, send the all-off config and drop anything after it.
    fn start(&mut self) -> DriverResult<()> {
        self.started = true;
        self.transport.write(&UNLOCK)?;
        drain(self.transport.as_mut(), UNLOCK_WAIT)?;
        self.transport.write(&CONFIG)?;
        std::thread::sleep(CONFIG_SETTLE);
        drain(self.transport.as_mut(), CONFIG_DRAIN)?;
        Ok(())
    }

    fn parse(&mut self, bytes: &[u8], events: &mut Vec<Input>) {
        // A reply that came late: it holds bytes no control sends, and its
        // zero runs would read as presses of Tall.
        if bytes.len() >= REPLY_LEN && bytes.iter().any(|byte| decode(*byte).is_none()) {
            return;
        }
        for byte in bytes {
            match decode(*byte) {
                Some(Code::Key { index, pressed }) => {
                    if let Some(held) = self.keys.get_mut(usize::from(index)) {
                        if *held != pressed {
                            *held = pressed;
                            events.push(Input::Key { index, pressed });
                        }
                    }
                }
                Some(Code::DialPress { index, pressed }) => {
                    if let Some(held) = self.dials.get_mut(usize::from(index)) {
                        if *held != pressed {
                            *held = pressed;
                            events.push(Input::DialPress { index, pressed });
                        }
                    }
                }
                Some(Code::Turn { index, clockwise }) => {
                    let step: i8 = if clockwise { 1 } else { -1 };
                    push_turn(events, index, step);
                }
                Some(Code::TurnEnd) | None => {}
            }
        }
    }
}

/// Adds a detent, folding it into the previous event when that is the same
/// dial turning the same way.
fn push_turn(events: &mut Vec<Input>, index: u8, step: i8) {
    if let Some(Input::DialTurn { index: last, delta }) = events.last_mut() {
        if *last == index && delta.signum() == step && delta.checked_add(step).is_some() {
            *delta += step;
            return;
        }
    }
    events.push(Input::DialTurn { index, delta: step });
}

impl Driver for TourBox {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
            self.start()?;
        }
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_READS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.parse(buf.get(..read).unwrap_or_default(), &mut events);
        }
        Ok(events)
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
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    const UNLOCK_REPLY: [u8; 26] = [
        0x07, 0xca, 0x31, 0x78, 0x1b, 0xb2, 0xb5, 0xdb, 0x1d, 0xd5, 0x6c, 0x07, 0xf0, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x03, 0x02, 0x02, 0x00, 0x00,
    ];

    fn hex(text: &str) -> Vec<u8> {
        text.split_whitespace()
            .map(|pair| u8::from_str_radix(pair, 16).unwrap())
            .collect()
    }

    /// A started driver, with the setup writes cleared from the recorder.
    fn started(model: &str) -> (TourBox, Recorder) {
        let recorder = Recorder::default();
        let mut deck = TourBox::new(by_id(model).unwrap(), Box::new(recorder.clone()));
        assert!(deck.poll(0).unwrap().is_empty());
        recorder.0.lock().unwrap().writes.clear();
        (deck, recorder)
    }

    #[test]
    fn the_config_matches_the_documented_all_off_message() {
        let expected = hex(
            "B5 00 5D 04 00 05 00 06 00 07 00 08 00 09 00 0B 00 0C 00 0D 00 0E 00 0F 00 \
             26 00 27 00 28 00 29 00 3B 00 3C 00 3D 00 3E 00 3F 00 40 00 41 00 42 00 \
             43 00 44 00 45 00 46 00 47 00 48 00 49 00 4A 00 4B 00 4C 00 4D 00 4E 00 \
             4F 00 50 00 51 00 52 00 53 00 54 00 A8 00 A9 00 AA 00 AB 00 FE",
        );
        assert_eq!(CONFIG.to_vec(), expected);
        assert_eq!(UNLOCK.to_vec(), hex("55 00 07 88 94 00 1A FE"));
    }

    #[test]
    fn setup_unlocks_configures_and_drops_the_reply() {
        let recorder = Recorder::default();
        let mut deck = TourBox::new(by_id("tourbox-elite").unwrap(), Box::new(recorder.clone()));
        // The reply split across two reads; its zero runs must not press Tall.
        recorder.queue_read(UNLOCK_REPLY[..10].to_vec());
        recorder.queue_read(UNLOCK_REPLY[10..].to_vec());
        assert!(deck.poll(0).unwrap().is_empty());
        assert_eq!(recorder.writes(), vec![UNLOCK.to_vec(), CONFIG.to_vec()]);
        // Setup happens once.
        recorder.queue_read(vec![0x02, 0x82]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 2,
                    pressed: true
                },
                Input::Key {
                    index: 2,
                    pressed: false
                },
            ]
        );
        assert_eq!(recorder.writes().len(), 2);
    }

    #[test]
    fn every_button_maps_to_its_key_or_dial() {
        let (mut deck, recorder) = started("tourbox-neo");
        let presses = [
            0x00, 0x01, 0x02, 0x03, 0x22, 0x23, 0x2a, 0x10, 0x11, 0x12, 0x13, 0x37, 0x0a, 0x38,
        ];
        let releases: Vec<u8> = presses.iter().map(|id| id | 0x80).collect();
        recorder.queue_read(presses.to_vec());
        recorder.queue_read(releases);
        let events = deck.poll(0).unwrap();
        let mut expected = Vec::new();
        for pressed in [true, false] {
            expected.extend((0..11).map(|index| Input::Key { index, pressed }));
            expected.extend((0..3).map(|index| Input::DialPress { index, pressed }));
        }
        assert_eq!(events, expected);
        assert!(recorder.writes().is_empty(), "nothing to draw or light");
    }

    #[test]
    fn each_detent_counts_once_in_both_directions() {
        let (mut deck, recorder) = started("tourbox-elite");
        // Knob: two clockwise detents, one pair split across reads.
        recorder.queue_read(vec![0x44, 0xc4, 0x44]);
        recorder.queue_read(vec![0xc4]);
        // Knob counter-clockwise, scroll up then down, dial both ways.
        recorder.queue_read(vec![
            0x04, 0x84, 0x49, 0xc9, 0x09, 0x89, 0x09, 0x89, 0x4f, 0xcf, 0x0f, 0x8f,
        ]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialTurn { index: 0, delta: 2 },
                Input::DialTurn {
                    index: 0,
                    delta: -1
                },
                Input::DialTurn { index: 1, delta: 1 },
                Input::DialTurn {
                    index: 1,
                    delta: -2
                },
                Input::DialTurn { index: 2, delta: 1 },
                Input::DialTurn {
                    index: 2,
                    delta: -1
                },
            ]
        );
    }

    #[test]
    fn a_fast_spin_is_not_mistaken_for_a_reply() {
        let (mut deck, recorder) = started("tourbox-neo");
        let spin: Vec<u8> = std::iter::repeat_n([0x4f, 0xcf], 20).flatten().collect();
        recorder.queue_read(spin);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::DialTurn {
                index: 2,
                delta: 20
            }]
        );
    }

    #[test]
    fn stray_bytes_and_unmatched_releases_are_ignored() {
        let (mut deck, recorder) = started("tourbox-neo");
        // A late reply is dropped whole.
        recorder.queue_read(UNLOCK_REPLY.to_vec());
        // A release with no press, unknown ids, a repeated press, then a real one.
        recorder.queue_read(vec![0x80, 0x3c, 0x7e, 0xff, 0x03, 0x03]);
        recorder.queue_read(vec![]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 3,
                pressed: true
            }]
        );
    }

    #[test]
    fn the_probe_needs_a_full_reply() {
        let mut recorder = Recorder::default();
        recorder.queue_read(UNLOCK_REPLY[..12].to_vec());
        recorder.queue_read(UNLOCK_REPLY[12..].to_vec());
        assert!(probe(&mut recorder).unwrap());
        assert_eq!(recorder.writes(), vec![UNLOCK.to_vec()]);

        let mut silent = Recorder::default();
        assert!(!probe(&mut silent).unwrap());
        let mut short = Recorder::default();
        short.queue_read(vec![0x07, 0x00, 0x00]);
        assert!(!probe(&mut short).unwrap());
    }
}
