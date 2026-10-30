//! Adafruit MacroPad RP2040, running the Companion CircuitPython HID firmware
//! (`boot.py`, `code.py` and `settings.py` from maximkulkin/nodejs-macropad).
//!
//! Twelve keys in four rows of three, each over an RGB NeoPixel. The board's
//! encoder, its push switch, the OLED and the speaker are not handled by the
//! firmware, so Ryu models none of them.
//!
//! The firmware enables one HID device with three top-level collections, all
//! on the Consumer page (usage 0x01): report 1 is the keys, report 2 sets one
//! key's colour, report 3 turns one key off. Key numbers are 1 to 12 both
//! ways, row by row from the top left, so Ryu's key is the number minus one.
//!
//! In: `01 k0 k1 k2 k3 k4 k5`, the keys held now (0 an empty slot) in no
//! useful order, sent once per press or release. At most six are reported; a
//! seventh press is dropped. Out: `02 key R G B`, applied at once. The
//! firmware keeps only the latest report of each id per loop, so writes are
//! spaced a few milliseconds apart and every key is resent now and then.
//! Report 3 is never used: the firmware applies it after report 2 in the same
//! loop, so an "off" could win over a later colour. There is no brightness
//! command, so Ryu scales the colours, and caps them, since twelve NeoPixels
//! at full white draw more than a USB port should give.
//!
//! Device id 239a:8108 is CircuitPython's generic MacroPad id, which stock
//! CircuitPython and users' own keyboards share, so a collection is driven
//! only when its report descriptor is the Companion firmware's
//! ([`collection_roles`]). Windows lists the three collections as separate
//! paths, each with a descriptor of its own: keys are then read from one and
//! colours written to another.
//!
//! Facts per the MIT-licensed nodejs-macropad (Maxim Kulkin) and its firmware;
//! no code was copied.

use std::time::Duration;

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const MODEL_ID: &str = "adafruit-macropad-rp2040";
const KEYS: usize = 12;
/// The six key slots of the input report.
const SLOTS: usize = 6;
const KEY_REPORT: u8 = 0x01;
const COLOR_REPORT: u8 = 0x02;
const MAX_REPORTS_PER_POLL: usize = 32;
/// A colour channel at 100 % brightness, out of 255: twelve white keys at
/// full NeoPixel drive would want about 720 mA.
const CEILING: u16 = 128;
/// The gap between two writes, so the firmware's loop sees each one.
const WRITE_PACE: Duration = Duration::from_millis(4);
/// Heartbeats between two resends of every key, in case one write was lost.
const REFRESH_EVERY: u8 = 5;

const BLACK: [u8; 3] = [0, 0, 0];

const BUTTON_PAGE: u16 = 0x09;
const CONSUMER_PAGE: u16 = 0x0c;
const LED_PAGE: u16 = 0x08;
/// Consumer page: Programmable Buttons, the key collection's usage.
const PROGRAMMABLE_BUTTONS: u16 = 0x03;
/// LED page: RGB LED, the colour collection's usage.
const RGB_LED: u16 = 0x52;
const RED_CHANNEL: u16 = 0x53;
const BLUE_CHANNEL: u16 = 0x54;
const GREEN_CHANNEL: u16 = 0x55;

/// What one HID collection of the firmware does, read from its descriptor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Roles {
    /// Report 1, the held keys, on the Button page inside Programmable Buttons.
    pub reads_keys: bool,
    /// Report 2, an RGB LED's red, green and blue channels.
    pub takes_colours: bool,
}

/// The firmware's roles a report descriptor declares. Linux and macOS give
/// the whole descriptor (both roles); Windows rebuilds one per collection.
/// Any other descriptor (stock CircuitPython's consumer control, a user's own
/// keyboard) declares neither.
#[must_use]
pub fn collection_roles(descriptor: &[u8]) -> Roles {
    let mut roles = Roles::default();
    let mut page: u16 = 0;
    let mut report_id: u8 = 0;
    // Usages declared since the last main item, as (page, usage).
    let mut usages: Vec<(u16, u16)> = Vec::new();
    let mut usage_min: Option<(u16, u16)> = None;
    // The usage of each open collection.
    let mut collections: Vec<(u16, u16)> = Vec::new();
    let mut channels = [false; 3];
    let mut at = 0;
    while let Some(&prefix) = descriptor.get(at) {
        if prefix == 0xfe {
            // A long item: its data size is the next byte.
            let size = descriptor.get(at + 1).copied().unwrap_or(0);
            at += 3 + usize::from(size);
            continue;
        }
        let size = match prefix & 0x03 {
            3 => 4,
            n => usize::from(n),
        };
        let Some(data) = descriptor.get(at + 1..at + 1 + size) else {
            break;
        };
        at += 1 + size;
        let value = data
            .iter()
            .rev()
            .fold(0u32, |acc, byte| (acc << 8) | u32::from(*byte));
        // A 4-byte usage carries its own page in the high half.
        let usage = |value: u32| {
            if size == 4 {
                ((value >> 16) as u16, value as u16)
            } else {
                (page, value as u16)
            }
        };
        match (prefix >> 2) & 0x03 {
            // Main items.
            0 => {
                let tag = prefix >> 4;
                match tag {
                    // Collection.
                    0x0a => {
                        collections.push(usages.first().copied().unwrap_or((page, 0)));
                    }
                    // End Collection.
                    0x0c => {
                        collections.pop();
                    }
                    // Input: the key array.
                    0x08 if report_id == KEY_REPORT
                        && page == BUTTON_PAGE
                        && collections.contains(&(CONSUMER_PAGE, PROGRAMMABLE_BUTTONS)) =>
                    {
                        roles.reads_keys = true;
                    }
                    // Output: the colour channels.
                    0x09 if report_id == COLOR_REPORT
                        && collections.contains(&(LED_PAGE, RGB_LED)) =>
                    {
                        for (usage_page, usage) in &usages {
                            match (*usage_page, *usage) {
                                (LED_PAGE, RED_CHANNEL) => channels[0] = true,
                                (LED_PAGE, GREEN_CHANNEL) => channels[1] = true,
                                (LED_PAGE, BLUE_CHANNEL) => channels[2] = true,
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
                usages.clear();
                usage_min = None;
            }
            // Global items.
            1 => match prefix >> 4 {
                0x00 => page = value as u16,
                0x08 => report_id = value as u8,
                _ => {}
            },
            // Local items.
            2 => match prefix >> 4 {
                0x00 => usages.push(usage(value)),
                0x01 => usage_min = Some(usage(value)),
                0x02 => {
                    if let Some((min_page, min)) = usage_min.take() {
                        let (_, max) = usage(value);
                        for one in min..=max.min(min.saturating_add(0xff)) {
                            usages.push((min_page, one));
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    roles.takes_colours = channels.iter().all(|seen| *seen);
    roles
}

/// `colour * percent / 100`, then capped to [`CEILING`], per channel.
fn dimmed(color: [u8; 3], percent: u8) -> [u8; 3] {
    color.map(|channel| {
        (u32::from(channel) * u32::from(percent) * u32::from(CEILING) / (100 * 255)) as u8
    })
}

/// The keys an input report holds, by Ryu index; `None` for another report.
fn held_keys(report: &[u8]) -> Option<[bool; KEYS]> {
    if report.first() != Some(&KEY_REPORT) {
        return None;
    }
    let mut held = [false; KEYS];
    for number in report.iter().skip(1).take(SLOTS) {
        if (1..=KEYS as u8).contains(number) {
            held[usize::from(*number - 1)] = true;
        }
    }
    Some(held)
}

pub struct MacroPad {
    /// Where the key reports arrive (and colours go, with no `lights`).
    transport: Box<dyn Transport>,
    /// The colour collection, where Windows lists it apart from the keys'.
    lights: Option<Box<dyn Transport>>,
    held: [bool; KEYS],
    /// The colour each key should show, at full brightness.
    colors: [[u8; 3]; KEYS],
    dirty: [bool; KEYS],
    brightness: u8,
    heartbeats: u8,
    pace: Duration,
    /// A write already went out, so the next waits [`WRITE_PACE`].
    wrote: bool,
}

impl MacroPad {
    /// Blanks every key: the firmware keeps whatever a previous host set.
    #[must_use]
    pub fn new(
        model: &'static DeviceModel,
        transport: Box<dyn Transport>,
        lights: Option<Box<dyn Transport>>,
    ) -> Self {
        Self::paced(model, transport, lights, WRITE_PACE)
    }

    fn paced(
        model: &'static DeviceModel,
        transport: Box<dyn Transport>,
        lights: Option<Box<dyn Transport>>,
        pace: Duration,
    ) -> Self {
        let mut pad = Self {
            transport,
            lights,
            held: [false; KEYS],
            colors: [BLACK; KEYS],
            dirty: [false; KEYS],
            brightness: 100,
            heartbeats: 0,
            pace,
            wrote: false,
        };
        if let Err(error) = pad.send_all() {
            tracing::debug!(device = model.id, %error, "ryu-decks: macropad blanking failed");
        }
        pad
    }

    fn send_key(&mut self, key: usize) -> DriverResult<()> {
        let [r, g, b] = dimmed(self.colors[key], self.brightness);
        if self.wrote && !self.pace.is_zero() {
            std::thread::sleep(self.pace);
        }
        self.wrote = true;
        let report = [COLOR_REPORT, key as u8 + 1, r, g, b];
        match self.lights.as_mut() {
            Some(lights) => lights.write(&report),
            None => self.transport.write(&report),
        }
    }

    fn send_all(&mut self) -> DriverResult<()> {
        self.dirty = [false; KEYS];
        for key in 0..KEYS {
            self.send_key(key)?;
        }
        Ok(())
    }

    fn receive(&mut self, report: &[u8], events: &mut Vec<Input>) {
        let Some(held) = held_keys(report) else {
            return;
        };
        for (index, (now, before)) in held.iter().zip(self.held).enumerate() {
            if *now != before {
                events.push(Input::Key {
                    index: index as u8,
                    pressed: *now,
                });
            }
        }
        self.held = held;
    }
}

impl Driver for MacroPad {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; 64];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.receive(&buf[..read], &mut events);
        }
        self.wrote = false;
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let key = usize::from(key);
        if key >= KEYS {
            return Ok(());
        }
        let color = face.map_or(BLACK, |face| face.color);
        if self.colors[key] != color {
            self.colors[key] = color;
            self.dirty[key] = true;
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        for key in 0..KEYS {
            if self.dirty[key] {
                self.dirty[key] = false;
                self.send_key(key)?;
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        let percent = percent.min(100);
        if percent == self.brightness {
            return Ok(());
        }
        self.brightness = percent;
        self.send_all()
    }

    /// Resends every key now and then, in case the firmware's loop missed one.
    fn heartbeat(&mut self) -> DriverResult<()> {
        self.heartbeats += 1;
        if self.heartbeats < REFRESH_EVERY {
            return Ok(());
        }
        self.heartbeats = 0;
        self.send_all()
    }
}

impl Drop for MacroPad {
    /// Leaves the keys dark when Ryu lets go of the deck.
    fn drop(&mut self) {
        self.colors = [BLACK; KEYS];
        let _ = self.send_all();
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::by_id;

    /// The firmware's descriptor, whole as Linux and macOS give it.
    const DESCRIPTOR: &[u8] = &[
        0x05, 0x0c, 0x09, 0x01, 0xa1, 0x04, 0x85, 0x01, 0x09, 0x03, 0xa1, 0x04, 0x05, 0x09, 0x09,
        0x03, 0x15, 0x00, 0x25, 0x0c, 0x75, 0x08, 0x95, 0x06, 0x81, 0x00, 0xc0, 0xc0, //
        0x05, 0x0c, 0x09, 0x01, 0xa1, 0x04, 0x85, 0x02, 0x05, 0x08, 0x09, 0x52, 0xa1, 0x02, 0x05,
        0x08, 0x09, 0x3d, 0x15, 0x00, 0x25, 0x0c, 0x75, 0x08, 0x95, 0x01, 0x91, 0x00, 0x05, 0x08,
        0x09, 0x53, 0x15, 0x00, 0x25, 0xff, 0x75, 0x08, 0x95, 0x01, 0x91, 0x02, 0x05, 0x08, 0x09,
        0x55, 0x15, 0x00, 0x25, 0xff, 0x75, 0x08, 0x95, 0x01, 0x91, 0x02, 0x05, 0x08, 0x09, 0x54,
        0x15, 0x00, 0x25, 0xff, 0x75, 0x08, 0x95, 0x01, 0x91, 0x02, 0xc0, 0xc0, //
        0x05, 0x0c, 0x09, 0x01, 0xa1, 0x04, 0x85, 0x03, 0x05, 0x08, 0x09, 0x52, 0xa1, 0x04, 0x05,
        0x08, 0x09, 0x41, 0x15, 0x00, 0x25, 0x0c, 0x75, 0x08, 0x95, 0x01, 0x91, 0x02, 0xc0, 0xc0,
    ];
    /// Where the key collection ends and the colour collection starts.
    const SECOND: usize = 28;
    const THIRD: usize = 100;

    fn pad() -> (MacroPad, Recorder) {
        let recorder = Recorder::default();
        let pad = MacroPad::paced(
            by_id(MODEL_ID).unwrap(),
            Box::new(recorder.clone()),
            None,
            Duration::ZERO,
        );
        recorder.0.lock().unwrap().writes.clear();
        (pad, recorder)
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

    #[test]
    fn the_whole_descriptor_reads_keys_and_takes_colours() {
        assert_eq!(DESCRIPTOR.len(), 130);
        assert_eq!(DESCRIPTOR[SECOND..SECOND + 2], [0x05, 0x0c]);
        assert_eq!(DESCRIPTOR[THIRD..THIRD + 2], [0x05, 0x0c]);
        assert_eq!(
            collection_roles(DESCRIPTOR),
            Roles {
                reads_keys: true,
                takes_colours: true
            }
        );
    }

    #[test]
    fn each_windows_collection_has_one_role() {
        assert_eq!(
            collection_roles(&DESCRIPTOR[..SECOND]),
            Roles {
                reads_keys: true,
                takes_colours: false
            }
        );
        assert_eq!(
            collection_roles(&DESCRIPTOR[SECOND..THIRD]),
            Roles {
                reads_keys: false,
                takes_colours: true
            }
        );
        assert_eq!(collection_roles(&DESCRIPTOR[THIRD..]), Roles::default());
    }

    #[test]
    fn rebuilt_descriptors_with_extended_usages_and_ranges_still_match() {
        // Report 2 with 4-byte (page-carrying) usages and the channels as a
        // usage range, as a host may rebuild them.
        let colours = [
            0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x02, 0x0b, 0x52, 0x00, 0x08, 0x00, 0xa1,
            0x02, 0x05, 0x08, 0x19, 0x53, 0x29, 0x55, 0x75, 0x08, 0x95, 0x03, 0x91, 0x02, 0xc0,
            0xc0,
        ];
        assert!(collection_roles(&colours).takes_colours);
    }

    #[test]
    fn other_circuitpython_descriptors_are_not_the_firmware() {
        // Stock CircuitPython's consumer control: report 3, Consumer usages.
        let consumer = [
            0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x03, 0x75, 0x10, 0x95, 0x01, 0x15, 0x01,
            0x26, 0x8c, 0x02, 0x19, 0x01, 0x2a, 0x8c, 0x02, 0x81, 0x00, 0xc0,
        ];
        assert_eq!(collection_roles(&consumer), Roles::default());
        // A keyboard with report 1 and caps-lock style LEDs.
        let keyboard = [
            0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7,
            0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x05, 0x08, 0x19, 0x01,
            0x29, 0x05, 0x91, 0x02, 0xc0,
        ];
        assert_eq!(collection_roles(&keyboard), Roles::default());
        // Truncated and empty descriptors never panic.
        assert_eq!(collection_roles(&[]), Roles::default());
        assert_eq!(collection_roles(&DESCRIPTOR[..7]), Roles::default());
        assert_eq!(collection_roles(&[0xfe, 0xff]), Roles::default());
    }

    #[test]
    fn opening_blanks_all_twelve_keys() {
        let recorder = Recorder::default();
        let _pad = MacroPad::paced(
            by_id(MODEL_ID).unwrap(),
            Box::new(recorder.clone()),
            None,
            Duration::ZERO,
        );
        let writes = recorder.writes();
        assert_eq!(writes.len(), 12);
        for (at, write) in writes.iter().enumerate() {
            assert_eq!(write, &vec![0x02, at as u8 + 1, 0, 0, 0]);
        }
    }

    #[test]
    fn key_reports_are_sets_diffed_into_presses_and_releases() {
        let (mut pad, recorder) = pad();
        recorder.queue_read(vec![0x01, 1, 0, 0, 0, 0, 0]); // KEY1 down
        recorder.queue_read(vec![0x01, 1, 12, 0, 0, 0, 0]); // KEY12 down
                                                            // KEY1 up: the firmware moves the last slot into the freed one.
        recorder.queue_read(vec![0x01, 12, 0, 0, 0, 0, 0]);
        recorder.queue_read(vec![0x02, 5, 0, 0, 0, 0, 0]); // not a key report
        recorder.queue_read(vec![0x01, 12, 13, 0, 0xff, 0, 0]); // out of range ignored
        recorder.queue_read(vec![0x01, 12, 5]); // short: KEY5 down
        recorder.queue_read(vec![0x01, 0, 0, 0, 0, 0, 0]); // all up
        recorder.queue_read(vec![]); // nothing at all
        assert_eq!(
            pad.poll(0).unwrap(),
            vec![
                key(0, true),
                key(11, true),
                key(0, false),
                key(4, true),
                key(4, false),
                key(11, false),
            ]
        );
    }

    #[test]
    fn faces_set_one_key_per_report_capped_and_scaled() {
        let (mut pad, recorder) = pad();
        pad.set_face(0, Some(&face([255, 0, 128]))).unwrap();
        pad.set_face(11, Some(&face([10, 20, 30]))).unwrap();
        pad.set_face(12, Some(&face([9, 9, 9]))).unwrap(); // no such key
        pad.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![vec![0x02, 1, 128, 0, 64], vec![0x02, 12, 5, 10, 15]]
        );
        // An unbound key goes dark through report 2, never report 3; an
        // unchanged one is not resent.
        pad.set_face(0, None).unwrap();
        pad.set_face(11, Some(&face([10, 20, 30]))).unwrap();
        pad.flush().unwrap();
        assert_eq!(recorder.writes()[2], vec![0x02, 1, 0, 0, 0]);
        assert_eq!(recorder.writes().len(), 3);
        assert!(recorder.writes().iter().all(|write| write[0] == 0x02));
    }

    #[test]
    fn brightness_scales_and_resends_every_key() {
        let (mut pad, recorder) = pad();
        pad.set_face(2, Some(&face([200, 100, 255]))).unwrap();
        pad.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        pad.set_brightness(50).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 12);
        assert_eq!(writes[2], vec![0x02, 3, 50, 25, 64]);
        assert_eq!(writes[0], vec![0x02, 1, 0, 0, 0]);
        pad.set_brightness(50).unwrap();
        assert_eq!(recorder.writes().len(), 12);
    }

    #[test]
    fn colours_go_to_the_lighting_collection_when_it_is_apart() {
        let keys = Recorder::default();
        let lights = Recorder::default();
        let mut pad = MacroPad::paced(
            by_id(MODEL_ID).unwrap(),
            Box::new(keys.clone()),
            Some(Box::new(lights.clone())),
            Duration::ZERO,
        );
        pad.set_face(4, Some(&face([255, 255, 255]))).unwrap();
        pad.flush().unwrap();
        assert!(keys.writes().is_empty());
        assert_eq!(lights.writes().len(), 13);
        assert_eq!(lights.writes()[12], vec![0x02, 5, 128, 128, 128]);
        keys.queue_read(vec![0x01, 7, 0, 0, 0, 0, 0]);
        assert_eq!(pad.poll(0).unwrap(), vec![key(6, true)]);
    }

    #[test]
    fn heartbeats_resend_every_key_now_and_then() {
        let (mut pad, recorder) = pad();
        for _ in 0..REFRESH_EVERY - 1 {
            pad.heartbeat().unwrap();
        }
        assert!(recorder.writes().is_empty());
        pad.heartbeat().unwrap();
        assert_eq!(recorder.writes().len(), 12);
    }

    #[test]
    fn letting_go_blanks_every_key() {
        let (mut pad, recorder) = pad();
        pad.set_face(3, Some(&face([1, 2, 3]))).unwrap();
        pad.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        drop(pad);
        let writes = recorder.writes();
        assert_eq!(writes.len(), 12);
        assert!(writes.iter().all(|write| write[2..] == [0, 0, 0]));
    }
}
