//! Turning a [`Face`] into the exact bytes a key screen expects.

use std::io::Cursor;

use image::codecs::bmp::BmpEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::{self, FilterType};
use image::{ExtendedColorType, ImageEncoder, Rgba, RgbaImage};

use super::{DriverError, DriverResult, Face};
use crate::catalog::{ImageMode, KeyImage, Mirror, Rotation};

/// The face as an upright `width` x `height` picture: the colour, with the rendered
/// image scaled over it. Unbound keys are black.
#[must_use]
pub fn compose(face: Option<&Face>, width: u32, height: u32) -> RgbaImage {
    let [r, g, b] = face.map_or([0, 0, 0], |face| face.color);
    let mut canvas = RgbaImage::from_pixel(width, height, Rgba([r, g, b, 255]));
    if let Some(picture) = face.and_then(|face| face.image.as_ref()) {
        let scaled = if picture.dimensions() == (width, height) {
            picture.clone()
        } else {
            imageops::resize(picture, width, height, FilterType::Triangle)
        };
        imageops::overlay(&mut canvas, &scaled, 0, 0);
    }
    canvas
}

/// Encode a face for a key with the given geometry: orient it the way the panel
/// is mounted, then encode it in the panel's format.
pub fn encode(face: Option<&Face>, format: &KeyImage) -> DriverResult<Vec<u8>> {
    let upright = compose(face, format.width, format.height);
    let rotated = match format.rotation {
        Rotation::None => upright,
        Rotation::Quarter => imageops::rotate90(&upright),
        Rotation::ThreeQuarter => imageops::rotate270(&upright),
    };
    let oriented = match format.mirror {
        Mirror::None => rotated,
        Mirror::Horizontal => imageops::flip_horizontal(&rotated),
        Mirror::Both => imageops::rotate180(&rotated),
    };
    encode_image(&oriented, format.mode)
}

fn encode_image(picture: &RgbaImage, mode: ImageMode) -> DriverResult<Vec<u8>> {
    let (width, height) = picture.dimensions();
    let mut out = Vec::new();
    let result = match mode {
        ImageMode::Rgb565 | ImageMode::Rgb565Be => {
            out.reserve(picture.as_raw().len() / 2);
            for pixel in picture.pixels() {
                let [r, g, b, _] = pixel.0;
                let packed =
                    (u16::from(r & 0xf8) << 8) | (u16::from(g & 0xfc) << 3) | u16::from(b >> 3);
                if mode == ImageMode::Rgb565Be {
                    out.extend_from_slice(&packed.to_be_bytes());
                } else {
                    out.extend_from_slice(&packed.to_le_bytes());
                }
            }
            Ok(())
        }
        ImageMode::Bgr24 => {
            out.reserve(picture.as_raw().len() / 4 * 3);
            for pixel in picture.pixels() {
                let [r, g, b, _] = pixel.0;
                out.extend_from_slice(&[b, g, r]);
            }
            Ok(())
        }
        ImageMode::MonoColumns => {
            out = pack_columns(picture);
            Ok(())
        }
        ImageMode::MonoRows => {
            out = pack_rows(picture);
            Ok(())
        }
        ImageMode::Png => PngEncoder::new(Cursor::new(&mut out)).write_image(
            picture.as_raw(),
            width,
            height,
            ExtendedColorType::Rgba8,
        ),
        ImageMode::Jpeg | ImageMode::Bmp => {
            // Neither panel format carries alpha; the face is already opaque.
            let rgb = image::DynamicImage::ImageRgba8(picture.clone()).into_rgb8();
            if mode == ImageMode::Jpeg {
                JpegEncoder::new_with_quality(&mut out, 90).write_image(
                    rgb.as_raw(),
                    width,
                    height,
                    ExtendedColorType::Rgb8,
                )
            } else {
                BmpEncoder::new(&mut out).write_image(
                    rgb.as_raw(),
                    width,
                    height,
                    ExtendedColorType::Rgb8,
                )
            }
        }
    };
    result.map_err(|error| DriverError::Image(error.to_string()))?;
    Ok(out)
}

/// One bit per pixel in 8-row bands, one byte per column of each band with
/// bit 0 the band's top row (the Logitech G13's LCD). A pixel is set when it is
/// bright (luma of 128 or more), so a light label on a dark key shows as lit
/// text. The last band is padded with dark rows: always `width * ceil(height /
/// 8)` bytes.
fn pack_columns(picture: &RgbaImage) -> Vec<u8> {
    let (width, height) = picture.dimensions();
    let bands = height.div_ceil(8);
    let mut out = vec![0u8; (width * bands) as usize];
    for (x, y, pixel) in picture.enumerate_pixels() {
        let [r, g, b, _] = pixel.0;
        let luma = (299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b)) / 1000;
        if luma >= 128 {
            out[(x + (y / 8) * width) as usize] |= 1 << (y % 8);
        }
    }
    out
}

/// One bit per pixel, row by row, the most significant bit of each byte the
/// leftmost pixel (the Maschine MK2's displays). A pixel is set when it is
/// bright, as in [`pack_columns`]; each row is padded to whole bytes.
fn pack_rows(picture: &RgbaImage) -> Vec<u8> {
    let (width, height) = picture.dimensions();
    let row_bytes = width.div_ceil(8);
    let mut out = vec![0u8; (row_bytes * height) as usize];
    for (x, y, pixel) in picture.enumerate_pixels() {
        let [r, g, b, _] = pixel.0;
        let luma = (299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b)) / 1000;
        if luma >= 128 {
            out[(y * row_bytes + x / 8) as usize] |= 0x80 >> (x % 8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face_with_marker() -> Face {
        // A white face with one red pixel in the top-left corner.
        let mut picture = RgbaImage::from_pixel(4, 4, Rgba([255, 255, 255, 255]));
        picture.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        Face {
            label: String::new(),
            color: [0, 0, 0],
            image: Some(picture),
        }
    }

    #[test]
    fn unbound_keys_are_black() {
        let picture = compose(None, 2, 2);
        assert!(picture.pixels().all(|p| *p == Rgba([0, 0, 0, 255])));
    }

    #[test]
    fn a_colour_without_an_image_fills_the_key() {
        let face = Face {
            label: String::new(),
            color: [10, 20, 30],
            image: None,
        };
        let picture = compose(Some(&face), 3, 3);
        assert!(picture.pixels().all(|p| *p == Rgba([10, 20, 30, 255])));
    }

    #[test]
    fn upside_down_panels_get_the_face_rotated() {
        let format = KeyImage {
            width: 4,
            height: 4,
            mode: ImageMode::Png,
            rotation: Rotation::None,
            mirror: Mirror::Both,
        };
        let bytes = encode(Some(&face_with_marker()), &format).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().into_rgba8();
        assert_eq!(*decoded.get_pixel(3, 3), Rgba([255, 0, 0, 255]));
        assert_eq!(*decoded.get_pixel(0, 0), Rgba([255, 255, 255, 255]));
    }

    #[test]
    fn encodes_each_panel_format() {
        for mode in [ImageMode::Bmp, ImageMode::Jpeg, ImageMode::Png] {
            let format = KeyImage {
                width: 72,
                height: 72,
                mode,
                rotation: Rotation::Quarter,
                mirror: Mirror::Horizontal,
            };
            let bytes = encode(None, &format).unwrap();
            let decoded = image::load_from_memory(&bytes).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (72, 72), "{mode:?}");
        }
    }

    #[test]
    fn rgb565_is_little_endian_five_six_five() {
        let face = Face {
            label: String::new(),
            color: [0xff, 0x00, 0x08],
            image: None,
        };
        let format = KeyImage {
            width: 2,
            height: 1,
            mode: ImageMode::Rgb565,
            rotation: Rotation::None,
            mirror: Mirror::None,
        };
        // Red 0b11111 << 11, blue 0b00001: 0xf801, low byte first.
        assert_eq!(
            encode(Some(&face), &format).unwrap(),
            vec![0x01, 0xf8, 0x01, 0xf8]
        );
        // The CT wheel takes the same pixels high byte first.
        let big_endian = KeyImage {
            mode: ImageMode::Rgb565Be,
            ..format
        };
        assert_eq!(
            encode(Some(&face), &big_endian).unwrap(),
            vec![0xf8, 0x01, 0xf8, 0x01]
        );
    }

    #[test]
    fn mono_columns_light_bright_pixels_in_eight_row_bands() {
        let format = KeyImage {
            width: 2,
            height: 10,
            mode: ImageMode::MonoColumns,
            rotation: Rotation::None,
            mirror: Mirror::None,
        };
        let yellow = Face {
            label: String::new(),
            color: [255, 255, 0],
            image: None,
        };
        assert_eq!(
            encode(Some(&yellow), &format).unwrap(),
            vec![0xff, 0xff, 0x03, 0x03]
        );
        let blue = Face {
            color: [0, 0, 255],
            ..yellow
        };
        assert_eq!(encode(Some(&blue), &format).unwrap(), vec![0, 0, 0, 0]);
    }

    #[test]
    fn mono_rows_set_the_leftmost_pixel_in_the_top_bit() {
        let format = KeyImage {
            width: 10,
            height: 2,
            mode: ImageMode::MonoRows,
            rotation: Rotation::None,
            mirror: Mirror::None,
        };
        let white = Face {
            label: String::new(),
            color: [255, 255, 255],
            image: None,
        };
        assert_eq!(
            encode(Some(&white), &format).unwrap(),
            vec![0xff, 0xc0, 0xff, 0xc0]
        );
        let mut picture = RgbaImage::from_pixel(10, 2, Rgba([0, 0, 0, 255]));
        picture.put_pixel(1, 1, Rgba([255, 255, 255, 255]));
        let dot = Face {
            image: Some(picture),
            color: [0, 0, 0],
            ..white
        };
        assert_eq!(encode(Some(&dot), &format).unwrap(), vec![0, 0, 0x40, 0]);
    }
}
