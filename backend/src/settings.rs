//! Machine-level driver switches, kept in `<data dir>/settings.json`.
//!
//! `{"hid": false}` starts the sidecar without the device loop, for headless runs
//! and machines where no deck will ever be attached. `{"touch_bar": false}` leaves a
//! Mac's Touch Bar alone. Both default to on. A missing file keeps the defaults, and
//! so does an unreadable one (with a warning), because a typo must not quietly take
//! every attached deck offline.

use std::path::Path;

use serde::Deserialize;

use crate::paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct DriverSettings {
    /// Open attached decks and run the device loop.
    pub hid: bool,
    /// Drive the MacBook Pro Touch Bar (only when `hid` is also on).
    pub touch_bar: bool,
}

impl Default for DriverSettings {
    fn default() -> Self {
        Self {
            hid: true,
            touch_bar: true,
        }
    }
}

impl DriverSettings {
    /// Read the switches from `root`'s settings file. Synchronous on purpose: the
    /// Touch Bar decision is made on the main thread before the runtime starts.
    #[must_use]
    pub fn load(root: &Path) -> Self {
        let path = paths::settings_path(root);
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            tracing::warn!(
                path = %path.display(),
                %error,
                "ryu-decks: ignoring an unreadable settings file; drivers stay on"
            );
            Self::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::DriverSettings;
    use crate::paths;

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ryu-decks-settings-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn a_missing_file_keeps_every_driver_on() {
        let dir = scratch_dir("missing");
        let _ = std::fs::remove_file(paths::settings_path(&dir));
        assert_eq!(DriverSettings::load(&dir), DriverSettings::default());
        assert!(DriverSettings::default().hid && DriverSettings::default().touch_bar);
    }

    #[test]
    fn a_partial_file_switches_off_only_what_it_names() {
        let dir = scratch_dir("partial");
        std::fs::write(paths::settings_path(&dir), br#"{"touch_bar": false}"#).unwrap();
        let settings = DriverSettings::load(&dir);
        assert!(settings.hid);
        assert!(!settings.touch_bar);
        std::fs::write(paths::settings_path(&dir), br#"{"hid": false}"#).unwrap();
        assert!(!DriverSettings::load(&dir).hid);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_file_keeps_the_defaults() {
        let dir = scratch_dir("malformed");
        std::fs::write(paths::settings_path(&dir), b"hid=off").unwrap();
        assert_eq!(DriverSettings::load(&dir), DriverSettings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
