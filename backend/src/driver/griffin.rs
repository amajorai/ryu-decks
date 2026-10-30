//! Griffin PowerMate (USB): one aluminium knob that turns and pushes, with a
//! blue light in its base.
//!
//! The knob is dial 0: a push is a dial press, a turn a dial turn. Positive
//! steps are taken to be clockwise (not confirmed on hardware); the knob counts
//! about 94 steps a turn. The base light is the deck light, one colour, whose
//! brightest channel (scaled by the brightness setting) sets the light's level
//! from 0 to 255.
//!
//! On macOS and Windows it is a plain HID device with no report ids. An input
//! report is 3 to 6 bytes: byte 0 bit 0 is the push, byte 1 the turn since the
//! last report as a signed byte (up to 7 steps either way; the rest echo the
//! light and are not read). The light is set by Griffin's vendor control
//! request, which the device also takes as a 9-byte feature report holding the
//! request's setup bytes after report id 0: `00 41 01 cmd table op/level arg 00
//! 00`. Command 1 sets the steady level, 2 and 3 switch pulsing while the
//! computer sleeps and while it is awake; Ryu keeps the light steady while it
//! drives it and leaves the sleep pulse on.
//!
//! On Linux the kernel keeps the PowerMate from hidraw and its own driver
//! exposes it as an input event node ([`super::evdev`]): the push is `BTN_0`,
//! the turn `REL_DIAL`, and the light one `EV_MSC`/`MSC_PULSELED` event whose
//! value packs the level (bits 0-7), pulse speed (8-16), pulse table (17-18),
//! pulse while asleep (19) and pulse while awake (20). The kernel driver turns
//! that into the same control requests.

use super::evdev::{Event, BTN_0, EV_KEY, EV_MSC, EV_REL, MSC_PULSELED, RECORD_LEN, REL_DIAL};
use super::{Driver, DriverResult, Face, Input, Transport};

const READ_BUFFER: usize = 16;
const MAX_REPORTS_PER_POLL: usize = 16;
/// Room for this many event records in one evdev read.
const EVENTS_PER_READ: usize = 16;

/// The vendor request (`bmRequestType` 0x41, `bRequest` 0x01) as a feature
/// report: report id 0, then its setup bytes, `wValue` and `wIndex` little-endian.
const REQUEST_TYPE: u8 = 0x41;
const REQUEST: u8 = 0x01;
const SET_STATIC_BRIGHTNESS: u16 = 0x0001;
const SET_PULSE_ASLEEP: u16 = 0x0002;
const SET_PULSE_AWAKE: u16 = 0x0003;

/// The kernel's normal pulse speed (neither slowed nor sped up), sent in every
/// `MSC_PULSELED` value so the sleep pulse keeps its usual pace.
const NORMAL_PULSE_SPEED: i32 = 255;
const PULSE_SPEED_SHIFT: u32 = 8;
const PULSE_ASLEEP_BIT: u32 = 19;

/// One vendor command as the 9-byte feature report.
#[must_use]
pub fn command(value: u16, index: u16) -> [u8; 9] {
    let [value_low, value_high] = value.to_le_bytes();
    let [index_low, index_high] = index.to_le_bytes();
    [
        0x00,
        REQUEST_TYPE,
        REQUEST,
        value_low,
        value_high,
        index_low,
        index_high,
        0x00,
        0x00,
    ]
}

/// The `MSC_PULSELED` value for a steady light at `level`, pulsing only while
/// the computer sleeps.
#[must_use]
pub fn pulse_led_value(level: u8) -> i32 {
    i32::from(level) | NORMAL_PULSE_SPEED << PULSE_SPEED_SHIFT | 1 << PULSE_ASLEEP_BIT
}

pub struct PowerMate {
    transport: Box<dyn Transport>,
    /// Reached through a Linux input event node rather than HID.
    evdev: bool,
    pressed: bool,
    /// The deck light staged by [`Driver::set_deck_light`].
    light: Option<[u8; 3]>,
    percent: u8,
    /// The level last sent, once one has been.
    sent: Option<u8>,
    /// The pulse settings went out (HID only; evdev sends them every time).
    pulse_set: bool,
}

impl PowerMate {
    #[must_use]
    pub fn new(transport: Box<dyn Transport>) -> Self {
        let evdev = transport.is_evdev();
        Self {
            transport,
            evdev,
            pressed: false,
            light: None,
            percent: 100,
            sent: None,
            pulse_set: false,
        }
    }

    /// The light's level: its brightest channel scaled by the brightness.
    fn level(&self) -> u8 {
        let Some(color) = self.light else { return 0 };
        let brightest = u32::from(color.into_iter().max().unwrap_or(0));
        let scaled = (brightest * u32::from(self.percent) + 50) / 100;
        u8::try_from(scaled).unwrap_or(u8::MAX)
    }

    fn send_level(&mut self, level: u8) -> DriverResult<()> {
        if self.evdev {
            let event = Event {
                kind: EV_MSC,
                code: MSC_PULSELED,
                value: pulse_led_value(level),
            };
            self.transport.write(&event.encode())?;
        } else {
            if !self.pulse_set {
                // The kernel's order: asleep, then awake, then the level.
                self.transport
                    .send_feature_report(&command(SET_PULSE_ASLEEP, 1))?;
                self.transport
                    .send_feature_report(&command(SET_PULSE_AWAKE, 0))?;
                self.pulse_set = true;
            }
            self.transport
                .send_feature_report(&command(SET_STATIC_BRIGHTNESS, u16::from(level)))?;
        }
        self.sent = Some(level);
        Ok(())
    }

    fn send_if_changed(&mut self) -> DriverResult<()> {
        let level = self.level();
        if self.sent == Some(level) {
            return Ok(());
        }
        self.send_level(level)
    }

    fn press(&mut self, pressed: bool, events: &mut Vec<Input>) {
        if pressed != self.pressed {
            self.pressed = pressed;
            events.push(Input::DialPress { index: 0, pressed });
        }
    }

    fn parse_report(&mut self, report: &[u8]) -> Vec<Input> {
        let (Some(&buttons), Some(&turn)) = (report.first(), report.get(1)) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        self.press(buttons & 0x01 != 0, &mut events);
        let delta = i8::from_le_bytes([turn]);
        if delta != 0 {
            events.push(Input::DialTurn { index: 0, delta });
        }
        events
    }

    fn parse_events(&mut self, records: &[u8]) -> Vec<Input> {
        let mut events = Vec::new();
        for record in records.chunks_exact(RECORD_LEN) {
            let Some(event) = Event::decode(record) else {
                continue;
            };
            match (event.kind, event.code) {
                (EV_KEY, BTN_0) => self.press(event.value != 0, &mut events),
                (EV_REL, REL_DIAL) if event.value != 0 => {
                    let delta = i8::try_from(event.value.clamp(-128, 127)).unwrap_or(0);
                    events.push(Input::DialTurn { index: 0, delta });
                }
                _ => {}
            }
        }
        events
    }
}

impl Driver for PowerMate {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; RECORD_LEN * EVENTS_PER_READ];
        let size = if self.evdev { buf.len() } else { READ_BUFFER };
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf[..size], wait)?;
            if read == 0 {
                break;
            }
            let received = &buf[..read.min(size)];
            if self.evdev {
                events.extend(self.parse_events(received));
            } else {
                events.extend(self.parse_report(received));
            }
        }
        Ok(events)
    }

    /// No keys: the knob is a dial.
    fn set_face(&mut self, _key: u8, _face: Option<&Face>) -> DriverResult<()> {
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.send_if_changed()
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.percent = percent.min(100);
        self.send_if_changed()
    }

    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        self.light = color;
        Ok(())
    }
}

impl Drop for PowerMate {
    /// The light goes dark when Ryu lets go; the sleep pulse stays on.
    fn drop(&mut self) {
        if self.sent.is_some_and(|level| level != 0) {
            let _ = self.send_level(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::testing::Recorder;

    fn driver(evdev: bool) -> (PowerMate, Recorder) {
        let recorder = Recorder::default();
        recorder.0.lock().unwrap().evdev = evdev;
        (PowerMate::new(Box::new(recorder.clone())), recorder)
    }

    fn record(kind: u16, code: u16, value: i32) -> Vec<u8> {
        Event { kind, code, value }.encode().to_vec()
    }

    #[test]
    fn hid_reports_press_and_turn_dial_zero() {
        let (mut knob, recorder) = driver(false);
        // Pushed and turned one step back; the 6-byte form echoes the light.
        recorder.queue_read(vec![0x01, 0xff, 0x00, 0x80, 0x04, 0x00]);
        // Still pushed, turned seven steps forward (the 3-byte form).
        recorder.queue_read(vec![0x01, 0x07, 0x00]);
        // Released, no turn.
        recorder.queue_read(vec![0x00, 0x00, 0x00]);
        // Too short to read: dropped.
        recorder.queue_read(vec![0x01]);
        assert_eq!(
            knob.poll(0).unwrap(),
            vec![
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                Input::DialTurn {
                    index: 0,
                    delta: -1
                },
                Input::DialTurn { index: 0, delta: 7 },
                Input::DialPress {
                    index: 0,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn hid_light_sets_steady_mode_then_the_level_once_per_change() {
        let (mut knob, recorder) = driver(false);
        knob.set_deck_light(Some([0, 0, 200])).unwrap();
        knob.flush().unwrap();
        knob.flush().unwrap();
        knob.set_brightness(50).unwrap();
        knob.set_deck_light(None).unwrap();
        knob.flush().unwrap();
        assert_eq!(
            recorder.features(),
            vec![
                vec![0x00, 0x41, 0x01, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00],
                vec![0x00, 0x41, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00],
                vec![0x00, 0x41, 0x01, 0x01, 0x00, 200, 0x00, 0x00, 0x00],
                vec![0x00, 0x41, 0x01, 0x01, 0x00, 100, 0x00, 0x00, 0x00],
                vec![0x00, 0x41, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00],
            ]
        );
        assert!(recorder.writes().is_empty());
        drop(knob);
        assert_eq!(recorder.features().len(), 5, "already dark");
    }

    #[test]
    fn letting_go_turns_the_light_off() {
        let (mut knob, recorder) = driver(false);
        knob.set_deck_light(Some([255, 255, 255])).unwrap();
        knob.flush().unwrap();
        drop(knob);
        assert_eq!(
            recorder.features().last().unwrap(),
            &vec![0x00, 0x41, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn evdev_events_press_and_turn_dial_zero() {
        let (mut knob, recorder) = driver(true);
        let mut batch = record(EV_KEY, BTN_0, 1);
        batch.extend(record(0, 0, 0));
        batch.extend(record(EV_REL, REL_DIAL, 3));
        batch.extend(record(EV_REL, REL_DIAL, -400));
        batch.extend([0x02, 0x00]); // a torn record is skipped
        recorder.queue_read(batch);
        recorder.queue_read(record(EV_KEY, BTN_0, 0));
        assert_eq!(
            knob.poll(0).unwrap(),
            vec![
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                Input::DialTurn { index: 0, delta: 3 },
                Input::DialTurn {
                    index: 0,
                    delta: -128
                },
                Input::DialPress {
                    index: 0,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn evdev_light_is_one_pulse_led_event_with_the_whole_state() {
        let (mut knob, recorder) = driver(true);
        knob.set_deck_light(Some([10, 128, 20])).unwrap();
        knob.flush().unwrap();
        // 128 | normal speed (255 << 8) | pulse while asleep (1 << 19).
        assert_eq!(pulse_led_value(128), 0x0008_ff80);
        assert_eq!(
            recorder.writes(),
            vec![record(EV_MSC, MSC_PULSELED, 0x0008_ff80)]
        );
        assert!(recorder.features().is_empty());
        drop(knob);
        assert_eq!(
            recorder.writes().last().unwrap(),
            &record(EV_MSC, MSC_PULSELED, 0x0008_ff00)
        );
    }
}
