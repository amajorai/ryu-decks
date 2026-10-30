//! iCON Pro Audio's P1-Nano, over class-compliant USB MIDI
//! ([`super::midi`]), in its Mackie Control (MCU) mode.
//!
//! Mixxx's P1-Nano mapping names the port `Icon P1-Nano MIDI 1` and the USB
//! device 1d03:0806 (shown only: a MIDI deck is found by its port name).
//! [`port_matches`] keys on `P1-NANO` in any case, on Windows the first
//! interface and under ALSA the device's first port, which is taken to be
//! its Mackie Control port (an assumption). The P1-M and the P1-X extender
//! are not driven: no reference gives their port names.
//!
//! In, on MIDI channel 1 only: every button and fader touch is a Note On,
//! pressed at a velocity over 64 and released at 64 or under (a Note Off
//! releases too). A key's index is its Mackie note, `0x00`-`0x70`, eight to
//! a row: Rec, Solo, Mute, Select and V-Pot push by strip, the global
//! buttons in Mackie note order (`0x28` Track to `0x67` User B), the fader
//! touches and the master fader touch. Which of them the P1-Nano has is not
//! in any reference read; one it lacks never fires. Notes on channels 2-4
//! (the D-5 touchscreen's) are ignored, so they never alias onto a key. The
//! V-Pots are `CC 0x10+s` (dials 1-8) and the jog wheel `CC 0x3C` (dial 9),
//! sign-magnitude relative: bit 6 set turns anticlockwise, bits 0-5 count
//! the steps (0 counts as 1). The one motor fader is dial 10: it reports as
//! pitch bend on the selected strip's channel (1-8) or on 9 as master, so
//! any of those turns it. Its first value only seeds it, and so does the
//! first after it moves to another channel; then every 256 of its 14 bits
//! (64 steps end to end) turn the dial a step. A Mackie SysEx from the deck
//! (`F0 00 00 66 <id> ...`) tells Ryu the id to put in its LCD headers
//! (`14` until then).
//!
//! Out, on channel 1: a button's LED is `90 <note> 7F` on and `90 <note>
//! 00` off (Ryu never blinks one). The 2 x 56 LCD (`F0 00 00 66 14 12
//! <00|38> <55 chars> F7`) shows each strip's Select label over its V-Pot
//! push label, six characters each. Each strip's backlight is the dial
//! light of the V-Pot under it, all eight in one iCON SysEx (`F0 00 02 4E
//! 16 14` then R G B per strip, 0-127, `F7`), sent only when it changes:
//! each channel halved, then under 20 off and else lifted by 20 (as Ardour
//! does, so dim colours still show); a dark colour is black and no colour
//! white. Opening sends the Mackie wake-up (`F0 00 00 66 14 00 F7`),
//! blanks the LCD and the timecode digits (`B0 40-49 20`), turns the V-Pot
//! rings (`B0 30-37 00`; Mixxx writes the P1-Nano's rings on `10-17`, so
//! this is unverified), the strip meters (`D0`) and master meters (`D1`;
//! Mixxx sends the P1-Nano's meters inside an iCON SysEx, so this too is
//! unverified) and every LED off. Every write waits in a queue drained by
//! poll and flush, at most 32 at a time, a millisecond apart and five after
//! a SysEx (a guess: the deck's buffer is not known; no wait in tests), and
//! Ryu never sleeps in the poll loop. Letting go blanks the displays, turns
//! the LEDs and rings off and the backlights white at once. Ryu never sends
//! pitch bend (the fader has a motor), the two-character display CCs, the
//! QCon's second-LCD SysEx or ring positions. There is no handshake,
//! keep-alive or brightness command.
//!
//! Facts from Ardour's Mackie Control surface and P1-Nano device file (C++,
//! GPL-2.0-or-later), DrivenByMoss's Mackie Control support (Java,
//! LGPL-3.0) and Mixxx's Icon P1-Nano mapping (GPL-2.0-or-later); no code
//! was copied. Untested on hardware.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::icon_qcon::{lcd_char, relative, Fader, NAMES};
use super::launch_control::{Message, Parser};
use super::midi::PortName;
use super::{Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, Protocol};

pub const NANO_ID: &str = "icon-p1-nano";
/// The P1-Nano's USB product id under iCON's vendor id (Mixxx), shown only.
pub const NANO_PRODUCT_ID: u16 = 0x0806;
pub const ROWS: u8 = 15;
pub const COLS: u8 = 8;
/// Notes `0x00` to `0x70`, each its own key.
pub const KEYS: u8 = 113;
/// Eight V-Pots, the jog wheel and the fader.
pub const DIALS: u8 = 10;
/// The scribble strips over the V-Pots (dials 1-8) are the only dial
/// lights; the jog wheel and the fader have none.
pub const STRIP_LIGHTS: u8 = 8;

const STRIPS: u8 = 8;
const JOG_DIAL: u8 = 8;
const FADER_DIAL: u8 = 9;
/// Pitch bend channels 1-8 are the strips, 9 the master.
const MASTER_CHANNEL: u8 = 8;
const LAST_NOTE: u8 = KEYS - 1;
/// The strip Select notes head the LCD's first line, the V-Pot pushes its
/// second.
const SELECT: u8 = 0x18;
const VPOT_PUSH: u8 = 0x20;
const LAST_STRIP_LIGHT: u8 = 0x1f;
/// The global buttons with a light, Track to Scrub.
const GLOBAL_LIGHTS: std::ops::RangeInclusive<u8> = 0x28..=0x65;

const NOTE_OFF: u8 = 0x80;
const NOTE_ON: u8 = 0x90;
const CONTROL_CHANGE: u8 = 0xb0;
const CHANNEL_PRESSURE: u8 = 0xd0;
const PITCH_BEND: u8 = 0xe0;
const STRIP_METER: u8 = 0xd0;
const MASTER_METER: u8 = 0xd1;
const VPOT_CC: u8 = 0x10;
const JOG_CC: u8 = 0x3c;
/// The Mackie expression pedal input, absolute: not read.
const EXTERNAL_CC: u8 = 0x2e;
const RING_CC: u8 = 0x30;
const TIMECODE_CC: u8 = 0x40;
const TIMECODE_DIGITS: u8 = 10;
const BLANK: u8 = 0x20;
/// Velocities over this press.
const PRESS_OVER: u8 = 0x40;

const LED_OFF: u8 = 0;
const LED_ON: u8 = 0x7f;

const SYSEX_START: u8 = 0xf0;
const SYSEX_END: u8 = 0xf7;
/// Mackie's manufacturer bytes, for the LCD text.
const MACKIE: [u8; 3] = [0x00, 0x00, 0x66];
const LCD_COMMAND: u8 = 0x12;
const HELLO_COMMAND: u8 = 0x00;
/// The Mackie main unit's SysEx id, until the deck sends its own.
const DEVICE: u8 = 0x14;
/// iCON's manufacturer bytes and the scribble-strip colour command.
const ICON_COLOURS: [u8; 5] = [0x00, 0x02, 0x4e, 0x16, 0x14];
const WHITE: u8 = 0x7f;
/// A halved channel under this is off; any other is lifted by [`LIFT`].
const LIFT_FLOOR: u8 = 20;
const LIFT: u8 = 20;

/// Characters on each LCD line as staged, of which [`SENT_LINE`] are sent
/// (as Ardour does), and the offset of the second line.
const LINE: usize = 56;
const SENT_LINE: usize = 55;
const LINE_TWO: u8 = 0x38;
/// Characters per strip cell, then a space.
const CELL: usize = 7;
const CELL_TEXT: usize = 6;

/// A colour is dark (its light off) while every channel is under this.
const DARK: u8 = 32;
/// Fader units (of 16384) per dial step: 64 steps from end to end.
const FADER_STEP: i32 = 256;

/// The gap after a short message and after a SysEx (none in tests). The
/// deck's buffer is not documented, so these are a guess.
const SHORT_GAP: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(1)
};
const SYSEX_GAP: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(5)
};
/// How far behind the clock may fall, so a burst after a quiet spell is
/// still paced.
const MAX_CREDIT: Duration = Duration::from_millis(10);
/// The most writes one poll or flush sends.
const MAX_WRITES: usize = 32;
const READ_LEN: usize = 256;
const MAX_MESSAGES_PER_POLL: usize = 64;

fn is_model(model: &DeviceModel) -> bool {
    model.protocol == Protocol::IconP1Midi && model.id == NANO_ID
}

/// Whether a normalised port name is the P1-Nano's: `P1-NANO` in the
/// uppercased name, on Windows the first interface, under ALSA port 0.
#[must_use]
pub fn port_matches(model: &DeviceModel, name: &PortName) -> bool {
    is_model(model)
        && name.interface.unwrap_or(1) == 1
        && name.alsa_port.unwrap_or(0) == 0
        && name.base.to_ascii_uppercase().contains("P1-NANO")
}

/// The Mackie function of each key.
#[must_use]
pub fn key_names(model: &DeviceModel) -> Option<Vec<&'static str>> {
    is_model(model).then(|| NAMES.to_vec())
}

/// The keys with a light: Rec, Solo, Mute, Select and the global buttons
/// from Track to Scrub.
#[must_use]
pub fn lit_keys(model: &DeviceModel) -> Option<Vec<u8>> {
    is_model(model).then(|| (0..KEYS).filter(|key| has_light(*key)).collect())
}

fn has_light(key: u8) -> bool {
    key <= LAST_STRIP_LIGHT || GLOBAL_LIGHTS.contains(&key)
}

/// On for any colour that is not dark.
fn is_lit(color: Option<[u8; 3]>) -> bool {
    color.is_some_and(|color| color.iter().any(|channel| *channel >= DARK))
}

/// A strip's backlight as the iCON SysEx takes it: white with no colour
/// set, black for a dark one, else each channel halved, then off under
/// [`LIFT_FLOOR`] and lifted by [`LIFT`] otherwise.
fn strip_rgb(color: Option<[u8; 3]>) -> [u8; 3] {
    match color {
        None => [WHITE; 3],
        Some(_) if !is_lit(color) => [0; 3],
        Some(color) => color.map(|channel| {
            let half = channel >> 1;
            if half < LIFT_FLOOR {
                0
            } else {
                half.saturating_add(LIFT).min(WHITE)
            }
        }),
    }
}

/// The one SysEx that sets all eight backlights.
fn colour_message(strips: &[[u8; 3]; STRIPS as usize]) -> Vec<u8> {
    let mut message = Vec::with_capacity(31);
    message.push(SYSEX_START);
    message.extend_from_slice(&ICON_COLOURS);
    for rgb in strips {
        message.extend(rgb.iter().map(|channel| channel & 0x7f));
    }
    message.push(SYSEX_END);
    message
}

fn lcd_message(device: u8, line: u8, text: &[u8; LINE]) -> Vec<u8> {
    let mut message = Vec::with_capacity(SENT_LINE + 8);
    message.push(SYSEX_START);
    message.extend_from_slice(&MACKIE);
    message.extend_from_slice(&[device, LCD_COMMAND, line * LINE_TWO]);
    message.extend_from_slice(text.get(..SENT_LINE).unwrap_or(text));
    message.push(SYSEX_END);
    message
}

/// Writes a label into its strip's cell, padded with spaces.
fn put_cell(line: &mut [u8; LINE], strip: usize, label: &str) {
    let Some(cell) = line.get_mut(strip * CELL..strip * CELL + CELL_TEXT) else {
        return;
    };
    cell.fill(BLANK);
    for (slot, ch) in cell.iter_mut().zip(label.chars()) {
        *slot = lcd_char(ch);
    }
}

/// What a queued write sets, so a newer write for the same thing replaces
/// one still waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Led(u8),
    Line(u8),
    Colours,
    Other,
}

#[derive(Debug)]
struct Out {
    slot: Slot,
    bytes: Vec<u8>,
}

pub struct IconP1 {
    transport: Box<dyn Transport>,
    /// The reset has been queued.
    started: bool,
    /// Writes waiting their turn, drained by poll and flush; dropped, not
    /// sent, when Ryu lets go.
    outbox: VecDeque<Out>,
    /// When the next write may go out.
    ready_at: Instant,
    short_gap: Duration,
    sysex_gap: Duration,
    /// Each key's LED as staged, then as last queued.
    leds: [bool; KEYS as usize],
    queued: [bool; KEYS as usize],
    held: [bool; KEYS as usize],
    /// The LCD's two lines as staged, then as last queued.
    lines: [[u8; LINE]; 2],
    queued_lines: [Option<[u8; LINE]>; 2],
    /// The id in the LCD's SysEx header.
    device: u8,
    /// Each strip's backlight colour as staged, then the bytes last queued.
    strips: [Option<[u8; 3]>; STRIPS as usize],
    queued_strips: Option<[[u8; 3]; STRIPS as usize]>,
    /// The fader, and the pitch bend channel it last reported on.
    fader: Fader,
    fader_channel: Option<u8>,
    /// The unexpected-input hint has been logged.
    warned: bool,
    parser: Parser,
}

impl IconP1 {
    #[must_use]
    pub fn new(_model: &DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            started: false,
            outbox: VecDeque::new(),
            ready_at: Instant::now(),
            short_gap: SHORT_GAP,
            sysex_gap: SYSEX_GAP,
            leds: [false; KEYS as usize],
            queued: [false; KEYS as usize],
            held: [false; KEYS as usize],
            lines: [[BLANK; LINE]; 2],
            queued_lines: [None; 2],
            device: DEVICE,
            strips: [None; STRIPS as usize],
            queued_strips: None,
            fader: Fader::default(),
            fader_channel: None,
            warned: false,
            parser: Parser::default(),
        }
    }

    fn push(&mut self, slot: Slot, bytes: Vec<u8>) {
        if slot != Slot::Other {
            self.outbox.retain(|out| out.slot != slot);
        }
        self.outbox.push_back(Out { slot, bytes });
    }

    /// The messages that blank the LCD and timecode digits and turn the
    /// rings off.
    fn reset_messages(&self) -> Vec<(Slot, Vec<u8>)> {
        let mut messages = Vec::new();
        for line in 0..2 {
            messages.push((
                Slot::Line(line),
                lcd_message(self.device, line, &[BLANK; LINE]),
            ));
        }
        for digit in 0..TIMECODE_DIGITS {
            messages.push((
                Slot::Other,
                vec![CONTROL_CHANGE, TIMECODE_CC + digit, BLANK],
            ));
        }
        for strip in 0..STRIPS {
            messages.push((Slot::Other, vec![CONTROL_CHANGE, RING_CC + strip, 0]));
        }
        messages
    }

    /// Wake-up, blank displays, rings, meters and every LED off, then
    /// whatever is already staged.
    fn start(&mut self) {
        self.started = true;
        self.outbox.clear();
        let hello = vec![
            SYSEX_START,
            MACKIE[0],
            MACKIE[1],
            MACKIE[2],
            self.device,
            HELLO_COMMAND,
            SYSEX_END,
        ];
        self.push(Slot::Other, hello);
        for (slot, bytes) in self.reset_messages() {
            self.push(slot, bytes);
        }
        for strip in 0..STRIPS {
            self.push(Slot::Other, vec![STRIP_METER, strip << 4]);
        }
        self.push(Slot::Other, vec![MASTER_METER, 0x00]);
        self.push(Slot::Other, vec![MASTER_METER, 0x10]);
        for key in (0..KEYS).filter(|key| has_light(*key)) {
            self.push(Slot::Led(key), vec![NOTE_ON, key, LED_OFF]);
        }
        self.queued = [false; KEYS as usize];
        self.queued_lines = [Some([BLANK; LINE]); 2];
        self.queued_strips = None;
        self.queue_changes();
    }

    /// Queues each LED, LCD line and the backlights where they differ from
    /// what was queued.
    fn queue_changes(&mut self) {
        for key in (0..KEYS).filter(|key| has_light(*key)) {
            let index = usize::from(key);
            let on = self.leds[index];
            if self.queued[index] == on {
                continue;
            }
            self.queued[index] = on;
            let value = if on { LED_ON } else { LED_OFF };
            self.push(Slot::Led(key), vec![NOTE_ON, key, value]);
        }
        for line in 0..2u8 {
            let text = self.lines[usize::from(line)];
            if self.queued_lines[usize::from(line)] == Some(text) {
                continue;
            }
            self.queued_lines[usize::from(line)] = Some(text);
            self.push(Slot::Line(line), lcd_message(self.device, line, &text));
        }
        let strips = self.strips.map(strip_rgb);
        if self.queued_strips != Some(strips) {
            self.queued_strips = Some(strips);
            self.push(Slot::Colours, colour_message(&strips));
        }
    }

    /// Writes the queued messages whose gap has passed, at most
    /// [`MAX_WRITES`], never sleeping.
    fn pump(&mut self) -> DriverResult<()> {
        let now = Instant::now();
        if let Some(floor) = now.checked_sub(MAX_CREDIT) {
            self.ready_at = self.ready_at.max(floor);
        }
        for _ in 0..MAX_WRITES {
            let Some(out) = self.outbox.front() else {
                break;
            };
            if self.ready_at > now {
                break;
            }
            self.transport.write(&out.bytes)?;
            let gap = if out.bytes.first() == Some(&SYSEX_START) {
                self.sysex_gap
            } else {
                self.short_gap
            };
            self.outbox.pop_front();
            self.ready_at += gap;
        }
        Ok(())
    }

    fn hint(&mut self) {
        if !self.warned {
            self.warned = true;
            tracing::warn!(
                "the iCON P1-Nano sent a message Ryu does not expect; set it to its Mackie \
                 Control (MCU) mode for Ryu to read it"
            );
        }
    }

    fn note(&mut self, note: u8, pressed: bool, events: &mut Vec<Input>) {
        if note > LAST_NOTE {
            self.hint();
            return;
        }
        let Some(held) = self.held.get_mut(usize::from(note)) else {
            return;
        };
        if *held != pressed {
            *held = pressed;
            events.push(Input::Key {
                index: note,
                pressed,
            });
        }
    }

    fn control(&mut self, number: u8, value: u8, events: &mut Vec<Input>) {
        let index = match number {
            JOG_CC => JOG_DIAL,
            _ if (VPOT_CC..VPOT_CC + STRIPS).contains(&number) => number - VPOT_CC,
            EXTERNAL_CC => return,
            _ => {
                self.hint();
                return;
            }
        };
        events.push(Input::DialTurn {
            index,
            delta: relative(value),
        });
    }

    /// The one fader, on whichever channel it reports: a new channel only
    /// seeds it, so switching strips never jumps the dial.
    fn fader(&mut self, channel: u8, lsb: u8, msb: u8, events: &mut Vec<Input>) {
        if channel > MASTER_CHANNEL {
            return;
        }
        if self.fader_channel != Some(channel) {
            self.fader_channel = Some(channel);
            self.fader = Fader::default();
        }
        let value = u16::from(lsb & 0x7f) | u16::from(msb & 0x7f) << 7;
        let delta = self.fader.moved(value, FADER_STEP);
        if delta != 0 {
            events.push(Input::DialTurn {
                index: FADER_DIAL,
                delta,
            });
        }
    }

    /// A Mackie SysEx from the deck carries the id its LCD answers to.
    fn sysex(&mut self, bytes: &[u8]) {
        if bytes.get(1..4) != Some(&MACKIE[..]) {
            return;
        }
        let Some(&device) = bytes.get(4) else {
            return;
        };
        if device >= 0x80 || device == self.device {
            return;
        }
        self.device = device;
        if self.started {
            self.queued_lines = [None; 2];
            self.queue_changes();
        }
    }

    fn receive(&mut self, message: &Message, events: &mut Vec<Input>) {
        let [status, data1, data2] = match message {
            Message::SysEx(bytes) => return self.sysex(bytes),
            Message::Channel(bytes) => *bytes,
        };
        match status {
            NOTE_ON => self.note(data1, data2 > PRESS_OVER, events),
            NOTE_OFF => self.note(data1, false, events),
            CONTROL_CHANGE => self.control(data1, data2, events),
            _ if status & 0xf0 == PITCH_BEND => self.fader(status & 0x0f, data1, data2, events),
            // Notes and controls on other channels are the D-5
            // touchscreen's; pressure is meter traffic.
            _ if matches!(
                status & 0xf0,
                NOTE_ON | NOTE_OFF | CONTROL_CHANGE | CHANNEL_PRESSURE
            ) => {}
            _ => self.hint(),
        }
    }
}

impl Driver for IconP1 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        if !self.started {
            self.start();
        }
        self.pump()?;
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
            self.receive(message, &mut events);
        }
        Ok(events)
    }

    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if key >= KEYS {
            return Ok(());
        }
        if has_light(key) {
            if let Some(led) = self.leds.get_mut(usize::from(key)) {
                *led = is_lit(face.map(|face| face.color));
            }
        }
        let label = face.map_or("", |face| face.label.as_str());
        if (SELECT..SELECT + STRIPS).contains(&key) {
            put_cell(&mut self.lines[0], usize::from(key - SELECT), label);
        } else if (VPOT_PUSH..VPOT_PUSH + STRIPS).contains(&key) {
            put_cell(&mut self.lines[1], usize::from(key - VPOT_PUSH), label);
        }
        Ok(())
    }

    /// The reset the first time, then whatever changed; what the pace
    /// allows goes out now, the rest from later polls.
    fn flush(&mut self) -> DriverResult<()> {
        if self.started {
            self.queue_changes();
        } else {
            self.start();
        }
        self.pump()
    }

    /// A V-Pot's light is the backlight of the scribble strip above it; the
    /// jog wheel and the fader have none.
    fn set_dial_light(&mut self, dial: u8, color: Option<[u8; 3]>) -> DriverResult<()> {
        if let Some(strip) = self.strips.get_mut(usize::from(dial)) {
            *strip = color;
        }
        Ok(())
    }

    /// The P1-Nano has no brightness control.
    fn set_brightness(&mut self, _percent: u8) -> DriverResult<()> {
        Ok(())
    }
}

impl Drop for IconP1 {
    /// Turns every LED off, blanks the displays and turns the backlights
    /// white at once, without pacing; anything still queued is dropped.
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        self.outbox.clear();
        for key in (0..KEYS).filter(|key| has_light(*key)) {
            let _ = self.transport.write(&[NOTE_ON, key, LED_OFF]);
        }
        for (_, bytes) in self.reset_messages() {
            let _ = self.transport.write(&bytes);
        }
        let _ = self
            .transport
            .write(&colour_message(&[[WHITE; 3]; STRIPS as usize]));
    }
}

#[cfg(test)]
mod tests {
    use super::super::midi::normalise;
    use super::super::testing::Recorder;
    use super::*;
    use crate::catalog::{by_id, identify_midi};

    fn open() -> (IconP1, Recorder) {
        let recorder = Recorder::default();
        let deck = IconP1::new(by_id(NANO_ID).unwrap(), Box::new(recorder.clone()));
        (deck, recorder)
    }

    /// Opens the deck and sends everything the reset queued.
    fn opened() -> (IconP1, Recorder) {
        let (mut deck, recorder) = open();
        deck.flush().unwrap();
        while !deck.outbox.is_empty() {
            deck.poll(0).unwrap();
        }
        (deck, recorder)
    }

    fn face(label: &str, color: [u8; 3]) -> Face {
        Face {
            label: label.to_owned(),
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

    fn feed(deck: &mut IconP1, recorder: &Recorder, messages: &[&[u8]]) -> Vec<Input> {
        for message in messages {
            recorder.queue_read(message.to_vec());
        }
        deck.poll(0).unwrap()
    }

    fn lcd(device: u8, offset: u8, text: &str) -> Vec<u8> {
        let mut message = vec![0xf0, 0x00, 0x00, 0x66, device, 0x12, offset];
        let mut chars = text.as_bytes().to_vec();
        chars.resize(55, b' ');
        message.extend(chars);
        message.push(0xf7);
        message
    }

    fn colours(strips: [[u8; 3]; 8]) -> Vec<u8> {
        let mut message = vec![0xf0, 0x00, 0x02, 0x4e, 0x16, 0x14];
        message.extend(strips.iter().flatten());
        message.push(0xf7);
        message
    }

    fn drain(deck: &mut IconP1) {
        deck.flush().unwrap();
        while !deck.outbox.is_empty() {
            deck.poll(0).unwrap();
        }
    }

    #[test]
    fn opening_wakes_blanks_and_darkens_everything_in_bounded_batches() {
        assert!(SHORT_GAP.is_zero() && SYSEX_GAP.is_zero());
        let (mut deck, recorder) = open();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 32);
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 64);
        while !deck.outbox.is_empty() {
            deck.poll(0).unwrap();
        }
        let writes = recorder.writes();
        assert_eq!(writes.len(), 1 + 2 + 10 + 8 + 8 + 2 + 94 + 1);
        assert_eq!(writes[0], vec![0xf0, 0x00, 0x00, 0x66, 0x14, 0x00, 0xf7]);
        assert_eq!(writes[1], lcd(0x14, 0x00, ""));
        assert_eq!(writes[1].len(), 63);
        assert_eq!(writes[2], lcd(0x14, 0x38, ""));
        for digit in 0..10u8 {
            assert_eq!(
                writes[3 + usize::from(digit)],
                vec![0xb0, 0x40 + digit, 0x20]
            );
        }
        for strip in 0..8u8 {
            assert_eq!(
                writes[13 + usize::from(strip)],
                vec![0xb0, 0x30 + strip, 0x00]
            );
            assert_eq!(writes[21 + usize::from(strip)], vec![0xd0, strip << 4]);
        }
        assert_eq!(writes[29..31], [vec![0xd1, 0x00], vec![0xd1, 0x10]]);
        let leds: Vec<u8> = writes[31..125].iter().map(|write| write[1]).collect();
        let expected: Vec<u8> = (0x00..=0x1f).chain(0x28..=0x65).collect();
        assert_eq!(leds, expected);
        assert!(writes[31..125]
            .iter()
            .all(|write| write[0] == 0x90 && write[2] == 0));
        // No colour set: every backlight white, in one 31-byte SysEx.
        assert_eq!(writes[125], colours([[0x7f; 3]; 8]));
        assert_eq!(writes[125].len(), 31);
        // Never pitch bend, the two-character display or the second LCD.
        assert!(writes.iter().all(|write| {
            write[0] & 0xf0 != 0xe0
                && !(write[0] == 0xb0 && matches!(write[1], 0x4a | 0x4b))
                && write.get(3) != Some(&0x67)
        }));
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert_eq!(recorder.writes().len(), 126);
    }

    #[test]
    fn writes_are_paced_from_the_poll_loop_without_sleeping() {
        let (mut deck, recorder) = open();
        deck.short_gap = Duration::from_secs(60);
        deck.sysex_gap = Duration::from_secs(60);
        deck.ready_at = Instant::now() + Duration::from_secs(60);
        let begun = Instant::now();
        deck.flush().unwrap();
        deck.poll(0).unwrap();
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert!(recorder.writes().is_empty());
        assert_eq!(deck.outbox.len(), 126);
        // Letting go is never paced, and drops whatever is queued.
        drop(deck);
        assert!(begun.elapsed() < Duration::from_secs(1));
        assert_eq!(recorder.writes().len(), 94 + 2 + 10 + 8 + 1);
    }

    #[test]
    fn keys_are_channel_one_notes_and_other_channels_are_ignored() {
        let (mut deck, recorder) = opened();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0x90, 0x00, 0x7f],
                &[0x90, 0x00, 0x7f],
                &[0x90, 0x5e, 0x7f],
                &[0x80, 0x5e, 0x00],
                &[0x90, 0x00, 0x00],
                &[0x90, 0x70, 0x7f],
                // A touch at 64 is not pressed.
                &[0x90, 0x68, 0x40],
                &[0x90, 0x68, 0x41],
                &[0x90, 0x68, 0x40],
                // The D-5 touchscreen's notes, and notes past the master touch.
                &[0x91, 0x00, 0x7f],
                &[0x92, 0x05, 0x7f],
                &[0x93, 0x0f, 0x7f],
                &[0x81, 0x00, 0x00],
                &[0x90, 0x71, 0x7f],
                &[0x90, 0x7f, 0x7f],
            ],
        );
        assert_eq!(
            events,
            [
                key(0x00, true),
                key(0x5e, true),
                key(0x5e, false),
                key(0x00, false),
                key(0x70, true),
                key(0x68, true),
                key(0x68, false),
            ]
        );
    }

    #[test]
    fn v_pots_and_jog_are_sign_magnitude() {
        let (mut deck, recorder) = opened();
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xb0, 0x10, 0x01],
                &[0xb0, 0x10, 0x41],
                &[0xb0, 0x17, 0x00],
                &[0xb0, 0x17, 0x40],
                &[0xb0, 0x13, 0x05],
                &[0xb0, 0x13, 0x45],
                &[0xb0, 0x3c, 0x01],
                &[0xb0, 0x3c, 0x42],
                // Another channel, the pedal and an unknown control.
                &[0xb1, 0x10, 0x01],
                &[0xb0, 0x2e, 0x40],
                &[0xb0, 0x50, 0x01],
            ],
        );
        assert_eq!(
            events,
            [
                turn(0, 1),
                turn(0, -1),
                turn(7, 1),
                turn(7, -1),
                turn(3, 5),
                turn(3, -5),
                turn(8, 1),
                turn(8, -2),
            ]
        );
    }

    #[test]
    fn the_fader_is_read_on_any_strip_channel_and_reseeded_on_a_switch() {
        let (mut deck, recorder) = opened();
        let before = recorder.writes().len();
        // 0x2000 seeds on channel 9 (master); up 512 is two steps.
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xe8, 0x00, 0x40],
                &[0xe8, 0x00, 0x44],
                &[0xe8, 0x7f, 0x44],
            ],
        );
        assert_eq!(events, [turn(9, 2)]);
        // A new channel seeds again, however far its value is.
        let events = feed(
            &mut deck,
            &recorder,
            &[
                &[0xe0, 0x00, 0x00],
                &[0xe0, 0x00, 0x02],
                &[0xe3, 0x00, 0x7f],
                &[0xe3, 0x00, 0x7d],
                // Channels past the master are not the fader.
                &[0xe9, 0x00, 0x00],
                &[0xe3, 0x00, 0x7b],
            ],
        );
        assert_eq!(events, [turn(9, 1), turn(9, -1), turn(9, -1)]);
        // Ryu never sends the fader back.
        assert_eq!(recorder.writes().len(), before);
    }

    #[test]
    fn faces_light_buttons_and_label_the_lcd() {
        let (mut deck, recorder) = opened();
        let before = recorder.writes().len();
        deck.set_face(0x18, Some(&face("Vocals", [255, 0, 0])))
            .unwrap();
        deck.set_face(0x19, Some(&face("Bass guitar", [0, 0, 0])))
            .unwrap();
        deck.set_face(0x20, Some(&face("Pan", [255, 0, 0])))
            .unwrap();
        deck.set_face(0x5e, Some(&face("Play", [0, 200, 0])))
            .unwrap();
        // No light: V-Pot pushes, User A, fader touches; past the end.
        deck.set_face(0x66, Some(&face("", [255, 0, 0]))).unwrap();
        deck.set_face(0x68, Some(&face("", [255, 0, 0]))).unwrap();
        deck.set_face(113, Some(&face("", [255, 0, 0]))).unwrap();
        drain(&mut deck);
        assert_eq!(
            recorder.writes()[before..],
            [
                vec![0x90, 0x18, 0x7f],
                vec![0x90, 0x5e, 0x7f],
                lcd(0x14, 0x00, "Vocals Bass g"),
                lcd(0x14, 0x38, "Pan"),
            ]
        );
        deck.set_face(0x5e, Some(&face("Play", [0, 0, 255])))
            .unwrap();
        deck.set_brightness(10).unwrap();
        drain(&mut deck);
        assert_eq!(recorder.writes().len(), before + 4);
        deck.set_face(0x18, None).unwrap();
        drain(&mut deck);
        assert_eq!(
            recorder.writes()[before + 4..],
            [vec![0x90, 0x18, 0x00], lcd(0x14, 0x00, "       Bass g")]
        );
    }

    #[test]
    fn dial_lights_colour_the_scribble_strips_once_per_change() {
        assert_eq!(strip_rgb(None), [0x7f; 3]);
        assert_eq!(strip_rgb(Some([10, 20, 31])), [0, 0, 0]);
        assert_eq!(strip_rgb(Some([255, 128, 39])), [127, 84, 0]);
        assert_eq!(strip_rgb(Some([40, 80, 216])), [40, 60, 127]);
        let (mut deck, recorder) = opened();
        let before = recorder.writes().len();
        deck.set_dial_light(0, Some([255, 0, 0])).unwrap();
        deck.set_dial_light(7, Some([0, 0, 0])).unwrap();
        // The jog wheel and the fader have no light.
        deck.set_dial_light(8, Some([0, 255, 0])).unwrap();
        deck.set_dial_light(9, Some([0, 255, 0])).unwrap();
        drain(&mut deck);
        let mut strips = [[0x7f; 3]; 8];
        strips[0] = [0x7f, 0, 0];
        strips[7] = [0, 0, 0];
        assert_eq!(recorder.writes()[before..], [colours(strips)]);
        // Unchanged bytes are not sent again.
        deck.set_dial_light(0, Some([254, 1, 1])).unwrap();
        drain(&mut deck);
        assert_eq!(recorder.writes().len(), before + 1);
        deck.set_dial_light(7, None).unwrap();
        drain(&mut deck);
        strips[7] = [0x7f; 3];
        assert_eq!(recorder.writes()[before + 1..], [colours(strips)]);
    }

    #[test]
    fn the_decks_own_sysex_id_heads_its_lcd_lines() {
        let (mut deck, recorder) = opened();
        let before = recorder.writes().len();
        // Its own id changes nothing; another id rewrites both lines.
        feed(
            &mut deck,
            &recorder,
            &[&[0xf0, 0x00, 0x00, 0x66, 0x14, 0x01, 0xf7]],
        );
        assert_eq!(recorder.writes().len(), before);
        feed(
            &mut deck,
            &recorder,
            &[&[0xf0, 0x00, 0x00, 0x66, 0x10, 0x01, 0xf7]],
        );
        drain(&mut deck);
        assert_eq!(
            recorder.writes()[before..],
            [lcd(0x10, 0x00, ""), lcd(0x10, 0x38, "")]
        );
        // Short and foreign SysEx are ignored.
        feed(&mut deck, &recorder, &[&[0xf0, 0x00, 0x00, 0xf7]]);
        feed(
            &mut deck,
            &recorder,
            &[&[0xf0, 0x00, 0x02, 0x4e, 0x15, 0xf7]],
        );
        drain(&mut deck);
        assert_eq!(recorder.writes().len(), before + 2);
    }

    #[test]
    fn letting_go_darkens_blanks_and_whitens() {
        let (mut deck, recorder) = opened();
        deck.set_face(0x5e, Some(&face("Play", [0, 200, 0])))
            .unwrap();
        deck.set_dial_light(2, Some([0, 0, 255])).unwrap();
        drain(&mut deck);
        let before = recorder.writes().len();
        drop(deck);
        let writes = recorder.writes();
        let close = &writes[before..];
        assert_eq!(close.len(), 94 + 2 + 10 + 8 + 1);
        assert!(close[..94]
            .iter()
            .all(|write| write[0] == 0x90 && write[2] == 0));
        assert_eq!(close[94], lcd(0x14, 0x00, ""));
        assert_eq!(close[95], lcd(0x14, 0x38, ""));
        assert_eq!(close[114], colours([[0x7f; 3]; 8]));
        assert!(close.iter().all(|write| write[0] & 0xf0 != 0xe0));
        // A deck never opened sends nothing.
        let (deck, recorder) = open();
        drop(deck);
        assert!(recorder.writes().is_empty());
    }

    #[test]
    fn the_port_matcher_takes_only_the_p1_nanos_first_port() {
        let model = by_id(NANO_ID).unwrap();
        for name in [
            "Icon P1-Nano MIDI 1",
            "Icon P1-Nano MIDI 1 24:0",
            "iCON P1-NANO",
            "MIDIIN (iCON P1-Nano)",
            "MIDIOUT1 (iCON P1-Nano)",
        ] {
            assert!(port_matches(model, &normalise(name)), "{name}");
            assert_eq!(identify_midi(&normalise(name), false).unwrap().id, NANO_ID);
        }
        for name in [
            "Icon P1-Nano MIDI 2 24:1",
            "MIDIIN2 (iCON P1-Nano)",
            "iCON P1-M V1.00",
            "Platform M+ V1.07",
            "iCON QCON Pro X V1.15",
            "X-Touch One",
        ] {
            assert!(!port_matches(model, &normalise(name)), "{name}");
        }
        let qcon = by_id(super::super::icon_qcon::PRO_X_ID).unwrap();
        assert!(!port_matches(qcon, &normalise("Icon P1-Nano MIDI 1")));
        assert!(identify_midi(&normalise("iCON P1-M V1.00"), false).is_none());
    }

    #[test]
    fn keys_carry_mackie_names_and_lights() {
        let model = by_id(NANO_ID).unwrap();
        let names = key_names(model).unwrap();
        assert_eq!(names.len(), usize::from(KEYS));
        assert_eq!(
            (names[0], names[0x5e], names[0x70]),
            ("Rec 1", "Play", "Master Touch")
        );
        let lit = lit_keys(model).unwrap();
        assert_eq!(lit.len(), 94);
        assert!(!lit.contains(&0x20) && !lit.contains(&0x66) && !lit.contains(&0x68));
    }
}
