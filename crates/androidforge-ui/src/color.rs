//! Colour parsing, pixel sampling and tolerance comparison for screenshot checks.
//!
//! This module is pure standard-library code; PNG decoding is delegated to
//! [`crate::visual`] so there is exactly one decoder in the crate.
//!
//! # Coordinate and alpha conventions
//!
//! * Coordinates are device pixels, `(x, y)` with `(0, 0)` at the top-left.
//! * [`Image`](crate::visual::Image) buffers are row-major RGBA with 8 bits per
//!   channel. Sampling returns the raw RGBA value; no alpha compositing or
//!   premultiplication is performed.
//! * Hex colours are `#RRGGBB` or `#RGB`; they are always fully opaque
//!   (`alpha = 255`). An explicit alpha suffix is deliberately rejected so
//!   callers cannot accidentally introduce a different alpha policy.
//! * Colour comparisons use the RGB channels only. Alpha is ignored because
//!   device screenshots are opaque; a semi-transparent pixel is compared by
//!   its stored RGB bytes, not by compositing it over any background.
//!
//! # Tolerance metric
//!
//! [`compare`], [`matches`] and [`check_pixel`] use the **maximum absolute
//! difference across the R, G and B channels** (each in `0..=255`, as `u8`).
//! A tolerance `t` passes when `max_channel_delta <= t`, so `t = 0` means an
//! exact match. This is a plain channel-wise metric: there is no perceptual,
//! gamma-corrected or CIE-based distance hidden behind the API. The separate
//! [`contrast_ratio`] helper is an explicitly documented WCAG calculation and
//! is never used by tolerance comparison.

use std::collections::BTreeMap;
use std::fmt;

use androidforge_core::error::{ForgeError, Result};

use crate::visual::Image;

/// An opaque 8-bit RGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Convert to RGBA with fully opaque alpha.
    pub const fn to_rgba(self) -> Rgba {
        Rgba {
            r: self.r,
            g: self.g,
            b: self.b,
            a: 255,
        }
    }

    /// Canonical uppercase `#RRGGBB` spelling.
    pub fn hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

/// An 8-bit RGBA colour. Alpha is preserved from the decoded PNG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Drop alpha; comparison never considers the alpha channel.
    pub const fn rgb(self) -> Rgb {
        Rgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }
}

impl From<Rgb> for Rgba {
    fn from(color: Rgb) -> Self {
        color.to_rgba()
    }
}

impl From<Rgba> for Rgb {
    fn from(color: Rgba) -> Self {
        color.rgb()
    }
}

/// Parse `#RRGGBB` or `#RGB` into an opaque [`Rgb`] value.
///
/// `#abc` expands each nibble (`#aabbcc`); both upper- and lowercase digits
/// are accepted. Leading/trailing whitespace is ignored. Eight-digit forms
/// with an alpha component are rejected with an explicit message.
pub fn parse_hex(value: &str) -> Result<Rgb> {
    let text = value.trim();
    let digits = text.strip_prefix('#').ok_or_else(|| {
        invalid_hex(
            value,
            "expected a leading '#' followed by 3 or 6 hex digits",
        )
    })?;
    if !digits.is_ascii() {
        return Err(invalid_hex(value, "expected ASCII hexadecimal digits only"));
    }
    let byte = digits.as_bytes();
    let nibble = |index: usize| -> Result<u8> {
        (byte[index] as char)
            .to_digit(16)
            .map(|digit| digit as u8)
            .ok_or_else(|| invalid_hex(value, "expected hexadecimal digits 0-9, a-f or A-F only"))
    };
    match byte.len() {
        3 => Ok(Rgb::new(nibble(0)? * 17, nibble(1)? * 17, nibble(2)? * 17)),
        6 => Ok(Rgb::new(
            nibble(0)? << 4 | nibble(1)?,
            nibble(2)? << 4 | nibble(3)?,
            nibble(4)? << 4 | nibble(5)?,
        )),
        8 => Err(invalid_hex(
            value,
            "alpha is not accepted: use #RRGGBB (implicit alpha 255)",
        )),
        _ => Err(invalid_hex(
            value,
            "expected 3 (#RGB) or 6 (#RRGGBB) hexadecimal digits",
        )),
    }
}

fn invalid_hex(value: &str, detail: &str) -> ForgeError {
    ForgeError::invalid_argument(format!("invalid colour {value:?}: {detail}"))
}

/// A positive, axis-aligned rectangle in device pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    /// Create a region; empty regions are rejected explicitly.
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(ForgeError::invalid_argument(
                "region width and height must both be greater than zero",
            ));
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// The entire image.
    pub fn full(image: &Image) -> Self {
        Self {
            x: 0,
            y: 0,
            width: image.width,
            height: image.height,
        }
    }

    /// Inclusive-free right edge, if it fits in `u32`.
    pub fn right(&self) -> Option<u32> {
        self.x.checked_add(self.width)
    }

    /// Inclusive-free bottom edge, if it fits in `u32`.
    pub fn bottom(&self) -> Option<u32> {
        self.y.checked_add(self.height)
    }
}

/// Read the RGBA value at `(x, y)`.
///
/// Out-of-bounds coordinates return `E_INVALID_ARGUMENT` with the requested
/// point and the image dimensions in the message.
pub fn sample_pixel(image: &Image, x: u32, y: u32) -> Result<Rgba> {
    if !image.contains(x, y) {
        return Err(ForgeError::invalid_argument(format!(
            "pixel ({x}, {y}) is outside the {}x{} image",
            image.width, image.height
        )));
    }
    let index = (y as usize * image.width as usize + x as usize) * 4;
    Ok(Rgba::new(
        image.rgba[index],
        image.rgba[index + 1],
        image.rgba[index + 2],
        image.rgba[index + 3],
    ))
}

/// Read the RGB channels at `(x, y)`, discarding alpha.
pub fn sample_rgb(image: &Image, x: u32, y: u32) -> Result<Rgb> {
    sample_pixel(image, x, y).map(Rgba::rgb)
}

/// Most frequent RGB colour in a region.
///
/// Alpha is ignored. Ties are resolved deterministically: the colour that
/// first appears in row-major order (top-left, left-to-right, top-to-bottom)
/// wins. Zero-sized or out-of-bounds regions are errors, never empty results.
pub fn majority_color(image: &Image, region: Region) -> Result<Rgb> {
    let mut counts: BTreeMap<Rgb, (u32, u64)> = BTreeMap::new();
    let mut first_index = 0u64;
    for_each_pixel(image, region, |color| {
        match counts.get_mut(&color) {
            Some(entry) => entry.0 += 1,
            None => {
                counts.insert(color, (1, first_index));
            }
        }
        first_index += 1;
    })?;

    let mut best: Option<(Rgb, u32, u64)> = None;
    for (color, (count, seen)) in counts {
        let is_better = best.is_none_or(|(_, best_count, best_seen)| {
            count > best_count || (count == best_count && seen < best_seen)
        });
        if is_better {
            best = Some((color, count, seen));
        }
    }
    best.map(|(color, _, _)| color)
        .ok_or_else(|| ForgeError::invalid_argument("region contains no pixels"))
}

/// Per-channel average RGB colour in a region, rounding half up.
///
/// Each channel is summed as `u64` (so large regions cannot overflow), then
/// divided by the pixel count with half-up rounding. Alpha is ignored.
pub fn average_color(image: &Image, region: Region) -> Result<Rgb> {
    let mut total = (0u64, 0u64, 0u64);
    let mut count = 0u64;
    for_each_pixel(image, region, |color| {
        total.0 += u64::from(color.r);
        total.1 += u64::from(color.g);
        total.2 += u64::from(color.b);
        count += 1;
    })?;
    let round_half_up = |sum: u64| -> u8 { ((sum + count / 2) / count) as u8 };
    Ok(Rgb::new(
        round_half_up(total.0),
        round_half_up(total.1),
        round_half_up(total.2),
    ))
}

/// Largest absolute difference between corresponding RGB channels.
///
/// The metric is `max(abs(a.r - b.r), abs(a.g - b.g), abs(a.b - b.b))`.
pub fn channel_delta(a: Rgb, b: Rgb) -> u8 {
    a.r.abs_diff(b.r)
        .max(a.g.abs_diff(b.g))
        .max(a.b.abs_diff(b.b))
}

/// Result of a tolerance comparison; see the module docs for the metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorComparison {
    pub expected: Rgb,
    pub actual: Rgb,
    pub tolerance: u8,
    pub max_channel_delta: u8,
    pub passed: bool,
}

/// Compare two colours using the documented max-channel-difference metric.
pub fn compare(expected: Rgb, actual: Rgb, tolerance: u8) -> ColorComparison {
    let delta = channel_delta(expected, actual);
    ColorComparison {
        expected,
        actual,
        tolerance,
        max_channel_delta: delta,
        passed: delta <= tolerance,
    }
}

/// Whether two colours match within `tolerance`.
pub fn matches(expected: Rgb, actual: Rgb, tolerance: u8) -> bool {
    compare(expected, actual, tolerance).passed
}

/// Sample `(x, y)` and compare it to `expected` in one step.
///
/// Useful for `capture assert-color`: a failed assertion can report the
/// actual colour and `max_channel_delta` carried by [`ColorComparison`].
pub fn check_pixel(
    image: &Image,
    x: u32,
    y: u32,
    expected: Rgb,
    tolerance: u8,
) -> Result<ColorComparison> {
    let actual = sample_rgb(image, x, y)?;
    Ok(compare(expected, actual, tolerance))
}

/// WCAG 2.x relative luminance of an opaque sRGB colour.
///
/// This is an explicitly defined formula (linearised sRGB with coefficients
/// 0.2126 / 0.7152 / 0.0722), not a perceptual distance used by [`compare`].
pub fn relative_luminance(color: Rgb) -> f64 {
    fn channel(value: u8) -> f64 {
        let value = f64::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
}

/// WCAG 2.x contrast ratio `(L_lighter + 0.05) / (L_darker + 0.05)`.
///
/// Returns `1.0` for identical colours and `21.0` for black versus white.
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let luminance_a = relative_luminance(a);
    let luminance_b = relative_luminance(b);
    let lighter = luminance_a.max(luminance_b);
    let darker = luminance_a.min(luminance_b);
    (lighter + 0.05) / (darker + 0.05)
}

fn for_each_pixel(image: &Image, region: Region, mut visit: impl FnMut(Rgb)) -> Result<()> {
    if region.width == 0 || region.height == 0 {
        return Err(ForgeError::invalid_argument(
            "region width and height must both be greater than zero",
        ));
    }
    let right = region.right().ok_or_else(|| {
        ForgeError::invalid_argument(format!(
            "region x {} + width {} overflows u32",
            region.x, region.width
        ))
    })?;
    let bottom = region.bottom().ok_or_else(|| {
        ForgeError::invalid_argument(format!(
            "region y {} + height {} overflows u32",
            region.y, region.height
        ))
    })?;
    if right > image.width || bottom > image.height {
        return Err(ForgeError::invalid_argument(format!(
            "region {}x{}+{},{} exceeds the {}x{} image",
            region.width, region.height, region.x, region.y, image.width, image.height
        )));
    }
    for y in region.y..bottom {
        for x in region.x..right {
            visit(sample_rgb(image, x, y)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image(pixels: &[[u8; 4]], width: u32, height: u32) -> Image {
        Image::from_rgba(width, height, pixels.iter().flatten().copied().collect()).unwrap()
    }

    #[test]
    fn parse_hex_expands_short_and_long_forms() {
        assert_eq!(parse_hex("#FfF").unwrap(), Rgb::new(255, 255, 255));
        assert_eq!(parse_hex("#1a2B3c").unwrap(), Rgb::new(0x1a, 0x2b, 0x3c));
        assert_eq!(
            parse_hex(" #abc ").unwrap().to_rgba(),
            Rgba::new(0xaa, 0xbb, 0xcc, 255)
        );
        assert_eq!(parse_hex("#1a2B3c").unwrap().hex(), "#1A2B3C");
    }

    #[test]
    fn parse_hex_rejects_malformed_values_with_explicit_messages() {
        let cases = [
            ("fff", "leading '#'"),
            ("#12", "3 (#RGB) or 6 (#RRGGBB)"),
            ("#12345", "3 (#RGB) or 6 (#RRGGBB)"),
            ("#gg0000", "hexadecimal digits"),
            ("#11223344", "alpha is not accepted"),
            ("#ünï", "ASCII hexadecimal"),
        ];
        for (value, expected) in cases {
            let error = parse_hex(value).unwrap_err();
            assert!(
                error.message().contains(expected),
                "{value}: unexpected message {}",
                error.message()
            );
        }
    }

    #[test]
    fn sample_pixel_reads_exact_rgba_bytes() {
        let image = test_image(&[[1, 2, 3, 4], [5, 6, 7, 8]], 2, 1);
        assert_eq!(sample_pixel(&image, 0, 0).unwrap(), Rgba::new(1, 2, 3, 4));
        assert_eq!(sample_pixel(&image, 1, 0).unwrap(), Rgba::new(5, 6, 7, 8));
        assert_eq!(sample_rgb(&image, 1, 0).unwrap(), Rgb::new(5, 6, 7));
    }

    #[test]
    fn out_of_bounds_sample_has_explicit_message() {
        let image = test_image(&[[0, 0, 0, 255]], 1, 1);
        for (x, y) in [(1, 0), (0, 1), (u32::MAX, 0)] {
            let error = sample_pixel(&image, x, y).unwrap_err();
            assert_eq!(
                error.code(),
                androidforge_core::error::ErrorCode::InvalidArgument
            );
            assert!(
                error
                    .message()
                    .contains(&format!("pixel ({x}, {y}) is outside the 1x1 image")),
                "unexpected message: {}",
                error.message()
            );
        }
    }

    #[test]
    fn majority_color_counts_rgb_and_breaks_ties_by_first_occurrence() {
        // Two colours occur twice each: the top-left one must win.
        let image = test_image(
            &[
                [9, 0, 0, 255],
                [8, 0, 0, 255],
                [8, 0, 0, 255],
                [9, 0, 0, 255],
            ],
            2,
            2,
        );
        assert_eq!(
            majority_color(&image, Region::full(&image)).unwrap(),
            Rgb::new(9, 0, 0)
        );

        // Alpha differences do not create distinct colours.
        let image = test_image(
            &[[4, 5, 6, 255], [4, 5, 6, 0], [4, 5, 6, 128], [1, 1, 1, 255]],
            2,
            2,
        );
        assert_eq!(
            majority_color(&image, Region::full(&image)).unwrap(),
            Rgb::new(4, 5, 6)
        );
    }

    #[test]
    fn majority_color_can_be_restricted_to_a_region() {
        let image = test_image(
            &[
                [1, 0, 0, 255],
                [2, 0, 0, 255],
                [2, 0, 0, 255],
                [2, 0, 0, 255],
            ],
            2,
            2,
        );
        assert_eq!(
            majority_color(&image, Region::new(0, 0, 1, 1).unwrap()).unwrap(),
            Rgb::new(1, 0, 0)
        );
    }

    #[test]
    fn average_color_uses_integer_half_up_rounding() {
        let image = test_image(&[[0, 0, 0, 255], [1, 3, 5, 255]], 2, 1);
        assert_eq!(
            average_color(&image, Region::full(&image)).unwrap(),
            Rgb::new(1, 2, 3)
        );
    }

    #[test]
    fn tolerance_boundary_is_inclusive() {
        let expected = Rgb::new(10, 20, 30);
        let actual = Rgb::new(13, 24, 33); // deltas 3, 4, 3
        assert_eq!(channel_delta(expected, actual), 4);
        assert!(!matches(expected, actual, 3));
        assert!(matches(expected, actual, 4));
        let comparison = compare(expected, actual, 4);
        assert!(comparison.passed);
        assert_eq!(comparison.max_channel_delta, 4);
        assert_eq!(comparison.actual, actual);
        assert!(matches(expected, expected, 0));
        assert!(!matches(expected, Rgb::new(11, 20, 30), 0));
    }

    #[test]
    fn check_pixel_reports_actual_color_and_delta() {
        let image = test_image(&[[10, 20, 30, 255]], 1, 1);
        let comparison = check_pixel(&image, 0, 0, Rgb::new(8, 20, 32), 1).unwrap();
        assert_eq!(comparison.actual, Rgb::new(10, 20, 30));
        assert_eq!(comparison.max_channel_delta, 2);
        assert!(!comparison.passed);
    }

    #[test]
    fn invalid_regions_report_explicit_errors() {
        let image = test_image(&[[0, 0, 0, 255]], 1, 1);
        assert!(
            Region::new(0, 0, 0, 1)
                .unwrap_err()
                .message()
                .contains("greater than zero")
        );

        let outside = Region::new(1, 0, 1, 1).unwrap();
        let error = majority_color(&image, outside).unwrap_err();
        assert!(error.message().contains("exceeds the 1x1 image"));

        let overflow = Region::new(u32::MAX, 0, 2, 1).unwrap();
        let error = average_color(&image, overflow).unwrap_err();
        assert!(error.message().contains("overflows u32"));
    }

    #[test]
    fn contrast_ratio_is_the_documented_wcag_value() {
        let ratio = contrast_ratio(Rgb::new(0, 0, 0), Rgb::new(255, 255, 255));
        assert!((ratio - 21.0).abs() < 1e-9, "unexpected ratio {ratio}");
        assert!((contrast_ratio(Rgb::new(18, 52, 86), Rgb::new(18, 52, 86)) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn generated_png_fixture_decodes_and_samples() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("sample.png");
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = png::Encoder::new(file, 2, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[0x22, 0x33, 0x44, 0xEE, 0xFF, 0x00])
            .unwrap();
        drop(writer);

        let decoded = crate::visual::decode(&path).unwrap();
        assert_eq!(
            sample_pixel(&decoded, 0, 0).unwrap(),
            Rgba::new(0x22, 0x33, 0x44, 255)
        );
        assert_eq!(
            sample_pixel(&decoded, 1, 0).unwrap(),
            Rgba::new(0xEE, 0xFF, 0x00, 255)
        );
        assert_eq!(
            majority_color(&decoded, Region::new(0, 0, 1, 1).unwrap()).unwrap(),
            Rgb::new(0x22, 0x33, 0x44)
        );
        assert!(matches(
            Rgb::new(0x22, 0x33, 0x44),
            sample_rgb(&decoded, 0, 0).unwrap(),
            0
        ));
    }
}
