//! Optional OCR fallback for UI selectors.
//!
//! UIAutomator is always the source of truth. OCR is attempted only when the
//! caller explicitly requests `--ocr-fallback` and the hierarchy has no match.
//! The external engine is local; no image leaves the machine.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::util::short_id;
use androidforge_diagnostics::capture_frame;
use androidforge_ui::{Rect, Selector};

use crate::context::CliContext;

#[derive(Debug, Clone)]
pub struct OcrMatch {
    pub text: String,
    pub bounds: Rect,
    pub confidence: f32,
}

pub async fn find(ctx: &CliContext, serial: &str, selector: &Selector) -> Result<Vec<OcrMatch>> {
    unsupported_conditions(selector)?;
    let adb = ctx.adb()?;
    let path = std::env::temp_dir().join(format!("androidforge-ocr-{}.png", short_id()));
    let frame = capture_frame(
        &adb,
        serial,
        &path,
        ctx.parse_timeout(None, ctx.config.file.timeouts.screenshot_secs)?,
    )
    .await?;
    let result = match run_engine(&path) {
        Ok(result) => result,
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
    };
    let _ = std::fs::remove_file(&path);
    let lines = parse_tsv(&result)?;
    let region = selector.region.as_deref().map(parse_region).transpose()?;
    let regex = selector
        .text_regex
        .as_deref()
        .map(regex::Regex::new)
        .transpose()
        .map_err(|error| ForgeError::invalid_argument(format!("invalid --text-regex: {error}")))?;
    let mut matches = lines
        .into_iter()
        .filter(|line| line.confidence >= 0.0)
        .filter(|line| {
            selector
                .text
                .as_deref()
                .is_none_or(|text| line.text == text)
                && selector
                    .text_contains
                    .as_deref()
                    .is_none_or(|text| line.text.contains(text))
                && regex
                    .as_ref()
                    .is_none_or(|regex| regex.is_match(&line.text))
                && region.is_none_or(|bounds| {
                    bounds.contains(line.bounds.center().0, line.bounds.center().1)
                })
        })
        .collect::<Vec<_>>();
    if let Some(index) = selector.index {
        if index >= matches.len() {
            return Err(ForgeError::new(
                ErrorCode::SelectorNotFound,
                format!(
                    "OCR matched {} line(s), but index {} was requested (frame {}x{})",
                    matches.len(),
                    index,
                    frame.width,
                    frame.height
                ),
            )
            .with_suggestion("Use --index 0 through the last listed OCR match."));
        }
        matches = vec![matches.remove(index)];
    }
    Ok(matches)
}

fn unsupported_conditions(selector: &Selector) -> Result<()> {
    if selector.resource_id.is_some()
        || selector.class.is_some()
        || selector.role.is_some()
        || selector.within.is_some()
        || selector.above.is_some()
        || selector.below.is_some()
        || selector.left_of.is_some()
        || selector.right_of.is_some()
    {
        return Err(ForgeError::new(
            ErrorCode::UnsupportedOperation,
            "OCR fallback supports text, text-regex, region and index; it cannot prove resource-id, class, role or relative-node conditions",
        )
        .with_suggestion("Use a UIAutomator selector for identity/relationship conditions.")
        .with_suggestion("Use --text, --contains or --text-regex with --ocr-fallback."));
    }
    Ok(())
}

fn parse_region(value: &str) -> Result<Rect> {
    let numbers = value
        .split(',')
        .map(str::trim)
        .map(str::parse::<i32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| {
            ForgeError::invalid_argument(format!(
                "--region {value:?} must be four integers: x1,y1,x2,y2 ({error})"
            ))
        })?;
    if numbers.len() != 4 || numbers[2] <= numbers[0] || numbers[3] <= numbers[1] {
        return Err(ForgeError::invalid_argument(format!(
            "--region {value:?} must be x1,y1,x2,y2 with x2>x1 and y2>y1"
        )));
    }
    Ok(Rect {
        left: numbers[0],
        top: numbers[1],
        right: numbers[2],
        bottom: numbers[3],
    })
}

fn engine() -> String {
    std::env::var("ANDROIDFORGE_OCR_COMMAND").unwrap_or_else(|_| "tesseract".to_string())
}

fn run_engine(image: &PathBuf) -> Result<String> {
    let engine = engine();
    let mut command = if engine.to_ascii_lowercase().ends_with(".cmd")
        || engine.to_ascii_lowercase().ends_with(".bat")
    {
        let mut command = Command::new("cmd.exe");
        command.arg("/c").arg(&engine);
        command
    } else {
        Command::new(&engine)
    };
    let output = command
        .arg(image)
        .arg("stdout")
        .arg("tsv")
        .output()
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::ToolNotFound,
                format!("OCR fallback could not run {engine:?}: {error}"),
            )
            .with_suggestion(
                "Install Tesseract and put tesseract on PATH, or set ANDROIDFORGE_OCR_COMMAND.",
            )
            .with_suggestion("Omit --ocr-fallback to use UIAutomator only.")
        })?;
    if !output.status.success() {
        return Err(ForgeError::new(
            ErrorCode::CommandFailed,
            format!(
                "OCR command {engine:?} failed with exit {}: {}",
                output.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| {
        ForgeError::new(
            ErrorCode::CommandFailed,
            format!("OCR command produced non-UTF-8 output: {error}"),
        )
    })
}

type OcrLineKey = (u32, u32, u32, u32);
type OcrLineAccumulator = (Vec<String>, Rect, Vec<f32>);

fn parse_tsv(value: &str) -> Result<Vec<OcrMatch>> {
    let mut lines: BTreeMap<OcrLineKey, OcrLineAccumulator> = BTreeMap::new();
    for line in value.lines().skip(1) {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() < 12 {
            continue;
        }
        let Ok(level) = fields[0].parse::<u32>() else {
            continue;
        };
        if level != 5 {
            continue;
        }
        let Ok(page) = fields[1].parse::<u32>() else {
            continue;
        };
        let Ok(block) = fields[2].parse::<u32>() else {
            continue;
        };
        let Ok(par) = fields[3].parse::<u32>() else {
            continue;
        };
        let Ok(line_no) = fields[4].parse::<u32>() else {
            continue;
        };
        let Ok(left) = fields[6].parse::<i32>() else {
            continue;
        };
        let Ok(top) = fields[7].parse::<i32>() else {
            continue;
        };
        let Ok(width) = fields[8].parse::<i32>() else {
            continue;
        };
        let Ok(height) = fields[9].parse::<i32>() else {
            continue;
        };
        let Ok(confidence) = fields[10].parse::<f32>() else {
            continue;
        };
        let text = fields[11].trim();
        if text.is_empty() || confidence < 0.0 {
            continue;
        }
        let rect = Rect {
            left,
            top,
            right: left + width,
            bottom: top + height,
        };
        let entry = lines
            .entry((page, block, par, line_no))
            .or_insert_with(|| (Vec::new(), rect, Vec::new()));
        entry.0.push(text.to_string());
        entry.1.left = entry.1.left.min(rect.left);
        entry.1.top = entry.1.top.min(rect.top);
        entry.1.right = entry.1.right.max(rect.right);
        entry.1.bottom = entry.1.bottom.max(rect.bottom);
        entry.2.push(confidence);
    }
    Ok(lines
        .into_values()
        .map(|(words, bounds, confidences)| OcrMatch {
            text: words.join(" "),
            bounds,
            confidence: confidences.iter().sum::<f32>() / confidences.len().max(1) as f32,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tesseract_words_into_lines() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   5\t1\t1\t1\t1\t1\t10\t20\t40\t20\t96.0\tSave\n\
                   5\t1\t1\t1\t1\t2\t55\t20\t50\t20\t92.5\tchanges\n\
                   5\t1\t1\t1\t2\t1\t10\t60\t30\t20\t88.0\tCancel\n";
        let lines = parse_tsv(tsv).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Save changes");
        assert_eq!(lines[0].bounds.left, 10);
        assert_eq!(lines[0].bounds.right, 105);
        assert_eq!(lines[1].text, "Cancel");
    }

    #[test]
    fn rejects_tsv_without_a_words_header() {
        assert!(parse_tsv("").unwrap().is_empty());
    }
}
