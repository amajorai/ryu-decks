//! Control decks for Ryu: Elgato Stream Deck, Ulanzi D200, Work Louder Creator
//! Micro 2 / Codex Micro, Loupedeck / Razer Stream Controller and Mirabox / Ajazz
//! keys that raise Ryu app events and ask agents.

pub mod api;
pub mod catalog;
pub mod dispatch;
pub mod driver;
pub mod host;
pub mod manager;
pub mod paths;
pub mod settings;
pub mod store;

pub const PLUGIN_ID: &str = "@ryu/decks";
pub const DEFAULT_PORT: u16 = 8029;
