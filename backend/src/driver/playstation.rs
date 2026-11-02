//! Sony PlayStation controllers over USB as HID gamepads: the DualShock 4, the
//! DualSense and DualSense Edge, and the Access controller. Keys and dials come
//! from input report 0x01; the one light Ryu drives is the RGB light bar.
//!
//! Every pad sends input report `01`, 64 bytes over USB. Offsets below count
//! the report id as byte 0. A shorter `01` is the reduced Bluetooth frame and
//! every other id is something else; both are ignored (Ryu drives these pads
//! over USB only).
//!
//! - DualSense and Edge: sticks at 1 to 4 (left X, left Y, right X, right Y,
//!   0x80 centred, Y growing downward), the triggers at 5 and 6, then the hat
//!   and face buttons at 8, the shoulder and menu buttons at 9 and PS,
//!   touchpad and mute at 10. The Edge adds its Fn buttons and back paddles in
//!   10's high nibble (named only by the Linux driver, so their left/right
//!   order is not yet checked on a pad).
//! - DualShock 4: the sticks at 1 to 4, the hat and face buttons at 5, the
//!   shoulder and menu buttons at 6, PS and touchpad at 7, the triggers at 8
//!   and 9.
//! - Access controller: its first bytes carry the active profile's mapped
//!   actions, which are not used. The raw buttons are read instead: B1 to B8
//!   at 16, centre, stick click, PS and profile at 17, the raw stick's Y at 18
//!   and X at 19 (0xFF is up and right). The PS bit and the stick's byte order
//!   come from one reference each and are not yet checked on a controller.
//!
//! The gamepads' keys follow the Xbox driver's order and thresholds: the face
//! buttons, shoulders and triggers, the d-pad, the menu buttons, then each
//! stick's four directions (and the DualSense's mute, the Edge's Fn buttons
//! and paddles after them). A trigger or stick direction is held past half its
//! travel (after a 15% dead zone) and let go below 35%. The stick axes are
//! dials that repeat while the stick is off centre; the stick clicks press
//! dials 0 and 2 (the Access controller's one stick: dial 0).
//!
//! Output is one report per light change, only for the fields it flags, so
//! rumble, audio and the rest are never touched:
//!
//! - DualSense and Edge: report `02`, 48 bytes (the length its descriptor
//!   declares). Byte 2 flag 0x04 sets the light bar from 45 to 47. On the
//!   first write, the firmware's light-bar animation is released (39 and 42 set
//!   to 0x02) and the player LEDs and mute LED are cleared.
//! - DualShock 4: report `05`, 32 bytes, flag 0x02 at byte 1 and the colour
//!   at 6 to 8. The first write also stops any blink (flag 0x04, both times 0).
//!   The rumble flag is never set, though Linux and the WebHID driver set both;
//!   genuine Sony pads take the light alone.
//! - Access controller: report `02`, 32 bytes, mutator 0x04 at byte 1 and the
//!   colour at 6 to 8; that mutator also applies the player indicator at 5,
//!   which stays 0 (off). The first write turns the player indicator off. The
//!   profile and status LEDs are left alone.
//!
//! There is no brightness register: Ryu scales the colour. The light goes out
//! when Ryu lets go.

use std::time::{Duration, Instant};

use super::xbox::{dead_zone, repeat_period, rotary_level, PRESS_AT, RELEASE_RATIO};
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

const READ_BUFFER: usize = 78;
const MAX_REPORTS_PER_POLL: usize = 16;
const INPUT_REPORT: u8 = 0x01;
/// A full USB input report; a shorter report 0x01 is the reduced Bluetooth one.
const USB_INPUT_LEN: usize = 64;

/// The gamepads' key names, in key order. The DualShock 4 has the first 24
/// (with Share for Create), the DualSense 25 and the Edge all 29.
pub const KEY_NAMES: [&str; 29] = [
    "Cross",
    "Circle",
    "Square",
    "Triangle",
    "L1",
    "R1",
    "L2",
    "R2",
    "Up",
    "Down",
    "Left",
    "Right",
    "Create",
    "Options",
    "PS",
    "Touchpad",
    "LS up",
    "LS down",
    "LS left",
    "LS right",
    "RS up",
    "RS down",
    "RS left",
    "RS right",
    "Mute",
    "Left Fn",
    "Right Fn",
    "Left paddle",
    "Right paddle",
];

/// The Access controller's key names: its eight perimeter buttons, then the
/// centre, PS and profile buttons.
pub const ACCESS_KEY_NAMES: [&str; 11] = [
    "B1", "B2", "B3", "B4", "B5", "B6", "B7", "B8", "Center", "PS", "Profile",
];

const KEY_CROSS: usize = 0;
const KEY_CIRCLE: usize = 1;
const KEY_SQUARE: usize = 2;
const KEY_TRIANGLE: usize = 3;
const KEY_L1: usize = 4;
const KEY_R1: usize = 5;
const KEY_L2: usize = 6;
const KEY_R2: usize = 7;
const KEY_UP: usize = 8;
const KEY_DOWN: usize = 9;
const KEY_LEFT: usize = 10;
const KEY_RIGHT: usize = 11;
const KEY_CREATE: usize = 12;
const KEY_OPTIONS: usize = 13;
const KEY_PS: usize = 14;
const KEY_TOUCHPAD: usize = 15;
const KEY_STICKS: usize = 16;
const KEY_MUTE: usize = 24;
const KEY_EDGE: usize = 25;
const MAX_KEYS: usize = KEY_NAMES.len();

const ACCESS_KEY_CENTER: usize = 8;
const ACCESS_KEY_PS: usize = 9;
const ACCESS_KEY_PROFILE: usize = 10;

/// The face byte's high nibble, as (mask, key); its low nibble is the hat.
const FACE_BUTTONS: [(u8, usize); 4] = [
    (0x10, KEY_SQUARE),
    (0x20, KEY_CROSS),
    (0x40, KEY_CIRCLE),
    (0x80, KEY_TRIANGLE),
];
/// The shoulder and menu byte, as (mask, key). 0x04 and 0x08 are the
/// triggers' digital bits, unused: the analog values are read instead.
const MENU_BUTTONS: [(u8, usize); 4] = [
    (0x01, KEY_L1),
    (0x02, KEY_R1),
    (0x10, KEY_CREATE),
    (0x20, KEY_OPTIONS),
];
const LEFT_CLICK: u8 = 0x40;
const RIGHT_CLICK: u8 = 0x80;
/// The PS and touchpad byte (the DualSense's also has mute and the Edge's extras).
const PS_BUTTON: u8 = 0x01;
const TOUCHPAD_BUTTON: u8 = 0x02;
const MUTE_BUTTON: u8 = 0x04;
/// The Edge's Fn 1, Fn 2, left paddle and right paddle bits, in key order.
const EDGE_BUTTONS: [u8; 4] = [0x10, 0x20, 0x40, 0x80];

/// The Access controller's second raw button byte.
const ACCESS_CENTER: u8 = 0x01;
const ACCESS_STICK_CLICK: u8 = 0x02;
const ACCESS_PS: u8 = 0x04;
const ACCESS_PROFILE: u8 = 0x08;

const STICK_CENTRE: f32 = 128.0;
const STICK_SPAN: f32 = 127.0;
const TRIGGER_MAX: f32 = 255.0;

/// The dials the left and right stick clicks press.
const LEFT_CLICK_DIAL: u8 = 0;
const RIGHT_CLICK_DIAL: u8 = 2;
const MAX_AXES: usize = 4;

const DUALSENSE_OUTPUT: u8 = 0x02;
const DUALSENSE_OUTPUT_LEN: usize = 48;
const DUALSENSE_FLAG_MIC_LED: u8 = 0x01;
const DUALSENSE_FLAG_LIGHTBAR: u8 = 0x04;
const DUALSENSE_FLAG_PLAYER_LEDS: u8 = 0x10;
const DUALSENSE_FLAG2_LIGHTBAR_SETUP: u8 = 0x02;
const DUALSENSE_LIGHT_OUT: u8 = 0x02;

const DS4_OUTPUT: u8 = 0x05;
const DS4_OUTPUT_LEN: usize = 32;
const DS4_FLAG_LED: u8 = 0x02;
const DS4_FLAG_BLINK: u8 = 0x04;

const ACCESS_OUTPUT: u8 = 0x02;
const ACCESS_OUTPUT_LEN: usize = 32;
const ACCESS_MUTATOR_LED: u8 = 0x04;
const ACCESS_MUTATOR_PLAYER: u8 = 0x10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    DualSense,
    DualShock4,
    Access,
}

/// What the controller last reported, before thresholds.
#[derive(Debug, Clone, Copy, Default)]
struct Pad {
    /// The digital buttons, by key index (trigger and stick keys unused).
    buttons: [bool; MAX_KEYS],
    clicks: [bool; 2],
    /// Left and right trigger, 0 to 1.
    triggers: [f32; 2],
    /// The stick axes, -1 to 1, right and up positive: left X, left Y, right
    /// X, right Y (the Access controller's X and Y only).
    sticks: [f32; MAX_AXES],
}

/// One stick axis's repeating dial turn.
#[derive(Debug, Clone, Copy, Default)]
struct Rotary {
    level: i8,
    next: Option<Instant>,
}

pub struct PlayStation {
    transport: Box<dyn Transport>,
    kind: Kind,
    keys: usize,
    axes: usize,
    pad: Pad,
    held: [bool; MAX_KEYS],
    clicks: [bool; 2],
    rotaries: [Rotary; MAX_AXES],
    /// The light colour asked for, before brightness.
    light: Option<[u8; 3]>,
    brightness: u8,
    /// The colour last sent, after brightness; `None` before the first write.
    sent: Option<[u8; 3]>,
}

impl PlayStation {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = match model.protocol {
            Protocol::PlayStationDualShock4 => Kind::DualShock4,
            Protocol::PlayStationAccess => Kind::Access,
            _ => Kind::DualSense,
        };
        Self {
            transport,
            kind,
            keys: usize::from(model.keys).min(MAX_KEYS),
            axes: usize::from(model.dials).min(MAX_AXES),
            pad: Pad::default(),
            held: [false; MAX_KEYS],
            clicks: [false; 2],
            rotaries: [Rotary::default(); MAX_AXES],
            light: None,
            brightness: 100,
            sent: None,
        }
    }

    /// Reads one frame into `pad`. Returns whether it was a full USB input report.
    fn read_frame(&mut self, frame: &[u8]) -> bool {
        if frame.first() != Some(&INPUT_REPORT) || frame.len() < USB_INPUT_LEN {
            return false;
        }
        match self.kind {
            Kind::DualSense => self.read_gamepad(frame, 8, 9, 10, [5, 6]),
            Kind::DualShock4 => self.read_gamepad(frame, 5, 6, 7, [8, 9]),
            Kind::Access => self.read_access(frame),
        }
    }

    fn read_gamepad(
        &mut self,
        frame: &[u8],
        face_at: usize,
        menu_at: usize,
        ps_at: usize,
        triggers_at: [usize; 2],
    ) -> bool {
        let (Some(&face), Some(&menu), Some(&ps), Some(&left), Some(&right)) = (
            frame.get(face_at),
            frame.get(menu_at),
            frame.get(ps_at),
            frame.get(triggers_at[0]),
            frame.get(triggers_at[1]),
        ) else {
            return false;
        };
        for (axis, value) in self.pad.sticks.iter_mut().enumerate() {
            let centred = stick(frame.get(1 + axis).copied().unwrap_or(0x80));
            // Both Y axes grow downward.
            *value = if axis % 2 == 1 { -centred } else { centred };
        }
        self.pad.triggers = [trigger(left), trigger(right)];
        let (up, right_dir, down, left_dir) = hat(face & 0x0f);
        let buttons = &mut self.pad.buttons;
        buttons[KEY_UP] = up;
        buttons[KEY_RIGHT] = right_dir;
        buttons[KEY_DOWN] = down;
        buttons[KEY_LEFT] = left_dir;
        for (mask, key) in FACE_BUTTONS {
            buttons[key] = face & mask != 0;
        }
        for (mask, key) in MENU_BUTTONS {
            buttons[key] = menu & mask != 0;
        }
        buttons[KEY_PS] = ps & PS_BUTTON != 0;
        buttons[KEY_TOUCHPAD] = ps & TOUCHPAD_BUTTON != 0;
        // Bits past the model's keys (the DS4's frame counter, the Edge's
        // extras on a plain DualSense) are never read.
        if self.kind == Kind::DualSense {
            buttons[KEY_MUTE] = ps & MUTE_BUTTON != 0;
            for (offset, mask) in EDGE_BUTTONS.into_iter().enumerate() {
                buttons[KEY_EDGE + offset] = ps & mask != 0;
            }
        }
        self.pad.clicks = [menu & LEFT_CLICK != 0, menu & RIGHT_CLICK != 0];
        true
    }

    fn read_access(&mut self, frame: &[u8]) -> bool {
        let (Some(&perimeter), Some(&misc), Some(&y), Some(&x)) =
            (frame.get(16), frame.get(17), frame.get(18), frame.get(19))
        else {
            return false;
        };
        let buttons = &mut self.pad.buttons;
        for (bit, held) in buttons.iter_mut().take(8).enumerate() {
            *held = perimeter & (1 << bit) != 0;
        }
        buttons[ACCESS_KEY_CENTER] = misc & ACCESS_CENTER != 0;
        buttons[ACCESS_KEY_PS] = misc & ACCESS_PS != 0;
        buttons[ACCESS_KEY_PROFILE] = misc & ACCESS_PROFILE != 0;
        self.pad.clicks = [misc & ACCESS_STICK_CLICK != 0, false];
        // Already up-positive: 0xFF is up.
        self.pad.sticks = [stick(x), stick(y), 0.0, 0.0];
        true
    }

    /// How far each key is pressed, 0 to 1, after the dead zone.
    fn key_travel(&self, key: usize) -> f32 {
        if self.kind == Kind::Access {
            return f32::from(u8::from(self.pad.buttons[key]));
        }
        match key {
            KEY_L2 => dead_zone(self.pad.triggers[0]),
            KEY_R2 => dead_zone(self.pad.triggers[1]),
            _ if (KEY_STICKS..KEY_MUTE).contains(&key) => {
                let direction = key - KEY_STICKS;
                let stick = direction / 4;
                let (axis, toward) = match direction % 4 {
                    0 => (stick * 2 + 1, 1.0),
                    1 => (stick * 2 + 1, -1.0),
                    2 => (stick * 2, -1.0),
                    _ => (stick * 2, 1.0),
                };
                (dead_zone(self.pad.sticks[axis]) * toward).max(0.0)
            }
            _ => f32::from(u8::from(self.pad.buttons[key])),
        }
    }

    /// The events a new frame raises at `now`.
    fn apply(&mut self, now: Instant) -> Vec<Input> {
        let mut events = Vec::new();
        for key in 0..self.keys {
            let travel = self.key_travel(key);
            let held = self.held[key];
            let pressed = if held {
                travel > PRESS_AT * RELEASE_RATIO
            } else {
                travel >= PRESS_AT
            };
            if pressed != held {
                self.held[key] = pressed;
                events.push(Input::Key {
                    index: key as u8,
                    pressed,
                });
            }
        }
        let dials = [LEFT_CLICK_DIAL, RIGHT_CLICK_DIAL];
        for ((index, pressed), held) in dials
            .into_iter()
            .zip(self.pad.clicks)
            .zip(self.clicks.iter_mut())
        {
            if pressed != *held {
                *held = pressed;
                events.push(Input::DialPress { index, pressed });
            }
        }
        for axis in 0..self.axes {
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
            rotary.next = Some(if next <= now { now + period } else { next });
            events.push(turn(axis, rotary.level));
        }
        events
    }

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

    /// The light colour to send, after brightness.
    fn scaled(&self) -> [u8; 3] {
        let percent = u16::from(self.brightness);
        self.light.unwrap_or([0; 3]).map(|channel| {
            u8::try_from((u16::from(channel) * percent + 50) / 100).unwrap_or(u8::MAX)
        })
    }

    /// The report that sets the light to `rgb` and changes nothing else.
    fn light_report(&self, rgb: [u8; 3]) -> Vec<u8> {
        match self.kind {
            Kind::DualSense => {
                let mut report = vec![0u8; DUALSENSE_OUTPUT_LEN];
                report[0] = DUALSENSE_OUTPUT;
                report[2] = DUALSENSE_FLAG_LIGHTBAR;
                report[45..48].copy_from_slice(&rgb);
                report
            }
            Kind::DualShock4 => {
                let mut report = vec![0u8; DS4_OUTPUT_LEN];
                report[0] = DS4_OUTPUT;
                report[1] = DS4_FLAG_LED;
                report[6..9].copy_from_slice(&rgb);
                report
            }
            Kind::Access => {
                let mut report = vec![0u8; ACCESS_OUTPUT_LEN];
                report[0] = ACCESS_OUTPUT;
                report[1] = ACCESS_MUTATOR_LED;
                report[6..9].copy_from_slice(&rgb);
                report
            }
        }
    }

    /// The reports sent once, before the first colour.
    fn setup_reports(&self) -> Vec<Vec<u8>> {
        match self.kind {
            Kind::DualSense => {
                let mut release = vec![0u8; DUALSENSE_OUTPUT_LEN];
                release[0] = DUALSENSE_OUTPUT;
                release[39] = DUALSENSE_FLAG2_LIGHTBAR_SETUP;
                release[42] = DUALSENSE_LIGHT_OUT;
                // Player LEDs (mask at 44) and the mute LED (mode at 9) off.
                let mut clear = vec![0u8; DUALSENSE_OUTPUT_LEN];
                clear[0] = DUALSENSE_OUTPUT;
                clear[2] = DUALSENSE_FLAG_PLAYER_LEDS | DUALSENSE_FLAG_MIC_LED;
                vec![release, clear]
            }
            // The colour goes with the blink stop (on and off times 0).
            Kind::DualShock4 => Vec::new(),
            Kind::Access => {
                let mut player = vec![0u8; ACCESS_OUTPUT_LEN];
                player[0] = ACCESS_OUTPUT;
                player[1] = ACCESS_MUTATOR_PLAYER;
                vec![player]
            }
        }
    }

    /// Sends the light when it changed, after the setup reports the first time.
    fn send_light(&mut self) -> DriverResult<()> {
        let rgb = self.scaled();
        if self.sent == Some(rgb) {
            return Ok(());
        }
        let mut report = self.light_report(rgb);
        if self.sent.is_none() {
            for setup in self.setup_reports() {
                self.transport.write(&setup)?;
            }
            if self.kind == Kind::DualShock4 {
                report[1] |= DS4_FLAG_BLINK;
            }
        }
        self.transport.write(&report)?;
        self.sent = Some(rgb);
        Ok(())
    }
}

impl Driver for PlayStation {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.poll_at(timeout_ms, Instant::now)
    }

    /// No key screens or key lights: a layout only marks the keys in the Companion.
    fn set_face(&mut self, _key: u8, _face: Option<&Face>) -> DriverResult<()> {
        Ok(())
    }

    /// The light bar, in one colour.
    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        self.light = color;
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.send_light()
    }

    /// No brightness register: Ryu scales the colour.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.send_light()
    }
}

impl Drop for PlayStation {
    /// Leaves the light off when Ryu lets go.
    fn drop(&mut self) {
        if self.sent.is_some_and(|rgb| rgb != [0; 3]) {
            let report = self.light_report([0; 3]);
            let _ = self.transport.write(&report);
        }
    }
}

/// The key names of a PlayStation model, cut to its keys.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    let keys = usize::from(model.keys);
    match model.protocol {
        Protocol::PlayStationAccess => Some(ACCESS_KEY_NAMES.iter().copied().take(keys).collect()),
        Protocol::PlayStationDualShock4 => Some(
            KEY_NAMES
                .iter()
                .enumerate()
                .take(keys)
                .map(|(index, name)| if index == KEY_CREATE { "Share" } else { name })
                .collect(),
        ),
        Protocol::PlayStationDualSense => Some(KEY_NAMES.iter().copied().take(keys).collect()),
        _ => None,
    }
}

/// A u8 stick byte (0x80 centred) as -1 to 1, growing with the byte.
fn stick(raw: u8) -> f32 {
    ((f32::from(raw) - STICK_CENTRE) / STICK_SPAN).clamp(-1.0, 1.0)
}

fn trigger(raw: u8) -> f32 {
    (f32::from(raw) / TRIGGER_MAX).clamp(0.0, 1.0)
}

/// A hat value (0 north, clockwise to 7 north-west; anything above 7 is
/// released) as up, right, down, left.
const fn hat(value: u8) -> (bool, bool, bool, bool) {
    match value {
        0 => (true, false, false, false),
        1 => (true, true, false, false),
        2 => (false, true, false, false),
        3 => (false, true, true, false),
        4 => (false, false, true, false),
        5 => (false, false, true, true),
        6 => (false, false, false, true),
        7 => (true, false, false, true),
        _ => (false, false, false, false),
    }
}

fn turn(axis: usize, level: i8) -> Input {
    Input::DialTurn {
        index: axis as u8,
        delta: level.signum(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn open(id: &str) -> (PlayStation, Recorder) {
        let recorder = Recorder::default();
        (
            PlayStation::new(by_id(id).unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    fn press(index: u8, pressed: bool) -> Input {
        Input::DialPress { index, pressed }
    }

    /// A DualSense USB report: sticks centred unless set, hat released.
    fn dualsense(set: &[(usize, u8)]) -> Vec<u8> {
        let mut frame = vec![0u8; 64];
        frame[0] = 0x01;
        frame[1..5].fill(0x80);
        frame[8] = 0x08;
        for &(at, value) in set {
            frame[at] = value;
        }
        frame
    }

    /// A DualShock 4 USB report: sticks centred unless set, hat released.
    fn ds4(set: &[(usize, u8)]) -> Vec<u8> {
        let mut frame = vec![0u8; 64];
        frame[0] = 0x01;
        frame[1..5].fill(0x80);
        frame[5] = 0x08;
        for &(at, value) in set {
            frame[at] = value;
        }
        frame
    }

    /// An Access controller USB report: raw stick centred unless set.
    fn access(set: &[(usize, u8)]) -> Vec<u8> {
        let mut frame = vec![0u8; 64];
        frame[0] = 0x01;
        frame[1..5].fill(0x80);
        frame[8] = 0x08;
        frame[18] = 0x80;
        frame[19] = 0x80;
        for &(at, value) in set {
            frame[at] = value;
        }
        frame
    }

    #[test]
    fn dualsense_buttons_map_to_their_keys() {
        let (mut pad, recorder) = open("sony-dualsense");
        // Hat 1 (up-right), Square and Triangle; L1, Create, both clicks; PS,
        // touchpad, mute, and every Edge bit (ignored on a plain DualSense).
        recorder.queue_read(dualsense(&[
            (8, 0x01 | 0x10 | 0x80),
            (9, 0x01 | 0x10 | 0x40 | 0x80),
            (10, 0xf7),
        ]));
        recorder.queue_read(dualsense(&[]));
        assert_eq!(
            pad.poll(0).unwrap(),
            vec![
                key(2, true),
                key(3, true),
                key(4, true),
                key(8, true),
                key(11, true),
                key(12, true),
                key(14, true),
                key(15, true),
                key(24, true),
                press(0, true),
                press(2, true),
                key(2, false),
                key(3, false),
                key(4, false),
                key(8, false),
                key(11, false),
                key(12, false),
                key(14, false),
                key(15, false),
                key(24, false),
                press(0, false),
                press(2, false),
            ]
        );
        assert!(recorder.writes().is_empty() && recorder.features().is_empty());
    }

    #[test]
    fn the_other_bits_are_cross_circle_r1_options_and_hat_diagonals() {
        let (mut pad, recorder) = open("sony-dualsense");
        // Hat 5 (down-left), Cross, Circle; R1 and Options. The digital
        // trigger bits alone press nothing.
        recorder.queue_read(dualsense(&[
            (8, 0x05 | 0x20 | 0x40),
            (9, 0x02 | 0x20 | 0x0c),
        ]));
        // Hat 7 is up-left; 8 and 15 are released.
        recorder.queue_read(dualsense(&[(8, 0x07)]));
        recorder.queue_read(dualsense(&[(8, 0x0f)]));
        assert_eq!(
            pad.poll(0).unwrap(),
            vec![
                key(0, true),
                key(1, true),
                key(5, true),
                key(9, true),
                key(10, true),
                key(13, true),
                key(0, false),
                key(1, false),
                key(5, false),
                key(8, true),
                key(9, false),
                key(13, false),
                key(8, false),
                key(10, false),
            ]
        );
    }

    #[test]
    fn the_edge_reads_its_fn_buttons_and_paddles() {
        let (mut pad, recorder) = open("sony-dualsense-edge");
        recorder.queue_read(dualsense(&[(10, 0x10 | 0x80)]));
        recorder.queue_read(dualsense(&[(10, 0x20 | 0x40)]));
        assert_eq!(
            pad.poll(0).unwrap(),
            vec![
                key(25, true),
                key(28, true),
                key(25, false),
                key(26, true),
                key(27, true),
                key(28, false),
            ]
        );
    }

    #[test]
    fn triggers_and_sticks_follow_the_xbox_thresholds() {
        let (mut pad, recorder) = open("sony-dualsense");
        let start = Instant::now();
        // 0.575 of 255 is 146.6: 147 presses, 146 does not.
        recorder.queue_read(dualsense(&[(5, 146)]));
        assert!(pad.poll_at(0, || start).unwrap().is_empty());
        // L2 147, R2 full; left stick full up (Y 0), right stick full right.
        recorder.queue_read(dualsense(&[(5, 147), (6, 255), (2, 0), (3, 255)]));
        assert_eq!(
            pad.poll_at(0, || start).unwrap(),
            vec![
                key(6, true),
                key(7, true),
                key(16, true),
                key(23, true),
                Input::DialTurn { index: 1, delta: 1 },
                Input::DialTurn { index: 2, delta: 1 },
            ]
        );
        // Full travel repeats 15 times a second.
        assert_eq!(
            pad.poll_at(0, || start + Duration::from_millis(67))
                .unwrap(),
            vec![
                Input::DialTurn { index: 1, delta: 1 },
                Input::DialTurn { index: 2, delta: 1 },
            ]
        );
        // L2 holds above 35% (0.4475 of 255 is 114.1: 115 holds, 114 lets go).
        recorder.queue_read(dualsense(&[(5, 115), (6, 255), (2, 0), (3, 255)]));
        assert!(pad
            .poll_at(0, || start + Duration::from_millis(70))
            .unwrap()
            .is_empty());
        recorder.queue_read(dualsense(&[(5, 114)]));
        assert_eq!(
            pad.poll_at(0, || start + Duration::from_millis(80))
                .unwrap(),
            vec![key(6, false), key(7, false), key(16, false), key(23, false)]
        );
        assert!(pad
            .poll_at(0, || start + Duration::from_secs(2))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn dualshock4_reads_its_own_offsets() {
        let (mut pad, recorder) = open("sony-dualshock-4");
        let start = Instant::now();
        // Hat 3 (down-right) and Cross at 5; R1, Share, R3 at 6; PS, touchpad
        // and frame-counter bits at 7; R2 full at 9; left stick pushed down.
        recorder.queue_read(ds4(&[
            (5, 0x03 | 0x20),
            (6, 0x02 | 0x10 | 0x80),
            (7, 0x01 | 0x02 | 0xfc),
            (9, 255),
            (2, 255),
        ]));
        assert_eq!(
            pad.poll_at(0, || start).unwrap(),
            vec![
                key(0, true),
                key(5, true),
                key(7, true),
                key(9, true),
                key(11, true),
                key(12, true),
                key(14, true),
                key(15, true),
                key(17, true),
                press(2, true),
                Input::DialTurn {
                    index: 1,
                    delta: -1
                },
            ]
        );
        assert_eq!(pad.keys, 24);
    }

    #[test]
    fn the_access_controller_reads_raw_buttons_not_the_mapped_header() {
        let (mut pad, recorder) = open("sony-access-controller");
        let start = Instant::now();
        // The mapped header says Cross and L1; only the raw bytes count: B1,
        // B8, centre, stick click, PS, profile; stick up (0xFF) and left.
        recorder.queue_read(access(&[
            (8, 0x28),
            (9, 0x01),
            (16, 0x81),
            (17, 0x0f),
            (18, 0xff),
            (19, 0x00),
        ]));
        assert_eq!(
            pad.poll_at(0, || start).unwrap(),
            vec![
                key(0, true),
                key(7, true),
                key(8, true),
                key(9, true),
                key(10, true),
                press(0, true),
                Input::DialTurn {
                    index: 0,
                    delta: -1
                },
                Input::DialTurn { index: 1, delta: 1 },
            ]
        );
        recorder.queue_read(access(&[(16, 0x7e)]));
        assert_eq!(
            pad.poll_at(0, || start).unwrap(),
            vec![
                key(0, false),
                key(1, true),
                key(2, true),
                key(3, true),
                key(4, true),
                key(5, true),
                key(6, true),
                key(7, false),
                key(8, false),
                key(9, false),
                key(10, false),
                press(0, false),
            ]
        );
    }

    #[test]
    fn short_and_other_frames_are_ignored() {
        let (mut pad, recorder) = open("sony-dualsense-edge");
        // The reduced Bluetooth 0x01, a Bluetooth 0x31, a truncated USB report.
        recorder.queue_read(vec![
            0x01, 0x80, 0x80, 0x80, 0x80, 0xff, 0xff, 0xff, 0xff, 0xff,
        ]);
        let mut bluetooth = dualsense(&[(9, 0xff), (10, 0xff)]);
        bluetooth[0] = 0x31;
        bluetooth.resize(78, 0xff);
        recorder.queue_read(bluetooth);
        recorder.queue_read(dualsense(&[(9, 0xff)])[..63].to_vec());
        recorder.queue_read(vec![]);
        assert!(pad.poll(0).unwrap().is_empty());
        assert!(pad.set_face(0, None).is_ok() && pad.heartbeat().is_ok());
    }

    #[test]
    fn dualsense_light_releases_the_firmware_light_then_sets_the_colour() {
        let (mut pad, recorder) = open("sony-dualsense");
        pad.set_deck_light(Some([200, 100, 50])).unwrap();
        pad.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 3);
        assert!(writes
            .iter()
            .all(|report| report.len() == 48 && report[0] == 0x02));
        let mut release = vec![0u8; 48];
        release[0] = 0x02;
        release[39] = 0x02;
        release[42] = 0x02;
        assert_eq!(writes[0], release);
        let mut clear = vec![0u8; 48];
        clear[0] = 0x02;
        clear[2] = 0x11;
        assert_eq!(writes[1], clear);
        let mut light = vec![0u8; 48];
        light[0] = 0x02;
        light[2] = 0x04;
        light[45..48].copy_from_slice(&[200, 100, 50]);
        assert_eq!(writes[2], light);
        // Unchanged: nothing more. Half brightness scales the colour.
        pad.flush().unwrap();
        assert_eq!(recorder.writes().len(), 3);
        pad.set_brightness(50).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 4);
        assert_eq!(&writes[3][45..48], &[100, 50, 25]);
        assert_eq!(writes[3][1], 0, "rumble is never flagged");
        pad.set_deck_light(None).unwrap();
        pad.flush().unwrap();
        assert_eq!(&recorder.writes()[4][45..48], &[0, 0, 0]);
        // Already dark: letting go sends nothing more.
        drop(pad);
        assert_eq!(recorder.writes().len(), 5);
    }

    #[test]
    fn dualshock4_light_stops_the_blink_once_and_never_flags_rumble() {
        let (mut pad, recorder) = open("sony-dualshock-4");
        pad.set_deck_light(Some([255, 0, 128])).unwrap();
        pad.flush().unwrap();
        pad.set_deck_light(Some([0, 255, 0])).unwrap();
        pad.flush().unwrap();
        drop(pad);
        let writes = recorder.writes();
        let report = |flags: u8, rgb: [u8; 3]| {
            let mut report = vec![0u8; 32];
            report[0] = 0x05;
            report[1] = flags;
            report[6..9].copy_from_slice(&rgb);
            report
        };
        assert_eq!(
            writes,
            vec![
                report(0x06, [255, 0, 128]),
                report(0x02, [0, 255, 0]),
                report(0x02, [0, 0, 0]),
            ]
        );
    }

    #[test]
    fn access_light_turns_the_player_indicator_off_first() {
        let (mut pad, recorder) = open("sony-access-controller");
        pad.set_brightness(0).unwrap();
        pad.set_deck_light(Some([255, 255, 255])).unwrap();
        pad.flush().unwrap();
        pad.set_brightness(100).unwrap();
        drop(pad);
        let writes = recorder.writes();
        let mut player = vec![0u8; 32];
        player[0] = 0x02;
        player[1] = 0x10;
        let light = |rgb: [u8; 3]| {
            let mut report = vec![0u8; 32];
            report[0] = 0x02;
            report[1] = 0x04;
            report[6..9].copy_from_slice(&rgb);
            report
        };
        assert_eq!(
            writes,
            vec![
                player,
                light([0, 0, 0]),
                light([255, 255, 255]),
                light([0, 0, 0]),
            ]
        );
    }

    #[test]
    fn key_names_are_cut_to_each_model() {
        let names = |id: &str| key_names(by_id(id).unwrap()).unwrap();
        let ds4 = names("sony-dualshock-4");
        assert_eq!((ds4.len(), ds4[12], ds4[23]), (24, "Share", "RS right"));
        let dualsense = names("sony-dualsense");
        assert_eq!(
            (dualsense.len(), dualsense[12], dualsense[24]),
            (25, "Create", "Mute")
        );
        let edge = names("sony-dualsense-edge");
        assert_eq!(
            (edge.len(), edge[0], edge[28]),
            (29, "Cross", "Right paddle")
        );
        let access = names("sony-access-controller");
        assert_eq!((access.len(), access[0], access[10]), (11, "B1", "Profile"));
        assert!(key_names(by_id("microsoft-xbox-series").unwrap()).is_none());
    }
}
