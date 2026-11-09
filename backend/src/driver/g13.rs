//! Logitech G13 Advanced Gameboard: 22 G-keys, four mode keys with LEDs, the
//! round button and four buttons under a 160 x 43 monochrome LCD, two thumb
//! buttons and a thumbstick, with one RGB backlight that also lights the LCD.
//!
//! Everything goes over HID interface 0, with no handshake or keep-alive:
//!
//! - Input is an 8-byte report holding the whole state. Byte 1 is the stick's
//!   X (low is left), byte 2 its Y (low is up), and bytes 3 to 7 a 40-bit key
//!   bitmap, least significant bit first. Bits that are not keys (two unused
//!   bits, the backlight state, the backlight button and a toggle) are masked.
//!   Ryu raises a press or release only when a key's bit changes, and turns the
//!   stick into four direction keys with hysteresis, since it has no axis input.
//! - The LCD is output report `03`: 31 zero bytes, then 960 bytes of pixels in
//!   six bands of 160 columns, each byte eight pixels down one column (bit 0 at
//!   the top). Only the first 43 of the 48 rows are on the glass.
//! - The backlight is feature report `07 r g b 00`. There is no brightness
//!   command, so Ryu scales the colour.
//! - The M1, M2, M3 and MR LEDs are feature report `05 mask 00 00 00`, one bit
//!   each (0x01 to 0x08), only on or off.

use super::{face, Driver, DriverResult, Face, Input, Transport};
use crate::catalog::{DeviceModel, KeyImage};

/// Bigger than a report, so a report of another length is seen and dropped.
const READ_BUFFER: usize = 64;
const REPORT_LEN: usize = 8;
/// The stick streams reports while it moves, so drain generously.
const MAX_REPORTS_PER_POLL: usize = 32;
const LCD_REPORT: u8 = 0x03;
/// Zero bytes between the LCD report id and the pixels.
const LCD_HEADER: usize = 32;
/// 160 columns times six 8-row bands.
pub const LCD_BYTES: usize = 960;
const LED_REPORT: u8 = 0x05;
const BACKLIGHT_REPORT: u8 = 0x07;
/// A light that is only on or off is on for any colour brighter than this, as
/// on the Blackmagic panels.
const ON_OFF_THRESHOLD: u8 = 16;
/// The stick axis reads as pushed past these, and as back at rest only once it
/// returns inside `RELEASE_LOW..=RELEASE_HIGH`, so a stick held near the edge
/// does not chatter.
const PRESS_LOW: u8 = 0x40;
const PRESS_HIGH: u8 = 0xc0;
const RELEASE_LOW: u8 = 0x50;
const RELEASE_HIGH: u8 = 0xb0;

/// The Ryu keys read from the bitmap, in key order: the bitmap bit of each.
/// G1 to G22, then M1, M2, M3, MR, the round button, L1 to L4, the two thumb
/// buttons and the stick click.
const KEY_BITS: [u8; 34] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, // G1-G22
    29, 30, 31, 32, // M1, M2, M3, MR
    24, // the round button left of L1
    25, 26, 27, 28, // L1-L4
    33, 34, 35, // thumb left, thumb down, stick click
];
/// The first of the four stick-direction keys (up, down, left, right).
const STICK_KEY: u8 = 34;
/// M1, M2, M3 and MR: the keys with an LED.
pub const LIT_KEYS: [u8; 4] = [22, 23, 24, 25];

/// The legend of each key, in key order. The LCD (key 38) is the info screen.
pub const KEY_NAMES: [&str; 38] = [
    "G1",
    "G2",
    "G3",
    "G4",
    "G5",
    "G6",
    "G7",
    "G8",
    "G9",
    "G10",
    "G11",
    "G12",
    "G13",
    "G14",
    "G15",
    "G16",
    "G17",
    "G18",
    "G19",
    "G20",
    "G21",
    "G22",
    "M1",
    "M2",
    "M3",
    "MR",
    "Round button",
    "L1",
    "L2",
    "L3",
    "L4",
    "Thumb left",
    "Thumb down",
    "Stick click",
    "Stick up",
    "Stick down",
    "Stick left",
    "Stick right",
];

/// Each key's `(row, column)` on a 7 x 7 picture of the board: the buttons
/// under the LCD, the mode keys, the G-keys as they are staggered, then the
/// thumb buttons and the stick.
pub const KEY_PLACES: [(u8, u8); 38] = [
    (2, 0),
    (2, 1),
    (2, 2),
    (2, 3),
    (2, 4),
    (2, 5),
    (2, 6),
    (3, 0),
    (3, 1),
    (3, 2),
    (3, 3),
    (3, 4),
    (3, 5),
    (3, 6),
    (4, 1),
    (4, 2),
    (4, 3),
    (4, 4),
    (4, 5),
    (5, 2),
    (5, 3),
    (5, 4),
    (1, 1),
    (1, 2),
    (1, 3),
    (1, 4),
    (0, 1),
    (0, 2),
    (0, 3),
    (0, 4),
    (0, 5),
    (6, 0),
    (6, 1),
    (6, 2),
    (6, 3),
    (6, 4),
    (6, 5),
    (6, 6),
];

/// A face drawn for the 160 x 43 LCD the G13 shares with the G15, G15 v2 and
/// G510: [`LCD_BYTES`] of column bands, one bit per pixel, set where the face
/// is bright.
pub fn lcd_frame(face: Option<&Face>, format: &KeyImage) -> DriverResult<Vec<u8>> {
    let mut frame = face::encode(face, format)?;
    frame.resize(LCD_BYTES, 0);
    Ok(frame)
}

/// The output report that shows `pixels` on that LCD: id `03`, 31 zero bytes,
/// then the 960 pixel bytes (992 bytes in all). Missing pixels are blank and
/// extra ones are dropped, so `&[]` blanks the screen.
#[must_use]
pub fn lcd_report(pixels: &[u8]) -> Vec<u8> {
    let mut report = vec![0u8; LCD_HEADER + LCD_BYTES];
    report[0] = LCD_REPORT;
    let len = pixels.len().min(LCD_BYTES);
    report[LCD_HEADER..LCD_HEADER + len].copy_from_slice(&pixels[..len]);
    report
}

pub struct G13 {
    model: &'static DeviceModel,
    transport: Box<dyn Transport>,
    /// The bitmap bits of the keys held at the last report.
    held: u64,
    /// Whether each stick direction (up, down, left, right) is held.
    stick: [bool; 4],
    /// The LCD frame to show, and the one last sent.
    lcd: Vec<u8>,
    sent_lcd: Option<Vec<u8>>,
    /// The M-key LED mask to show, and the one last sent.
    leds: u8,
    sent_leds: Option<u8>,
    /// The backlight colour before brightness, `None` for off.
    color: Option<[u8; 3]>,
    brightness: u8,
    sent_backlight: Option<[u8; 3]>,
}

impl G13 {
    #[must_use]
    pub fn new(model: &'static DeviceModel, transport: Box<dyn Transport>) -> Self {
        Self {
            model,
            transport,
            held: 0,
            stick: [false; 4],
            lcd: vec![0; LCD_BYTES],
            sent_lcd: None,
            leds: 0,
            sent_leds: None,
            color: None,
            brightness: 100,
            sent_backlight: None,
        }
    }

    fn parse(&mut self, report: &[u8], events: &mut Vec<Input>) {
        let ([_, x, y, bitmap @ ..], REPORT_LEN) = (report, report.len()) else {
            return;
        };
        let mut bits = 0u64;
        for (at, byte) in bitmap.iter().enumerate() {
            bits |= u64::from(*byte) << (8 * at);
        }
        for (index, bit) in (0u8..).zip(KEY_BITS) {
            let mask = 1u64 << bit;
            let now = bits & mask != 0;
            if now != (self.held & mask != 0) {
                events.push(Input::Key {
                    index,
                    pressed: now,
                });
            }
        }
        self.held = bits;
        // Up, down, left, right: low Y is up, low X is left.
        let directions = [(*y, true), (*y, false), (*x, true), (*x, false)];
        for (offset, (value, low)) in (0u8..).zip(directions) {
            let was = self.stick[usize::from(offset)];
            let now = if low {
                value < PRESS_LOW || (was && value < RELEASE_LOW)
            } else {
                value > PRESS_HIGH || (was && value > RELEASE_HIGH)
            };
            if now != was {
                self.stick[usize::from(offset)] = now;
                events.push(Input::Key {
                    index: STICK_KEY + offset,
                    pressed: now,
                });
            }
        }
    }

    fn info_image(&self) -> Option<(u8, KeyImage)> {
        self.model.info_key().zip(self.model.info_image)
    }

    fn scaled_backlight(&self) -> [u8; 3] {
        let [r, g, b] = self.color.unwrap_or([0, 0, 0]);
        let scale =
            |channel: u8| u8::try_from(u16::from(channel) * u16::from(self.brightness) / 100);
        [
            scale(r).unwrap_or(u8::MAX),
            scale(g).unwrap_or(u8::MAX),
            scale(b).unwrap_or(u8::MAX),
        ]
    }

    /// Sends a light's feature report. Neither has been checked through hidapi
    /// on hardware, so a refusal is logged rather than failing the paint.
    fn send_feature(&mut self, report: &[u8], what: &str) {
        if let Err(error) = self.transport.send_feature_report(report) {
            tracing::warn!(%error, "ryu-decks: the G13 did not take the {what} report");
        }
    }
}

impl Driver for G13 {
    fn poll(&mut self, timeout_ms: i32) -> DriverResult<Vec<Input>> {
        let mut events = Vec::new();
        let mut buf = [0u8; READ_BUFFER];
        for attempt in 0..MAX_REPORTS_PER_POLL {
            let wait = if attempt == 0 { timeout_ms } else { 0 };
            let read = self.transport.read_timeout(&mut buf, wait)?;
            if read == 0 {
                break;
            }
            self.parse(buf.get(..read).unwrap_or_default(), &mut events);
        }
        Ok(events)
    }

    /// The LCD takes the info key's face in one bit per pixel; M1 to MR take an
    /// on or off LED from the colour. Other keys have no light of their own.
    fn set_face(&mut self, key: u8, face: Option<&Face>) -> DriverResult<()> {
        if let Some((info, format)) = self.info_image() {
            if key == info {
                self.lcd = lcd_frame(face, &format)?;
                return Ok(());
            }
        }
        let Some(bit) = LIT_KEYS.iter().position(|lit| *lit == key) else {
            return Ok(());
        };
        let [r, g, b] = face.map_or([0, 0, 0], |face| face.color);
        let mask = 1u8 << bit;
        if r.max(g).max(b) > ON_OFF_THRESHOLD {
            self.leds |= mask;
        } else {
            self.leds &= !mask;
        }
        Ok(())
    }

    /// Sends the LEDs, the backlight and the LCD, each only when it changed.
    fn flush(&mut self) -> DriverResult<()> {
        if self.sent_leds != Some(self.leds) {
            self.send_feature(&[LED_REPORT, self.leds, 0, 0, 0], "mode LED");
            self.sent_leds = Some(self.leds);
        }
        let backlight = self.scaled_backlight();
        if self.sent_backlight != Some(backlight) {
            let [r, g, b] = backlight;
            self.send_feature(&[BACKLIGHT_REPORT, r, g, b, 0], "backlight");
            self.sent_backlight = Some(backlight);
        }
        if self.sent_lcd.as_ref() != Some(&self.lcd) {
            self.transport.write(&lcd_report(&self.lcd))?;
            self.sent_lcd = Some(self.lcd.clone());
        }
        Ok(())
    }

    /// No brightness command: the backlight colour is scaled, which dims the
    /// LCD with it.
    fn set_brightness(&mut self, percent: u8) -> DriverResult<()> {
        self.brightness = percent.min(100);
        self.flush()
    }

    fn set_deck_light(&mut self, color: Option<[u8; 3]>) -> DriverResult<()> {
        self.color = color;
        Ok(())
    }
}

impl Drop for G13 {
    /// Leaves the LCD blank and the lights off when Ryu lets go of the board.
    fn drop(&mut self) {
        let _ = self
            .transport
            .send_feature_report(&[LED_REPORT, 0, 0, 0, 0]);
        let _ = self
            .transport
            .send_feature_report(&[BACKLIGHT_REPORT, 0, 0, 0, 0]);
        let _ = self.transport.write(&lcd_report(&[]));
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};

    use super::*;
    use crate::catalog::by_id;
    use crate::driver::testing::Recorder;

    fn board() -> (G13, Recorder) {
        let recorder = Recorder::default();
        (
            G13::new(by_id("logitech-g13").unwrap(), Box::new(recorder.clone())),
            recorder,
        )
    }

    /// A report with the stick at rest and the given bitmap bits set.
    fn report(bits: &[u8]) -> Vec<u8> {
        let mut report = vec![0x01, 0x80, 0x80, 0, 0, 0, 0, 0];
        for bit in bits {
            report[3 + usize::from(bit / 8)] |= 1 << (bit % 8);
        }
        report
    }

    fn stick(x: u8, y: u8) -> Vec<u8> {
        vec![0x01, x, y, 0, 0, 0, 0, 0]
    }

    fn key(index: u8, pressed: bool) -> Input {
        Input::Key { index, pressed }
    }

    fn color(rgb: [u8; 3]) -> Face {
        Face {
            label: String::new(),
            color: rgb,
            image: None,
        }
    }

    #[test]
    fn keys_follow_the_bitmap_in_ryu_order() {
        let (mut g13, recorder) = board();
        // G1, G22, M1, MR, the round button, L4, thumb left, the stick click.
        recorder.queue_read(report(&[0, 21, 29, 32, 24, 28, 33, 35]));
        recorder.queue_read(report(&[21]));
        let events = g13.poll(0).unwrap();
        assert_eq!(
            events,
            vec![
                key(0, true),
                key(21, true),
                key(22, true),
                key(25, true),
                key(26, true),
                key(30, true),
                key(31, true),
                key(33, true),
                key(0, false),
                key(22, false),
                key(25, false),
                key(26, false),
                key(30, false),
                key(31, false),
                key(33, false),
            ]
        );
    }

    #[test]
    fn state_bits_and_the_backlight_button_are_not_keys() {
        let (mut g13, recorder) = board();
        // UNDEF1, LIGHT_STATE, UNDEF3, LIGHT, LIGHT2, MISC_TOGGLE.
        recorder.queue_read(report(&[22, 23, 36, 37, 38, 39]));
        assert!(g13.poll(0).unwrap().is_empty());
    }

    #[test]
    fn reports_of_another_length_are_dropped() {
        let (mut g13, recorder) = board();
        recorder.queue_read(vec![0x01, 0x80, 0x80, 0xff]);
        let mut long = report(&[0]);
        long.push(0);
        recorder.queue_read(long);
        assert!(g13.poll(0).unwrap().is_empty());
    }

    #[test]
    fn the_stick_is_four_keys_with_hysteresis() {
        let (mut g13, recorder) = board();
        // Up, then easing back but still past the release line, then rest.
        recorder.queue_read(stick(0x80, 0x10));
        recorder.queue_read(stick(0x80, 0x48));
        recorder.queue_read(stick(0x80, 0x50));
        // Right and down together, then back to rest.
        recorder.queue_read(stick(0xf0, 0xf0));
        recorder.queue_read(stick(0xb8, 0xb8));
        recorder.queue_read(stick(0x80, 0x80));
        // Left, then a value between the lines that does not press it again.
        recorder.queue_read(stick(0x00, 0x80));
        recorder.queue_read(stick(0x80, 0x80));
        recorder.queue_read(stick(0x48, 0x80));
        assert_eq!(
            g13.poll(0).unwrap(),
            vec![
                key(34, true),
                key(34, false),
                key(35, true),
                key(37, true),
                key(35, false),
                key(37, false),
                key(36, true),
                key(36, false),
            ]
        );
    }

    #[test]
    fn first_flush_sends_the_leds_backlight_and_a_blank_lcd() {
        let (mut g13, recorder) = board();
        g13.set_deck_light(Some([0, 0, 255])).unwrap();
        g13.flush().unwrap();
        assert_eq!(
            recorder.features(),
            vec![vec![0x05, 0, 0, 0, 0], vec![0x07, 0, 0, 255, 0]]
        );
        let writes = recorder.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].len(), 992);
        assert_eq!(writes[0][0], 0x03);
        assert!(writes[0][1..].iter().all(|byte| *byte == 0));
        // Nothing changed, so nothing is sent again.
        g13.flush().unwrap();
        assert_eq!(recorder.features().len(), 2);
        assert_eq!(recorder.writes().len(), 1);
    }

    #[test]
    fn mode_keys_share_one_led_mask() {
        let (mut g13, recorder) = board();
        g13.flush().unwrap();
        g13.set_face(22, Some(&color([255, 0, 0]))).unwrap();
        g13.set_face(24, Some(&color([0, 40, 0]))).unwrap();
        g13.set_face(25, Some(&color([10, 10, 10]))).unwrap();
        // A G-key has no LED of its own.
        g13.set_face(0, Some(&color([255, 255, 255]))).unwrap();
        g13.flush().unwrap();
        g13.set_face(22, None).unwrap();
        g13.set_face(23, Some(&color([255, 255, 255]))).unwrap();
        g13.flush().unwrap();
        let features = recorder.features();
        assert_eq!(features[2], vec![0x05, 0x05, 0, 0, 0]);
        assert_eq!(features[3], vec![0x05, 0x06, 0, 0, 0]);
        assert_eq!(features.len(), 4);
    }

    #[test]
    fn brightness_scales_the_backlight() {
        let (mut g13, recorder) = board();
        g13.set_deck_light(Some([200, 100, 50])).unwrap();
        g13.flush().unwrap();
        g13.set_brightness(50).unwrap();
        g13.set_deck_light(None).unwrap();
        g13.flush().unwrap();
        let features = recorder.features();
        assert_eq!(features[1], vec![0x07, 200, 100, 50, 0]);
        assert_eq!(features[2], vec![0x07, 100, 50, 25, 0]);
        assert_eq!(features[3], vec![0x07, 0, 0, 0, 0]);
    }

    #[test]
    fn the_info_key_draws_the_lcd_in_column_bands() {
        let (mut g13, recorder) = board();
        assert_eq!(g13.model.info_key(), Some(38));
        // White pixels at (0, 0), (5, 9) and (159, 42) on black.
        let mut picture = RgbaImage::from_pixel(160, 43, Rgba([0, 0, 0, 255]));
        for (x, y) in [(0, 0), (5, 9), (159, 42)] {
            picture.put_pixel(x, y, Rgba([255, 255, 255, 255]));
        }
        let face = Face {
            label: "LCD".to_owned(),
            color: [0, 0, 0],
            image: Some(picture),
        };
        g13.set_face(38, Some(&face)).unwrap();
        g13.flush().unwrap();
        let lcd = &recorder.writes()[0][32..];
        assert_eq!(lcd.len(), 960);
        assert_eq!(lcd[0], 0x01);
        // Row 9 is bit 1 of band 1.
        assert_eq!(lcd[5 + 160], 0x02);
        // Row 42 is bit 2 of band 5; the five rows below it are off the glass.
        assert_eq!(lcd[159 + 5 * 160], 0x04);
        assert_eq!(lcd.iter().filter(|byte| **byte != 0).count(), 3);
    }

    #[test]
    fn dropping_the_board_blanks_it() {
        let (g13, recorder) = board();
        drop(g13);
        assert_eq!(
            recorder.features(),
            vec![vec![0x05, 0, 0, 0, 0], vec![0x07, 0, 0, 0, 0]]
        );
        assert_eq!(recorder.writes()[0].len(), 992);
    }
}
