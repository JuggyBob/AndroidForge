use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::toolchain::{Toolchain, which};
use androidforge_process::{CommandSpec, Runner};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvdInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u32>,
}

pub async fn list_avds(toolchain: &Toolchain, trace: bool) -> Result<Vec<AvdInfo>> {
    let configs = read_avd_configs(toolchain);
    if let Some(emulator) = &toolchain.emulator {
        let runner = Runner::new(trace);
        let spec = CommandSpec::new(emulator.to_string_lossy().into_owned())
            .args(["-list-avds"])
            .timeout(Duration::from_secs(20))
            .trace(trace)
            .hide_window(true);
        if let Ok(output) = runner.run(spec).await
            && output.success
        {
            let names = output
                .stdout_text()
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with("INFO"))
                .map(str::to_string)
                .collect::<Vec<_>>();
            if !names.is_empty() {
                return Ok(names
                    .into_iter()
                    .map(|name| {
                        configs.get(&name).cloned().unwrap_or_else(|| AvdInfo {
                            name: name.clone(),
                            path: toolchain.avd_home.as_ref().map(|home| {
                                home.join(format!("{name}.avd"))
                                    .to_string_lossy()
                                    .into_owned()
                            }),
                            target: None,
                            abi: None,
                            device: None,
                            tag: None,
                            ram_mb: None,
                        })
                    })
                    .collect());
            }
        }
    }
    if configs.is_empty() {
        return Err(
            ForgeError::new(ErrorCode::EmulatorNotFound, "no Android AVDs were found")
                .with_suggestion("Create an AVD with Android Studio or `avdmanager`.")
                .with_suggestion(
                    "Run `androidforge doctor` to inspect emulator and AVD discovery.",
                ),
        );
    }
    let mut avds = configs.into_values().collect::<Vec<_>>();
    avds.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(avds)
}

fn read_avd_configs(toolchain: &Toolchain) -> BTreeMap<String, AvdInfo> {
    let Some(home) = &toolchain.avd_home else {
        return BTreeMap::new();
    };
    let mut avds = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(home) else {
        return avds;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("ini") {
            continue;
        }
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let ini = std::fs::read_to_string(&path).unwrap_or_default();
        let avd_path = parse_ini(&ini)
            .get("path")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(format!("{name}.avd")));
        let config_path = avd_path.join("config.ini");
        let config = std::fs::read_to_string(&config_path)
            .ok()
            .map(|text| parse_ini(&text))
            .unwrap_or_default();
        let target = parse_ini(&ini).get("target").cloned();
        avds.insert(
            name.clone(),
            AvdInfo {
                name,
                path: Some(avd_path.to_string_lossy().into_owned()),
                target,
                abi: config.get("abi.type").cloned(),
                device: config.get("hw.device.name").cloned(),
                tag: config.get("tag.id").cloned(),
                ram_mb: config
                    .get("hw.ramSize")
                    .and_then(|value| value.parse::<u32>().ok()),
            },
        );
    }
    avds
}

fn parse_ini(text: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            values.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    values
}

/// An installed SDK system image, discovered from the SDK directory layout.
/// No `sdkmanager --list` invocation is needed (and none is attempted), so this
/// stays offline-safe and bounded by the size of `system-images/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemImageInfo {
    /// Full SDK package id, e.g. `system-images;android-35;google_apis;x86_64`.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_level: Option<u32>,
    pub tag: String,
    pub abi: String,
    /// Path relative to the SDK root, with `/` separators.
    pub path: String,
    pub source: String,
}

/// Parse one `system-images/<platform>/<tag>/<abi>` entry.
///
/// Returns `None` for directories that are not recognisable platform/tag/ABI
/// triples instead of guessing values. All components must use safe
/// characters; no path separators or `..` segments are ever accepted.
pub fn parse_system_image_parts(platform: &str, tag: &str, abi: &str) -> Option<SystemImageInfo> {
    if !is_safe_token(platform) || !is_safe_token(tag) || !is_safe_token(abi) {
        return None;
    }
    let api_level = platform
        .strip_prefix("android-")
        .and_then(|version| version.split(['_', '-']).next())
        .and_then(|version| version.parse::<u32>().ok())?;
    Some(SystemImageInfo {
        id: format!("system-images;{platform};{tag};{abi}"),
        api_level: Some(api_level),
        tag: tag.to_string(),
        abi: abi.to_string(),
        path: format!("system-images/{platform}/{tag}/{abi}"),
        source: "sdk_layout".to_string(),
    })
}

/// List installed system images from `<sdk>/system-images`.
///
/// Missing or unreadable directories produce an empty list; an absent SDK root
/// is an error rather than a fabricated empty inventory.
pub fn list_system_images(toolchain: &Toolchain) -> Result<Vec<SystemImageInfo>> {
    let sdk = toolchain.sdk_root.as_deref().ok_or_else(|| {
        ForgeError::new(ErrorCode::SdkNotFound, "Android SDK was not found")
            .with_suggestion("Set ANDROID_HOME or ANDROID_SDK_ROOT.")
            .with_suggestion("Run `androidforge doctor` to inspect SDK discovery.")
    })?;
    let root = sdk.join("system-images");
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut images = Vec::new();
    let mut seen = BTreeSet::new();
    for platform in sorted_dir_names(&root) {
        let platform_dir = root.join(&platform);
        for tag in sorted_dir_names(&platform_dir) {
            let tag_dir = platform_dir.join(&tag);
            for abi in sorted_dir_names(&tag_dir) {
                let Some(image) = parse_system_image_parts(&platform, &tag, &abi) else {
                    continue;
                };
                if seen.insert(image.id.clone()) {
                    images.push(image);
                }
            }
        }
    }
    images.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(images)
}

fn sorted_dir_names(parent: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(parent)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Explicit request for `avdmanager create avd`. AndroidForge never fills in
/// a name, image or device from defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAvdRequest {
    pub name: String,
    /// Full SDK system-image package id, e.g. `system-images;android-35;google_apis;x86_64`.
    pub image: String,
    /// Device profile id accepted by `avdmanager -d`, e.g. `pixel_7`.
    pub device: String,
}

impl CreateAvdRequest {
    pub fn validate(&self) -> Result<()> {
        validate_avd_name(&self.name)?;
        validate_image_id(&self.image)?;
        validate_device_id(&self.device)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CreateAvdOptions {
    /// Build the exact argv and return without running any process.
    pub dry_run: bool,
    /// Explicit opt-in. Without it, `create_avd` never executes `avdmanager`.
    pub yes: bool,
    pub trace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAvdResult {
    pub name: String,
    pub image: String,
    pub device: String,
    pub executable: String,
    /// Arguments passed after the executable; never a shell command string.
    pub argv: Vec<String>,
    pub dry_run: bool,
    pub executed: bool,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

pub fn validate_avd_name(name: &str) -> Result<()> {
    validate_identifier(
        name,
        64,
        "AVD names may contain only letters, numbers, '.', '_' and '-' and must not start with '-' or '.'",
    )
}

pub fn validate_image_id(image: &str) -> Result<()> {
    if image.len() > 200 || !image.contains(';') {
        return Err(ForgeError::invalid_argument(
            "system images must be explicit SDK package ids such as system-images;android-35;google_apis;x86_64",
        ));
    }
    for part in image.split(';') {
        validate_identifier(
            part,
            120,
            "system image id segments may contain only letters, numbers, '.', '_' and '-'",
        )?;
    }
    Ok(())
}

pub fn validate_device_id(device: &str) -> Result<()> {
    if device.is_empty()
        || device.len() > 120
        || !device
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '.' | '_' | '-'))
        || device.starts_with('-')
        || device.starts_with('.')
        || device.contains("..")
        || device.trim() != device
    {
        return Err(ForgeError::invalid_argument(
            "device ids may contain only letters, numbers, spaces, '.', '_' and '-' and must not start with '-' or '.'",
        ));
    }
    Ok(())
}

fn validate_identifier(value: &str, max_len: usize, message: &str) -> Result<()> {
    let invalid = value.is_empty()
        || value.len() > max_len
        || value.contains("..")
        || value.starts_with('-')
        || value.starts_with('.')
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
    if invalid {
        return Err(ForgeError::invalid_argument(message).with_suggestion(
            "Use a short ASCII name without path separators, quotes or shell metacharacters.",
        ));
    }
    Ok(())
}

fn is_safe_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && !value.contains("..")
        && !value.starts_with('-')
        && !value.starts_with('.')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

/// Build the exact argument vector for `avdmanager create avd`.
/// This is a pure function so callers can inspect a plan before opting in.
pub fn create_avd_args(request: &CreateAvdRequest) -> Result<Vec<String>> {
    request.validate()?;
    Ok(vec![
        "create".to_string(),
        "avd".to_string(),
        "-n".to_string(),
        request.name.clone(),
        "-k".to_string(),
        request.image.clone(),
        "-d".to_string(),
        request.device.clone(),
    ])
}

/// Locate `avdmanager` from the SDK layout or PATH. A dry run still reports a
/// conventional executable name when the tool is absent, but never executes.
pub fn find_avdmanager(toolchain: &Toolchain) -> Option<PathBuf> {
    let executable = if cfg!(windows) {
        "avdmanager.bat"
    } else {
        "avdmanager"
    };
    let mut candidates = Vec::new();
    if let Some(root) = &toolchain.sdk_root {
        let tools = root.join("cmdline-tools");
        if let Ok(entries) = std::fs::read_dir(&tools) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    candidates.push(entry.path().join("bin").join(executable));
                }
            }
        }
        candidates.push(root.join("tools").join("bin").join(executable));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .or_else(|| which("avdmanager"))
}

/// Core wrapper around `avdmanager create avd`.
///
/// Safety invariants:
/// * names are validated before any work;
/// * `--dry-run` only returns a plan and cannot execute a process;
/// * without `yes: true` the call fails before looking for or running a tool;
/// * an existing AVD with the same name is never overwritten (`--force` is not
///   used);
/// * all arguments are passed as a vector, never through a shell.
pub async fn create_avd(
    toolchain: &Toolchain,
    request: &CreateAvdRequest,
    options: &CreateAvdOptions,
) -> Result<CreateAvdResult> {
    let argv = create_avd_args(request)?;
    let executable_path = find_avdmanager(toolchain);
    let executable = executable_path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "avdmanager".to_string());
    let mut result = CreateAvdResult {
        name: request.name.clone(),
        image: request.image.clone(),
        device: request.device.clone(),
        executable,
        argv,
        dry_run: options.dry_run,
        executed: false,
        success: false,
        stdout: None,
        stderr: None,
        notes: Vec::new(),
    };
    if options.dry_run {
        result
            .notes
            .push("dry run: no command was executed and no AVD was created".to_string());
        if executable_path.is_none() {
            result.notes.push(
                "avdmanager was not found; execution also requires the SDK command-line tools"
                    .to_string(),
            );
        }
        return Ok(result);
    }
    if !options.yes {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "creating an AVD requires explicit opt-in: pass --yes (or preview with --dry-run)",
        )
        .with_suggestion("Review the exact argv with --dry-run before passing --yes."));
    }
    let program = executable_path.ok_or_else(|| {
        ForgeError::new(ErrorCode::ToolNotFound, "avdmanager was not found")
            .with_suggestion("Install the Android SDK command-line tools.")
            .with_suggestion("Run `androidforge doctor` to inspect tool discovery.")
    })?;
    if let Some(home) = &toolchain.avd_home
        && home.join(format!("{}.ini", request.name)).is_file()
    {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            format!(
                "AVD '{}' already exists; AndroidForge never overwrites an AVD",
                request.name
            ),
        )
        .with_suggestion("Choose a different name or remove the existing AVD explicitly."));
    }
    let spec = CommandSpec::new(program.to_string_lossy().into_owned())
        .args(result.argv.clone())
        // `avdmanager` can prompt to create custom hardware; decline so this
        // bounded foreground command cannot wait forever on stdin.
        .stdin("no\n")
        .timeout(Duration::from_secs(180))
        .trace(options.trace)
        .hide_window(true);
    let output = Runner::new(options.trace).run(spec).await?;
    result.executed = true;
    result.success = output.success;
    result.stdout = Some(output.stdout_text());
    result.stderr = Some(output.stderr_text());
    if !output.success {
        result.notes.push(format!(
            "avdmanager exited with code {}; no AVD creation was confirmed",
            output
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ini() {
        let values =
            parse_ini("avd.ini.encoding=UTF-8\npath=C:\\Users\\me\\.android\\avd\\Pixel.avd\n");
        assert_eq!(values.get("avd.ini.encoding").unwrap(), "UTF-8");
        assert!(values.get("path").unwrap().contains("Pixel.avd"));
    }

    #[test]
    fn parses_system_image_parts() {
        let image = parse_system_image_parts("android-35", "google_apis", "x86_64").unwrap();
        assert_eq!(image.id, "system-images;android-35;google_apis;x86_64");
        assert_eq!(image.api_level, Some(35));
        assert_eq!(image.tag, "google_apis");
        assert_eq!(image.abi, "x86_64");
        assert_eq!(image.path, "system-images/android-35/google_apis/x86_64");
        assert_eq!(image.source, "sdk_layout");
    }

    #[test]
    fn rejects_unsafe_or_unknown_system_image_parts() {
        assert!(parse_system_image_parts("preview", "google_apis", "x86_64").is_none());
        assert!(parse_system_image_parts("android-35", "../evil", "x86_64").is_none());
        assert!(parse_system_image_parts("android-35", "google_apis", "x86;rm").is_none());
        assert!(parse_system_image_parts("android-35", "google_apis", "").is_none());
    }

    #[test]
    fn lists_images_from_sdk_layout_without_sdkmanager() {
        let sdk = tempfile::tempdir().unwrap();
        let base = sdk.path().join("system-images");
        std::fs::create_dir_all(base.join("android-35/google_apis/x86_64")).unwrap();
        std::fs::create_dir_all(base.join("android-35/default/arm64-v8a")).unwrap();
        std::fs::create_dir_all(base.join("not-a-platform/unknown/x86_64")).unwrap();
        std::fs::write(base.join("android-35/package.xml"), "ignored").unwrap();
        let toolchain = Toolchain {
            sdk_root: Some(sdk.path().to_path_buf()),
            ..Toolchain::default()
        };
        let images = list_system_images(&toolchain).unwrap();
        assert_eq!(
            images
                .iter()
                .map(|image| image.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "system-images;android-35;default;arm64-v8a",
                "system-images;android-35;google_apis;x86_64",
            ]
        );
    }

    #[test]
    fn missing_sdk_root_is_an_error_not_an_empty_inventory() {
        assert!(list_system_images(&Toolchain::default()).is_err());
        let sdk = tempfile::tempdir().unwrap();
        let toolchain = Toolchain {
            sdk_root: Some(sdk.path().to_path_buf()),
            ..Toolchain::default()
        };
        assert!(list_system_images(&toolchain).unwrap().is_empty());
    }

    #[test]
    fn validates_create_names_and_identifiers() {
        assert!(validate_avd_name("Pixel_7-api35").is_ok());
        assert!(validate_avd_name("123").is_ok());
        for bad in [
            "",
            "../evil",
            "-name",
            ".hidden",
            "with space",
            "a/b",
            "a;b",
            &"x".repeat(65),
        ] {
            assert!(
                validate_avd_name(bad).is_err(),
                "accepted bad AVD name {bad:?}"
            );
        }
        assert!(validate_image_id("system-images;android-35;google_apis;x86_64").is_ok());
        assert!(validate_image_id("android-35;google_apis;x86_64").is_ok());
        for bad in [
            "",
            "x86_64",
            "system-images;;google_apis;x86_64",
            "../evil;x;y",
            "system-images;android-35;google_apis;C:\\tmp",
        ] {
            assert!(
                validate_image_id(bad).is_err(),
                "accepted bad image {bad:?}"
            );
        }
        assert!(validate_device_id("pixel_7").is_ok());
        assert!(validate_device_id("Nexus 5").is_ok());
        for bad in ["", "../pixel", "-pixel", "pixel\\7", "pixel|7", "a;b"] {
            assert!(
                validate_device_id(bad).is_err(),
                "accepted bad device {bad:?}"
            );
        }
    }

    #[test]
    fn build_create_args_are_explicit_and_safe() {
        let request = CreateAvdRequest {
            name: "Pixel_7-api35".to_string(),
            image: "system-images;android-35;google_apis;x86_64".to_string(),
            device: "pixel_7".to_string(),
        };
        assert_eq!(
            create_avd_args(&request).unwrap(),
            vec![
                "create",
                "avd",
                "-n",
                "Pixel_7-api35",
                "-k",
                "system-images;android-35;google_apis;x86_64",
                "-d",
                "pixel_7",
            ]
        );
        let invalid = CreateAvdRequest {
            name: "../evil".to_string(),
            ..request
        };
        assert!(create_avd_args(&invalid).is_err());
    }

    #[tokio::test]
    async fn dry_run_returns_a_plan_and_executes_nothing() {
        let toolchain = Toolchain::default();
        let request = CreateAvdRequest {
            name: "Pixel_7".to_string(),
            image: "system-images;android-35;google_apis;x86_64".to_string(),
            device: "pixel_7".to_string(),
        };
        let result = create_avd(
            &toolchain,
            &request,
            &CreateAvdOptions {
                dry_run: true,
                yes: true,
                trace: false,
            },
        )
        .await
        .unwrap();
        assert!(result.dry_run);
        assert!(!result.executed);
        assert!(!result.success);
        assert!(result.argv.iter().any(|arg| arg == "create"));
        assert!(result.notes[0].contains("no command was executed"));
    }

    #[tokio::test]
    async fn creation_requires_explicit_opt_in() {
        let toolchain = Toolchain::default();
        let request = CreateAvdRequest {
            name: "Pixel_7".to_string(),
            image: "system-images;android-35;google_apis;x86_64".to_string(),
            device: "pixel_7".to_string(),
        };
        let error = create_avd(&toolchain, &request, &CreateAvdOptions::default())
            .await
            .unwrap_err();
        assert!(error.message().contains("requires explicit opt-in"));
    }

    #[tokio::test]
    async fn invalid_create_request_is_rejected_before_dry_run() {
        let request = CreateAvdRequest {
            name: "../evil".to_string(),
            image: "system-images;android-35;google_apis;x86_64".to_string(),
            device: "pixel_7".to_string(),
        };
        assert!(
            create_avd(
                &Toolchain::default(),
                &request,
                &CreateAvdOptions {
                    dry_run: true,
                    yes: false,
                    trace: false,
                }
            )
            .await
            .is_err()
        );
    }
}
