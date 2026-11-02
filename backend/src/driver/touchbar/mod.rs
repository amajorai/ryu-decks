//! The MacBook Pro Touch Bar as a deck (macOS only).
//!
//! AppKit owns the Touch Bar and must run on the process's main thread, while
//! every other deck lives on the device-loop thread. The two talk over a [`Link`]:
//! the [`TouchBar`] driver sends the bound keys' faces to the main thread, and the
//! main thread sends back the number of each key tapped. Ryu puts a button in the
//! Touch Bar's Control Strip; tapping it opens a bar with the bound keys, each
//! drawn with the face the Companion rendered. Nothing is added until a key is
//! bound, so a Mac with an empty Touch Bar layout never starts AppKit.

#[cfg(target_os = "macos")]
mod appkit;

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{face, Driver, DriverError, DriverResult, Face, Input};
use crate::catalog::DeviceModel;

/// The catalog id (and device id) of the Touch Bar.
pub const MODEL_ID: &str = "apple-touch-bar";

/// One bound key, ready for the Touch Bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub key: u8,
    pub label: String,
    pub color: [u8; 3],
    /// The Companion's rendered face, as PNG. `None` when the key has no rendered
    /// face; the button then shows its label on its colour.
    pub png: Option<Vec<u8>>,
}

/// What the device loop asks of the main thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Show these keys, in this order, and offer the Control Strip button.
    Show(Vec<Button>),
    /// No key is bound: take the Control Strip button away.
    Hide,
}

/// The device loop's end of the link to the main thread.
#[derive(Clone)]
pub struct Link {
    commands: Sender<Command>,
    taps: Arc<Mutex<Receiver<u8>>>,
}

/// The main thread's end of the link.
pub struct MainEnd {
    pub commands: Receiver<Command>,
    pub taps: Sender<u8>,
}

#[must_use]
pub fn link() -> (Link, MainEnd) {
    let (commands, command_inbox) = mpsc::channel();
    let (taps, tap_inbox) = mpsc::channel();
    (
        Link {
            commands,
            taps: Arc::new(Mutex::new(tap_inbox)),
        },
        MainEnd {
            commands: command_inbox,
            taps,
        },
    )
}

/// Whether this computer has a Touch Bar Ryu can drive. Only Touch Bar Macs run
/// `TouchBarServer`. `{"touch_bar": false}` in the app's `settings.json` keeps the
/// sidecar from calling this at all.
#[must_use]
pub fn available() -> bool {
    cfg!(target_os = "macos")
        && std::process::Command::new("/usr/bin/pgrep")
            .args(["-xq", "TouchBarServer"])
            .status()
            .is_ok_and(|status| status.success())
}

/// Run the Touch Bar on the calling thread, which must be the process's main
/// thread. Returns only if the link closes before a key is bound, or AppKit
/// cannot start.
#[cfg(target_os = "macos")]
pub fn run_main_thread(end: MainEnd) {
    appkit::run(end);
}

pub struct TouchBar {
    model: &'static DeviceModel,
    link: Option<Link>,
    buttons: BTreeMap<u8, Button>,
}

impl TouchBar {
    #[must_use]
    pub fn new(model: &'static DeviceModel, link: Link) -> Self {
        Self {
            model,
            link: Some(link),
            buttons: BTreeMap::new(),
        }
    }

    /// A Touch Bar with no main thread behind it. The device loop never opens the
    /// Touch Bar as a USB device; this only keeps [`super::for_model`] total.
    #[must_use]
    pub fn unlinked(model: &'static DeviceModel) -> Self {
        Self {
            model,
            link: None,
            buttons: BTreeMap::new(),
        }
    }

    fn link(&self) -> DriverResult<&Link> {
        self.link.as_ref().ok_or_else(|| {
            DriverError::Io("the Touch Bar is driven by the main thread, not opened".to_owned())
        })
    }
}

/// A tap is a click: the Touch Bar's buttons report the tap, not the touch.
fn click(index: u8) -> [Input; 2] {
    [
        Input::Key {
            index,
            pressed: true,
        },
        Input::Key {
            index,
            pressed: false,
        },
    ]
}

impl Driver for TouchBar {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let link = self.link()?;
        let taps = link
            .taps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let wait = Duration::from_millis(u64::try_from(timeout_ms).unwrap_or(0));
        let gone = || DriverError::Io("the Touch Bar stopped".to_owned());
        let mut inputs = Vec::new();
        match taps.recv_timeout(wait) {
            Ok(key) => inputs.extend(click(key)),
            Err(RecvTimeoutError::Timeout) => return Ok(inputs),
            Err(RecvTimeoutError::Disconnected) => return Err(gone()),
        }
        loop {
            match taps.try_recv() {
                Ok(key) => inputs.extend(click(key)),
                Err(TryRecvError::Empty) => return Ok(inputs),
                Err(TryRecvError::Disconnected) => return Err(gone()),
            }
        }
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if self.model.strip_segment(key).is_none() {
            return Ok(());
        }
        let Some(face) = face else {
            self.buttons.remove(&key);
            return Ok(());
        };
        let png = match (face.image.as_ref(), self.model.strip_image) {
            (Some(_), Some(format)) => Some(face::encode(Some(face), &format)?),
            _ => None,
        };
        self.buttons.insert(
            key,
            Button {
                key,
                label: face.label.clone(),
                color: face.color,
                png,
            },
        );
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        let command = if self.buttons.is_empty() {
            Command::Hide
        } else {
            Command::Show(self.buttons.values().cloned().collect())
        };
        self.link()?
            .commands
            .send(command)
            .map_err(|_| DriverError::Io("the Touch Bar stopped".to_owned()))
    }

    /// The Touch Bar follows the Mac's own brightness.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;

    fn face(label: &str, with_image: bool) -> Face {
        Face {
            label: label.to_owned(),
            color: [20, 120, 240],
            image: with_image.then(|| image::RgbaImage::new(8, 8)),
        }
    }

    #[test]
    fn flush_shows_only_bound_keys_in_order() {
        let model = by_id(MODEL_ID).unwrap();
        let (link, end) = link();
        let mut bar = TouchBar::new(model, link);
        bar.set_face(3, Some(&face("Ship", true))).unwrap();
        bar.set_face(0, Some(&face("Standup", false))).unwrap();
        bar.set_face(1, None).unwrap();
        bar.flush().unwrap();
        let Command::Show(buttons) = end.commands.try_recv().unwrap() else {
            panic!("expected the bound keys");
        };
        assert_eq!(
            buttons.iter().map(|button| button.key).collect::<Vec<_>>(),
            vec![0, 3]
        );
        assert_eq!(buttons[0].label, "Standup");
        assert_eq!(buttons[0].png, None, "no rendered face: label on colour");
        let png = buttons[1].png.as_deref().unwrap();
        assert_eq!(&png[1..4], b"PNG");

        // Unbinding the last key takes the Control Strip button away.
        bar.set_face(0, None).unwrap();
        bar.set_face(3, None).unwrap();
        bar.flush().unwrap();
        assert_eq!(end.commands.try_recv().unwrap(), Command::Hide);
    }

    #[test]
    fn taps_read_as_clicks_and_a_closed_link_disconnects() {
        let model = by_id(MODEL_ID).unwrap();
        let (link, end) = link();
        let mut bar = TouchBar::new(model, link);
        assert!(bar.poll(0).unwrap().is_empty());
        end.taps.send(2).unwrap();
        end.taps.send(5).unwrap();
        assert_eq!(
            bar.poll(0).unwrap(),
            [click(2), click(5)].concat(),
            "both taps drained in one poll"
        );
        drop(end);
        assert!(bar.poll(0).is_err());
        assert!(bar.flush().is_err());
    }

    #[test]
    fn an_unlinked_touch_bar_refuses_to_work() {
        let mut bar = TouchBar::unlinked(by_id(MODEL_ID).unwrap());
        assert!(bar.poll(0).is_err());
        assert!(bar.flush().is_err());
    }
}
