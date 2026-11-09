//! Work Louder Creator Micro 2 and the OpenAI Codex Micro built on it.
//!
//! The keypad runs a JSON-RPC server on its vendor HID collection (usage page
//! `0xFF00`). Reports are 64 bytes: report id `0x06`, a channel (`2` is RPC), a
//! payload length of at most 61, then UTF-8. Messages are CRLF-terminated and may
//! span reports. Keys mapped to the `KV_OAI_AG00`..`AG05` keycodes send no
//! keystroke; they raise a `v.oai.hid` notification instead, and their lights take
//! host colours through `v.oai.thstatus`. Protocol per the MIT-licensed
//! 00cyre/creator-micro-kit, verified there against firmware 0.6.2.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::DeviceModel;

const REPORT_ID: u8 = 0x06;
const REPORT_LEN: usize = 64;
const CHANNEL_RPC: u8 = 2;
const MAX_CHUNK: usize = 61;
/// The firmware rejects request ids of 1000 and above.
const MAX_REQUEST_ID: u32 = 999;
/// Firmware animation index for a steady colour; 0 turns the light off.
const EFFECT_SOLID: u8 = 1;
const EFFECT_OFF: u8 = 0;
/// A line longer than this is not a message we understand; drop the buffer.
const MAX_LINE: usize = 8 * 1024;

pub struct WorkLouder {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    inbox: Vec<u8>,
    next_id: u32,
    brightness: f32,
    lights: BTreeMap<u8, Option<[u8; 3]>>,
    dirty: Vec<u8>,
}

impl WorkLouder {
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            model,
            transport,
            inbox: Vec::new(),
            next_id: 1,
            brightness: 1.0,
            lights: BTreeMap::new(),
            dirty: Vec::new(),
        }
    }

    fn call(&mut self, method: &str, params: Value) -> DriverResult<()> {
        let id = self.next_id;
        self.next_id = if id >= MAX_REQUEST_ID { 1 } else { id + 1 };
        let message = json!({ "method": method, "params": params, "id": id });
        for report in rpc_reports(&message.to_string()) {
            self.transport.write(&report)?;
        }
        Ok(())
    }

    fn light_entry(&self, key: u8) -> Value {
        match self.lights.get(&key).copied().flatten() {
            Some([r, g, b]) => json!({
                "id": key,
                "c": (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
                "b": self.brightness,
                "e": EFFECT_SOLID,
            }),
            None => json!({ "id": key, "b": 0, "e": EFFECT_OFF }),
        }
    }

    /// Feed one input report; returns any completed key events.
    fn receive(&mut self, report: &[u8]) -> Vec<Input> {
        let Some((channel, payload)) = split_report(report) else {
            return Vec::new();
        };
        if channel != CHANNEL_RPC {
            return Vec::new();
        }
        self.inbox.extend_from_slice(payload);
        let mut events = Vec::new();
        while let Some(end) = self.inbox.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.inbox.drain(..=end).collect();
            if let Some(event) = parse_notification(&line, self.model.keys) {
                events.push(event);
            } else if let Some(error) = rejected_request(&line) {
                // Replies go to every open reader, so this may be another
                // client's request; it is still the only sign a paint failed.
                tracing::warn!(device = self.model.id, %error, "ryu-decks: keypad rejected a request");
            }
        }
        if self.inbox.len() > MAX_LINE {
            self.inbox.clear();
        }
        events
    }
}

/// Split one RPC message into 64-byte reports. The message is wrapped in CRLF on
/// both sides: the leading one ends any partial line another writer (or a failed
/// earlier write) left in the device's per-channel buffer, so it cannot merge into
/// this message and get it rejected.
fn rpc_reports(message: &str) -> Vec<Vec<u8>> {
    let mut bytes = b"\r\n".to_vec();
    bytes.extend_from_slice(message.as_bytes());
    bytes.extend_from_slice(b"\r\n");
    bytes
        .chunks(MAX_CHUNK)
        .map(|chunk| {
            let mut report = vec![REPORT_ID, CHANNEL_RPC, chunk.len() as u8];
            report.extend_from_slice(chunk);
            report.resize(REPORT_LEN, 0);
            report
        })
        .collect()
}

fn split_report(report: &[u8]) -> Option<(u8, &[u8])> {
    let (&id, rest) = report.split_first()?;
    if id != REPORT_ID || rest.len() < 2 {
        return None;
    }
    let channel = rest[0];
    let len = usize::from(rest[1]).min(MAX_CHUNK).min(rest.len() - 2);
    Some((channel, &rest[2..2 + len]))
}

/// `{"error":{"message":..},"method":"v.oai.thstatus",..}` -> a readable reason.
fn rejected_request(line: &[u8]) -> Option<String> {
    let reply: Value = serde_json::from_slice(line.trim_ascii()).ok()?;
    let error = reply.get("error")?;
    let method = reply
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("request");
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("no detail");
    Some(format!("{method}: {message}"))
}

/// `{"m":"v.oai.hid","p":{"k":"AG03","act":1}}` -> key 3 pressed.
fn parse_notification(line: &[u8], keys: u8) -> Option<Input> {
    let message: Value = serde_json::from_slice(line.trim_ascii()).ok()?;
    if message.get("m")?.as_str()? != "v.oai.hid" {
        return None;
    }
    let params = message.get("p")?;
    let index: u8 = params
        .get("k")?
        .as_str()?
        .strip_prefix("AG")?
        .parse()
        .ok()?;
    if index >= keys {
        return None;
    }
    let pressed = params.get("act").and_then(Value::as_i64) == Some(1);
    Some(Input::Key { index, pressed })
}

impl Driver for WorkLouder {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; REPORT_LEN + 1];
        for attempt in 0..32 {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            events.extend(self.receive(&buf[..read]));
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if key >= self.model.keys {
            return Ok(());
        }
        self.lights.insert(key, face.map(|face| face.color));
        if !self.dirty.contains(&key) {
            self.dirty.push(key);
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        if self.dirty.is_empty() {
            return Ok(());
        }
        let mut keys = std::mem::take(&mut self.dirty);
        keys.sort_unstable();
        let entries: Vec<Value> = keys.iter().map(|key| self.light_entry(*key)).collect();
        self.call("v.oai.thstatus", Value::Array(entries))
    }

    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = f32::from(percent.min(100)) / 100.0;
        self.dirty = self.lights.keys().copied().collect();
        self.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::by_id;

    fn reports_for(message: &str) -> Vec<Vec<u8>> {
        rpc_reports(message)
    }

    fn reassemble(reports: &[Vec<u8>]) -> String {
        let mut text = Vec::new();
        for report in reports {
            let (channel, payload) = split_report(report).unwrap();
            assert_eq!(channel, CHANNEL_RPC);
            text.extend_from_slice(payload);
        }
        String::from_utf8(text).unwrap()
    }

    #[test]
    fn long_messages_span_reports_and_end_in_crlf() {
        let message = "x".repeat(130);
        let reports = reports_for(&message);
        // 2 + 130 + 2 bytes in 61-byte chunks.
        assert_eq!(reports.len(), 3);
        assert!(reports
            .iter()
            .all(|r| r.len() == REPORT_LEN && r[0] == REPORT_ID));
        assert_eq!(reports[0][2], 61);
        assert_eq!(reassemble(&reports), format!("\r\n{message}\r\n"));
    }

    #[test]
    fn agent_key_notifications_become_key_events_across_reports() {
        let recorder = Recorder::default();
        let mut pad = WorkLouder::new(
            by_id("openai-codex-micro").unwrap(),
            Box::new(recorder.clone()),
        );
        let note = r#"{"m":"v.oai.hid","p":{"k":"AG04","act":1,"ag":0}}"#;
        for report in reports_for(note) {
            recorder.queue_read(report);
        }
        recorder
            .queue_read(reports_for(r#"{"m":"v.oai.hid","p":{"k":"AG04","act":0}}"#)[0].clone());
        // Debug output on channel 1 is ignored.
        recorder.queue_read(vec![REPORT_ID, 1, 3, b'h', b'i', b'\n']);
        assert_eq!(
            pad.poll(0).unwrap(),
            vec![
                Input::Key {
                    index: 4,
                    pressed: true
                },
                Input::Key {
                    index: 4,
                    pressed: false
                }
            ]
        );
    }

    #[test]
    fn error_replies_are_recognised() {
        assert_eq!(
            rejected_request(
                br#"{"error":{"code":-32700,"message":"JSON error - InvalidInput"},"id":4,"method":"v.oai.thstatus"}"#
            )
            .as_deref(),
            Some("v.oai.thstatus: JSON error - InvalidInput")
        );
        assert_eq!(rejected_request(br#"{"result":{"ok":1},"id":4}"#), None);
    }

    #[test]
    fn unknown_or_out_of_range_notifications_are_ignored() {
        assert_eq!(
            parse_notification(br#"{"m":"v.oai.rad","p":{"a":0.5}}"#, 6),
            None
        );
        assert_eq!(
            parse_notification(br#"{"m":"v.oai.hid","p":{"k":"AG09","act":1}}"#, 6),
            None
        );
        assert_eq!(parse_notification(br#"{"result":{},"id":3}"#, 6), None);
        assert_eq!(parse_notification(b"not json", 6), None);
    }

    #[test]
    fn faces_become_thread_status_colours() {
        let recorder = Recorder::default();
        let mut pad = WorkLouder::new(
            by_id("worklouder-creator-micro-2").unwrap(),
            Box::new(recorder.clone()),
        );
        pad.set_face(
            1,
            Some(&Face {
                label: String::new(),
                color: [0x2d, 0x7f, 0xf9],
                image: None,
            }),
        )
        .unwrap();
        pad.set_face(0, None).unwrap();
        pad.flush().unwrap();
        let sent = reassemble(&recorder.writes());
        let message: Value = serde_json::from_str(sent.trim()).unwrap();
        assert_eq!(message["method"], "v.oai.thstatus");
        assert_eq!(message["params"][0], json!({ "id": 0, "b": 0, "e": 0 }));
        assert_eq!(
            message["params"][1],
            json!({ "id": 1, "c": 0x2d7ff9, "b": 1.0, "e": 1 })
        );
        assert!(message["id"].as_u64().unwrap() < 1000);
    }
}
