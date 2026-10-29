//! Blackmagic Design's USB panels: the DaVinci Resolve Speed Editor and Replay
//! Editor and the ATEM Micro Panel. Keys with lights instead of screens, a jog
//! wheel and a T-bar.
//!
//! The panel says nothing until the host has passed a challenge-response
//! handshake over one 10-byte feature report, and stops again when the
//! handshake is not repeated before the timeout it returns; [`Driver::heartbeat`]
//! repeats it at half that time.
//!
//! Every input report starts with its id. The button report is the list of
//! keycodes held right now (16-bit little-endian, ending at the first zero), so
//! presses and releases come from comparing it with the last one. The jog report
//! carries a signed 32-bit movement at byte 2, and the T-bar report its absolute
//! position (0 to 4096) at byte 1; both become dial turns. The T-bar's first
//! report only sets the baseline, so plugging the panel in does not read as a move.
//!
//! Lights are bits in whole-state output reports, least significant bit first
//! after the report id: on or off per key on the editors (the jog-mode keys have
//! a report of their own), one bit per colour channel on the ATEM, plus the
//! T-bar's 16 segments. A report goes out only when its bits changed, the
//! jog-mode report before the main one.
//!
//! Report layouts, keycodes, light bits and the handshake's constants are
//! interoperability facts from node-blackmagic-controller (MIT) and smunaut's
//! blackmagic-misc (Apache-2.0); this code is Ryu's own.

use std::time::{Duration, Instant};

use super::{Driver, DriverError, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const READ_BUFFER: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 16;
const AUTH_REPORT_LEN: usize = 10;
/// The handshake's steps, echoed in byte 1 of each answer.
const AUTH_RESET: u8 = 0;
const AUTH_HOST_CHALLENGE: u8 = 1;
const AUTH_DEVICE_ANSWER: u8 = 2;
const AUTH_RESPONSE: u8 = 3;
const AUTH_OK: u8 = 4;
/// What a panel that names no timeout is assumed to allow, in seconds.
const DEFAULT_AUTH_TIMEOUT_S: u64 = 600;
/// How long to wait before trying a failed re-handshake again.
const AUTH_RETRY: Duration = Duration::from_millis(7500);
/// Consecutive failed re-handshakes before the panel counts as lost.
const AUTH_ATTEMPTS: u8 = 3;
/// The output report that sets the jog wheel's mode, and the mode Ryu wants:
/// relative, so each report is a movement rather than a position.
const JOG_MODE_REPORT: u8 = 0x03;
const JOG_RELATIVE: u8 = 0;
/// The other relative mode a panel may already be in; modes 1 and 3 report
/// absolute positions and are not turned into steps.
const JOG_RELATIVE_ALT: u8 = 2;
/// Jog units per dial step. In its absolute mode the wheel spans -4096 to 4096
/// over about half a turn, so this is roughly 64 steps a turn.
const JOG_STEP: i64 = 256;
/// T-bar units per dial step: its full travel (4096) is 64 steps.
const TBAR_STEP: i32 = 64;
const TBAR_SEGMENTS: u16 = 16;
/// A light that is only on or off is on for any colour brighter than this, so
/// only black and near-black leave it off (the palette's dark slate lights it).
const ON_OFF_THRESHOLD: u8 = 16;
/// One colour channel of an ATEM key is lit from this level up.
const RGB_THRESHOLD: u8 = 128;

const AUTH_EVEN: [u64; 8] = [
    0x3ae1_206f_97c1_0bc8,
    0x2a9a_b32b_ebf2_44c6,
    0x20a6_f8b8_df9a_df0a,
    0xaf80_ece5_2cfc_1719,
    0xec2e_e2f7_414f_d151,
    0xb055_adfd_7334_4a15,
    0xa63d_2e30_5900_1187,
    0x751b_f623_f42e_0dde,
];
const AUTH_ODD: [u64; 8] = [
    0x3e22_b34f_502e_7fde,
    0x2465_6b98_1875_ab1c,
    0xa17f_3456_df7b_f8c3,
    0x6df7_2e19_41ae_f698,
    0x7222_6f01_1e66_ab94,
    0x3831_a3c6_0629_6b42,
    0xfd7f_f818_8133_2c89,
    0x61a3_f647_4ff2_36c6,
];
const AUTH_MASK: u64 = 0xa79a_63f5_85d3_7bf0;

/// The answer to a panel's challenge.
#[must_use]
pub fn auth_response(challenge: u64) -> u64 {
    let turn = |value: u64| value.rotate_right(8);
    let n = (challenge & 7) as usize;
    let mut value = challenge;
    for _ in 0..n {
        value = turn(value);
    }
    let key = if value & 1 == (0x78 >> n) & 1 {
        AUTH_EVEN[n]
    } else {
        value ^= turn(value);
        AUTH_ODD[n]
    };
    value ^ (turn(value) & AUTH_MASK) ^ key
}

/// Where a key's light lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Led {
    None,
    /// One on/off bit in the main light report.
    Main(u8),
    /// One on/off bit in the jog-mode light report (the editors' JOG, SHTL,
    /// SCRL and SLOW JOG keys).
    Jog(u8),
    /// Red, green and blue bits from here on, in the main light report.
    Rgb(u8),
}

#[derive(Debug, Clone, Copy)]
pub struct PanelKey {
    /// The legend printed on the key.
    pub name: &'static str,
    pub code: u16,
    pub row: u8,
    pub col: u8,
    pub led: Led,
}

const fn key(row: u8, col: u8, code: u16, name: &'static str, led: Led) -> PanelKey {
    PanelKey {
        name,
        code,
        row,
        col,
        led,
    }
}

/// An ATEM Micro Panel key: every one has an RGB light, at a bit set by its code.
const fn atem(row: u8, col: u8, code: u16, name: &'static str) -> PanelKey {
    key(row, col, code, name, Led::Rgb((16 + 3 * (code - 1)) as u8))
}

/// One panel model's reports and keys.
#[derive(Debug)]
pub struct Panel {
    /// The feature report the handshake runs over.
    auth_report: u8,
    /// How many handshakes in a row it wants (the Replay Editor does two).
    auth_rounds: u8,
    button_report: u8,
    jog_report: Option<u8>,
    tbar_report: Option<u8>,
    /// The T-bar's dial index, after the jog wheel where there is one.
    tbar_dial: u8,
    main_report: u8,
    /// The main light report's length, id included.
    main_len: usize,
    /// The jog-mode keys' light report (two bytes), on the editors.
    jog_light_report: Option<u8>,
    /// The first of the T-bar's 16 light bits in the main report.
    tbar_first_bit: Option<u8>,
    /// Keys in Ryu's order: row by row, left to right.
    pub keys: &'static [PanelKey],
}

const SPEED_EDITOR_KEYS: &[PanelKey] = &[
    key(0, 0, 0x01, "Smart Insert", Led::None),
    key(0, 1, 0x02, "Append", Led::None),
    key(0, 2, 0x03, "Ripple Overwrite", Led::None),
    key(0, 3, 0x31, "Esc", Led::None),
    key(0, 4, 0x1f, "Sync Bin", Led::None),
    key(0, 5, 0x2c, "Audio Level", Led::None),
    key(0, 6, 0x2d, "Full View", Led::None),
    key(0, 7, 0x1a, "Source", Led::None),
    key(0, 9, 0x1b, "Timeline", Led::None),
    key(1, 0, 0x04, "Close Up", Led::Main(0)),
    key(1, 1, 0x05, "Place on Top", Led::None),
    key(1, 2, 0x06, "Source Overwrite", Led::None),
    key(1, 3, 0x22, "Transition", Led::Main(4)),
    key(1, 4, 0x2f, "Split", Led::None),
    key(1, 5, 0x2e, "Snap", Led::Main(5)),
    key(1, 6, 0x2b, "Ripple Delete", Led::None),
    key(1, 7, 0x1c, "Shuttle", Led::Jog(1)),
    key(1, 8, 0x1d, "Jog", Led::Jog(0)),
    key(1, 9, 0x1e, "Scroll", Led::Jog(2)),
    key(2, 0, 0x07, "In", Led::None),
    key(2, 2, 0x08, "Out", Led::None),
    key(2, 3, 0x39, "Cam 7", Led::Main(6)),
    key(2, 4, 0x3a, "Cam 8", Led::Main(7)),
    key(2, 5, 0x3b, "Cam 9", Led::Main(8)),
    key(2, 6, 0x30, "Live Overwrite", Led::Main(9)),
    key(3, 0, 0x09, "Trim In", Led::None),
    key(3, 1, 0x0a, "Trim Out", Led::None),
    key(3, 2, 0x0b, "Roll", Led::None),
    key(3, 3, 0x36, "Cam 4", Led::Main(10)),
    key(3, 4, 0x37, "Cam 5", Led::Main(11)),
    key(3, 5, 0x38, "Cam 6", Led::Main(12)),
    key(3, 6, 0x25, "Video Only", Led::Main(13)),
    key(4, 0, 0x0c, "Slip Source", Led::None),
    key(4, 1, 0x0d, "Slip Destination", Led::None),
    key(4, 2, 0x0e, "Transition Duration", Led::None),
    key(4, 3, 0x33, "Cam 1", Led::Main(14)),
    key(4, 4, 0x34, "Cam 2", Led::Main(15)),
    key(4, 5, 0x35, "Cam 3", Led::Main(16)),
    key(4, 6, 0x26, "Audio Only", Led::Main(17)),
    key(5, 0, 0x0f, "Cut", Led::Main(1)),
    key(5, 1, 0x10, "Dissolve", Led::Main(2)),
    key(5, 2, 0x11, "Smooth Cut", Led::Main(3)),
    key(5, 3, 0x3c, "Stop / Play", Led::None),
];

const REPLAY_EDITOR_KEYS: &[PanelKey] = &[
    key(0, 0, 0x65, "Undo", Led::None),
    key(0, 1, 0x3e, "New Timeline", Led::None),
    key(0, 6, 0x3f, "Cue", Led::Main(0)),
    key(0, 7, 0x40, "Run", Led::Main(1)),
    key(0, 8, 0x41, "Dump", Led::Main(2)),
    key(0, 10, 0x6b, "Go to End", Led::None),
    key(0, 11, 0x42, "Input View", Led::Main(3)),
    key(0, 12, 0x43, "Multi Source", Led::Main(4)),
    key(0, 14, 0x44, "Single Clip", Led::Main(5)),
    key(1, 1, 0x3d, "Trim All", Led::None),
    key(1, 3, 0x2b, "Ripple Delete", Led::None),
    key(1, 4, 0x25, "Video Only", Led::Main(6)),
    key(1, 5, 0x45, "Auto Sting", Led::Main(7)),
    key(1, 6, 0x46, "Ramp Down", Led::None),
    key(1, 7, 0x47, "Speed Position", Led::Main(8)),
    key(1, 9, 0x1f, "Sync Bin", Led::None),
    key(1, 10, 0x68, "Add Mark", Led::None),
    key(1, 11, 0x2d, "Full View", Led::Main(9)),
    key(2, 4, 0x26, "Audio Only", Led::Main(10)),
    key(2, 5, 0x48, "Select Sting", Led::Main(11)),
    key(2, 6, 0x49, "Slow", Led::None),
    key(2, 7, 0x4a, "Set Speed", Led::Main(12)),
    key(2, 9, 0x2f, "Split", Led::None),
    key(2, 10, 0x2e, "Snap", Led::Main(13)),
    key(2, 11, 0x4b, "Multiview View", Led::Main(14)),
    key(2, 12, 0x1a, "Source", Led::None),
    key(2, 14, 0x1b, "Timeline", Led::None),
    key(3, 0, 0x63, "Live Speed", Led::Main(15)),
    key(3, 1, 0x01, "Smart Insert", Led::None),
    key(3, 2, 0x02, "Append", Led::None),
    key(3, 3, 0x03, "Ripple Overwrite", Led::None),
    key(4, 1, 0x04, "Close Up", Led::Main(16)),
    key(4, 2, 0x05, "Place on Top", Led::None),
    key(4, 3, 0x06, "Source Overwrite", Led::None),
    key(4, 4, 0x4d, "Set Point of Interest", Led::Main(17)),
    key(4, 5, 0x4c, "Go to Point of Interest", Led::None),
    key(4, 6, 0x5d, "2 Sec", Led::Main(18)),
    key(4, 7, 0x5e, "3 Sec", Led::Main(19)),
    key(4, 8, 0x5f, "4 Sec", Led::Main(20)),
    key(4, 9, 0x60, "5 Sec", Led::Main(21)),
    key(4, 10, 0x61, "6 Sec", Led::Main(22)),
    key(4, 11, 0x62, "7 Sec", Led::Main(23)),
    key(4, 12, 0x69, "Slow Jog", Led::Jog(3)),
    key(4, 13, 0x1d, "Jog", Led::Jog(0)),
    key(4, 14, 0x1e, "Scroll", Led::Jog(2)),
    key(5, 1, 0x07, "In", Led::None),
    key(5, 3, 0x08, "Out", Led::None),
    key(5, 4, 0x64, "All Cams", Led::Main(24)),
    key(5, 5, 0x5c, "Cams 9-16", Led::Main(25)),
    key(5, 6, 0x4e, "Title 1", Led::Main(26)),
    key(5, 7, 0x4f, "Title 2", Led::Main(27)),
    key(5, 8, 0x50, "Title 3", Led::Main(28)),
    key(5, 9, 0x51, "Title 4", Led::Main(29)),
    key(5, 10, 0x52, "Title 5", Led::Main(30)),
    key(5, 11, 0x53, "Title 6", Led::Main(31)),
    key(6, 1, 0x09, "Trim In", Led::None),
    key(6, 2, 0x0a, "Trim Out", Led::None),
    key(6, 3, 0x0b, "Roll", Led::None),
    key(7, 1, 0x67, "Slip", Led::None),
    key(7, 2, 0x66, "Slide", Led::None),
    key(7, 3, 0x0e, "Transition Duration", Led::None),
    key(7, 4, 0x54, "Cam 1", Led::Main(32)),
    key(7, 5, 0x55, "Cam 2", Led::Main(33)),
    key(7, 6, 0x56, "Cam 3", Led::Main(34)),
    key(7, 7, 0x57, "Cam 4", Led::Main(35)),
    key(7, 8, 0x58, "Cam 5", Led::Main(36)),
    key(7, 9, 0x59, "Cam 6", Led::Main(37)),
    key(7, 10, 0x5a, "Cam 7", Led::Main(38)),
    key(7, 11, 0x5b, "Cam 8", Led::Main(39)),
    key(8, 1, 0x0f, "Cut", Led::Main(40)),
    key(8, 2, 0x10, "Dissolve", Led::Main(41)),
    key(8, 3, 0x22, "Transition", Led::Main(42)),
    key(8, 4, 0x3c, "Stop / Play", Led::None),
];

const ATEM_MICRO_PANEL_KEYS: &[PanelKey] = &[
    atem(0, 5, 0x01, "Macro"),
    atem(0, 6, 0x02, "Key 1 On"),
    atem(0, 7, 0x03, "Key 2 On"),
    atem(0, 8, 0x04, "Key 3 On"),
    atem(0, 9, 0x05, "Key 4 On"),
    atem(0, 10, 0x1f, "M/E 1"),
    atem(0, 11, 0x20, "M/E 2"),
    atem(0, 12, 0x21, "M/E 3"),
    atem(0, 13, 0x22, "M/E 4"),
    atem(1, 5, 0x06, "Next Background"),
    atem(1, 6, 0x07, "Next Key 1"),
    atem(1, 7, 0x08, "Next Key 2"),
    atem(1, 8, 0x09, "Next Key 3"),
    atem(1, 9, 0x0a, "Next Key 4"),
    atem(1, 11, 0x23, "Dip"),
    atem(1, 12, 0x24, "DVE"),
    atem(1, 13, 0x25, "Sting"),
    atem(1, 15, 0x2d, "DSK 1 Tie"),
    atem(1, 16, 0x2e, "DSK 2 Tie"),
    atem(2, 0, 0x0b, "Program 1"),
    atem(2, 1, 0x0c, "Program 2"),
    atem(2, 2, 0x0d, "Program 3"),
    atem(2, 3, 0x0e, "Program 4"),
    atem(2, 4, 0x0f, "Program 5"),
    atem(2, 5, 0x10, "Program 6"),
    atem(2, 6, 0x11, "Program 7"),
    atem(2, 7, 0x12, "Program 8"),
    atem(2, 8, 0x13, "Program 9"),
    atem(2, 9, 0x14, "Program 10"),
    atem(2, 10, 0x26, "Shift"),
    atem(2, 11, 0x27, "Mix"),
    atem(2, 12, 0x28, "Wipe"),
    atem(2, 13, 0x29, "Arm"),
    atem(2, 15, 0x2f, "DSK 1 Cut"),
    atem(2, 16, 0x30, "DSK 2 Cut"),
    atem(3, 0, 0x15, "Preview 1"),
    atem(3, 1, 0x16, "Preview 2"),
    atem(3, 2, 0x17, "Preview 3"),
    atem(3, 3, 0x18, "Preview 4"),
    atem(3, 4, 0x19, "Preview 5"),
    atem(3, 5, 0x1a, "Preview 6"),
    atem(3, 6, 0x1b, "Preview 7"),
    atem(3, 7, 0x1c, "Preview 8"),
    atem(3, 8, 0x1d, "Preview 9"),
    atem(3, 9, 0x1e, "Preview 10"),
    atem(3, 10, 0x2a, "Preview Transition"),
    atem(3, 11, 0x2b, "Cut"),
    atem(3, 13, 0x2c, "Auto"),
    atem(3, 15, 0x31, "DSK 1 Auto"),
    atem(3, 16, 0x32, "DSK 2 Auto"),
];

const SPEED_EDITOR: Panel = Panel {
    auth_report: 6,
    auth_rounds: 1,
    button_report: 0x04,
    jog_report: Some(0x03),
    tbar_report: None,
    tbar_dial: 0,
    main_report: 0x02,
    main_len: 5,
    jog_light_report: Some(0x04),
    tbar_first_bit: None,
    keys: SPEED_EDITOR_KEYS,
};

const REPLAY_EDITOR: Panel = Panel {
    auth_report: 6,
    auth_rounds: 2,
    button_report: 0x04,
    jog_report: Some(0x03),
    tbar_report: Some(0x0a),
    tbar_dial: 1,
    main_report: 0x09,
    main_len: 9,
    jog_light_report: Some(0x04),
    tbar_first_bit: Some(43),
    keys: REPLAY_EDITOR_KEYS,
};

const ATEM_MICRO_PANEL: Panel = Panel {
    auth_report: 5,
    auth_rounds: 1,
    button_report: 0x03,
    jog_report: None,
    tbar_report: Some(0x08),
    tbar_dial: 0,
    main_report: 0x02,
    main_len: 32,
    jog_light_report: None,
    tbar_first_bit: Some(0),
    keys: ATEM_MICRO_PANEL_KEYS,
};

/// The panel a model id names, if it is a Blackmagic one.
#[must_use]
pub fn panel(model_id: &str) -> Option<&'static Panel> {
    match model_id {
        "blackmagic-speed-editor" => Some(&SPEED_EDITOR),
        "blackmagic-replay-editor" => Some(&REPLAY_EDITOR),
        "blackmagic-atem-micro-panel" => Some(&ATEM_MICRO_PANEL),
        _ => None,
    }
}

fn set_bit(buffer: &mut [u8], bit: u8, on: bool) {
    let Some(byte) = buffer.get_mut(1 + usize::from(bit / 8)) else {
        return;
    };
    let mask = 1 << (bit % 8);
    if on {
        *byte |= mask;
    } else {
        *byte &= !mask;
    }
}

/// A light buffer and what the panel last received of it.
struct Lights {
    staged: Vec<u8>,
    sent: Option<Vec<u8>>,
}

impl Lights {
    fn new(report: u8, len: usize) -> Self {
        let mut staged = vec![0; len];
        staged[0] = report;
        Self { staged, sent: None }
    }

    fn send_if_changed(&mut self, transport: &mut dyn Transport) -> DriverResult<()> {
        if self.sent.as_ref() != Some(&self.staged) {
            transport.write(&self.staged)?;
            self.sent = Some(self.staged.clone());
        }
        Ok(())
    }
}

pub struct Blackmagic {
    transport: Box<dyn Transport>,
    panel: &'static Panel,
    held: Vec<bool>,
    main: Lights,
    jog_lights: Option<Lights>,
    /// When the next handshake is due; `None` before the first one.
    next_auth: Option<Instant>,
    failed_auths: u8,
    /// Jog units not yet turned into a step.
    jog_rest: i64,
    tbar: Option<u16>,
    tbar_rest: i32,
}

impl Blackmagic {
    /// [`super::for_model`] builds this only for a model [`panel`] knows.
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let panel = panel(model.id).unwrap_or(&SPEED_EDITOR);
        Self {
            transport,
            panel,
            held: vec![false; panel.keys.len()],
            main: Lights::new(panel.main_report, panel.main_len),
            jog_lights: panel.jog_light_report.map(|report| Lights::new(report, 2)),
            next_auth: None,
            failed_auths: 0,
            jog_rest: 0,
            tbar: None,
            tbar_rest: 0,
        }
    }

    /// Sends one handshake step and reads the panel's answer, which must echo
    /// the report id and `expect`.
    fn auth_step(
        &mut self,
        step: u8,
        payload: u64,
        expect: u8,
    ) -> DriverResult<[u8; AUTH_REPORT_LEN]> {
        let report = self.panel.auth_report;
        let mut out = [0u8; AUTH_REPORT_LEN];
        out[0] = report;
        out[1] = step;
        out[2..].copy_from_slice(&payload.to_le_bytes());
        self.transport.send_feature_report(&out)?;
        let mut answer = [0u8; AUTH_REPORT_LEN];
        answer[0] = report;
        let read = self.transport.get_feature_report(&mut answer)?;
        if read < AUTH_REPORT_LEN || answer[0] != report || answer[1] != expect {
            return Err(DriverError::Io(format!(
                "the panel refused the handshake at step {step}"
            )));
        }
        Ok(answer)
    }

    /// One full handshake; returns how long the panel stays unlocked.
    fn handshake(&mut self) -> DriverResult<Duration> {
        let challenge = self.auth_step(AUTH_RESET, 0, AUTH_RESET)?;
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&challenge[2..]);
        let challenge = u64::from_le_bytes(bytes);
        // Ryu does not check the panel's answer to a host challenge of zero.
        self.auth_step(AUTH_HOST_CHALLENGE, 0, AUTH_DEVICE_ANSWER)?;
        let status = self.auth_step(AUTH_RESPONSE, auth_response(challenge), AUTH_OK)?;
        let seconds = u64::from(u16::from_le_bytes([status[2], status[3]]));
        Ok(Duration::from_secs(if seconds == 0 {
            DEFAULT_AUTH_TIMEOUT_S
        } else {
            seconds
        }))
    }

    /// Every handshake the panel wants; the last one's timeout counts.
    fn authenticate(&mut self) -> DriverResult<()> {
        let mut timeout = Duration::from_secs(DEFAULT_AUTH_TIMEOUT_S);
        for _ in 0..self.panel.auth_rounds.max(1) {
            timeout = self.handshake()?;
        }
        self.failed_auths = 0;
        self.next_auth = Some(Instant::now() + timeout / 2);
        Ok(())
    }

    /// The first handshake, before anything else is sent or expected, then the
    /// jog wheel put in its relative mode: another app may have left it
    /// absolute. A panel that will not shake hands is refused, so the manager
    /// waits before opening it again instead of reconnecting every scan.
    fn ensure_authenticated(&mut self) -> DriverResult<()> {
        if self.next_auth.is_some() {
            return Ok(());
        }
        self.authenticate().map_err(|error| {
            DriverError::Refused(format!(
                "the panel did not accept Ryu's handshake ({error})"
            ))
        })?;
        if self.panel.jog_report.is_some() {
            let mut mode = [0u8; 7];
            mode[0] = JOG_MODE_REPORT;
            mode[1] = JOG_RELATIVE;
            mode[6] = 0xff;
            self.transport.write(&mode)?;
        }
        Ok(())
    }

    fn parse(&mut self, report: &[u8]) -> Vec<Input> {
        let Some(&id) = report.first() else {
            return Vec::new();
        };
        if id == self.panel.button_report {
            self.parse_buttons(report)
        } else if Some(id) == self.panel.jog_report {
            self.parse_jog(report)
        } else if Some(id) == self.panel.tbar_report {
            self.parse_tbar(report)
        } else {
            // The battery level and anything else.
            Vec::new()
        }
    }

    fn parse_buttons(&mut self, report: &[u8]) -> Vec<Input> {
        let codes: Vec<u16> = report
            .get(1..)
            .unwrap_or_default()
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|code| *code != 0)
            .collect();
        let mut events = Vec::new();
        for (index, (key, held)) in self.panel.keys.iter().zip(&mut self.held).enumerate() {
            let pressed = codes.contains(&key.code);
            if pressed != *held {
                *held = pressed;
                events.push(Input::Key {
                    index: index as u8,
                    pressed,
                });
            }
        }
        events
    }

    fn parse_jog(&mut self, report: &[u8]) -> Vec<Input> {
        let (Some(&mode), Some(bytes)) = (report.get(1), report.get(2..6)) else {
            return Vec::new();
        };
        // An absolute position is not a movement; drop it until the mode set
        // after the handshake takes effect.
        if mode != JOG_RELATIVE && mode != JOG_RELATIVE_ALT {
            return Vec::new();
        }
        let value = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        self.jog_rest += i64::from(value);
        let steps = self.jog_rest / JOG_STEP;
        // A fling past what one event can carry is dropped, not saved up.
        self.jog_rest -= steps * JOG_STEP;
        dial_turn(0, steps)
    }

    fn parse_tbar(&mut self, report: &[u8]) -> Vec<Input> {
        let (Some(&low), Some(&high)) = (report.get(1), report.get(2)) else {
            return Vec::new();
        };
        let position = u16::from_le_bytes([low, high]);
        let Some(previous) = self.tbar.replace(position) else {
            return Vec::new();
        };
        self.tbar_rest += i32::from(position) - i32::from(previous);
        let steps = self.tbar_rest / TBAR_STEP;
        self.tbar_rest -= steps * TBAR_STEP;
        dial_turn(self.panel.tbar_dial, i64::from(steps))
    }
}

fn dial_turn(index: u8, steps: i64) -> Vec<Input> {
    if steps == 0 {
        return Vec::new();
    }
    let delta = i8::try_from(steps.clamp(-127, 127)).unwrap_or(0);
    vec![Input::DialTurn { index, delta }]
}

impl Driver for Blackmagic {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_authenticated()?;
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

    /// Lights the key in its colour; keys without a light ignore it.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let Some(panel_key) = self.panel.keys.get(usize::from(key)) else {
            return Ok(());
        };
        let [r, g, b] = face.map_or([0, 0, 0], |face| face.color);
        let on = r.max(g).max(b) > ON_OFF_THRESHOLD;
        match panel_key.led {
            Led::None => {}
            Led::Main(bit) => set_bit(&mut self.main.staged, bit, on),
            Led::Jog(bit) => {
                if let Some(lights) = &mut self.jog_lights {
                    set_bit(&mut lights.staged, bit, on);
                }
            }
            Led::Rgb(bit) => {
                set_bit(&mut self.main.staged, bit, r >= RGB_THRESHOLD);
                set_bit(&mut self.main.staged, bit + 1, g >= RGB_THRESHOLD);
                set_bit(&mut self.main.staged, bit + 2, b >= RGB_THRESHOLD);
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_authenticated()?;
        if let Some(lights) = &mut self.jog_lights {
            lights.send_if_changed(self.transport.as_mut())?;
        }
        self.main.send_if_changed(self.transport.as_mut())
    }

    /// The lights are only on or off.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }

    /// The T-bar's 16 segments: all lit for any colour, dark for none.
    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        let Some(first) = self.panel.tbar_first_bit else {
            return Ok(());
        };
        let on = color.is_some_and(|[r, g, b]| r.max(g).max(b) > ON_OFF_THRESHOLD);
        for segment in 0..TBAR_SEGMENTS {
            let bit = u8::try_from(u16::from(first) + segment).unwrap_or(u8::MAX);
            set_bit(&mut self.main.staged, bit, on);
        }
        Ok(())
    }

    /// Repeats the handshake before the panel's timeout runs out. A failure is
    /// retried after a short wait; the third in a row loses the panel.
    fn heartbeat(&mut self) -> DriverResult<()> {
        let Some(due) = self.next_auth else {
            return self.ensure_authenticated();
        };
        if Instant::now() < due {
            return Ok(());
        }
        match self.authenticate() {
            Ok(()) => Ok(()),
            Err(error) => {
                self.failed_auths = self.failed_auths.saturating_add(1);
                if self.failed_auths >= AUTH_ATTEMPTS {
                    return Err(error);
                }
                self.next_auth = Some(Instant::now() + AUTH_RETRY);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    const CHALLENGE: u64 = 0x0123_4567_89ab_cdef;

    fn face(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    /// Queues the answers to one handshake on report `id`, with `timeout` seconds.
    fn queue_handshake(recorder: &Recorder, id: u8, timeout: u16) {
        let mut challenge = vec![id, AUTH_RESET];
        challenge.extend_from_slice(&CHALLENGE.to_le_bytes());
        recorder.queue_feature_reply(challenge);
        recorder.queue_feature_reply(vec![id, AUTH_DEVICE_ANSWER, 0, 0, 0, 0, 0, 0, 0, 0]);
        let [low, high] = timeout.to_le_bytes();
        recorder.queue_feature_reply(vec![id, AUTH_OK, low, high, 0, 0, 0, 0, 0, 0]);
    }

    fn driver(model: &str) -> (Blackmagic, Recorder) {
        let recorder = Recorder::default();
        (
            Blackmagic::new(by_id(model).unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    #[test]
    fn the_auth_response_matches_known_answers() {
        // Worked out separately from the published algorithm.
        assert_eq!(auth_response(0), 0x3ae1_206f_97c1_0bc8);
        assert_eq!(auth_response(1), 0x2b9a_b32b_ebf2_44c6);
        assert_eq!(auth_response(5), 0x3831_a3c6_032d_6a42);
        assert_eq!(auth_response(CHALLENGE), 0xe5c7_b689_e996_7608);
        assert_eq!(auth_response(0xfedc_ba98_7654_3210), 0xc4a7_da47_61c7_69e8);
        assert_eq!(auth_response(u64::MAX), 0x61a3_f647_4ff2_36c6);
    }

    #[test]
    fn the_handshake_runs_before_input_and_answers_the_challenge() {
        let (mut deck, recorder) = driver("blackmagic-speed-editor");
        queue_handshake(&recorder, 6, 600);
        assert!(deck.poll(0).unwrap().is_empty());
        let mut response = vec![6, AUTH_RESPONSE];
        response.extend_from_slice(&auth_response(CHALLENGE).to_le_bytes());
        assert_eq!(
            recorder.features(),
            vec![
                vec![6, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                vec![6, 1, 0, 0, 0, 0, 0, 0, 0, 0],
                response
            ]
        );
        // Unlocked for 600 s, so the next handshake is due in about 300.
        let due = deck.next_auth.unwrap().duration_since(Instant::now());
        assert!(due > Duration::from_secs(290) && due <= Duration::from_secs(300));
        // Not yet due: the heartbeat sends nothing.
        deck.heartbeat().unwrap();
        assert_eq!(recorder.features().len(), 3);
    }

    #[test]
    fn a_wrong_step_echo_fails_the_handshake() {
        let (mut deck, recorder) = driver("blackmagic-speed-editor");
        recorder.queue_feature_reply(vec![6, AUTH_RESET, 1, 2, 3, 4, 5, 6, 7, 8]);
        recorder.queue_feature_reply(vec![6, 7, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(deck.poll(0).is_err());
    }

    #[test]
    fn the_replay_editor_shakes_hands_twice_and_the_atem_on_report_five() {
        let (mut replay, recorder) = driver("blackmagic-replay-editor");
        queue_handshake(&recorder, 6, 10);
        queue_handshake(&recorder, 6, 0);
        replay.poll(0).unwrap();
        assert_eq!(recorder.features().len(), 6);
        // A timeout of zero means the default 600 s.
        let due = replay.next_auth.unwrap().duration_since(Instant::now());
        assert!(due > Duration::from_secs(290));

        let (mut atem, recorder) = driver("blackmagic-atem-micro-panel");
        queue_handshake(&recorder, 5, 600);
        atem.poll(0).unwrap();
        assert!(recorder.features().iter().all(|report| report[0] == 5));
    }

    #[test]
    fn a_failed_rehandshake_is_retried_and_the_third_loses_the_panel() {
        let (mut deck, recorder) = driver("blackmagic-speed-editor");
        queue_handshake(&recorder, 6, 600);
        deck.poll(0).unwrap();
        for attempt in 1..=3 {
            deck.next_auth = Some(Instant::now() - Duration::from_secs(1));
            let result = deck.heartbeat();
            if attempt < 3 {
                result.unwrap();
                let wait = deck.next_auth.unwrap().duration_since(Instant::now());
                assert!(wait > Duration::from_secs(7) && wait <= AUTH_RETRY);
            } else {
                assert!(result.is_err());
            }
        }
        // A success in between resets the count.
        let (mut deck, recorder) = driver("blackmagic-speed-editor");
        queue_handshake(&recorder, 6, 600);
        deck.poll(0).unwrap();
        deck.next_auth = Some(Instant::now() - Duration::from_secs(1));
        deck.heartbeat().unwrap();
        assert_eq!(deck.failed_auths, 1);
        queue_handshake(&recorder, 6, 600);
        deck.next_auth = Some(Instant::now() - Duration::from_secs(1));
        deck.heartbeat().unwrap();
        assert_eq!(deck.failed_auths, 0);
    }

    #[test]
    fn held_keycodes_become_presses_and_releases() {
        let (mut deck, recorder) = driver("blackmagic-speed-editor");
        queue_handshake(&recorder, 6, 600);
        // CUT (0x0f, key 39) and CAM 1 (0x33, key 35) held, then only CUT.
        recorder.queue_read(vec![0x04, 0x0f, 0, 0x33, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        recorder.queue_read(vec![0x04, 0x0f, 0, 0, 0, 0x33, 0]);
        // An unknown code and a stray odd byte change nothing; an empty list
        // releases everything.
        recorder.queue_read(vec![0x04, 0x0f, 0, 0x77, 0, 0x99]);
        recorder.queue_read(vec![0x04]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 35,
                    pressed: true
                },
                Input::Key {
                    index: 39,
                    pressed: true
                },
                Input::Key {
                    index: 35,
                    pressed: false
                },
                Input::Key {
                    index: 39,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn the_jog_turns_dial_zero_in_steps() {
        let (mut deck, recorder) = driver("blackmagic-speed-editor");
        queue_handshake(&recorder, 6, 600);
        recorder.queue_read(vec![0x03, 0, 0x00, 0x02, 0, 0, 0]); // +512
        recorder.queue_read(vec![0x03, 0, 0x80, 0, 0, 0, 0]); // +128: half a step
        recorder.queue_read(vec![0x03, 0, 0x80, 0, 0, 0, 0]); // +128 completes it
        recorder.queue_read(vec![0x03, 0, 0x00, 0xfe, 0xff, 0xff, 0]); // -512
        recorder.queue_read(vec![0x03, 0, 0xff]); // truncated
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialTurn { index: 0, delta: 2 },
                Input::DialTurn { index: 0, delta: 1 },
                Input::DialTurn {
                    index: 0,
                    delta: -2
                },
            ]
        );
    }

    #[test]
    fn the_tbar_is_a_dial_after_the_jog() {
        let (mut deck, recorder) = driver("blackmagic-replay-editor");
        queue_handshake(&recorder, 6, 600);
        queue_handshake(&recorder, 6, 600);
        // The first position (1000) is the baseline; then +640 and -64.
        recorder.queue_read(vec![0x0a, 0xe8, 0x03]);
        recorder.queue_read(vec![0x0a, 0x68, 0x06]);
        recorder.queue_read(vec![0x0a, 0x28, 0x06]);
        // The Replay Editor's buttons are report 4 as on the Speed Editor.
        recorder.queue_read(vec![0x04, 0x65, 0]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                Input::DialTurn {
                    index: 1,
                    delta: 10
                },
                Input::DialTurn {
                    index: 1,
                    delta: -1
                },
                Input::Key {
                    index: 0,
                    pressed: true
                },
            ]
        );

        let (mut atem, recorder) = driver("blackmagic-atem-micro-panel");
        queue_handshake(&recorder, 5, 600);
        recorder.queue_read(vec![0x08, 0, 0]);
        recorder.queue_read(vec![0x08, 0, 0x10]); // 4096: full travel
                                                  // On the ATEM report 3 is buttons: AUTO (0x2c) is key 47.
        recorder.queue_read(vec![0x03, 0x2c, 0]);
        assert_eq!(
            atem.poll(0).unwrap(),
            vec![
                Input::DialTurn {
                    index: 0,
                    delta: 64
                },
                Input::Key {
                    index: 47,
                    pressed: true
                },
            ]
        );
    }

    #[test]
    fn speed_editor_lights_are_bits_and_only_changes_are_sent() {
        let (mut deck, recorder) = driver("blackmagic-speed-editor");
        queue_handshake(&recorder, 6, 600);
        // CUT (bit 1), AUDIO ONLY (bit 17), the JOG key (jog bit 0), and a key
        // without a light.
        deck.set_face(39, Some(&face([255, 255, 255]))).unwrap();
        deck.set_face(38, Some(&face([200, 0, 0]))).unwrap();
        deck.set_face(17, Some(&face([0, 0, 255]))).unwrap();
        deck.set_face(0, Some(&face([255, 0, 0]))).unwrap();
        deck.set_face(99, Some(&face([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                JOG_MODE_RELATIVE.to_vec(),
                vec![0x04, 0b001],
                vec![0x02, 0b10, 0, 0b10, 0]
            ]
        );
        // Unchanged: nothing more. Near-black turns CUT off; only the main
        // report goes.
        deck.flush().unwrap();
        deck.set_face(39, Some(&face([10, 10, 10]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 4);
        assert_eq!(recorder.writes()[3], vec![0x02, 0, 0, 0b10, 0]);
        // The Companion's default colour, a dark slate, lights it again.
        deck.set_face(39, Some(&face([0x1f, 0x29, 0x37]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes()[4], vec![0x02, 0b10, 0, 0b10, 0]);
    }

    /// What the driver sends after the first handshake: relative jog mode.
    const JOG_MODE_RELATIVE: [u8; 7] = [0x03, 0, 0, 0, 0, 0, 0xff];

    #[test]
    fn the_jog_is_set_relative_and_absolute_reports_are_dropped() {
        let (mut deck, recorder) = driver("blackmagic-speed-editor");
        queue_handshake(&recorder, 6, 600);
        // Left absolute by another app: positions of 2000 and 2100 are not
        // movements. Mode 2 is relative too.
        recorder.queue_read(vec![0x03, 1, 0xd0, 0x07, 0, 0, 0xff]);
        recorder.queue_read(vec![0x03, 3, 0x34, 0x08, 0, 0, 0xff]);
        recorder.queue_read(vec![0x03, 2, 0x00, 0x01, 0, 0, 0xff]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::DialTurn { index: 0, delta: 1 }]
        );
        assert_eq!(recorder.writes(), vec![JOG_MODE_RELATIVE.to_vec()]);
    }

    #[test]
    fn a_panel_that_will_not_shake_hands_is_refused() {
        let (mut deck, _recorder) = driver("blackmagic-speed-editor");
        // No answers queued: the first handshake fails.
        assert!(matches!(deck.flush(), Err(DriverError::Refused(_))));
        assert!(matches!(deck.poll(0), Err(DriverError::Refused(_))));
    }

    #[test]
    fn replay_editor_lights_and_tbar_segments() {
        let (mut deck, recorder) = driver("blackmagic-replay-editor");
        queue_handshake(&recorder, 6, 600);
        queue_handshake(&recorder, 6, 600);
        // TRANSITION is bit 42; SLOW JOG is jog bit 3; the T-bar is bits 43..58.
        deck.set_face(71, Some(&face([255, 255, 255]))).unwrap();
        deck.set_face(42, Some(&face([255, 255, 255]))).unwrap();
        deck.set_deck_light(Some([0, 255, 0])).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                JOG_MODE_RELATIVE.to_vec(),
                vec![0x04, 0b1000],
                vec![0x09, 0, 0, 0, 0, 0, 0b1111_1100, 0xff, 0b0000_0111],
            ]
        );
        deck.set_deck_light(None).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[3],
            vec![0x09, 0, 0, 0, 0, 0, 0b0000_0100, 0, 0]
        );
    }

    #[test]
    fn atem_keys_take_one_bit_per_channel() {
        let (mut deck, recorder) = driver("blackmagic-atem-micro-panel");
        queue_handshake(&recorder, 5, 600);
        // MACRO (key 0) is bits 16..18: red and blue. AUTO (key 47, code 0x2c)
        // is bits 145..147: green only. Dim channels stay dark.
        deck.set_face(0, Some(&face([255, 100, 200]))).unwrap();
        deck.set_face(47, Some(&face([0, 128, 0]))).unwrap();
        deck.set_deck_light(Some([255, 255, 255])).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 1, "the ATEM has no jog-mode report");
        let report = &writes[0];
        assert_eq!(report.len(), 32);
        assert_eq!(&report[..4], &[0x02, 0xff, 0xff, 0b101]);
        // Bit 146 is byte 1 + 18, mask 1 << 2.
        assert_eq!(report[19], 0b100);
        assert_eq!(report.iter().skip(4).filter(|byte| **byte != 0).count(), 1);
    }

    #[test]
    fn every_panel_matches_its_catalog_row() {
        for id in [
            "blackmagic-speed-editor",
            "blackmagic-replay-editor",
            "blackmagic-atem-micro-panel",
        ] {
            let model = by_id(id).unwrap();
            let panel = panel(id).unwrap();
            assert_eq!(panel.keys.len(), usize::from(model.keys), "{id}");
            let mut codes = std::collections::HashSet::new();
            let mut cells = std::collections::HashSet::new();
            let mut order = Vec::new();
            for key in panel.keys {
                assert!(codes.insert(key.code), "{id} repeats {:#x}", key.code);
                assert!(key.row < model.rows && key.col < model.cols, "{id}");
                assert!(cells.insert((key.row, key.col)), "{id}");
                order.push((key.row, key.col));
                if let Led::Main(bit) | Led::Rgb(bit) = key.led {
                    assert!(1 + usize::from(bit) / 8 < panel.main_len, "{id}");
                }
            }
            let mut sorted = order.clone();
            sorted.sort_unstable();
            assert_eq!(order, sorted, "{id} keys run row by row");
        }
        let lit = |id: &str| {
            panel(id)
                .unwrap()
                .keys
                .iter()
                .filter(|key| key.led != Led::None)
                .count()
        };
        assert_eq!(lit("blackmagic-speed-editor"), 21);
        assert_eq!(lit("blackmagic-replay-editor"), 46);
        assert_eq!(lit("blackmagic-atem-micro-panel"), 50);
    }
}
