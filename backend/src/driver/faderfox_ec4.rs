//! Faderfox's EC4 encoder controller, over class-compliant USB MIDI
//! ([`super::midi`]).
//!
//! The EC4 lists one MIDI port each way, called `Faderfox EC4` (ALSA adds its
//! usual ` MIDI 1` and client numbers). [`port_matches`] takes a name
//! containing `Faderfox EC4` in any case, which no PC4, SC4 or UC4 carries,
//! and on Windows only the first interface.
//!
//! The deck plays whatever setup the user loaded, and it has no command that
//! writes its working memory without rewriting its EEPROM, so Ryu pushes no
//! mapping: it reads two known layouts.
//!
//! - Factory setups 1-14, any group: encoder `i` (0-15, row by row from the
//!   top left) is absolute Control Change `16 * ((group - 1) mod 8) + i` on
//!   the channel of the setup's number, and its push button a Note On (127)
//!   then, expected though not confirmed, a Note Off (velocity 127) on the
//!   same channel and number. Ryu reads them only on the channel of the setup
//!   the deck reported (before any report, channels 1-14), by number modulo
//!   16, so a group change needs nothing, while external gear forwarded from
//!   the deck's DIN MIDI In on another channel is not taken for the deck.
//!   Absolute values become steps from the last value seen. Once Ryu knows
//!   the setup and group it sends each encoder `64` (the deck takes feedback
//!   for its programmed commands) and counts from there; an encoder left near
//!   either end is put back to 64 once it has been still a quarter second.
//!   Setups 15 and 16 (Ableton's) use other channels and numbers and are
//!   ignored.
//! - DrivenByMoss's template, in whichever setup: channel 16, encoder `i` is
//!   relative (two's-complement) Control Change `10 + i` and its push button
//!   Control Change `30 + i`, 127 pressed and 0 released.
//!
//! The firmware-2 SysEx is the same in every setup: each message is
//! `F0 00 00 00 4E 2C 1B`, then three-byte commands `4E <function> <value>`,
//! then `F7`. Function `28` reports the setup (`1s`, s from 0) and `24` the
//! group; Ryu asks for both with `F0 00 00 00 4E 20 10 F7` at open, and the
//! deck sends them again whenever the user changes either. Function `26`
//! names an extra key (`11` Shift, `12`-`15` User 1-4, which are FUNC held
//! with encoder 1, 5, 9 or 13) and `2A` a push button pressed with Shift held
//! (`10 + n`); the `2E` after it says pressed (`11`) or released.
//!
//! Keys 0-15 are the encoder pushes, row by row; keys 16-19 User 1-4 and key
//! 20 Shift. Dials 0-15 are the encoders in the same order. Pushes are keys,
//! never dial presses, so each key's label can go to its encoder's name.
//!
//! Out: the 4 x 20 OLED's encoder-name area takes text by
//! `F0 00 00 00 4E 2C 1B 4E 22 10 4A <addr hi> <addr lo> 4D <char hi>
//! <char lo>... F7`, each byte as two nibbles (`0x20 | high`, `0x10 | low`),
//! the address advancing by itself. Encoder `k`'s four-character name sits at
//! address `4k`; it shows only where the setup's own stored name is `----`.
//! Ryu writes each key's label there (its first four characters, anything
//! outside the deck's ASCII-like set as a space, `----` with none) and writes
//! all 16 again after each setup or group report, since the deck forgets them.
//! Every write waits in a queue that poll and flush drain a millisecond apart,
//! so the shared poll loop never sleeps. When Ryu lets go it hides the full-
//! screen overlay (`4E 22 15`) and puts the factory dashes back in every name.
//! There are no lights Ryu can set, no brightness command and no keep-alive;
//! Ryu never sends a setup dump.
//!
//! Facts from DrivenByMoss (Java, LGPL-3.0, Jürgen Moßgraber) and its EC4
//! template file, Faderfox's EC4 manual (V03) and its Faderfox Universal 2
//! Ableton script (no licence; facts only), and the faderfox-editor project's
//! SysEx notes and factory dump (MIT, privatepublic-de); no code was copied.
//! Untested on hardware.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const FADERFOX_EC4_ID: &str = "faderfox-ec4";

/// The 16 encoder pushes, User 1-4 and Shift.
pub const KEYS: u8 = 21;
/// User 1-4 and Shift, after the encoder pushes.
pub const EXTRA_KEYS: u8 = 5;
pub const DIALS: u8 = 16;
/// The encoders four by four, User 1-4 on the fifth row, Shift below.
pub const ROWS: u8 = 6;
pub const COLS: u8 = 4;
const ENCODERS: u8 = 16;
const FIRST_USER_KEY: u8 = 16;
const SHIFT_KEY: u8 = 20;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// The highest channel (from 0) a factory setup uses: setup 14.
const LAST_FACTORY_CHANNEL: u8 = 13;
/// DrivenByMoss's template: channel 16, encoders CC 10-25, pushes CC 30-45.
const TEMPLATE_CHANNEL: u8 = 15;
const TEMPLATE_FIRST_ENCODER: u8 = 10;
const TEMPLATE_FIRST_PUSH: u8 = 30;

/// Every EC4 SysEx starts so: Faderfox's manufacturer bytes, then the EC4.
const HEADER: [u8; 7] = [0xf0, 0x00, 0x00, 0x00, 0x4e, 0x2c, 0x1b];
const SYSEX_END: u8 = 0xf7;
/// Asks for the current setup and group.
const REQUEST: [u8; 8] = [0xf0, 0x00, 0x00, 0x00, 0x4e, 0x20, 0x10, 0xf7];
const COMMAND: u8 = 0x4e;
const SETUP: u8 = 0x28;
const GROUP: u8 = 0x24;
const EXTENDED_KEY: u8 = 0x26;
const SHIFTED_PUSH: u8 = 0x2a;
const KEY_STATE: u8 = 0x2e;
const KEY_PRESSED: u8 = 0x11;
const DISPLAY: u8 = 0x22;
/// The display targets: the encoder names, the overlay off.
const NAMES_AREA: u8 = 0x10;
const HIDE_OVERLAY: u8 = 0x15;
const ADDRESS: u8 = 0x4a;
const CHARACTER: u8 = 0x4d;
const NAME_LEN: usize = 4;
const DASHES: [u8; NAME_LEN] = *b"----";

/// The gap between two writes: Faderfox warns against flooding the deck.
const PACE: Duration = Duration::from_millis(1);
/// How long an encoder near either end stays still before Ryu re-centres it.
const SETTLE: Duration = Duration::from_millis(250);
/// How long Ryu waits for the setup report before logging a hint.
const REPLY_WAIT: Duration = Duration::from_secs(3);
const CENTRE: u8 = 64;
const LOW_END: u8 = 8;
const HIGH_END: u8 = 119;
const READ_LEN: usize = 256;
const MAX_MESSAGES_PER_POLL: usize = 64;

const NAMES: [&str; KEYS as usize] = [
    "Push 1", "Push 2", "Push 3", "Push 4", "Push 5", "Push 6", "Push 7", "Push 8", "Push 9",
    "Push 10", "Push 11", "Push 12", "Push 13", "Push 14", "Push 15", "Push 16", "User 1",
    "User 2", "User 3", "User 4", "Shift",
];

/// Whether a normalised port name is an EC4's, on Windows its first
/// interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.protocol == Protocol::FaderfoxEc4Midi
        && name.interface.unwrap_or(1) == 1
        && name.base.to_ascii_uppercase().contains("FADERFOX EC4")
}

/// The legend on each key: the encoder pushes, User 1-4 and Shift.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::FaderfoxEc4Midi).then(|| NAMES.to_vec())
}

/// A character the EC4 shows as itself; anything else becomes a space (the
/// deck has umlauts or blanks at `$`, `@`, `[`-`` ` `` and `{`-`~`).
fn display_char(c: char) -> u8 {
    match c {
        ' ' | '!' | '"' | '#' | '%'..='?' | 'A'..='Z' | 'a'..='z' => {
            u8::try_from(c).unwrap_or(b' ')
        }
        _ => b' ',
    }
}

/// A key's four-character name: its label's first four characters, or the
/// factory dashes with none.
fn name_of(face: Option<&Face>) -> [u8; NAME_LEN] {
    let label = face.map_or("", |face| face.label.trim());
    if label.is_empty() {
        return DASHES;
    }
    let mut name = [b' '; NAME_LEN];
    for (slot, c) in name.iter_mut().zip(label.chars()) {
        *slot = display_char(c);
    }
    name
}

/// One three-byte command: a byte as two nibbles.
const fn command(code: u8, value: u8) -> [u8; 3] {
    [code, 0x20 | (value >> 4), 0x10 | (value & 0x0f)]
}

/// Text written to the encoder-name area from `address` on.
fn names_message(address: u8, text: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(HEADER.len() + 7 + 3 * text.len());
    message.extend_from_slice(&HEADER);
    message.extend_from_slice(&[COMMAND, DISPLAY, NAMES_AREA]);
    message.extend_from_slice(&command(ADDRESS, address));
    for byte in text {
        message.extend_from_slice(&command(CHARACTER, *byte));
    }
    message.push(SYSEX_END);
    message
}

/// Every name back to the factory dashes.
fn clear_names() -> Vec<u8> {
    names_message(0, &[b'-'; NAME_LEN * ENCODERS as usize])
}

/// The full-screen overlay hidden.
fn hide_overlay() -> Vec<u8> {
    let mut message = HEADER.to_vec();
    message.extend_from_slice(&[COMMAND, DISPLAY, HIDE_OVERLAY]);
    message.push(SYSEX_END);
    message
}

/// A relative two's-complement step: 1-63 clockwise, 65-127 anticlockwise.
fn relative(value: u8) -> Option<i8> {
    match value {
        1..=63 => i8::try_from(value).ok(),
        65..=127 => i8::try_from(i16::from(value) - 128).ok(),
        _ => None,
    }
}

/// An absolute encoder of a factory setup.
#[derive(Debug, Default, Clone, Copy)]
struct Encoder {
    /// The value it was last at, as far as Ryu knows.
    last: Option<u8>,
    /// The status and controller it last sent on, to put it back to 64.
    origin: Option<(u8, u8)>,
    /// When it last moved, while it waits near an end to be re-centred.
    near_end_since: Option<Instant>,
}

/// One queued write; a name replaces that name still waiting.
struct Outgoing {
    name_slot: Option<u8>,
    bytes: Vec<u8>,
}

pub struct FaderfoxEc4 {
    transport: Box<dyn Transport>,
    opened: Instant,
    pace: Duration,
    settle: Duration,
    /// The setup request is queued.
    requested: bool,
    /// The deck reported its setup (0-15) and group (0-15).
    setup: Option<u8>,
    group: Option<u8>,
    /// The missing-report hint is logged.
    warned: bool,
    /// Writes waiting their turn, one each time `pace` has passed.
    outbox: VecDeque<Outgoing>,
    next_send: Instant,
    /// Each encoder's name as staged, then as last queued.
    names: [[u8; NAME_LEN]; ENCODERS as usize],
    sent_names: [Option<[u8; NAME_LEN]>; ENCODERS as usize],
    encoders: [Encoder; ENCODERS as usize],
    held: [bool; KEYS as usize],
    /// Push buttons whose Note On came twice with no release between: the
    /// setup sends none (lower value 0), so Ryu releases them itself.
    press_only: [bool; ENCODERS as usize],
    /// Push buttons pressed by a Note On not yet released by a Note Off.
    note_held: [bool; ENCODERS as usize],
    /// The key a `26` or `2A` named, waiting for its `2E` state.
    pending_key: Option<u8>,
    parser: Parser,
}

impl FaderfoxEc4 {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            opened: Instant::now(),
            pace: PACE,
            settle: SETTLE,
            requested: false,
            setup: None,
            group: None,
            warned: false,
            outbox: VecDeque::new(),
            next_send: Instant::now(),
            names: [DASHES; ENCODERS as usize],
            sent_names: [None; ENCODERS as usize],
            encoders: [Encoder::default(); ENCODERS as usize],
            held: [false; KEYS as usize],
            press_only: [false; ENCODERS as usize],
            note_held: [false; ENCODERS as usize],
            pending_key: None,
            parser: Parser::default(),
        }
    }

    fn send(&mut self, name_slot: Option<u8>, bytes: Vec<u8>) {
        if name_slot.is_some() {
            self.outbox.retain(|queued| queued.name_slot != name_slot);
        }
        self.outbox.push_back(Outgoing { name_slot, bytes });
    }

    /// Writes the queued messages whose turn has come, without sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        while let Some(message) = self.outbox.front() {
            let now = Instant::now();
            if now < self.next_send {
                break;
            }
            self.transport.write(&message.bytes)?;
            self.outbox.pop_front();
            self.next_send = now + self.pace;
        }
        Ok(())
    }

    /// The factory setup's channel (from 0) Ryu reads, if the message's is
    /// one: the reported setup's, or before a report any of setups 1-14.
    fn factory_channel(&self, channel: u8) -> bool {
        match self.setup {
            Some(setup) => setup <= LAST_FACTORY_CHANNEL && channel == setup,
            None => channel <= LAST_FACTORY_CHANNEL,
        }
    }

    /// Queues every name that changed since it was last queued, once the
    /// deck has shown it takes the firmware-2 SysEx.
    fn queue_names(&mut self) {
        if self.setup.is_none() {
            return;
        }
        for slot in 0..ENCODERS {
            let index = usize::from(slot);
            let name = self.names[index];
            if self.sent_names[index] != Some(name) {
                self.send(Some(slot), names_message(slot * 4, &name));
                self.sent_names[index] = Some(name);
            }
        }
    }

    /// After a setup or group report: every encoder of a factory setup back
    /// to 64, and every name again.
    fn reported(&mut self) {
        if let (Some(setup), Some(group)) = (self.setup, self.group) {
            if setup <= LAST_FACTORY_CHANNEL {
                for (encoder, state) in (0u8..).zip(self.encoders.iter_mut()) {
                    let origin = (CONTROL_CHANGE | setup, 16 * (group & 7) + encoder);
                    *state = Encoder {
                        last: Some(CENTRE),
                        origin: Some(origin),
                        near_end_since: None,
                    };
                }
                for state in self.encoders {
                    if let Some((status, number)) = state.origin {
                        self.send(None, vec![status, number, CENTRE]);
                    }
                }
            }
        }
        self.sent_names = [None; ENCODERS as usize];
        self.queue_names();
    }

    /// Queues `64` for every encoder that has waited near an end long enough.
    fn recentre(&mut self) {
        let settle = self.settle;
        let mut due = Vec::new();
        for state in &mut self.encoders {
            let Some(since) = state.near_end_since else {
                continue;
            };
            if since.elapsed() < settle {
                continue;
            }
            state.near_end_since = None;
            if let Some((status, number)) = state.origin {
                state.last = Some(CENTRE);
                due.push(vec![status, number, CENTRE]);
            }
        }
        for message in due {
            self.send(None, message);
        }
    }

    /// Opens the session: the setup request, once.
    fn start(&mut self) {
        if !self.requested {
            self.requested = true;
            self.send(None, REQUEST.to_vec());
        }
    }

    fn read(&mut self, timeout_ms: i32) -> DriverResult<Vec<Message>> {
        let mut messages = Vec::new();
        let mut buf = [0u8; READ_LEN];
        for attempt in 0..MAX_MESSAGES_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.parser
                .feed(buf.get(..read).unwrap_or_default(), &mut messages);
        }
        Ok(messages)
    }

    fn press(&mut self, key: u8, pressed: bool, events: &mut Vec<Input>) {
        let Some(held) = self.held.get_mut(usize::from(key)) else {
            return;
        };
        if *held != pressed {
            *held = pressed;
            events.push(Input::Key {
                index: key,
                pressed,
            });
        }
    }

    /// A factory push button's Note On or Off. A press already held through
    /// the Shift SysEx is not pressed again.
    fn push_note(&mut self, key: u8, on: bool, events: &mut Vec<Input>) {
        let index = usize::from(key);
        let (Some(note_held), Some(press_only)) = (
            self.note_held.get(index).copied(),
            self.press_only.get(index).copied(),
        ) else {
            return;
        };
        if !on {
            self.note_held[index] = false;
            self.press_only[index] = false;
            self.press(key, false, events);
            return;
        }
        // A second Note On with no release between: this button sends none.
        let press_only = press_only || note_held;
        self.press_only[index] = press_only;
        if press_only {
            self.note_held[index] = false;
            self.press(key, false, events);
            self.press(key, true, events);
            self.press(key, false, events);
        } else {
            self.note_held[index] = true;
            self.press(key, true, events);
        }
    }

    fn turn_absolute(&mut self, status: u8, number: u8, value: u8, events: &mut Vec<Input>) {
        let dial = number % ENCODERS;
        let Some(state) = self.encoders.get_mut(usize::from(dial)) else {
            return;
        };
        state.origin = Some((status, number));
        let old = state.last.replace(value);
        state.near_end_since = (value <= LOW_END || value >= HIGH_END).then(Instant::now);
        let Some(old) = old else {
            return;
        };
        let delta = (i16::from(value) - i16::from(old)).clamp(-127, 127);
        if let Ok(delta) = i8::try_from(delta) {
            if delta != 0 {
                events.push(Input::DialTurn { index: dial, delta });
            }
        }
    }

    fn receive_sysex(&mut self, bytes: &[u8], events: &mut Vec<Input>) {
        let Some(body) = bytes
            .strip_prefix(&HEADER[..])
            .and_then(|rest| rest.strip_suffix(&[SYSEX_END]))
        else {
            return;
        };
        let mut report = false;
        for triplet in body.chunks_exact(3) {
            let [code, function, value] = [triplet[0], triplet[1], triplet[2]];
            if code != COMMAND {
                break;
            }
            let low = value & 0x0f;
            match function {
                SETUP => {
                    self.setup = Some(low);
                    report = true;
                }
                GROUP => {
                    self.group = Some(low);
                    report = true;
                }
                EXTENDED_KEY => {
                    self.pending_key = match value {
                        0x11 => Some(SHIFT_KEY),
                        0x12..=0x15 => Some(FIRST_USER_KEY + value - 0x12),
                        _ => None,
                    };
                }
                SHIFTED_PUSH => {
                    self.pending_key = (value & 0xf0 == 0x10).then_some(low);
                }
                KEY_STATE => {
                    if let Some(key) = self.pending_key.take() {
                        self.press(key, value == KEY_PRESSED, events);
                    }
                }
                _ => {}
            }
        }
        if report {
            self.reported();
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let [status, number, value] = match message {
            Message::SysEx(bytes) => return self.receive_sysex(bytes, events),
            Message::Channel(bytes) => *bytes,
        };
        let (kind, channel) = (status & 0xf0, status & 0x0f);
        if channel == TEMPLATE_CHANNEL {
            if kind != CONTROL_CHANGE {
                return;
            }
            match number.checked_sub(TEMPLATE_FIRST_ENCODER) {
                Some(dial) if dial < ENCODERS => {
                    if let Some(delta) = relative(value) {
                        events.push(Input::DialTurn { index: dial, delta });
                    }
                    return;
                }
                _ => {}
            }
            if let Some(key) = number
                .checked_sub(TEMPLATE_FIRST_PUSH)
                .filter(|key| *key < ENCODERS)
            {
                self.press(key, value > 0, events);
            }
            return;
        }
        if !self.factory_channel(channel) {
            return;
        }
        match kind {
            NOTE_ON => self.push_note(number % ENCODERS, value > 0, events),
            NOTE_OFF => self.push_note(number % ENCODERS, false, events),
            CONTROL_CHANGE => self.turn_absolute(status, number, value, events),
            _ => {}
        }
    }
}

impl Driver for FaderfoxEc4 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.start();
        self.pump()?;
        let mut events = Vec::new();
        for message in &self.read(timeout_ms)? {
            self.receive(message, &mut events);
        }
        if self.setup.is_none() && !self.warned && self.opened.elapsed() >= REPLY_WAIT {
            self.warned = true;
            tracing::warn!(
                "the Faderfox EC4 did not report its setup; it needs firmware 2.0 for \
                 encoder names, Shift and the User keys"
            );
        }
        self.recentre();
        self.pump()?;
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(name) = self.names.get_mut(usize::from(key)) {
            *name = name_of(face);
        }
        Ok(())
    }

    /// Queues each name that changed, then writes what is due.
    fn flush(&mut self) -> DriverResult<()> {
        self.start();
        self.queue_names();
        self.recentre();
        self.pump()
    }

    /// The EC4 sets its brightness only in its own setup mode.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for FaderfoxEc4 {
    /// Hides the overlay and puts the factory dashes back in every name, if
    /// the deck ever took Ryu's SysEx.
    fn drop(&mut self) {
        if self.setup.is_none() {
            return;
        }
        self.outbox.clear();
        let _ = self.transport.write(&hide_overlay());
        // At most the deck's own gap, whatever a test set.
        let gap = self.pace.min(PACE);
        if !gap.is_zero() {
            std::thread::sleep(gap);
        }
        let _ = self.transport.write(&clear_names());
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    fn open() -> (FaderfoxEc4, Recorder) {
        let recorder = Recorder::default();
        let mut deck =
            FaderfoxEc4::new(by_id(FADERFOX_EC4_ID).unwrap(), Box::new(recorder.clone()));
        deck.pace = Duration::ZERO;
        deck.settle = Duration::ZERO;
        (deck, recorder)
    }

    fn labelled(label: &str) -> Face {
        Face {
            label: label.to_owned(),
            color: [0, 0, 0],
            image: None,
        }
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    fn turn(index: u8, delta: i8) -> Input {
        Input::DialTurn { index, delta }
    }

    fn sysex(commands: &[[u8; 3]]) -> Vec<u8> {
        let mut message = HEADER.to_vec();
        for triplet in commands {
            message.extend_from_slice(triplet);
        }
        message.push(0xf7);
        message
    }

    fn report(setup: u8, group: u8) -> Vec<u8> {
        sysex(&[[0x4e, 0x28, 0x10 | setup], [0x4e, 0x24, 0x10 | group]])
    }

    fn name(slot: u8, text: &[u8; 4]) -> Vec<u8> {
        let mut message = vec![
            0xf0, 0x00, 0x00, 0x00, 0x4e, 0x2c, 0x1b, 0x4e, 0x22, 0x10, 0x4a,
        ];
        let address = slot * 4;
        message.extend([0x20 | (address >> 4), 0x10 | (address & 0x0f)]);
        for byte in text {
            message.extend([0x4d, 0x20 | (byte >> 4), 0x10 | (byte & 0x0f)]);
        }
        message.push(0xf7);
        message
    }

    /// A deck that answered setup `setup` group `group`, its writes cleared.
    fn ready(setup: u8, group: u8) -> (FaderfoxEc4, Recorder) {
        let (mut deck, recorder) = open();
        recorder.queue_read(report(setup, group));
        deck.poll(0).unwrap();
        recorder.0.lock().unwrap().writes.clear();
        (deck, recorder)
    }

    #[test]
    fn the_model_is_found_by_its_port_name_only() {
        for raw in [
            "Faderfox EC4",
            "Faderfox EC4:Faderfox EC4 MIDI 1 24:0",
            "MIDIIN2 (Faderfox EC4)",
            "FADERFOX EC4",
        ] {
            let name = normalise(raw);
            let model = identify_midi(&name, raw.starts_with("MIDIIN"));
            if raw.starts_with("MIDIIN2") {
                assert!(model.is_none(), "{raw}");
                continue;
            }
            assert_eq!(model.map(|model| model.id), Some(FADERFOX_EC4_ID), "{raw}");
        }
        for raw in ["Faderfox PC4", "Faderfox UC4", "Faderfox SC4", "EC4"] {
            assert!(
                identify_midi(&normalise(raw), false)
                    .is_none_or(|model| model.id != FADERFOX_EC4_ID),
                "{raw}"
            );
        }
        let model = by_id(FADERFOX_EC4_ID).unwrap();
        assert!(model.protocol.is_midi() && !model.protocol.has_brightness());
        assert_eq!(
            (
                model.rows,
                model.cols,
                model.keys,
                model.extra_keys,
                model.dials
            ),
            (6, 4, 21, 5, 16)
        );
        assert_eq!(model.dial_lights, DialLights::None);
        assert!(!model.key_lights && model.deck_lights == 0);
        let view = ModelView::from(model);
        assert_eq!(view.vendor_id, "0000");
        assert!(view.usb_ids.is_empty());
        assert_eq!(view.key_names.as_ref().map(Vec::len), Some(21));
    }

    #[test]
    fn opening_asks_for_the_setup_and_writes_nothing_else_until_answered() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&labelled("Play"))).unwrap();
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(
            recorder.writes(),
            [vec![0xf0, 0x00, 0x00, 0x00, 0x4e, 0x20, 0x10, 0xf7]]
        );
        // Without a report the factory layout still reads, but nothing is
        // seeded and no name is written; Drop sends nothing either.
        recorder.queue_read(vec![0xb0, 0x05, 0x40]);
        recorder.queue_read(vec![0xb0, 0x05, 0x43]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(5, 3)]);
        drop(deck);
        assert_eq!(recorder.writes().len(), 1);
    }

    #[test]
    fn a_report_seeds_the_encoders_and_writes_every_name() {
        let (mut deck, recorder) = open();
        deck.set_face(0, Some(&labelled("Play"))).unwrap();
        deck.set_face(15, Some(&labelled("Ab$cd"))).unwrap();
        deck.poll(0).unwrap();
        recorder.0.lock().unwrap().writes.clear();
        // Setup 3 (s = 2), group 10 (g = 9): channel 3, controllers 16-31.
        recorder.queue_read(report(2, 9));
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 16 + 16);
        for (encoder, write) in (0u8..).zip(&writes[..16]) {
            assert_eq!(*write, vec![0xb2, 16 + encoder, 0x40]);
        }
        assert_eq!(
            writes[16],
            [
                0xf0, 0x00, 0x00, 0x00, 0x4e, 0x2c, 0x1b, 0x4e, 0x22, 0x10, 0x4a, 0x20, 0x10, 0x4d,
                0x25, 0x10, 0x4d, 0x26, 0x1c, 0x4d, 0x26, 0x11, 0x4d, 0x27, 0x19, 0xf7
            ]
        );
        assert_eq!(writes[16].len(), 26);
        assert_eq!(writes[17], name(1, b"----"));
        // `$` is not ASCII on the deck: a space instead.
        assert_eq!(writes[31], name(15, b"Ab c"));
        // Counting starts from the seeded 64.
        recorder.queue_read(vec![0xb2, 0x13, 0x42]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(3, 2)]);
        // Only changed names go out afterwards.
        recorder.0.lock().unwrap().writes.clear();
        deck.set_face(0, Some(&labelled("Play"))).unwrap();
        deck.set_face(4, Some(&labelled("Rec"))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes(), [name(4, b"Rec ")]);
        // A group change alone re-seeds and re-sends everything.
        recorder.0.lock().unwrap().writes.clear();
        recorder.queue_read(sysex(&[[0x4e, 0x24, 0x10]]));
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 32);
        assert_eq!(writes[0], vec![0xb2, 0x00, 0x40]);
        assert_eq!(writes[20], name(4, b"Rec "));
    }

    #[test]
    fn factory_input_is_read_on_the_reported_setups_channel_only() {
        let (mut deck, recorder) = ready(0, 0);
        for message in [
            // Push 6: Note On, then the factory Note Off at velocity 127.
            vec![0x90, 0x05, 0x7f],
            vec![0x80, 0x05, 0x7f],
            // A Note On at velocity 0 releases too.
            vec![0x90, 0x16, 0x7f],
            vec![0x90, 0x16, 0x00],
            // Another channel (forwarded DIN traffic) is ignored.
            vec![0x91, 0x05, 0x7f],
            vec![0xb3, 0x05, 0x50],
            // Group 2's numbers on channel 1: encoder 3, from 64 down 4.
            vec![0xb0, 0x12, 0x3c],
            // Program change and pitch bend are ignored.
            vec![0xc0, 0x05],
            vec![0xe0, 0x00, 0x40],
        ] {
            recorder.queue_read(message);
        }
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(5, true),
                key(5, false),
                key(6, true),
                key(6, false),
                turn(2, -4)
            ]
        );
    }

    #[test]
    fn before_a_report_any_factory_channel_counts_and_setups_15_and_16_never() {
        let (mut deck, recorder) = open();
        recorder.queue_read(vec![0x9d, 0x00, 0x7f]);
        recorder.queue_read(vec![0x9e, 0x01, 0x7f]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(0, true)]);
        // Setup 15 reported: its channel-13 CCs and CC pushes are not read.
        recorder.queue_read(report(14, 0));
        recorder.queue_read(vec![0xbc, 0x28, 0x6c]);
        recorder.queue_read(vec![0x9d, 0x00, 0x00]);
        assert!(deck.poll(0).unwrap().is_empty());
        // Nothing is seeded for setup 15, but the names are written.
        let writes = recorder.writes();
        assert!(writes.iter().all(|write| write[0] == 0xf0));
        assert_eq!(writes.len(), 1 + 16);
    }

    #[test]
    fn the_drivenbymoss_template_reads_relative_encoders_and_cc_pushes() {
        let (mut deck, recorder) = ready(0, 0);
        for message in [
            vec![0xbf, 0x0a, 0x01],
            vec![0xbf, 0x19, 0x7d],
            vec![0xbf, 0x0b, 0x40],
            vec![0xbf, 0x1e, 0x7f],
            vec![0xbf, 0x1e, 0x00],
            vec![0xbf, 0x2d, 0x7f],
            vec![0xbf, 0x2e, 0x7f],
            vec![0x9f, 0x00, 0x7f],
        ] {
            recorder.queue_read(message);
        }
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, 1),
                turn(15, -3),
                key(0, true),
                key(0, false),
                key(15, true)
            ]
        );
        // No re-centring for relative encoders.
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn shift_user_and_shifted_pushes_come_from_sysex() {
        let (mut deck, recorder) = ready(0, 0);
        for message in [
            sysex(&[[0x4e, 0x26, 0x11], [0x4e, 0x2e, 0x11]]),
            sysex(&[[0x4e, 0x2a, 0x14], [0x4e, 0x2e, 0x11]]),
            sysex(&[[0x4e, 0x2a, 0x14], [0x4e, 0x2e, 0x10]]),
            sysex(&[[0x4e, 0x26, 0x11], [0x4e, 0x2e, 0x10]]),
            sysex(&[[0x4e, 0x26, 0x12], [0x4e, 0x2e, 0x11]]),
            sysex(&[[0x4e, 0x26, 0x15], [0x4e, 0x2e, 0x11]]),
            sysex(&[[0x4e, 0x26, 0x14], [0x4e, 0x2e, 0x10]]),
            sysex(&[[0x4e, 0x26, 0x15], [0x4e, 0x2e, 0x10]]),
            // An unknown function, a stray state and a short body.
            sysex(&[[0x4e, 0x30, 0x11], [0x4e, 0x2e, 0x11]]),
            vec![0xf0, 0x00, 0x00, 0x00, 0x4e, 0x2c, 0x1b, 0x4e, 0x26, 0xf7],
            // Another Faderfox device's header.
            vec![
                0xf0, 0x00, 0x00, 0x00, 0x4e, 0x2c, 0x1a, 0x4e, 0x26, 0x11, 0x4e, 0x2e, 0x11, 0xf7,
            ],
        ] {
            recorder.queue_read(message);
        }
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(20, true),
                key(4, true),
                key(4, false),
                key(20, false),
                key(16, true),
                key(19, true),
                key(19, false),
            ]
        );
    }

    #[test]
    fn a_shifted_push_never_presses_a_held_key_twice() {
        let (mut deck, recorder) = ready(0, 0);
        recorder.queue_read(vec![0x90, 0x02, 0x7f]);
        recorder.queue_read(sysex(&[[0x4e, 0x2a, 0x12], [0x4e, 0x2e, 0x11]]));
        recorder.queue_read(vec![0x80, 0x02, 0x7f]);
        recorder.queue_read(sysex(&[[0x4e, 0x2a, 0x12], [0x4e, 0x2e, 0x10]]));
        assert_eq!(deck.poll(0).unwrap(), vec![key(2, true), key(2, false)]);
    }

    #[test]
    fn a_push_that_sends_no_release_is_released_by_ryu() {
        let (mut deck, recorder) = ready(0, 0);
        recorder.queue_read(vec![0x90, 0x07, 0x7f]);
        recorder.queue_read(vec![0x90, 0x07, 0x7f]);
        recorder.queue_read(vec![0x90, 0x07, 0x7f]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(7, true),
                key(7, false),
                key(7, true),
                key(7, false),
                key(7, true),
                key(7, false)
            ]
        );
        // A real release turns that off again.
        recorder.queue_read(vec![0x90, 0x07, 0x7f]);
        recorder.queue_read(vec![0x80, 0x07, 0x7f]);
        recorder.queue_read(vec![0x90, 0x07, 0x7f]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![key(7, true), key(7, false), key(7, true)]
        );
    }

    #[test]
    fn an_encoder_left_near_an_end_is_put_back_to_the_centre_once_still() {
        let (mut deck, recorder) = ready(1, 2);
        // Setup 2, group 3: channel 2, controllers 32-47.
        recorder.queue_read(vec![0xb1, 0x21, 0x7a]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(1, 58)]);
        assert_eq!(recorder.writes(), [vec![0xb1, 0x21, 0x40]]);
        // From the centre again.
        recorder.queue_read(vec![0xb1, 0x21, 0x3f]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(1, -1)]);
        assert_eq!(recorder.writes().len(), 1);
        // Not before it has been still long enough.
        deck.settle = Duration::from_secs(60);
        recorder.queue_read(vec![0xb1, 0x21, 0x02]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(1, -61)]);
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 1);
    }

    #[test]
    fn writes_are_paced_by_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open();
        deck.pace = Duration::from_secs(60);
        let begun = Instant::now();
        recorder.queue_read(report(0, 0));
        deck.poll(0).unwrap();
        deck.set_face(3, Some(&labelled("A"))).unwrap();
        deck.flush().unwrap();
        deck.set_face(3, Some(&labelled("B"))).unwrap();
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert_eq!(recorder.writes().len(), 1);
        // A newer name replaces the one still waiting for its slot.
        let names: Vec<_> = deck
            .outbox
            .iter()
            .filter(|queued| queued.name_slot == Some(3))
            .map(|queued| queued.bytes.clone())
            .collect();
        assert_eq!(names, [name(3, b"B   ")]);
        deck.pace = Duration::ZERO;
        deck.next_send = Instant::now();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 1 + 16 + 16);
    }

    #[test]
    fn letting_go_hides_the_overlay_and_restores_the_dashes() {
        let (mut deck, recorder) = ready(0, 0);
        deck.set_face(0, Some(&labelled("Play"))).unwrap();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(
            writes,
            [
                vec![0xf0, 0x00, 0x00, 0x00, 0x4e, 0x2c, 0x1b, 0x4e, 0x22, 0x15, 0xf7],
                clear_names()
            ]
        );
        let clear = &writes[1];
        assert_eq!(clear.len(), 206);
        assert_eq!(
            clear[..13],
            [0xf0, 0x00, 0x00, 0x00, 0x4e, 0x2c, 0x1b, 0x4e, 0x22, 0x10, 0x4a, 0x20, 0x10]
        );
        assert!(clear[13..205]
            .chunks_exact(3)
            .all(|triplet| triplet == [0x4d, 0x22, 0x1d]));
        // Never a setup dump.
        assert!(writes.iter().all(|write| !write.contains(&0x49)));
    }

    #[test]
    fn malformed_input_is_ignored() {
        let (mut deck, recorder) = open();
        for message in [
            vec![0x90],
            vec![0xf0, 0xf7],
            vec![0xf0, 0x00, 0x00, 0x00, 0x4e, 0x2c, 0x1b, 0xf7],
            vec![
                0xf0, 0x00, 0x00, 0x00, 0x4e, 0x2c, 0x1b, 0x4e, 0x2e, 0x11, 0xf7,
            ],
        ] {
            recorder.queue_read(message);
        }
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(deck.setup.is_none());
    }
}
