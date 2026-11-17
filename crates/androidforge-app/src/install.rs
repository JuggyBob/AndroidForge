use std::path::PathBuf;
use std::time::Duration;

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::BuildIdentity;
use androidforge_core::util::{normalize_fingerprint, sha256_file};
use serde::{Deserialize, Serialize};

use crate::apk::{ApkInfo, ApkInspector};
use crate::diff::{ApkDiff, Equivalence, compare, inspect_installed};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_apk_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,
    #[serde(default)]
    pub debuggable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installer: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureDiagnosis {
    pub incoming_package: String,
    pub incoming_version_name: Option<String>,
    pub incoming_version_code: Option<i64>,
    pub incoming_cert_sha256: Option<String>,
    pub installed_version_name: Option<String>,
    pub installed_version_code: Option<i64>,
    pub installed_cert_sha256: Option<String>,
    pub install_output: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub apk_path: PathBuf,
    pub fresh: bool,
    pub confirmed: bool,
    pub allow_downgrade: bool,
    pub verify_installed: bool,
    /// Skip the install when the candidate is provably byte-equivalent to the
    /// installed base APK and the split shape is known.
    pub only_if_changed: bool,
    pub expected_package: Option<String>,
    /// Packages the adapter accepts for this install. Usually the release and
    /// debug application ids, which legitimately differ.
    pub allowed_packages: Vec<String>,
    /// Declared certificate fingerprints. Empty means "record but do not
    /// require a specific signer".
    pub expected_cert_sha256: Vec<String>,
    pub timeout: Duration,
}

impl InstallOptions {
    pub fn new(apk_path: impl Into<PathBuf>) -> Self {
        Self {
            apk_path: apk_path.into(),
            fresh: false,
            confirmed: false,
            allow_downgrade: false,
            verify_installed: true,
            only_if_changed: false,
            expected_package: None,
            allowed_packages: Vec::new(),
            expected_cert_sha256: Vec::new(),
            timeout: Duration::from_secs(300),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub apk: ApkInfo,
    pub build: BuildIdentity,
    pub installed: InstalledPackage,
    #[serde(default)]
    pub skipped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<ApkDiff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_diagnosis: Option<SignatureDiagnosis>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub async fn install(
    adb: &AdbClient,
    inspector: &ApkInspector,
    serial: &str,
    options: &InstallOptions,
) -> Result<InstallResult> {
    if options.fresh && options.only_if_changed {
        return Err(ForgeError::invalid_argument(
            "--fresh uninstalls the application; it cannot be combined with --only-if-changed",
        )
        .with_suggestion(
            "Use --only-if-changed alone to preserve data and skip equivalent installs.",
        ));
    }
    if options.fresh && !options.confirmed {
        return Err(ForgeError::new(
            ErrorCode::InvalidArgument,
            "--fresh deletes the installed application and all of its data",
        )
        .with_suggestion("Re-run with --yes after confirming that app data may be destroyed.")
        .with_suggestion(
            "Back up debuggable app data first with `androidforge app data-backup`.",
        ));
    }
    let apk = inspector.inspect(&options.apk_path, false).await?;
    let package = apk.package_or_error()?.to_string();
    if let Some(expected) = &options.expected_package
        && expected != &package
    {
        return Err(ForgeError::new(
            ErrorCode::InstallVerificationFailed,
            format!("the APK declares package '{package}', but the adapter expects '{expected}'"),
        )
        .with_context("preflight", true)
        .with_suggestion(
            "Install the APK for the package configured in .androidforge/project.toml.",
        )
        .with_suggestion("Or pass the correct --expected-package value."));
    }
    if !options.allowed_packages.is_empty()
        && !options
            .allowed_packages
            .iter()
            .any(|allowed| allowed == &package)
    {
        let allowed = options.allowed_packages.join(", ");
        return Err(ForgeError::new(
            ErrorCode::InstallVerificationFailed,
            format!("the APK declares package '{package}', which is not configured by the adapter"),
        )
        .with_context_string("apk_package", package.clone())
        .with_context_string("allowed_packages", allowed)
        .with_context("preflight", true)
        .with_suggestion(
            "Set [project].package and [project].debug_package to the application ids you install.",
        )
        .with_suggestion("Or pass the correct --expected-package value."));
    }
    if !options.expected_cert_sha256.is_empty() {
        let actual = apk.signature.cert_sha256.as_deref().map(|value| {
            normalize_fingerprint(value).unwrap_or_else(|| value.to_ascii_uppercase())
        });
        let matches = actual.as_deref().is_some_and(|actual| {
            options
                .expected_cert_sha256
                .iter()
                .filter_map(|value| normalize_fingerprint(value))
                .any(|expected| expected == actual)
        });
        if !matches {
            return Err(ForgeError::new(
                ErrorCode::SignatureVerificationFailed,
                format!("APK {package} is not signed by a configured certificate"),
            )
            .with_context_string(
                "expected_cert_sha256",
                options.expected_cert_sha256.join(", "),
            )
            .with_context(
                "actual_cert_sha256",
                serde_json::to_value(actual).unwrap_or(serde_json::Value::Null),
            )
            .with_context("preflight", true)
            .with_suggestion(
                "Add the actual certificate to [project].expected_cert_sha256 if this signer is trusted.",
            )
            .with_suggestion("Rebuild the APK with the expected signing key."));
        }
    }

    let existing = installed_package(adb, serial, &package).await?;
    let mut only_if_changed_diff = None;
    let mut only_if_changed_reason = None;
    if options.only_if_changed {
        match &existing {
            None => {
                only_if_changed_reason = Some("not_installed".to_string());
            }
            Some(existing_package) => {
                match inspect_installed(adb, inspector, serial, &package).await {
                    Ok(Some(identity)) => {
                        let diff = compare(&apk, &identity);
                        if diff.safe_to_skip_install {
                            let build = BuildIdentity {
                                package: Some(package.clone()),
                                version_name: existing_package
                                    .version_name
                                    .clone()
                                    .or(diff.installed.version_name.clone()),
                                version_code: existing_package
                                    .version_code
                                    .or(diff.installed.version_code),
                                apk_path: Some(options.apk_path.to_string_lossy().into_owned()),
                                apk_sha256: Some(apk.sha256.clone()),
                                installed_apk_path: Some(diff.installed.base_remote_path.clone()),
                                installed_apk_sha256: Some(diff.installed.base_apk.sha256.clone()),
                                cert_sha256: diff
                                    .installed
                                    .base_apk
                                    .signature
                                    .cert_sha256
                                    .clone()
                                    .or_else(|| apk.signature.cert_sha256.clone()),
                                debug_signed: apk.signature.debug_signed(),
                                verified: true,
                                notes: diff.warnings.clone(),
                            };
                            let mut warnings = vec![
                                "Installation was skipped because the candidate APK is byte-equivalent to the installed base APK."
                                    .to_string(),
                            ];
                            warnings.extend(diff.warnings.clone());
                            return Ok(InstallResult {
                                apk,
                                build,
                                installed: existing_package.clone(),
                                skipped: true,
                                decision: Some("skipped".to_string()),
                                reason: Some(
                                    "candidate package, version and base APK SHA-256 match the installed application"
                                        .to_string(),
                                ),
                                diff: Some(diff),
                                signature_diagnosis: None,
                                warnings,
                            });
                        }
                        only_if_changed_reason = Some(match diff.overall {
                            Equivalence::Different => "content_different".to_string(),
                            Equivalence::Unknown => "identity_unknown".to_string(),
                            Equivalence::Same => "split_identity_unknown".to_string(),
                        });
                        only_if_changed_diff = Some(diff);
                    }
                    Ok(None) => {
                        only_if_changed_reason = Some("installed_apk_unavailable".to_string());
                    }
                    Err(error) => {
                        only_if_changed_reason =
                            Some(format!("installed_apk_uninspectable: {}", error.message()));
                    }
                }
            }
        }
    }
    if options.fresh && existing.is_some() {
        let output = adb
            .run_checked(
                Some(serial),
                &["uninstall".to_string(), package.clone()],
                Some(options.timeout),
                None,
            )
            .await
            .map_err(|error| {
                ForgeError::new(
                    ErrorCode::InstallFailed,
                    format!("fresh install could not uninstall the existing {package}"),
                )
                .with_cause(error.message())
                .with_suggestion(
                    "Inspect the device package state with `androidforge app info <package>`.",
                )
            })?;
        if !output.stdout_text().contains("Success") {
            return Err(ForgeError::new(
                ErrorCode::InstallFailed,
                format!("device did not confirm uninstall of {package}"),
            )
            .with_context_string("output", truncate(output.combined_text().trim(), 4000)));
        }
    }

    let mut args = vec!["install".to_string(), "-r".to_string()];
    if options.allow_downgrade {
        args.push("-d".to_string());
    }
    args.push(options.apk_path.to_string_lossy().into_owned());
    let output = adb
        .run_raw(Some(serial), &args, Some(options.timeout), None)
        .await?;
    let combined = output.combined_text();
    let succeeded = output.success && combined.contains("Success");
    if !succeeded {
        return Err(install_failure(
            adb,
            inspector,
            serial,
            &package,
            &apk,
            existing,
            combined.trim().to_string(),
        )
        .await);
    }

    let Some(installed) = installed_package(adb, serial, &package).await? else {
        return Err(ForgeError::new(
            ErrorCode::InstallVerificationFailed,
            format!("ADB reported success, but package {package} is not installed"),
        )
        .with_suggestion("Run `androidforge app info <package>` and inspect logcat.")
        .with_suggestion("Check whether a device management policy removed the app.")
        .device(serial.to_string()));
    };

    if let Some(expected) = &options.expected_package
        && &installed.package != expected
    {
        return Err(ForgeError::new(
            ErrorCode::InstallVerificationFailed,
            format!(
                "device reports package '{}' but '{}' was requested",
                installed.package, expected
            ),
        )
        .device(serial.to_string()));
    }
    if let (Some(expected), Some(actual)) = (apk.version_code, installed.version_code)
        && expected != actual
    {
        return Err(ForgeError::new(
            ErrorCode::InstallVerificationFailed,
            format!(
                "incoming APK versionCode {expected} does not match installed versionCode {actual}"
            ),
        )
        .with_suggestion("Verify that the device did not keep an older package.")
        .with_suggestion("Inspect `androidforge app info <package>` and logcat.")
        .device(serial.to_string()));
    }

    let mut warnings = Vec::new();
    let mut installed_apk_sha256 = None;
    let mut installed_cert = None;
    let mut installed_cert_error = None;
    if options.verify_installed
        && let Some(remote) = &installed.base_apk_path
    {
        match pull_and_inspect_installed(adb, inspector, serial, remote).await {
            Ok((sha256, cert)) => {
                installed_apk_sha256 = Some(sha256.clone());
                installed_cert = cert;
                if sha256 != apk.sha256 {
                    warnings.push(
                            "the installed base APK hash differs from the local APK; this can happen on split/optimized installs"
                                .to_string(),
                        );
                }
            }
            Err(error) => {
                installed_cert_error = Some(error.message().to_string());
                warnings.push(format!(
                    "installed APK could not be verified: {}",
                    error.message()
                ));
            }
        }
    }

    if let (Some(incoming), Some(installed_cert)) = (
        apk.signature.cert_sha256.as_deref(),
        installed_cert.as_deref(),
    ) {
        let incoming = normalize_fingerprint(incoming);
        let installed = normalize_fingerprint(installed_cert);
        if incoming.is_some() && installed.is_some() && incoming != installed {
            return Err(ForgeError::new(
                ErrorCode::InstallVerificationFailed,
                "the installed package has a different signing certificate than the APK",
            )
            .with_suggestion("Uninstall the app explicitly and reinstall the expected build.")
            .device(serial.to_string()));
        }
    }
    if only_if_changed_reason.as_deref().is_some_and(|reason| {
        reason.starts_with("identity_unknown") || reason.contains("uninspectable")
    }) {
        warnings.push(
            "AndroidForge could not prove the installed build equivalent, so it installed rather than skipping. The reason is recorded in the result."
                .to_string(),
        );
    }

    let build = BuildIdentity {
        package: Some(package.clone()),
        version_name: installed
            .version_name
            .clone()
            .or_else(|| apk.version_name.clone()),
        version_code: installed.version_code.or(apk.version_code),
        apk_path: Some(options.apk_path.to_string_lossy().into_owned()),
        apk_sha256: Some(apk.sha256.clone()),
        installed_apk_path: installed.base_apk_path.clone(),
        installed_apk_sha256,
        cert_sha256: installed_cert.or_else(|| apk.signature.cert_sha256.clone()),
        debug_signed: apk.signature.debug_signed(),
        verified: installed_cert_error.is_none(),
        notes: warnings.clone(),
    };

    Ok(InstallResult {
        apk,
        build,
        installed,
        skipped: false,
        decision: options.only_if_changed.then(|| "installed".to_string()),
        reason: only_if_changed_reason,
        diff: only_if_changed_diff,
        signature_diagnosis: None,
        warnings,
    })
}

async fn install_failure(
    adb: &AdbClient,
    inspector: &ApkInspector,
    serial: &str,
    package: &str,
    apk: &ApkInfo,
    existing: Option<InstalledPackage>,
    install_output: String,
) -> ForgeError {
    let reason = failure_reason(&install_output).unwrap_or_else(|| install_output.clone());
    let lower = reason.to_ascii_lowercase();

    if lower.contains("install_failed_update_incompatible")
        || lower.contains("install_failed_shared_user_incompatible")
        || lower.contains("signatures do not match")
    {
        let installed = existing.clone();
        let mut notes = Vec::new();
        let mut installed_cert = None;
        if let Some(installed) = &installed {
            if let Some(remote) = &installed.base_apk_path {
                match pull_and_inspect_installed(adb, inspector, serial, remote).await {
                    Ok((_, cert)) => installed_cert = cert,
                    Err(error) => notes.push(format!(
                        "could not inspect the installed APK certificate: {}",
                        error.message()
                    )),
                }
            }
        } else {
            notes.push("the currently installed package could not be inspected".to_string());
        }
        let incoming_cert = apk.signature.cert_sha256.clone();
        let message = format!(
            "Android rejected the update because {package} is already installed with a different signing certificate. AndroidForge did not uninstall it."
        );
        let suggestions = vec![
            "Build the APK with the same signing key as the installed application.".to_string(),
            "If the existing app data can be destroyed, run `androidforge app uninstall <package>` and install again.".to_string(),
            "To replace it and delete app data in one explicit step, run `androidforge app install <apk> --fresh --yes`.".to_string(),
            "If the existing data matters, back it up first when the app is debuggable, then reinstall.".to_string(),
        ];
        let diagnosis = SignatureDiagnosis {
            incoming_package: package.to_string(),
            incoming_version_name: apk.version_name.clone(),
            incoming_version_code: apk.version_code,
            incoming_cert_sha256: incoming_cert.clone(),
            installed_version_name: installed.as_ref().and_then(|pkg| pkg.version_name.clone()),
            installed_version_code: installed.as_ref().and_then(|pkg| pkg.version_code),
            installed_cert_sha256: installed_cert.clone(),
            install_output: install_output.clone(),
            message: message.clone(),
            suggestions: suggestions.clone(),
            notes: notes.clone(),
        };
        let mut error = ForgeError::new(ErrorCode::InstallSignatureMismatch, message)
            .with_suggestions(suggestions)
            .with_context_string("package", package)
            .with_context_string("install_output", truncate(&install_output, 4000))
            .with_context(
                "signature_diagnosis",
                serde_json::to_value(&diagnosis).unwrap_or(serde_json::Value::Null),
            )
            .device(serial.to_string());
        error = error.with_context_string(
            "incoming_cert_sha256",
            incoming_cert.unwrap_or_else(|| "unavailable".to_string()),
        );
        error = error.with_context_string(
            "installed_cert_sha256",
            installed_cert.unwrap_or_else(|| "unavailable".to_string()),
        );
        return error;
    }

    let (code, suggestion) = if lower.contains("install_failed_version_downgrade") {
        (
            ErrorCode::InstallFailed,
            "Pass --downgrade to allow a lower versionCode, or uninstall the newer build explicitly.",
        )
    } else if lower.contains("install_failed_insufficient_storage") {
        (
            ErrorCode::InstallFailed,
            "Free storage on the device and retry. Inspect space with `androidforge adb shell df -h`.",
        )
    } else if lower.contains("install_parse_failed")
        || lower.contains("install_failed_invalid_apk")
        || lower.contains("failed to parse")
    {
        (
            ErrorCode::ApkInvalid,
            "Rebuild the APK and inspect it with `androidforge apk inspect <path>`.",
        )
    } else if lower.contains("install_failed_test_only") {
        (
            ErrorCode::InstallFailed,
            "The APK is marked testOnly. Build a non-testOnly APK, or use raw ADB with the -t flag if this is intentional.",
        )
    } else if lower.contains("install_failed_user_restricted") {
        (
            ErrorCode::InstallFailed,
            "The device policy blocks installs. Enable installation from the current source or use a different device.",
        )
    } else {
        (
            ErrorCode::InstallFailed,
            "Inspect the install output and device logcat. The APK and device state are unchanged by AndroidForge.",
        )
    };
    ForgeError::new(code, format!("installation of {package} failed: {reason}"))
        .with_suggestion(suggestion)
        .with_context_string("package", package)
        .with_context_string("install_output", truncate(&install_output, 4000))
        .device(serial.to_string())
}

pub async fn installed_package(
    adb: &AdbClient,
    serial: &str,
    package: &str,
) -> Result<Option<InstalledPackage>> {
    let path_output = match adb
        .shell_args(
            serial,
            &["pm".to_string(), "path".to_string(), package.to_string()],
            Some(Duration::from_secs(15)),
            None,
        )
        .await
    {
        Ok(output) => output,
        Err(_) => return Ok(None),
    };
    let paths = path_output
        .stdout_text()
        .lines()
        .filter_map(|line| line.trim().strip_prefix("package:"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Ok(None);
    }
    let dumpsys = adb
        .shell_args(
            serial,
            &[
                "dumpsys".to_string(),
                "package".to_string(),
                package.to_string(),
            ],
            Some(Duration::from_secs(30)),
            Some(16 * 1024 * 1024),
        )
        .await?;
    let text = dumpsys.stdout_text();
    let version_code =
        find_assignment(&text, "versionCode=").and_then(|value| value.parse::<i64>().ok());
    let version_name = find_assignment(&text, "versionName=");
    let data_dir = find_assignment(&text, "dataDir=");
    let installer = find_assignment(&text, "installerPackageName=");
    let flags = parse_flags(&text);
    let debuggable = flags.iter().any(|flag| flag == "DEBUGGABLE") || text.contains("DEBUGGABLE");
    Ok(Some(InstalledPackage {
        package: package.to_string(),
        version_name,
        version_code,
        base_apk_path: paths.first().cloned(),
        data_dir,
        debuggable,
        installer,
        flags,
    }))
}

async fn pull_and_inspect_installed(
    adb: &AdbClient,
    inspector: &ApkInspector,
    serial: &str,
    remote: &str,
) -> Result<(String, Option<String>)> {
    let temp = tempfile::tempdir().map_err(|error| {
        ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!("could not create a temporary directory: {error}"),
        )
    })?;
    let local = temp.path().join("installed.apk");
    adb.pull(serial, remote, &local).await?;
    let sha256 = sha256_file(&local)?;
    let info = inspector.inspect(&local, false).await?;
    Ok((sha256, info.signature.cert_sha256))
}

fn failure_reason(output: &str) -> Option<String> {
    for marker in [
        "Failure [",
        "failure [",
        "INSTALL_FAILED",
        "INSTALL_PARSE_FAILED",
    ] {
        if let Some(index) = output.find(marker) {
            if marker.contains('[') {
                let rest = &output[index + marker.len()..];
                if let Some(end) = rest.find(']') {
                    return Some(rest[..end].trim().to_string());
                }
            }
            let rest = &output[index..];
            return Some(rest.lines().next().unwrap_or(rest).trim().to_string());
        }
    }
    None
}

fn find_assignment(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(index) = line.find(key) {
            let rest = &line[index + key.len()..];
            let value = rest
                .split(|ch: char| ch.is_whitespace() || ch == ',' || ch == '}')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
            if !value.is_empty() && value != "null" {
                return Some(value);
            }
        }
    }
    None
}

fn parse_flags(text: &str) -> Vec<String> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("flags=[") {
            let inner = rest.split(']').next().unwrap_or(rest);
            return inner
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>();
        }
        if let Some(rest) = line.strip_prefix("pkgFlags=[") {
            let inner = rest.split(']').next().unwrap_or(rest);
            return inner
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>();
        }
    }
    Vec::new()
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    value.chars().take(max).collect::<String>() + "\n[truncated]"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_install_failure_reason() {
        assert_eq!(
            failure_reason(
                "Performing Streamed Install\nFailure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: ...]"
            )
            .as_deref(),
            Some("INSTALL_FAILED_UPDATE_INCOMPATIBLE: ...")
        );
        assert_eq!(
            failure_reason("INSTALL_PARSE_FAILED_NO_CERTIFICATES").as_deref(),
            Some("INSTALL_PARSE_FAILED_NO_CERTIFICATES")
        );
    }

    #[test]
    fn parses_dumpsys_assignments() {
        let text = "versionCode=42 minSdk=24 targetSdk=35\nversionName=1.2.3\ndataDir=/data/user/0/com.example\nflags=[ DEBUGGABLE HAS_CODE ]\n";
        assert_eq!(find_assignment(text, "versionCode=").as_deref(), Some("42"));
        assert_eq!(
            find_assignment(text, "versionName=").as_deref(),
            Some("1.2.3")
        );
        assert_eq!(
            find_assignment(text, "dataDir=").as_deref(),
            Some("/data/user/0/com.example")
        );
        assert_eq!(parse_flags(text), vec!["DEBUGGABLE", "HAS_CODE"]);
    }

    #[test]
    fn normalizes_certificates_from_apksigner() {
        assert!(normalize_fingerprint("61:ED:37:7E").is_none());
    }
}
