//! Logitech G-key RGB keyboards on HID++ 2.0: the G915 (wired) and G815, whose
//! lights are zones of `PerKeyLighting2` (0x8081), and the G910 Orion Spark and
//! Spectrum, G513 and G Pro keyboard, whose lights are the older `PerKeyLighting`
//! (0x8080) key groups. The G915, G815 and G910 G-keys are the deck's keys;
//! the G513 and G Pro have none, so only their backlight (the deck light) is
//! driven. The rest of each keyboard keeps typing through its own interface.
//!
//! Everything goes through the HID++ vendor collection (the wired G915's
//! interface 2, usage page 0xFF00, usage 2; the others' interface 1, usage
//! page 0xFF43, usage 0x0602) at device index 0xFF. A request is `[report, 0xff, feature index,
//! function << 4 | software id, params..]`, short (`0x10`, 7 bytes) or long
//! (`0x11`, 20 bytes); the device takes one at a time and echoes the first
//! four bytes in its answer, or answers `[.., 0xff, 0xff, index, function,
//! code]` with an error. Reports that arrive while an answer is awaited
//! (G-key presses) are queued for the next poll. Feature indexes are looked
//! up through the root feature (index 0) when the board opens, with each
//! model's known indexes kept as fallbacks.
//!
//! The G-keys report nothing until GamingGKeys (0x8010) function 2 diverts
//! them; a notification on that index then holds every G-key's state as a
//! little-endian bitmask, bit 0 for G1. Diversion is turned off again when
//! the deck closes, so the G-keys go back to their own actions.
//!
//! Lights are taken only once something is lit (a deck light colour or a
//! bound G-key), so a keyboard with nothing set keeps its own effect. On the
//! 0x8081 boards Ryu claims software control through RgbEffects (0x8071),
//! sets each cluster's static effect as the base colour, paints the zones and
//! commits the frame (nothing shows until it is committed); software control
//! is given back when nothing is lit any more, and when the deck closes. On
//! the 0x8080 boards a running onboard effect wins over per-key writes, so a
//! fixed colour is first set through ColorLedEffects (0x8070); key groups then
//! go out as unanswered 64-byte `0x12` frames, fenced by a root ping, and are
//! committed. Those boards have no software control to give back: with
//! nothing lit they go dark until they are replugged or their own software
//! takes them again. The firmware and Logitech G HUB retake the lights (and
//! the G-keys) from time to time, so the heartbeat asserts them again. There
//! is no brightness command; the brightness scales the colours.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

const SHORT_REPORT: u8 = 0x10;
const LONG_REPORT: u8 = 0x11;
const VERY_LONG_REPORT: u8 = 0x12;
const SHORT_LEN: usize = 7;
const LONG_LEN: usize = 20;
const VERY_LONG_LEN: usize = 64;
/// The device index of a keyboard on its own cable.
const DEVICE_INDEX: u8 = 0xff;
/// The error answer's marker, in place of the feature index.
const ERROR_MARK: u8 = 0xff;
const SOFTWARE_ID: u8 = 0x0f;

/// HID++ feature ids.
const ROOT_INDEX: u8 = 0x00;
const GAMING_G_KEYS: u16 = 0x8010;
const COLOR_LED_EFFECTS: u16 = 0x8070;
const RGB_EFFECTS: u16 = 0x8071;
const PER_KEY_LIGHTING: u16 = 0x8080;
const PER_KEY_LIGHTING_2: u16 = 0x8081;

/// Root functions.
const GET_FEATURE: u8 = 0;
const PING: u8 = 1;
/// GamingGKeys function 2 turns host reporting (diversion) on or off.
const DIVERT_G_KEYS: u8 = 2;
/// PerKeyLighting2 functions.
const ZONE_INFO: u8 = 0;
const SET_ZONES_INDIVIDUAL: u8 = 1;
const SET_ZONES_SINGLE_VALUE: u8 = 6;
const FRAME_END_2: u8 = 7;
/// RgbEffects functions.
const RGB_INFO: u8 = 0;
const SET_CLUSTER_EFFECT: u8 = 1;
const SW_CONTROL: u8 = 5;
/// ColorLedEffects functions.
const LED_INFO: u8 = 0;
const LED_ZONE_INFO: u8 = 1;
const LED_EFFECT_INFO: u8 = 2;
const SET_ZONE_EFFECT: u8 = 3;
/// PerKeyLighting functions.
const SET_KEY_GROUP: u8 = 3;
const FRAME_END: u8 = 5;

/// The effect id of a static, single-colour effect (both effect features).
const STATIC_EFFECT: u16 = 0x0001;
/// "Every cluster" and "every effect" in an RgbEffects info request.
const ALL: u8 = 0xff;
/// Software-control flag that takes every cluster.
const ALL_CLUSTERS: u8 = 0x01;
/// RgbEffects persistence: applied to RAM only.
const RGB_VOLATILE: u8 = 0x01;
/// ColorLedEffects persistence: applied to RAM only.
const LED_VOLATILE: u8 = 0x00;

/// Zone ids 0 and 0xFF end a list; neither is ever a zone.
const LIST_END: u8 = 0xff;
/// The G-key zones of the 0x8081 boards, G1 first.
const G_KEY_ZONE: u8 = 0xb4;
const ZONES_PER_SINGLE_VALUE: usize = 13;
const ZONES_PER_INDIVIDUAL: usize = 4;
/// Zone presence: three pages of a 14-byte bitfield, 112 zones a page.
const PRESENCE_PAGES: u8 = 3;
const PRESENCE_BYTES: usize = 14;
const ZONES_PER_PAGE: u16 = 112;

/// PerKeyLighting key groups.
const GROUP_KEYS: u8 = 0x01;
const GROUP_G_KEYS: u8 = 0x04;
const GROUP_LOGO: u8 = 0x10;
/// One key group frame: an 8-byte header, then (id, R, G, B) entries.
const GROUP_HEADER_LEN: usize = 8;
/// The keys group's ids are HID keyboard usages: A to Menu, the ABNT slash,
/// and the eight modifiers.
const KEY_USAGES: [std::ops::RangeInclusive<u8>; 3] = [0x04..=0x65, 0x87..=0x87, 0xe0..=0xe7];

/// How long an answer is waited for.
const REQUEST_TIMEOUT: Duration = Duration::from_millis(100);
/// Pause between effect writes; the controller may drop closer ones.
const EFFECT_GAP: Duration = Duration::from_millis(8);
/// Bounds on what an info answer may claim, against a garbled one.
const MAX_CLUSTERS: u8 = 8;
const MAX_EFFECTS: u8 = 16;

const READ_BUFFER: usize = 64;
const MAX_REPORTS_PER_POLL: usize = 32;

/// The G-keys' printed names, G1 first.
pub const KEY_NAMES: [&str; 9] = ["G1", "G2", "G3", "G4", "G5", "G6", "G7", "G8", "G9"];

/// The G-keys' names on a model of this family.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    is_family(model.protocol).then(|| {
        KEY_NAMES
            .iter()
            .copied()
            .take(usize::from(model.keys))
            .collect()
    })
}

const fn is_family(protocol: Protocol) -> bool {
    matches!(
        protocol,
        Protocol::LogitechPerKeyLighting2 | Protocol::LogitechPerKeyLighting
    )
}

/// The feature indexes a model's firmware is known to use, for when the root
/// feature does not answer: the G915's from codex-agent-indicator, the
/// others' from g810-led.
struct Known {
    g_keys: Option<u8>,
    lighting: u8,
    effects: Option<u8>,
}

fn known_indexes(model_id: &str) -> Known {
    match model_id {
        "logitech-g915" => Known {
            g_keys: None,
            lighting: 0x0a,
            effects: Some(0x09),
        },
        "logitech-g815" => Known {
            g_keys: Some(0x0a),
            lighting: 0x10,
            effects: Some(0x0f),
        },
        "logitech-g910" => Known {
            g_keys: Some(0x08),
            lighting: 0x0f,
            effects: Some(0x10),
        },
        // The G513 and G Pro.
        _ => Known {
            g_keys: None,
            lighting: 0x0c,
            effects: Some(0x0d),
        },
    }
}

/// The zones the G915 is known to have, for when it does not list them.
fn known_zones() -> Vec<u8> {
    (0x01..=0x6f)
        .chain([0x99])
        .chain(0x9b..=0x9e)
        .chain(0xb4..=0xb8)
        .chain([0xd2])
        .collect()
}

/// The zone ids a presence page's bitfield lists, bit 0 of each byte first.
fn present_zones(page: u8, bitfield: &[u8]) -> Vec<u8> {
    let base = u16::from(page) * ZONES_PER_PAGE;
    let mut zones = Vec::new();
    for (byte_index, byte) in bitfield.iter().take(PRESENCE_BYTES).enumerate() {
        for bit in 0..8u16 {
            if byte & (1 << bit) == 0 {
                continue;
            }
            let id = base + byte_index as u16 * 8 + bit;
            if let Ok(zone) = u8::try_from(id) {
                if zone != 0 && zone != LIST_END {
                    zones.push(zone);
                }
            }
        }
    }
    zones
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// PerKeyLighting2 (0x8081) and RgbEffects (0x8071): the G915 and G815.
    Zones,
    /// PerKeyLighting (0x8080) and ColorLedEffects (0x8070): the G910, G513 and G Pro.
    Groups,
}

/// What the root feature said about a feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeatureLookup {
    Found(u8),
    /// Answered: the board does not have it.
    Absent,
    /// Timed out or refused: the model's known index may stand in.
    NoAnswer,
}

/// How a request ended.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    /// The whole answering report.
    Accepted(Vec<u8>),
    /// A HID++ error answer, with its error code.
    Refused(u8),
    TimedOut,
}

impl Answer {
    /// Byte `at` of the answer's parameters (after the 4-byte header).
    fn param(&self, at: usize) -> Option<u8> {
        match self {
            Self::Accepted(report) => report.get(4 + at).copied(),
            _ => None,
        }
    }
}

pub struct RgbKeyboard {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    kind: Kind,
    ready: bool,
    /// Whether the board takes short requests: unknown until one is answered
    /// or refused by the host, then remembered.
    short_works: Option<bool>,
    g_keys: Option<u8>,
    lighting: u8,
    /// The root feature said the board has no lighting feature.
    lighting_absent: bool,
    effects: Option<u8>,
    /// The 0x8081 zones the deck light paints (every zone but the G-keys').
    backlight_zones: Vec<u8>,
    /// The 0x8081 G-key zones the board lists, G1 first.
    g_key_zones: Vec<u8>,
    /// The (zone or cluster, effect index) of each static effect, once read.
    static_effects: Option<Vec<(u8, u8)>>,
    /// Software control as it was before Ryu claimed it (control, events).
    previous_control: (u8, u8),
    claimed: bool,
    /// The G-keys held down, bit 0 for G1.
    held: u32,
    pending: VecDeque<Input>,
    /// A HID++ error answer to an earlier, unanswered write: (index, function, code).
    stray_error: Option<(u8, u8, u8)>,
    key_colors: Vec<Option<[u8; 3]>>,
    deck_color: Option<[u8; 3]>,
    brightness: u8,
    dirty: bool,
}

impl RgbKeyboard {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        let kind = if model.protocol == Protocol::LogitechPerKeyLighting2 {
            Kind::Zones
        } else {
            Kind::Groups
        };
        let known = known_indexes(model.id);
        Self {
            model,
            transport,
            kind,
            ready: false,
            short_works: None,
            g_keys: known.g_keys,
            lighting: known.lighting,
            lighting_absent: false,
            effects: known.effects,
            backlight_zones: Vec::new(),
            g_key_zones: Vec::new(),
            static_effects: None,
            previous_control: (0, 0),
            claimed: false,
            held: 0,
            pending: VecDeque::new(),
            stray_error: None,
            key_colors: vec![None; usize::from(model.keys)],
            deck_color: None,
            brightness: 100,
            dirty: true,
        }
    }

    const fn function_byte(function: u8) -> u8 {
        (function << 4) | SOFTWARE_ID
    }

    fn packet(long: bool, index: u8, function: u8, params: &[u8]) -> Vec<u8> {
        let (report, len) = if long {
            (LONG_REPORT, LONG_LEN)
        } else {
            (SHORT_REPORT, SHORT_LEN)
        };
        let mut packet = vec![report, DEVICE_INDEX, index, Self::function_byte(function)];
        packet.extend_from_slice(params);
        packet.resize(len, 0);
        packet
    }

    /// A request with up to three parameters: short where the board takes
    /// short ones, else long.
    fn call(&mut self, index: u8, function: u8, params: &[u8]) -> DriverResult<Answer> {
        if params.len() <= SHORT_LEN - 4 && self.short_works != Some(false) {
            let short = Self::packet(false, index, function, params);
            match self.transport.write(&short) {
                Ok(()) => {
                    let answer = self.await_answer(index, Self::function_byte(function))?;
                    if answer != Answer::TimedOut {
                        self.short_works = Some(true);
                        return Ok(answer);
                    }
                    if self.short_works == Some(true) {
                        return Ok(answer);
                    }
                }
                // The host refuses a report the collection does not declare.
                Err(_) => self.short_works = Some(false),
            }
        }
        let answer = self.call_long(index, function, params)?;
        if answer != Answer::TimedOut && self.short_works.is_none() {
            self.short_works = Some(false);
        }
        Ok(answer)
    }

    fn call_long(&mut self, index: u8, function: u8, params: &[u8]) -> DriverResult<Answer> {
        self.transport
            .write(&Self::packet(true, index, function, params))?;
        self.await_answer(index, Self::function_byte(function))
    }

    /// Reads until the answer to `(index, function)` arrives, queueing input
    /// and noting error answers to earlier writes.
    fn await_answer(&mut self, index: u8, function: u8) -> DriverResult<Answer> {
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let mut buf = [0u8; READ_BUFFER];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(Answer::TimedOut);
            }
            let wait = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                return Ok(Answer::TimedOut);
            }
            let report = &buf[..read];
            let is_answer = matches!(report.first(), Some(&(SHORT_REPORT | LONG_REPORT)))
                && report.get(1) == Some(&DEVICE_INDEX);
            if is_answer {
                match (report.get(2), report.get(3), report.get(4), report.get(5)) {
                    (Some(&ERROR_MARK), Some(&failed), Some(&echo), Some(&code)) => {
                        if failed == index && echo == function {
                            return Ok(Answer::Refused(code));
                        }
                        self.stray_error.get_or_insert((failed, echo, code));
                        continue;
                    }
                    (Some(&answered), Some(&echo), ..) if answered == index && echo == function => {
                        return Ok(Answer::Accepted(report.to_vec()));
                    }
                    _ => {}
                }
            }
            let inputs = self.parse(report);
            self.pending.extend(inputs);
        }
    }

    /// Whether a request was taken; a refusal or silence is logged.
    fn expect(&self, answer: &Answer, what: &str) -> bool {
        match answer {
            Answer::Accepted(_) => true,
            Answer::Refused(code) => {
                tracing::debug!(
                    model = self.model.id,
                    code,
                    "ryu-decks: the keyboard refused {what}"
                );
                false
            }
            Answer::TimedOut => {
                tracing::debug!(
                    model = self.model.id,
                    "ryu-decks: the keyboard did not answer {what}"
                );
                false
            }
        }
    }

    /// The index of a feature, through the root feature: the board's own
    /// answer, which says it has none (index 0) or does not answer at all.
    fn feature_index(&mut self, feature: u16) -> DriverResult<FeatureLookup> {
        let [high, low] = feature.to_be_bytes();
        let answer = self.call(ROOT_INDEX, GET_FEATURE, &[high, low, 0])?;
        Ok(match answer.param(0) {
            Some(0) => FeatureLookup::Absent,
            Some(index) => FeatureLookup::Found(index),
            None => FeatureLookup::NoAnswer,
        })
    }

    /// Looks the features up, reads what the lights are made of and diverts
    /// the G-keys, once.
    fn ensure_ready(&mut self) -> DriverResult<()> {
        if self.ready {
            return Ok(());
        }
        self.ready = true;
        let (lighting, effects) = match self.kind {
            Kind::Zones => (PER_KEY_LIGHTING_2, RGB_EFFECTS),
            Kind::Groups => (PER_KEY_LIGHTING, COLOR_LED_EFFECTS),
        };
        // The known indexes stand in only for a root feature that does not
        // answer; one that says the feature is absent is believed.
        match self.feature_index(lighting)? {
            FeatureLookup::Found(index) => self.lighting = index,
            FeatureLookup::Absent => self.lighting_absent = true,
            FeatureLookup::NoAnswer => {}
        }
        match self.feature_index(effects)? {
            FeatureLookup::Found(index) => self.effects = Some(index),
            FeatureLookup::Absent => self.effects = None,
            FeatureLookup::NoAnswer => {}
        }
        if self.model.keys > 0 {
            match self.feature_index(GAMING_G_KEYS)? {
                FeatureLookup::Found(index) => self.g_keys = Some(index),
                FeatureLookup::Absent => self.g_keys = None,
                FeatureLookup::NoAnswer => {}
            }
        }
        match self.kind {
            Kind::Zones if self.lighting_absent => {}
            Kind::Zones => self.read_zones()?,
            Kind::Groups => self.read_fixed_effects()?,
        }
        self.divert(true)?;
        Ok(())
    }

    /// The zones the board lists, split into the backlight's and the G-keys'.
    fn read_zones(&mut self) -> DriverResult<()> {
        let mut zones = Vec::new();
        for page in 0..PRESENCE_PAGES {
            let answer = self.call(self.lighting, ZONE_INFO, &[0x00, page, 0x00])?;
            // The bitfield fills a long answer from its third parameter on.
            let bitfield = match &answer {
                Answer::Accepted(report) if report.len() >= LONG_LEN => &report[6..LONG_LEN],
                _ => {
                    tracing::debug!(
                        model = self.model.id,
                        "ryu-decks: the keyboard did not list its light zones; using the known ones"
                    );
                    zones = known_zones();
                    break;
                }
            };
            zones.extend(present_zones(page, bitfield));
        }
        let g_zones: Vec<u8> = (0..self.model.keys).map(|key| G_KEY_ZONE + key).collect();
        self.g_key_zones = g_zones
            .iter()
            .copied()
            .filter(|zone| zones.contains(zone))
            .collect();
        self.backlight_zones = zones
            .into_iter()
            .filter(|zone| !g_zones.contains(zone))
            .collect();
        Ok(())
    }

    /// Each ColorLedEffects zone's fixed-colour effect, from the board's own
    /// lists; a zone whose list cannot be read is left out.
    fn read_fixed_effects(&mut self) -> DriverResult<()> {
        let Some(index) = self.effects else {
            self.static_effects = Some(Vec::new());
            return Ok(());
        };
        let mut targets = Vec::new();
        let zones = self
            .call(index, LED_INFO, &[0, 0, 0])?
            .param(0)
            .unwrap_or(0);
        for zone in 0..zones.min(MAX_CLUSTERS) {
            let info = self.call(index, LED_ZONE_INFO, &[zone, 0, 0])?;
            if info.param(0) != Some(zone) {
                continue;
            }
            let effects = info.param(3).unwrap_or(0).min(MAX_EFFECTS);
            for effect in 0..effects {
                let info = self.call(index, LED_EFFECT_INFO, &[zone, effect, 0])?;
                let id = info
                    .param(2)
                    .zip(info.param(3))
                    .map(|(high, low)| u16::from_be_bytes([high, low]));
                if id == Some(STATIC_EFFECT) {
                    targets.push((zone, effect));
                    break;
                }
            }
        }
        if targets.is_empty() {
            tracing::debug!(
                model = self.model.id,
                "ryu-decks: no fixed colour effect found; the onboard effect may stay on"
            );
        }
        self.static_effects = Some(targets);
        Ok(())
    }

    /// Each RgbEffects cluster's static effect, read once; codex-agent-indicator's
    /// clusters 0 and 1, effect 0, when the board does not list them.
    fn read_static_clusters(&mut self, index: u8) -> DriverResult<Vec<(u8, u8)>> {
        let mut targets = Vec::new();
        let clusters = self
            .call(index, RGB_INFO, &[ALL, ALL, 0])?
            .param(2)
            .unwrap_or(0);
        for cluster in 0..clusters.min(MAX_CLUSTERS) {
            let effects = self
                .call(index, RGB_INFO, &[cluster, ALL, 0])?
                .param(4)
                .unwrap_or(0);
            for effect in 0..effects.min(MAX_EFFECTS) {
                let info = self.call(index, RGB_INFO, &[cluster, effect, 0])?;
                let id = info
                    .param(2)
                    .zip(info.param(3))
                    .map(|(high, low)| u16::from_be_bytes([high, low]));
                if id == Some(STATIC_EFFECT) {
                    targets.push((cluster, effect));
                    break;
                }
            }
        }
        if targets.is_empty() {
            targets = vec![(0, 0), (1, 0)];
        }
        Ok(targets)
    }

    /// Turns G-key reporting to the host on or off, waiting for the answer.
    fn divert(&mut self, on: bool) -> DriverResult<bool> {
        let Some(index) = self.g_keys else {
            return Ok(true);
        };
        if self.model.keys == 0 {
            return Ok(true);
        }
        let answer = self.call_long(index, DIVERT_G_KEYS, &[u8::from(on)])?;
        Ok(self.expect(&answer, "the G-key diversion"))
    }

    /// A colour at the current brightness.
    fn scaled(&self, color: Option<[u8; 3]>) -> [u8; 3] {
        let percent = u16::from(self.brightness.min(100));
        color
            .unwrap_or([0; 3])
            .map(|channel| u8::try_from(u16::from(channel) * percent / 100).unwrap_or(u8::MAX))
    }

    fn wants_lights(&self) -> bool {
        self.deck_color.is_some() || self.key_colors.iter().any(Option::is_some)
    }

    /// Takes the lights from the firmware and sets the base colour. Returns
    /// whether the first frame should be painted twice (a fresh claim).
    fn claim(&mut self) -> DriverResult<bool> {
        let base = self.scaled(self.deck_color);
        match self.kind {
            Kind::Zones => {
                let Some(index) = self.effects else {
                    return Ok(false);
                };
                let current = self.call(index, SW_CONTROL, &[0, 0, 0])?;
                if let (Some(control), Some(events)) = (current.param(1), current.param(2)) {
                    self.previous_control = (control, events);
                }
                let (control, events) = self.previous_control;
                let fresh = control & ALL_CLUSTERS == 0;
                let answer = self.call(index, SW_CONTROL, &[1, control | ALL_CLUSTERS, events])?;
                self.expect(&answer, "software control of the lights");
                if self.static_effects.is_none() {
                    self.static_effects = Some(self.read_static_clusters(index)?);
                }
                for (cluster, effect) in self.static_effects.clone().unwrap_or_default() {
                    let mut params = [0u8; 16];
                    params[..5].copy_from_slice(&[cluster, effect, base[0], base[1], base[2]]);
                    params[12] = RGB_VOLATILE;
                    // Any answer will do: some firmware answers it late.
                    self.call_long(index, SET_CLUSTER_EFFECT, &params)?;
                    std::thread::sleep(EFFECT_GAP);
                }
                Ok(fresh)
            }
            Kind::Groups => {
                let Some(index) = self.effects else {
                    return Ok(false);
                };
                for (zone, effect) in self.static_effects.clone().unwrap_or_default() {
                    let mut params = [0u8; 16];
                    params[..5].copy_from_slice(&[zone, effect, base[0], base[1], base[2]]);
                    params[12] = LED_VOLATILE;
                    let answer = self.call_long(index, SET_ZONE_EFFECT, &params)?;
                    self.expect(&answer, "the fixed colour effect");
                    std::thread::sleep(EFFECT_GAP);
                }
                Ok(false)
            }
        }
    }

    /// Gives software control back to the firmware (0x8081 boards).
    fn release(&mut self) -> DriverResult<()> {
        if self.kind != Kind::Zones {
            return Ok(());
        }
        let Some(index) = self.effects else {
            return Ok(());
        };
        let (control, events) = self.previous_control;
        let answer = self.call(index, SW_CONTROL, &[1, control & !ALL_CLUSTERS, events])?;
        self.expect(&answer, "the lights back");
        Ok(())
    }

    /// Paints every light and commits the frame.
    fn paint(&mut self) -> DriverResult<bool> {
        // A board without the lighting feature has nothing to paint, and the
        // frames would land on whatever feature sits at the known index.
        if self.lighting_absent {
            return Ok(true);
        }
        match self.kind {
            Kind::Zones => self.paint_zones(),
            Kind::Groups => self.paint_groups(),
        }
    }

    fn paint_zones(&mut self) -> DriverResult<bool> {
        let [r, g, b] = self.scaled(self.deck_color);
        let backlight = self.backlight_zones.clone();
        for chunk in backlight.chunks(ZONES_PER_SINGLE_VALUE) {
            let mut params = vec![r, g, b];
            params.extend_from_slice(chunk);
            if chunk.len() < ZONES_PER_SINGLE_VALUE {
                params.push(LIST_END);
            }
            let answer = self.call_long(self.lighting, SET_ZONES_SINGLE_VALUE, &params)?;
            if !self.expect(&answer, "the backlight colour") {
                return Ok(false);
            }
        }
        let keys: Vec<(u8, [u8; 3])> = self
            .g_key_zones
            .iter()
            .map(|zone| {
                let key = usize::from(zone - G_KEY_ZONE);
                (
                    *zone,
                    self.scaled(self.key_colors.get(key).copied().flatten()),
                )
            })
            .collect();
        for chunk in keys.chunks(ZONES_PER_INDIVIDUAL) {
            let mut params = Vec::new();
            for (zone, [r, g, b]) in chunk {
                params.extend_from_slice(&[*zone, *r, *g, *b]);
            }
            if chunk.len() < ZONES_PER_INDIVIDUAL {
                params.push(LIST_END);
            }
            let answer = self.call_long(self.lighting, SET_ZONES_INDIVIDUAL, &params)?;
            if !self.expect(&answer, "a G-key colour") {
                return Ok(false);
            }
        }
        // Volatile, no frame numbers: show it now.
        let answer = self.call_long(self.lighting, FRAME_END_2, &[0, 0, 0, 0, 0])?;
        Ok(self.expect(&answer, "the frame"))
    }

    /// One key group's frames, `per_frame` entries each; byte 7 is the count
    /// this frame carries.
    fn group_frames(index: u8, group: u8, entries: &[(u8, [u8; 3])], long: bool) -> Vec<Vec<u8>> {
        let (report, len) = if long {
            (VERY_LONG_REPORT, VERY_LONG_LEN)
        } else {
            (LONG_REPORT, LONG_LEN)
        };
        let per_frame = (len - GROUP_HEADER_LEN) / 4;
        entries
            .chunks(per_frame)
            .map(|chunk| {
                let count = u8::try_from(chunk.len()).unwrap_or(u8::MAX);
                let mut frame = vec![
                    report,
                    DEVICE_INDEX,
                    index,
                    Self::function_byte(SET_KEY_GROUP),
                    0x00,
                    group,
                    0x00,
                    count,
                ];
                for (id, [r, g, b]) in chunk {
                    frame.extend_from_slice(&[*id, *r, *g, *b]);
                }
                frame.resize(len, 0);
                frame
            })
            .collect()
    }

    fn paint_groups(&mut self) -> DriverResult<bool> {
        let index = self.lighting;
        let backlight = self.scaled(self.deck_color);
        let keys: Vec<(u8, [u8; 3])> = KEY_USAGES
            .iter()
            .flat_map(Clone::clone)
            .map(|usage| (usage, backlight))
            .collect();
        let mut frames = Self::group_frames(index, GROUP_KEYS, &keys, true);
        // The logo: two lights on the G910, one on the G Pro; the G513 takes none.
        let logos: &[u8] = match self.model.id {
            "logitech-g910" => &[1, 2],
            "logitech-g-pro-keyboard" => &[1],
            _ => &[],
        };
        let logo: Vec<(u8, [u8; 3])> = logos.iter().map(|id| (*id, backlight)).collect();
        frames.extend(Self::group_frames(index, GROUP_LOGO, &logo, false));
        let g_keys: Vec<(u8, [u8; 3])> = (0..self.model.keys)
            .map(|key| {
                let color = self.key_colors.get(usize::from(key)).copied().flatten();
                (key + 1, self.scaled(color))
            })
            .collect();
        frames.extend(Self::group_frames(index, GROUP_G_KEYS, &g_keys, true));
        // The frames are never answered; an error answer names its frame.
        self.stray_error = None;
        for frame in &frames {
            self.transport.write(frame)?;
        }
        // The board answers in order, so once the ping is back any error for
        // a frame has been read.
        let ping = self.call(ROOT_INDEX, PING, &[0, 0, 0x5a])?;
        self.expect(&ping, "the ping");
        let frame_function = Self::function_byte(SET_KEY_GROUP);
        if let Some((_, _, code)) = self
            .stray_error
            .take()
            .filter(|(failed, echo, _)| *failed == index && *echo == frame_function)
        {
            tracing::debug!(
                model = self.model.id,
                code,
                "ryu-decks: the keyboard refused a key colour frame; not committing"
            );
            return Ok(false);
        }
        let answer = self.call_long(index, FRAME_END, &[])?;
        Ok(self.expect(&answer, "the frame"))
    }

    /// Brings the lights in line with what is set. `reassert` claims them again
    /// and repaints even when nothing changed (the heartbeat).
    fn sync(&mut self, reassert: bool) -> DriverResult<()> {
        if !self.wants_lights() {
            if self.claimed && self.dirty {
                match self.kind {
                    Kind::Zones => {
                        self.release()?;
                        self.claimed = false;
                    }
                    // Nothing to give back to: everything goes dark.
                    Kind::Groups => {
                        self.paint()?;
                    }
                }
            }
            self.dirty = false;
            return Ok(());
        }
        if !self.claimed {
            let fresh = self.claim()?;
            self.claimed = true;
            // The first frame after a fresh claim may be dropped while the
            // board leaves its onboard mode.
            if self.paint()? && fresh {
                self.paint()?;
            }
            self.dirty = false;
            return Ok(());
        }
        if reassert && self.kind == Kind::Zones {
            if let Some(index) = self.effects {
                let (control, events) = self.previous_control;
                let answer = self.call(index, SW_CONTROL, &[1, control | ALL_CLUSTERS, events])?;
                self.expect(&answer, "software control of the lights");
            }
        }
        if (self.dirty || reassert) && self.paint()? {
            self.dirty = false;
        }
        Ok(())
    }

    fn parse(&mut self, report: &[u8]) -> Vec<Input> {
        let mut events = Vec::new();
        let Some(index) = self.g_keys else {
            return events;
        };
        // A notification: function 0, software id 0, then the G-key bitmask.
        let (Some(&LONG_REPORT), Some(&DEVICE_INDEX), Some(&feature), Some(&0), Some(mask)) = (
            report.first(),
            report.get(1),
            report.get(2),
            report.get(3),
            report.get(4..8),
        ) else {
            return events;
        };
        if feature != index {
            return events;
        }
        let held = u32::from_le_bytes([mask[0], mask[1], mask[2], mask[3]]);
        for key in 0..self.model.keys.min(32) {
            let bit = 1u32 << key;
            if (held ^ self.held) & bit != 0 {
                events.push(Input::Key {
                    index: key,
                    pressed: held & bit != 0,
                });
            }
        }
        let keys_mask = if self.model.keys >= 32 {
            u32::MAX
        } else {
            (1u32 << self.model.keys) - 1
        };
        self.held = held & keys_mask;
        events
    }
}

impl Driver for RgbKeyboard {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        self.ensure_ready()?;
        let mut events: Vec<Input> = self.pending.drain(..).collect();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 && events.is_empty() {
                timeout_ms
            } else {
                0
            };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            events.extend(self.parse(&buf[..read]));
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        let color = face.map(|face| face.color);
        if let Some(slot) = self.key_colors.get_mut(usize::from(key)) {
            if *slot != color {
                *slot = color;
                self.dirty = true;
            }
        }
        Ok(())
    }

    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        if self.deck_color != color {
            self.deck_color = color;
            self.dirty = true;
        }
        Ok(())
    }

    fn flush(&mut self) -> DriverResult<()> {
        self.ensure_ready()?;
        self.sync(false)
    }

    /// No brightness command: the colours are scaled instead.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        let percent = percent.min(100);
        if percent != self.brightness {
            self.brightness = percent;
            self.dirty = true;
        }
        self.flush()
    }

    /// The firmware and G HUB retake the lights and the G-keys: claim them again.
    fn heartbeat(&mut self) -> DriverResult<()> {
        if !self.ready {
            return Ok(());
        }
        self.divert(true)?;
        self.sync(true)
    }
}

impl Drop for RgbKeyboard {
    /// Hands the G-keys and the lights back without waiting for answers.
    fn drop(&mut self) {
        if !self.ready {
            return;
        }
        let mut goodbye = Vec::new();
        if let Some(index) = self.g_keys.filter(|_| self.model.keys > 0) {
            goodbye.push(Self::packet(true, index, DIVERT_G_KEYS, &[0]));
        }
        if self.claimed && self.kind == Kind::Zones {
            if let Some(index) = self.effects {
                let (control, events) = self.previous_control;
                let long = self.short_works == Some(false);
                goodbye.push(Self::packet(
                    long,
                    index,
                    SW_CONTROL,
                    &[1, control & !ALL_CLUSTERS, events],
                ));
            }
        }
        for packet in goodbye {
            if let Err(error) = self.transport.write(&packet) {
                tracing::debug!(model = self.model.id, %error, "ryu-decks: could not hand the keyboard back");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;
    use crate::driver::DriverError;

    /// A keyboard that answers like the real one: features at fixed indexes,
    /// the zones it lists, the effects it has, and nothing for 0x12 frames.
    #[derive(Clone)]
    struct Board {
        recorder: Recorder,
        features: Vec<(u16, u8)>,
        /// Zone presence pages, or `None` to refuse the request.
        presence: Option<[[u8; PRESENCE_BYTES]; 3]>,
        /// Answers nothing at all.
        silent: bool,
        /// Refuses short reports, as Windows does on a long-only collection.
        long_only: bool,
        /// Errors the next key-group frame.
        refuse_frame: bool,
    }

    impl Board {
        fn new(features: &[(u16, u8)]) -> Self {
            Self {
                recorder: Recorder::default(),
                features: features.to_vec(),
                presence: None,
                silent: false,
                long_only: false,
                refuse_frame: false,
            }
        }

        fn index_of(&self, feature: u16) -> Option<u8> {
            self.features
                .iter()
                .find(|(id, _)| *id == feature)
                .map(|(_, index)| *index)
        }

        fn answer(&mut self, data: &[u8]) -> Option<Vec<u8>> {
            let (&report, &index, &function) = (data.first()?, data.get(2)?, data.get(3)?);
            let params = data.get(4..).unwrap_or_default();
            let mut reply = data[..4].to_vec();
            if report == VERY_LONG_REPORT {
                if std::mem::take(&mut self.refuse_frame) {
                    return Some(vec![0x11, 0xff, 0xff, index, function, 0x02, 0, 0]);
                }
                return None;
            }
            let fx = self.index_of(RGB_EFFECTS);
            let led = self.index_of(COLOR_LED_EFFECTS);
            let payload: Vec<u8> = match (index, function >> 4) {
                (ROOT_INDEX, GET_FEATURE) => {
                    let feature = u16::from_be_bytes([params[0], params[1]]);
                    vec![self.index_of(feature).unwrap_or(0)]
                }
                (i, ZONE_INFO) if Some(i) == self.index_of(PER_KEY_LIGHTING_2) => {
                    let pages = self.presence?;
                    let mut payload = vec![0x00, params[1]];
                    payload.extend_from_slice(&pages[usize::from(params[1])]);
                    reply[0] = LONG_REPORT;
                    payload
                }
                (i, SW_CONTROL) if Some(i) == fx && params[0] == 0 => vec![0x00, 0x00, 0x02],
                (i, RGB_INFO) if Some(i) == fx => match (params[0], params[1]) {
                    (ALL, ALL) => vec![ALL, ALL, 2],
                    (cluster, ALL) => vec![cluster, ALL, 0, 0, 3],
                    // Effect 2 of each cluster is the static one.
                    (cluster, effect) => vec![cluster, effect, 0, u8::from(effect == 2)],
                },
                (i, LED_INFO) if Some(i) == led => vec![1],
                (i, LED_ZONE_INFO) if Some(i) == led => {
                    reply[0] = LONG_REPORT;
                    vec![params[0], 0, 1, 2]
                }
                (i, LED_EFFECT_INFO) if Some(i) == led => {
                    vec![params[0], params[1], 0, u8::from(params[1] == 1)]
                }
                _ => params.to_vec(),
            };
            reply.extend_from_slice(&payload);
            // An answer too long for a short report comes back long.
            if reply.len() > SHORT_LEN {
                reply[0] = LONG_REPORT;
            }
            reply.resize(
                if reply[0] == SHORT_REPORT {
                    SHORT_LEN
                } else {
                    LONG_LEN
                },
                0,
            );
            Some(reply)
        }
    }

    impl Transport for Board {
        fn write(&mut self, data: &[u8]) -> DriverResult<()> {
            if self.long_only && data.first() == Some(&SHORT_REPORT) {
                return Err(DriverError::Io("report not declared".to_owned()));
            }
            self.recorder.write(data)?;
            if !self.silent {
                if let Some(reply) = self.answer(data) {
                    self.recorder.queue_read(reply);
                }
            }
            Ok(())
        }

        fn read_timeout(&mut self, buf: &mut [u8], timeout_ms: i32) -> DriverResult<usize> {
            self.recorder.read_timeout(buf, timeout_ms)
        }

        fn send_feature_report(&mut self, data: &[u8]) -> DriverResult<()> {
            self.recorder.send_feature_report(data)
        }

        fn get_feature_report(&mut self, buf: &mut [u8]) -> DriverResult<usize> {
            self.recorder.get_feature_report(buf)
        }
    }

    fn padded(prefix: &[u8], len: usize) -> Vec<u8> {
        let mut packet = prefix.to_vec();
        packet.resize(len, 0);
        packet
    }

    fn face(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    fn g915_board() -> Board {
        let mut board = Board::new(&[
            (GAMING_G_KEYS, 0x05),
            (PER_KEY_LIGHTING_2, 0x0a),
            (RGB_EFFECTS, 0x09),
        ]);
        // Zones 1 to 3 on page 0, the G-key zones 0xb4 to 0xb8 (page 1,
        // zone 112 + 68 = 180 on: byte 8 bits 4 to 7, byte 9 bit 0) and the
        // logo 0xd2 (page 1, 210 - 112 = 98: byte 12 bit 2).
        let mut pages = [[0u8; PRESENCE_BYTES]; 3];
        pages[0][0] = 0b0000_1110;
        pages[1][8] = 0b1111_0000;
        pages[1][9] = 0b0000_0001;
        pages[1][12] = 0b0000_0100;
        board.presence = Some(pages);
        board
    }

    fn open(model: &str, board: &Board) -> RgbKeyboard {
        RgbKeyboard::new(by_id(model).unwrap(), Box::new(board.clone()))
    }

    #[test]
    fn presence_bitfields_name_zones_bit_zero_first() {
        assert_eq!(present_zones(0, &[0b1000_0011, 0x01]), vec![1, 7, 8]);
        assert_eq!(present_zones(2, &[0x01]), vec![224]);
        // The last page's high bits run past 255, and 0xFF is no zone.
        let mut high = [0u8; PRESENCE_BYTES];
        high[3] = 0x80;
        high[4] = 0x01;
        assert!(present_zones(2, &high).is_empty());
        assert!(present_zones(0, &[]).is_empty());
    }

    #[test]
    fn g915_opens_by_looking_up_its_features_and_diverting_the_g_keys() {
        let board = g915_board();
        let mut deck = open("logitech-g915", &board);
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = board.recorder.writes();
        assert_eq!(
            writes[..6],
            [
                vec![0x10, 0xff, 0x00, 0x0f, 0x80, 0x81, 0x00],
                vec![0x10, 0xff, 0x00, 0x0f, 0x80, 0x71, 0x00],
                vec![0x10, 0xff, 0x00, 0x0f, 0x80, 0x10, 0x00],
                vec![0x10, 0xff, 0x0a, 0x0f, 0x00, 0x00, 0x00],
                vec![0x10, 0xff, 0x0a, 0x0f, 0x00, 0x01, 0x00],
                vec![0x10, 0xff, 0x0a, 0x0f, 0x00, 0x02, 0x00],
            ]
        );
        assert_eq!(writes[6], padded(&[0x11, 0xff, 0x05, 0x2f, 0x01], LONG_LEN));
        assert_eq!(
            writes.len(),
            7,
            "nothing is lit, so the lights are left alone"
        );
        assert_eq!(deck.backlight_zones, vec![1, 2, 3, 0xd2]);
        assert_eq!(deck.g_key_zones, vec![0xb4, 0xb5, 0xb6, 0xb7, 0xb8]);
    }

    #[test]
    fn g_key_bitmasks_become_presses_and_releases() {
        let board = g915_board();
        let mut deck = open("logitech-g915", &board);
        deck.poll(0).unwrap();
        // G1 and G3 down, then G3 up and G5 down; bits past G5 are not keys.
        board
            .recorder
            .queue_read(padded(&[0x11, 0xff, 0x05, 0x00, 0x05], LONG_LEN));
        board.recorder.queue_read(padded(
            &[0x11, 0xff, 0x05, 0x00, 0x11, 0x00, 0x00, 0x80],
            LONG_LEN,
        ));
        // Another feature's notification, an answer and a short read are not input.
        board
            .recorder
            .queue_read(padded(&[0x11, 0xff, 0x0a, 0x00, 0xff], LONG_LEN));
        board
            .recorder
            .queue_read(padded(&[0x11, 0xff, 0x05, 0x2f, 0xff], LONG_LEN));
        board
            .recorder
            .queue_read(vec![0x11, 0xff, 0x05, 0x00, 0xff]);
        board
            .recorder
            .queue_read(padded(&[0x11, 0xff, 0x05, 0x00], LONG_LEN));
        let key = |index, pressed| Input::Key { index, pressed };
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(2, true),
                key(2, false),
                key(4, true),
                key(0, false),
                key(4, false),
            ]
        );
    }

    #[test]
    fn a_press_while_an_answer_is_awaited_is_kept_for_the_next_poll() {
        let mut board = g915_board();
        board.silent = true;
        let mut deck = open("logitech-g915", &board);
        deck.ready = true;
        deck.g_keys = Some(0x05);
        board
            .recorder
            .queue_read(padded(&[0x11, 0xff, 0x05, 0x00, 0x02], LONG_LEN));
        board
            .recorder
            .queue_read(padded(&[0x11, 0xff, 0x05, 0x2f, 0x01], LONG_LEN));
        assert!(deck.divert(true).unwrap());
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 1,
                pressed: true
            }]
        );
    }

    #[test]
    fn g915_claims_software_control_and_paints_zones_then_commits() {
        let board = g915_board();
        let mut deck = open("logitech-g915", &board);
        deck.poll(0).unwrap();
        let opened = board.recorder.writes().len();
        deck.set_deck_light(Some([200, 100, 50])).unwrap();
        deck.set_face(1, Some(&face([10, 20, 30]))).unwrap();
        deck.set_brightness(50).unwrap();
        let writes = board.recorder.writes()[opened..].to_vec();
        // Get software control (none held), then take every cluster.
        assert_eq!(writes[0], vec![0x10, 0xff, 0x09, 0x5f, 0x00, 0x00, 0x00]);
        assert_eq!(writes[1], vec![0x10, 0xff, 0x09, 0x5f, 0x01, 0x01, 0x02]);
        // Two clusters' effect lists, then each one's static effect (index 2).
        assert_eq!(writes[2], vec![0x10, 0xff, 0x09, 0x0f, 0xff, 0xff, 0x00]);
        let base = |cluster| {
            let mut packet = padded(&[0x11, 0xff, 0x09, 0x1f, cluster, 2, 100, 50, 25], LONG_LEN);
            packet[16] = 0x01;
            packet
        };
        let painted = writes.iter().position(|packet| packet[3] == 0x1f).unwrap();
        assert_eq!(writes[painted], base(0));
        assert_eq!(writes[painted + 1], base(1));
        let frame = [
            padded(
                &[
                    0x11, 0xff, 0x0a, 0x6f, 100, 50, 25, 0x01, 0x02, 0x03, 0xd2, 0xff,
                ],
                LONG_LEN,
            ),
            padded(
                &[
                    0x11, 0xff, 0x0a, 0x1f, 0xb4, 0, 0, 0, 0xb5, 5, 10, 15, 0xb6, 0, 0, 0, 0xb7, 0,
                    0, 0,
                ],
                LONG_LEN,
            ),
            padded(&[0x11, 0xff, 0x0a, 0x1f, 0xb8, 0, 0, 0, 0xff], LONG_LEN),
            padded(&[0x11, 0xff, 0x0a, 0x7f], LONG_LEN),
        ];
        // A fresh claim paints the first frame twice.
        let rest = &writes[painted + 2..];
        assert_eq!(rest.len(), 8);
        assert_eq!(rest[..4], frame);
        assert_eq!(rest[4..], frame);

        // Nothing lit any more: software control goes back, as it was.
        let before = board.recorder.writes().len();
        deck.set_deck_light(None).unwrap();
        deck.set_face(1, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            board.recorder.writes()[before..],
            [vec![0x10, 0xff, 0x09, 0x5f, 0x01, 0x00, 0x02]]
        );
        assert!(!deck.claimed);
    }

    #[test]
    fn the_heartbeat_asserts_the_g_keys_control_and_frame_again() {
        let board = g915_board();
        let mut deck = open("logitech-g915", &board);
        deck.set_deck_light(Some([1, 2, 3])).unwrap();
        deck.flush().unwrap();
        let before = board.recorder.writes().len();
        deck.heartbeat().unwrap();
        let writes = board.recorder.writes()[before..].to_vec();
        assert_eq!(writes[0], padded(&[0x11, 0xff, 0x05, 0x2f, 0x01], LONG_LEN));
        assert_eq!(writes[1], vec![0x10, 0xff, 0x09, 0x5f, 0x01, 0x01, 0x02]);
        assert_eq!(
            writes.last().unwrap(),
            &padded(&[0x11, 0xff, 0x0a, 0x7f], LONG_LEN)
        );
        assert_eq!(writes.len(), 2 + 4);
    }

    #[test]
    fn closing_hands_the_g_keys_and_the_lights_back() {
        let board = g915_board();
        let mut deck = open("logitech-g915", &board);
        deck.set_deck_light(Some([1, 2, 3])).unwrap();
        deck.flush().unwrap();
        let before = board.recorder.writes().len();
        drop(deck);
        assert_eq!(
            board.recorder.writes()[before..],
            [
                padded(&[0x11, 0xff, 0x05, 0x2f, 0x00], LONG_LEN),
                vec![0x10, 0xff, 0x09, 0x5f, 0x01, 0x00, 0x02],
            ]
        );
    }

    #[test]
    fn a_silent_g915_falls_back_to_the_known_indexes_and_zones() {
        let mut board = g915_board();
        board.silent = true;
        let mut deck = open("logitech-g915", &board);
        deck.poll(0).unwrap();
        assert_eq!(
            (deck.lighting, deck.effects, deck.g_keys),
            (0x0a, Some(0x09), None)
        );
        assert_eq!(deck.backlight_zones, {
            let mut zones = known_zones();
            zones.retain(|zone| !(0xb4..=0xb8).contains(zone));
            zones
        });
        assert_eq!(deck.g_key_zones, vec![0xb4, 0xb5, 0xb6, 0xb7, 0xb8]);
        // No G-key feature, so no diversion; nothing failed outright.
        assert!(board
            .recorder
            .writes()
            .iter()
            .all(|packet| packet[3] != 0x2f));
        deck.set_deck_light(Some([9, 9, 9])).unwrap();
        deck.flush().unwrap();
        deck.heartbeat().unwrap();
    }

    #[test]
    fn a_feature_the_root_says_is_absent_is_not_driven_at_a_known_index() {
        // A G915 whose firmware lists no lighting feature: the known index
        // (0x0a) belongs to something else here.
        let board = Board::new(&[(GAMING_G_KEYS, 0x05), (RGB_EFFECTS, 0x09)]);
        let mut deck = open("logitech-g915", &board);
        deck.poll(0).unwrap();
        assert!(deck.lighting_absent);
        assert_eq!(deck.g_keys, Some(0x05));
        deck.set_deck_light(Some([9, 9, 9])).unwrap();
        deck.flush().unwrap();
        assert!(board
            .recorder
            .writes()
            .iter()
            .all(|packet| packet[2] != 0x0a));
        // And one that lists no G-key feature drops the known G-key index.
        let board = Board::new(&[(PER_KEY_LIGHTING, 0x0f), (COLOR_LED_EFFECTS, 0x10)]);
        let mut deck = open("logitech-g910", &board);
        deck.poll(0).unwrap();
        assert_eq!(deck.g_keys, None);
        assert!(board
            .recorder
            .writes()
            .iter()
            .all(|packet| packet[2] != 0x08));
    }

    #[test]
    fn a_long_only_collection_gets_long_requests() {
        let mut board = g915_board();
        board.long_only = true;
        let mut deck = open("logitech-g815", &board);
        deck.poll(0).unwrap();
        let writes = board.recorder.writes();
        assert_eq!(
            writes[0],
            padded(&[0x11, 0xff, 0x00, 0x0f, 0x80, 0x81], LONG_LEN)
        );
        assert!(writes.iter().all(|packet| packet[0] == LONG_REPORT));
        assert_eq!(deck.short_works, Some(false));
    }

    fn g910_board() -> Board {
        Board::new(&[
            (GAMING_G_KEYS, 0x08),
            (PER_KEY_LIGHTING, 0x0f),
            (COLOR_LED_EFFECTS, 0x10),
        ])
    }

    #[test]
    fn g910_reads_nine_g_keys() {
        let board = g910_board();
        let mut deck = open("logitech-g910", &board);
        deck.poll(0).unwrap();
        assert!(board
            .recorder
            .writes()
            .contains(&padded(&[0x11, 0xff, 0x08, 0x2f, 0x01], LONG_LEN)));
        board
            .recorder
            .queue_read(padded(&[0x11, 0xff, 0x08, 0x00, 0x00, 0x01], LONG_LEN));
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![Input::Key {
                index: 8,
                pressed: true
            }]
        );
    }

    #[test]
    fn g910_sets_a_fixed_effect_then_streams_key_groups_and_commits() {
        let board = g910_board();
        let mut deck = open("logitech-g910", &board);
        deck.set_face(0, Some(&face([255, 0, 0]))).unwrap();
        deck.set_face(8, Some(&face([0, 0, 255]))).unwrap();
        deck.set_deck_light(Some([0, 255, 0])).unwrap();
        deck.flush().unwrap();
        let writes = board.recorder.writes();
        let start = writes.iter().position(|packet| packet[3] == 0x3f).unwrap();
        // The fixed colour (zone 0's effect 1) in RAM only.
        assert_eq!(
            writes[start],
            padded(&[0x11, 0xff, 0x10, 0x3f, 0x00, 0x01, 0, 255, 0], LONG_LEN)
        );
        let frames = &writes[start + 1..];
        // 107 keys: seven frames of 14 and one of 9, then the logo and G-keys.
        assert_eq!(
            frames[0][..12],
            [0x12, 0xff, 0x0f, 0x3f, 0x00, 0x01, 0x00, 0x0e, 0x04, 0, 255, 0]
        );
        assert_eq!(frames[0].len(), VERY_LONG_LEN);
        assert_eq!(
            frames[7][..8],
            [0x12, 0xff, 0x0f, 0x3f, 0x00, 0x01, 0x00, 0x09]
        );
        assert_eq!(frames[7][8..12], [0x87, 0, 255, 0]);
        assert_eq!(frames[7][40..44], [0xe7, 0, 255, 0]);
        assert_eq!(frames[7][44], 0, "zero padded past the last entry");
        assert_eq!(
            frames[8],
            padded(
                &[0x11, 0xff, 0x0f, 0x3f, 0x00, 0x10, 0x00, 0x02, 1, 0, 255, 0, 2, 0, 255, 0],
                LONG_LEN
            )
        );
        assert_eq!(
            frames[9][..8],
            [0x12, 0xff, 0x0f, 0x3f, 0x00, 0x04, 0x00, 0x09]
        );
        assert_eq!(frames[9][8..12], [1, 255, 0, 0]);
        assert_eq!(frames[9][12..16], [2, 0, 0, 0]);
        assert_eq!(frames[9][40..44], [9, 0, 0, 255]);
        assert_eq!(frames[10], vec![0x10, 0xff, 0x00, 0x1f, 0x00, 0x00, 0x5a]);
        assert_eq!(frames[11], padded(&[0x11, 0xff, 0x0f, 0x5f], LONG_LEN));
        assert_eq!(frames.len(), 12);
    }

    #[test]
    fn a_refused_frame_is_not_committed() {
        let mut board = g910_board();
        board.refuse_frame = true;
        let mut deck = open("logitech-g910", &board);
        deck.set_deck_light(Some([1, 1, 1])).unwrap();
        deck.flush().unwrap();
        let writes = board.recorder.writes();
        assert_eq!(
            writes.last().unwrap()[..4],
            [0x10, 0xff, 0x00, 0x1f],
            "ping, no commit"
        );
    }

    #[test]
    fn g513_and_g_pro_are_lights_only_with_their_own_logos() {
        for (model, logo) in [
            ("logitech-g513", None),
            ("logitech-g-pro-keyboard", Some(1u8)),
        ] {
            let board = Board::new(&[(PER_KEY_LIGHTING, 0x0c), (COLOR_LED_EFFECTS, 0x0d)]);
            let mut deck = open(model, &board);
            deck.poll(0).unwrap();
            assert!(
                board
                    .recorder
                    .writes()
                    .iter()
                    .all(|packet| packet[4..6] != [0x80, 0x10]),
                "no G-key feature is asked for"
            );
            // Nothing lit: the board keeps its own effect.
            deck.flush().unwrap();
            let opened = board.recorder.writes().len();
            assert!(board
                .recorder
                .writes()
                .iter()
                .all(|packet| packet[3] != 0x3f));
            deck.set_deck_light(Some([4, 5, 6])).unwrap();
            deck.flush().unwrap();
            let writes = board.recorder.writes()[opened..].to_vec();
            let logos: Vec<&Vec<u8>> = writes
                .iter()
                .filter(|packet| packet.get(5) == Some(&GROUP_LOGO) && packet[3] == 0x3f)
                .collect();
            match logo {
                None => assert!(logos.is_empty(), "{model}"),
                Some(id) => assert_eq!(
                    logos,
                    vec![&padded(
                        &[0x11, 0xff, 0x0c, 0x3f, 0x00, 0x10, 0x00, 0x01, id, 4, 5, 6],
                        LONG_LEN
                    )]
                ),
            }
            assert!(writes
                .iter()
                .all(|packet| packet.get(5) != Some(&GROUP_G_KEYS)));
            assert_eq!(
                writes.last().unwrap(),
                &padded(&[0x11, 0xff, 0x0c, 0x5f], LONG_LEN)
            );
            // Unset again: there is nothing to hand back to, so it goes dark.
            deck.set_deck_light(None).unwrap();
            deck.flush().unwrap();
            let dark = board.recorder.writes();
            let keys = dark
                .iter()
                .rev()
                .find(|packet| packet[0] == VERY_LONG_REPORT)
                .unwrap();
            assert_eq!(keys[8..12], [0x87, 0, 0, 0]);
        }
    }

    #[test]
    fn brightness_scales_and_g_keys_are_named() {
        let board = g915_board();
        let mut deck = open("logitech-g915", &board);
        deck.brightness = 25;
        assert_eq!(deck.scaled(Some([200, 100, 3])), [50, 25, 0]);
        assert_eq!(deck.scaled(None), [0, 0, 0]);
        assert_eq!(
            key_names(by_id("logitech-g915").unwrap()).unwrap(),
            vec!["G1", "G2", "G3", "G4", "G5"]
        );
        assert_eq!(key_names(by_id("logitech-g910").unwrap()).unwrap().len(), 9);
        assert!(key_names(by_id("logitech-g13").unwrap()).is_none());
    }
}
