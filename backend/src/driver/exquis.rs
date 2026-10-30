//! Intuitive Instruments' (Dualo's) Exquis, in its Developer Mode over
//! class-compliant USB MIDI ([`super::midi`]).
//!
//! The port is found by name ([`port_matches`]): `exquis` in any case in the
//! normalised name, on Windows the first (or an unnumbered) interface. No
//! reference states the full name each host gives the port; the exquis-midi
//! library opens the first input and output whose names match `/exquis/i`, so
//! the match follows it. That library's own virtual port, `exquis-midi`
//! (listed under a client prefix on ALSA), is never taken.
//!
//! Every message Ryu sends is SysEx `F0 00 21 7E 7F <cmd> ... F7` (Dualo's
//! manufacturer id, target `7F` = every device), every payload byte 7-bit.
//! On open Ryu sends Setup (`cmd 00`) with the zone mask `2F`: pads, encoders,
//! slider, Up/Down and the other buttons are taken over, Settings and Sound
//! are left to the device so its settings menu stays reachable. LED colours
//! are `cmd 04`: a start LED id, then `r g b fx` for each LED after it, each
//! component 0-127 and `fx 00` (opaque, no effect). Only taken zones accept
//! LED writes. Ryu sends one message per changed contiguous run within the
//! LEDs it owns: pads `0`-`60`, slider portions `80`-`85`, and `102`-`117`
//! (buttons `102`-`109`, encoders `110`-`113`, encoder buttons
//! `114`-`117`). There is no brightness command: Ryu scales the colours
//! itself. Nothing is paced or acknowledged and there is no keep-alive to
//! send (the device sends its own heartbeat, `F0 00 21 7E F7`, ignored).
//!
//! In, all on MIDI channel 16 (anything on channels 1-15, the native MPE
//! notes, is ignored): a pad is Note On `9F <pad> 7F` and Note Off `8F <pad>
//! 00` (a velocity-0 Note On also releases); a button, encoder push or slider
//! portion is `BF <id> 7F/00`; an encoder turn is `BF <110 + n> <v>`, `v - 64`
//! steps (clockwise positive); the slider also reports `BF 90 <portion>`
//! (decimal 90 = `5A`), 0-5 the portion touched, 127 none. The two slider
//! forms are deduplicated per portion. The device sends Refresh (`cmd 03`)
//! when entering its settings menu (page `7F`) and when leaving it: Ryu then
//! repaints every LED it owns. Tempo, root note and scale replies are
//! ignored.
//!
//! Layout (Ryu keys): the 61 hexagonal pads, pad 0 bottom left to pad 60 top
//! right, in 11 rows of alternately 6 and 5 pads; the display shows them row
//! by row from the top (the half-pad offset of the 5-pad rows cannot be shown
//! on a grid). Then Record, Loop, Clips, Play/Stop, Down and Up, then Undo and
//! Redo, then the six slider portions. Their places are a logical arrangement
//! only; no reference gives the physical ones, nor which end of the slider is
//! portion 0. The four encoders are dials, each with a light (its encoder and
//! encoder-button LEDs take the dial's colour).
//!
//! When Ryu lets go it turns every LED it owns off and leaves Developer Mode
//! (Setup with mask `00`).
//!
//! Facts from Intuitive Instruments' Developer Mode MIDI specification
//! (2025-03-10; vendor documentation, facts only) and the exquis-midi 0.1.0
//! library (MIT); no code was copied. Untested on hardware.

use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const EXQUIS_ID: &str = "exquis";

/// Six columns: 11 pad rows, then a row of six buttons, a row of two and the
/// slider's six portions.
pub const COLS: u8 = 6;
pub const ROWS: u8 = 14;
const PAD_ROWS: u8 = 11;
pub const PADS: u8 = 61;
/// Record, Loop, Clips, Play/Stop, Down, Up, Undo, Redo.
const BUTTONS: u8 = 8;
const SLIDER_PORTIONS: u8 = 6;
const FIRST_BUTTON_KEY: u8 = PADS;
const FIRST_SLIDER_KEY: u8 = PADS + BUTTONS;
pub const KEYS: u8 = PADS + BUTTONS + SLIDER_PORTIONS;
pub const EXTRA_KEYS: u8 = BUTTONS + SLIDER_PORTIONS;
pub const DIALS: u8 = 4;

/// The first pad of each hex row, from the bottom.
const ROW_STARTS: [u8; PAD_ROWS as usize] = [0, 6, 11, 17, 22, 28, 33, 39, 44, 50, 55];

const SYSEX_START: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
/// Manufacturer id, then the target (every device).
const HEADER: [u8; 5] = [SYSEX_START, 0x00, 0x21, 0x7e, 0x7f];
const CMD_SETUP: u8 = 0x00;
const CMD_REFRESH: u8 = 0x03;
const CMD_SET_LED: u8 = 0x04;
/// Pads, encoders, slider, Up/Down and the other buttons; not Settings/Sound.
const SETUP_MASK: u8 = 0x2f;
/// The page byte of a Refresh sent on entering the settings menu.
const ENTERING_MENU: u8 = 0x7f;
const NO_FX: u8 = 0x00;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
/// MIDI channel 16.
const DEV_CHANNEL: u8 = 0x0f;

/// LED and control ids.
const FIRST_SLIDER_ID: u8 = 80;
const SLIDER_ID: u8 = 90;
const SLIDER_UNTOUCHED: u8 = 127;
const FIRST_BUTTON_ID: u8 = 102;
const FIRST_ENCODER_ID: u8 = 110;
const FIRST_ENCODER_BUTTON_ID: u8 = 114;
/// Encoder turns are centred on this value.
const ENCODER_CENTRE: i16 = 64;

/// The LED runs Ryu owns: start id and length.
const RUNS: [(u8, u8); 3] = [
    (0, PADS),
    (FIRST_SLIDER_ID, SLIDER_PORTIONS),
    (FIRST_BUTTON_ID, BUTTONS + 2 * DIALS),
];
const LED_IDS: usize = 128;

const BLACK: [u8; 3] = [0, 0, 0];
const READ_LEN: usize = 256;
const MAX_MESSAGES_PER_POLL: usize = 64;

/// Whether a MIDI port, by its normalised name, is the Exquis's: `exquis` in
/// any case, never the exquis-midi library's virtual `exquis-midi` port, on
/// Windows the first (or an unnumbered) interface.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    if model.protocol != Protocol::ExquisMidi || name.interface.unwrap_or(1) != 1 {
        return false;
    }
    let base = name.base.to_ascii_lowercase();
    base.contains("exquis") && !base.contains("exquis-midi") && !base.contains("ryu")
}

const KEY_NAMES: [&str; KEYS as usize] = [
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
    "Pad 17",
    "Pad 18",
    "Pad 19",
    "Pad 20",
    "Pad 21",
    "Pad 22",
    "Pad 23",
    "Pad 24",
    "Pad 25",
    "Pad 26",
    "Pad 27",
    "Pad 28",
    "Pad 29",
    "Pad 30",
    "Pad 31",
    "Pad 32",
    "Pad 33",
    "Pad 34",
    "Pad 35",
    "Pad 36",
    "Pad 37",
    "Pad 38",
    "Pad 39",
    "Pad 40",
    "Pad 41",
    "Pad 42",
    "Pad 43",
    "Pad 44",
    "Pad 45",
    "Pad 46",
    "Pad 47",
    "Pad 48",
    "Pad 49",
    "Pad 50",
    "Pad 51",
    "Pad 52",
    "Pad 53",
    "Pad 54",
    "Pad 55",
    "Pad 56",
    "Pad 57",
    "Pad 58",
    "Pad 59",
    "Pad 60",
    "Pad 61",
    "Record",
    "Loop",
    "Clips",
    "Play/Stop",
    "Down",
    "Up",
    "Undo",
    "Redo",
    "Slider 1",
    "Slider 2",
    "Slider 3",
    "Slider 4",
    "Slider 5",
    "Slider 6",
];

/// The name Ryu gives each key: the pads from the bottom left, the eight
/// buttons and the slider portions.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    (model.protocol == Protocol::ExquisMidi).then(|| KEY_NAMES.to_vec())
}

/// The hex row (from the bottom) and column of a pad.
fn pad_place(pad: u8) -> Option<(u8, u8)> {
    if pad >= PADS {
        return None;
    }
    let row = ROW_STARTS.iter().rposition(|start| *start <= pad)?;
    let start = ROW_STARTS.get(row)?;
    Some((u8::try_from(row).ok()?, pad - start))
}

/// The pads with the top hex row first, then the six buttons Record to Up,
/// Undo and Redo, and the slider portions, each on a row of its own.
#[must_use]
pub fn key_cells(model: &DeviceModel) -> Option<Vec<u16>> {
    (model.protocol == Protocol::ExquisMidi).then(|| {
        let cols = u16::from(COLS);
        let pads = (0..PADS).filter_map(|pad| {
            let (row, col) = pad_place(pad)?;
            Some(u16::from(PAD_ROWS - 1 - row) * cols + u16::from(col))
        });
        let row = |n: u8| u16::from(PAD_ROWS + n) * cols;
        let buttons = (0..6).map(move |col| row(0) + col);
        let undo_redo = (0..2).map(move |col| row(1) + col);
        let slider = (0..u16::from(SLIDER_PORTIONS)).map(move |col| row(2) + col);
        pads.chain(buttons).chain(undo_redo).chain(slider).collect()
    })
}

/// The LED id of a key.
#[cfg(test)]
const fn key_led(key: u8) -> Option<u8> {
    if key < PADS {
        Some(key)
    } else if key < FIRST_SLIDER_KEY {
        Some(FIRST_BUTTON_ID + key - FIRST_BUTTON_KEY)
    } else if key < KEYS {
        Some(FIRST_SLIDER_ID + key - FIRST_SLIDER_KEY)
    } else {
        None
    }
}

/// One 8-bit component at a brightness, as the device's 7-bit value.
fn component(value: u8, percent: u8) -> u8 {
    let scaled = u16::from(value) * u16::from(percent.min(100)) / 100;
    u8::try_from(scaled >> 1).unwrap_or(0x7f)
}

/// A Set LED colours message from `start`.
fn led_message(start: u8, colors: &[[u8; 3]]) -> Vec<u8> {
    let mut message = Vec::with_capacity(HEADER.len() + 3 + colors.len() * 4);
    message.extend_from_slice(&HEADER);
    message.extend_from_slice(&[CMD_SET_LED, start]);
    for [red, green, blue] in colors {
        message.extend_from_slice(&[*red, *green, *blue, NO_FX]);
    }
    message.push(SYSEX_END);
    message
}

fn setup_message(mask: u8) -> Vec<u8> {
    let mut message = HEADER.to_vec();
    message.extend_from_slice(&[CMD_SETUP, mask, SYSEX_END]);
    message
}

pub struct Exquis {
    transport: Box<dyn Transport>,
    /// Developer Mode set up and every LED sent once.
    started: bool,
    colors: [[u8; 3]; KEYS as usize],
    dial_colors: [Option<[u8; 3]>; DIALS as usize],
    brightness: u8,
    /// What each LED id was last sent (7-bit); `None` until it is.
    sent: [Option<[u8; 3]>; LED_IDS],
    held: [bool; KEYS as usize],
    pushed: [bool; DIALS as usize],
    parser: Parser,
}

impl Exquis {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            colors: [BLACK; KEYS as usize],
            dial_colors: [None; DIALS as usize],
            brightness: 100,
            sent: [None; LED_IDS],
            held: [false; KEYS as usize],
            pushed: [false; DIALS as usize],
            parser: Parser::default(),
        }
    }

    /// The 8-bit colour an LED id should show.
    fn wanted(&self, led: u8) -> [u8; 3] {
        let key = |key: u8| self.colors.get(usize::from(key)).copied();
        let dial = |dial: u8| self.dial_colors.get(usize::from(dial)).copied().flatten();
        let color = match led {
            0..=60 => key(led),
            80..=85 => key(FIRST_SLIDER_KEY + led - FIRST_SLIDER_ID),
            102..=109 => key(FIRST_BUTTON_KEY + led - FIRST_BUTTON_ID),
            110..=113 => dial(led - FIRST_ENCODER_ID),
            114..=117 => dial(led - FIRST_ENCODER_BUTTON_ID),
            _ => None,
        };
        color.unwrap_or(BLACK)
    }

    /// The 7-bit colour an LED id is sent, at the current brightness.
    fn target(&self, led: u8) -> [u8; 3] {
        self.wanted(led)
            .map(|value| component(value, self.brightness))
    }

    fn press(&mut self, key: u8, pressed: bool, events: &mut Vec<Input>) {
        if let Some(held) = self.held.get_mut(usize::from(key)) {
            if *held != pressed {
                *held = pressed;
                events.push(Input::Key {
                    index: key,
                    pressed,
                });
            }
        }
    }

    fn push(&mut self, dial: u8, pressed: bool, events: &mut Vec<Input>) {
        if let Some(held) = self.pushed.get_mut(usize::from(dial)) {
            if *held != pressed {
                *held = pressed;
                events.push(Input::DialPress {
                    index: dial,
                    pressed,
                });
            }
        }
    }

    /// `BF 90 <portion>`: the portion touched (0-5) or none (127).
    fn slider(&mut self, portion: u8, events: &mut Vec<Input>) {
        if portion >= SLIDER_PORTIONS && portion != SLIDER_UNTOUCHED {
            return;
        }
        for other in 0..SLIDER_PORTIONS {
            if other != portion {
                self.press(FIRST_SLIDER_KEY + other, false, events);
            }
        }
        if portion < SLIDER_PORTIONS {
            self.press(FIRST_SLIDER_KEY + portion, true, events);
        }
    }

    fn control(&mut self, id: u8, value: u8, events: &mut Vec<Input>) {
        let on = value != 0;
        match id {
            80..=85 => self.press(FIRST_SLIDER_KEY + id - FIRST_SLIDER_ID, on, events),
            SLIDER_ID => self.slider(value, events),
            102..=109 => self.press(FIRST_BUTTON_KEY + id - FIRST_BUTTON_ID, on, events),
            110..=113 => {
                let steps = (i16::from(value) - ENCODER_CENTRE).clamp(-127, 127);
                if steps != 0 {
                    events.push(Input::DialTurn {
                        index: id - FIRST_ENCODER_ID,
                        delta: i8::try_from(steps).unwrap_or(0),
                    });
                }
            }
            114..=117 => self.push(id - FIRST_ENCODER_BUTTON_ID, on, events),
            _ => {}
        }
    }

    /// Handles one message; returns `Some(leaving)` for a Refresh.
    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) -> Option<bool> {
        match message {
            Message::Channel([status, number, value]) => {
                if status & 0x0f != DEV_CHANNEL {
                    return None;
                }
                match status & 0xf0 {
                    NOTE_ON if *number < PADS => self.press(*number, *value > 0, events),
                    NOTE_OFF if *number < PADS => self.press(*number, false, events),
                    CONTROL_CHANGE => self.control(*number, *value, events),
                    _ => {}
                }
                None
            }
            Message::SysEx(bytes) => {
                if bytes.get(..HEADER.len()) != Some(&HEADER[..])
                    || bytes.get(HEADER.len()) != Some(&CMD_REFRESH)
                {
                    return None;
                }
                let page = bytes
                    .get(HEADER.len() + 1)
                    .copied()
                    .filter(|byte| *byte != SYSEX_END);
                Some(page != Some(ENTERING_MENU))
            }
        }
    }

    /// Sends every changed LED, one message per contiguous changed run.
    fn send_changes(&mut self) -> DriverResult<()> {
        for (start, len) in RUNS {
            let mut run: Option<(u8, Vec<[u8; 3]>)> = None;
            for led in start..start + len {
                let target = self.target(led);
                let slot = usize::from(led);
                if self.sent.get(slot).copied().flatten() == Some(target) {
                    if let Some((first, colors)) = run.take() {
                        self.transport.write(&led_message(first, &colors))?;
                    }
                    continue;
                }
                if let Some(sent) = self.sent.get_mut(slot) {
                    *sent = Some(target);
                }
                run.get_or_insert_with(|| (led, Vec::new())).1.push(target);
            }
            if let Some((first, colors)) = run {
                self.transport.write(&led_message(first, &colors))?;
            }
        }
        Ok(())
    }
}

impl Driver for Exquis {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
            self.flush()?;
        }
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
        let mut events = Vec::new();
        let mut repaint = false;
        for message in &messages {
            if let Some(leaving) = self.receive(message, &mut events) {
                // The menu drew over the LEDs: forget what they show.
                self.sent = [None; LED_IDS];
                repaint = leaving;
            }
        }
        if repaint {
            self.send_changes()?;
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some(color) = self.colors.get_mut(usize::from(key)) {
            *color = face.map_or(BLACK, |face| face.color);
        }
        Ok(())
    }

    /// Sets up Developer Mode the first time, then sends the changed LEDs
    /// (every one the first time).
    fn flush(&mut self) -> DriverResult<()> {
        if !self.started {
            self.transport.write(&setup_message(SETUP_MASK))?;
            self.started = true;
        }
        self.send_changes()
    }

    /// The device has no brightness: the colours are scaled, and the LEDs
    /// whose scaled colour changed are resent at once (the manager does not
    /// flush after this).
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }

    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if let Some(slot) = self.dial_colors.get_mut(usize::from(dial)) {
            *slot = color;
        }
        Ok(())
    }
}

impl Drop for Exquis {
    /// Turns every owned LED off and leaves Developer Mode.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        for (start, len) in RUNS {
            let black = vec![BLACK; usize::from(len)];
            let _ = self.transport.write(&led_message(start, &black));
        }
        let _ = self.transport.write(&setup_message(0));
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi, DialLights, Family, ModelView};

    fn fresh() -> (Exquis, Recorder) {
        let recorder = Recorder::default();
        let deck = Exquis::new(by_id(EXQUIS_ID).unwrap(), Box::new(recorder.clone()));
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

    fn set_leds(start: u8, colors: &[[u8; 3]]) -> Vec<u8> {
        let mut bytes = vec![0xf0, 0x00, 0x21, 0x7e, 0x7f, 0x04, start];
        for [r, g, b] in colors {
            bytes.extend_from_slice(&[*r, *g, *b, 0x00]);
        }
        bytes.push(0xf7);
        bytes
    }

    #[test]
    fn opening_sets_up_developer_mode_then_sends_every_owned_led() {
        let (mut deck, recorder) = fresh();
        deck.set_face(0, Some(&lit([255, 128, 0]))).unwrap();
        deck.set_dial_light(1, Some([0, 0, 255])).unwrap();
        deck.poll(0).unwrap();
        let writes = recorder.writes();
        assert_eq!(writes.len(), 4);
        assert_eq!(
            writes[0],
            vec![0xf0, 0x00, 0x21, 0x7e, 0x7f, 0x00, 0x2f, 0xf7]
        );
        let mut pads = vec![[0, 0, 0]; 61];
        pads[0] = [127, 64, 0];
        assert_eq!(writes[1], set_leds(0, &pads));
        assert_eq!(writes[1].len(), 252);
        assert_eq!(writes[2], set_leds(80, &[[0, 0, 0]; 6]));
        assert_eq!(writes[2].len(), 32);
        let mut upper = vec![[0, 0, 0]; 16];
        upper[9] = [0, 0, 127];
        upper[13] = [0, 0, 127];
        assert_eq!(writes[3], set_leds(102, &upper));
        assert_eq!(writes[3].len(), 72);
        assert!(writes
            .iter()
            .all(|write| write[1..write.len() - 1].iter().all(|byte| *byte < 0x80)));
        // Nothing changed: nothing sent.
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 4);
    }

    #[test]
    fn later_flushes_send_only_the_changed_runs() {
        let (mut deck, recorder) = fresh();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_face(5, Some(&lit([255, 255, 255]))).unwrap();
        deck.set_face(6, Some(&lit([2, 4, 6]))).unwrap();
        deck.set_face(9, Some(&lit([0, 200, 0]))).unwrap();
        // Record, Redo and the last slider portion.
        deck.set_face(61, Some(&lit([200, 0, 0]))).unwrap();
        deck.set_face(68, Some(&lit([0, 0, 200]))).unwrap();
        deck.set_face(74, Some(&lit([10, 10, 10]))).unwrap();
        deck.set_dial_light(3, Some([100, 100, 100])).unwrap();
        deck.set_face(200, Some(&lit([255, 0, 0]))).unwrap();
        deck.set_dial_light(9, Some([255, 0, 0])).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [
                set_leds(5, &[[127, 127, 127], [1, 2, 3]]),
                set_leds(9, &[[0, 100, 0]]),
                set_leds(85, &[[5, 5, 5]]),
                set_leds(102, &[[100, 0, 0]]),
                set_leds(109, &[[0, 0, 100]]),
                set_leds(113, &[[50, 50, 50]]),
                set_leds(117, &[[50, 50, 50]]),
            ]
        );
        let before = recorder.writes().len();
        deck.set_face(5, None).unwrap();
        deck.set_dial_light(3, None).unwrap();
        deck.flush().unwrap();
        assert_eq!(
            recorder.writes()[before..],
            [
                set_leds(5, &[[0, 0, 0]]),
                set_leds(113, &[[0, 0, 0]]),
                set_leds(117, &[[0, 0, 0]]),
            ]
        );
    }

    #[test]
    fn brightness_scales_the_colours() {
        let (mut deck, recorder) = fresh();
        deck.set_face(0, Some(&lit([255, 100, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        deck.set_brightness(50).unwrap();
        assert_eq!(recorder.writes()[before..], [set_leds(0, &[[63, 25, 0]])]);
        assert_eq!(component(255, 200), 127);
        assert_eq!(component(255, 0), 0);
    }

    #[test]
    fn pads_buttons_encoders_and_pushes_map_to_keys_and_dials() {
        let (mut deck, recorder) = fresh();
        for message in [
            // A pad, its release, a velocity-0 release, the top-right pad.
            vec![0x9f, 0x00, 0x7f],
            vec![0x8f, 0x00, 0x00],
            vec![0x9f, 0x1e, 0x7f, 0x1e, 0x00],
            vec![0x9f, 0x3c, 0x7f],
            // Record and Redo.
            vec![0xbf, 102, 0x7f],
            vec![0xbf, 102, 0x00],
            vec![0xbf, 109, 0x7f],
            // Encoder 1 two steps clockwise, encoder 4 one anticlockwise.
            vec![0xbf, 110, 66],
            vec![0xbf, 113, 63],
            vec![0xbf, 111, 64],
            // Encoder pushes 1 and 4.
            vec![0xbf, 114, 0x7f],
            vec![0xbf, 114, 0x7f],
            vec![0xbf, 114, 0x00],
            vec![0xbf, 117, 0x7f],
            // Ignored: native MPE on channels 1-15, unknown ids (Settings,
            // Sound, 118), a pad past 60, the heartbeat, tempo, a bend.
            vec![0x90, 0x00, 0x7f],
            vec![0xb0, 102, 0x7f],
            vec![0x9e, 0x00, 0x7f],
            vec![0xbf, 100, 0x7f],
            vec![0xbf, 101, 0x7f],
            vec![0xbf, 118, 0x7f],
            vec![0x9f, 61, 0x7f],
            vec![0xf0, 0x00, 0x21, 0x7e, 0xf7],
            vec![0xf0, 0x00, 0x21, 0x7e, 0x7f, 0x05, 0x00, 0x78, 0xf7],
            vec![0xef, 0x00, 0x40],
            vec![0xbf],
        ] {
            recorder.queue_read(message);
        }
        let events = deck.poll(0).unwrap();
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(0, false),
                key(30, true),
                key(30, false),
                key(60, true),
                key(61, true),
                key(61, false),
                key(68, true),
                Input::DialTurn { index: 0, delta: 2 },
                Input::DialTurn {
                    index: 3,
                    delta: -1
                },
                Input::DialPress {
                    index: 0,
                    pressed: true
                },
                Input::DialPress {
                    index: 0,
                    pressed: false
                },
                Input::DialPress {
                    index: 3,
                    pressed: true
                },
            ]
        );
        // Only the opening writes: input never makes Ryu write.
        assert_eq!(recorder.writes().len(), 4);
    }

    #[test]
    fn the_two_slider_forms_never_press_twice_or_release_unpressed() {
        let (mut deck, recorder) = fresh();
        deck.flush().unwrap();
        for message in [
            // Portion 0 by both forms, then slide to portion 1.
            vec![0xbf, 90, 0],
            vec![0xbf, 80, 0x7f],
            vec![0xbf, 90, 1],
            vec![0xbf, 81, 0x7f],
            vec![0xbf, 80, 0x00],
            // Untouched, then the button form's late release.
            vec![0xbf, 90, 127],
            vec![0xbf, 81, 0x00],
            // Button form only, then an out-of-range portion.
            vec![0xbf, 85, 0x7f],
            vec![0xbf, 90, 9],
            vec![0xbf, 85, 0x00],
        ] {
            recorder.queue_read(message);
        }
        assert_eq!(
            deck.poll(0).unwrap(),
            vec![
                key(69, true),
                key(69, false),
                key(70, true),
                key(70, false),
                key(74, true),
                key(74, false),
            ]
        );
    }

    #[test]
    fn leaving_the_settings_menu_repaints_every_led() {
        let (mut deck, recorder) = fresh();
        deck.set_face(1, Some(&lit([0, 255, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        // Entering the menu: nothing sent.
        recorder.queue_read(vec![0xf0, 0x00, 0x21, 0x7e, 0x7f, 0x03, 0x7f, 0xf7]);
        assert!(deck.poll(0).unwrap().is_empty());
        assert_eq!(recorder.writes().len(), before);
        // Leaving it: the whole frame again.
        recorder.queue_read(vec![0xf0, 0x00, 0x21, 0x7e, 0x7f, 0x03, 0x02, 0xf7]);
        assert!(deck.poll(0).unwrap().is_empty());
        let writes = recorder.writes();
        assert_eq!(writes.len() - before, 3);
        let mut pads = vec![[0, 0, 0]; 61];
        pads[1] = [0, 127, 0];
        assert_eq!(writes[before], set_leds(0, &pads));
        assert_eq!(writes[before + 1], set_leds(80, &[[0, 0, 0]; 6]));
        assert_eq!(writes[before + 2], set_leds(102, &[[0, 0, 0]; 16]));
        // A Refresh with no page repaints too; other SysEx does not.
        recorder.queue_read(vec![0xf0, 0x00, 0x21, 0x7e, 0x7f, 0x03, 0xf7]);
        recorder.queue_read(vec![0xf0, 0x00, 0x21, 0x7f, 0x7f, 0x03, 0x02, 0xf7]);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), writes.len() + 3);
    }

    #[test]
    fn letting_go_turns_the_leds_off_and_leaves_developer_mode() {
        let (mut deck, recorder) = fresh();
        deck.set_face(0, Some(&lit([255, 0, 0]))).unwrap();
        deck.flush().unwrap();
        let before = recorder.writes().len();
        drop(deck);
        assert_eq!(
            recorder.writes()[before..],
            [
                set_leds(0, &[[0, 0, 0]; 61]),
                set_leds(80, &[[0, 0, 0]; 6]),
                set_leds(102, &[[0, 0, 0]; 16]),
                vec![0xf0, 0x00, 0x21, 0x7e, 0x7f, 0x00, 0x00, 0xf7],
            ]
        );
        let (deck, recorder) = fresh();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn the_port_is_found_by_exquis_but_never_the_librarys_virtual_port() {
        let model = by_id(EXQUIS_ID).unwrap();
        for raw in [
            "Exquis",
            "Exquis MIDI 1",
            "Exquis:Exquis MIDI 1 24:0",
            "EXQUIS",
            "MIDIIN1 (Exquis)",
        ] {
            assert!(port_matches(model, &normalise(raw)), "{raw}");
            assert_eq!(
                identify_midi(&normalise(raw), raw.starts_with("MIDI")).map(|model| model.id),
                Some(EXQUIS_ID),
                "{raw}"
            );
        }
        for raw in [
            "exquis-midi",
            "RtMidi Output Client:exquis-midi 130:0",
            "MIDIIN2 (Exquis)",
            "Ryu Decks exquis",
            "Exq",
        ] {
            assert!(!port_matches(model, &normalise(raw)), "{raw}");
            assert!(identify_midi(&normalise(raw), false).is_none(), "{raw}");
        }
        let k2 = by_id(super::super::xone_k2::XONE_K2_ID).unwrap();
        assert!(!port_matches(k2, &normalise("Exquis")));
    }

    #[test]
    fn the_pads_sit_top_row_first_then_buttons_and_slider() {
        let model = by_id(EXQUIS_ID).unwrap();
        assert_eq!(model.family, Family::IntuitiveInstruments);
        assert!(model.protocol.is_midi() && model.protocol.has_brightness());
        assert_eq!(model.dial_lights, DialLights::PerDial);
        assert_eq!(model.dial_light_colors(), 4);
        assert!(model.product_ids.is_empty() && model.more_usb_ids.is_empty());
        assert_eq!(
            (
                model.rows,
                model.cols,
                model.keys,
                model.extra_keys,
                model.dials
            ),
            (14, 6, 75, 14, 4)
        );
        let view = ModelView::from(model);
        assert_eq!(view.vendor_id, "0000");
        assert!(view.key_lights && view.brightness);
        assert_eq!(view.key_light_colors, "rgb");
        let names = view.key_names.unwrap();
        assert_eq!(names.len(), usize::from(model.keys));
        assert_eq!(
            (names[0], names[60], names[61], names[68], names[69], names[74]),
            ("Pad 1", "Pad 61", "Record", "Redo", "Slider 1", "Slider 6")
        );
        let cells = view.key_cells.unwrap();
        assert_eq!(cells.len(), usize::from(model.keys));
        // Pad 0 bottom left, 5 the end of the bottom row, 6 the first of the
        // five-pad row above it, 55-60 the top row.
        assert_eq!((cells[0], cells[5], cells[6], cells[10]), (60, 65, 54, 58));
        assert_eq!((cells[55], cells[60]), (0, 5));
        assert_eq!(
            (cells[61], cells[66], cells[67], cells[68]),
            (66, 71, 72, 73)
        );
        assert_eq!((cells[69], cells[74]), (78, 83));
        let mut sorted = cells.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), cells.len());
        assert!(cells
            .iter()
            .all(|cell| *cell < u16::from(model.rows) * u16::from(model.cols)));
        for key in 0..KEYS {
            assert!(key_led(key).is_some());
        }
        assert_eq!(
            (
                key_led(0),
                key_led(60),
                key_led(61),
                key_led(69),
                key_led(75)
            ),
            (Some(0), Some(60), Some(102), Some(80), None)
        );
    }
}
