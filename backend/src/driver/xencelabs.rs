//! Xencelabs Quick Keys: eight keys whose labels show as text on one monochrome
//! OLED, an unlabelled button, and a wheel with a push button and an RGB ring.
//! The wired unit (28bd:5202) is one deck; the wireless dongle (28bd:5203) can
//! carry several, and Ryu binds to the first one it reports.
//!
//! Every report uses report id 0x02. A command is one 32-byte output report,
//! the id first; for a deck behind the dongle, bytes 10 to 15 carry its six-byte
//! id. Input `[02 f0 keys(u16 LE) .. .. .. wheel]` is either one wheel step
//! (byte 7 non-zero: bit 0 clockwise, bit 1 anticlockwise) or the held keys
//! (bits 0-7 the labelled keys, bit 8 the unlabelled button, bit 9 the wheel
//! push). The dongle announces its decks with `02 f8 state .. id@10..16` and
//! tags their other reports with the id at bytes 12 to 18.
//!
//! There are no key images: a key's label is up to eight UTF-16 units of text.
//! Labels, the ring colour and the brightness are kept, and sent again whenever
//! a deck (re)appears behind the dongle.

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const REPORT_ID: u8 = 0x02;
const COMMAND_LEN: usize = 32;
const READ_BUFFER: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 32;
/// Keys with a label on the display.
const LABELLED_KEYS: u8 = 8;
/// The unlabelled button, the key after the labelled ones.
const BUTTON_KEY: u8 = 8;
const BUTTON_BIT: u16 = 1 << 8;
const WHEEL_PUSH_BIT: u16 = 1 << 9;
/// A label holds at most eight UTF-16 code units.
const LABEL_UNITS: usize = 8;
const SURFACE_ID_LEN: usize = 6;
/// Where a command carries the dongle deck's id.
const COMMAND_ID_AT: usize = 10;
/// Where a status report carries a deck's id, and where other reports do.
const STATUS_ID_AT: usize = 10;
const TAGGED_ID_AT: usize = 12;
/// Wheel steps per report: 1 is fastest, 5 slowest.
const WHEEL_SPEED_NORMAL: u8 = 3;
/// Display orientation: 1 = 0 degrees.
const ROTATE_0: u8 = 1;
const STATE_CONNECTED: u8 = 2;
const STATE_ALREADY_CONNECTED: u8 = 3;
const STATE_LOST: u8 = 4;

type SurfaceId = [u8; SURFACE_ID_LEN];

pub struct QuickKeys {
    transport: Box<dyn Transport>,
    /// Behind the wireless dongle, which needs a deck to bind to.
    wireless: bool,
    started: bool,
    /// The dongle deck this driver drives; always `None` when wired.
    surface: Option<SurfaceId>,
    held: u16,
    labels: [String; LABELLED_KEYS as usize],
    /// What the display shows; `None` when unknown (not sent yet, or the deck
    /// came back).
    shown: [Option<String>; LABELLED_KEYS as usize],
    ring: [u8; 3],
    shown_ring: Option<[u8; 3]>,
    brightness: Option<u8>,
}

impl QuickKeys {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            wireless: model.id == "xencelabs-quick-keys-wireless",
            started: false,
            surface: None,
            held: 0,
            labels: Default::default(),
            shown: Default::default(),
            ring: [0; 3],
            shown_ring: None,
            brightness: None,
        }
    }

    /// Whether commands reach a deck: always when wired, once one is bound
    /// behind the dongle.
    fn reachable(&self) -> bool {
        !self.wireless || self.surface.is_some()
    }

    /// A 32-byte command, addressed to the bound dongle deck if there is one.
    fn command(&self, head: &[u8]) -> [u8; COMMAND_LEN] {
        let mut buf = [0u8; COMMAND_LEN];
        buf[0] = REPORT_ID;
        for (slot, byte) in buf[1..].iter_mut().zip(head) {
            *slot = *byte;
        }
        if let Some(id) = self.surface {
            buf[COMMAND_ID_AT..COMMAND_ID_AT + SURFACE_ID_LEN].copy_from_slice(&id);
        }
        buf
    }

    fn send(&mut self, head: &[u8]) -> DriverResult<()> {
        let buf = self.command(head);
        self.transport.write(&buf)
    }

    /// Wired: subscribe and set up at once. Dongle: ask which decks it has.
    fn ensure_started(&mut self) -> DriverResult<()> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        if self.wireless {
            self.send(&[0xb8, 0x01])
        } else {
            self.set_up()
        }
    }

    /// Subscribes to key and battery reports, sets the wheel speed, the
    /// orientation and no sleep timeout, and forgets what the deck shows so the
    /// next flush sends everything.
    fn set_up(&mut self) -> DriverResult<()> {
        self.send(&[0xb0, 0x04])?;
        self.send(&[0xb4, 0x10])?;
        self.send(&[0xb4, 0x04, 0x01, 0x01, WHEEL_SPEED_NORMAL])?;
        self.send(&[0xb1, ROTATE_0])?;
        self.send(&[0xb4, 0x08, 0x01, 0x00])?;
        self.shown = Default::default();
        self.shown_ring = None;
        if let Some(level) = self.brightness {
            self.send_brightness(level)?;
        }
        Ok(())
    }

    fn send_brightness(&mut self, level: u8) -> DriverResult<()> {
        self.send(&[0xb1, 0x0a, 0x01, level])
    }

    fn send_label(&mut self, key: u8) -> DriverResult<()> {
        let units = label_units(&self.labels[usize::from(key)]);
        let mut buf = self.command(&[0xb1, 0x00, key + 1, 0x00, (units.len() * 2) as u8]);
        for (index, unit) in units.iter().enumerate() {
            let at = 16 + index * 2;
            buf[at..at + 2].copy_from_slice(&unit.to_le_bytes());
        }
        self.transport.write(&buf)
    }

    fn send_staged(&mut self) -> DriverResult<()> {
        if !self.reachable() {
            return Ok(());
        }
        for key in 0..LABELLED_KEYS {
            let index = usize::from(key);
            if self.shown[index].as_ref() != Some(&self.labels[index]) {
                self.send_label(key)?;
                self.shown[index] = Some(self.labels[index].clone());
            }
        }
        if self.shown_ring != Some(self.ring) {
            let [red, green, blue] = self.ring;
            self.send(&[0xb4, 0x01, 0x01, 0x00, 0x00, red, green, blue])?;
            self.shown_ring = Some(self.ring);
        }
        Ok(())
    }

    /// Releases everything held, for a deck that went away mid-press.
    fn release_all(&mut self, events: &mut Vec<Input>) {
        self.key_changes(0, events);
    }

    fn key_changes(&mut self, keys: u16, events: &mut Vec<Input>) {
        let changed = keys ^ self.held;
        self.held = keys;
        for bit in 0..10u8 {
            let mask = 1u16 << bit;
            if changed & mask == 0 {
                continue;
            }
            let pressed = keys & mask != 0;
            if mask == WHEEL_PUSH_BIT {
                events.push(Input::DialPress { index: 0, pressed });
            } else if mask == BUTTON_BIT {
                events.push(Input::Key {
                    index: BUTTON_KEY,
                    pressed,
                });
            } else {
                events.push(Input::Key {
                    index: bit,
                    pressed,
                });
            }
        }
    }

    /// A dongle status report: a deck appeared, was already there, or went away.
    fn status(&mut self, report: &[u8], events: &mut Vec<Input>) -> DriverResult<()> {
        let (Some(&state), Some(id)) = (report.get(2), surface_id(report, STATUS_ID_AT)) else {
            return Ok(());
        };
        // A new connection is always set up; one the dongle already had only
        // when nothing is bound yet. Other decks are left alone.
        match state {
            STATE_CONNECTED if self.surface.is_none_or(|bound| bound == id) => self.bind(id),
            STATE_ALREADY_CONNECTED if self.surface.is_none() => self.bind(id),
            STATE_LOST if self.surface == Some(id) => {
                self.surface = None;
                self.release_all(events);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn bind(&mut self, id: SurfaceId) -> DriverResult<()> {
        self.surface = Some(id);
        self.set_up()?;
        self.send_staged()
    }

    fn parse(&mut self, report: &[u8], events: &mut Vec<Input>) -> DriverResult<()> {
        if report.first() != Some(&REPORT_ID) {
            return Ok(());
        }
        let kind = report.get(1).copied();
        if self.wireless {
            if kind == Some(0xf8) {
                return self.status(report, events);
            }
            let Some(bound) = self.surface else {
                return Ok(());
            };
            // The id's place in these reports is unconfirmed, so only an id
            // that is there and names another deck drops the report.
            if surface_id(report, TAGGED_ID_AT)
                .is_some_and(|id| id != [0; SURFACE_ID_LEN] && id != bound)
            {
                return Ok(());
            }
        }
        if kind != Some(0xf0) {
            // Battery reports (0xf2) and anything else carry no input.
            return Ok(());
        }
        let wheel = report.get(7).copied().unwrap_or(0);
        if wheel != 0 {
            // A wheel step: its key bits are not valid.
            let delta = if wheel & 0x01 != 0 {
                1
            } else if wheel & 0x02 != 0 {
                -1
            } else {
                0
            };
            if delta != 0 {
                events.push(Input::DialTurn { index: 0, delta });
            }
            return Ok(());
        }
        let (Some(&low), Some(&high)) = (report.get(2), report.get(3)) else {
            return Ok(());
        };
        self.key_changes(u16::from_le_bytes([low, high]) & 0x03ff, events);
        Ok(())
    }
}

fn surface_id(report: &[u8], at: usize) -> Option<SurfaceId> {
    report
        .get(at..at + SURFACE_ID_LEN)
        .and_then(|slice| slice.try_into().ok())
}

/// A label as at most eight UTF-16 units, never splitting a surrogate pair.
/// Control characters become spaces.
fn label_units(label: &str) -> Vec<u16> {
    let mut units = Vec::with_capacity(LABEL_UNITS);
    for c in label.trim().chars() {
        let c = if c.is_control() { ' ' } else { c };
        let mut pair = [0u16; 2];
        let encoded = c.encode_utf16(&mut pair);
        if units.len() + encoded.len() > LABEL_UNITS {
            break;
        }
        units.extend_from_slice(encoded);
    }
    units
}

/// Ryu's 0-100 % as the display's four levels (0 off, 3 full).
fn brightness_level(percent: u8) -> u8 {
    let level = (u16::from(percent.min(100)) * 3 + 50) / 100;
    level as u8
}

impl Driver for QuickKeys {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_started()?;
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            let report = buf[..read].to_vec();
            self.parse(&report, &mut events)?;
        }
        Ok(events)
    }

    /// The label of keys 1 to 8 shows on the display; colours cannot.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(slot) = self.labels.get_mut(usize::from(key)) {
            *slot = face.map(|face| face.label.clone()).unwrap_or_default();
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_started()?;
        self.send_staged()
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.ensure_started()?;
        let level = brightness_level(percent);
        self.brightness = Some(level);
        if self.reachable() {
            self.send_brightness(level)?;
        }
        Ok(())
    }

    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if dial == 0 {
            self.ring = color.unwrap_or([0; 3]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn driver(model: &str) -> (QuickKeys, Recorder) {
        let recorder = Recorder::default();
        (
            QuickKeys::new(by_id(model).unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    fn command(head: &[u8], id: Option<SurfaceId>) -> Vec<u8> {
        let mut buf = vec![0u8; COMMAND_LEN];
        buf[0] = REPORT_ID;
        buf[1..=head.len()].copy_from_slice(head);
        if let Some(id) = id {
            buf[10..16].copy_from_slice(&id);
        }
        buf
    }

    fn set_up(id: Option<SurfaceId>) -> Vec<Vec<u8>> {
        vec![
            command(&[0xb0, 0x04], id),
            command(&[0xb4, 0x10], id),
            command(&[0xb4, 0x04, 0x01, 0x01, 0x03], id),
            command(&[0xb1, 0x01], id),
            command(&[0xb4, 0x08, 0x01, 0x00], id),
        ]
    }

    fn label(key: u8, text: &str, id: Option<SurfaceId>) -> Vec<u8> {
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut buf = command(&[0xb1, 0x00, key + 1, 0x00, (units.len() * 2) as u8], id);
        for (index, unit) in units.iter().enumerate() {
            buf[16 + index * 2..18 + index * 2].copy_from_slice(&unit.to_le_bytes());
        }
        buf
    }

    fn face(label: &str) -> Face {
        Face {
            label: label.to_owned(),
            color: [1, 2, 3],
            image: None,
        }
    }

    fn key_report(bits: u16) -> Vec<u8> {
        let [low, high] = bits.to_le_bytes();
        let mut report = vec![0u8; 32];
        report[..4].copy_from_slice(&[0x02, 0xf0, low, high]);
        report
    }

    #[test]
    fn wired_sets_up_then_sends_labels_and_the_ring() {
        let (mut deck, recorder) = driver("xencelabs-quick-keys");
        deck.set_face(0, Some(&face("Review"))).unwrap();
        deck.set_face(7, Some(&face("Ship it now please"))).unwrap();
        deck.set_face(8, Some(&face("no label"))).unwrap();
        deck.set_dial_light(0, Some([0x10, 0x20, 0x30])).unwrap();
        deck.flush().unwrap();
        let mut expected = set_up(None);
        expected.push(label(0, "Review", None));
        for key in 1..7 {
            expected.push(label(key, "", None));
        }
        expected.push(label(7, "Ship it ", None));
        expected.push(command(
            &[0xb4, 0x01, 0x01, 0x00, 0x00, 0x10, 0x20, 0x30],
            None,
        ));
        assert_eq!(recorder.writes(), expected);
        // Only what changed goes out again.
        deck.set_face(0, None).unwrap();
        deck.set_dial_light(0, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[expected.len()..],
            [
                label(0, "", None),
                command(&[0xb4, 0x01, 0x01, 0x00, 0x00, 0, 0, 0], None),
            ]
        );
    }

    #[test]
    fn label_bytes_are_utf16le_with_their_length() {
        let (mut deck, recorder) = driver("xencelabs-quick-keys");
        deck.set_face(2, Some(&face("Ab"))).unwrap();
        deck.flush().unwrap();
        let sent = &recorder.writes()[5 + 2];
        assert_eq!(&sent[..6], &[0x02, 0xb1, 0x00, 0x03, 0x00, 0x04]);
        assert_eq!(&sent[16..20], &[b'A', 0, b'b', 0]);
        assert!(sent[20..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn labels_never_split_a_surrogate_pair() {
        assert_eq!(label_units("1234567\u{1f600}").len(), 7);
        assert_eq!(label_units("123456\u{1f600}").len(), 8);
        assert_eq!(label_units("  a\nb ").len(), 3);
    }

    #[test]
    fn brightness_takes_four_levels() {
        assert_eq!(
            [0, 16, 17, 50, 83, 84, 100, 255].map(brightness_level),
            [0, 0, 1, 2, 2, 3, 3, 3]
        );
        let (mut deck, recorder) = driver("xencelabs-quick-keys");
        deck.set_brightness(60).unwrap();
        assert_eq!(
            recorder.writes().last().unwrap(),
            &command(&[0xb1, 0x0a, 0x01, 0x02], None)
        );
    }

    #[test]
    fn keys_the_button_and_the_wheel() {
        let (mut deck, recorder) = driver("xencelabs-quick-keys");
        recorder.queue_read(key_report(0b0000_0001_0001_0000));
        // A wheel report carries no valid key bits: nothing is released.
        let mut wheel = key_report(0);
        wheel[7] = 0x01;
        recorder.queue_read(wheel.clone());
        wheel[7] = 0x02;
        recorder.queue_read(wheel);
        recorder.queue_read(key_report(0b0000_0010_0000_0000));
        // Not report 0x02, a battery report, and a truncated report: ignored.
        recorder.queue_read(vec![0x01, 0xf0, 0xff, 0xff]);
        recorder.queue_read(vec![0x02, 0xf2, 0x01, 0x50]);
        recorder.queue_read(vec![0x02, 0xf0, 0xff]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 4,
                    pressed: true
                },
                Input::Key {
                    index: 8,
                    pressed: true
                },
                Input::DialTurn { index: 0, delta: 1 },
                Input::DialTurn {
                    index: 0,
                    delta: -1
                },
                Input::Key {
                    index: 4,
                    pressed: false
                },
                Input::Key {
                    index: 8,
                    pressed: false
                },
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
            ]
        );
    }

    fn status(state: u8, id: SurfaceId) -> Vec<u8> {
        let mut report = vec![0u8; 32];
        report[..3].copy_from_slice(&[0x02, 0xf8, state]);
        report[10..16].copy_from_slice(&id);
        report
    }

    #[test]
    fn the_dongle_binds_its_first_deck_and_addresses_every_command() {
        const DECK: SurfaceId = [1, 2, 3, 4, 5, 6];
        const OTHER: SurfaceId = [9, 9, 9, 9, 9, 9];
        let (mut deck, recorder) = driver("xencelabs-quick-keys-wireless");
        deck.set_face(1, Some(&face("Go"))).unwrap();
        deck.flush().unwrap();
        // Nothing is bound yet: only the discovery request goes out.
        assert_eq!(recorder.writes(), vec![command(&[0xb8, 0x01], None)]);

        recorder.queue_read(status(STATE_ALREADY_CONNECTED, DECK));
        recorder.queue_read(status(STATE_ALREADY_CONNECTED, OTHER));
        let mut press = key_report(0b10);
        press[12..18].copy_from_slice(&DECK);
        recorder.queue_read(press);
        let mut foreign = key_report(0b100);
        foreign[12..18].copy_from_slice(&OTHER);
        recorder.queue_read(foreign);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 1,
                pressed: true
            }]
        );
        let mut expected = vec![command(&[0xb8, 0x01], None)];
        expected.extend(set_up(Some(DECK)));
        expected.push(label(0, "", Some(DECK)));
        expected.push(label(1, "Go", Some(DECK)));
        for key in 2..8 {
            expected.push(label(key, "", Some(DECK)));
        }
        expected.push(command(&[0xb4, 0x01, 0x01, 0, 0, 0, 0, 0], Some(DECK)));
        assert_eq!(recorder.writes(), expected);

        // Lost mid-press: the key is released and sends stop.
        recorder.queue_read(status(STATE_LOST, DECK));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 1,
                pressed: false
            }]
        );
        deck.set_face(1, Some(&face("Again"))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), expected.len());

        // Back again: set up and every label resent.
        recorder.queue_read(status(STATE_CONNECTED, DECK));
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), expected.len() + 5 + 8 + 1);
        assert_eq!(
            writes[expected.len() + 5 + 1],
            label(1, "Again", Some(DECK))
        );
    }
}
