//! Reusable PNG decoding for screenshot-based visual checks.
//!
//! This is the single PNG decoder used by colour sampling and screenshot
//! comparison. It normalises every supported PNG colour type to row-major
//! RGBA bytes so callers never have to interpret PNG-specific layouts.
//! The colour module in [`crate::color`] works on the [`Image`] produced here.

use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use androidforge_core::error::{ErrorCode, ForgeError, Result};

/// A decoded image in row-major RGBA order (four bytes per pixel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// RGBA samples in row-major order: `(y * width + x) * 4`.
    pub rgba: Vec<u8>,
}

impl Image {
    /// Build an image from an existing RGBA buffer.
    ///
    /// The buffer length must be exactly `width * height * 4`; no padding,
    /// strides or premultiplied alpha are accepted.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self> {
        let expected = pixel_count(width, height)?.checked_mul(4).ok_or_else(|| {
            ForgeError::invalid_argument("image RGBA buffer size overflows usize")
        })?;
        if rgba.len() != expected {
            return Err(ForgeError::invalid_argument(format!(
                "RGBA buffer has {} bytes, expected {} for a {width}x{height} image",
                rgba.len(),
                expected
            )));
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// Whether `(x, y)` addresses a pixel in this image.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x < self.width && y < self.height
    }
}

/// Decode a PNG file into RGBA bytes.
///
/// Malformed images, unsupported buffer sizes and unexpected palette data are
/// reported as `E_UI_ASSERTION_FAILED` errors that name the offending file.
pub fn decode(path: &Path) -> Result<Image> {
    let file = File::open(path).map_err(|error| {
        ForgeError::new(
            ErrorCode::FileNotFound,
            format!("could not open screenshot {}: {error}", path.display()),
        )
    })?;
    decode_reader(file, &path.display().to_string())
}

/// Decode a PNG from any reader; `source` is included in error messages.
///
/// This is the reusable core behind [`decode`]. It applies the same
/// transformations as the CLI screenshot decoder
/// (`EXPAND | STRIP_16`) so all callers see identical 8-bit samples.
pub fn decode_reader<R: Read + Seek>(reader: R, source: &str) -> Result<Image> {
    let mut decoder = png::Decoder::new(BufReader::new(reader));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| malformed(source, format!("could not decode PNG: {error}")))?;
    let buffer_size = reader
        .output_buffer_size()
        .ok_or_else(|| malformed(source, "declares an unsupported PNG buffer size"))?;
    let mut buffer = vec![0u8; buffer_size];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| malformed(source, format!("could not read PNG: {error}")))?;
    let width = info.width;
    let height = info.height;
    let samples = info.color_type.samples();
    let capacity = pixel_count(width, height).and_then(|count| {
        count
            .checked_mul(4)
            .ok_or_else(|| malformed(source, "decoded image size overflows usize"))
    })?;
    let mut rgba = Vec::with_capacity(capacity);
    for pixel in buffer[..info.buffer_size()].chunks_exact(samples) {
        match info.color_type {
            png::ColorType::Rgba => rgba.extend_from_slice(&pixel[..4]),
            png::ColorType::Rgb => {
                rgba.extend_from_slice(&pixel[..3]);
                rgba.push(255);
            }
            png::ColorType::GrayscaleAlpha => {
                let value = pixel[0];
                rgba.extend_from_slice(&[value, value, value, pixel[1]]);
            }
            png::ColorType::Grayscale => {
                let value = pixel[0];
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
            png::ColorType::Indexed => {
                return Err(malformed(
                    source,
                    "stayed palette-indexed after decoding; re-export it as RGB/RGBA",
                ));
            }
        }
    }
    if rgba.len() != capacity {
        return Err(malformed(
            source,
            format!("decoded to {} RGBA bytes, expected {capacity}", rgba.len()),
        ));
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

fn pixel_count(width: u32, height: u32) -> Result<usize> {
    (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| ForgeError::invalid_argument("image pixel count overflows usize"))
}

fn malformed(source: &str, detail: impl fmt::Display) -> ForgeError {
    ForgeError::new(
        ErrorCode::UiAssertionFailed,
        format!("malformed PNG {source}: {detail}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(path: &Path, width: u32, height: u32, pixels: &[[u8; 3]]) {
        let file = File::create(path).unwrap();
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let bytes = pixels
            .iter()
            .flat_map(|pixel| pixel.iter().copied())
            .collect::<Vec<_>>();
        writer.write_image_data(&bytes).unwrap();
    }

    #[test]
    fn decode_normalises_rgb_png_to_opaque_rgba() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("fixture.png");
        write_png(&path, 2, 1, &[[10, 20, 30], [40, 50, 60]]);

        let image = decode(&path).unwrap();
        assert_eq!(image.width, 2);
        assert_eq!(image.height, 1);
        assert_eq!(image.rgba, vec![10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[test]
    fn decode_rejects_non_png_bytes_with_explicit_message() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("bad.png");
        std::fs::write(&path, b"not a png").unwrap();

        let error = decode(&path).unwrap_err();
        assert_eq!(error.code(), ErrorCode::UiAssertionFailed);
        assert!(
            error.message().contains("malformed PNG"),
            "unexpected message: {}",
            error.message()
        );
        assert!(error.message().contains("bad.png"));
    }

    #[test]
    fn from_rgba_rejects_wrong_buffer_length() {
        let error = Image::from_rgba(2, 1, vec![0, 0, 0, 255]).unwrap_err();
        assert!(error.message().contains("expected 8 for a 2x1 image"));
    }
}
