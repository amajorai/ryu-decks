//! Shared Ryu sidecar data-directory seam.

use std::path::{Path, PathBuf};

/// This app's own data dir: `<data dir>/plugin-data/@ryu+decks`
/// (`RYU_PLUGIN_DATA_DIR` when Core spawned the process).
pub fn data_dir() -> PathBuf {
    ryu_sidecar_runtime::plugin_data_dir(crate::PLUGIN_ID)
}

/// Key layouts and device settings: `<root>/decks.db`.
pub fn db_path(root: &Path) -> PathBuf {
    root.join("decks.db")
}

/// Machine-level driver switches: `<root>/settings.json` (see `settings`).
pub fn settings_path(root: &Path) -> PathBuf {
    root.join("settings.json")
}
