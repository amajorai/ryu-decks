//! GameSir Cyclone 2 controllers, through the vendor configuration collection
//! (usage page 0xFFF0, usage 0x40) that GameSir Connect uses. The XInput
//! gamepad interface and the keyboard, mouse and consumer collections beside
//! it are never opened.
//!
//! Every report is 64 bytes and starts with its id. The host sends `0F cmd
//! args..` padded with zeros; the controller answers with `10 cmd ..` and
//! streams its state as `12 ..`, but only while a keep-alive (`0F F2 00`)
//! arrives at least once a second. Ryu sends one when the controller opens
//! and then from every poll at which a second has passed (the manager's
//! heartbeat is only every two seconds).
//!
//! The state report (`12`, 63 bytes or more; offsets count the id as byte 0)
//! carries the inputs twice: bytes 1 to 9 after the active profile's remaps,
//! and 54 to 62 as physically pressed. Ryu reads the physical block, so a
//! remap made in GameSir Connect does not change which Ryu key fires:
//!
//! - 54 to 57: left X, left Y, right X, right Y, 0 to 255 centred on 128,
//!   Y growing downward (Ryu flips it so up is positive, as on the Xbox pad);
//! - 58: the d-pad hat in bits 0 to 3 (0 up, clockwise to 7 up-left, 15
//!   released), then X, A, B, Y in bits 4 to 7;
//! - 59: LB, RB, LT, RT (digital), View, Menu, left and right stick click;
//! - 60: Home, Capture, then L4, R4 and M in bits 3 to 5;
//! - 61 and 62: the analog triggers (unused: the digital bits are read).
//!
//! Byte 37 is the light animation playing (0 to 3). The 27 keys follow the
//! Xbox driver's order and thresholds: the face buttons, shoulders and
//! triggers, the d-pad, View, Menu, Home and Capture, the back buttons L4, R4
//! and M, then each stick's four directions (held past half its travel after
//! a 15% dead zone, let go below 35%). The stick axes are four dials that
//! repeat while held off centre; the stick clicks press dials 0 and 2.
//!
//! The deck light is written into the controller's light profile (profile 32),
//! which keeps four animations of 124 bytes from offset 1: a key-frame count,
//! 5, `20 - speed`, a brightness of 0 to 100, then eight frames of five RGB
//! zones (lower left, lower right, upper left, Home, upper right). Writes are
//! `0F 03 20 offset(BE) len data` (at most 58 bytes), answered `10 06 00` (or
//! `10 06 01`, busy); reads are `0F 04 20 offset len`, answered `10 05 20
//! offset len data`. One command is in flight at a time, resent after 200 ms
//! and given up after ten sends. State reports that arrive meanwhile are kept
//! for the next poll.
//!
//! The profile is stored on the controller, so Ryu writes only on a change.
//! Before the first write it reads the playing animation's 124 bytes; the
//! colour then replaces that animation with one static frame (`01 05 14`
//! and the brightness), and the original is written back when the deck light
//! is cleared, when the controller switches to another animation (then the
//! new one is backed up and coloured) and when Ryu lets go. Brightness is the
//! animation's own brightness byte. With no deck light set, Ryu never writes
//! to the controller's lights. The game profiles (1 to 5) are never touched.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::xbox::{dead_zone, repeat_period, rotary_level, PRESS_AT, RELEASE_RATIO};
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const REPORT_LEN: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 64;
/// At most this many changed states are kept while a command waits.
const MAX_PENDING_STATES: usize = 64;

const HOST_REPORT: u8 = 0x0f;
const ANSWER_REPORT: u8 = 0x10;
const STATE_REPORT: u8 = 0x12;
/// The shortest state report that reaches the physical block's last byte.
const STATE_LEN: usize = 63;

const CMD_WRITE: u8 = 0x03;
const CMD_READ: u8 = 0x04;
const ANSWER_READ: u8 = 0x05;
const ANSWER_WRITE: u8 = 0x06;
const CMD_LIGHT_STATUS: u8 = 0x0d;
const CMD_KEEP_ALIVE: u8 = 0xf2;

/// The light profile's number.
const LIGHT_PROFILE: u8 = 0x20;
const ANIMATIONS: u8 = 4;
const ANIMATION_LEN: usize = 124;
/// The most data one read or write carries.
const CHUNK: usize = 58;
/// The animation brightness byte's offset in an animation.
const BRIGHTNESS_AT: u16 = 3;
/// The playing animation's index in the state report.
const ANIMATION_AT: usize = 37;
/// The physical input block's first byte in the state report.
const PHYSICAL: usize = 54;
const BLOCK_LEN: usize = 9;

const KEEP_ALIVE_EVERY: Duration = Duration::from_secs(1);
const ANSWER_WAIT: Duration = Duration::from_millis(200);
const SENDS: usize = 10;
/// After a light write went unanswered, no light write is tried for this
/// long (each unanswered request holds the device thread for up to two
/// seconds), doubling with every further failure up to [`MAX_REST_DOUBLINGS`].
const RETRY_AFTER: Duration = Duration::from_secs(5);
const MAX_REST_DOUBLINGS: u32 = 6;

/// The key names, in key order: three rows of nine.
pub const KEY_NAMES: [&str; 27] = [
    "A", "B", "X", "Y", "LB", "RB", "LT", "RT", "Up", "Down", "Left", "Right", "View", "Menu",
    "Home", "Capture", "L4", "R4", "M", "LS up", "LS down", "LS left", "LS right", "RS up",
    "RS down", "RS left", "RS right",
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
const KEY_HOME: usize = 14;
const KEY_CAPTURE: usize = 15;
const KEY_L4: usize = 16;
const KEY_R4: usize = 17;
const KEY_M: usize = 18;
/// The first stick-direction key: left stick up, down, left, right, then the
/// right stick's.
const KEY_STICKS: usize = 19;
const KEYS: usize = 27;

/// Block byte 4's high nibble, as (mask, key); its low nibble is the hat.
const FACE_BUTTONS: [(u8, usize); 4] = [(0x10, KEY_X), (0x20, KEY_A), (0x40, KEY_B), (0x80, KEY_Y)];
/// Block byte 5, as (mask, key); 0x40 and 0x80 are the stick clicks.
const MENU_BUTTONS: [(u8, usize); 6] = [
    (0x01, KEY_LB),
    (0x02, KEY_RB),
    (0x04, KEY_LT),
    (0x08, KEY_RT),
    (0x10, KEY_VIEW),
    (0x20, KEY_MENU),
];
const LEFT_CLICK: u8 = 0x40;
const RIGHT_CLICK: u8 = 0x80;
/// Block byte 6, as (mask, key).
const EXTRA_BUTTONS: [(u8, usize); 5] = [
    (0x01, KEY_HOME),
    (0x02, KEY_CAPTURE),
    (0x08, KEY_L4),
    (0x10, KEY_R4),
    (0x20, KEY_M),
];

const AXES: usize = 4;
const LEFT_CLICK_DIAL: u8 = 0;
const RIGHT_CLICK_DIAL: u8 = 2;

/// What the controller last reported, before thresholds.
#[derive(Debug, Clone, Copy, Default)]
struct Pad {
    /// The digital buttons, by key index (stick keys unused).
    buttons: [bool; KEYS],
    left_click: bool,
    right_click: bool,
    /// Left X, left Y, right X, right Y, -1 to 1, right and up positive.
    sticks: [f32; AXES],
}

impl Pad {
    /// The pad a physical input block describes.
    fn from_block(block: &[u8; BLOCK_LEN]) -> Self {
        let mut pad = Self::default();
        for (axis, value) in pad.sticks.iter_mut().enumerate() {
            let centred = stick(block[axis]);
            // Y grows downward on the wire.
            *value = if axis % 2 == 1 { -centred } else { centred };
        }
        let (up, right, down, left) = match block[4] & 0x0f {
            0 => (true, false, false, false),
            1 => (true, true, false, false),
            2 => (false, true, false, false),
            3 => (false, true, true, false),
            4 => (false, false, true, false),
            5 => (false, false, true, true),
            6 => (false, false, false, true),
            7 => (true, false, false, true),
            _ => (false, false, false, false),
        };
        pad.buttons[KEY_UP] = up;
        pad.buttons[KEY_RIGHT] = right;
        pad.buttons[KEY_DOWN] = down;
        pad.buttons[KEY_LEFT] = left;
        for (mask, key) in FACE_BUTTONS {
            pad.buttons[key] = block[4] & mask != 0;
        }
        for (mask, key) in MENU_BUTTONS {
            pad.buttons[key] = block[5] & mask != 0;
        }
        for (mask, key) in EXTRA_BUTTONS {
            pad.buttons[key] = block[6] & mask != 0;
        }
        pad.left_click = block[5] & LEFT_CLICK != 0;
        pad.right_click = block[5] & RIGHT_CLICK != 0;
        pad
    }
}

/// One stick axis's repeating dial turn.
#[derive(Debug, Clone, Copy, Default)]
struct Rotary {
    level: i8,
    next: Option<Instant>,
}

/// What Ryu last wrote into the light profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Written {
    slot: u8,
    rgb: [u8; 3],
    brightness: u8,
}

pub struct GameSir {
    transport: Box<dyn Transport>,
    pad: Pad,
    keys: [bool; KEYS],
    clicks: [bool; 2],
    rotaries: [Rotary; AXES],
    last_keep_alive: Option<Instant>,
    /// The last physical block seen, applied or still pending.
    seen: Option<[u8; BLOCK_LEN]>,
    /// Changed physical blocks not yet turned into events.
    pending: VecDeque<[u8; BLOCK_LEN]>,
    /// The animation the controller is playing, from the latest state report.
    animation: Option<u8>,
    /// The deck light Ryu wants; `None` leaves the controller's own lights.
    light: Option<[u8; 3]>,
    brightness: u8,
    written: Option<Written>,
    /// The controller changed the light profile itself since Ryu's last write,
    /// so the next write is a whole one.
    stale: bool,
    /// The animation slot Ryu changed and its original 124 bytes.
    backup: Option<(u8, Vec<u8>)>,
    /// Where the backup is also kept on disk while Ryu's colour is on the
    /// controller, so a killed sidecar or an unplugged controller cannot lose
    /// the user's animation. `None` keeps it in memory only.
    backup_dir: Option<PathBuf>,
    /// A backup left on disk by an earlier run has been looked for.
    recovered: bool,
    /// When a light write last went unanswered.
    failed_at: Option<Instant>,
    /// Light writes that went unanswered in a row.
    failures: u32,
}

impl GameSir {
    /// Opens the state stream with a first keep-alive.
    #[must_use]
    pub fn new(_model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let mut pad = Self {
            transport,
            pad: Pad::default(),
            keys: [false; KEYS],
            clicks: [false; 2],
            rotaries: [Rotary::default(); AXES],
            last_keep_alive: None,
            seen: None,
            pending: VecDeque::new(),
            animation: None,
            light: None,
            brightness: 100,
            written: None,
            stale: false,
            backup: None,
            backup_dir: None,
            recovered: false,
            failed_at: None,
            failures: 0,
        };
        // A failed first send is retried by the first poll.
        let _ = pad.keep_alive(Instant::now());
        pad
    }

    /// Keeps the original animation in `dir` while Ryu's colour replaces it.
    #[must_use]
    pub fn with_backup_dir(mut self, dir: PathBuf) -> Self {
        self.backup_dir = Some(dir);
        self
    }

    fn backup_path(&self, slot: u8) -> Option<PathBuf> {
        self.backup_dir
            .as_ref()
            .map(|dir| dir.join(format!("light-slot-{slot}.bin")))
    }

    /// The original animation an earlier run kept on disk for `slot`.
    fn saved_backup(&self, slot: u8) -> Option<Vec<u8>> {
        let data = std::fs::read(self.backup_path(slot)?).ok()?;
        (data.len() == ANIMATION_LEN).then_some(data)
    }

    /// Best effort: a disk that cannot be written still leaves the in-memory
    /// backup working.
    fn save_backup(&self, slot: u8, data: &[u8]) {
        let (Some(dir), Some(path)) = (self.backup_dir.as_ref(), self.backup_path(slot)) else {
            return;
        };
        if let Err(error) = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&path, data))
        {
            tracing::warn!(%error, "ryu-decks: could not keep the GameSir light backup");
        }
    }

    fn forget_backup(&self, slot: u8) {
        if let Some(path) = self.backup_path(slot) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Picks up a backup an earlier run left on disk when the controller still
    /// plays Ryu's colour (the sidecar was killed or the controller unplugged
    /// before the original went back). A slot that no longer holds Ryu's frame
    /// was changed since, so its file is stale. Whether the check finished.
    fn recover(&mut self) -> DriverResult<bool> {
        if self.recovered || self.backup_dir.is_none() || self.backup.is_some() {
            return Ok(true);
        }
        for slot in 0..ANIMATIONS {
            let Some(saved) = self.saved_backup(slot) else {
                continue;
            };
            let Some(current) = self.read_animation(slot)? else {
                return Ok(false);
            };
            if is_ryu_frame(&current) {
                self.backup = Some((slot, saved));
                break;
            }
            self.forget_backup(slot);
        }
        self.recovered = true;
        Ok(true)
    }

    /// Sends the keep-alive when a second has passed since the last one.
    fn keep_alive(&mut self, now: Instant) -> DriverResult<()> {
        if self
            .last_keep_alive
            .is_some_and(|last| now.saturating_duration_since(last) < KEEP_ALIVE_EVERY)
        {
            return Ok(());
        }
        self.transport.write(&packet(CMD_KEEP_ALIVE, &[0x00]))?;
        self.last_keep_alive = Some(now);
        Ok(())
    }

    /// Takes a report that is not an answer: a state report's playing
    /// animation and, when it changed, its physical block; a notification
    /// that the controller changed the light profile itself.
    fn take_report(&mut self, frame: &[u8]) {
        match frame.first() {
            Some(&STATE_REPORT) if frame.len() >= STATE_LEN => {
                if let Some(&animation) = frame.get(ANIMATION_AT) {
                    self.animation = (animation < ANIMATIONS).then_some(animation);
                }
                let Some(block) = frame
                    .get(PHYSICAL..PHYSICAL + BLOCK_LEN)
                    .and_then(|block| <[u8; BLOCK_LEN]>::try_from(block).ok())
                else {
                    return;
                };
                if self.seen != Some(block) {
                    self.seen = Some(block);
                    if self.pending.len() == MAX_PENDING_STATES {
                        self.pending.pop_front();
                    }
                    self.pending.push_back(block);
                }
            }
            // `10 0F n ..`: the controller changed bytes of profile n itself.
            Some(&ANSWER_REPORT)
                if frame.get(1) == Some(&0x0f) && frame.get(2) == Some(&LIGHT_PROFILE) =>
            {
                // What Ryu wrote may be gone: the next light change rewrites it whole.
                self.stale = true;
            }
            _ => {}
        }
    }

    /// How far each key is pressed, 0 to 1, after the dead zone.
    fn key_travel(&self, key: usize) -> f32 {
        if key < KEY_STICKS {
            return f32::from(u8::from(self.pad.buttons[key]));
        }
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

    /// The events the current pad raises at `now`.
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
            rotary.next = Some(now + repeat_period(level));
            if previous == 0 || previous.signum() != level.signum() {
                events.push(turn(axis, level));
            }
        }
        events
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

    /// How long the first read may wait: until the next repeat turn or
    /// keep-alive is due, and never past `timeout_ms`.
    fn first_wait(&self, timeout_ms: i32, now: Instant) -> i32 {
        let keep_alive = self.last_keep_alive.map_or(Duration::ZERO, |last| {
            (last + KEEP_ALIVE_EVERY).saturating_duration_since(now)
        });
        let wait = self
            .rotaries
            .iter()
            .filter_map(|rotary| rotary.next)
            .map(|due| due.saturating_duration_since(now))
            .fold(keep_alive, Duration::min);
        i32::try_from(wait.as_millis())
            .unwrap_or(i32::MAX)
            .min(timeout_ms.max(0))
    }

    fn poll_at(&mut self, timeout_ms: i32, now: impl Fn() -> Instant) -> DriverResult<Vec<Input>> {
        self.keep_alive(now())?;
        // The controller switched animations (M and the left stick) since the
        // last poll: move Ryu's colour to the one now playing.
        let switched = self
            .written
            .zip(self.animation)
            .is_some_and(|(written, playing)| written.slot != playing);
        // A light write that went unanswered is tried again once it has rested.
        if switched || self.failed_at.is_some() {
            self.send_light(now())?;
        }
        let mut buf = [0u8; REPORT_LEN];
        let first_wait = self.first_wait(timeout_ms, now());
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { first_wait } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            if let Some(frame) = buf.get(..read) {
                self.take_report(frame);
            }
        }
        let mut events = self.drain_pending(now());
        events.extend(self.due_turns(now()));
        Ok(events)
    }

    /// Turns every pending state into events, oldest first.
    fn drain_pending(&mut self, now: Instant) -> Vec<Input> {
        let mut events = Vec::new();
        while let Some(block) = self.pending.pop_front() {
            self.pad = Pad::from_block(&block);
            events.extend(self.apply(now));
        }
        events
    }

    /// Sends one command and waits for the answer `matches` accepts, resending
    /// it every 200 ms, ten times at most. `None` when nothing answered.
    fn request(
        &mut self,
        command: &[u8],
        matches: impl Fn(&[u8]) -> bool,
    ) -> DriverResult<Option<Vec<u8>>> {
        let mut buf = [0u8; REPORT_LEN];
        for _ in 0..SENDS {
            self.transport.write(command)?;
            let deadline = Instant::now() + ANSWER_WAIT;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                let wait = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
                let read = self.transport.read_timeout(&mut buf, wait)?;
                if read == 0 {
                    break;
                }
                let Some(frame) = buf.get(..read) else {
                    continue;
                };
                if matches(frame) {
                    return Ok(Some(frame.to_vec()));
                }
                // State reports and notifications are kept; a busy answer
                // (`10 06 01`) and anything else just waits on.
                self.take_report(frame);
            }
        }
        Ok(None)
    }

    /// Reads `len` bytes (58 at most) of the light profile at `offset`.
    fn read_chunk(&mut self, offset: u16, len: usize) -> DriverResult<Option<Vec<u8>>> {
        let [high, low] = offset.to_be_bytes();
        let size = len as u8;
        let header = [ANSWER_REPORT, ANSWER_READ, LIGHT_PROFILE, high, low, size];
        let answer = self.request(
            &packet(CMD_READ, &[LIGHT_PROFILE, high, low, size]),
            |frame| frame.get(..6) == Some(&header[..]) && frame.len() >= 6 + len,
        )?;
        Ok(answer.and_then(|frame| frame.get(6..6 + len).map(<[u8]>::to_vec)))
    }

    /// Writes `data` (58 bytes at most) into the light profile at `offset`.
    /// Whether the controller took it.
    fn write_chunk(&mut self, offset: u16, data: &[u8]) -> DriverResult<bool> {
        let [high, low] = offset.to_be_bytes();
        let mut args = vec![LIGHT_PROFILE, high, low, data.len() as u8];
        args.extend_from_slice(data);
        let answer = self.request(&packet(CMD_WRITE, &args), |frame| {
            frame.get(..3) == Some(&[ANSWER_REPORT, ANSWER_WRITE, 0x00][..])
        })?;
        Ok(answer.is_some())
    }

    /// The animation playing: from the state report, or read from the profile.
    fn playing(&mut self) -> DriverResult<Option<u8>> {
        if let Some(animation) = self.animation {
            return Ok(Some(animation));
        }
        let read = self.read_chunk(0, 1)?;
        Ok(read
            .and_then(|data| data.first().copied())
            .filter(|animation| *animation < ANIMATIONS))
    }

    /// Reads an animation's 124 bytes in chunks of 58, 58 and 8.
    fn read_animation(&mut self, slot: u8) -> DriverResult<Option<Vec<u8>>> {
        let start = animation_offset(slot);
        let mut data = Vec::with_capacity(ANIMATION_LEN);
        while data.len() < ANIMATION_LEN {
            let len = (ANIMATION_LEN - data.len()).min(CHUNK);
            let Some(chunk) = self.read_chunk(start + data.len() as u16, len)? else {
                return Ok(None);
            };
            data.extend(chunk);
        }
        Ok(Some(data))
    }

    /// Writes the backed-up animation back and lets the controller play it.
    /// Whether it went back (or there was nothing to put back).
    fn restore(&mut self) -> DriverResult<bool> {
        let Some((slot, data)) = self.backup.clone() else {
            return Ok(true);
        };
        let start = animation_offset(slot);
        for (index, chunk) in data.chunks(CHUNK).enumerate() {
            if !self.write_chunk(start + (index * CHUNK) as u16, chunk)? {
                return Ok(false);
            }
        }
        // Play (01) from the first key frame (1-based); no answer comes.
        self.transport
            .write(&packet(CMD_LIGHT_STATUS, &[0x01, 0x01]))?;
        self.forget_backup(slot);
        self.backup = None;
        self.written = None;
        self.stale = false;
        Ok(true)
    }

    /// Brings the light profile in line with the deck light, writing only
    /// what changed.
    /// Nothing is sent while an unanswered write rests: a controller that
    /// does not answer must not stall every other deck on each repaint.
    fn send_light(&mut self, now: Instant) -> DriverResult<()> {
        if self.resting(now) {
            return Ok(());
        }
        if self.try_send_light()? {
            self.failed_at = None;
            self.failures = 0;
        } else {
            self.failed_at = Some(now);
            self.failures = self.failures.saturating_add(1);
        }
        Ok(())
    }

    /// Whether the last light write went unanswered too recently to try again.
    fn resting(&self, now: Instant) -> bool {
        let doublings = self.failures.saturating_sub(1).min(MAX_REST_DOUBLINGS);
        let rest = RETRY_AFTER * 2u32.pow(doublings);
        self.failed_at
            .is_some_and(|at| now.saturating_duration_since(at) < rest)
    }

    /// Whether the controller took every write it needed.
    fn try_send_light(&mut self) -> DriverResult<bool> {
        let Some(rgb) = self.light else {
            if !self.recover()? {
                return Ok(false);
            }
            return self.restore();
        };
        let Some(slot) = self.playing()? else {
            return Ok(false);
        };
        let wanted = Written {
            slot,
            rgb,
            brightness: self.brightness,
        };
        if self.written == Some(wanted) && !self.stale {
            return Ok(true);
        }
        if self
            .backup
            .as_ref()
            .is_some_and(|(backed, _)| *backed != slot)
            && !self.restore()?
        {
            return Ok(false);
        }
        if self.backup.is_none() {
            // Never write over an animation that could not be backed up.
            let Some(read) = self.read_animation(slot)? else {
                return Ok(false);
            };
            // Ryu's own frame in the slot means an earlier run was cut off before
            // it put the original back: keep that run's backup, not our colour.
            let saved = self.saved_backup(slot);
            let original = match saved {
                Some(saved) if is_ryu_frame(&read) => saved,
                _ => {
                    self.save_backup(slot, &read);
                    read
                }
            };
            self.backup = Some((slot, original));
        }
        let start = animation_offset(slot);
        let only_brightness = !self.stale
            && self
                .written
                .is_some_and(|written| written.slot == slot && written.rgb == rgb);
        if only_brightness {
            if !self.write_chunk(start + BRIGHTNESS_AT, &[self.brightness])? {
                return Ok(false);
            }
        } else {
            // One key frame, five zones, speed 0 (static), the brightness,
            // then the colour in every zone.
            let mut frame = vec![0x01, 0x05, 0x14, self.brightness];
            for _ in 0..5 {
                frame.extend_from_slice(&rgb);
            }
            if !self.write_chunk(start, &frame)? {
                return Ok(false);
            }
            // Hold the first key frame (1-based) so it shows at once.
            self.transport
                .write(&packet(CMD_LIGHT_STATUS, &[0x00, 0x01]))?;
        }
        self.written = Some(wanted);
        self.stale = false;
        Ok(true)
    }
}

impl Driver for GameSir {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.poll_at(timeout_ms, Instant::now)
    }

    /// No key screens or key lights: a layout only marks the keys in the Companion.
    fn set_face(&mut self, _key: u8, _face: Option<&Face>) -> DriverResult<()> {
        Ok(())
    }

    /// One colour for the controller's light zones; `None` gives the
    /// controller its own animation back.
    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        self.light = color;
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.send_light(Instant::now())
    }

    /// The animation's own brightness byte, 0 to 100, while Ryu's colour is on.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.send_light(Instant::now())
    }

    fn heartbeat(&mut self) -> DriverResult<()> {
        self.keep_alive(Instant::now())
    }
}

impl Drop for GameSir {
    /// Puts the controller's own animation back. A controller that has gone
    /// (its dongle lost it) fails the write, and nothing more is tried.
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Whether an animation is the static frame Ryu writes: one key frame, speed
/// `0x14` and the same colour in all five zones.
fn is_ryu_frame(data: &[u8]) -> bool {
    let Some(zones) = data.get(4..19) else {
        return false;
    };
    data[..3] == [0x01, 0x05, 0x14] && zones.chunks(3).all(|zone| zone == &zones[..3])
}

/// A host report: `0F cmd args..`, zero-padded to 64 bytes.
fn packet(command: u8, args: &[u8]) -> Vec<u8> {
    let mut report = vec![0u8; REPORT_LEN];
    report[0] = HOST_REPORT;
    report[1] = command;
    for (at, byte) in report.iter_mut().skip(2).zip(args) {
        *at = *byte;
    }
    report
}

/// Where an animation starts in the light profile.
fn animation_offset(slot: u8) -> u16 {
    1 + ANIMATION_LEN as u16 * u16::from(slot)
}

/// A stick byte (128 centred) as -1 to 1, growing with the byte.
fn stick(raw: u8) -> f32 {
    let offset = f32::from(raw) - 128.0;
    let span = if raw > 128 { 127.0 } else { 128.0 };
    (offset / span).clamp(-1.0, 1.0)
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

    fn driver() -> (GameSir, Recorder) {
        let recorder = Recorder::default();
        (
            GameSir::new(
                by_id("gamesir-cyclone-2").unwrap(),
                Box::new(recorder.clone()),
            ),
            recorder,
        )
    }

    /// A state report: `physical` at 54 to 62, `mapped` at 1 to 9, the
    /// playing animation at 37.
    fn state(physical: [u8; 9], mapped: [u8; 9], animation: u8) -> Vec<u8> {
        let mut frame = vec![0u8; 64];
        frame[0] = 0x12;
        frame[1..10].copy_from_slice(&mapped);
        frame[37] = animation;
        frame[54..63].copy_from_slice(&physical);
        frame
    }

    /// A physical block: sticks centred, the hat released, then three button
    /// bytes.
    fn block(face: u8, menu: u8, extra: u8) -> [u8; 9] {
        [128, 128, 128, 128, 0x0f | face, menu, extra, 0, 0]
    }

    fn idle() -> [u8; 9] {
        block(0, 0, 0)
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    fn host(bytes: &[u8]) -> Vec<u8> {
        let mut report = bytes.to_vec();
        report.resize(64, 0);
        report
    }

    fn answer(bytes: &[u8]) -> Vec<u8> {
        host(bytes)
    }

    const KEEP_ALIVE: [u8; 3] = [0x0f, 0xf2, 0x00];

    #[test]
    fn a_keep_alive_goes_out_on_open_and_then_every_second() {
        let (mut pad, recorder) = driver();
        assert_eq!(recorder.writes(), vec![host(&KEEP_ALIVE)]);
        let start = pad.last_keep_alive.unwrap();
        pad.poll_at(0, || start + Duration::from_millis(999))
            .unwrap();
        assert_eq!(recorder.writes().len(), 1);
        pad.poll_at(0, || start + Duration::from_millis(1000))
            .unwrap();
        assert_eq!(recorder.writes().len(), 2);
        // The manager's heartbeat shares the same clock: none is due yet.
        pad.heartbeat().unwrap();
        assert_eq!(recorder.writes().len(), 2);
        pad.poll_at(0, || start + Duration::from_millis(2100))
            .unwrap();
        assert_eq!(recorder.writes(), vec![host(&KEEP_ALIVE); 3]);
    }

    #[test]
    fn buttons_come_from_the_physical_block_not_the_remapped_one() {
        let (mut pad, recorder) = driver();
        // Physically A (0x20) and View (0x10); the profile remaps A to B.
        recorder.queue_read(state(block(0x20, 0x10, 0), block(0x40, 0x10, 0), 0));
        recorder.queue_read(state(idle(), idle(), 0));
        assert_eq!(
            pad.poll(0).unwrap(),
            vec![key(0, true), key(12, true), key(0, false), key(12, false)]
        );
    }

    #[test]
    fn every_button_bit_names_its_key() {
        let (mut pad, recorder) = driver();
        // X, A, B, Y; LB, RB, LT, RT, View, Menu; Home, Capture, L4, R4, M.
        recorder.queue_read(state(block(0xf0, 0x3f, 0x3b), idle(), 0));
        assert_eq!(
            pad.poll(0).unwrap(),
            [0, 1, 2, 3, 4, 5, 6, 7, 12, 13, 14, 15, 16, 17, 18]
                .into_iter()
                .map(|index| key(index, true))
                .collect::<Vec<_>>()
        );
        // Byte 60 bit 2 is nothing.
        recorder.queue_read(state(block(0, 0, 0x04), idle(), 0));
        assert_eq!(
            pad.poll(0).unwrap(),
            [0, 1, 2, 3, 4, 5, 6, 7, 12, 13, 14, 15, 16, 17, 18]
                .into_iter()
                .map(|index| key(index, false))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_hat_presses_the_d_pad_keys() {
        let (mut pad, recorder) = driver();
        let hat = |value: u8| {
            let mut physical = idle();
            physical[4] = value;
            state(physical, idle(), 0)
        };
        recorder.queue_read(hat(1));
        recorder.queue_read(hat(5));
        recorder.queue_read(hat(0x0f));
        assert_eq!(
            pad.poll(0).unwrap(),
            vec![
                key(8, true),
                key(11, true),
                key(8, false),
                key(9, true),
                key(10, true),
                key(11, false),
                key(9, false),
                key(10, false),
            ]
        );
    }

    #[test]
    fn sticks_are_up_positive_keys_and_dials_and_clicks_press_dials() {
        let (mut pad, recorder) = driver();
        let start = Instant::now();
        // Left stick full up (raw 0: Y grows downward), right stick full right,
        // both clicks.
        recorder.queue_read(state([128, 0, 255, 128, 0x0f, 0xc0, 0, 0, 0], idle(), 0));
        assert_eq!(
            pad.poll_at(0, || start).unwrap(),
            vec![
                key(19, true),
                key(26, true),
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                Input::DialPress {
                    index: 2,
                    pressed: true
                },
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
        // Left stick down and left.
        recorder.queue_read(state([0, 255, 128, 128, 0x0f, 0, 0, 0, 0], idle(), 0));
        assert_eq!(
            pad.poll_at(0, || start + Duration::from_millis(80))
                .unwrap(),
            vec![
                key(19, false),
                key(20, true),
                key(21, true),
                key(26, false),
                Input::DialPress {
                    index: 0,
                    pressed: false
                },
                Input::DialPress {
                    index: 2,
                    pressed: false
                },
                Input::DialTurn {
                    index: 0,
                    delta: -1
                },
                Input::DialTurn {
                    index: 1,
                    delta: -1
                },
            ]
        );
        assert_eq!(stick(255), 1.0);
        assert_eq!(stick(0), -1.0);
        assert_eq!(stick(128), 0.0);
    }

    #[test]
    fn short_answers_and_unchanged_states_raise_nothing() {
        let (mut pad, recorder) = driver();
        let mut short = state(block(0x20, 0, 0), idle(), 0);
        short.truncate(62);
        recorder.queue_read(short);
        recorder.queue_read(answer(&[0x10, 0x06, 0x00]));
        recorder.queue_read(answer(&[0x10, 0x0e]));
        recorder.queue_read(vec![0x12]);
        recorder.queue_read(vec![]);
        assert!(pad.poll(0).unwrap().is_empty());
        recorder.queue_read(state(block(0x20, 0, 0), idle(), 0));
        recorder.queue_read(state(block(0x20, 0, 0), idle(), 0));
        assert_eq!(pad.poll(0).unwrap(), vec![key(0, true)]);
        assert!(pad.set_face(0, None).is_ok());
        // No deck light set: nothing touches the light profile.
        assert!(pad.flush().is_ok() && pad.set_brightness(30).is_ok());
        assert_eq!(recorder.writes(), vec![host(&KEEP_ALIVE)]);
    }

    /// The answer to reading `len` bytes at `offset`, filled with `fill`.
    fn read_answer(offset: u16, len: u8, fill: u8) -> Vec<u8> {
        let [high, low] = offset.to_be_bytes();
        let mut frame = vec![0x10, 0x05, 0x20, high, low, len];
        frame.extend(vec![fill; usize::from(len)]);
        answer(&frame)
    }

    const WRITE_ACK: [u8; 3] = [0x10, 0x06, 0x00];

    #[test]
    fn a_deck_light_backs_up_the_playing_animation_then_writes_one_static_frame() {
        let (mut pad, recorder) = driver();
        recorder.queue_read(state(idle(), idle(), 2));
        pad.poll(0).unwrap();
        // Animation 2 starts at 1 + 124 * 2 = 249 (0x00F9).
        recorder.queue_read(read_answer(249, 58, 0xa1));
        // A press made while the light is written is kept for the next poll.
        recorder.queue_read(state(block(0x20, 0, 0), idle(), 2));
        recorder.queue_read(read_answer(307, 58, 0xa2));
        recorder.queue_read(answer(&[0x10, 0x0e, 0x01]));
        recorder.queue_read(read_answer(365, 8, 0xa3));
        recorder.queue_read(answer(&[0x10, 0x06, 0x01]));
        recorder.queue_read(answer(&WRITE_ACK));
        pad.set_deck_light(Some([0x11, 0x22, 0x33])).unwrap();
        pad.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[1..],
            [
                host(&[0x0f, 0x04, 0x20, 0x00, 0xf9, 58]),
                host(&[0x0f, 0x04, 0x20, 0x01, 0x33, 58]),
                host(&[0x0f, 0x04, 0x20, 0x01, 0x6d, 8]),
                host(&[
                    0x0f, 0x03, 0x20, 0x00, 0xf9, 0x13, 0x01, 0x05, 0x14, 100, 0x11, 0x22, 0x33,
                    0x11, 0x22, 0x33, 0x11, 0x22, 0x33, 0x11, 0x22, 0x33, 0x11, 0x22, 0x33,
                ]),
                host(&[0x0f, 0x0d, 0x00, 0x01]),
            ]
        );
        assert_eq!(pad.poll(0).unwrap(), vec![key(0, true)]);
        // The same light again writes nothing; brightness writes its one byte.
        pad.flush().unwrap();
        assert_eq!(recorder.writes().len(), 6);
        recorder.queue_read(answer(&WRITE_ACK));
        pad.set_brightness(40).unwrap();
        assert_eq!(
            recorder.writes()[6],
            host(&[0x0f, 0x03, 0x20, 0x00, 0xfc, 0x01, 40])
        );
        // Clearing the light writes the original animation back and plays it.
        for _ in 0..3 {
            recorder.queue_read(answer(&WRITE_ACK));
        }
        pad.set_deck_light(None).unwrap();
        pad.flush().unwrap();
        let writes = recorder.writes();
        let mut first = vec![0x0f, 0x03, 0x20, 0x00, 0xf9, 58];
        first.extend([0xa1; 58]);
        let mut second = vec![0x0f, 0x03, 0x20, 0x01, 0x33, 58];
        second.extend([0xa2; 58]);
        let mut third = vec![0x0f, 0x03, 0x20, 0x01, 0x6d, 8];
        third.extend([0xa3; 8]);
        assert_eq!(
            writes[7..],
            [
                host(&first),
                host(&second),
                host(&third),
                host(&[0x0f, 0x0d, 0x01, 0x01])
            ]
        );
        // Nothing is left to put back when Ryu lets go.
        drop(pad);
        assert_eq!(recorder.writes().len(), 11);
    }

    /// A read answer carrying `data`, which starts at the offset `at`.
    fn read_data(at: u16, data: &[u8]) -> Vec<u8> {
        let [high, low] = at.to_be_bytes();
        let mut frame = vec![0x10, 0x05, 0x20, high, low, data.len() as u8];
        frame.extend_from_slice(data);
        answer(&frame)
    }

    #[test]
    fn the_original_animation_survives_an_unclean_exit() {
        let dir = std::env::temp_dir().join(format!(
            "ryu-decks-gamesir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let open = || {
            let recorder = Recorder::default();
            let pad = GameSir::new(
                by_id("gamesir-cyclone-2").unwrap(),
                Box::new(recorder.clone()),
            )
            .with_backup_dir(dir.clone());
            (pad, recorder)
        };
        let original = vec![0xa1; ANIMATION_LEN];
        let mut ryu = vec![0x01, 0x05, 0x14, 100];
        for _ in 0..5 {
            ryu.extend([0x11, 0x22, 0x33]);
        }
        ryu.resize(ANIMATION_LEN, 0);
        assert!(is_ryu_frame(&ryu));
        assert!(!is_ryu_frame(&original));
        let reads = |recorder: &Recorder, data: &[u8]| {
            recorder.queue_read(read_data(1, &data[..58]));
            recorder.queue_read(read_data(59, &data[58..116]));
            recorder.queue_read(read_data(117, &data[116..]));
        };
        // The first run backs the original up on disk, writes its colour, and
        // is killed (no Drop) before the original goes back.
        let (mut first, recorder) = open();
        recorder.queue_read(state(idle(), idle(), 0));
        first.poll(0).unwrap();
        reads(&recorder, &original);
        recorder.queue_read(answer(&WRITE_ACK));
        first.set_deck_light(Some([0x11, 0x22, 0x33])).unwrap();
        first.flush().unwrap();
        std::mem::forget(first);
        // The next run finds Ryu's own frame in the slot: the controller reads
        // it back as the "original", but the saved one is kept instead.
        let (mut second, recorder) = open();
        recorder.queue_read(state(idle(), idle(), 0));
        second.poll(0).unwrap();
        reads(&recorder, &ryu);
        recorder.queue_read(answer(&WRITE_ACK));
        second.set_deck_light(Some([0x44, 0x55, 0x66])).unwrap();
        second.flush().unwrap();
        for _ in 0..3 {
            recorder.queue_read(answer(&WRITE_ACK));
        }
        second.set_deck_light(None).unwrap();
        second.flush().unwrap();
        let writes = recorder.writes();
        let mut first_chunk = vec![0x0f, 0x03, 0x20, 0x00, 0x01, 58];
        first_chunk.extend([0xa1; 58]);
        assert!(writes.contains(&host(&first_chunk)));
        assert!(!dir.join("light-slot-0.bin").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unanswered_backup_read_writes_nothing() {
        let (mut pad, recorder) = driver();
        recorder.queue_read(state(idle(), idle(), 0));
        pad.poll(0).unwrap();
        pad.set_deck_light(Some([1, 2, 3])).unwrap();
        pad.flush().unwrap();
        // Ten sends of the first read, then nothing else.
        assert_eq!(
            recorder.writes()[1..],
            vec![host(&[0x0f, 0x04, 0x20, 0x00, 0x01, 58]); 10]
        );
        drop(pad);
        assert_eq!(recorder.writes().len(), 11);
    }

    #[test]
    fn an_unanswered_light_write_rests_before_repaints_try_it_again() {
        let (mut pad, recorder) = driver();
        recorder.queue_read(state(idle(), idle(), 0));
        pad.poll(0).unwrap();
        let start = Instant::now();
        pad.set_deck_light(Some([1, 2, 3])).unwrap();
        pad.send_light(start).unwrap();
        let read = host(&[0x0f, 0x04, 0x20, 0x00, 0x01, 58]);
        assert_eq!(recorder.writes()[1..], vec![read.clone(); 10]);
        // Repaints, brightness and polls while it rests send nothing more.
        pad.send_light(start + Duration::from_secs(1)).unwrap();
        pad.set_brightness(50).unwrap();
        pad.poll_at(0, || start + Duration::from_millis(4900))
            .unwrap();
        let reads = |writes: Vec<Vec<u8>>| writes.iter().filter(|w| **w == read).count();
        assert_eq!(reads(recorder.writes()), 10);
        // After the rest a poll tries it again; a second failure rests twice as long.
        pad.poll_at(0, || start + Duration::from_secs(5)).unwrap();
        assert_eq!(reads(recorder.writes()), 20);
        pad.poll_at(0, || start + Duration::from_secs(14)).unwrap();
        assert_eq!(reads(recorder.writes()), 20);
        pad.poll_at(0, || start + Duration::from_secs(15)).unwrap();
        assert_eq!(reads(recorder.writes()), 30);
    }

    #[test]
    fn the_light_profile_is_asked_which_animation_plays_before_any_state() {
        let (mut pad, recorder) = driver();
        recorder.queue_read(read_answer(0, 1, 1));
        recorder.queue_read(read_answer(125, 58, 0));
        recorder.queue_read(read_answer(183, 58, 0));
        recorder.queue_read(read_answer(241, 8, 0));
        recorder.queue_read(answer(&WRITE_ACK));
        pad.set_deck_light(Some([9, 9, 9])).unwrap();
        pad.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes[1], host(&[0x0f, 0x04, 0x20, 0x00, 0x00, 1]));
        assert_eq!(writes[5][..6], [0x0f, 0x03, 0x20, 0x00, 0x7d, 0x13]);
    }

    #[test]
    fn switching_animations_moves_the_colour_and_letting_go_restores_it() {
        let (mut pad, recorder) = driver();
        recorder.queue_read(state(idle(), idle(), 0));
        pad.poll(0).unwrap();
        for (offset, len) in [(1, 58), (59, 58), (117, 8)] {
            recorder.queue_read(read_answer(offset, len, 0x55));
        }
        recorder.queue_read(answer(&WRITE_ACK));
        pad.set_deck_light(Some([0xff, 0, 0])).unwrap();
        pad.flush().unwrap();
        assert_eq!(recorder.writes().len(), 6);
        // M and the left stick switch to animation 3 (and say so).
        recorder.queue_read(answer(&[0x10, 0x0f, 0x20, 0x00, 0x00, 0x00, 0x01]));
        recorder.queue_read(state(idle(), idle(), 3));
        assert!(pad.poll(0).unwrap().is_empty());
        assert_eq!(recorder.writes().len(), 6);
        // The next poll moves the colour.
        for _ in 0..3 {
            recorder.queue_read(answer(&WRITE_ACK));
        }
        for (offset, len) in [(373, 58), (431, 58), (489, 8)] {
            recorder.queue_read(read_answer(offset, len, 0x77));
        }
        recorder.queue_read(answer(&WRITE_ACK));
        assert!(pad.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        // Animation 0 goes back, then animation 3 is backed up and coloured.
        assert_eq!(writes[6][..7], [0x0f, 0x03, 0x20, 0x00, 0x01, 58, 0x55]);
        assert_eq!(writes[9], host(&[0x0f, 0x0d, 0x01, 0x01]));
        assert_eq!(writes[10], host(&[0x0f, 0x04, 0x20, 0x01, 0x75, 58]));
        assert_eq!(
            writes[13][..10],
            [0x0f, 0x03, 0x20, 0x01, 0x75, 0x13, 0x01, 0x05, 0x14, 100]
        );
        assert_eq!(writes.len(), 15);
        // Letting go writes animation 3 back.
        for _ in 0..3 {
            recorder.queue_read(answer(&WRITE_ACK));
        }
        drop(pad);
        let writes = recorder.writes();
        assert_eq!(writes[15][..7], [0x0f, 0x03, 0x20, 0x01, 0x75, 58, 0x77]);
        assert_eq!(writes[18], host(&[0x0f, 0x0d, 0x01, 0x01]));
    }
}
