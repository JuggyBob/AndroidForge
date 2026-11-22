//! PNG screenshot comparison for visual regression checks.

use std::path::Path;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct VisualRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct VisualBoundingBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct VisualDiff {
    pub baseline: String,
    pub current: String,
    pub width: u32,
    pub height: u32,
    pub region: VisualRegion,
    pub compared_pixels: u64,
    pub changed_pixels: u64,
    pub changed_ratio: f64,
    pub max_channel_delta: u8,
    pub mean_channel_delta: f64,
    pub diff_threshold: u8,
    pub max_diff_ratio: f64,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_bounds: Option<VisualBoundingBox>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_image: Option<String>,
}

struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

pub fn parse_region(value: &str) -> Result<VisualRegion> {
    let parts = value
        .split(',')
        .map(|part| part.trim().parse::<u32>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| {
            ForgeError::invalid_argument("--region must be four device-pixel integers: x1,y1,x2,y2")
        })?;
    if parts.len() != 4 {
        return Err(ForgeError::invalid_argument(
            "--region must be four device-pixel integers: x1,y1,x2,y2",
        ));
    }
    let (x1, y1, x2, y2) = (parts[0], parts[1], parts[2], parts[3]);
    if x2 <= x1 || y2 <= y1 {
        return Err(ForgeError::invalid_argument(
            "--region needs x2>x1 and y2>y1",
        ));
    }
    Ok(VisualRegion {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    })
}

pub fn compare_png(
    baseline_path: &Path,
    current_path: &Path,
    region: Option<VisualRegion>,
    diff_threshold: u8,
    max_diff_ratio: f64,
    diff_output: Option<&Path>,
) -> Result<VisualDiff> {
    let baseline = decode(baseline_path)?;
    let current = decode(current_path)?;
    if baseline.width != current.width || baseline.height != current.height {
        return Err(ForgeError::new(
            ErrorCode::UiAssertionFailed,
            format!(
                "screenshot sizes differ: baseline {}x{}, current {}x{}",
                baseline.width, baseline.height, current.width, current.height
            ),
        )
        .with_context(
            "baseline",
            serde_json::json!({"path": baseline_path, "width": baseline.width, "height": baseline.height}),
        )
        .with_context(
            "current",
            serde_json::json!({"path": current_path, "width": current.width, "height": current.height}),
        )
        .with_suggestion(
            "Recapture the baseline from the same device resolution and density before comparing.",
        ));
    }
    let region = region.unwrap_or(VisualRegion {
        x: 0,
        y: 0,
        width: baseline.width,
        height: baseline.height,
    });
    if region.x + region.width > baseline.width || region.y + region.height > baseline.height {
        return Err(ForgeError::invalid_argument(format!(
            "region {}x{}+{},{} exceeds the {}x{} screenshot",
            region.width, region.height, region.x, region.y, baseline.width, baseline.height
        ))
        .with_suggestion("Use --region with device-pixel bounds inside the captured image."));
    }
    let mut changed = 0u64;
    let mut max_channel_delta = 0u8;
    let mut total_delta = 0u64;
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut diff_pixels = Vec::with_capacity((region.width * region.height * 3) as usize);
    for y in region.y..region.y + region.height {
        for x in region.x..region.x + region.width {
            let index = ((y * baseline.width + x) * 4) as usize;
            let base = &baseline.rgba[index..index + 4];
            let now = &current.rgba[index..index + 4];
            let mut pixel_changed = false;
            for channel in 0..3 {
                let delta = base[channel].abs_diff(now[channel]);
                total_delta += u64::from(delta);
                max_channel_delta = max_channel_delta.max(delta);
                if delta > diff_threshold {
                    pixel_changed = true;
                }
            }
            if pixel_changed {
                changed += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                diff_pixels.extend_from_slice(&[255, 0, 0]);
            } else {
                let gray = (u16::from(now[0]) + u16::from(now[1]) + u16::from(now[2])) / 3;
                let dim = (gray / 3) as u8;
                diff_pixels.extend_from_slice(&[dim, dim, dim]);
            }
        }
    }
    let compared = u64::from(region.width) * u64::from(region.height);
    let ratio = if compared == 0 {
        0.0
    } else {
        changed as f64 / compared as f64
    };
    let mean_delta = if compared == 0 {
        0.0
    } else {
        total_delta as f64 / (compared * 3) as f64
    };
    let diff_image = if let Some(path) = diff_output {
        write_diff_image(path, region.width, region.height, &diff_pixels)?;
        Some(path.to_string_lossy().into_owned())
    } else {
        None
    };
    Ok(VisualDiff {
        baseline: baseline_path.to_string_lossy().into_owned(),
        current: current_path.to_string_lossy().into_owned(),
        width: baseline.width,
        height: baseline.height,
        region: region.clone(),
        compared_pixels: compared,
        changed_pixels: changed,
        changed_ratio: ratio,
        max_channel_delta,
        mean_channel_delta: mean_delta,
        diff_threshold,
        max_diff_ratio,
        passed: ratio <= max_diff_ratio,
        changed_bounds: (changed > 0).then(|| VisualBoundingBox {
            x: min_x,
            y: min_y,
            width: max_x - min_x + 1,
            height: max_y - min_y + 1,
        }),
        diff_image,
    })
}

fn decode(path: &Path) -> Result<Image> {
    let file = std::fs::File::open(path).map_err(|error| {
        ForgeError::new(
            ErrorCode::FileNotFound,
            format!("could not open screenshot {}: {error}", path.display()),
        )
    })?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|error| {
        ForgeError::new(
            ErrorCode::UiAssertionFailed,
            format!("could not decode PNG {}: {error}", path.display()),
        )
    })?;
    let buffer_size = reader.output_buffer_size().ok_or_else(|| {
        ForgeError::new(
            ErrorCode::UiAssertionFailed,
            format!("PNG {} declares an unsupported buffer size", path.display()),
        )
    })?;
    let mut buffer = vec![0u8; buffer_size];
    let info = reader.next_frame(&mut buffer).map_err(|error| {
        ForgeError::new(
            ErrorCode::UiAssertionFailed,
            format!("could not read PNG {}: {error}", path.display()),
        )
    })?;
    let width = info.width;
    let height = info.height;
    let samples = info.color_type.samples();
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
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
                return Err(ForgeError::new(
                    ErrorCode::UiAssertionFailed,
                    format!(
                        "PNG {} stayed palette-indexed after decoding; re-export it as RGB/RGBA",
                        path.display()
                    ),
                ));
            }
        }
    }
    if rgba.len() != (width * height * 4) as usize {
        return Err(ForgeError::new(
            ErrorCode::UiAssertionFailed,
            format!(
                "PNG {} decoded to an unexpected buffer size",
                path.display()
            ),
        ));
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

fn write_diff_image(path: &Path, width: u32, height: u32, rgb: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        androidforge_core::util::ensure_dir(parent)?;
    }
    let file = std::fs::File::create(path).map_err(|error| {
        ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!("could not create diff image {}: {error}", path.display()),
        )
    })?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| {
        ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!("could not write diff image {}: {error}", path.display()),
        )
    })?;
    writer.write_image_data(rgb).map_err(|error| {
        ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!("could not encode diff image {}: {error}", path.display()),
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(path: &Path, width: u32, height: u32, pixels: &[[u8; 3]]) {
        let file = std::fs::File::create(path).unwrap();
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
    fn identical_images_pass_with_zero_ratio() {
        let temp = tempfile::tempdir().unwrap();
        let left = temp.path().join("left.png");
        let right = temp.path().join("right.png");
        let pixels = [[10, 20, 30], [40, 50, 60], [70, 80, 90], [1, 2, 3]];
        write_png(&left, 2, 2, &pixels);
        write_png(&right, 2, 2, &pixels);
        let diff = compare_png(&left, &right, None, 0, 0.0, None).unwrap();
        assert!(diff.passed);
        assert_eq!(diff.changed_pixels, 0);
        assert!(diff.changed_bounds.is_none());
    }

    #[test]
    fn region_changes_are_reported_and_can_fail() {
        let temp = tempfile::tempdir().unwrap();
        let left = temp.path().join("left.png");
        let right = temp.path().join("right.png");
        let mut pixels = [[0u8; 3]; 4];
        write_png(&left, 2, 2, &pixels);
        pixels[3] = [255, 255, 255];
        write_png(&right, 2, 2, &pixels);
        let region = parse_region("1,1,2,2").unwrap();
        let diff = compare_png(&left, &right, Some(region), 0, 0.0, None).unwrap();
        assert!(!diff.passed);
        assert_eq!(diff.changed_pixels, 1);
        assert_eq!(diff.compared_pixels, 1);
        assert_eq!(diff.changed_ratio, 1.0);
        assert_eq!(diff.changed_bounds.unwrap().x, 1);
    }

    #[test]
    fn threshold_ignores_small_deltas() {
        let temp = tempfile::tempdir().unwrap();
        let left = temp.path().join("left.png");
        let right = temp.path().join("right.png");
        write_png(&left, 1, 1, &[[10, 10, 10]]);
        write_png(&right, 1, 1, &[[14, 14, 14]]);
        let diff = compare_png(&left, &right, None, 5, 0.0, None).unwrap();
        assert!(diff.passed);
        assert_eq!(diff.changed_pixels, 0);
        assert_eq!(diff.max_channel_delta, 4);
    }

    #[test]
    fn visual_diff_payload_matches_schema() {
        let temp = tempfile::tempdir().unwrap();
        let left = temp.path().join("left.png");
        let right = temp.path().join("right.png");
        let mut pixels = [[0u8; 3]; 4];
        write_png(&left, 2, 2, &pixels);
        pixels[0] = [255, 255, 255];
        write_png(&right, 2, 2, &pixels);
        let diff = compare_png(
            &left,
            &right,
            Some(VisualRegion {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            }),
            0,
            0.0,
            Some(&temp.path().join("diff.png")),
        )
        .unwrap();
        let value = serde_json::to_value(&diff).unwrap();
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../../schema/visual-diff.schema.json"))
                .unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        if let Err(error) = validator.validate(&value) {
            panic!("visual-diff schema rejected payload: {error}\npayload: {value:#}");
        }
    }

    #[test]
    fn region_must_fit_the_image() {
        assert!(parse_region("0,0,0,10").is_err());
        let temp = tempfile::tempdir().unwrap();
        let left = temp.path().join("left.png");
        write_png(&left, 1, 1, &[[0, 0, 0]]);
        let error = compare_png(
            &left,
            &left,
            Some(VisualRegion {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            }),
            0,
            0.0,
            None,
        )
        .unwrap_err();
        assert!(error.message().contains("exceeds"));
    }
}
