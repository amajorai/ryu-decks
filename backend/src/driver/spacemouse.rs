//! 3Dconnexion 3D mice (SpaceNavigator, SpaceMouse, SpacePilot, SpaceExplorer):
//! a six-axis puck, a button bitmap and, on most models, one LED that is only
//! on or off. No screens, no colours and no brightness.
//!
//! The device streams plain input reports with the report id in byte 0:
//!
//! - `01 tx ty tz` (three signed 16-bit little-endian values), and, in a report
//!   of 13 bytes or more (the wireless and newer models), `rx ry rz` after them;
//! - `02 rx ry rz` (the wired models send rotation on its own);
//! - `03` then the button bitmap, little-endian, as many bytes as the model has.
//!
//! Anything else (`17 64 01` from a SpaceMouse Wireless, probably its battery)
//! is ignored, and so is a report too short to hold what its id promises.
//!
//! The puck is raised as six dials, in the order TX, TY, TZ, RX, RY, RZ: each
//! axis is quantised to a position of -7 to 7 (with a dead zone, since a puck at
//! rest jitters by a few counts) and a change of position is a dial turn by the
//! difference, the way the Contour shuttle ring is. The first sample of an axis
//! only sets its baseline. The only output is the LED: report `04 01` turns it
//! on and `04 00` off.

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const READ_BUFFER: usize = 64;
/// The puck reports about every 7 ms while it is pushed, so drain generously.
const MAX_REPORTS_PER_POLL: usize = 32;
const TRANSLATION_REPORT: u8 = 0x01;
const ROTATION_REPORT: u8 = 0x02;
const BUTTON_REPORT: u8 = 0x03;
const LED_REPORT: u8 = 0x04;
/// Counts either side of rest that read as rest.
const DEAD_ZONE: u16 = 30;
/// Counts per dial position past the dead zone; full deflection is about 350.
const STEP: u16 = 45;
/// The furthest position an axis reaches either way.
const MAX_POSITION: i8 = 7;
/// A report 1 this long carries rotation too (bytes 7 to 12).
const SIX_AXIS_REPORT_LEN: usize = 13;

/// The SpaceNavigator, SpaceMouse Compact and SpaceMouse Wireless: LEFT, RIGHT.
const TWO_BUTTONS: &[u64] = &[1 << 0, 1 << 1];
/// The SpaceMouse Compact's two buttons. The references disagree on whether
/// they sit in the bitmap's first byte or its second, so either counts.
const COMPACT: &[u64] = &[1 << 0 | 1 << 8, 1 << 1 | 1 << 9];
/// SpaceMouse Pro, Pro Wireless and the Universal Receiver.
const PRO: &[u64] = &bits(&[0, 1, 2, 4, 5, 8, 12, 13, 14, 15, 22, 23, 24, 25, 26]);
const PRO_NAMES: &[&str] = &[
    "Menu", "Fit", "Top", "Right", "Front", "Roll", "1", "2", "3", "4", "Esc", "Alt", "Shift",
    "Ctrl", "Rotation",
];
/// SpacePilot PRO. The numbered keys follow pyspacemouse's table as written
/// (1 and 2 on bits 17 and 18, 3 to 5 on bits 14 to 16).
const SPACEPILOT_PRO: &[u64] = &bits(&[
    0, 1, 2, 4, 5, 8, 10, 17, 18, 14, 15, 16, 22, 23, 24, 25, 26, 27, 28, 29, 30,
]);
const SPACEPILOT_PRO_NAMES: &[&str] = &[
    "Menu", "Fit", "Top", "Right", "Front", "Roll", "ISO", "1", "2", "3", "4", "5", "Esc", "Alt",
    "Shift", "Ctrl", "Rotation", "Pan/Zoom", "Dominant", "+", "-",
];
/// SpaceMouse Enterprise: the 22 of its 31 keys the references map.
const ENTERPRISE: &[u64] = &bits(&[
    0, 1, 2, 4, 5, 8, 10, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
]);
const ENTERPRISE_NAMES: &[&str] = &[
    "Menu", "Fit", "Top", "Right", "Front", "Roll", "ISO", "1", "2", "3", "4", "5", "6", "7", "8",
    "9", "10", "Esc", "Alt", "Shift", "Ctrl", "Lock",
];
/// SpaceExplorer: fifteen keys on bits 0 to 14.
const SPACE_EXPLORER: &[u64] = &bits(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
const SPACE_EXPLORER_NAMES: &[&str] = &[
    "1", "2", "Top", "Left", "Right", "Front", "Esc", "Alt", "Shift", "Ctrl", "Fit", "Panel", "+",
    "-", "2D",
];
/// The classic SpacePilot: 21 keys on bits 0 to 20.
const SPACEPILOT: &[u64] = &bits(&[
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
]);
const SPACEPILOT_NAMES: &[&str] = &[
    "1", "2", "3", "4", "5", "6", "Top", "Left", "Right", "Front", "Esc", "Alt", "Shift", "Ctrl",
    "Fit", "Panel", "Zoom -", "Zoom +", "Dominant", "3D lock", "Config",
];

/// One mask per bit, in order.
const fn bits<const N: usize>(positions: &[u8; N]) -> [u64; N] {
    let mut masks = [0u64; N];
    let mut at = 0;
    while at < N {
        masks[at] = 1 << positions[at];
        at += 1;
    }
    masks
}

/// The button masks of a model, in key order: a key is down while any bit of
/// its mask is set.
fn key_masks(model_id: &str) -> &'static [u64] {
    match model_id {
        "3dconnexion-spacemouse-compact" => COMPACT,
        "3dconnexion-spacemouse-pro"
        | "3dconnexion-spacemouse-pro-wireless"
        | "3dconnexion-universal-receiver" => PRO,
        "3dconnexion-spacepilot-pro" => SPACEPILOT_PRO,
        "3dconnexion-spacemouse-enterprise" => ENTERPRISE,
        "3dconnexion-spaceexplorer" => SPACE_EXPLORER,
        "3dconnexion-spacepilot" => SPACEPILOT,
        "3dconnexion-spacemouse-usb" => &[],
        _ => TWO_BUTTONS,
    }
}

/// The legend on each key of a model, in key order (empty for a model with no
/// buttons).
#[must_use]
pub fn key_names(model_id: &str) -> &'static [&'static str] {
    match model_id {
        "3dconnexion-spacemouse-pro"
        | "3dconnexion-spacemouse-pro-wireless"
        | "3dconnexion-universal-receiver" => PRO_NAMES,
        "3dconnexion-spacepilot-pro" => SPACEPILOT_PRO_NAMES,
        "3dconnexion-spacemouse-enterprise" => ENTERPRISE_NAMES,
        "3dconnexion-spaceexplorer" => SPACE_EXPLORER_NAMES,
        "3dconnexion-spacepilot" => SPACEPILOT_NAMES,
        "3dconnexion-spacemouse-usb" => &[],
        _ => &["Left", "Right"],
    }
}

/// An axis value as a dial position: 0 inside the dead zone, then one step
/// every [`STEP`] counts, up to [`MAX_POSITION`] either way.
fn position(value: i16) -> i8 {
    let past = value.unsigned_abs().saturating_sub(DEAD_ZONE) / STEP;
    let steps = i8::try_from(past).unwrap_or(MAX_POSITION).min(MAX_POSITION);
    if value < 0 {
        -steps
    } else {
        steps
    }
}

/// The signed 16-bit little-endian value at `at`, if the report holds it.
fn axis(report: &[u8], at: usize) -> Option<i16> {
    Some(i16::from_le_bytes([*report.get(at)?, *report.get(at + 1)?]))
}

/// Three axes starting at `at`, if the report holds all of them.
fn three_axes(report: &[u8], at: usize) -> Option<[i16; 3]> {
    Some([
        axis(report, at)?,
        axis(report, at + 2)?,
        axis(report, at + 4)?,
    ])
}

pub struct SpaceMouse {
    transport: Box<dyn Transport>,
    masks: &'static [u64],
    keys: Vec<bool>,
    /// Each axis's last dial position; `None` until its first sample.
    axes: [Option<i8>; 6],
    /// Bits no key is mapped to that were last seen set, so each new one is
    /// logged once.
    unmapped: u64,
    has_led: bool,
    /// The LED state staged by [`Driver::set_deck_light`] and the one last sent.
    led: bool,
    sent_led: Option<bool>,
}

impl SpaceMouse {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let masks = key_masks(model.id);
        Self {
            transport,
            masks,
            keys: vec![false; masks.len()],
            axes: [None; 6],
            unmapped: 0,
            has_led: model.deck_lights > 0,
            led: false,
            sent_led: None,
        }
    }

    fn parse(&mut self, report: &[u8]) -> Vec<Input> {
        let mut events = Vec::new();
        match report.first() {
            Some(&TRANSLATION_REPORT) => {
                if let Some(values) = three_axes(report, 1) {
                    self.move_axes(0, values, &mut events);
                }
                if report.len() >= SIX_AXIS_REPORT_LEN {
                    if let Some(values) = three_axes(report, 7) {
                        self.move_axes(3, values, &mut events);
                    }
                }
            }
            Some(&ROTATION_REPORT) => {
                if let Some(values) = three_axes(report, 1) {
                    self.move_axes(3, values, &mut events);
                }
            }
            Some(&BUTTON_REPORT) => self.buttons(report, &mut events),
            _ => {}
        }
        events
    }

    fn move_axes(&mut self, first: usize, values: [i16; 3], events: &mut Vec<Input>) {
        for (offset, value) in values.into_iter().enumerate() {
            let index = first + offset;
            let now = position(value);
            let Some(slot) = self.axes.get_mut(index) else {
                continue;
            };
            if let Some(before) = slot.replace(now) {
                let delta = now - before;
                if delta != 0 {
                    events.push(Input::DialTurn {
                        index: index as u8,
                        delta,
                    });
                }
            }
        }
    }

    fn buttons(&mut self, report: &[u8], events: &mut Vec<Input>) {
        let mut bitmap = [0u8; 8];
        for (slot, byte) in bitmap.iter_mut().zip(report.iter().skip(1)) {
            *slot = *byte;
        }
        let bitmap = u64::from_le_bytes(bitmap);
        for (index, (mask, held)) in self.masks.iter().zip(self.keys.iter_mut()).enumerate() {
            let pressed = bitmap & mask != 0;
            if pressed != *held {
                *held = pressed;
                events.push(Input::Key {
                    index: index as u8,
                    pressed,
                });
            }
        }
        let mapped = self.masks.iter().fold(0, |all, mask| all | mask);
        let unmapped = bitmap & !mapped;
        if unmapped & !self.unmapped != 0 {
            tracing::debug!(
                bits = format!("{unmapped:#x}"),
                "ryu-decks: 3Dconnexion button bits with no key"
            );
        }
        self.unmapped = unmapped;
    }
}

impl Driver for SpaceMouse {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            events.extend(self.parse(&buf[..read]));
        }
        Ok(events)
    }

    /// No screens: a layout only names the keys in the Companion.
    fn set_face(&mut self, _key: u8, _face: Option<&Face>) -> DriverResult<()> {
        Ok(())
    }

    /// Sends the LED state when it changed. Only the SpaceNavigator's LED is
    /// known to answer, so a refused write is logged, never an error.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.has_led || self.sent_led == Some(self.led) {
            return Ok(());
        }
        let report = [LED_REPORT, u8::from(self.led)];
        if let Err(error) = self.transport.write(&report) {
            tracing::debug!(%error, "ryu-decks: the 3Dconnexion LED did not take the report");
        }
        self.sent_led = Some(self.led);
        Ok(())
    }

    /// The LED is only on or off.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }

    /// Any colour turns the LED on; `None` turns it off.
    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        self.led = color.is_some();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn driver(model: &str) -> (SpaceMouse, Recorder) {
        let recorder = Recorder::default();
        (
            SpaceMouse::new(by_id(model).unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    fn turn(index: u8, delta: i8) -> Input {
        Input::DialTurn { index, delta }
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    #[test]
    fn axes_decode_as_signed_little_endian_from_the_references_captures() {
        assert_eq!(
            three_axes(&[0x01, 0, 0, 0, 0, 0x34, 0x00], 1),
            Some([0, 0, 52])
        );
        assert_eq!(
            three_axes(&[0x02, 0, 0, 0, 0, 0x2d, 0x00], 1),
            Some([0, 0, 45])
        );
        let wireless = [0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x30, 0x00];
        assert_eq!(three_axes(&wireless, 7), Some([0, 0, 48]));
        assert_eq!(axis(&[0x01, 0xa2, 0xfe], 1), Some(-350));
        assert_eq!(axis(&[0x01, 0xa2], 1), None);
        assert_eq!(
            (position(29), position(74), position(75), position(-350)),
            (0, 0, 1, -7)
        );
        assert_eq!((position(i16::MAX), position(i16::MIN)), (7, -7));
    }

    #[test]
    fn a_wired_puck_turns_dials_from_reports_one_and_two() {
        let (mut deck, recorder) = driver("3dconnexion-spacenavigator");
        // Baselines: translation at rest, rotation at rest.
        recorder.queue_read(vec![0x01, 0, 0, 0, 0, 0x34, 0x00]);
        recorder.queue_read(vec![0x02, 0, 0, 0, 0, 0x2d, 0x00]);
        // Push right (TX +120: position 2) and twist (RZ -200: position -3).
        recorder.queue_read(vec![0x01, 120, 0, 0, 0, 0, 0]);
        recorder.queue_read(vec![0x02, 0, 0, 0, 0, 0x38, 0xff]);
        // Back to rest.
        recorder.queue_read(vec![0x01, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(0, 2), turn(5, -3), turn(0, -2)]
        );
        assert!(recorder.writes().is_empty(), "input needs no handshake");
    }

    #[test]
    fn a_long_report_one_carries_rotation_and_a_short_one_does_not() {
        let (mut deck, recorder) = driver("3dconnexion-spacemouse-wireless");
        recorder.queue_read(vec![0x01, 0, 0, 0, 0, 0x19, 0, 0, 0, 0, 0, 0, 0]);
        // RY +100 (position 1) and TY -300 (position -6).
        recorder.queue_read(vec![0x01, 0, 0, 0xd4, 0xfe, 0, 0, 0, 0, 100, 0, 0, 0]);
        // A short report 1 moves translation only.
        recorder.queue_read(vec![0x01, 0, 0, 0, 0, 0, 0, 0xff, 0x7f]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(1, -6), turn(4, 1), turn(1, 6)]
        );
    }

    #[test]
    fn short_and_unknown_reports_are_ignored() {
        let (mut deck, recorder) = driver("3dconnexion-spacemouse-wireless");
        for report in [
            vec![0x17, 0x64, 0x01],
            vec![0x01, 0x00],
            vec![0x02, 0x10, 0x00, 0x20],
            vec![0x03],
            vec![0x09; 20],
        ] {
            recorder.queue_read(report);
        }
        assert!(deck.poll(0).unwrap().is_empty());
        // No baseline was taken from a short report.
        recorder.queue_read(vec![0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(deck.poll(0).unwrap().is_empty());
    }

    #[test]
    fn buttons_raise_presses_and_releases_on_change() {
        let (mut deck, recorder) = driver("3dconnexion-spacenavigator");
        recorder.queue_read(vec![0x03, 0b01, 0x00]);
        recorder.queue_read(vec![0x03, 0b11, 0x00]);
        recorder.queue_read(vec![0x03, 0b11, 0x00]);
        recorder.queue_read(vec![0x03, 0b00, 0x00]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![key(0, true), key(1, true), key(0, false), key(1, false)]
        );
    }

    #[test]
    fn the_compact_takes_its_buttons_from_either_byte() {
        let (mut deck, recorder) = driver("3dconnexion-spacemouse-compact");
        recorder.queue_read(vec![0x03, 0x00, 0b10]);
        recorder.queue_read(vec![0x03, 0b10, 0x00]);
        recorder.queue_read(vec![0x03, 0x00, 0x00]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(1, true), key(1, false)]);
    }

    #[test]
    fn the_pro_maps_its_keys_across_four_bytes() {
        let (mut deck, recorder) = driver("3dconnexion-spacemouse-pro");
        // FIT (bit 1), "1" (bit 12), ESC (bit 22) and ROTATION (bit 26).
        recorder.queue_read(vec![0x03, 0b10, 0b0001_0000, 0b0100_0000, 0b0100]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![key(1, true), key(6, true), key(10, true), key(14, true)]
        );
        assert_eq!(key_names("3dconnexion-spacemouse-pro")[14], "Rotation");
    }

    #[test]
    fn the_spacepilot_pro_and_enterprise_follow_their_tables() {
        let (mut pilot, recorder) = driver("3dconnexion-spacepilot-pro");
        // Bit 17 is "1" (key 7), bit 14 is "3" (key 9), bit 30 is MINUS (key 20).
        recorder.queue_read(vec![0x03, 0, 0b0100_0000, 0b10, 0b0100_0000]);
        assert_eq!(
            pilot.poll(0).unwrap(),
            vec![key(7, true), key(9, true), key(20, true)]
        );
        let (mut enterprise, recorder) = driver("3dconnexion-spacemouse-enterprise");
        // Bit 21 is "10" (key 16), bit 26 is LOCK (key 21); bit 31 has no key.
        recorder.queue_read(vec![0x03, 0, 0, 0b0010_0000, 0b1000_0100]);
        assert_eq!(
            enterprise.poll(0).unwrap(),
            vec![key(16, true), key(21, true)]
        );
        assert_eq!(key_names("3dconnexion-spacemouse-enterprise")[16], "10");
    }

    #[test]
    fn the_spaceexplorer_and_spacepilot_use_sequential_bits() {
        let (mut explorer, recorder) = driver("3dconnexion-spaceexplorer");
        recorder.queue_read(vec![0x03, 0, 0b0100_0000]);
        assert_eq!(explorer.poll(0).unwrap(), vec![key(14, true)]);
        assert_eq!(key_names("3dconnexion-spaceexplorer")[14], "2D");
        let (mut pilot, recorder) = driver("3dconnexion-spacepilot");
        recorder.queue_read(vec![0x03, 0, 0, 0b1_0000]);
        assert_eq!(pilot.poll(0).unwrap(), vec![key(20, true)]);
        assert_eq!(key_names("3dconnexion-spacepilot")[20], "Config");
    }

    #[test]
    fn every_model_names_each_key() {
        for model in crate::catalog::MODELS
            .iter()
            .filter(|model| model.protocol == crate::catalog::Protocol::SpaceMouseHid)
        {
            assert_eq!(
                key_masks(model.id).len(),
                usize::from(model.keys),
                "{}",
                model.id
            );
            assert_eq!(
                key_names(model.id).len(),
                usize::from(model.keys),
                "{}",
                model.id
            );
        }
    }

    #[test]
    fn the_led_is_on_for_any_colour_and_sent_once_per_change() {
        let (mut deck, recorder) = driver("3dconnexion-spacenavigator");
        deck.set_deck_light(Some([0, 0, 255])).unwrap();
        deck.flush().unwrap();
        deck.set_deck_light(Some([255, 0, 0])).unwrap();
        deck.flush().unwrap();
        deck.set_deck_light(None).unwrap();
        deck.flush().unwrap();
        deck.set_brightness(10).unwrap();
        deck.set_face(0, None).unwrap();
        assert_eq!(recorder.writes(), vec![vec![0x04, 0x01], vec![0x04, 0x00]]);
    }

    #[test]
    fn a_model_without_an_led_writes_nothing() {
        let (mut deck, recorder) = driver("3dconnexion-spacepilot");
        deck.set_deck_light(Some([255, 255, 255])).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
    }
}
