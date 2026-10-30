//! Haute42 B16 leverless controller, flashed with SoundMinded's SurfaceCore
//! firmware.
//!
//! SurfaceCore turns the B16's RP2040 into a TinyUSB composite device (CDC,
//! MIDI and one vendor HID interface) on cafe:2043. Ryu only opens the HID
//! interface (usage page 0xFF00): 64-byte reports both ways with no report id,
//! so a write is a 0x00 placeholder followed by the 64-byte payload. The stock
//! Haute42 (GP2040-CE) firmware enumerates as something else and is not driven.
//!
//! In: `01 id state` is button `id` (0 to 20) going down (`state` 1) or up
//! (`state` 0), one report per edge. Ids 0 to 15 are B01 to B16, 16 to 20
//! Function1 to Function5; Ryu's key is the id. `05 ...` answers the device-info
//! request, `03 01 00` the hello. Out: `02 id R G B` sets the light under B01 to
//! B16 (ids 0 to 15) and takes effect at once; `04` asks for the device info.
//! The five function buttons have no light, there is no command for every key
//! at once, and no brightness, so Ryu scales the colours itself.
//!
//! Each write takes one interrupt-OUT interval (10 ms), so only lights whose
//! colour on the device changes are sent. The firmware drops an input report
//! while the previous one is still waiting for the host, so a quick chord or
//! release can be lost; there is no state query to recover it from.
//!
//! Facts per the MIT-licensed SurfaceCore firmware and SurfaceCore Companion
//! module (soundminded/SurfaceCore); no code was copied.

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

/// The catalog id of the B16 on SurfaceCore.
pub const MODEL_ID: &str = "haute42-b16";
/// The HID payload length, both ways (`CFG_TUD_HID_EP_BUFSIZE`).
const PAYLOAD_LEN: usize = 64;
/// A write: the report-id placeholder hidapi expects, then the payload.
const WRITE_LEN: usize = PAYLOAD_LEN + 1;
/// Buttons the firmware reports: B01 to B16, then Function1 to Function5.
const BUTTONS: u8 = 21;
/// B01 to B16 each have an RGB light; the function buttons do not.
const LIGHTS: usize = 16;
const MAX_REPORTS_PER_POLL: usize = 32;
/// Heartbeats on which the device-info request is sent again before warning.
const PROBE_ATTEMPTS: u8 = 3;

const BUTTON_EVENT: u8 = 0x01;
const LED_UPDATE: u8 = 0x02;
const DEVICE_INFO_REQUEST: u8 = 0x04;
const DEVICE_INFO_RESPONSE: u8 = 0x05;
/// Board type 1 in the device-info answer is the Haute42 B16.
const BOARD_HAUTE42_B16: u8 = 0x01;

/// The B16's key names: the firmware's B01 to B16, then the function buttons.
pub const KEY_NAMES: [&str; BUTTONS as usize] = [
    "B01", "B02", "B03", "B04", "B05", "B06", "B07", "B08", "B09", "B10", "B11", "B12", "B13",
    "B14", "B15", "B16", "Fn1", "Fn2", "Fn3", "Fn4", "Fn5",
];
/// The keys with a light: B01 to B16.
pub const LIT_KEYS: [u8; LIGHTS] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// Each key's (row, column) on the 4 x 15 grid, after the SurfaceCore Companion
/// module's layout. It only approximates where the buttons sit on the B16.
pub const KEY_PLACES: [(u8, u8); BUTTONS as usize] = [
    (0, 0),
    (0, 4),
    (0, 5),
    (0, 6),
    (0, 7),
    (0, 8),
    (1, 0),
    (1, 1),
    (1, 2),
    (1, 4),
    (1, 5),
    (1, 6),
    (1, 7),
    (2, 6),
    (3, 5),
    (3, 6),
    (0, 10),
    (0, 11),
    (0, 12),
    (0, 13),
    (0, 14),
];

const BLACK: [u8; 3] = [0, 0, 0];

/// One write: the 0x00 placeholder, the command bytes, then zeros.
fn report(command: &[u8]) -> [u8; WRITE_LEN] {
    let mut out = [0u8; WRITE_LEN];
    for (slot, byte) in out[1..].iter_mut().zip(command) {
        *slot = *byte;
    }
    out
}

/// `colour * percent / 100`, per channel.
fn dimmed(color: [u8; 3], percent: u8) -> [u8; 3] {
    color.map(|channel| (u16::from(channel) * u16::from(percent) / 100) as u8)
}

/// A button event payload -> the key it names and whether it went down.
fn button_event(payload: &[u8]) -> Option<Input> {
    if payload.first() != Some(&BUTTON_EVENT) {
        return None;
    }
    let (&id, &state) = (payload.get(1)?, payload.get(2)?);
    if id >= BUTTONS || state > 1 {
        return None;
    }
    Some(Input::Key {
        index: id,
        pressed: state == 1,
    })
}

/// What the device-info answer says about the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Board {
    /// Not answered yet.
    Unknown,
    /// 21 buttons, 16 LEDs, board type 1: a B16 on SurfaceCore.
    Haute42B16,
    /// Something else on the same ids (0xCAFE is TinyUSB's shared vendor id).
    Other,
}

/// A device-info answer (`05 proto_major proto_minor fw_major fw_minor fw_patch
/// buttons leds board`) -> which board it names.
fn board(payload: &[u8]) -> Option<Board> {
    if payload.first() != Some(&DEVICE_INFO_RESPONSE) {
        return None;
    }
    let fields = (payload.get(6)?, payload.get(7)?, payload.get(8)?);
    Some(
        if fields == (&BUTTONS, &(LIGHTS as u8), &BOARD_HAUTE42_B16) {
            Board::Haute42B16
        } else {
            Board::Other
        },
    )
}

pub struct SurfaceCore {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    /// The colour each light should show, at full brightness.
    colors: [[u8; 3]; LIGHTS],
    /// The colour each light was last sent, dimmed; `None` when unknown (a
    /// write failed).
    shown: [Option<[u8; 3]>; LIGHTS],
    brightness: u8,
    board: Board,
    probes_sent: u8,
}

impl SurfaceCore {
    /// Asks which board this is and blanks the lights: the firmware boots with
    /// them dark, but a previous host may have lit them.
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let mut deck = Self {
            model,
            transport,
            colors: [BLACK; LIGHTS],
            shown: [None; LIGHTS],
            brightness: 100,
            board: Board::Unknown,
            probes_sent: 0,
        };
        deck.probe();
        if let Err(error) = deck.flush() {
            tracing::debug!(device = model.id, %error, "ryu-decks: SurfaceCore setup write failed");
        }
        deck
    }

    fn probe(&mut self) {
        self.probes_sent += 1;
        if let Err(error) = self.transport.write(&report(&[DEVICE_INFO_REQUEST])) {
            tracing::debug!(device = self.model.id, %error, "ryu-decks: SurfaceCore device-info request failed");
        }
    }

    fn receive(&mut self, payload: &[u8]) -> Option<Input> {
        if let Some(board) = board(payload) {
            if board == Board::Other && self.board != Board::Other {
                tracing::warn!(
                    device = self.model.id,
                    "ryu-decks: the device on cafe:2043 is not a Haute42 B16 on SurfaceCore (its device info names another board)"
                );
            }
            self.board = board;
            return None;
        }
        button_event(payload)
    }
}

impl Driver for SurfaceCore {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; PAYLOAD_LEN];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            events.extend(self.receive(&buf[..read]));
        }
        Ok(events)
    }

    /// Stages a light; keys 16 to 20 (the function buttons) have none.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// One `02` per light whose dimmed colour differs from what it shows.
    fn flush(&mut self) -> DriverResult<()> {
        for id in 0..LIGHTS {
            let color = dimmed(self.colors[id], self.brightness);
            if self.shown[id] == Some(color) {
                continue;
            }
            self.shown[id] = None;
            let [r, g, b] = color;
            self.transport
                .write(&report(&[LED_UPDATE, id as u8, r, g, b]))?;
            self.shown[id] = Some(color);
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        let percent = percent.min(100);
        if percent == self.brightness {
            return Ok(());
        }
        self.brightness = percent;
        self.flush()
    }

    /// Asks again while the device-info answer is missing (the firmware drops
    /// it while an earlier report is still pending), then warns once.
    fn heartbeat(&mut self) -> DriverResult<()> {
        if self.board != Board::Unknown || self.probes_sent > PROBE_ATTEMPTS {
            return Ok(());
        }
        if self.probes_sent < PROBE_ATTEMPTS {
            self.probe();
        } else {
            self.probes_sent += 1;
            tracing::warn!(
                device = self.model.id,
                "ryu-decks: the device on cafe:2043 did not answer the SurfaceCore device-info request; it needs SurfaceCore firmware"
            );
        }
        Ok(())
    }
}

impl Drop for SurfaceCore {
    /// Leaves the lights dark when Ryu lets go: they keep their colour while
    /// the controller stays powered.
    fn drop(&mut self) {
        for id in 0..LIGHTS {
            if self.shown[id] != Some(BLACK) {
                let _ = self
                    .transport
                    .write(&report(&[LED_UPDATE, id as u8, 0, 0, 0]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::by_id;

    fn open() -> (SurfaceCore, Recorder) {
        let recorder = Recorder::default();
        let deck = SurfaceCore::new(by_id(MODEL_ID).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    fn deck() -> (SurfaceCore, Recorder) {
        let (deck, recorder) = open();
        recorder.0.lock().unwrap().writes.clear();
        (deck, recorder)
    }

    fn face(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    fn payload(bytes: &[u8]) -> Vec<u8> {
        let mut out = bytes.to_vec();
        out.resize(PAYLOAD_LEN, 0);
        out
    }

    const INFO: [u8; 9] = [0x05, 1, 0, 0, 3, 0, 0x15, 0x10, 0x01];

    #[test]
    fn opening_asks_for_the_device_info_and_blanks_every_light() {
        let (_deck, recorder) = open();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 17);
        assert!(writes.iter().all(|w| w.len() == WRITE_LEN && w[0] == 0));
        assert_eq!(writes[0], report(&[0x04]).to_vec());
        assert!(writes[0][2..].iter().all(|b| *b == 0));
        for id in 0..16u8 {
            assert_eq!(
                &writes[usize::from(id) + 1][..6],
                &[0, 0x02, id, 0, 0, 0],
                "light {id}"
            );
        }
    }

    #[test]
    fn button_events_map_ids_to_keys_and_skip_anything_else() {
        let (mut deck, recorder) = deck();
        recorder.queue_read(payload(&[0x01, 0, 1])); // B01 down
        recorder.queue_read(payload(&[0x01, 15, 0])); // B16 up
        recorder.queue_read(payload(&[0x01, 20, 1])); // Function5 down
        recorder.queue_read(payload(&[0x03, 1, 0])); // hello answer
        recorder.queue_read(payload(&INFO)); // device info
        recorder.queue_read(payload(&[0x01, 21, 1])); // no button 21
        recorder.queue_read(payload(&[0x01, 3, 2])); // no state 2
        recorder.queue_read(vec![0x01, 3]); // short
        recorder.queue_read(payload(&[0x09, 3, 1])); // unknown type
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 0,
                    pressed: true
                },
                Input::Key {
                    index: 15,
                    pressed: false
                },
                Input::Key {
                    index: 20,
                    pressed: true
                },
            ]
        );
        assert_eq!(deck.board, Board::Haute42B16);
    }

    #[test]
    fn faces_light_only_the_sixteen_lit_keys_and_only_when_changed() {
        let (mut deck, recorder) = deck();
        deck.set_face(0, Some(&face([0x2d, 0x7f, 0xf9]))).unwrap();
        deck.set_face(15, Some(&face([1, 2, 3]))).unwrap();
        deck.set_face(16, Some(&face([9, 9, 9]))).unwrap(); // Function1: no light
        deck.set_face(20, Some(&face([9, 9, 9]))).unwrap();
        deck.set_face(21, Some(&face([9, 9, 9]))).unwrap(); // no such key
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0], report(&[0x02, 0, 0x2d, 0x7f, 0xf9]).to_vec());
        assert_eq!(writes[0].len(), 65);
        assert_eq!(&writes[1][..6], &[0, 0x02, 15, 1, 2, 3]);
        // An unbound key goes dark; an unchanged one is not resent.
        deck.set_face(0, None).unwrap();
        deck.set_face(15, Some(&face([1, 2, 3]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 3);
        assert_eq!(&writes[2][..6], &[0, 0x02, 0, 0, 0, 0]);
    }

    #[test]
    fn brightness_scales_on_the_host_and_resends_only_what_changes() {
        let (mut deck, recorder) = deck();
        deck.set_face(0, Some(&face([200, 100, 255]))).unwrap();
        deck.set_face(1, Some(&face([1, 1, 1]))).unwrap();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        deck.set_brightness(50).unwrap();
        // Key 1 dims to black; the dark keys stay as they are.
        assert_eq!(
            recorder.writes(),
            vec![
                report(&[0x02, 0, 100, 50, 127]).to_vec(),
                report(&[0x02, 1, 0, 0, 0]).to_vec(),
            ]
        );
        deck.set_brightness(0).unwrap();
        assert_eq!(recorder.writes()[2], report(&[0x02, 0, 0, 0, 0]).to_vec());
        // The same level again sends nothing.
        deck.set_brightness(0).unwrap();
        assert_eq!(recorder.writes().len(), 3);
        deck.set_brightness(100).unwrap();
        assert_eq!(recorder.writes().len(), 5);
    }

    #[test]
    fn a_missing_device_info_answer_is_asked_for_again_then_given_up() {
        let (mut deck, recorder) = deck();
        for _ in 0..6 {
            deck.heartbeat().unwrap();
        }
        // Two more requests (three in all), then nothing.
        assert_eq!(
            recorder.writes(),
            vec![report(&[0x04]).to_vec(), report(&[0x04]).to_vec()]
        );
    }

    #[test]
    fn a_stale_button_report_before_the_answer_still_counts() {
        let (mut deck, recorder) = deck();
        recorder.queue_read(payload(&[0x01, 7, 1]));
        recorder.queue_read(payload(&INFO));
        assert_eq!(deck.poll(0).unwrap().len(), 1);
        deck.heartbeat().unwrap();
        assert!(recorder.writes().is_empty(), "answered: not asked again");
    }

    #[test]
    fn another_board_on_the_same_ids_is_told_apart() {
        assert_eq!(board(&payload(&INFO)), Some(Board::Haute42B16));
        let mut other = INFO;
        other[8] = 0x02;
        assert_eq!(board(&payload(&other)), Some(Board::Other));
        other = INFO;
        other[6] = 24;
        assert_eq!(board(&payload(&other)), Some(Board::Other));
        assert_eq!(board(&[0x05, 1, 0]), None, "short");
        assert_eq!(board(&payload(&[0x01, 0, 1])), None);
        let (mut deck, recorder) = deck();
        recorder.queue_read(payload(&other));
        deck.poll(0).unwrap();
        assert_eq!(deck.board, Board::Other);
        deck.heartbeat().unwrap();
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn letting_go_blanks_only_the_lit_lights() {
        let (mut deck, recorder) = deck();
        deck.set_face(4, Some(&face([5, 6, 7]))).unwrap();
        deck.flush().unwrap();
        recorder.0.lock().unwrap().writes.clear();
        drop(deck);
        assert_eq!(
            recorder.writes(),
            vec![report(&[0x02, 4, 0, 0, 0]).to_vec()]
        );
    }

    #[test]
    fn the_layout_tables_cover_every_key() {
        assert_eq!(KEY_NAMES.len(), usize::from(BUTTONS));
        assert_eq!(KEY_PLACES.len(), usize::from(BUTTONS));
        let cells: Vec<u16> = KEY_PLACES
            .iter()
            .map(|(row, col)| u16::from(*row) * 15 + u16::from(*col))
            .collect();
        assert_eq!(
            cells,
            vec![0, 4, 5, 6, 7, 8, 15, 16, 17, 19, 20, 21, 22, 36, 50, 51, 10, 11, 12, 13, 14]
        );
    }
}
