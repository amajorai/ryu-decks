//! Loupedeck Live, Live S and CT, and the Razer Stream Controllers built on them.
//!
//! These are USB serial devices that speak WebSocket framing after an HTTP-style
//! upgrade. A message is `[length, command, transaction id, data..]`. Keys are
//! areas of one touchscreen: a face is an RGB565 rectangle written into the
//! framebuffer and then drawn. Touch keys report coordinates, so presses are
//! mapped back to a key by position; the round buttons, knobs and the Stream
//! Controller X's keys report button ids. The Live's and CT's side strips are
//! strip keys: three segments a side, one beside each knob. The CT's wheel has
//! a round screen of its own, bound as one more key.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{face, Driver, DriverError, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const UPGRADE_REQUEST: &[u8] = b"GET /index.html\nHTTP/1.1\nConnection: Upgrade\nUpgrade: websocket\nSec-WebSocket-Key: 123abc\n\n";
const HANDSHAKE_TRIES: usize = 20;
const HANDSHAKE_WAIT_MS: i32 = 100;
const READ_BURST: usize = 16;
/// A misbehaving device must not grow the inbox without bound.
const MAX_INBOX: usize = 64 * 1024;

const BUTTON_PRESS: u8 = 0x00;
const KNOB_ROTATE: u8 = 0x01;
const SET_COLOR: u8 = 0x02;
const SET_BRIGHTNESS: u8 = 0x09;
const FRAMEBUFFER: u8 = 0x10;
const DRAW: u8 = 0x0f;
const TOUCH: u8 = 0x4d;
const TOUCH_END: u8 = 0x6d;
/// Brightness runs 0-10 on the wire.
const MAX_BRIGHTNESS: u16 = 10;
/// The CT draws its side strips on displays of their own.
const CT_LEFT_STRIP: [u8; 2] = [0x00, b'L'];
const CT_RIGHT_STRIP: [u8; 2] = [0x00, b'R'];
/// The CT wheel's round screen; its touches carry touch id 0.
const CT_WHEEL: [u8; 2] = [0x00, b'W'];
const WHEEL_TOUCH_ID: u8 = 0;

/// The CT's twelve labelled buttons, in the order they follow the round ones:
/// Home, Enter, Undo, Save, Keyboard, Fn (left), A, B, C, D, Fn (right), E.
const CT_LABELLED_BUTTONS: [u8; 12] = [
    0x0f, 0x12, 0x10, 0x13, 0x11, 0x14, 0x15, 0x18, 0x16, 0x19, 0x17, 0x1a,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Live,
    LiveS,
    Ct,
    StreamControllerX,
}

/// Where a model's keys sit on its screen.
#[derive(Debug, Clone, Copy)]
struct Screen {
    display: [u8; 2],
    /// Left edge of the key grid when drawing.
    draw_x: u16,
    /// Left edge of the key grid in touch coordinates (the CT draws its keys on
    /// a separate centre display but reports touches across all three).
    touch_x: u16,
    key_size: u16,
    cols: u8,
    /// Touches map to keys; on the Stream Controller X the keys are buttons.
    touch_keys: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Key(u8),
    Dial(u8),
}

pub struct Loupedeck {
    model: &'static DeviceModel,
    kind: Kind,
    screen: Screen,
    transport: Box<dyn Transport>,
    upgraded: bool,
    inbox: Vec<u8>,
    transaction: u8,
    frames: BTreeMap<u8, Vec<u8>>,
    lights: BTreeMap<u8, [u8; 3]>,
    /// Touch id -> the key it went down on, so the lift releases that key.
    touches: HashMap<u8, u8>,
}

impl Loupedeck {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = match model.id {
            "loupedeck-live-s" => Kind::LiveS,
            "loupedeck-ct" => Kind::Ct,
            "razer-stream-controller-x" => Kind::StreamControllerX,
            _ => Kind::Live,
        };
        let key_size = model.image.map_or(90, |image| image.width) as u16;
        let screen = match kind {
            Kind::Live => Screen {
                display: [0x00, b'M'],
                draw_x: 60,
                touch_x: 60,
                key_size,
                cols: model.cols,
                touch_keys: true,
            },
            Kind::LiveS => Screen {
                display: [0x00, b'M'],
                draw_x: 15,
                touch_x: 15,
                key_size,
                cols: model.cols,
                touch_keys: true,
            },
            Kind::Ct => Screen {
                display: [0x00, b'A'],
                draw_x: 0,
                touch_x: 60,
                key_size,
                cols: model.cols,
                touch_keys: true,
            },
            Kind::StreamControllerX => Screen {
                display: [0x00, b'M'],
                draw_x: 0,
                touch_x: 0,
                key_size,
                cols: model.cols,
                touch_keys: false,
            },
        };
        Self {
            model,
            kind,
            screen,
            transport,
            upgraded: false,
            inbox: Vec::new(),
            transaction: 0,
            frames: BTreeMap::new(),
            lights: BTreeMap::new(),
            touches: HashMap::new(),
        }
    }

    /// Switch the port into WebSocket framing. Anything the device sends after
    /// its HTTP answer stays in the inbox.
    fn upgrade(&mut self) -> DriverResult<()> {
        if self.upgraded {
            return Ok(());
        }
        self.transport.write(UPGRADE_REQUEST)?;
        let mut buf = [0u8; 1024];
        for _ in 0..HANDSHAKE_TRIES {
            let len = self.transport.read_timeout(&mut buf, HANDSHAKE_WAIT_MS)?;
            self.inbox.extend_from_slice(&buf[..len]);
            if let Some(end) = find(&self.inbox, b"\r\n\r\n") {
                if !self.inbox.starts_with(b"HTTP/1.1") {
                    self.inbox.clear();
                    return Err(DriverError::Io(
                        "the deck did not accept the connection".to_owned(),
                    ));
                }
                self.inbox.drain(..end + 4);
                self.upgraded = true;
                return Ok(());
            }
        }
        self.inbox.clear();
        Err(DriverError::Io("the deck did not answer".to_owned()))
    }

    fn send(&mut self, command: u8, data: &[u8]) -> DriverResult<()> {
        self.transaction = self.transaction.wrapping_add(1).max(1);
        let mut message = Vec::with_capacity(3 + data.len());
        message.push(u8::try_from(3 + data.len()).unwrap_or(u8::MAX));
        message.push(command);
        message.push(self.transaction);
        message.extend_from_slice(data);
        self.transport.write(&frame(&message))
    }

    fn button_target(&self, id: u8) -> Option<Target> {
        let screen_keys = self.model.screen_keys();
        match (self.kind, id) {
            (Kind::StreamControllerX, 0x1b..=0x29) => Some(Target::Key(id - 0x1b)),
            (Kind::StreamControllerX, _) => None,
            (Kind::Ct, 0x00) => Some(Target::Dial(6)),
            (Kind::Live | Kind::Ct, 0x01..=0x06) => Some(Target::Dial(id - 1)),
            (Kind::LiveS, 0x01 | 0x02) => Some(Target::Dial(id - 1)),
            (_, 0x07..=0x0e) => {
                let key = screen_keys + (id - 0x07);
                (key < self.model.keys).then_some(Target::Key(key))
            }
            (Kind::Ct, _) => CT_LABELLED_BUTTONS
                .iter()
                .position(|button| *button == id)
                .map(|offset| Target::Key(screen_keys + 8 + offset as u8)),
            _ => None,
        }
    }

    /// The button id of a lit key, for `SET_COLOR`.
    fn light_id(&self, key: u8) -> Option<u8> {
        let offset = key.checked_sub(self.model.screen_keys())?;
        match self.kind {
            Kind::StreamControllerX => None,
            Kind::Ct if offset >= 8 => CT_LABELLED_BUTTONS.get(usize::from(offset - 8)).copied(),
            _ => (key < self.model.keys).then_some(0x07 + offset),
        }
    }

    /// Which side strip `x` falls on (0 left, 1 right), on decks that have them.
    fn side_at(&self, x: u16) -> Option<u8> {
        if self.model.strip_keys == 0 {
            return None;
        }
        let screen = self.screen;
        let grid_end = screen.touch_x + u16::from(screen.cols) * screen.key_size;
        if x < screen.touch_x {
            Some(0)
        } else if x >= grid_end {
            Some(1)
        } else {
            None
        }
    }

    /// Strip segments per side, and each segment's size.
    fn strip_geometry(&self) -> Option<(u8, u16, u16)> {
        let format = self.model.strip_image?;
        let per_side = self.model.strip_keys / 2;
        let width = u16::try_from(format.width).ok()?;
        let height = u16::try_from(format.height).ok()?;
        (per_side > 0).then_some((per_side, width, height))
    }

    fn touched_key(&self, x: u16, y: u16) -> Option<u8> {
        let screen = self.screen;
        if let Some(side) = self.side_at(x) {
            let (per_side, _, height) = self.strip_geometry()?;
            let row = u8::try_from(y / height).ok()?.min(per_side - 1);
            return Some(self.model.keys + side * per_side + row);
        }
        let column = x.checked_sub(screen.touch_x)? / screen.key_size;
        let row = y / screen.key_size;
        if column >= u16::from(screen.cols) {
            return None;
        }
        let key = row * u16::from(screen.cols) + column;
        u8::try_from(key)
            .ok()
            .filter(|key| *key < self.model.screen_keys())
    }

    /// Where a key's face is drawn: the display, then x, y, width and height.
    fn region(&self, key: u8) -> Option<([u8; 2], [u16; 4])> {
        let screen = self.screen;
        if self.model.wheel_key() == Some(key) {
            let format = self.model.wheel_image?;
            let width = u16::try_from(format.width).ok()?;
            let height = u16::try_from(format.height).ok()?;
            return Some((CT_WHEEL, [0, 0, width, height]));
        }
        if let Some(segment) = self.model.strip_segment(key) {
            let (per_side, width, height) = self.strip_geometry()?;
            let (side, row) = (segment / per_side, segment % per_side);
            let y = u16::from(row) * height;
            return Some(match (self.kind, side) {
                (Kind::Ct, 0) => (CT_LEFT_STRIP, [0, y, width, height]),
                (Kind::Ct, _) => (CT_RIGHT_STRIP, [0, y, width, height]),
                // The Live draws its strips on the same display as the keys,
                // either side of the grid.
                (_, 0) => (screen.display, [0, y, width, height]),
                (_, _) => {
                    let x = screen.draw_x + u16::from(screen.cols) * screen.key_size;
                    (screen.display, [x, y, width, height])
                }
            });
        }
        let size = screen.key_size;
        let column = u16::from(key % screen.cols);
        let row = u16::from(key / screen.cols);
        Some((
            screen.display,
            [screen.draw_x + column * size, row * size, size, size],
        ))
    }

    fn handle(&mut self, message: &[u8]) -> Option<Input> {
        let (&command, rest) = message.get(1..)?.split_first()?;
        let data = rest.get(1..)?;
        match command {
            BUTTON_PRESS => {
                let pressed = *data.get(1)? == 0x00;
                match self.button_target(*data.first()?)? {
                    Target::Key(index) => Some(Input::Key { index, pressed }),
                    Target::Dial(index) => Some(Input::DialPress { index, pressed }),
                }
            }
            KNOB_ROTATE => match self.button_target(*data.first()?)? {
                Target::Dial(index) => Some(Input::DialTurn {
                    index,
                    delta: i8::from_be_bytes([*data.get(1)?]),
                }),
                Target::Key(_) => None,
            },
            TOUCH | TOUCH_END if self.screen.touch_keys => {
                let x = u16::from_be_bytes([*data.get(1)?, *data.get(2)?]);
                let y = u16::from_be_bytes([*data.get(3)?, *data.get(4)?]);
                let id = *data.get(5)?;
                if command == TOUCH_END {
                    let index = self.touches.remove(&id)?;
                    return Some(Input::Key {
                        index,
                        pressed: false,
                    });
                }
                if self.touches.contains_key(&id) {
                    // A finger sliding across the screen is still the same press.
                    return None;
                }
                // On the CT, touch id 0 is the wheel's own screen, wherever on
                // it the finger lands.
                let index = if self.kind == Kind::Ct && id == WHEEL_TOUCH_ID {
                    self.model.wheel_key()?
                } else {
                    self.touched_key(x, y)?
                };
                self.touches.insert(id, index);
                Some(Input::Key {
                    index,
                    pressed: true,
                })
            }
            _ => None,
        }
    }

    /// Pull every complete frame out of the inbox.
    fn drain_messages(&mut self) -> Vec<Vec<u8>> {
        let mut messages = Vec::new();
        loop {
            let Some(start) = self.inbox.iter().position(|byte| *byte == 0x82) else {
                self.inbox.clear();
                break;
            };
            let Some(&len) = self.inbox.get(start + 1) else {
                self.inbox.drain(..start);
                break;
            };
            let end = start + 2 + usize::from(len);
            if self.inbox.len() < end {
                self.inbox.drain(..start);
                break;
            }
            messages.push(self.inbox[start + 2..end].to_vec());
            self.inbox.drain(..end);
        }
        messages
    }
}

/// Wrap a message in a masked binary WebSocket frame with a zero mask.
fn frame(message: &[u8]) -> Vec<u8> {
    let len = message.len();
    let mut out = Vec::with_capacity(14 + len);
    out.push(0x82);
    if len <= 125 {
        out.push(0x80 | len as u8);
    } else {
        out.push(0xff);
        out.extend_from_slice(&(len as u64).to_be_bytes());
    }
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(message);
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

impl Driver for Loupedeck {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.upgrade()?;
        let mut buf = [0u8; 4096];
        let mut wait = timeout_ms;
        for _ in 0..READ_BURST {
            let len = self.transport.read_timeout(&mut buf, wait)?;
            if len == 0 {
                break;
            }
            self.inbox.extend_from_slice(&buf[..len]);
            wait = 0;
        }
        if self.inbox.len() > MAX_INBOX {
            self.inbox.clear();
        }
        let messages = self.drain_messages();
        Ok(messages
            .iter()
            .filter_map(|message| self.handle(message))
            .collect())
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if self.model.wheel_key() == Some(key) {
            if let Some(format) = self.model.wheel_image {
                self.frames.insert(key, face::encode(face, &format)?);
            }
        } else if self.model.strip_segment(key).is_some() {
            if let Some(format) = self.model.strip_image {
                self.frames.insert(key, face::encode(face, &format)?);
            }
        } else if key < self.model.screen_keys() {
            if let Some(format) = self.model.image {
                self.frames.insert(key, face::encode(face, &format)?);
            }
        } else if self.model.key_lights && self.light_id(key).is_some() {
            self.lights
                .insert(key, face.map_or([0, 0, 0], |face| face.color));
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.upgrade()?;
        let frames = std::mem::take(&mut self.frames);
        let mut drawn = BTreeSet::new();
        for (key, pixels) in frames {
            let Some((display, rectangle)) = self.region(key) else {
                continue;
            };
            let mut data = Vec::with_capacity(10 + pixels.len());
            data.extend_from_slice(&display);
            for value in rectangle {
                data.extend_from_slice(&value.to_be_bytes());
            }
            data.extend_from_slice(&pixels);
            self.send(FRAMEBUFFER, &data)?;
            drawn.insert(display);
        }
        for display in drawn {
            self.send(DRAW, &display)?;
        }
        let lights = std::mem::take(&mut self.lights);
        for (key, [r, g, b]) in lights {
            if let Some(id) = self.light_id(key) {
                self.send(SET_COLOR, &[id, r, g, b])?;
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.upgrade()?;
        let level = (u16::from(percent.min(100)) * MAX_BRIGHTNESS + 50) / 100;
        self.send(SET_BRIGHTNESS, &[level as u8])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog;
    use crate::driver::testing::Recorder;

    const HTTP_OK: &[u8] = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n";

    fn driver(model: &str) -> (Loupedeck, Recorder) {
        let recorder = Recorder::default();
        let model = catalog::by_id(model).unwrap();
        (Loupedeck::new(model, Box::new(recorder.clone())), recorder)
    }

    /// A device-to-host frame (unmasked) carrying one message.
    fn incoming(command: u8, data: &[u8]) -> Vec<u8> {
        let mut message = vec![(3 + data.len()) as u8, command, 0];
        message.extend_from_slice(data);
        let mut out = vec![0x82, message.len() as u8];
        out.extend(message);
        out
    }

    fn touch(command: u8, x: u16, y: u16, id: u8) -> Vec<u8> {
        let [xh, xl] = x.to_be_bytes();
        let [yh, yl] = y.to_be_bytes();
        incoming(command, &[0, xh, xl, yh, yl, id])
    }

    #[test]
    fn the_handshake_comes_first_and_keeps_trailing_input() {
        let (mut deck, recorder) = driver("loupedeck-live");
        let mut answer = HTTP_OK.to_vec();
        answer.extend(incoming(BUTTON_PRESS, &[0x07, 0x00]));
        recorder.queue_read(answer);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 12,
                pressed: true
            }]
        );
        assert_eq!(recorder.writes()[0], UPGRADE_REQUEST);
    }

    #[test]
    fn a_silent_port_is_an_error() {
        let (mut deck, _) = driver("loupedeck-live");
        assert!(deck.poll(0).is_err());
    }

    #[test]
    fn touches_press_and_release_the_key_under_the_finger() {
        let (mut deck, recorder) = driver("loupedeck-live");
        recorder.queue_read(HTTP_OK.to_vec());
        // Column 1, row 2 of the Live's 4 x 3 grid (x is offset by the left strip).
        let mut input = touch(TOUCH, 60 + 90 + 10, 180 + 5, 3);
        input.extend(touch(TOUCH, 60 + 90 + 40, 180 + 5, 3));
        input.extend(touch(TOUCH_END, 400, 10, 3));
        recorder.queue_read(input);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 9,
                    pressed: true
                },
                Input::Key {
                    index: 9,
                    pressed: false
                },
            ]
        );
    }

    /// The face message a framebuffer frame carries: display, then the header.
    fn framebuffer_target(frame: &[u8]) -> ([u8; 2], Vec<u16>) {
        let message = &frame[14..];
        assert_eq!(message[1], FRAMEBUFFER);
        let header = message[5..13]
            .chunks(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        ([message[3], message[4]], header)
    }

    #[test]
    fn side_strips_press_the_segment_beside_each_knob() {
        let (mut deck, recorder) = driver("loupedeck-live");
        recorder.queue_read(HTTP_OK.to_vec());
        // Left strip, middle; then right strip, bottom (past the last row too).
        let mut input = touch(TOUCH, 10, 100, 1);
        input.extend(touch(TOUCH_END, 10, 100, 1));
        input.extend(touch(TOUCH, 470, 268, 2));
        recorder.queue_read(input);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 21,
                    pressed: true
                },
                Input::Key {
                    index: 21,
                    pressed: false
                },
                Input::Key {
                    index: 25,
                    pressed: true
                },
            ]
        );
    }

    #[test]
    fn live_strip_faces_go_either_side_of_the_grid() {
        let (mut deck, recorder) = driver("loupedeck-live");
        recorder.queue_read(HTTP_OK.to_vec());
        let face = Face {
            label: String::new(),
            color: [0, 255, 0],
            image: None,
        };
        // Strip key 20 is the left top; 25 the right bottom.
        deck.set_face(20, Some(&face)).unwrap();
        deck.set_face(25, Some(&face)).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(
            framebuffer_target(&writes[1]),
            ([0x00, b'M'], vec![0, 0, 60, 90])
        );
        assert_eq!(
            framebuffer_target(&writes[2]),
            ([0x00, b'M'], vec![420, 180, 60, 90])
        );
        // One shared display, drawn once.
        assert_eq!(writes.len(), 4);
        assert_eq!(writes[3][7], DRAW);
    }

    #[test]
    fn the_ct_wheel_screen_is_a_key_of_its_own() {
        let (mut deck, recorder) = driver("loupedeck-ct");
        recorder.queue_read(HTTP_OK.to_vec());
        // Touch id 0 is the wheel, whatever the coordinates say.
        let mut input = touch(TOUCH, 120, 120, WHEEL_TOUCH_ID);
        input.extend(touch(TOUCH, 200, 40, WHEEL_TOUCH_ID));
        input.extend(touch(TOUCH_END, 200, 40, WHEEL_TOUCH_ID));
        recorder.queue_read(input);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 38,
                    pressed: true
                },
                Input::Key {
                    index: 38,
                    pressed: false
                },
            ]
        );
        let face = Face {
            label: String::new(),
            color: [255, 0, 8],
            image: None,
        };
        deck.set_face(38, Some(&face)).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(
            framebuffer_target(&writes[1]),
            ([0x00, b'W'], vec![0, 0, 240, 240])
        );
        // Big-endian pixels: 0xf801 high byte first.
        let message = &writes[1][14..];
        assert_eq!(&message[13..15], &[0xf8, 0x01]);
        assert_eq!(&writes[2][9..11], &[0x00, b'W'], "the wheel is drawn");
    }

    #[test]
    fn the_live_has_no_wheel_screen() {
        let (mut deck, recorder) = driver("loupedeck-live");
        recorder.queue_read(HTTP_OK.to_vec());
        // Touch id 0 on the Live is an ordinary touch (left strip, top).
        recorder.queue_read(touch(TOUCH, 10, 10, 0));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 20,
                pressed: true
            }]
        );
    }

    #[test]
    fn ct_strip_faces_use_their_own_displays() {
        let (mut deck, recorder) = driver("loupedeck-ct");
        recorder.queue_read(HTTP_OK.to_vec());
        recorder.queue_read(touch(TOUCH, 20, 200, 5));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 34,
                pressed: true
            }]
        );
        let face = Face {
            label: String::new(),
            color: [0, 0, 255],
            image: None,
        };
        deck.set_face(0, Some(&face)).unwrap();
        deck.set_face(33, Some(&face)).unwrap();
        deck.set_face(36, Some(&face)).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        let targets: Vec<_> = writes[1..4]
            .iter()
            .map(|frame| framebuffer_target(frame))
            .collect();
        assert_eq!(
            targets,
            vec![
                ([0x00, b'A'], vec![0, 0, 90, 90]),
                ([0x00, b'L'], vec![0, 90, 60, 90]),
                ([0x00, b'R'], vec![0, 90, 60, 90]),
            ]
        );
        // Each display that changed is drawn.
        let drawn: Vec<[u8; 2]> = writes[4..]
            .iter()
            .filter(|frame| frame[7] == DRAW)
            .map(|frame| [frame[9], frame[10]])
            .collect();
        assert_eq!(drawn, vec![[0x00, b'A'], [0x00, b'L'], [0x00, b'R']]);
    }

    #[test]
    fn knobs_turn_and_press_as_dials() {
        let (mut deck, recorder) = driver("loupedeck-ct");
        recorder.queue_read(HTTP_OK.to_vec());
        let mut input = incoming(KNOB_ROTATE, &[0x04, 0xfe]);
        input.extend(incoming(BUTTON_PRESS, &[0x00, 0x01]));
        input.extend(incoming(BUTTON_PRESS, &[0x0f, 0x00]));
        recorder.queue_read(input);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialTurn {
                    index: 3,
                    delta: -2
                },
                Input::DialPress {
                    index: 6,
                    pressed: false
                },
                // Home is the first labelled button, after 12 touch + 8 round keys.
                Input::Key {
                    index: 20,
                    pressed: true
                },
            ]
        );
    }

    #[test]
    fn stream_controller_x_keys_are_buttons() {
        let (mut deck, recorder) = driver("razer-stream-controller-x");
        let mut answer = HTTP_OK.to_vec();
        answer.extend(incoming(BUTTON_PRESS, &[0x29, 0x01]));
        answer.extend(touch(TOUCH, 10, 10, 1));
        recorder.queue_read(answer);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 14,
                pressed: false
            }]
        );
    }

    #[test]
    fn faces_land_in_the_framebuffer_and_buttons_light_up() {
        let (mut deck, recorder) = driver("loupedeck-live-s");
        recorder.queue_read(HTTP_OK.to_vec());
        let face = Face {
            label: String::new(),
            color: [255, 0, 0],
            image: None,
        };
        // Key 6 is row 1, column 1 of the 5-wide grid; key 16 is round button 2.
        deck.set_face(6, Some(&face)).unwrap();
        deck.set_face(16, Some(&face)).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        let framebuffer = &writes[1];
        // 64-bit length frame: 2 + 8 length bytes + 4 mask bytes.
        assert_eq!(&framebuffer[..2], &[0x82, 0xff]);
        let message_len = u64::from_be_bytes(framebuffer[2..10].try_into().unwrap());
        assert_eq!(message_len, 3 + 2 + 8 + 90 * 90 * 2);
        let message = &framebuffer[14..];
        assert_eq!(message[1], FRAMEBUFFER);
        assert_eq!(&message[3..5], &[0x00, b'M']);
        let header: Vec<u16> = message[5..13]
            .chunks(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(header, vec![15 + 90, 90, 90, 90]);
        assert_eq!(&message[13..15], &[0x00, 0xf8]);
        let draw = &writes[2];
        assert_eq!(&draw[..2], &[0x82, 0x85]);
        assert_eq!(draw[7], DRAW);
        let color = &writes[3];
        assert_eq!(color[7], SET_COLOR);
        assert_eq!(&color[9..13], &[0x08, 255, 0, 0]);
    }

    #[test]
    fn brightness_maps_to_ten_steps() {
        let (mut deck, recorder) = driver("loupedeck-live");
        recorder.queue_read(HTTP_OK.to_vec());
        deck.set_brightness(64).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[1][7], SET_BRIGHTNESS);
        assert_eq!(writes[1][9], 6);
    }

    #[test]
    fn every_lit_key_has_a_button() {
        for id in ["loupedeck-live", "loupedeck-live-s", "loupedeck-ct"] {
            let (deck, _) = driver(id);
            let model = deck.model;
            for key in model.screen_keys()..model.keys {
                let light = deck.light_id(key).unwrap();
                assert_eq!(
                    deck.button_target(light),
                    Some(Target::Key(key)),
                    "{id} {key}"
                );
            }
        }
    }
}
