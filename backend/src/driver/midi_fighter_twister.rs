//! DJ TechTools' Midi Fighter Twister, over class-compliant USB MIDI
//! ([`super::midi`]), on its factory configuration.
//!
//! The Twister lists one MIDI port each way, named after its USB product
//! string: `Midi Fighter Twister` (`Midi Fighter Twister 2`, `3` or `4` on the
//! alternate-id firmware images). That is the bare name on macOS and Windows,
//! so the shared `<token> MI` rule cannot take it; [`port_matches`] takes any
//! name containing `Midi Fighter Twister`, on Windows its first interface.
//!
//! Every message either way is a 3-byte Control Change. Each of the 16 knobs
//! (dials 0-15, row by row from the top left) is numbered `N = bank * 16 +
//! dial` in every bank (4 banks, or 8 on the 2026 eight-bank firmware), so Ryu
//! folds the banks: dial `N % 16`, bank `N / 16`.
//!
//! In:
//!
//! - a turn is `B0 N v` (channel 1), an absolute value 0-127, one unit per
//!   detent. Ryu turns the dial by the difference from the last value it
//!   knows. The firmware stops at 0 and 127, so once a knob has been still for
//!   [`IDLE`] Ryu puts it back to 64 (`B0 N 40`); it ignores that while the
//!   knob is moving, so the first value after a re-centre is read against
//!   both 64 and the value before it, and the smaller step wins;
//! - a push is `B1 N 7F` / `B1 N 00` (channel 2);
//! - the side buttons send on the system channel (channel 4, `B3`). Left top,
//!   left bottom, right top and right bottom (`s` 0, 2, 3, 5) send `B3 (8 + s +
//!   6 * bank) 7F/00`; they are keys 0-3 (left top, right top, left bottom,
//!   right bottom). Left middle and right middle switch banks in the firmware
//!   and send only `B3 <old bank> 00` then `B3 <new bank> 7F`.
//!
//! Out, addressed by `N` in the bank on show:
//!
//! - the RGB light under a knob takes a palette index, `B1 N h` (channel 2):
//!   1-125 is a hue wheel (1 blue, about 22 cyan, 43-50 green, 64 yellow,
//!   84-85 red, 107 magenta, 114 violet). 0 and 126-127 hand the light back to
//!   the user's own inactive and active colours. [`palette_index`] picks the
//!   nearest hue; white and grey take 127, the knob's own active colour;
//! - its brightness is the RGB dim animation `B2 N a` (channel 3), 17-48,
//!   where 17 is fully off. A dark or unbound dial is set to 17;
//! - the white ring's brightness is the ring dim animation `B5 N a` (channel
//!   6), 65-96. 65 is very dim but not off. Its position stays at the
//!   re-centre value, 64;
//! - `B2 N 00` and `B5 N 00` stop the animations, `B1 N 00` clears the
//!   colour.
//!
//! At open Ryu sends `F0 00 01 79 06 00 00 F7` (leave the newest firmware's
//! native mode; older firmware ignores it), forces bank 1 with `B3 00 7F` (not
//! echoed), and centres every knob of that bank. The deck keeps separate
//! lights for each bank, so when the bank changes Ryu centres the new bank's
//! knobs and sends their lights again. When Ryu lets go, every bank it lit
//! gets its animations stopped and its colours cleared.
//!
//! Facts from DJ TechTools' Midi_Fighter_Twister_Open_Source firmware (DJTT
//! Embedded Software License, which forbids redistribution; read for facts
//! only), cross-checked against Mixxx's Midi Fighter Twister mapping
//! (GPL-2.0-or-later); no code or tables were copied.

use std::time::{Duration, Instant};

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const TWISTER_ID: &str = "djtt-midi-fighter-twister";

/// The four bindable side buttons, on a 2 x 2 grid.
pub const KEYS: u8 = 4;
pub const ROWS: u8 = 2;
pub const COLS: u8 = 2;
pub const DIALS: u8 = 16;

/// Status bytes: Control Change on channels 1, 2, 3, 4 and 6.
const TURN: u8 = 0xb0;
const PUSH: u8 = 0xb1;
const RGB_ANIMATION: u8 = 0xb2;
const SYSTEM: u8 = 0xb3;
const RING_ANIMATION: u8 = 0xb5;

/// The value a knob is put back to.
const CENTRE: u8 = 64;
/// The first side-button number; below it, the system channel names banks.
const FIRST_SIDE: u8 = 8;
/// Side-button numbers move by this much from one bank to the next.
const SIDES_PER_BANK: u8 = 6;
/// Banks of the eight-bank firmware; the four-bank one uses the first four.
const MAX_BANKS: u8 = 8;
const PRESSED: u8 = 0x7f;

/// RGB dim levels: 17 is off, 48 full.
const RGB_OFF: u8 = 17;
/// Ring dim levels: 65 is the dimmest, 96 full.
const RING_DIM: u8 = 65;
/// Dim steps above the floor (both animations have 32).
const DIM_STEPS: u16 = 31;
/// The colour sent with a dark dial, so the knob's own colour does not show.
const DARK_INDEX: u8 = 1;
/// The knob's own active colour: white is not on the palette.
const ACTIVE_INDEX: u8 = 127;

/// How long a knob must be still before Ryu puts it back to the centre: the
/// firmware ignores a position for about 128 ms after the knob moves.
const IDLE: Duration = Duration::from_millis(200);
/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const NAMES: [&str; KEYS as usize] = ["Left top", "Right top", "Left bottom", "Right bottom"];

/// Whether a normalised port name is a Twister's, on Windows its first
/// interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    model.protocol == Protocol::MidiFighterTwisterMidi
        && name.interface.unwrap_or(1) == 1
        && name.base.contains("Midi Fighter Twister")
}

/// The legend on each side button.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::MidiFighterTwisterMidi).then(|| NAMES.to_vec())
}

/// Hue (degrees, unwrapped so the wheel runs one way) at a few palette
/// indices of the factory colour wheel, in index order.
const HUES: [(u8, i32); 8] = [
    (1, 240),
    (22, 180),
    (46, 120),
    (64, 60),
    (84, 0),
    (100, -41),
    (107, -60),
    (114, -94),
];

/// The factory palette index nearest a colour; `None` when it is dark.
#[must_use]
pub fn palette_index(color: [u8; 3]) -> Option<u8> {
    let [red, green, blue] = color.map(i32::from);
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    if max < i32::from(DARK) {
        return None;
    }
    let chroma = max - min;
    // Under a quarter saturated: white or grey.
    if chroma * 4 < max {
        return Some(ACTIVE_INDEX);
    }
    let hue = if max == red {
        (60 * (green - blue) / chroma).rem_euclid(360)
    } else if max == green {
        120 + 60 * (blue - red) / chroma
    } else {
        240 + 60 * (red - green) / chroma
    };
    // The wheel runs from blue (240) down through 0 to violet (266): hues
    // between blue and violet go to the nearer end.
    let unwrapped = match hue {
        ..=240 => hue,
        241..=252 => 240,
        _ => hue - 360,
    };
    for pair in HUES.windows(2) {
        let [(from_index, from_hue), (to_index, to_hue)] = [pair[0], pair[1]];
        if unwrapped <= from_hue && unwrapped >= to_hue {
            let span = from_hue - to_hue;
            let steps = i32::from(to_index) - i32::from(from_index);
            let index = i32::from(from_index) + (from_hue - unwrapped) * steps / span;
            return u8::try_from(index).ok();
        }
    }
    Some(HUES[HUES.len() - 1].0)
}

/// The three bytes one dial's light is: colour index, RGB level and ring level.
fn light(color: Option<[u8; 3]>, percent: u8) -> [u8; 3] {
    let step = |floor: u8| {
        let above = u16::from(percent.min(100)) * DIM_STEPS / 100;
        floor + u8::try_from(above).unwrap_or(0)
    };
    match color.and_then(palette_index) {
        Some(index) if percent > 0 => [index, step(RGB_OFF), step(RING_DIM)],
        _ => [DARK_INDEX, RGB_OFF, RING_DIM],
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Knob {
    /// The value the deck last sent or was set to.
    last: u8,
    /// When it last turned; `None` once it is back at the centre.
    moved: Option<Instant>,
    /// The value before the last re-centre, while no turn has come since.
    before_centre: Option<u8>,
}

pub struct Twister {
    transport: Box<dyn Transport>,
    idle: Duration,
    started: bool,
    bank: u8,
    /// Banks whose lights Ryu has set, one bit each.
    lit_banks: u8,
    percent: u8,
    colors: [Option<[u8; 3]>; DIALS as usize],
    /// The light last sent to each dial of the bank on show.
    sent: [Option<[u8; 3]>; DIALS as usize],
    knobs: [Knob; DIALS as usize],
    parser: Parser,
}

impl Twister {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            idle: IDLE,
            started: false,
            bank: 0,
            lit_banks: 0,
            percent: 100,
            colors: [None; DIALS as usize],
            sent: [None; DIALS as usize],
            knobs: [Knob::default(); DIALS as usize],
            parser: Parser::default(),
        }
    }

    fn number(&self, dial: u8) -> u8 {
        self.bank * DIALS + dial
    }

    /// Leaves native mode, shows bank 1 and centres its knobs.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        self.transport
            .write(&[0xf0, 0x00, 0x01, 0x79, 0x06, 0x00, 0x00, 0xf7])?;
        self.transport.write(&[SYSTEM, 0x00, PRESSED])?;
        self.show_bank(0)
    }

    /// Takes `bank` as the one on show: centres its knobs and sends their
    /// lights again.
    fn show_bank(&mut self, bank: u8) -> DriverResult<()> {
        self.bank = bank % MAX_BANKS;
        self.sent = [None; DIALS as usize];
        for dial in 0..DIALS {
            self.transport.write(&[TURN, self.number(dial), CENTRE])?;
            if let Some(knob) = self.knobs.get_mut(usize::from(dial)) {
                *knob = Knob {
                    last: CENTRE,
                    moved: None,
                    before_centre: None,
                };
            }
        }
        self.send_lights()
    }

    /// Sends each dial's light where it changed.
    fn send_lights(&mut self) -> DriverResult<()> {
        self.lit_banks |= 1 << self.bank;
        for dial in 0..DIALS {
            let index = usize::from(dial);
            let wanted = light(self.colors.get(index).copied().flatten(), self.percent);
            let before = self.sent.get(index).copied().flatten();
            let number = self.number(dial);
            for (position, status) in [PUSH, RGB_ANIMATION, RING_ANIMATION]
                .into_iter()
                .enumerate()
            {
                let value = wanted.get(position).copied().unwrap_or(0);
                if before.and_then(|sent| sent.get(position).copied()) != Some(value) {
                    self.transport.write(&[status, number, value])?;
                }
            }
            if let Some(sent) = self.sent.get_mut(index) {
                *sent = Some(wanted);
            }
        }
        Ok(())
    }

    /// Follows the bank a knob or button message came from.
    fn follow_bank(&mut self, bank: u8) -> DriverResult<()> {
        if bank % MAX_BANKS == self.bank {
            return Ok(());
        }
        self.show_bank(bank)
    }

    fn turn(&mut self, number: u8, value: u8, events: &mut Vec<Input>) -> DriverResult<()> {
        self.follow_bank(number / DIALS)?;
        let dial = number % DIALS;
        let Some(knob) = self.knobs.get_mut(usize::from(dial)) else {
            return Ok(());
        };
        let value = value.min(127);
        let step = |from: u8| i16::from(value) - i16::from(from);
        let mut delta = step(knob.last);
        // Right after a re-centre the deck may have kept its old value.
        if let Some(before) = knob.before_centre.take() {
            if step(before).abs() < delta.abs() {
                delta = step(before);
            }
        }
        knob.last = value;
        knob.moved = Some(Instant::now());
        if let Ok(delta) = i8::try_from(delta) {
            if delta != 0 {
                events.push(Input::DialTurn { index: dial, delta });
            }
        }
        Ok(())
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> DriverResult<()> {
        let Message::Channel([status, number, value]) = *message else {
            return Ok(());
        };
        match status {
            TURN => self.turn(number, value, events)?,
            PUSH => {
                self.follow_bank(number / DIALS)?;
                events.push(Input::DialPress {
                    index: number % DIALS,
                    pressed: value >= 64,
                });
            }
            SYSTEM if number < FIRST_SIDE => {
                if value == PRESSED {
                    self.follow_bank(number)?;
                }
            }
            SYSTEM => {
                let side = (number - FIRST_SIDE) % SIDES_PER_BANK;
                let key = match side {
                    0 => 0,
                    3 => 1,
                    2 => 2,
                    5 => 3,
                    // The middle buttons switch banks and are never keys.
                    _ => return Ok(()),
                };
                self.follow_bank((number - FIRST_SIDE) / SIDES_PER_BANK)?;
                events.push(Input::Key {
                    index: key,
                    pressed: value >= 64,
                });
            }
            _ => {}
        }
        Ok(())
    }

    /// Puts every knob that has been still long enough back to the centre.
    fn recentre(&mut self) -> DriverResult<()> {
        for dial in 0..DIALS {
            let number = self.number(dial);
            let Some(knob) = self.knobs.get_mut(usize::from(dial)) else {
                continue;
            };
            let Some(moved) = knob.moved else {
                continue;
            };
            if moved.elapsed() < self.idle {
                continue;
            }
            knob.moved = None;
            if knob.last != CENTRE {
                knob.before_centre = Some(knob.last);
                knob.last = CENTRE;
                self.transport.write(&[TURN, number, CENTRE])?;
            }
        }
        Ok(())
    }

    /// Reads whatever arrived, waiting up to `timeout_ms` for the first.
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
}

impl Driver for Twister {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_started()?;
        let mut events = Vec::new();
        for message in &self.read(timeout_ms)? {
            self.receive(message, &mut events)?;
        }
        self.recentre()?;
        Ok(events)
    }

    /// The side buttons have no lights.
    fn set_face(&mut self, _key: u8, _face: Option<&Face>) -> DriverResult<()> {
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        self.send_lights()
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.percent = percent.min(100);
        self.flush()
    }

    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if let Some(slot) = self.colors.get_mut(usize::from(dial)) {
            *slot = color;
        }
        Ok(())
    }
}

impl Drop for Twister {
    /// Stops the dimming and clears the colours of every bank Ryu lit, so the
    /// knobs show the user's own colours again.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for bank in (0..MAX_BANKS).filter(|bank| self.lit_banks & (1 << bank) != 0) {
            for dial in 0..DIALS {
                let number = bank * DIALS + dial;
                for status in [RGB_ANIMATION, RING_ANIMATION, PUSH] {
                    let _ = self.transport.write(&[status, number, 0]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    fn open() -> (Twister, Recorder) {
        let recorder = Recorder::default();
        let mut deck = Twister::new(by_id(TWISTER_ID).unwrap(), Box::new(recorder.clone()));
        deck.idle = Duration::from_secs(3600);
        (deck, recorder)
    }

    /// A started deck, with the start's writes cleared.
    fn ready() -> (Twister, Recorder) {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        (deck, recorder)
    }

    fn queue(recorder: &Recorder, messages: &[&[u8]]) {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
    }

    fn turn(index: u8, delta: i8) -> Input {
        Input::DialTurn { index, delta }
    }

    #[test]
    fn the_model_is_found_by_its_port_name() {
        for raw in [
            "Midi Fighter Twister:Midi Fighter Twister MIDI 1 24:0",
            "Midi Fighter Twister",
            "2- Midi Fighter Twister",
            "Midi Fighter Twister 3",
        ] {
            let model = identify_midi(&normalise(raw), false).unwrap();
            assert_eq!(model.id, TWISTER_ID, "{raw}");
        }
        assert!(identify_midi(&normalise("MIDIIN2 (Midi Fighter Twister)"), true).is_none());
        assert!(identify_midi(&normalise("Midi Fighter 3D"), false).is_none());
        let model = by_id(TWISTER_ID).unwrap();
        assert_eq!(model.dial_lights, DialLights::PerDial);
        let view = ModelView::from(model);
        assert!(view.brightness);
        assert_eq!(view.key_names, Some(NAMES.to_vec()));
    }

    #[test]
    fn opening_leaves_native_mode_shows_bank_one_and_centres_and_lights_its_knobs() {
        let (mut deck, recorder) = open();
        deck.set_dial_light(0, Some([255, 0, 0])).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(
            writes[..2],
            [
                vec![0xf0, 0x00, 0x01, 0x79, 0x06, 0x00, 0x00, 0xf7],
                vec![0xb3, 0x00, 0x7f]
            ]
        );
        for dial in 0..16u8 {
            assert_eq!(writes[2 + usize::from(dial)], vec![0xb0, dial, 0x40]);
        }
        // Then three bytes a dial: red at full, the rest dark.
        assert_eq!(writes.len(), 2 + 16 + 48);
        assert_eq!(
            writes[18..21],
            [vec![0xb1, 0, 84], vec![0xb2, 0, 48], vec![0xb5, 0, 96]]
        );
        assert_eq!(
            writes[21..24],
            [vec![0xb1, 1, 1], vec![0xb2, 1, 17], vec![0xb5, 1, 65]]
        );
        // A second flush with nothing changed sends nothing.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), writes.len());
    }

    #[test]
    fn flush_sends_only_what_changed_and_brightness_dims_both_lights() {
        let (mut deck, recorder) = ready();
        deck.set_dial_light(5, Some([0, 255, 0])).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![vec![0xb1, 5, 46], vec![0xb2, 5, 48], vec![0xb5, 5, 96]]
        );
        recorder.0.lock().unwrap().writes.clear();
        deck.set_brightness(50).unwrap();
        assert_eq!(
            recorder.writes(),
            vec![vec![0xb2, 5, 32], vec![0xb5, 5, 80]]
        );
        recorder.0.lock().unwrap().writes.clear();
        // Brightness 0 is the dark state.
        deck.set_brightness(0).unwrap();
        assert_eq!(
            recorder.writes(),
            vec![vec![0xb1, 5, 1], vec![0xb2, 5, 17], vec![0xb5, 5, 65]]
        );
    }

    #[test]
    fn colours_take_the_nearest_palette_hue() {
        assert_eq!(palette_index([0, 0, 255]), Some(1));
        assert_eq!(palette_index([0, 255, 255]), Some(22));
        assert_eq!(palette_index([0, 255, 0]), Some(46));
        assert_eq!(palette_index([255, 255, 0]), Some(64));
        assert_eq!(palette_index([255, 0, 0]), Some(84));
        assert_eq!(palette_index([255, 128, 0]), Some(74));
        assert_eq!(palette_index([255, 0, 255]), Some(107));
        assert_eq!(palette_index([111, 0, 255]), Some(114));
        assert_eq!(palette_index([40, 0, 255]), Some(1));
        assert_eq!(palette_index([255, 255, 255]), Some(ACTIVE_INDEX));
        assert_eq!(palette_index([128, 128, 128]), Some(ACTIVE_INDEX));
        assert_eq!(palette_index([20, 0, 31]), None);
        for red in (0..=255u8).step_by(15) {
            for green in (0..=255u8).step_by(15) {
                for blue in (0..=255u8).step_by(15) {
                    if let Some(index) = palette_index([red, green, blue]) {
                        assert!((1..=127).contains(&index), "{red} {green} {blue}");
                    }
                }
            }
        }
    }

    #[test]
    fn turns_are_differences_of_the_absolute_value() {
        let (mut deck, recorder) = ready();
        queue(&recorder, &[&[0xb0, 3, 65], &[0xb0, 3, 67], &[0xb0, 3, 66]]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(3, 1), turn(3, 2), turn(3, -1)]
        );
        // Running status, a real-time byte in between and a short read.
        queue(&recorder, &[&[0xb0, 4, 60, 0xf8, 4], &[58], &[0xb0, 4]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(4, -4), turn(4, -2)]);
        // The same value again is no turn.
        queue(&recorder, &[&[0xb0, 4, 58]]);
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn a_still_knob_is_put_back_to_the_centre() {
        let (mut deck, recorder) = ready();
        deck.idle = Duration::ZERO;
        queue(&recorder, &[&[0xb0, 2, 70]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(2, 6)]);
        assert_eq!(recorder.writes(), vec![vec![0xb0, 2, 64]]);
        // The deck took the centre: the next tick counts from 64.
        queue(&recorder, &[&[0xb0, 2, 65]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(2, 1)]);
        // A knob already at the centre is not sent again.
        recorder.0.lock().unwrap().writes.clear();
        queue(&recorder, &[&[0xb0, 2, 64]]);
        assert!(deck.poll(0).unwrap().is_empty());
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn a_turn_that_beat_the_recentre_counts_from_the_old_value() {
        let (mut deck, recorder) = ready();
        deck.idle = Duration::ZERO;
        queue(&recorder, &[&[0xb0, 7, 120]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(7, 56)]);
        // The deck ignored the centre and went on from 120.
        queue(&recorder, &[&[0xb0, 7, 121]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(7, 1)]);
    }

    #[test]
    fn pushes_and_side_buttons() {
        let (mut deck, recorder) = ready();
        queue(
            &recorder,
            &[
                &[0xb1, 9, 0x7f],
                &[0xb1, 9, 0x00],
                &[0xb3, 8, 0x7f],
                &[0xb3, 11, 0x7f],
                &[0xb3, 10, 0x7f],
                &[0xb3, 13, 0x00],
                // The middle buttons are never keys.
                &[0xb3, 9, 0x7f],
                &[0xb3, 12, 0x7f],
                // Other channels are ignored.
                &[0xb4, 1, 0x7f],
                &[0x93, 1, 0x7f],
                &[0xf0, 0x7e, 0x7f, 0x06, 0x02, 0xf7],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialPress {
                    index: 9,
                    pressed: true
                },
                Input::DialPress {
                    index: 9,
                    pressed: false
                },
                Input::Key {
                    index: 0,
                    pressed: true
                },
                Input::Key {
                    index: 1,
                    pressed: true
                },
                Input::Key {
                    index: 2,
                    pressed: true
                },
                Input::Key {
                    index: 3,
                    pressed: false
                },
            ]
        );
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn a_bank_change_centres_and_relights_the_new_bank() {
        let (mut deck, recorder) = ready();
        deck.set_dial_light(0, Some([255, 255, 0])).unwrap();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        // Bank 1 to bank 2.
        queue(&recorder, &[&[0xb3, 0, 0x00], &[0xb3, 1, 0x7f]]);
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(writes.len(), 16 + 48);
        assert_eq!(writes[0], vec![0xb0, 16, 64]);
        assert_eq!(writes[15], vec![0xb0, 31, 64]);
        assert_eq!(
            writes[16..19],
            [vec![0xb1, 16, 64], vec![0xb2, 16, 48], vec![0xb5, 16, 96]]
        );
        // Side buttons and knobs in bank 2 fold onto the same keys and dials.
        recorder.0.lock().unwrap().writes.clear();
        queue(
            &recorder,
            &[&[0xb3, 14, 0x7f], &[0xb0, 17, 66], &[0xb1, 31, 0x7f]],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 0,
                    pressed: true
                },
                turn(1, 2),
                Input::DialPress {
                    index: 15,
                    pressed: true
                },
            ]
        );
        assert!(recorder.writes().is_empty());
        // A knob from another bank (the eight-bank firmware) switches too.
        queue(&recorder, &[&[0xb0, 7 * 16 + 2, 65]]);
        assert_eq!(deck.poll(0).unwrap(), vec![turn(2, 1)]);
        assert_eq!(recorder.writes()[0], vec![0xb0, 112, 64]);
        assert_eq!(recorder.writes()[2], vec![0xb0, 114, 64]);
    }

    #[test]
    fn letting_go_restores_every_bank_ryu_lit() {
        let (mut deck, recorder) = ready();
        queue(&recorder, &[&[0xb3, 2, 0x7f]]);
        deck.poll(0).unwrap();
        recorder.0.lock().unwrap().writes.clear();
        drop(deck);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 2 * 16 * 3);
        assert_eq!(
            writes[..3],
            [vec![0xb2, 0, 0], vec![0xb5, 0, 0], vec![0xb1, 0, 0]]
        );
        assert_eq!(writes[16 * 3], vec![0xb2, 32, 0]);
    }

    #[test]
    fn a_deck_never_started_sends_nothing_on_letting_go() {
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }
}
