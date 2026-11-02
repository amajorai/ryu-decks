//! M-Audio's Oxygen Pro 25, 49 and 61, Oxygen Pro Mini and Hammer 88 Pro in
//! their DAW mode, over class-compliant USB MIDI ([`super::midi`]).
//!
//! Each deck lists three MIDI ports; Ryu uses only the third, the DAW
//! (`Mackie/HUI`) port: `MIDIIN3 (Oxygen Pro 61)` / `MIDIOUT3 (Oxygen Pro
//! 61)` on Windows (and, by Bitwig's list, Linux), `Oxygen Pro 61 Mackie/HUI`
//! on macOS (and `Hammer 88 Pro Mackie/HUI` on Linux too). [`port_matches`]
//! takes a name containing the model's token (`oxygen pro 25`, `oxygen pro
//! 49`, `oxygen pro 61`, `oxygen pro mini` or `hammer 88 pro`, any case, not
//! followed by a letter or digit) on interface 3, or, with no Windows
//! interface number, one containing `mackie/hui`. The keybed's port (`Oxygen
//! Pro 61` on interface 1, `Oxygen Pro 61 USB MIDI` on macOS) and the older
//! Oxygen models (no `pro`) never match. No plain ALSA spelling is in a
//! reference, so none is assumed.
//!
//! In (channel 1 unless noted):
//! - Pads: Note On above velocity 0 presses; Note Off, or velocity 0,
//!   releases. The 16 pads (8 on the Mini) are keys in two rows, top first:
//!   notes 40-43, 48-51 then 36-39, 44-47 (Mini: 40-43 then 48-51).
//! - Track buttons: CC 32 + i. They latch on the deck, so each press sends a
//!   single message whose value alternates 127 / 0; Ryu takes every message,
//!   whatever its value, as one tap (press then release).
//! - Other buttons (momentary, above 0 presses, 0 releases): Loop 114,
//!   Rewind 115, Fast Forward 116, Stop 117, Play 118, Record 119,
//!   Metronome 106, Back 104, Bank Left 110, Bank Right 111, Preset 112, DAW
//!   113 and (not on the Mini) Scene 1 107 and Scene 2 108; Shift is CC 105
//!   on channel 13; the mode buttons Off 57, Arm 58, Select 59, Mute 60,
//!   Solo 61, Volume 83, Pan 85, Device 86 and Sends 87 are on channel 16.
//!   Their places on the deck are not given, so their rows are logical.
//! - Dials: knobs CC 22 + i, faders CC 12 + i (Oxygen Pro 49 and 61 and
//!   Hammer 88 Pro only) and the master fader CC 41 (not on the Mini) are
//!   absolute: the first value seeds a dial, later ones turn it a step per
//!   two units moved. The main encoder is CC 103, relative: above 64 is +1
//!   (both references read it as "next"; clockwise is inferred), 1-63 is -1,
//!   0 and 64 nothing. Its push is CC 102, pressing that dial. The Mini's
//!   fader is not a dial (no reference agrees on all its controllers).
//! - Everything else (other channels and controllers, aftertouch, pitch bend,
//!   SysEx including the identity reply) is dropped.
//!
//! Out: on open, the universal identity request, then M-Audio SysEx `F0 00 01
//! 05 7F 00 00 <cmd> 00 01 <arg> F7`: firmware mode DAW (`6D` 2), control
//! mode 2 then 7 (`6E`), LED control on (`6B` 1) and LED mode software
//! (`6C` 3), all at once and without waiting for the identity reply (as
//! Bitwig does). Then a pad light is `90 <note> <index>` with index
//! `R + 4G + 16B` from 2-bit channels (inferred from the 14 named palette entries;
//! 0 is off, 64 and up would blink and is never sent), and a track button's
//! light `B0 <32 + i> 7F` on or `00` off. Only those keys are lit (Bitwig's
//! choice; Ableton also lights the transport, scene and mode buttons by
//! CC 127 / 0, whose LEDs are not confirmed). There is no brightness
//! command: Ryu scales the colours, and 0% turns every light off. The first
//! flush sends every light, later ones only changes; releasing DAW sends
//! them all again in case the deck repainted. On close: every pad and track
//! light off, then LED mode firmware (`6C` 0, inferred from Ableton's
//! default). No pacing and no keep-alive.
//!
//! Facts from Bitwig's Oxygen Pro extension (bitwig/bitwig-extensions, MIT,
//! Copyright 2019 Bitwig GmbH; port names, notes, controllers, init SysEx,
//! lights), cross-checked against Ableton's Oxygen_Pro, Oxygen_Pro_Mini and
//! Hammer_88_Pro Remote Scripts as published in
//! gluon/AbletonLive12_MIDIRemoteScripts (decompiled, no licence; read for
//! facts and USB ids only); no code was copied.

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const OXYGEN_PRO_25_ID: &str = "maudio-oxygen-pro-25";
pub const OXYGEN_PRO_49_ID: &str = "maudio-oxygen-pro-49";
pub const OXYGEN_PRO_61_ID: &str = "maudio-oxygen-pro-61";
pub const OXYGEN_PRO_MINI_ID: &str = "maudio-oxygen-pro-mini";
pub const HAMMER_88_PRO_ID: &str = "maudio-hammer-88-pro";

/// The full-size decks: 16 pads, 8 track buttons and 24 other buttons.
pub const ROWS: u8 = 6;
pub const COLS: u8 = 8;
pub const KEYS: u8 = 48;
/// The Oxygen Pro 25: 8 knobs, the master fader and the encoder.
pub const DIALS_25: u8 = 10;
/// The Oxygen Pro 49 and 61 and Hammer 88 Pro: 8 knobs, 8 faders, the
/// master fader and the encoder.
pub const DIALS_FADERS: u8 = 18;
/// The Mini: 8 pads, 4 track buttons and 22 other buttons.
pub const MINI_ROWS: u8 = 9;
pub const MINI_COLS: u8 = 4;
pub const MINI_KEYS: u8 = 34;
/// The Mini: 4 knobs and the encoder.
pub const MINI_DIALS: u8 = 5;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// Channel 13: Shift.
const CC_CH13: u8 = 0xbc;
/// Channel 16: the knob and button mode buttons.
const CC_CH16: u8 = 0xbf;

const FIRST_TRACK_CC: u8 = 32;
const FIRST_KNOB_CC: u8 = 22;
const FIRST_FADER_CC: u8 = 12;
const MASTER_CC: u8 = 41;
const ENCODER_CC: u8 = 103;
const ENCODER_PUSH_CC: u8 = 102;
const DAW_CC: u8 = 113;

const LED_ON: u8 = 0x7f;
const LED_OFF: u8 = 0;
/// A colour is dark (a track light off) while every channel is under this.
const DARK: u8 = 32;
/// Control units (of 128) per dial step: 64 steps from end to end.
const STEP: i16 = 2;
const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 64;
const MAX_MESSAGES_PER_POLL: usize = 64;

const IDENTITY_REQUEST: [u8; 6] = [0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7];
/// The M-Audio SysEx commands sent on open, each `(cmd, arg)`.
const START: [(u8, u8); 5] = [
    // Firmware mode: DAW.
    (0x6d, 0x02),
    // Control mode 2, then 7.
    (0x6e, 0x02),
    (0x6e, 0x07),
    // LED control on.
    (0x6b, 0x01),
    // LED mode: software.
    (0x6c, 0x03),
];
/// LED mode: firmware, sent when Ryu lets go.
const LED_MODE_FIRMWARE: (u8, u8) = (0x6c, 0x00);

/// One M-Audio command: `F0 00 01 05 7F 00 00 <cmd> 00 01 <arg> F7`.
const fn command(cmd: u8, arg: u8) -> [u8; 12] {
    [
        0xf0, 0x00, 0x01, 0x05, 0x7f, 0x00, 0x00, cmd, 0x00, 0x01, arg, 0xf7,
    ]
}

/// The full-size decks' pad notes, in key order: top row, then bottom row.
const PADS: [u8; 16] = [
    40, 41, 42, 43, 48, 49, 50, 51, 36, 37, 38, 39, 44, 45, 46, 47,
];
/// The Mini's pad notes, in key order: top row, then bottom row.
const MINI_PADS: [u8; 8] = [40, 41, 42, 43, 48, 49, 50, 51];

/// The full-size decks' other buttons, in key order, by status and controller.
const BUTTONS: [(u8, u8); 24] = [
    (CONTROL_CHANGE, 115),
    (CONTROL_CHANGE, 116),
    (CONTROL_CHANGE, 117),
    (CONTROL_CHANGE, 118),
    (CONTROL_CHANGE, 119),
    (CONTROL_CHANGE, 114),
    (CONTROL_CHANGE, 106),
    (CONTROL_CHANGE, 104),
    (CONTROL_CHANGE, 110),
    (CONTROL_CHANGE, 111),
    (CONTROL_CHANGE, 112),
    (CONTROL_CHANGE, DAW_CC),
    (CONTROL_CHANGE, 107),
    (CONTROL_CHANGE, 108),
    (CC_CH13, 105),
    (CC_CH16, 57),
    (CC_CH16, 58),
    (CC_CH16, 59),
    (CC_CH16, 60),
    (CC_CH16, 61),
    (CC_CH16, 83),
    (CC_CH16, 85),
    (CC_CH16, 86),
    (CC_CH16, 87),
];
/// The Mini's other buttons: the same without Scene 1 and 2.
const MINI_BUTTONS: [(u8, u8); 22] = [
    (CONTROL_CHANGE, 115),
    (CONTROL_CHANGE, 116),
    (CONTROL_CHANGE, 117),
    (CONTROL_CHANGE, 118),
    (CONTROL_CHANGE, 119),
    (CONTROL_CHANGE, 114),
    (CONTROL_CHANGE, 106),
    (CONTROL_CHANGE, 104),
    (CONTROL_CHANGE, 110),
    (CONTROL_CHANGE, 111),
    (CONTROL_CHANGE, 112),
    (CONTROL_CHANGE, DAW_CC),
    (CC_CH13, 105),
    (CC_CH16, 57),
    (CC_CH16, 58),
    (CC_CH16, 59),
    (CC_CH16, 60),
    (CC_CH16, 61),
    (CC_CH16, 83),
    (CC_CH16, 85),
    (CC_CH16, 86),
    (CC_CH16, 87),
];

const NAMES: [&str; KEYS as usize] = [
    "Pad 1",
    "Pad 2",
    "Pad 3",
    "Pad 4",
    "Pad 5",
    "Pad 6",
    "Pad 7",
    "Pad 8",
    "Pad 9",
    "Pad 10",
    "Pad 11",
    "Pad 12",
    "Pad 13",
    "Pad 14",
    "Pad 15",
    "Pad 16",
    "Track 1",
    "Track 2",
    "Track 3",
    "Track 4",
    "Track 5",
    "Track 6",
    "Track 7",
    "Track 8",
    "Rewind",
    "Fast Forward",
    "Stop",
    "Play",
    "Record",
    "Loop",
    "Metronome",
    "Back",
    "Bank Left",
    "Bank Right",
    "Preset",
    "DAW",
    "Scene 1",
    "Scene 2",
    "Shift",
    "Off",
    "Arm",
    "Select",
    "Mute",
    "Solo",
    "Volume",
    "Pan",
    "Device",
    "Sends",
];

const MINI_NAMES: [&str; MINI_KEYS as usize] = [
    "Pad 1",
    "Pad 2",
    "Pad 3",
    "Pad 4",
    "Pad 5",
    "Pad 6",
    "Pad 7",
    "Pad 8",
    "Track 1",
    "Track 2",
    "Track 3",
    "Track 4",
    "Rewind",
    "Fast Forward",
    "Stop",
    "Play",
    "Record",
    "Loop",
    "Metronome",
    "Back",
    "Bank Left",
    "Bank Right",
    "Preset",
    "DAW",
    "Shift",
    "Off",
    "Arm",
    "Select",
    "Mute",
    "Solo",
    "Volume",
    "Pan",
    "Device",
    "Sends",
];

/// One model's controls.
struct Layout {
    pads: &'static [u8],
    tracks: u8,
    buttons: &'static [(u8, u8)],
    names: &'static [&'static str],
    knobs: u8,
    faders: u8,
    master: bool,
    /// The model token its port name must contain, lowercase.
    token: &'static str,
}

impl Layout {
    fn of(model: &DeviceModel) -> Option<&'static Self> {
        if model.protocol != Protocol::MAudioOxygenProMidi {
            return None;
        }
        match model.id {
            OXYGEN_PRO_25_ID => Some(&PRO_25),
            OXYGEN_PRO_49_ID => Some(&PRO_49),
            OXYGEN_PRO_61_ID => Some(&PRO_61),
            OXYGEN_PRO_MINI_ID => Some(&MINI),
            HAMMER_88_PRO_ID => Some(&HAMMER),
            _ => None,
        }
    }

    const fn keys(&self) -> usize {
        self.pads.len() + self.tracks as usize + self.buttons.len()
    }

    /// Lit keys: the pads, then the track buttons.
    const fn lit(&self) -> u8 {
        // At most 24: never truncated.
        #[allow(clippy::cast_possible_truncation)]
        let pads = self.pads.len() as u8;
        pads + self.tracks
    }

    const fn master_dial(&self) -> u8 {
        self.knobs + self.faders
    }

    const fn encoder_dial(&self) -> u8 {
        self.knobs + self.faders + if self.master { 1 } else { 0 }
    }

    const fn dials(&self) -> u8 {
        self.encoder_dial() + 1
    }

    fn pad_key(&self, note: u8) -> Option<u8> {
        self.pads
            .iter()
            .position(|pad| *pad == note)
            .and_then(|at| u8::try_from(at).ok())
    }

    fn button_key(&self, status: u8, cc: u8) -> Option<u8> {
        self.buttons
            .iter()
            .position(|button| *button == (status, cc))
            .and_then(|at| u8::try_from(at).ok())
            .map(|at| self.lit() + at)
    }

    /// The dial an absolute controller turns.
    const fn absolute_dial(&self, cc: u8) -> Option<u8> {
        if cc >= FIRST_KNOB_CC && cc < FIRST_KNOB_CC + self.knobs {
            Some(cc - FIRST_KNOB_CC)
        } else if self.faders > 0 && cc >= FIRST_FADER_CC && cc < FIRST_FADER_CC + self.faders {
            Some(self.knobs + cc - FIRST_FADER_CC)
        } else if self.master && cc == MASTER_CC {
            Some(self.master_dial())
        } else {
            None
        }
    }
}

const PRO_25: Layout = Layout {
    pads: &PADS,
    tracks: 8,
    buttons: &BUTTONS,
    names: &NAMES,
    knobs: 8,
    faders: 0,
    master: true,
    token: "oxygen pro 25",
};
const PRO_49: Layout = Layout {
    faders: 8,
    token: "oxygen pro 49",
    ..PRO_25
};
const PRO_61: Layout = Layout {
    faders: 8,
    token: "oxygen pro 61",
    ..PRO_25
};
const HAMMER: Layout = Layout {
    faders: 8,
    token: "hammer 88 pro",
    ..PRO_25
};
const MINI: Layout = Layout {
    pads: &MINI_PADS,
    tracks: 4,
    buttons: &MINI_BUTTONS,
    names: &MINI_NAMES,
    knobs: 4,
    faders: 0,
    master: false,
    token: "oxygen pro mini",
};

/// Whether `token` appears in `base` not followed by a letter or digit, so
/// `oxygen pro 25` never takes a longer model name.
fn has_token(base: &str, token: &str) -> bool {
    base.match_indices(token).any(|(at, found)| {
        base.as_bytes()
            .get(at + found.len())
            .is_none_or(|next| !next.is_ascii_alphanumeric())
    })
}

/// Whether a normalised port name is this model's DAW port: its token, on
/// Windows interface 3, or with no interface number and `Mackie/HUI`.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    let Some(layout) = Layout::of(model) else {
        return false;
    };
    let base = name.base.to_ascii_lowercase();
    if !has_token(&base, layout.token) {
        return false;
    }
    match name.interface {
        Some(interface) => interface == 3,
        None => base.contains("mackie/hui"),
    }
}

/// The legend on each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    Layout::of(model).map(|layout| layout.names.to_vec())
}

/// The pads and track buttons have lights; the other buttons none Ryu sets.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    Layout::of(model).map(|layout| (0..layout.lit()).collect())
}

/// A colour scaled by a brightness percentage.
fn scaled(color: [u8; 3], percent: u8) -> [u8; 3] {
    color.map(|channel| {
        u8::try_from(u16::from(channel) * u16::from(percent.min(100)) / 100).unwrap_or(u8::MAX)
    })
}

/// A pad's palette index: 2 bits each of red, green and blue.
fn palette_index(color: [u8; 3]) -> u8 {
    let [red, green, blue] =
        color.map(|channel| u8::try_from(((u16::from(channel) + 42) / 85).min(3)).unwrap_or(3));
    red + 4 * green + 16 * blue
}

pub struct OxygenPro {
    transport: Box<dyn Transport>,
    layout: &'static Layout,
    /// The start-up commands and every light sent since open.
    started: bool,
    colors: Vec<[u8; 3]>,
    brightness: u8,
    /// The value each lit key was last sent; `None` until it is.
    sent: Vec<Option<u8>>,
    held: Vec<bool>,
    /// Each absolute dial's last value, and the units not yet a step.
    values: Vec<Option<u8>>,
    rest: Vec<i16>,
    encoder_held: bool,
    parser: Parser,
}

impl OxygenPro {
    #[must_use]
    pub fn new(model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        let layout = Layout::of(model).unwrap_or(&PRO_61);
        let lit = usize::from(layout.lit());
        let dials = usize::from(layout.dials());
        Self {
            transport,
            layout,
            started: false,
            colors: vec![BLACK; lit],
            brightness: 100,
            sent: vec![None; lit],
            held: vec![false; layout.keys()],
            values: vec![None; dials],
            rest: vec![0; dials],
            encoder_held: false,
            parser: Parser::default(),
        }
    }

    /// The message that sets a lit key's light to `value`.
    fn light(&self, key: u8, value: u8) -> Option<[u8; 3]> {
        let pads = self.layout.pads;
        match pads.get(usize::from(key)) {
            Some(note) => Some([NOTE_ON, *note, value]),
            None => {
                let track = key.checked_sub(self.layout.lit() - self.layout.tracks)?;
                (track < self.layout.tracks).then_some([
                    CONTROL_CHANGE,
                    FIRST_TRACK_CC + track,
                    value,
                ])
            }
        }
    }

    /// The value a lit key's light should show.
    fn value(&self, key: u8) -> u8 {
        let Some(color) = self.colors.get(usize::from(key)) else {
            return LED_OFF;
        };
        if self.brightness == 0 {
            return LED_OFF;
        }
        if usize::from(key) < self.layout.pads.len() {
            palette_index(scaled(*color, self.brightness))
        } else if color.iter().any(|channel| *channel >= DARK) {
            LED_ON
        } else {
            LED_OFF
        }
    }

    /// Sends one key's light, whether or not it changed.
    fn send(&mut self, key: u8) -> DriverResult<()> {
        let value = self.value(key);
        let Some(message) = self.light(key, value) else {
            return Ok(());
        };
        self.transport.write(&message)?;
        if let Some(sent) = self.sent.get_mut(usize::from(key)) {
            *sent = Some(value);
        }
        Ok(())
    }

    fn send_all(&mut self) -> DriverResult<()> {
        for key in 0..self.layout.lit() {
            self.send(key)?;
        }
        Ok(())
    }

    /// The start-up commands, then every light. Nothing waits: there is no
    /// reply to wait for.
    fn start(&mut self) -> DriverResult<()> {
        self.started = true;
        self.transport.write(&IDENTITY_REQUEST)?;
        for (cmd, arg) in START {
            self.transport.write(&command(cmd, arg))?;
        }
        self.send_all()
    }

    fn press(&mut self, key: u8, pressed: bool, events: &mut Vec<Input>) -> DriverResult<()> {
        let Some(held) = self.held.get_mut(usize::from(key)) else {
            return Ok(());
        };
        if *held == pressed {
            return Ok(());
        }
        *held = pressed;
        events.push(Input::Key {
            index: key,
            pressed,
        });
        // In case the deck repainted its own lights in its DAW menu.
        if !pressed && self.layout.button_key(CONTROL_CHANGE, DAW_CC) == Some(key) {
            self.send_all()?;
        }
        Ok(())
    }

    /// An absolute knob or fader: the first value seeds it, each later one
    /// turns its dial a step per two units moved.
    fn control(&mut self, dial: u8, value: u8, events: &mut Vec<Input>) {
        let at = usize::from(dial);
        let (Some(last), Some(rest)) = (self.values.get_mut(at), self.rest.get_mut(at)) else {
            return;
        };
        let Some(old) = last.replace(value) else {
            return;
        };
        let moved = *rest + i16::from(value) - i16::from(old);
        let steps = moved / STEP;
        *rest = moved - steps * STEP;
        if steps != 0 {
            events.push(Input::DialTurn {
                index: dial,
                delta: i8::try_from(steps.clamp(-127, 127)).unwrap_or(0),
            });
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> DriverResult<()> {
        let Message::Channel([status, number, value]) = *message else {
            return Ok(());
        };
        let layout = self.layout;
        match status {
            NOTE_ON | NOTE_OFF => match layout.pad_key(number) {
                Some(key) => self.press(key, status == NOTE_ON && value > 0, events),
                None => Ok(()),
            },
            CONTROL_CHANGE => {
                if number >= FIRST_TRACK_CC && number < FIRST_TRACK_CC + layout.tracks {
                    // A latching button: every message is one tap.
                    let key = layout.lit() - layout.tracks + number - FIRST_TRACK_CC;
                    events.push(Input::Key {
                        index: key,
                        pressed: true,
                    });
                    events.push(Input::Key {
                        index: key,
                        pressed: false,
                    });
                    return Ok(());
                }
                if let Some(dial) = layout.absolute_dial(number) {
                    self.control(dial, value, events);
                    return Ok(());
                }
                match number {
                    ENCODER_CC => {
                        let delta = match value {
                            65..=127 => 1,
                            1..=63 => -1,
                            _ => return Ok(()),
                        };
                        events.push(Input::DialTurn {
                            index: layout.encoder_dial(),
                            delta,
                        });
                        Ok(())
                    }
                    ENCODER_PUSH_CC => {
                        let pressed = value > 0;
                        if self.encoder_held != pressed {
                            self.encoder_held = pressed;
                            events.push(Input::DialPress {
                                index: layout.encoder_dial(),
                                pressed,
                            });
                        }
                        Ok(())
                    }
                    _ => match layout.button_key(status, number) {
                        Some(key) => self.press(key, value > 0, events),
                        None => Ok(()),
                    },
                }
            }
            CC_CH13 | CC_CH16 => match layout.button_key(status, number) {
                Some(key) => self.press(key, value > 0, events),
                None => Ok(()),
            },
            _ => Ok(()),
        }
    }
}

impl Driver for OxygenPro {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
            self.start()?;
        }
        let mut events = Vec::new();
        let mut messages = Vec::new();
        let mut buf = [0u8; READ_LEN];
        for attempt in 0..MAX_MESSAGES_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.parser
                .feed(buf.get(..read).unwrap_or_default(), &mut messages);
        }
        for message in &messages {
            self.receive(message, &mut events)?;
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// The start-up commands and every light the first time, then only the
    /// lights whose value changed.
    fn flush(&mut self) -> DriverResult<()> {
        if !self.started {
            return self.start();
        }
        for key in 0..self.layout.lit() {
            let value = self.value(key);
            if self.sent.get(usize::from(key)).copied().flatten() != Some(value) {
                self.send(key)?;
            }
        }
        Ok(())
    }

    /// No brightness command: Ryu scales the pad colours, and 0% turns every
    /// light off.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        if self.started {
            self.flush()?;
        }
        Ok(())
    }
}

impl Drop for OxygenPro {
    /// Every light off, then the LEDs back to the firmware. Unpaced.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for key in 0..self.layout.lit() {
            if let Some(message) = self.light(key, LED_OFF) {
                let _ = self.transport.write(&message);
            }
        }
        let (cmd, arg) = LED_MODE_FIRMWARE;
        let _ = self.transport.write(&command(cmd, arg));
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, ModelView};

    fn open(id: &str) -> (OxygenPro, Recorder) {
        let recorder = Recorder::default();
        let deck = OxygenPro::new(by_id(id).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    fn lit(color: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color,
            image: None,
        }
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    fn turn(index: u8, delta: i8) -> Input {
        Input::DialTurn { index, delta }
    }

    fn queue(recorder: &Recorder, messages: &[&[u8]]) {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
    }

    const INIT: [&[u8]; 6] = [
        &[0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7],
        &[
            0xf0, 0x00, 0x01, 0x05, 0x7f, 0x00, 0x00, 0x6d, 0x00, 0x01, 0x02, 0xf7,
        ],
        &[
            0xf0, 0x00, 0x01, 0x05, 0x7f, 0x00, 0x00, 0x6e, 0x00, 0x01, 0x02, 0xf7,
        ],
        &[
            0xf0, 0x00, 0x01, 0x05, 0x7f, 0x00, 0x00, 0x6e, 0x00, 0x01, 0x07, 0xf7,
        ],
        &[
            0xf0, 0x00, 0x01, 0x05, 0x7f, 0x00, 0x00, 0x6b, 0x00, 0x01, 0x01, 0xf7,
        ],
        &[
            0xf0, 0x00, 0x01, 0x05, 0x7f, 0x00, 0x00, 0x6c, 0x00, 0x01, 0x03, 0xf7,
        ],
    ];

    #[test]
    fn the_palette_packs_two_bits_a_channel() {
        assert_eq!(palette_index([255, 0, 0]), 3);
        assert_eq!(palette_index([0, 255, 0]), 12);
        assert_eq!(palette_index([0, 0, 255]), 48);
        assert_eq!(palette_index([255, 255, 0]), 15);
        assert_eq!(palette_index([255, 170, 0]), 11);
        assert_eq!(palette_index([0, 170, 255]), 56);
        assert_eq!(palette_index([255, 255, 255]), 63);
        assert_eq!(palette_index([20, 30, 40]), 0);
        for value in 0..=255u8 {
            assert!(palette_index([value, value, value]) < 64);
        }
    }

    #[test]
    fn opening_sends_the_start_up_commands_then_every_light() {
        let (mut deck, recorder) = open(OXYGEN_PRO_25_ID);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(16, Some(&lit([255, 255, 255]))).unwrap();
        // A key with no light: kept nowhere, sends nothing.
        deck.set_face(30, Some(&lit([255, 255, 255]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 6 + 24);
        for (write, expected) in writes.iter().zip(INIT) {
            assert_eq!(write.as_slice(), expected);
        }
        assert_eq!(writes[6], vec![0x90, 0x28, 0x03]);
        assert_eq!(writes[7], vec![0x90, 0x29, 0x00]);
        assert_eq!(writes[14], vec![0x90, 0x24, 0x00]);
        assert_eq!(writes[22], vec![0xb0, 0x20, 0x7f]);
        assert_eq!(writes[23], vec![0xb0, 0x21, 0x00]);
        assert_eq!(writes[29], vec![0xb0, 0x27, 0x00]);
        // Only changes after.
        deck.flush().unwrap();
        assert_eq!(recorder.writes().len(), 30);
        deck.set_face(16, None).unwrap();
        deck.set_face(15, Some(&lit([0, 0, 255]))).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[30..],
            [vec![0x90, 0x2f, 0x30], vec![0xb0, 0x20, 0x00]]
        );
    }

    #[test]
    fn brightness_scales_the_pads_and_zero_turns_everything_off() {
        let (mut deck, recorder) = open(OXYGEN_PRO_25_ID);
        deck.set_face(0, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(17, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        let start = recorder.writes().len();
        deck.set_brightness(0).unwrap();
        assert_eq!(
            recorder.writes()[start..],
            [vec![0x90, 0x28, 0x00], vec![0xb0, 0x21, 0x00]]
        );
        deck.set_brightness(50).unwrap();
        // 255 at half: 127, the middle-low step on each channel.
        assert_eq!(
            recorder.writes()[start + 2..],
            [vec![0x90, 0x28, 1 + 4 + 16], vec![0xb0, 0x21, 0x7f]]
        );
    }

    #[test]
    fn pads_press_and_release_and_track_buttons_tap() {
        let (mut deck, recorder) = open(OXYGEN_PRO_61_ID);
        queue(
            &recorder,
            &[
                &[0x90, 0x28, 0x64],
                &[0x80, 0x28, 0x40],
                &[0x90, 0x24, 0x10],
                &[0x90, 0x24, 0x00],
                &[0x90, 0x2f, 0x7f],
                // Aftertouch, a pad note on channel 2, a keybed note.
                &[0xa0, 0x2f, 0x20],
                &[0x91, 0x28, 0x7f],
                &[0x90, 0x3c, 0x7f],
                // Track buttons latch: each message is one tap.
                &[0xb0, 0x20, 0x7f],
                &[0xb0, 0x20, 0x00],
                &[0xb0, 0x27, 0x7f],
                // Not a track button on this deck, nor on channel 2.
                &[0xb0, 0x28, 0x7f],
                &[0xb1, 0x21, 0x7f],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(0, true),
                key(0, false),
                key(8, true),
                key(8, false),
                key(15, true),
                key(16, true),
                key(16, false),
                key(16, true),
                key(16, false),
                key(23, true),
                key(23, false),
            ]
        );
    }

    #[test]
    fn buttons_are_matched_by_channel_and_controller() {
        let (mut deck, recorder) = open(OXYGEN_PRO_49_ID);
        queue(
            &recorder,
            &[
                // Rewind, Back, Scene 2.
                &[0xb0, 0x73, 0x7f],
                &[0xb0, 0x73, 0x00],
                &[0xb0, 0x68, 0x7f],
                &[0xb0, 0x6c, 0x7f],
                // Shift only on channel 13.
                &[0xb0, 0x69, 0x7f],
                &[0xbc, 0x69, 0x7f],
                &[0xbc, 0x69, 0x00],
                // Off and Sends on channel 16; Sends' number on channel 1
                // is not a button.
                &[0xbf, 0x39, 0x7f],
                &[0xbf, 0x57, 0x7f],
                &[0xb0, 0x57, 0x7f],
                // Pitch bend, program change, SysEx.
                &[0xe0, 0x00, 0x40],
                &[0xc0, 0x01],
                &[0xf0, 0x7e, 0x7f, 0x06, 0x02, 0x00, 0x01, 0x05, 0xf7],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(24, true),
                key(24, false),
                key(31, true),
                key(37, true),
                key(38, true),
                key(38, false),
                key(39, true),
                key(47, true),
            ]
        );
    }

    #[test]
    fn knobs_and_faders_seed_then_turn_and_the_encoder_steps() {
        let (mut deck, recorder) = open(OXYGEN_PRO_61_ID);
        queue(
            &recorder,
            &[
                // Knob 1: seeds, +4 units, unchanged, -3 units.
                &[0xb0, 0x16, 0x40],
                &[0xb0, 0x16, 0x44],
                &[0xb0, 0x16, 0x44],
                &[0xb0, 0x16, 0x41],
                // Fader 8 and the master fader.
                &[0xb0, 0x13, 0x00],
                &[0xb0, 0x13, 0x0a],
                &[0xb0, 0x29, 0x7f],
                &[0xb0, 0x29, 0x00],
                // The encoder: next, previous, ignored 0 and 64.
                &[0xb0, 0x67, 0x41],
                &[0xb0, 0x67, 0x01],
                &[0xb0, 0x67, 0x00],
                &[0xb0, 0x67, 0x40],
                // Its push.
                &[0xb0, 0x66, 0x7f],
                &[0xb0, 0x66, 0x00],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                turn(0, 2),
                turn(0, -1),
                turn(15, 5),
                turn(16, -63),
                turn(17, 1),
                turn(17, -1),
                Input::DialPress {
                    index: 17,
                    pressed: true
                },
                Input::DialPress {
                    index: 17,
                    pressed: false
                },
            ]
        );
    }

    #[test]
    fn the_pro_25_has_a_master_but_no_channel_faders() {
        let (mut deck, recorder) = open(OXYGEN_PRO_25_ID);
        queue(
            &recorder,
            &[
                &[0xb0, 0x0c, 0x00],
                &[0xb0, 0x0c, 0x7f],
                &[0xb0, 0x29, 0x00],
                &[0xb0, 0x29, 0x04],
                &[0xb0, 0x67, 0x7f],
            ],
        );
        assert_eq!(deck.poll(0).unwrap(), vec![turn(8, 2), turn(9, 1)]);
    }

    #[test]
    fn the_mini_has_eight_pads_four_tracks_and_four_knobs() {
        let (mut deck, recorder) = open(OXYGEN_PRO_MINI_ID);
        deck.set_face(4, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_face(11, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 6 + 12);
        assert_eq!(writes[10], vec![0x90, 0x30, 0x03]);
        assert_eq!(writes[17], vec![0xb0, 0x23, 0x7f]);
        queue(
            &recorder,
            &[
                &[0x90, 0x2b, 0x7f],
                &[0x90, 0x24, 0x7f],
                &[0xb0, 0x23, 0x00],
                &[0xb0, 0x24, 0x7f],
                // No Scene buttons, master or faders.
                &[0xb0, 0x6b, 0x7f],
                &[0xb0, 0x29, 0x00],
                &[0xb0, 0x29, 0x7f],
                &[0xb0, 0x0c, 0x00],
                &[0xb0, 0x0c, 0x7f],
                &[0xb0, 0x19, 0x00],
                &[0xb0, 0x19, 0x02],
                &[0xb0, 0x1a, 0x00],
                &[0xb0, 0x1a, 0x7f],
                &[0xbc, 0x69, 0x7f],
                &[0xbf, 0x57, 0x7f],
                &[0xb0, 0x67, 0x41],
            ],
        );
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(3, true),
                key(11, true),
                key(11, false),
                turn(3, 1),
                key(24, true),
                key(33, true),
                turn(4, 1),
            ]
        );
    }

    #[test]
    fn releasing_daw_sends_every_light_again() {
        let (mut deck, recorder) = open(HAMMER_88_PRO_ID);
        deck.flush().unwrap();
        let start = recorder.writes().len();
        queue(&recorder, &[&[0xb0, 0x71, 0x7f], &[0xb0, 0x71, 0x00]]);
        assert_eq!(deck.poll(0).unwrap(), vec![key(35, true), key(35, false)]);
        assert_eq!(recorder.writes().len(), start + 24);
    }

    #[test]
    fn letting_go_turns_the_lights_off_and_hands_back_the_leds() {
        let (mut deck, recorder) = open(OXYGEN_PRO_25_ID);
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        drop(deck);
        let writes = &recorder.writes()[30..];
        assert_eq!(writes.len(), 25);
        assert!(writes[..16]
            .iter()
            .all(|write| write.len() == 3 && write[0] == 0x90 && write[2] == 0));
        assert!(writes[16..24]
            .iter()
            .all(|write| write.len() == 3 && write[0] == 0xb0 && write[2] == 0));
        assert_eq!(
            writes[24],
            vec![0xf0, 0x00, 0x01, 0x05, 0x7f, 0x00, 0x00, 0x6c, 0x00, 0x01, 0x00, 0xf7]
        );
        // Never opened: nothing sent.
        let (deck, recorder) = open(OXYGEN_PRO_25_ID);
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn only_the_daw_port_matches() {
        let cases = [
            ("MIDIIN3 (Oxygen Pro 61)", true, Some(OXYGEN_PRO_61_ID)),
            ("MIDIOUT3 (Oxygen Pro 61)", true, Some(OXYGEN_PRO_61_ID)),
            ("MIDIIN3 (2- Oxygen Pro 61)", true, Some(OXYGEN_PRO_61_ID)),
            ("Oxygen Pro 25 Mackie/HUI", false, Some(OXYGEN_PRO_25_ID)),
            ("Oxygen Pro 49 Mackie/HUI", false, Some(OXYGEN_PRO_49_ID)),
            ("MIDIIN3 (Oxygen Pro Mini)", true, Some(OXYGEN_PRO_MINI_ID)),
            (
                "Oxygen Pro Mini Mackie/HUI",
                false,
                Some(OXYGEN_PRO_MINI_ID),
            ),
            ("Hammer 88 Pro Mackie/HUI", false, Some(HAMMER_88_PRO_ID)),
            ("MIDIOUT3 (Hammer 88 Pro)", true, Some(HAMMER_88_PRO_ID)),
            // The keybed and second ports, and other Oxygens.
            ("Oxygen Pro 61", true, None),
            ("MIDIIN2 (Oxygen Pro 61)", true, None),
            ("MIDIIN1 (Oxygen Pro 61)", true, None),
            ("Oxygen Pro 61 USB MIDI", false, None),
            ("Hammer 88 Pro USB MIDI", false, None),
            ("Hammer 88 Pro", true, None),
            ("Oxygen 61", false, None),
            ("Oxygen 61 Mackie/HUI", false, None),
            ("MIDIIN3 (Oxygen 49)", true, None),
            ("MIDIIN3 (Oxygen Pro 250)", true, None),
        ];
        for (raw, windows, expected) in cases {
            let name = normalise(raw);
            assert_eq!(
                identify_midi(&name, windows).map(|model| model.id),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn the_model_views_name_every_key() {
        for (id, keys, dials, rows, cols) in [
            (OXYGEN_PRO_25_ID, KEYS, DIALS_25, ROWS, COLS),
            (OXYGEN_PRO_49_ID, KEYS, DIALS_FADERS, ROWS, COLS),
            (OXYGEN_PRO_61_ID, KEYS, DIALS_FADERS, ROWS, COLS),
            (HAMMER_88_PRO_ID, KEYS, DIALS_FADERS, ROWS, COLS),
            (
                OXYGEN_PRO_MINI_ID,
                MINI_KEYS,
                MINI_DIALS,
                MINI_ROWS,
                MINI_COLS,
            ),
        ] {
            let model = by_id(id).unwrap();
            let layout = Layout::of(model).unwrap();
            assert_eq!(layout.keys(), usize::from(keys), "{id}");
            assert_eq!(layout.dials(), dials, "{id}");
            let view = ModelView::from(model);
            assert_eq!(
                (view.rows, view.cols, view.keys, view.dials),
                (rows, cols, keys, dials)
            );
            assert_eq!(view.key_names.unwrap().len(), usize::from(keys));
            assert_eq!(view.lit_keys.unwrap().len(), usize::from(layout.lit()));
            assert_eq!(view.dial_lights, DialLights::None);
            assert!(view.key_lights && view.brightness);
            assert_eq!(model.dial_light_colors(), 0);
        }
        assert_eq!(
            Layout::of(by_id(OXYGEN_PRO_49_ID).unwrap()).unwrap().token,
            "oxygen pro 49"
        );
        assert_eq!(
            Layout::of(by_id(OXYGEN_PRO_61_ID).unwrap()).unwrap().token,
            "oxygen pro 61"
        );
        assert_eq!(
            Layout::of(by_id(HAMMER_88_PRO_ID).unwrap()).unwrap().token,
            "hammer 88 pro"
        );
    }
}
