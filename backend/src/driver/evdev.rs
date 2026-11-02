//! Linux input event nodes (`/dev/input/eventN`) as a [`Transport`], for a
//! deck the kernel keeps away from hidraw: the Griffin PowerMate is on the
//! kernel's HID ignore list, so hidapi never sees it on Linux, and its own
//! input driver exposes it as an event node instead.
//!
//! The kernel's `struct input_event` starts with a `timeval` whose size depends
//! on the platform, so the link speaks a fixed record to drivers instead:
//! [`RECORD_LEN`] bytes, the event type (u16 LE), code (u16 LE) and value
//! (i32 LE). A read returns whole records; a write takes whole records and
//! sends each as one native event.

/// Bytes in one record a driver reads or writes through an [`EvdevLink`].
pub const RECORD_LEN: usize = 8;

pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
pub const EV_MSC: u16 = 0x04;
/// The first of the generic "misc" buttons (the PowerMate's knob push).
pub const BTN_0: u16 = 0x100;
/// A relative dial (the PowerMate's knob turn).
pub const REL_DIAL: u16 = 0x07;
/// The PowerMate driver's packed LED state, written as an `EV_MSC` event.
pub const MSC_PULSELED: u16 = 0x01;

/// One event in the link's portable form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

impl Event {
    #[must_use]
    pub fn encode(self) -> [u8; RECORD_LEN] {
        let mut record = [0u8; RECORD_LEN];
        record[0..2].copy_from_slice(&self.kind.to_le_bytes());
        record[2..4].copy_from_slice(&self.code.to_le_bytes());
        record[4..8].copy_from_slice(&self.value.to_le_bytes());
        record
    }

    /// The event in one record; `None` for a short one.
    #[must_use]
    pub fn decode(record: &[u8]) -> Option<Self> {
        let kind = u16::from_le_bytes([*record.first()?, *record.get(1)?]);
        let code = u16::from_le_bytes([*record.get(2)?, *record.get(3)?]);
        let value = i32::from_le_bytes([
            *record.get(4)?,
            *record.get(5)?,
            *record.get(6)?,
            *record.get(7)?,
        ]);
        Some(Self { kind, code, value })
    }
}

#[cfg(target_os = "linux")]
pub use linux::{nodes, EvdevLink, EventNode};

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::CString;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    use super::{Event, RECORD_LEN};
    use crate::driver::{DriverError, DriverResult, Transport};

    /// Bytes in the kernel's `struct input_event` on this platform.
    const NATIVE_LEN: usize = std::mem::size_of::<libc::input_event>();
    /// The `timeval` in front of type, code and value.
    const TIME_LEN: usize = NATIVE_LEN - RECORD_LEN;
    /// Events read from the node in one go, at most.
    const MAX_EVENTS: usize = 16;

    /// An input event node found under `/sys/class/input`.
    #[derive(Debug, Clone)]
    pub struct EventNode {
        /// `/dev/input/eventN`.
        pub path: CString,
        pub vendor_id: u16,
        pub product_id: u16,
        /// The device's unique id, when the kernel driver reports one.
        pub uniq: Option<String>,
    }

    fn read_hex(path: &Path) -> Option<u16> {
        let text = std::fs::read_to_string(path).ok()?;
        u16::from_str_radix(text.trim(), 16).ok()
    }

    /// Every USB input event node on this machine, with its USB ids.
    #[must_use]
    pub fn nodes() -> Vec<EventNode> {
        let Ok(entries) = std::fs::read_dir("/sys/class/input") else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with("event") {
                continue;
            }
            let device = entry.path().join("device");
            // USB only (bus type 0x0003).
            if read_hex(&device.join("id/bustype")) != Some(0x0003) {
                continue;
            }
            let (Some(vendor_id), Some(product_id)) = (
                read_hex(&device.join("id/vendor")),
                read_hex(&device.join("id/product")),
            ) else {
                continue;
            };
            let Ok(path) = CString::new(format!("/dev/input/{name}")) else {
                continue;
            };
            let uniq = std::fs::read_to_string(device.join("uniq"))
                .ok()
                .map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty());
            found.push(EventNode {
                path,
                vendor_id,
                product_id,
                uniq,
            });
        }
        found
    }

    /// An opened event node, read without blocking and written to for output
    /// events the kernel driver acts on.
    pub struct EvdevLink {
        file: std::fs::File,
    }

    impl EvdevLink {
        /// Opens `path` for reading and writing.
        ///
        /// # Errors
        /// When the node cannot be opened, usually for want of the udev rule.
        pub fn open(path: &str) -> std::io::Result<Self> {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(path)?;
            Ok(Self { file })
        }

        /// Waits up to `timeout_ms` for the node to have something to read.
        fn wait(&self, timeout_ms: i32) -> DriverResult<bool> {
            let mut poll = libc::pollfd {
                fd: self.file.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: `poll` is one valid pollfd for an fd this link keeps open.
            let ready = unsafe { libc::poll(&raw mut poll, 1, timeout_ms.max(0)) };
            if ready < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    return Ok(false);
                }
                return Err(error.into());
            }
            if poll.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                return Err(DriverError::Io("the input device went away".to_owned()));
            }
            Ok(ready > 0)
        }
    }

    impl Transport for EvdevLink {
        fn write(&mut self, data: &[u8]) -> DriverResult<()> {
            for record in data.chunks_exact(RECORD_LEN) {
                let mut native = [0u8; NATIVE_LEN];
                let Some(event) = Event::decode(record) else {
                    continue;
                };
                native[TIME_LEN..TIME_LEN + 2].copy_from_slice(&event.kind.to_ne_bytes());
                native[TIME_LEN + 2..TIME_LEN + 4].copy_from_slice(&event.code.to_ne_bytes());
                native[TIME_LEN + 4..].copy_from_slice(&event.value.to_ne_bytes());
                self.file.write_all(&native)?;
            }
            Ok(())
        }

        fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> DriverResult<usize> {
            let wanted = (buf.len() / RECORD_LEN).min(MAX_EVENTS);
            if wanted == 0 || !self.wait(timeout_ms)? {
                return Ok(0);
            }
            let mut native = [0u8; NATIVE_LEN * MAX_EVENTS];
            let read = match self.file.read(&mut native[..NATIVE_LEN * wanted]) {
                Ok(0) => {
                    return Err(DriverError::Io("the input device went away".to_owned()));
                }
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(0),
                Err(error) => return Err(error.into()),
            };
            let mut len = 0;
            for event in native[..read].chunks_exact(NATIVE_LEN) {
                // `chunks_exact` makes every chunk NATIVE_LEN bytes long.
                let tail = &event[TIME_LEN..];
                let kind = u16::from_ne_bytes([tail[0], tail[1]]);
                let code = u16::from_ne_bytes([tail[2], tail[3]]);
                let value = i32::from_ne_bytes([tail[4], tail[5], tail[6], tail[7]]);
                let record = Event { kind, code, value }.encode();
                buf[len..len + RECORD_LEN].copy_from_slice(&record);
                len += RECORD_LEN;
            }
            Ok(len)
        }

        fn send_feature_report(&mut self, _data: &[u8]) -> DriverResult<()> {
            Err(DriverError::Io(
                "an input event node has no feature reports".to_owned(),
            ))
        }

        fn get_feature_report(&mut self, _buf: &mut [u8]) -> DriverResult<usize> {
            Err(DriverError::Io(
                "an input event node has no feature reports".to_owned(),
            ))
        }

        fn is_evdev(&self) -> bool {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_round_trip_and_short_ones_are_refused() {
        let event = Event {
            kind: EV_REL,
            code: REL_DIAL,
            value: -3,
        };
        let record = event.encode();
        assert_eq!(record, [0x02, 0x00, 0x07, 0x00, 0xfd, 0xff, 0xff, 0xff]);
        assert_eq!(Event::decode(&record), Some(event));
        assert_eq!(Event::decode(&record[..7]), None);
    }
}
