//! PrehKeyTec programmable POS keyboards (MCI and MSI): keys only, nothing to
//! draw or light.
//!
//! Ryu reads the keyboard's separate POSKey collection (usage page 0xFFFA),
//! never its ordinary keyboard interface. Each key must first be programmed in
//! PrehKeyTec's own tool as a unique `POSKey001` to `POSKey128` macro with the
//! OPOS/JavaPOS POSKey HID output turned on; until then the collection is silent.
//!
//! Every input report has id 0x04 and four data bytes. With the trailing zero
//! padding dropped, the data is the key's token, or 0xFE and the token for a
//! release. A one-byte token is one of [`SINGLE_CODES`] (POSKey 1 to 77), and
//! 0xE0 with one of [`EXTENDED_CODES`] is POSKey 78 to 128; the codes are
//! scan-code shaped, so a code's value is not its key's number. Presses of a
//! key already down and releases of one already up are dropped.
//!
//! POSKey 1 is the bottom-left key, numbered left to right and then upwards, so
//! the grid is flipped to Ryu's top-left-first keys. The MCI 3100's POS section
//! is sparse: 22 keys along the bottom row and four at the right of the top row.
//!
//! The only thing ever written is the identity query ([`write_identity_query`]):
//! four two-byte reports `04 EF`, `04 50`, `04 EF`, `04 10`. The answer comes
//! back on the same 0x04 reports, framed by 0x86 and 0x96, as Latin-1 lines
//! whose `ID` line names the model (several share a product id) and whose `SN`
//! line is the keyboard's serial. Neither framing byte is ever part of a key
//! token, so an answer that arrives late is skipped rather than read as keys.

use std::collections::BTreeSet;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::{Driver, DriverError, DriverResult, Face, Input, Transport};
use crate::catalog::{by_id, DeviceModel};

/// The report id of every POSKey report, keys and identity answers alike.
pub const REPORT_ID: u8 = 0x04;
/// Leads a release, before the released key's token.
const RELEASE: u8 = 0xfe;
/// Leads a two-byte (extended) token.
const EXTENDED: u8 = 0xe0;
/// The single-byte tokens, in POSKey order: the first is POSKey 1.
pub const SINGLE_CODES: [u8; 77] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
    0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1e, 0x1f, 0x20, 0x21,
    0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32,
    0x33, 0x34, 0x35, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f, 0x40, 0x41, 0x42, 0x43, 0x44, 0x47, 0x48, 0x49,
    0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x52, 0x53, 0x56, 0x57, 0x58,
];
/// The second bytes of the 0xE0 tokens, in POSKey order from POSKey 78.
pub const EXTENDED_CODES: [u8; 51] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x11,
    0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x1a, 0x1b, 0x1e, 0x1f, 0x23, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2b, 0x2c, 0x2d, 0x2f, 0x31, 0x32, 0x34, 0x39, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f, 0x40, 0x41,
    0x42, 0x43, 0x44,
];
/// The highest POSKey number.
pub const POSKEYS: u8 = 128;

/// The identity query's bytes, each sent as its own report after [`REPORT_ID`].
pub const IDENTITY_QUERY: [u8; 4] = [0xef, 0x50, 0xef, 0x10];
const IDENTITY_START: u8 = 0x86;
const IDENTITY_END: u8 = 0x96;
/// How long the reference waits for the identity answer.
pub const IDENTITY_TIMEOUT: Duration = Duration::from_secs(2);
/// The product codes Ryu knows, longest first where one starts another.
const PRODUCT_CODES: [&str; 8] = ["128A", "128", "3100", "3000", "96", "84", "60", "30"];

/// The MCI 3100's sparse POS section.
pub const MCI_3100_ID: &str = "prehkeytec-mci-3100";
/// POSKeys 23 to 26 sit at the right of the MCI 3100's top row, from this column.
const MCI_3100_TOP_FROM: u16 = 18;
/// The MCI 3100's keys along its bottom row (POSKey 1 to 22).
const MCI_3100_BOTTOM: u8 = 22;

const READ_BUFFER: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 32;

/// One key going down or up, by POSKey number (1 to 128).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PosKeyEvent {
    pub poskey: u8,
    pub pressed: bool,
}

/// Decodes one input report into a POSKey press or release. Anything else
/// (another report id, an empty or unknown token) is `None`.
#[must_use]
pub fn decode_report(report: &[u8]) -> Option<PosKeyEvent> {
    if report.first() != Some(&REPORT_ID) {
        return None;
    }
    let data = report.get(1..)?;
    let used = data
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |at| at + 1);
    let payload = data.get(..used)?;
    let (pressed, token) = match payload.split_first() {
        None => return None,
        Some((&RELEASE, rest)) => (false, rest),
        Some(_) => (true, payload),
    };
    decode_token(token).map(|poskey| PosKeyEvent { poskey, pressed })
}

/// The POSKey number a token stands for.
#[must_use]
pub fn decode_token(token: &[u8]) -> Option<u8> {
    let number = match token {
        [code] => SINGLE_CODES.iter().position(|known| known == code)? + 1,
        [EXTENDED, code] => {
            EXTENDED_CODES.iter().position(|known| known == code)? + SINGLE_CODES.len() + 1
        }
        _ => return None,
    };
    u8::try_from(number).ok()
}

/// Whether a model is the MCI 3100, whose keys do not fill its grid.
fn is_sparse(model: &DeviceModel) -> bool {
    model.id == MCI_3100_ID
}

/// The Ryu key (top-left first, row by row) a POSKey is on this model.
#[must_use]
pub fn key_for_poskey(model: &DeviceModel, poskey: u8) -> Option<u8> {
    if poskey == 0 || poskey > model.keys {
        return None;
    }
    if is_sparse(model) {
        // Ryu numbers the four top-row keys first, then the bottom row.
        let top = model.keys - MCI_3100_BOTTOM;
        return Some(if poskey > MCI_3100_BOTTOM {
            poskey - MCI_3100_BOTTOM - 1
        } else {
            poskey - 1 + top
        });
    }
    let cols = u16::from(model.cols);
    let rows = u16::from(model.rows);
    let from_bottom = u16::from(poskey - 1);
    let row = rows.checked_sub(1 + from_bottom / cols.max(1))?;
    let col = from_bottom % cols.max(1);
    u8::try_from(row * cols + col).ok()
}

/// The POSKey a Ryu key is on this model.
#[must_use]
pub fn poskey_for_key(model: &DeviceModel, key: u8) -> Option<u8> {
    (1..=model.keys.min(POSKEYS)).find(|poskey| key_for_poskey(model, *poskey) == Some(key))
}

/// Each key's cell on the panel, for the MCI 3100 whose keys leave gaps.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    if !is_sparse(model) {
        return None;
    }
    let cols = u16::from(model.cols);
    let top = u16::from(model.keys - MCI_3100_BOTTOM);
    let top_row = (0..top).map(|at| MCI_3100_TOP_FROM + at);
    let bottom_row = (0..u16::from(MCI_3100_BOTTOM)).map(|at| cols + at);
    Some(top_row.chain(bottom_row).collect())
}

/// "POSKey 1" to "POSKey 128", made once.
fn poskey_names() -> &'static [&'static str] {
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        (1..=POSKEYS)
            .map(|poskey| &*Box::leak(format!("POSKey {poskey}").into_boxed_str()))
            .collect()
    })
}

/// Each key's name, by the POSKey number the vendor tool programs into it.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Vec<&'static str> {
    (0..model.keys)
        .map(|key| {
            poskey_for_key(model, key)
                .and_then(|poskey| poskey_names().get(usize::from(poskey - 1)).copied())
                .unwrap_or("POSKey")
        })
        .collect()
}

/// What a keyboard says it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The `ID` line, such as `MCI 128C2N10E1I2 SL-578`.
    pub product_code: String,
    /// The `SN` line, empty when there is none.
    pub serial: String,
}

/// Collects an identity answer from reports of either firmware: one data byte
/// per report, zero padded, or four.
#[derive(Debug, Default)]
pub struct IdentityReader {
    receiving: bool,
    bytes: Vec<u8>,
}

impl IdentityReader {
    /// Takes one report; once the answer ends, its text.
    pub fn push(&mut self, report: &[u8]) -> Option<String> {
        if report.first() != Some(&REPORT_ID) {
            return None;
        }
        for byte in report.iter().skip(1).copied() {
            if !self.receiving {
                self.receiving = byte == IDENTITY_START;
                continue;
            }
            if byte == IDENTITY_END {
                // Latin-1: every byte is the code point of the same value.
                let text: String = self.bytes.iter().map(|byte| char::from(*byte)).collect();
                return Some(text);
            }
            if byte != 0 {
                self.bytes.push(byte);
            }
        }
        None
    }
}

/// The value of the first line that is `key`, whitespace, then something.
fn line_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let head = line.get(..key.len())?;
        let rest = line.get(key.len()..)?;
        if !head.eq_ignore_ascii_case(key) || !rest.starts_with(char::is_whitespace) {
            return None;
        }
        Some(rest.trim()).filter(|value| !value.is_empty())
    })
}

/// Reads the identity text: `None` without an `ID` line.
#[must_use]
pub fn parse_identity(text: &str) -> Option<Identity> {
    let text = text.replace('\0', "");
    let text = text.trim();
    let product_code = line_value(text, "ID")?.to_owned();
    let serial = line_value(text, "SN").unwrap_or_default().to_owned();
    Some(Identity {
        product_code,
        serial,
    })
}

/// The model a product code names, such as `MCI 30C1N14L071E2M3I2`.
#[must_use]
pub fn model_id_for_product_code(code: &str) -> Option<&'static str> {
    let code = code.trim_start();
    let family = code.get(..3)?;
    let rest = code.get(3..)?.trim_start();
    let rest = rest.strip_prefix('-').unwrap_or(rest).trim_start();
    let number = PRODUCT_CODES.iter().find(|number| {
        rest.get(..number.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(number))
            && !rest
                .get(number.len()..)
                .and_then(|after| after.chars().next())
                .is_some_and(|next| next.is_ascii_digit())
    })?;
    if family.eq_ignore_ascii_case("MSI") {
        return (*number == "60").then_some("prehkeytec-msi-60");
    }
    if !family.eq_ignore_ascii_case("MCI") {
        return None;
    }
    Some(match *number {
        "128A" => "prehkeytec-mci-128a",
        "128" => "prehkeytec-mci-128",
        "3100" => MCI_3100_ID,
        "3000" => "prehkeytec-mci-3000",
        "96" => "prehkeytec-mci-96",
        "84" => "prehkeytec-mci-84",
        "60" => "prehkeytec-mci-60",
        _ => "prehkeytec-mci-30",
    })
}

/// The listed model a product code names.
#[must_use]
pub fn model_for_identity(identity: &Identity) -> Option<&'static DeviceModel> {
    model_id_for_product_code(&identity.product_code).and_then(by_id)
}

/// Sends the identity query: one two-byte report per byte.
pub fn write_identity_query(transport: &mut dyn Transport) -> DriverResult<()> {
    for byte in IDENTITY_QUERY {
        transport.write(&[REPORT_ID, byte])?;
    }
    Ok(())
}

/// Asks the keyboard what it is and waits up to `timeout` for the answer.
/// `Ok(None)` when it did not answer, or answered without an `ID` line; key
/// reports that arrive meanwhile are dropped.
pub fn query_identity(
    transport: &mut dyn Transport,
    timeout: Duration,
) -> DriverResult<Option<Identity>> {
    write_identity_query(transport)?;
    let deadline = Instant::now() + timeout;
    let mut reader = IdentityReader::default();
    let mut buf = [0u8; READ_BUFFER];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(None);
        }
        let wait = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
        let read = transport.read_timeout(&mut buf, wait)?;
        if let Some(text) = reader.push(buf.get(..read).unwrap_or_default()) {
            return Ok(parse_identity(&text));
        }
    }
}

pub struct PrehKeyTec {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    /// POSKeys down, so repeats and stray releases are dropped.
    held: BTreeSet<u8>,
    /// Inside an identity answer, which is skipped up to its end.
    in_identity: bool,
    /// A read that failed while keys were down: their releases go out first,
    /// and the failure on the next poll.
    failed: Option<DriverError>,
}

impl PrehKeyTec {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            model,
            transport,
            held: BTreeSet::new(),
            in_identity: false,
            failed: None,
        }
    }

    /// Whether a report belongs to an identity answer, tracking where one
    /// starts and ends. Neither framing byte is ever a key token.
    fn skips_identity(&mut self, report: &[u8]) -> bool {
        if report.first() != Some(&REPORT_ID) {
            return false;
        }
        let mut skipped = self.in_identity;
        for byte in report.iter().skip(1) {
            if *byte == IDENTITY_START {
                self.in_identity = true;
                skipped = true;
            } else if *byte == IDENTITY_END {
                self.in_identity = false;
                skipped = true;
            }
        }
        skipped
    }

    fn parse(&mut self, report: &[u8]) -> Option<Input> {
        if self.skips_identity(report) {
            return None;
        }
        let event = decode_report(report)?;
        let changed = if event.pressed {
            self.held.insert(event.poskey)
        } else {
            self.held.remove(&event.poskey)
        };
        if !changed {
            return None;
        }
        let index = key_for_poskey(self.model, event.poskey)?;
        Some(Input::Key {
            index,
            pressed: event.pressed,
        })
    }

    /// Releases every key still down.
    fn release_all(&mut self) -> Vec<Input> {
        std::mem::take(&mut self.held)
            .into_iter()
            .filter_map(|poskey| key_for_poskey(self.model, poskey))
            .map(|index| Input::Key {
                index,
                pressed: false,
            })
            .collect()
    }
}

impl Driver for PrehKeyTec {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if let Some(error) = self.failed.take() {
            return Err(error);
        }
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = match self.transport.read_timeout(&mut buf, wait) {
                Ok(read) => read,
                Err(error) => {
                    let mut released = self.release_all();
                    if events.is_empty() && released.is_empty() {
                        return Err(error);
                    }
                    self.failed = Some(error);
                    events.append(&mut released);
                    return Ok(events);
                }
            };
            if read == 0 {
                break;
            }
            events.extend(self.parse(buf.get(..read).unwrap_or_default()));
        }
        Ok(events)
    }

    /// No screens and no lights: a layout only names the keys in the Companion.
    fn set_face(&mut self, _key: u8, _face: Option<&Face>) -> DriverResult<()> {
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        Ok(())
    }

    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::testing::Recorder;

    fn keyboard(id: &str) -> (PrehKeyTec, Recorder) {
        let recorder = Recorder::default();
        (
            PrehKeyTec::new(by_id(id).unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    fn down(poskey: u8) -> Option<PosKeyEvent> {
        Some(PosKeyEvent {
            poskey,
            pressed: true,
        })
    }

    fn up(poskey: u8) -> Option<PosKeyEvent> {
        Some(PosKeyEvent {
            poskey,
            pressed: false,
        })
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    #[test]
    fn the_captured_reports_decode_to_their_poskeys() {
        assert_eq!(decode_report(&[0x04, 0x01, 0x00, 0x00, 0x00]), down(1));
        assert_eq!(decode_report(&[0x04, 0xfe, 0x01, 0x00, 0x00]), up(1));
        assert_eq!(decode_report(&[0x04, 0x0d, 0x00, 0x00, 0x00]), down(13));
        assert_eq!(decode_report(&[0x04, 0x52]), down(73));
        assert_eq!(decode_report(&[0x04, 0x1f, 0x00, 0x00, 0x00]), down(30));
        assert_eq!(decode_report(&[0x04, 0xfe, 0x1f, 0x00, 0x00]), up(30));
        assert_eq!(decode_report(&[0x04, 0xe0, 0x07, 0x00, 0x00]), down(84));
        assert_eq!(decode_report(&[0x04, 0xfe, 0xe0, 0x07, 0x00]), up(84));
        assert_eq!(decode_report(&[0x04, 0xe0, 0x44, 0x00, 0x00]), down(128));
        assert_eq!(decode_report(&[0x04, 0x58]), down(77));
        assert_eq!(decode_report(&[0x04, 0xe0, 0x01]), down(78));
    }

    #[test]
    fn other_reports_and_unknown_tokens_are_dropped() {
        for report in [
            &[0x03, 0x01, 0x00, 0x00, 0x00][..],
            &[0x04, 0x00, 0x00, 0x00, 0x00],
            &[0x04, 0xe0, 0x10, 0x00, 0x00],
            &[0x04, 0xfe, 0x00, 0x00, 0x00],
            &[0x04, 0x1d],
            &[0x04, 0x01, 0x00, 0x02],
            &[0x04],
            &[],
        ] {
            assert_eq!(decode_report(report), None, "{report:02x?}");
        }
    }

    #[test]
    fn every_code_is_one_poskey() {
        let mut seen = BTreeSet::new();
        for code in SINGLE_CODES {
            assert!(seen.insert(decode_token(&[code]).unwrap()));
        }
        for code in EXTENDED_CODES {
            assert!(seen.insert(decode_token(&[EXTENDED, code]).unwrap()));
        }
        assert_eq!(seen, (1..=POSKEYS).collect());
    }

    #[test]
    fn poskey_one_is_the_bottom_left_key() {
        let mci84 = by_id("prehkeytec-mci-84").unwrap();
        // 7 x 12: POSKey 1 is row 6, column 0; POSKey 84 is row 0, column 11.
        assert_eq!(key_for_poskey(mci84, 1), Some(72));
        assert_eq!(key_for_poskey(mci84, 12), Some(83));
        assert_eq!(key_for_poskey(mci84, 13), Some(60));
        assert_eq!(key_for_poskey(mci84, 73), Some(0));
        assert_eq!(key_for_poskey(mci84, 84), Some(11));
        assert_eq!(key_for_poskey(mci84, 85), None);
        assert_eq!(key_for_poskey(mci84, 0), None);
        let mci30 = by_id("prehkeytec-mci-30").unwrap();
        assert_eq!(key_for_poskey(mci30, 1), Some(25));
        assert_eq!(key_for_poskey(mci30, 30), Some(4));
        let mci128 = by_id("prehkeytec-mci-128").unwrap();
        assert_eq!(key_for_poskey(mci128, 1), Some(112));
        assert_eq!(key_for_poskey(mci128, 128), Some(15));
        assert_eq!(poskey_for_key(mci128, 0), Some(113));
        assert_eq!(key_names(mci84)[72], "POSKey 1");
        assert_eq!(key_names(mci84)[0], "POSKey 73");
        assert_eq!(key_cells(mci84), None);
    }

    #[test]
    fn the_mci_3100_fills_its_bottom_row_and_four_top_right_keys() {
        let mci3100 = by_id(MCI_3100_ID).unwrap();
        let cells = key_cells(mci3100).unwrap();
        assert_eq!(cells.len(), 26);
        assert_eq!(&cells[..4], &[18, 19, 20, 21]);
        assert_eq!(cells[4], 22);
        assert_eq!(cells[25], 43);
        // POSKey 1 to 22: row 1, columns 0 to 21.
        assert_eq!(key_for_poskey(mci3100, 1), Some(4));
        assert_eq!(key_for_poskey(mci3100, 22), Some(25));
        // POSKey 23 to 26: row 0, columns 18 to 21.
        assert_eq!(key_for_poskey(mci3100, 23), Some(0));
        assert_eq!(key_for_poskey(mci3100, 26), Some(3));
        assert_eq!(key_for_poskey(mci3100, 27), None);
        let names = key_names(mci3100);
        assert_eq!(
            (names[0], names[4], names[25]),
            ("POSKey 23", "POSKey 1", "POSKey 22")
        );
    }

    #[test]
    fn presses_and_releases_are_raised_once_each() {
        let (mut deck, recorder) = keyboard("prehkeytec-mci-84");
        recorder.queue_read(vec![0x04, 0x01, 0x00, 0x00, 0x00]);
        // Held: a repeat is dropped.
        recorder.queue_read(vec![0x04, 0x01, 0x00, 0x00, 0x00]);
        recorder.queue_read(vec![0x04, 0xe0, 0x07, 0x00, 0x00]);
        recorder.queue_read(vec![0x04, 0xfe, 0x01, 0x00, 0x00]);
        // Not held: dropped.
        recorder.queue_read(vec![0x04, 0xfe, 0x01, 0x00, 0x00]);
        recorder.queue_read(vec![0x04, 0xfe, 0xe0, 0x07, 0x00]);
        // Beyond the 84 keys: decoded but on no key.
        recorder.queue_read(vec![0x04, 0xe0, 0x44, 0x00, 0x00]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![key(72, true), key(11, true), key(72, false), key(11, false)]
        );
        assert!(recorder.writes().is_empty(), "keys need no writes");
    }

    #[test]
    fn an_identity_answer_on_the_key_stream_is_skipped() {
        let (mut deck, recorder) = keyboard("prehkeytec-mci-84");
        // A one-byte-per-report answer: "ID 0" would otherwise read as keys.
        for byte in [0x86, b'I', b'D', b' ', 0x0b, 0x96] {
            recorder.queue_read(vec![0x04, byte, 0x00, 0x00, 0x00]);
        }
        recorder.queue_read(vec![0x04, 0x02, 0x00, 0x00, 0x00]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(73, true)]);
    }

    #[test]
    fn a_failed_read_releases_held_keys_first() {
        struct Failing(Recorder);
        impl Transport for Failing {
            fn write(&mut self, data: &[u8]) -> DriverResult<()> {
                self.0.write(data)
            }
            fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> DriverResult<usize> {
                match self.0.read_timeout(buf, timeout_ms)? {
                    0 => Err(DriverError::Io("unplugged".to_owned())),
                    read => Ok(read),
                }
            }
            fn send_feature_report(&mut self, data: &[u8]) -> DriverResult<()> {
                self.0.send_feature_report(data)
            }
            fn get_feature_report(&mut self, buf: &mut [u8]) -> DriverResult<usize> {
                self.0.get_feature_report(buf)
            }
        }
        let recorder = Recorder::default();
        let mut deck = PrehKeyTec::new(
            by_id("prehkeytec-mci-30").unwrap(),
            Box::new(Failing(recorder.clone())),
        );
        recorder.queue_read(vec![0x04, 0x01]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(25, true), key(25, false)]);
        assert!(deck.poll(0).is_err());
        assert!(
            deck.poll(0).is_err(),
            "nothing held: the error comes straight"
        );
    }

    #[test]
    fn faces_and_brightness_write_nothing() {
        let (mut deck, recorder) = keyboard("prehkeytec-mci-128");
        deck.set_face(0, None).unwrap();
        deck.set_brightness(40).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty());
        assert!(recorder.features().is_empty());
    }

    fn framed(text: &str, per_report: usize) -> Vec<Vec<u8>> {
        let mut bytes = vec![0x86];
        bytes.extend(text.chars().map(|c| u8::try_from(u32::from(c)).unwrap()));
        bytes.push(0x96);
        bytes
            .chunks(per_report)
            .map(|chunk| {
                let mut report = vec![0x04, 0, 0, 0, 0];
                report[1..=chunk.len()].copy_from_slice(chunk);
                report
            })
            .collect()
    }

    const MCI_128_ANSWER: &str = "(C) 1990 - 2019 by PrehKeyTec GmbH\n05129-605/5040 - MCI 128 Num\nJul 15 2019 14:04:00\nBL 3.30\nID MCI 128C2N10E1I2 SL-578\nHW 90328-606/1805\nSN 24342-1903971";

    #[test]
    fn the_identity_query_is_four_two_byte_reports() {
        let mut recorder = Recorder::default();
        for report in framed(MCI_128_ANSWER, 4) {
            recorder.queue_read(report);
        }
        let identity = query_identity(&mut recorder, IDENTITY_TIMEOUT)
            .unwrap()
            .unwrap();
        assert_eq!(
            recorder.writes(),
            vec![
                vec![0x04, 0xef],
                vec![0x04, 0x50],
                vec![0x04, 0xef],
                vec![0x04, 0x10]
            ]
        );
        assert_eq!(identity.product_code, "MCI 128C2N10E1I2 SL-578");
        assert_eq!(identity.serial, "24342-1903971");
        assert_eq!(
            model_for_identity(&identity).unwrap().id,
            "prehkeytec-mci-128"
        );
    }

    #[test]
    fn older_firmware_answers_one_byte_per_report() {
        let mut reader = IdentityReader::default();
        let mut text = None;
        // A key report before the answer starts is not part of it.
        reader.push(&[0x04, 0x01, 0x00, 0x00, 0x00]);
        for report in framed("ID MCI 30C1N14L071E2M3I2\nSN 1067562", 1) {
            text = text.or(reader.push(&report));
        }
        let identity = parse_identity(&text.unwrap()).unwrap();
        assert_eq!(
            identity,
            Identity {
                product_code: "MCI 30C1N14L071E2M3I2".to_owned(),
                serial: "1067562".to_owned(),
            }
        );
        assert_eq!(
            model_for_identity(&identity).unwrap().id,
            "prehkeytec-mci-30"
        );
    }

    #[test]
    fn no_answer_or_no_id_line_is_none() {
        let mut recorder = Recorder::default();
        assert_eq!(
            query_identity(&mut recorder, Duration::from_millis(5)).unwrap(),
            None
        );
        assert_eq!(parse_identity("BL 3.30\nSN 1"), None);
        assert_eq!(
            parse_identity("ID MCI 84C1").unwrap().serial,
            "",
            "no SN line"
        );
    }

    #[test]
    fn product_codes_name_their_models() {
        for (code, id) in [
            ("MCI 30C1N14L071E2M3I2", Some("prehkeytec-mci-30")),
            ("MCI 84C1", Some("prehkeytec-mci-84")),
            ("MCI 128C2N10E1I2 SL-578", Some("prehkeytec-mci-128")),
            ("MCI128A-example", Some("prehkeytec-mci-128a")),
            ("mci-96", Some("prehkeytec-mci-96")),
            ("  MCI - 3100", Some(MCI_3100_ID)),
            ("MCI 3000", Some("prehkeytec-mci-3000")),
            ("MCI 60", Some("prehkeytec-mci-60")),
            ("MSI 60", Some("prehkeytec-msi-60")),
            ("MSI 84", None),
            ("MCI 300", None),
            ("MCI 845", None),
            ("unknown", None),
            ("", None),
        ] {
            assert_eq!(model_id_for_product_code(code), id, "{code}");
        }
        for id in [
            "prehkeytec-mci-30",
            "prehkeytec-mci-60",
            "prehkeytec-mci-84",
            "prehkeytec-mci-96",
            "prehkeytec-mci-128",
            "prehkeytec-mci-128a",
            "prehkeytec-mci-3000",
            MCI_3100_ID,
            "prehkeytec-msi-60",
        ] {
            assert!(by_id(id).is_some(), "{id} is listed");
        }
    }
}
