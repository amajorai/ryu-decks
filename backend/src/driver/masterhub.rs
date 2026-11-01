//! Cooler Master MasterHub: a modular deck whose every module is a USB HID
//! device of its own on vendor id 0x2516, so each module is a deck in Ryu.
//!
//! Every module reports through one envelope: report id `0x01`, a channel
//! byte, a little-endian 16-bit payload length, then the payload. A report
//! that is shorter than its length says, or on the wrong channel, is dropped.
//!
//! - Keypad M4315 (channel `0x34`): `02 0f` then one byte per key, 15 keys in
//!   reading order, five to a row; only `0x01` is down. Each report is the
//!   whole state, so Ryu raises only changes. Its faces are 72 x 72 baseline
//!   JPEGs, sent in 320-byte output reports `02 36 chunk(le16) len(le16) key
//!   last` with up to 312 bytes each, the last chunk committing the image. It
//!   ignores the host until the "software mode" feature report `03 3a 01`
//!   arrives; brightness is feature `03 34 pp` (0 to 100).
//! - Knob M003K (channel `0x2a`): turns are `00 03` and one byte per knob
//!   (`01` clockwise, `00` counter-clockwise, anything else still), presses
//!   `03 03` and one byte per knob (`01` down). The report order is not the
//!   knob order: report byte 0 is knob 1, byte 1 knob 2 and byte 2 knob 0.
//! - Slider M005S (channel `0x2a`): `02 05` (some firmware sends `00 05`) and
//!   five positions from 0 to 100, leftmost first. Ryu has no fader input, so
//!   each change after the first sample is a dial turn by the difference.
//! - Roller M032LE (channel `0x2a`): `01 02` and one byte per roller, `00` up
//!   (+1), `01` down (-1), `ff` still and `7f` a turn with no detent. Some
//!   frames are just the two bytes, and some carry a length that does not
//!   match, which are read from bytes 4 and 5 of the report. Report byte 0 is
//!   the right roller, so Ryu's dial 0 (left) is byte 1.
//! - 1xDial (channel `0x44`): exactly `03 01 00` (clockwise), `03 01 01`
//!   (counter-clockwise) or `04 01 b` (pushed when `b` is `01`). Its round
//!   240 x 240 screen takes the keypad's chunking with command `0x45` and slot
//!   `0x30`, after the feature report `03 49 01 00 00 00 f0 00 f0 00 01 f0 f0`.
//! - Base M0064 only sends a heartbeat (channel `0x2b`), which is ignored.
//!
//! The knob, slider and base lights are set with 64-byte output reports: mode
//! `02 2c module led mode save`, colour `02 2d module led r g b save` and
//! brightness `02 2e module led pct`, where the module is `00` base, `01`
//! slider, `02` knob and led `ff` means every light. A solid colour is mode
//! static (`01`) then the colour, never saved to flash. Knob lights are
//! numbered differently again: knob 0 is light 2, knob 1 light 0, knob 2
//! light 1. Feature reports are 32 bytes. Nothing is acknowledged and no
//! keep-alive is needed.

use std::collections::BTreeMap;

use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageEncoder};

use super::{face, Driver, DriverError, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

pub const BASE_PRODUCT_ID: u16 = 0x01f6;
pub const KEYPAD_PRODUCT_ID: u16 = 0x01fe;
pub const SLIDER_PRODUCT_ID: u16 = 0x0200;
pub const KNOB_PRODUCT_ID: u16 = 0x0202;
pub const ROLLER_PRODUCT_ID: u16 = 0x0204;
pub const DIAL_PRODUCT_ID: u16 = 0x0206;

const READ_BUFFER: usize = 512;
const MAX_REPORTS_PER_POLL: usize = 32;

const INPUT_REPORT: u8 = 0x01;
const KEYPAD_CHANNEL: u8 = 0x34;
const M0_CHANNEL: u8 = 0x2a;
const DIAL_CHANNEL: u8 = 0x44;

const KEYS: usize = 15;
const KNOBS: usize = 3;
const SLIDERS: usize = 5;
const ROLLERS: usize = 2;

const IMAGE_REPORT_LEN: usize = 320;
const IMAGE_HEADER_LEN: usize = 8;
const IMAGE_CHUNK_MAX: usize = IMAGE_REPORT_LEN - IMAGE_HEADER_LEN;
const KEY_IMAGE_COMMAND: u8 = 0x36;
const DIAL_IMAGE_COMMAND: u8 = 0x45;
const DIAL_SCREEN_SLOT: u8 = 0x30;
/// The largest JPEG either screen accepts.
const JPEG_MAX: usize = 64 * 1024;

const FEATURE_LEN: usize = 32;
const LED_REPORT_LEN: usize = 64;
const LED_MODE: u8 = 0x2c;
const LED_COLOR: u8 = 0x2d;
const LED_BRIGHTNESS: u8 = 0x2e;
const LED_MODE_STATIC: u8 = 0x01;
const LED_ALL: u8 = 0xff;
const LED_MODULE_BASE: u8 = 0x00;
const LED_MODULE_SLIDER: u8 = 0x01;
const LED_MODULE_KNOB: u8 = 0x02;

/// The logical knob each report position names.
const KNOB_OF_REPORT: [u8; KNOBS] = [1, 2, 0];
/// The light each logical knob carries.
const LIGHT_OF_KNOB: [u8; KNOBS] = [2, 0, 1];
/// Ryu's dials run left to right; report byte 0 is the right roller.
const DIAL_OF_ROLLER: [u8; ROLLERS] = [1, 0];

/// Which MasterHub module a model is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    Base,
    Keypad,
    Slider,
    Knob,
    Roller,
    Dial,
}

impl Module {
    /// The module a catalog row describes, by its product id. A row this
    /// driver does not know is treated as the base: nothing to read or draw.
    #[must_use]
    pub fn of(model: &DeviceModel) -> Self {
        match model.product_ids.first().copied() {
            Some(KEYPAD_PRODUCT_ID) => Self::Keypad,
            Some(SLIDER_PRODUCT_ID) => Self::Slider,
            Some(KNOB_PRODUCT_ID) => Self::Knob,
            Some(ROLLER_PRODUCT_ID) => Self::Roller,
            Some(DIAL_PRODUCT_ID) => Self::Dial,
            _ => Self::Base,
        }
    }

    /// The LED module id M0 light reports address, for modules with lights.
    const fn led_module(self) -> Option<u8> {
        match self {
            Self::Base => Some(LED_MODULE_BASE),
            Self::Slider => Some(LED_MODULE_SLIDER),
            Self::Knob => Some(LED_MODULE_KNOB),
            _ => None,
        }
    }
}

pub struct MasterHub {
    model: &'static DeviceModel,
    module: Module,
    transport: Box<dyn Transport>,
    /// Whether the software-mode feature report has gone out.
    ready: bool,
    keys_down: [bool; KEYS],
    knobs_down: [bool; KNOBS],
    dial_down: bool,
    /// The last fader positions; `None` until the first sample.
    sliders: Option<[u8; SLIDERS]>,
    /// Encoded faces waiting for [`Driver::flush`], by key (or the dial screen).
    faces: BTreeMap<u8, Vec<u8>>,
    /// The frame the dial shows, so an identical one is not sent again.
    dial_frame: Option<Vec<u8>>,
    /// Light colours waiting for [`Driver::flush`], by light index.
    lights: BTreeMap<u8, [u8; 3]>,
}

impl MasterHub {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            model,
            module: Module::of(model),
            transport,
            ready: false,
            keys_down: [false; KEYS],
            knobs_down: [false; KNOBS],
            dial_down: false,
            sliders: None,
            faces: BTreeMap::new(),
            dial_frame: None,
            lights: BTreeMap::new(),
        }
    }

    /// Puts the keypad and the dial in software mode, once per open.
    fn ensure_ready(&mut self) -> DriverResult<()> {
        if self.ready {
            return Ok(());
        }
        match self.module {
            Module::Keypad => self
                .transport
                .send_feature_report(&keypad_software_mode())?,
            Module::Dial => self.transport.send_feature_report(&dial_software_mode())?,
            _ => {}
        }
        self.ready = true;
        Ok(())
    }

    fn parse(&mut self, report: &[u8]) -> Vec<Input> {
        match self.module {
            Module::Keypad => self.parse_keypad(report),
            Module::Knob => self.parse_knob(report),
            Module::Slider => self.parse_slider(report),
            Module::Roller => parse_roller(report),
            Module::Dial => self.parse_dial(report),
            // The base's only report is a heartbeat.
            Module::Base => Vec::new(),
        }
    }

    fn parse_keypad(&mut self, report: &[u8]) -> Vec<Input> {
        let Some(states) = payload(report, KEYPAD_CHANNEL)
            .filter(|body| body.first() == Some(&0x02) && body.get(1) == Some(&(KEYS as u8)))
            .and_then(|body| body.get(2..2 + KEYS))
        else {
            return Vec::new();
        };
        let mut events = Vec::new();
        for (index, (down, state)) in self.keys_down.iter_mut().zip(states).enumerate() {
            let pressed = *state == 0x01;
            if pressed != *down {
                *down = pressed;
                events.push(Input::Key {
                    index: index as u8,
                    pressed,
                });
            }
        }
        events
    }

    fn parse_knob(&mut self, report: &[u8]) -> Vec<Input> {
        let Some(body) = payload(report, M0_CHANNEL) else {
            return Vec::new();
        };
        if let Some(turns) = sub_report(body, 0x00, KNOBS) {
            return turns
                .iter()
                .zip(KNOB_OF_REPORT)
                .filter_map(|(byte, index)| {
                    let delta = match byte {
                        0x01 => 1,
                        0x00 => -1,
                        _ => return None,
                    };
                    Some((index, delta))
                })
                .map(|(index, delta)| Input::DialTurn { index, delta })
                .collect();
        }
        let Some(presses) = sub_report(body, 0x03, KNOBS) else {
            return Vec::new();
        };
        let mut state = [false; KNOBS];
        for (byte, knob) in presses.iter().zip(KNOB_OF_REPORT) {
            state[usize::from(knob)] = *byte == 0x01;
        }
        let mut events = Vec::new();
        for (index, (down, pressed)) in self.knobs_down.iter_mut().zip(state).enumerate() {
            if pressed != *down {
                *down = pressed;
                events.push(Input::DialPress {
                    index: index as u8,
                    pressed,
                });
            }
        }
        events
    }

    fn parse_slider(&mut self, report: &[u8]) -> Vec<Input> {
        let Some(values) = payload(report, M0_CHANNEL).and_then(|body| {
            sub_report(body, 0x02, SLIDERS).or_else(|| sub_report(body, 0x00, SLIDERS))
        }) else {
            return Vec::new();
        };
        if values.iter().any(|value| *value > 100) {
            return Vec::new();
        }
        let mut now = [0u8; SLIDERS];
        now.copy_from_slice(values);
        let Some(before) = self.sliders.replace(now) else {
            // The first sample only says where the faders start.
            return Vec::new();
        };
        before
            .iter()
            .zip(now)
            .enumerate()
            .filter(|(_, (old, new))| *old != new)
            .map(|(index, (old, new))| Input::DialTurn {
                index: index as u8,
                // Both are 0 to 100, so the difference fits.
                delta: i8::try_from(i16::from(new) - i16::from(*old)).unwrap_or(0),
            })
            .collect()
    }

    fn parse_dial(&mut self, report: &[u8]) -> Vec<Input> {
        let Some(body) = payload(report, DIAL_CHANNEL).filter(|body| body.len() == 3) else {
            return Vec::new();
        };
        match body {
            [0x03, 0x01, 0x00] => vec![Input::DialTurn { index: 0, delta: 1 }],
            [0x03, 0x01, 0x01] => vec![Input::DialTurn {
                index: 0,
                delta: -1,
            }],
            [0x04, 0x01, state] => {
                let pressed = *state == 0x01;
                if pressed == self.dial_down {
                    return Vec::new();
                }
                self.dial_down = pressed;
                vec![Input::DialPress { index: 0, pressed }]
            }
            _ => Vec::new(),
        }
    }

    /// Encodes one screen's face as a baseline JPEG within the size limit,
    /// lowering the quality until it fits.
    fn encode(&self, face: Option<&Face>, key: u8) -> DriverResult<Option<Vec<u8>>> {
        let format = match self.module {
            Module::Keypad if usize::from(key) < KEYS => self.model.image,
            Module::Dial if self.model.wheel_key() == Some(key) => self.model.wheel_image,
            _ => None,
        };
        let Some(format) = format else {
            return Ok(None);
        };
        let picture = face::compose(face, format.width, format.height);
        let rgb = image::DynamicImage::ImageRgba8(picture).into_rgb8();
        for quality in (1..=8).rev().map(|step| step * 10) {
            let mut out = Vec::new();
            JpegEncoder::new_with_quality(&mut out, quality)
                .write_image(
                    rgb.as_raw(),
                    format.width,
                    format.height,
                    ExtendedColorType::Rgb8,
                )
                .map_err(|error| DriverError::Image(error.to_string()))?;
            if out.len() <= JPEG_MAX {
                return Ok(Some(out));
            }
        }
        Err(DriverError::Image(
            "the face does not fit in 64 KiB even at the lowest quality".to_owned(),
        ))
    }

    fn send_image(&mut self, command: u8, slot: u8, jpeg: &[u8]) -> DriverResult<()> {
        for report in image_reports(command, slot, jpeg) {
            self.transport.write(&report)?;
        }
        Ok(())
    }

    fn stage_light(&mut self, light: u8, color: Option<[u8; 3]>) {
        self.lights.insert(light, color.unwrap_or([0, 0, 0]));
    }
}

impl Driver for MasterHub {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_ready()?;
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            events.extend(self.parse(buf.get(..read).unwrap_or_default()));
        }
        Ok(events)
    }

    /// Keypad keys and the dial's screen take a face; an unbound one is black.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(jpeg) = self.encode(face, key)? {
            self.faces.insert(key, jpeg);
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_ready()?;
        for (key, jpeg) in std::mem::take(&mut self.faces) {
            match self.module {
                Module::Keypad => self.send_image(KEY_IMAGE_COMMAND, key, &jpeg)?,
                // A frame the dial already shows is not sent again.
                Module::Dial if self.dial_frame.as_deref() != Some(jpeg.as_slice()) => {
                    self.send_image(DIAL_IMAGE_COMMAND, DIAL_SCREEN_SLOT, &jpeg)?;
                    self.dial_frame = Some(jpeg);
                }
                _ => {}
            }
        }
        if let Some(module) = self.module.led_module() {
            for (light, [r, g, b]) in std::mem::take(&mut self.lights) {
                self.transport.write(&led_report(&[
                    LED_MODE,
                    module,
                    light,
                    LED_MODE_STATIC,
                    0,
                ]))?;
                self.transport
                    .write(&led_report(&[LED_COLOR, module, light, r, g, b, 0]))?;
            }
        }
        Ok(())
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.ensure_ready()?;
        let percent = percent.min(100);
        match self.module {
            Module::Keypad => {
                let mut report = [0u8; FEATURE_LEN];
                report[..3].copy_from_slice(&[0x03, 0x34, percent]);
                self.transport.send_feature_report(&report)
            }
            Module::Knob | Module::Slider => {
                let module = self.module.led_module().unwrap_or(LED_MODULE_KNOB);
                self.transport
                    .write(&led_report(&[LED_BRIGHTNESS, module, LED_ALL, percent]))
            }
            Module::Base => {
                self.transport
                    .write(&led_report(&[LED_BRIGHTNESS, LED_MODULE_BASE, 0, percent]))
            }
            // The rollers have no lights and the dial no brightness command.
            Module::Roller | Module::Dial => Ok(()),
        }
    }

    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        match self.module {
            Module::Knob => {
                if let Some(light) = LIGHT_OF_KNOB.get(usize::from(dial)) {
                    self.stage_light(*light, color);
                }
            }
            Module::Slider if usize::from(dial) < SLIDERS => self.stage_light(dial, color),
            _ => {}
        }
        Ok(())
    }

    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        if self.module == Module::Base {
            self.stage_light(0, color);
        }
        Ok(())
    }
}

/// The payload of an input report on `channel`, if the report holds all of it.
fn payload(report: &[u8], channel: u8) -> Option<&[u8]> {
    if report.first() != Some(&INPUT_REPORT) || report.get(1) != Some(&channel) {
        return None;
    }
    let length = u16::from_le_bytes([*report.get(2)?, *report.get(3)?]);
    report.get(4..4 + usize::from(length))
}

/// The values of an M0 sub-report `id count values...`.
fn sub_report(body: &[u8], id: u8, count: usize) -> Option<&[u8]> {
    if body.first() != Some(&id) || body.get(1).map(|byte| usize::from(*byte)) != Some(count) {
        return None;
    }
    body.get(2..2 + count)
}

/// A byte a roller sends: a direction, still (`ff`) or a turn with no detent (`7f`).
const fn is_roller_byte(byte: u8) -> bool {
    matches!(byte, 0x00 | 0x01 | 0x7f | 0xff)
}

/// What a sub-report on the M0 channel is, whichever module it came from.
fn is_other_m0_report(body: &[u8]) -> bool {
    sub_report(body, 0x00, KNOBS).is_some()
        || sub_report(body, 0x03, KNOBS).is_some()
        || sub_report(body, 0x02, SLIDERS).is_some()
        || sub_report(body, 0x00, SLIDERS).is_some()
}

fn parse_roller(report: &[u8]) -> Vec<Input> {
    let strict = payload(report, M0_CHANNEL);
    let bytes = match strict {
        Some(body) => {
            if let Some(turns) = sub_report(body, 0x01, ROLLERS) {
                Some(turns)
            } else if body.len() == ROLLERS && body.iter().all(|byte| is_roller_byte(*byte)) {
                Some(body)
            } else if is_other_m0_report(body) {
                // Another module's report: no fallback.
                None
            } else {
                loose_roller_bytes(report)
            }
        }
        None => loose_roller_bytes(report),
    };
    let Some(bytes) = bytes else {
        return Vec::new();
    };
    bytes
        .iter()
        .zip(DIAL_OF_ROLLER)
        .filter_map(|(byte, index)| {
            let delta = match byte {
                0x00 => 1,
                0x01 => -1,
                _ => return None,
            };
            Some(Input::DialTurn { index, delta })
        })
        .collect()
}

/// The two roller bytes straight after the envelope, for frames whose length
/// field does not describe them.
fn loose_roller_bytes(report: &[u8]) -> Option<&[u8]> {
    if report.first() != Some(&INPUT_REPORT) || report.get(1) != Some(&M0_CHANNEL) {
        return None;
    }
    report
        .get(4..6)
        .filter(|bytes| bytes.iter().all(|byte| is_roller_byte(*byte)))
}

/// The output reports that carry one JPEG to a key (or the dial's screen).
#[must_use]
pub fn image_reports(command: u8, slot: u8, jpeg: &[u8]) -> Vec<[u8; IMAGE_REPORT_LEN]> {
    let chunks = jpeg.chunks(IMAGE_CHUNK_MAX);
    let last = chunks.len().saturating_sub(1);
    chunks
        .enumerate()
        .map(|(index, chunk)| {
            let mut report = [0u8; IMAGE_REPORT_LEN];
            let number = (index as u16).to_le_bytes();
            let length = (chunk.len() as u16).to_le_bytes();
            report[..IMAGE_HEADER_LEN].copy_from_slice(&[
                0x02,
                command,
                number[0],
                number[1],
                length[0],
                length[1],
                slot,
                u8::from(index == last),
            ]);
            report[IMAGE_HEADER_LEN..IMAGE_HEADER_LEN + chunk.len()].copy_from_slice(chunk);
            report
        })
        .collect()
}

fn led_report(body: &[u8]) -> [u8; LED_REPORT_LEN] {
    let mut report = [0u8; LED_REPORT_LEN];
    report[0] = 0x02;
    report[1..=body.len()].copy_from_slice(body);
    report
}

fn keypad_software_mode() -> [u8; FEATURE_LEN] {
    let mut report = [0u8; FEATURE_LEN];
    report[..3].copy_from_slice(&[0x03, 0x3a, 0x01]);
    report
}

/// Software mode at 240 x 240.
fn dial_software_mode() -> [u8; FEATURE_LEN] {
    let mut report = [0u8; FEATURE_LEN];
    report[..13].copy_from_slice(&[
        0x03, 0x49, 0x01, 0x00, 0x00, 0x00, 0xf0, 0x00, 0xf0, 0x00, 0x01, 0xf0, 0xf0,
    ]);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn open(id: &str) -> (MasterHub, Recorder) {
        let recorder = Recorder::default();
        (
            MasterHub::new(by_id(id).unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    /// An input report on `channel` with a correct length field, padded to 64.
    fn report(channel: u8, body: &[u8]) -> Vec<u8> {
        let mut out = vec![0x01, channel, body.len() as u8, 0x00];
        out.extend_from_slice(body);
        out.resize(64, 0);
        out
    }

    fn turn(index: u8, delta: i8) -> Input {
        Input::DialTurn { index, delta }
    }

    fn press(index: u8, pressed: bool) -> Input {
        Input::DialPress { index, pressed }
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    fn keypad_report(down: &[(usize, u8)]) -> Vec<u8> {
        let mut body = vec![0x02, 0x0f];
        body.extend_from_slice(&[0u8; KEYS]);
        for (index, state) in down {
            body[2 + index] = *state;
        }
        report(0x34, &body)
    }

    #[test]
    fn modules_are_told_apart_by_product_id() {
        for (id, module) in [
            ("coolermaster-masterhub-base", Module::Base),
            ("coolermaster-masterhub-keypad", Module::Keypad),
            ("coolermaster-masterhub-slider", Module::Slider),
            ("coolermaster-masterhub-knob", Module::Knob),
            ("coolermaster-masterhub-roller", Module::Roller),
            ("coolermaster-masterhub-dial", Module::Dial),
        ] {
            assert_eq!(Module::of(by_id(id).unwrap()), module, "{id}");
        }
    }

    #[test]
    fn keypad_snapshots_raise_only_changes_and_only_01_is_down() {
        let (mut deck, recorder) = open("coolermaster-masterhub-keypad");
        recorder.queue_read(keypad_report(&[(0, 0x01), (14, 0x01)]));
        // The same state again, and 0x02 on key 7 is not a press.
        recorder.queue_read(keypad_report(&[(0, 0x01), (7, 0x02), (14, 0x01)]));
        recorder.queue_read(keypad_report(&[(14, 0x01), (6, 0x01)]));
        // Wrong channel, wrong header, and a length longer than the report.
        let mut wrong = keypad_report(&[(3, 0x01)]);
        wrong[1] = 0x2a;
        recorder.queue_read(wrong);
        recorder.queue_read(report(0x34, &[0x03, 0x0f, 0x01]));
        recorder.queue_read(vec![0x01, 0x34, 0x40, 0x00, 0x02, 0x0f, 0x01]);
        recorder.queue_read(vec![0x01]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![key(0, true), key(14, true), key(0, false), key(6, true)]
        );
        // Software mode goes out on the first poll.
        let mut mode = vec![0x03, 0x3a, 0x01];
        mode.resize(32, 0);
        assert_eq!(recorder.features(), vec![mode]);
    }

    #[test]
    fn key_faces_are_chunked_into_320_byte_reports() {
        let jpeg: Vec<u8> = (0..313u16).map(|byte| byte as u8).collect();
        let reports = image_reports(KEY_IMAGE_COMMAND, 4, &jpeg);
        assert_eq!(reports.len(), 2);
        assert_eq!(
            &reports[0][..8],
            &[0x02, 0x36, 0x00, 0x00, 0x38, 0x01, 0x04, 0x00]
        );
        assert_eq!(&reports[0][8..], &jpeg[..312]);
        assert_eq!(
            &reports[1][..9],
            &[0x02, 0x36, 0x01, 0x00, 0x01, 0x00, 0x04, 0x01, jpeg[312]]
        );
        assert!(reports[1][9..].iter().all(|byte| *byte == 0));
        assert!(image_reports(KEY_IMAGE_COMMAND, 0, &[]).is_empty());
        let one = image_reports(DIAL_IMAGE_COMMAND, DIAL_SCREEN_SLOT, &[0xff, 0xd8]);
        assert_eq!(
            &one[0][..10],
            &[0x02, 0x45, 0x00, 0x00, 0x02, 0x00, 0x30, 0x01, 0xff, 0xd8]
        );
    }

    fn jpeg_of(reports: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for report in reports {
            let length = usize::from(u16::from_le_bytes([report[4], report[5]]));
            out.extend_from_slice(&report[8..8 + length]);
        }
        out
    }

    #[test]
    fn keypad_faces_are_72_pixel_jpegs_sent_on_flush() {
        let (mut deck, recorder) = open("coolermaster-masterhub-keypad");
        let face = Face {
            label: "Go".to_owned(),
            color: [200, 10, 10],
            image: None,
        };
        deck.set_face(2, Some(&face)).unwrap();
        deck.set_face(3, None).unwrap();
        // Not a keypad key.
        deck.set_face(15, Some(&face)).unwrap();
        assert!(recorder.writes().is_empty(), "nothing before flush");
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert!(writes.iter().all(|report| report.len() == 320));
        assert!(writes.iter().all(|report| report[..2] == [0x02, 0x36]));
        for key in [2u8, 3] {
            let chunks: Vec<Vec<u8>> = writes
                .iter()
                .filter(|report| report[6] == key)
                .cloned()
                .collect();
            assert_eq!(chunks.last().unwrap()[7], 1, "the last chunk commits");
            let picture = image::load_from_memory(&jpeg_of(&chunks)).unwrap();
            assert_eq!((picture.width(), picture.height()), (72, 72));
        }
        assert!(writes.iter().all(|report| report[6] < 15));
    }

    #[test]
    fn keypad_brightness_is_a_clamped_feature_report() {
        let (mut deck, recorder) = open("coolermaster-masterhub-keypad");
        deck.set_brightness(55).unwrap();
        deck.set_brightness(250).unwrap();
        let features = recorder.features();
        assert_eq!(features.len(), 3, "software mode, then two levels");
        assert_eq!(&features[1][..4], &[0x03, 0x34, 55, 0x00]);
        assert_eq!(&features[2][..3], &[0x03, 0x34, 100]);
        assert!(features.iter().all(|report| report.len() == 32));
    }

    #[test]
    fn knob_reports_are_reordered_to_logical_knobs() {
        let (mut deck, recorder) = open("coolermaster-masterhub-knob");
        // Report byte 0 clockwise (knob 1), byte 2 counter-clockwise (knob 0).
        recorder.queue_read(report(0x2a, &[0x00, 0x03, 0x01, 0xff, 0x00]));
        // Byte 1 pushed: knob 2.
        recorder.queue_read(report(0x2a, &[0x03, 0x03, 0x00, 0x01, 0x00]));
        recorder.queue_read(report(0x2a, &[0x03, 0x03, 0x00, 0x01, 0x02]));
        recorder.queue_read(report(0x2a, &[0x03, 0x03, 0x01, 0x00, 0x00]));
        // A slider report on the knob module is ignored.
        recorder.queue_read(report(0x2a, &[0x02, 0x05, 1, 2, 3, 4, 5]));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(1, 1),
                turn(0, -1),
                press(2, true),
                press(1, true),
                press(2, false),
            ]
        );
        assert!(recorder.features().is_empty(), "the knob needs no init");
    }

    #[test]
    fn knob_lights_use_their_own_numbering() {
        let (mut deck, recorder) = open("coolermaster-masterhub-knob");
        deck.set_dial_light(0, Some([0x33, 0x24, 0x3f])).unwrap();
        deck.set_dial_light(1, None).unwrap();
        deck.set_dial_light(3, Some([1, 2, 3])).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert!(writes.iter().all(|report| report.len() == 64));
        let heads: Vec<&[u8]> = writes.iter().map(|report| &report[..9]).collect();
        assert_eq!(
            heads,
            vec![
                // Knob 1 is light 0, turned off.
                &[0x02, 0x2c, 0x02, 0x00, 0x01, 0x00, 0, 0, 0][..],
                &[0x02, 0x2d, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0][..],
                // Knob 0 is light 2.
                &[0x02, 0x2c, 0x02, 0x02, 0x01, 0x00, 0, 0, 0][..],
                &[0x02, 0x2d, 0x02, 0x02, 0x33, 0x24, 0x3f, 0x00, 0][..],
            ]
        );
        deck.set_brightness(70).unwrap();
        assert_eq!(
            &recorder.writes()[4][..6],
            &[0x02, 0x2e, 0x02, 0xff, 70, 0x00]
        );
    }

    #[test]
    fn sliders_turn_by_the_change_after_the_first_sample() {
        let (mut deck, recorder) = open("coolermaster-masterhub-slider");
        recorder.queue_read(report(0x2a, &[0x02, 0x05, 10, 20, 30, 40, 50]));
        recorder.queue_read(report(0x2a, &[0x02, 0x05, 15, 20, 30, 0, 50]));
        // A value over 100 rejects the whole report.
        recorder.queue_read(report(0x2a, &[0x02, 0x05, 99, 20, 30, 0, 101]));
        // Some firmware sends id 0 with five values.
        recorder.queue_read(report(0x2a, &[0x00, 0x05, 15, 20, 30, 0, 100]));
        // A knob turn report is not a slider one.
        recorder.queue_read(report(0x2a, &[0x00, 0x03, 0x01, 0x01, 0x01]));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(0, 5), turn(3, -40), turn(4, 50)]
        );
        deck.set_dial_light(4, Some([9, 8, 7])).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            &recorder.writes()[1][..8],
            &[0x02, 0x2d, 0x01, 0x04, 9, 8, 7, 0]
        );
        deck.set_brightness(30).unwrap();
        assert_eq!(&recorder.writes()[2][..5], &[0x02, 0x2e, 0x01, 0xff, 30]);
    }

    #[test]
    fn rollers_run_left_to_right_and_accept_loose_frames() {
        let (mut deck, recorder) = open("coolermaster-masterhub-roller");
        // Byte 0 (right roller, dial 1) up; byte 1 (left, dial 0) down.
        recorder.queue_read(report(0x2a, &[0x01, 0x02, 0x00, 0x01]));
        // Still and no-detent bytes raise nothing.
        recorder.queue_read(report(0x2a, &[0x01, 0x02, 0xff, 0x7f]));
        // A bare two-byte payload.
        recorder.queue_read(report(0x2a, &[0x7f, 0x00]));
        // A length field that does not match: bytes 4 and 5 are read.
        let mut loose = vec![0x01, 0x2a, 0x09, 0x00, 0x01, 0xff];
        loose.resize(8, 0);
        recorder.queue_read(loose);
        // A knob report is dropped, not read loosely.
        recorder.queue_read(report(0x2a, &[0x00, 0x03, 0x00, 0x00, 0x00]));
        // Too short for the loose read.
        recorder.queue_read(vec![0x01, 0x2a, 0x09, 0x00, 0x01]);
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(1, 1), turn(0, -1), turn(0, 1), turn(1, -1)]
        );
        deck.set_brightness(50).unwrap();
        deck.set_dial_light(0, Some([1, 2, 3])).unwrap();
        deck.flush().unwrap();
        assert!(recorder.writes().is_empty() && recorder.features().is_empty());
    }

    #[test]
    fn the_dial_turns_presses_and_shows_one_240_pixel_frame() {
        let (mut deck, recorder) = open("coolermaster-masterhub-dial");
        recorder.queue_read(report(0x44, &[0x03, 0x01, 0x00]));
        recorder.queue_read(report(0x44, &[0x03, 0x01, 0x01]));
        recorder.queue_read(report(0x44, &[0x04, 0x01, 0x01]));
        recorder.queue_read(report(0x44, &[0x04, 0x01, 0x01]));
        recorder.queue_read(report(0x44, &[0x04, 0x01, 0x00]));
        // Any other length is ignored.
        recorder.queue_read(report(0x44, &[0x03, 0x01, 0x00, 0x00]));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![turn(0, 1), turn(0, -1), press(0, true), press(0, false)]
        );
        let mut mode = vec![
            0x03, 0x49, 0x01, 0x00, 0x00, 0x00, 0xf0, 0x00, 0xf0, 0x00, 0x01, 0xf0, 0xf0,
        ];
        mode.resize(32, 0);
        assert_eq!(recorder.features(), vec![mode]);

        let face = Face {
            label: String::new(),
            color: [0, 90, 200],
            image: None,
        };
        let wheel = by_id("coolermaster-masterhub-dial")
            .unwrap()
            .wheel_key()
            .unwrap();
        deck.set_face(wheel, Some(&face)).unwrap();
        deck.flush().unwrap();
        let sent = recorder.writes();
        assert!(sent
            .iter()
            .all(|report| report.len() == 320 && report[..2] == [0x02, 0x45] && report[6] == 0x30));
        let picture = image::load_from_memory(&jpeg_of(&sent)).unwrap();
        assert_eq!((picture.width(), picture.height()), (240, 240));
        // The same frame again is not resent.
        deck.set_face(wheel, Some(&face)).unwrap();
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), sent.len());
    }

    #[test]
    fn the_base_ignores_its_heartbeat_and_lights_one_led() {
        let (mut deck, recorder) = open("coolermaster-masterhub-base");
        recorder.queue_read(report(0x2b, &[0x00]));
        assert!(deck.poll(0).unwrap().is_empty());
        deck.set_deck_light(Some([10, 20, 30])).unwrap();
        deck.flush().unwrap();
        deck.set_brightness(40).unwrap();
        let writes = recorder.writes();
        assert_eq!(&writes[0][..6], &[0x02, 0x2c, 0x00, 0x00, 0x01, 0x00]);
        assert_eq!(&writes[1][..8], &[0x02, 0x2d, 0x00, 0x00, 10, 20, 30, 0]);
        assert_eq!(&writes[2][..5], &[0x02, 0x2e, 0x00, 0x00, 40]);
        assert!(recorder.features().is_empty());
    }
}
