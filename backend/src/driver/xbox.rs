//! Microsoft Xbox controllers read as HID gamepads: input only, with no screens
//! and no lights.
//!
//! Three frames are read, told apart by their first byte:
//!
//! - `07`, 5 bytes or more: a GIP Guide frame over USB; byte 4 nonzero means the
//!   Xbox button is held. macOS repeats it, so only a change counts.
//! - `20`, 18 bytes or more: a GIP input frame over USB. A u16 LE button bitmap
//!   at 4, the triggers (0 to 1023, u16 LE) at 6 and 8, and the sticks as i16 LE
//!   at 10 (left X), 12 (left Y), 14 (right X) and 16 (right Y), up and right
//!   positive. It carries no Xbox or Share button.
//! - `01`, 17 bytes or more: Bluetooth report 1. After the id: the sticks as u16
//!   LE centred on 32767.5 (Y down-positive), the triggers, a d-pad hat and three
//!   button bytes. This layout is not yet checked against a controller.
//!
//! Every other frame is ignored. Nothing is ever written to the pad.
//!
//! The 24 keys are the buttons, the triggers and each stick's four directions;
//! a trigger or stick direction is held once it passes half its travel (after a
//! 15% dead zone) and let go below 35%. The sticks' axes are also four dials
//! that turn on their own while the stick is held off centre, faster the
//! further it is pushed (2 to 15 steps a second); the stick clicks press dials
//! 0 (left) and 2 (right). Everything starts released and centred.

use std::time::{Duration, Instant};

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const READ_BUFFER: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 16;

/// The key names, in key order: three rows of eight.
pub const KEY_NAMES: [&str; 24] = [
    "A", "B", "X", "Y", "LB", "RB", "LT", "RT", "Up", "Down", "Left", "Right", "View", "Menu",
    "Xbox", "Share", "LS up", "LS down", "LS left", "LS right", "RS up", "RS down", "RS left",
    "RS right",
];

const KEY_A: usize = 0;
const KEY_B: usize = 1;
const KEY_X: usize = 2;
const KEY_Y: usize = 3;
const KEY_LB: usize = 4;
const KEY_RB: usize = 5;
const KEY_LT: usize = 6;
const KEY_RT: usize = 7;
const KEY_UP: usize = 8;
const KEY_DOWN: usize = 9;
const KEY_LEFT: usize = 10;
const KEY_RIGHT: usize = 11;
const KEY_VIEW: usize = 12;
const KEY_MENU: usize = 13;
const KEY_XBOX: usize = 14;
const KEY_SHARE: usize = 15;
/// The first stick-direction key: left stick up, down, left, right, then the
/// right stick's.
const KEY_STICKS: usize = 16;
const KEYS: usize = 24;

/// The GIP input frame's button bits, as (bit, key).
const GIP_BUTTONS: [(u8, usize); 12] = [
    (2, KEY_MENU),
    (3, KEY_VIEW),
    (4, KEY_A),
    (5, KEY_B),
    (6, KEY_X),
    (7, KEY_Y),
    (8, KEY_UP),
    (9, KEY_DOWN),
    (10, KEY_LEFT),
    (11, KEY_RIGHT),
    (12, KEY_LB),
    (13, KEY_RB),
];
const GIP_LEFT_CLICK: u8 = 14;
const GIP_RIGHT_CLICK: u8 = 15;

/// Bluetooth's three button bytes (body offsets 13, 14 and 15), as (mask, key).
const BT_FACE_BUTTONS: [(u8, usize); 6] = [
    (0x01, KEY_A),
    (0x02, KEY_B),
    (0x08, KEY_X),
    (0x10, KEY_Y),
    (0x40, KEY_LB),
    (0x80, KEY_RB),
];
const BT_MENU_BUTTONS: [(u8, usize); 3] = [(0x04, KEY_VIEW), (0x08, KEY_MENU), (0x10, KEY_XBOX)];
const BT_LEFT_CLICK: u8 = 0x20;
const BT_RIGHT_CLICK: u8 = 0x40;
const BT_SHARE: u8 = 0x01;

const TRIGGER_MAX: f32 = 1023.0;
const STICK_MAX: f32 = 32767.0;
const BT_STICK_CENTRE: f32 = 32767.5;

const DEAD_ZONE: f32 = 0.15;
pub(crate) const PRESS_AT: f32 = 0.5;
pub(crate) const RELEASE_RATIO: f32 = 0.7;
const ROTARY_LEVELS: f32 = 8.0;
const ROTARY_MAX_RATE: f32 = 15.0;
const ROTARY_MIN_RATE: f32 = 2.0;

/// Dial index of each stick axis: left X, left Y, right X, right Y.
const AXES: usize = 4;
/// The dials the left and right stick clicks press.
const LEFT_CLICK_DIAL: u8 = 0;
const RIGHT_CLICK_DIAL: u8 = 2;

/// What the controller last reported, before thresholds.
#[derive(Debug, Clone, Copy, Default)]
struct Pad {
    /// The digital buttons, by key index (trigger and stick keys unused).
    buttons: [bool; KEYS],
    left_click: bool,
    right_click: bool,
    /// Left and right trigger, 0 to 1.
    triggers: [f32; 2],
    /// Left X, left Y, right X, right Y, -1 to 1, right and up positive.
    sticks: [f32; AXES],
}

/// One stick axis's repeating dial turn.
#[derive(Debug, Clone, Copy, Default)]
struct Rotary {
    /// -8 to 8: how far the axis is pushed, in steps; 0 is centred.
    level: i8,
    next: Option<Instant>,
}

pub struct Xbox {
    transport: Box<dyn Transport>,
    pad: Pad,
    keys: [bool; KEYS],
    clicks: [bool; 2],
    rotaries: [Rotary; AXES],
}

impl Xbox {
    #[must_use]
    pub fn new(_model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            pad: Pad::default(),
            keys: [false; KEYS],
            clicks: [false; 2],
            rotaries: [Rotary::default(); AXES],
        }
    }

    /// Reads one frame into `pad`. Returns whether it was one the pad sends.
    fn read_frame(&mut self, frame: &[u8]) -> bool {
        match frame.first() {
            Some(0x07) if frame.len() >= 5 => {
                self.pad.buttons[KEY_XBOX] = frame.get(4).is_some_and(|held| *held != 0);
                true
            }
            Some(0x20) if frame.len() >= 18 => self.read_gip(frame),
            Some(0x01) if frame.len() >= 17 => {
                frame.get(1..).is_some_and(|body| self.read_bluetooth(body))
            }
            _ => false,
        }
    }

    fn read_gip(&mut self, frame: &[u8]) -> bool {
        let (Some(buttons), Some(left), Some(right)) =
            (u16_at(frame, 4), u16_at(frame, 6), u16_at(frame, 8))
        else {
            return false;
        };
        let held = |bit: u8| buttons & (1 << bit) != 0;
        for (bit, key) in GIP_BUTTONS {
            self.pad.buttons[key] = held(bit);
        }
        self.pad.left_click = held(GIP_LEFT_CLICK);
        self.pad.right_click = held(GIP_RIGHT_CLICK);
        self.pad.triggers = [trigger(left), trigger(right)];
        for (axis, value) in self.pad.sticks.iter_mut().enumerate() {
            let raw =
                u16_at(frame, 10 + axis * 2).map_or(0, |raw| i16::from_le_bytes(raw.to_le_bytes()));
            *value = (f32::from(raw) / STICK_MAX).clamp(-1.0, 1.0);
        }
        true
    }

    fn read_bluetooth(&mut self, body: &[u8]) -> bool {
        let (Some(left), Some(right), Some(&hat), Some(&face), Some(&menu), Some(&extra)) = (
            u16_at(body, 8),
            u16_at(body, 10),
            body.get(12),
            body.get(13),
            body.get(14),
            body.get(15),
        ) else {
            return false;
        };
        for (axis, value) in self.pad.sticks.iter_mut().enumerate() {
            let raw = u16_at(body, axis * 2).unwrap_or(0);
            let centred = ((f32::from(raw) - BT_STICK_CENTRE) / BT_STICK_CENTRE).clamp(-1.0, 1.0);
            // Bluetooth's Y axes grow downward.
            *value = if axis % 2 == 1 { -centred } else { centred };
        }
        self.pad.triggers = [trigger(left), trigger(right)];
        let (up, right_dir, down, left_dir) = match hat {
            1 => (true, false, false, false),
            2 => (true, true, false, false),
            3 => (false, true, false, false),
            4 => (false, true, true, false),
            5 => (false, false, true, false),
            6 => (false, false, true, true),
            7 => (false, false, false, true),
            8 => (true, false, false, true),
            _ => (false, false, false, false),
        };
        self.pad.buttons[KEY_UP] = up;
        self.pad.buttons[KEY_DOWN] = down;
        self.pad.buttons[KEY_LEFT] = left_dir;
        self.pad.buttons[KEY_RIGHT] = right_dir;
        for (mask, key) in BT_FACE_BUTTONS {
            self.pad.buttons[key] = face & mask != 0;
        }
        for (mask, key) in BT_MENU_BUTTONS {
            self.pad.buttons[key] = menu & mask != 0;
        }
        self.pad.left_click = menu & BT_LEFT_CLICK != 0;
        self.pad.right_click = menu & BT_RIGHT_CLICK != 0;
        self.pad.buttons[KEY_SHARE] = extra & BT_SHARE != 0;
        true
    }

    /// How far each key is pressed, 0 to 1, after the dead zone.
    fn key_travel(&self, key: usize) -> f32 {
        let sticks = self.pad.sticks.map(dead_zone);
        match key {
            KEY_LT => dead_zone(self.pad.triggers[0]),
            KEY_RT => dead_zone(self.pad.triggers[1]),
            _ if key >= KEY_STICKS => {
                let direction = key - KEY_STICKS;
                // Up, down, left, right of each stick; Y is the odd axis.
                let stick = direction / 4;
                let (axis, toward) = match direction % 4 {
                    0 => (stick * 2 + 1, 1.0),
                    1 => (stick * 2 + 1, -1.0),
                    2 => (stick * 2, -1.0),
                    _ => (stick * 2, 1.0),
                };
                (sticks[axis] * toward).max(0.0)
            }
            _ => f32::from(u8::from(self.pad.buttons[key])),
        }
    }

    /// The events a new frame raises at `now`.
    fn apply(&mut self, now: Instant) -> Vec<Input> {
        let mut events = Vec::new();
        for key in 0..KEYS {
            let travel = self.key_travel(key);
            let held = self.keys[key];
            let pressed = if held {
                travel > PRESS_AT * RELEASE_RATIO
            } else {
                travel >= PRESS_AT
            };
            if pressed != held {
                self.keys[key] = pressed;
                events.push(Input::Key {
                    index: key as u8,
                    pressed,
                });
            }
        }
        let clicks = [
            (LEFT_CLICK_DIAL, self.pad.left_click),
            (RIGHT_CLICK_DIAL, self.pad.right_click),
        ];
        for ((index, pressed), held) in clicks.into_iter().zip(self.clicks.iter_mut()) {
            if pressed != *held {
                *held = pressed;
                events.push(Input::DialPress { index, pressed });
            }
        }
        for axis in 0..AXES {
            let level = rotary_level(dead_zone(self.pad.sticks[axis]));
            if let Some(event) = self.set_level(axis, level, now) {
                events.push(event);
            }
        }
        events
    }

    /// Moves an axis to a new level: a turn at once when it leaves centre or
    /// changes direction, then repeats at the new level's rate.
    fn set_level(&mut self, axis: usize, level: i8, now: Instant) -> Option<Input> {
        let rotary = &mut self.rotaries[axis];
        let previous = rotary.level;
        if level == previous {
            return None;
        }
        rotary.level = level;
        if level == 0 {
            rotary.next = None;
            return None;
        }
        rotary.next = Some(now + repeat_period(level));
        (previous == 0 || previous.signum() != level.signum()).then(|| turn(axis, level))
    }

    /// The repeat turns due by `now`, at most one per axis.
    fn due_turns(&mut self, now: Instant) -> Vec<Input> {
        let mut events = Vec::new();
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
        events
    }

    /// How long until the next repeat turn is due, if any axis is off centre.
    fn until_due(&self, now: Instant) -> Option<Duration> {
        self.rotaries
            .iter()
            .filter_map(|rotary| rotary.next)
            .min()
            .map(|due| due.saturating_duration_since(now))
    }

    fn poll_at(&mut self, timeout_ms: i32, now: impl Fn() -> Instant) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        // While a stick is off centre, wake in time for its next turn: the pad
        // only reports on a change.
        let first_wait = self.until_due(now()).map_or(timeout_ms, |wait| {
            let wait = i32::try_from(wait.as_millis()).unwrap_or(i32::MAX);
            wait.min(timeout_ms.max(0))
        });
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { first_wait } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            if let Some(frame) = buf.get(..read) {
                if self.read_frame(frame) {
                    events.extend(self.apply(now()));
                }
            }
        }
        events.extend(self.due_turns(now()));
        Ok(events)
    }
}

impl Driver for Xbox {
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

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let pair = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([pair[0], pair[1]]))
}

fn trigger(raw: u16) -> f32 {
    (f32::from(raw) / TRIGGER_MAX).clamp(0.0, 1.0)
}

/// Zero inside the dead zone; outside it, rescaled so the edge of the zone is 0
/// and full travel is still 1.
pub(crate) fn dead_zone(value: f32) -> f32 {
    let magnitude = value.abs();
    if magnitude <= DEAD_ZONE {
        0.0
    } else {
        value.signum() * (magnitude - DEAD_ZONE) / (1.0 - DEAD_ZONE)
    }
}

/// An axis's repeat level, -8 to 8, from its dead-zoned value.
pub(crate) fn rotary_level(value: f32) -> i8 {
    if value == 0.0 {
        return 0;
    }
    let steps = (value.abs() * ROTARY_LEVELS)
        .ceil()
        .clamp(1.0, ROTARY_LEVELS) as i8;
    if value < 0.0 {
        -steps
    } else {
        steps
    }
}

/// Time between repeat turns at a level: 2 a second at the lowest, 15 at the top.
pub(crate) fn repeat_period(level: i8) -> Duration {
    let min = ROTARY_MIN_RATE.min(ROTARY_MAX_RATE);
    let rate = min + (ROTARY_MAX_RATE - min) * f32::from(level.unsigned_abs()) / ROTARY_LEVELS;
    Duration::from_secs_f32(1.0 / rate)
}

fn turn(axis: usize, level: i8) -> Input {
    Input::DialTurn {
        index: axis as u8,
        delta: level.signum(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn driver() -> (Xbox, Recorder) {
        let recorder = Recorder::default();
        (
            Xbox::new(
                by_id("microsoft-xbox-series").unwrap(),
                Box::new(recorder.clone()),
            ),
            recorder,
        )
    }

    /// A GIP input frame (USB).
    fn gip(buttons: u16, triggers: [u16; 2], sticks: [i16; 4]) -> Vec<u8> {
        let mut frame = vec![0x20, 0x00, 0x01, 0x0e];
        frame.extend(buttons.to_le_bytes());
        for value in triggers {
            frame.extend(value.to_le_bytes());
        }
        for value in sticks {
            frame.extend(value.to_le_bytes());
        }
        frame
    }

    /// A Bluetooth report 1.
    fn bluetooth(sticks: [u16; 4], triggers: [u16; 2], hat: u8, buttons: [u8; 3]) -> Vec<u8> {
        let mut frame = vec![0x01];
        for value in sticks.into_iter().chain(triggers) {
            frame.extend(value.to_le_bytes());
        }
        frame.push(hat);
        frame.extend(buttons);
        frame
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    #[test]
    fn gip_buttons_map_to_their_keys_and_clicks_press_dials() {
        let (mut pad, recorder) = driver();
        // A (bit 4), Menu (bit 2), d-pad up and right (bits 8, 11), LB (bit 12),
        // and both stick clicks (bits 14, 15).
        let held = 1 << 4 | 1 << 2 | 1 << 8 | 1 << 11 | 1 << 12 | 1 << 14 | 1 << 15;
        recorder.queue_read(gip(held, [0, 0], [0; 4]));
        recorder.queue_read(gip(0, [0, 0], [0; 4]));
        assert_eq!(
            pad.poll(0).unwrap(),
            vec![
                key(0, true),
                key(4, true),
                key(8, true),
                key(11, true),
                key(13, true),
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                Input::DialPress {
                    index: 2,
                    pressed: true
                },
                key(0, false),
                key(4, false),
                key(8, false),
                key(11, false),
                key(13, false),
                Input::DialPress {
                    index: 0,
                    pressed: false
                },
                Input::DialPress {
                    index: 2,
                    pressed: false
                },
            ]
        );
        assert!(recorder.writes().is_empty() && recorder.features().is_empty());
    }

    #[test]
    fn the_other_gip_bits_are_view_b_x_y_down_left_rb() {
        let (mut pad, recorder) = driver();
        let held = 1 << 3 | 1 << 5 | 1 << 6 | 1 << 7 | 1 << 9 | 1 << 10 | 1 << 13 | 0b11;
        recorder.queue_read(gip(held, [0, 0], [0; 4]));
        assert_eq!(
            pad.poll(0).unwrap(),
            [1, 2, 3, 5, 9, 10, 12]
                .into_iter()
                .map(|index| key(index, true))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_guide_frame_holds_the_xbox_key_and_repeats_are_ignored() {
        let (mut pad, recorder) = driver();
        for held in [1, 1, 1, 0, 0] {
            recorder.queue_read(vec![0x07, 0x30, 0x19, 0x02, held, 0x5b]);
        }
        // A GIP input frame leaves the Xbox key alone.
        recorder.queue_read(vec![0x07, 0x30, 0x19, 0x02, 1, 0x5b]);
        recorder.queue_read(gip(0, [0, 0], [0; 4]));
        assert_eq!(
            pad.poll(0).unwrap(),
            vec![key(14, true), key(14, false), key(14, true)]
        );
    }

    #[test]
    fn triggers_pass_the_dead_zone_then_hold_with_hysteresis() {
        let (mut pad, recorder) = driver();
        // 0.5 after the 15% dead zone is a raw 0.575: 589 of 1023 presses, 588 does not.
        recorder.queue_read(gip(0, [588, 0], [0; 4]));
        assert!(pad.poll(0).unwrap().is_empty());
        recorder.queue_read(gip(0, [589, 1023], [0; 4]));
        assert_eq!(pad.poll(0).unwrap(), vec![key(6, true), key(7, true)]);
        // It stays held above 35% (a raw 0.4475, 458 of 1023) and lets go at 457.
        recorder.queue_read(gip(0, [458, 1023], [0; 4]));
        assert!(pad.poll(0).unwrap().is_empty());
        recorder.queue_read(gip(0, [457, 0], [0; 4]));
        assert_eq!(pad.poll(0).unwrap(), vec![key(6, false), key(7, false)]);
    }

    #[test]
    fn stick_directions_are_keys_and_axes_turn_dials() {
        let (mut pad, recorder) = driver();
        let start = Instant::now();
        // Left stick full up (Y is up-positive over GIP), right stick full left.
        recorder.queue_read(gip(0, [0, 0], [0, 32767, -32768, 0]));
        let events = pad.poll_at(0, || start).unwrap();
        assert_eq!(
            events,
            vec![
                key(16, true),
                key(22, true),
                Input::DialTurn { index: 1, delta: 1 },
                Input::DialTurn {
                    index: 2,
                    delta: -1
                },
            ]
        );
        // Full travel repeats 15 times a second; nothing more is due yet.
        assert!(pad
            .poll_at(0, || start + Duration::from_millis(50))
            .unwrap()
            .is_empty());
        assert_eq!(
            pad.poll_at(0, || start + Duration::from_millis(67))
                .unwrap(),
            vec![
                Input::DialTurn { index: 1, delta: 1 },
                Input::DialTurn {
                    index: 2,
                    delta: -1
                },
            ]
        );
        // Back to centre: the keys let go and the repeats stop.
        recorder.queue_read(gip(0, [0, 0], [0; 4]));
        assert_eq!(
            pad.poll_at(0, || start + Duration::from_millis(80))
                .unwrap(),
            vec![key(16, false), key(22, false)]
        );
        assert!(pad
            .poll_at(0, || start + Duration::from_secs(2))
            .unwrap()
            .is_empty());
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn a_rotary_turns_at_once_only_from_centre_or_on_reversal() {
        let (mut pad, recorder) = driver();
        let start = Instant::now();
        let clock = Cell::new(start);
        let at = |ms: u64| start + Duration::from_millis(ms);
        // A small push right (level 1) turns once, then every 1000 / 3.625 ms.
        recorder.queue_read(gip(0, [0, 0], [8000, 0, 0, 0]));
        assert_eq!(
            pad.poll_at(0, || clock.get()).unwrap(),
            vec![Input::DialTurn { index: 0, delta: 1 }]
        );
        assert_eq!(rotary_level(dead_zone(8000.0 / STICK_MAX)), 1);
        // Pushing further (same direction) restarts the repeat with no turn.
        clock.set(at(100));
        recorder.queue_read(gip(0, [0, 0], [32767, 0, 0, 0]));
        assert_eq!(pad.poll_at(0, || clock.get()).unwrap(), vec![key(19, true)]);
        assert!(pad.poll_at(0, || at(160)).unwrap().is_empty());
        assert_eq!(
            pad.poll_at(0, || at(167)).unwrap(),
            vec![Input::DialTurn { index: 0, delta: 1 }]
        );
        // Reversing turns the other way at once.
        clock.set(at(170));
        recorder.queue_read(gip(0, [0, 0], [-32767, 0, 0, 0]));
        assert_eq!(
            pad.poll_at(0, || clock.get()).unwrap(),
            vec![
                key(18, true),
                key(19, false),
                Input::DialTurn {
                    index: 0,
                    delta: -1
                },
            ]
        );
    }

    #[test]
    fn levels_and_rates_follow_the_stick() {
        assert_eq!(rotary_level(0.0), 0);
        assert_eq!(rotary_level(0.01), 1);
        assert_eq!(rotary_level(-0.5), -4);
        assert_eq!(rotary_level(1.0), 8);
        assert_eq!(repeat_period(1).as_millis(), 275);
        assert_eq!(repeat_period(8).as_millis(), 66);
        assert_eq!(dead_zone(0.15), 0.0);
        assert!((dead_zone(-0.575) + 0.5).abs() < 1e-6);
    }

    #[test]
    fn bluetooth_reads_hat_buttons_share_and_down_positive_y() {
        let (mut pad, recorder) = driver();
        // Left stick pushed down (raw Y 65535 is down), hat 6 (down-left), A and
        // RB, View and both clicks, Xbox, Share.
        recorder.queue_read(bluetooth(
            [32768, 65535, 32768, 32768],
            [0, 0],
            6,
            [0x01 | 0x80, 0x04 | 0x10 | 0x20 | 0x40, 0x01],
        ));
        let start = Instant::now();
        assert_eq!(
            pad.poll_at(0, || start).unwrap(),
            vec![
                key(0, true),
                key(5, true),
                key(9, true),
                key(10, true),
                key(12, true),
                key(14, true),
                key(15, true),
                key(17, true),
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                Input::DialPress {
                    index: 2,
                    pressed: true
                },
                Input::DialTurn {
                    index: 1,
                    delta: -1
                },
            ]
        );
        // Hat 0 is centred, and B, X, Y, LB, Menu sit on their own bits.
        recorder.queue_read(bluetooth(
            [32768; 4],
            [0, 1023],
            0,
            [0x02 | 0x08 | 0x10 | 0x40, 0x08, 0],
        ));
        assert_eq!(
            pad.poll_at(0, || start).unwrap(),
            vec![
                key(0, false),
                key(1, true),
                key(2, true),
                key(3, true),
                key(4, true),
                key(5, false),
                key(7, true),
                key(9, false),
                key(10, false),
                key(12, false),
                key(13, true),
                key(14, false),
                key(15, false),
                key(17, false),
                Input::DialPress {
                    index: 0,
                    pressed: false
                },
                Input::DialPress {
                    index: 2,
                    pressed: false
                },
            ]
        );
        // An out-of-range hat is no direction.
        recorder.queue_read(bluetooth(
            [32768; 4],
            [0, 1023],
            2,
            [0x02 | 0x08 | 0x10 | 0x40, 0x08, 0],
        ));
        recorder.queue_read(bluetooth(
            [32768; 4],
            [0, 1023],
            9,
            [0x02 | 0x08 | 0x10 | 0x40, 0x08, 0],
        ));
        assert_eq!(
            pad.poll_at(0, || start).unwrap(),
            vec![key(8, true), key(11, true), key(8, false), key(11, false)]
        );
    }

    #[test]
    fn short_and_unknown_frames_are_ignored() {
        let (mut pad, recorder) = driver();
        let mut short = gip(0xffff, [1023, 1023], [32767; 4]);
        short.truncate(17);
        recorder.queue_read(short);
        recorder.queue_read(vec![0x07, 0, 0, 1]);
        recorder.queue_read(bluetooth([0; 4], [1023; 2], 1, [0xff; 3])[..16].to_vec());
        recorder.queue_read(vec![0x03, 0xff, 0xff, 0xff, 0xff, 0xff]);
        recorder.queue_read(vec![]);
        assert!(pad.poll(0).unwrap().is_empty());
        assert!(pad.set_face(0, None).is_ok() && pad.flush().is_ok());
        assert!(pad.set_brightness(50).is_ok());
        assert!(recorder.writes().is_empty() && recorder.features().is_empty());
    }
}
