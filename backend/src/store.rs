//! Key bindings and per-device settings, in one SQLite file under the app's data dir.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

/// What pressing a key does beyond raising the `key.pressed` app event, which
/// every press raises so workflows can react to any key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// Only the app event.
    None,
    /// Post `prompt` as a new chat turn to `agent_id` (the node default when unset).
    AskAgent {
        prompt: String,
        #[serde(default)]
        agent_id: Option<String>,
    },
}

impl Action {
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::AskAgent { .. } => "ask_agent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub device_id: String,
    pub key: u8,
    pub label: String,
    /// `#rrggbb`.
    pub color: String,
    /// The Companion-rendered face, as PNG bytes.
    pub face_png: Option<Vec<u8>>,
    pub action: Action,
    pub updated_at: i64,
}

pub struct Store {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS bindings (
    device_id TEXT NOT NULL,
    key_index INTEGER NOT NULL,
    label TEXT NOT NULL,
    color TEXT NOT NULL,
    face_png BLOB,
    action_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (device_id, key_index)
);
CREATE TABLE IF NOT EXISTS device_settings (
    device_id TEXT PRIMARY KEY,
    brightness INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS deck_lights (
    device_id TEXT PRIMARY KEY,
    color TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS dial_lights (
    device_id TEXT NOT NULL,
    dial INTEGER NOT NULL,
    color TEXT NOT NULL,
    PRIMARY KEY (device_id, dial)
);
";

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::with_connection(conn)
    }

    pub fn in_memory() -> Result<Self> {
        Self::with_connection(Connection::open_in_memory()?)
    }

    fn with_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)
            .context("creating the decks schema")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves SQLite itself consistent.
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn bindings(&self, device_id: &str) -> Result<Vec<Binding>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT key_index, label, color, face_png, action_json, updated_at
             FROM bindings WHERE device_id = ?1 ORDER BY key_index",
        )?;
        let rows = statement.query_map(params![device_id], |row| {
            Ok((
                row.get::<_, u8>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<Vec<u8>>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;
        let mut bindings = Vec::new();
        for row in rows {
            let (key, label, color, face_png, action_json, updated_at) = row?;
            bindings.push(Binding {
                device_id: device_id.to_owned(),
                key,
                label,
                color,
                face_png,
                // A row written by a newer build degrades to "event only".
                action: serde_json::from_str(&action_json).unwrap_or(Action::None),
                updated_at,
            });
        }
        Ok(bindings)
    }

    pub fn binding(&self, device_id: &str, key: u8) -> Result<Option<Binding>> {
        Ok(self
            .bindings(device_id)?
            .into_iter()
            .find(|binding| binding.key == key))
    }

    pub fn put_binding(&self, binding: &Binding) -> Result<()> {
        self.conn().execute(
            "INSERT INTO bindings (device_id, key_index, label, color, face_png, action_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (device_id, key_index) DO UPDATE SET
               label = excluded.label, color = excluded.color, face_png = excluded.face_png,
               action_json = excluded.action_json, updated_at = excluded.updated_at",
            params![
                binding.device_id,
                binding.key,
                binding.label,
                binding.color,
                binding.face_png,
                serde_json::to_string(&binding.action)?,
                binding.updated_at,
            ],
        )?;
        Ok(())
    }

    /// `true` when there was a binding to remove.
    pub fn delete_binding(&self, device_id: &str, key: u8) -> Result<bool> {
        let removed = self.conn().execute(
            "DELETE FROM bindings WHERE device_id = ?1 AND key_index = ?2",
            params![device_id, key],
        )?;
        Ok(removed > 0)
    }

    pub fn brightness(&self, device_id: &str) -> Result<Option<u8>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT brightness FROM device_settings WHERE device_id = ?1",
                params![device_id],
                |row| row.get::<_, u8>(0),
            )
            .optional()?)
    }

    pub fn set_brightness(&self, device_id: &str, percent: u8) -> Result<()> {
        self.conn().execute(
            "INSERT INTO device_settings (device_id, brightness) VALUES (?1, ?2)
             ON CONFLICT (device_id) DO UPDATE SET brightness = excluded.brightness",
            params![device_id, percent.min(100)],
        )?;
        Ok(())
    }

    /// A deck's dial light colours (`#rrggbb`), by dial.
    pub fn dial_colors(&self, device_id: &str) -> Result<std::collections::BTreeMap<u8, String>> {
        let conn = self.conn();
        let mut statement =
            conn.prepare("SELECT dial, color FROM dial_lights WHERE device_id = ?1")?;
        let colors = statement
            .query_map(params![device_id], |row| {
                Ok((row.get::<_, u8>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(colors)
    }

    /// Set a dial's light, or turn it off with `None`.
    pub fn set_dial_color(&self, device_id: &str, dial: u8, color: Option<&str>) -> Result<()> {
        let conn = self.conn();
        match color {
            Some(color) => conn.execute(
                "INSERT INTO dial_lights (device_id, dial, color) VALUES (?1, ?2, ?3)
                 ON CONFLICT (device_id, dial) DO UPDATE SET color = excluded.color",
                params![device_id, dial, color],
            )?,
            None => conn.execute(
                "DELETE FROM dial_lights WHERE device_id = ?1 AND dial = ?2",
                params![device_id, dial],
            )?,
        };
        Ok(())
    }

    /// The colour of a deck's own lights, if one is set.
    pub fn deck_light(&self, device_id: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT color FROM deck_lights WHERE device_id = ?1",
                params![device_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    }

    /// Set a deck's own lights, or turn them off with `None`.
    pub fn set_deck_light(&self, device_id: &str, color: Option<&str>) -> Result<()> {
        let conn = self.conn();
        match color {
            Some(color) => conn.execute(
                "INSERT INTO deck_lights (device_id, color) VALUES (?1, ?2)
                 ON CONFLICT (device_id) DO UPDATE SET color = excluded.color",
                params![device_id, color],
            )?,
            None => conn.execute(
                "DELETE FROM deck_lights WHERE device_id = ?1",
                params![device_id],
            )?,
        };
        Ok(())
    }

    /// Every device this node has a layout or setting for, connected or not.
    pub fn known_devices(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT device_id FROM bindings UNION SELECT device_id FROM device_settings
             UNION SELECT device_id FROM dial_lights UNION SELECT device_id FROM deck_lights
             ORDER BY device_id",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(key: u8, action: Action) -> Binding {
        Binding {
            device_id: "elgato-mk2-abc".to_owned(),
            key,
            label: format!("Key {key}"),
            color: "#112233".to_owned(),
            face_png: Some(vec![1, 2, 3]),
            action,
            updated_at: 10,
        }
    }

    #[test]
    fn bindings_round_trip_and_update_in_place() {
        let store = Store::in_memory().unwrap();
        store.put_binding(&binding(2, Action::None)).unwrap();
        let ask = Action::AskAgent {
            prompt: "Summarize my inbox".to_owned(),
            agent_id: Some("ryu".to_owned()),
        };
        let mut updated = binding(2, ask.clone());
        updated.label = "Inbox".to_owned();
        store.put_binding(&updated).unwrap();
        store.put_binding(&binding(0, Action::None)).unwrap();

        let all = store.bindings("elgato-mk2-abc").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].key, 0);
        assert_eq!(all[1], updated);
        assert!(store.bindings("other").unwrap().is_empty());

        assert!(store.delete_binding("elgato-mk2-abc", 2).unwrap());
        assert!(!store.delete_binding("elgato-mk2-abc", 2).unwrap());
        assert_eq!(store.binding("elgato-mk2-abc", 2).unwrap(), None);
    }

    #[test]
    fn deck_lights_set_and_clear() {
        let store = Store::in_memory().unwrap();
        store.set_deck_light("m", Some("#112233")).unwrap();
        assert_eq!(store.deck_light("m").unwrap().as_deref(), Some("#112233"));
        store.set_deck_light("m", None).unwrap();
        assert_eq!(store.deck_light("m").unwrap(), None);
    }

    #[test]
    fn dial_colors_set_and_clear() {
        let store = Store::in_memory().unwrap();
        store.set_dial_color("d", 1, Some("#ff0000")).unwrap();
        store.set_dial_color("d", 1, Some("#00ff00")).unwrap();
        store.set_dial_color("d", 3, Some("#0000ff")).unwrap();
        store.set_dial_color("d", 3, None).unwrap();
        let colors = store.dial_colors("d").unwrap();
        assert_eq!(colors.len(), 1);
        assert_eq!(colors[&1], "#00ff00");
        assert_eq!(store.known_devices().unwrap(), vec!["d".to_owned()]);
    }

    #[test]
    fn brightness_and_known_devices() {
        let store = Store::in_memory().unwrap();
        assert_eq!(store.brightness("d").unwrap(), None);
        store.set_brightness("d", 140).unwrap();
        assert_eq!(store.brightness("d").unwrap(), Some(100));
        store.put_binding(&binding(1, Action::None)).unwrap();
        assert_eq!(
            store.known_devices().unwrap(),
            vec!["d".to_owned(), "elgato-mk2-abc".to_owned()]
        );
    }

    #[test]
    fn actions_serialize_with_a_type_tag() {
        let ask = Action::AskAgent {
            prompt: "hi".to_owned(),
            agent_id: None,
        };
        assert_eq!(
            serde_json::to_value(&ask).unwrap(),
            serde_json::json!({ "type": "ask_agent", "prompt": "hi", "agent_id": null })
        );
        assert_eq!(
            serde_json::from_str::<Action>(r#"{"type":"none"}"#).unwrap(),
            Action::None
        );
        assert!(serde_json::from_str::<Action>(r#"{"type":"run_shell"}"#).is_err());
    }
}
