use std::path::{Path, PathBuf};
use std::time::Duration;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::model::BuildIdentity;
use androidforge_core::util::{normalize_fingerprint, sha256_file};
use androidforge_process::{CommandSpec, ProcessError, Runner};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApkSignatureStatus {
    pub checked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer_dn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v1: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v2: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v3: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl ApkSignatureStatus {
    pub fn debug_signed(&self) -> Option<bool> {
        self.signer_dn.as_ref().map(|dn| {
            let lower = dn.to_ascii_lowercase();
            lower.contains("cn=android debug") || lower.contains("android debug")
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApkInfo {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_sdk: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_sdk: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launchable_activity: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub architectures: Vec<String>,
    #[serde(default)]
    pub signature: ApkSignatureStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl ApkInfo {
    pub fn build_identity(&self) -> BuildIdentity {
        BuildIdentity {
            package: self.package.clone(),
            version_name: self.version_name.clone(),
            version_code: self.version_code,
            apk_path: Some(self.path.clone()),
            apk_sha256: Some(self.sha256.clone()),
            installed_apk_path: None,
            installed_apk_sha256: None,
            cert_sha256: self.signature.cert_sha256.clone(),
            debug_signed: self.signature.debug_signed(),
            verified: self.signature.verified.unwrap_or(false),
            notes: self.notes.clone(),
        }
    }

    pub fn package_or_error(&self) -> Result<&str> {
        self.package.as_deref().ok_or_else(|| {
            ForgeError::new(
                ErrorCode::ApkInvalid,
                format!("APK {} does not declare a package name", self.path),
            )
            .with_suggestion("Run `androidforge apk inspect <path>` to inspect the APK.")
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyCheck {
    pub name: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApkVerificationReport {
    pub passed: bool,
    pub apk: ApkInfo,
    pub checks: Vec<VerifyCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_cert_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version_code: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct VerifyOptions {
    pub expected_cert_sha256: Option<String>,
    pub expected_package: Option<String>,
    pub min_version_code: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ApkInspector {
    pub aapt2: Option<PathBuf>,
    pub apksigner: Option<PathBuf>,
    pub java: Option<PathBuf>,
    pub runner: Runner,
    pub trace: bool,
}

impl ApkInspector {
    pub fn new(
        aapt2: Option<PathBuf>,
        apksigner: Option<PathBuf>,
        java: Option<PathBuf>,
        trace: bool,
    ) -> Self {
        Self {
            aapt2,
            apksigner,
            java,
            runner: Runner::new(trace),
            trace,
        }
    }

    pub async fn inspect(&self, path: &Path, require_signature: bool) -> Result<ApkInfo> {
        if !path.is_file() {
            return Err(ForgeError::new(
                ErrorCode::ApkInvalid,
                format!("APK file {} was not found", path.display()),
            )
            .with_suggestion("Verify the APK path and build the application first."));
        }
        let size_bytes = std::fs::metadata(path)
            .map_err(|error| {
                ForgeError::new(
                    ErrorCode::ApkInvalid,
                    format!("could not inspect {}: {error}", path.display()),
                )
            })?
            .len();
        let sha256 = sha256_file(path)?;

        let aapt2 = self.aapt2.as_ref().ok_or_else(|| {
            ForgeError::new(
                ErrorCode::ToolNotFound,
                "AAPT2 is required to inspect APK metadata",
            )
            .with_suggestion("Install Android SDK Build-Tools and run `androidforge doctor`.")
        })?;
        let badging = self
            .run_tool(
                aapt2,
                &[
                    "dump".to_string(),
                    "badging".to_string(),
                    path.to_string_lossy().into_owned(),
                ],
                Duration::from_secs(60),
            )
            .await?;
        if !badging.success {
            return Err(ForgeError::new(
                ErrorCode::ApkInvalid,
                format!("AAPT2 could not read {}", path.display()),
            )
            .with_context_string("stdout", truncate(&badging.stdout_text(), 4000))
            .with_context_string("stderr", truncate(&badging.stderr_text(), 4000))
            .with_suggestion("Verify that the file is a valid APK."));
        }
        let metadata = parse_badging(&badging.stdout_text());
        if metadata.package.is_none() {
            return Err(ForgeError::new(
                ErrorCode::ApkInvalid,
                format!("AAPT2 did not find a package name in {}", path.display()),
            )
            .with_context_string("aapt2_output", truncate(&badging.stdout_text(), 4000)));
        }

        let mut info = ApkInfo {
            path: path.to_string_lossy().into_owned(),
            size_bytes,
            sha256,
            package: metadata.package,
            version_name: metadata.version_name,
            version_code: metadata.version_code,
            min_sdk: metadata.min_sdk,
            target_sdk: metadata.target_sdk,
            label: metadata.label,
            launchable_activity: metadata.launchable_activity,
            architectures: metadata.architectures,
            signature: ApkSignatureStatus::default(),
            notes: Vec::new(),
        };

        match &self.apksigner {
            Some(apksigner) => {
                match self
                    .run_tool(
                        apksigner,
                        &[
                            "verify".to_string(),
                            "--verbose".to_string(),
                            "--print-certs".to_string(),
                            path.to_string_lossy().into_owned(),
                        ],
                        Duration::from_secs(120),
                    )
                    .await
                {
                    Ok(output) => {
                        info.signature = parse_apksigner(
                            &output.stdout_text(),
                            &output.stderr_text(),
                            output.exit_code,
                        );
                    }
                    Err(error) => {
                        if require_signature {
                            return Err(error);
                        }
                        info.notes.push(format!(
                            "signature inspection was unavailable: {}",
                            error.message()
                        ));
                    }
                }
            }
            None => {
                if require_signature {
                    return Err(ForgeError::new(
                        ErrorCode::ToolNotFound,
                        "APK Signer is required to verify the APK signature",
                    )
                    .with_suggestion("Install Android SDK Build-Tools and run `androidforge doctor`.")
                    .with_suggestion("APK Signer is a local SDK tool; no key material is needed for verification."));
                }
                info.notes.push(
                    "APK Signer was not found; signing certificate and scheme details are unavailable."
                        .to_string(),
                );
            }
        }

        Ok(info)
    }

    pub async fn verify(
        &self,
        path: &Path,
        options: &VerifyOptions,
    ) -> Result<ApkVerificationReport> {
        let apk = self.inspect(path, true).await?;
        let mut checks = Vec::new();
        let signature_verified = apk.signature.verified.unwrap_or(false);
        checks.push(VerifyCheck {
            name: "signature_verified".to_string(),
            passed: signature_verified,
            expected: Some("true".to_string()),
            actual: apk.signature.verified.map(|value| value.to_string()),
            note: None,
        });
        checks.push(VerifyCheck {
            name: "signed".to_string(),
            passed: apk.signature.signed.unwrap_or(false),
            expected: Some("true".to_string()),
            actual: apk.signature.signed.map(|value| value.to_string()),
            note: None,
        });

        if let Some(expected) = &options.expected_cert_sha256 {
            let expected_norm = normalize_fingerprint(expected);
            let actual_norm = apk
                .signature
                .cert_sha256
                .as_deref()
                .and_then(normalize_fingerprint);
            checks.push(VerifyCheck {
                name: "certificate_sha256".to_string(),
                passed: match (&expected_norm, &actual_norm) {
                    (Some(expected), Some(actual)) => expected == actual,
                    _ => false,
                },
                expected: expected_norm.or_else(|| Some(expected.clone())),
                actual: apk.signature.cert_sha256.clone(),
                note: if normalize_fingerprint(expected).is_none() {
                    Some(
                        "expected certificate fingerprint is not a valid SHA-256 digest"
                            .to_string(),
                    )
                } else {
                    None
                },
            });
        }

        if let Some(expected_package) = &options.expected_package {
            checks.push(VerifyCheck {
                name: "package".to_string(),
                passed: apk.package.as_deref() == Some(expected_package.as_str()),
                expected: Some(expected_package.clone()),
                actual: apk.package.clone(),
                note: None,
            });
        }

        if let Some(min_version_code) = options.min_version_code {
            checks.push(VerifyCheck {
                name: "min_version_code".to_string(),
                passed: apk.version_code.unwrap_or(-1) >= min_version_code,
                expected: Some(min_version_code.to_string()),
                actual: apk.version_code.map(|value| value.to_string()),
                note: None,
            });
        }

        let passed = checks.iter().all(|check| check.passed);
        Ok(ApkVerificationReport {
            passed,
            apk,
            checks,
            expected_cert_sha256: options.expected_cert_sha256.clone(),
            expected_package: options.expected_package.clone(),
            min_version_code: options.min_version_code,
        })
    }

    async fn run_tool(
        &self,
        tool: &Path,
        args: &[String],
        timeout: Duration,
    ) -> Result<androidforge_process::CommandOutput> {
        let (program, mut all_args) = self.tool_invocation(tool)?;
        all_args.extend(args.iter().cloned());
        let spec = CommandSpec::new(program)
            .args(all_args)
            .timeout(timeout)
            .trace(self.trace)
            .hide_window(true)
            .max_stdout(8 * 1024 * 1024)
            .max_stderr(4 * 1024 * 1024);
        self.runner
            .run(spec)
            .await
            .map_err(|error| map_tool_error(error, tool))
    }

    fn tool_invocation(&self, tool: &Path) -> Result<(String, Vec<String>)> {
        let extension = tool
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());
        match extension.as_deref() {
            Some("jar") => {
                let java = self.java.as_ref().ok_or_else(|| {
                    ForgeError::new(
                        ErrorCode::ToolNotFound,
                        format!("Java is required to run {}", tool.display()),
                    )
                    .with_suggestion("Install a JDK or set [sdk].java / JAVA_HOME.")
                })?;
                Ok((
                    java.to_string_lossy().into_owned(),
                    vec!["-jar".to_string(), tool.to_string_lossy().into_owned()],
                ))
            }
            Some("bat") | Some("cmd") => {
                // Build-tools ship apksigner.bat, but the jar is the stable
                // interface and avoids routing normal operations through cmd.exe.
                let stem = tool
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("tool");
                if let Some(parent) = tool.parent() {
                    let candidates = [
                        parent.join("lib").join(format!("{stem}.jar")),
                        parent
                            .parent()
                            .unwrap_or(parent)
                            .join("lib")
                            .join(format!("{stem}.jar")),
                    ];
                    for jar in candidates {
                        if jar.is_file() {
                            let java = self.java.as_ref().ok_or_else(|| {
                                ForgeError::new(
                                    ErrorCode::ToolNotFound,
                                    format!("Java is required to run {}", jar.display()),
                                )
                                .with_suggestion("Install a JDK or set [sdk].java / JAVA_HOME.")
                            })?;
                            return Ok((
                                java.to_string_lossy().into_owned(),
                                vec!["-jar".to_string(), jar.to_string_lossy().into_owned()],
                            ));
                        }
                    }
                }
                Err(ForgeError::new(
                    ErrorCode::ToolNotFound,
                    format!(
                        "{} is a batch wrapper and its Java archive was not found",
                        tool.display()
                    ),
                )
                .with_suggestion("Point [sdk].apksigner at apksigner.jar or install a complete Build-Tools package."))
            }
            _ => Ok((tool.to_string_lossy().into_owned(), Vec::new())),
        }
    }
}

fn map_tool_error(error: ProcessError, tool: &Path) -> ForgeError {
    match error {
        ProcessError::Spawn { source, .. } => ForgeError::new(
            ErrorCode::ToolNotFound,
            format!("could not start {}: {source}", tool.display()),
        )
        .with_suggestion("Run `androidforge doctor` to inspect Android SDK tool discovery."),
        ProcessError::Timeout { timeout_ms, .. } => ForgeError::new(
            ErrorCode::CommandTimeout,
            format!("{} timed out after {timeout_ms} ms", tool.display()),
        )
        .with_suggestion("Retry with a larger timeout or verify the APK is readable."),
        other => ForgeError::from(other),
    }
}

#[derive(Debug, Default)]
struct Badging {
    package: Option<String>,
    version_name: Option<String>,
    version_code: Option<i64>,
    min_sdk: Option<u32>,
    target_sdk: Option<u32>,
    label: Option<String>,
    launchable_activity: Option<String>,
    architectures: Vec<String>,
}

fn parse_badging(output: &str) -> Badging {
    let mut badging = Badging::default();
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("package:") {
            let values = parse_key_values(rest);
            badging.package = values.get("name").cloned();
            badging.version_name = values.get("versionName").cloned();
            badging.version_code = values
                .get("versionCode")
                .and_then(|value| value.parse::<i64>().ok());
        } else if let Some(rest) = line
            .strip_prefix("sdkVersion:")
            .or_else(|| line.strip_prefix("minSdkVersion:"))
        {
            badging.min_sdk = quoted_values(rest)
                .first()
                .and_then(|value| value.parse::<u32>().ok());
        } else if let Some(rest) = line.strip_prefix("targetSdkVersion:") {
            badging.target_sdk = quoted_values(rest)
                .first()
                .and_then(|value| value.parse::<u32>().ok());
        } else if let Some(rest) = line.strip_prefix("application-label:") {
            badging.label = first_quoted(rest);
        } else if let Some(rest) = line.strip_prefix("application-label-") {
            if badging.label.is_none()
                && let Some((_, value)) = rest.split_once(':')
            {
                badging.label = first_quoted(value);
            }
        } else if let Some(rest) = line.strip_prefix("launchable-activity:") {
            badging.launchable_activity = parse_key_values(rest).get("name").cloned();
        } else if let Some(rest) = line.strip_prefix("native-code:") {
            badging.architectures = quoted_values(rest);
        }
    }
    badging
}

fn parse_key_values(input: &str) -> std::collections::BTreeMap<String, String> {
    let mut values = std::collections::BTreeMap::new();
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }
        let key_start = index;
        while index < chars.len() && chars[index] != '=' && !chars[index].is_whitespace() {
            index += 1;
        }
        if index >= chars.len() || chars[index] != '=' {
            while index < chars.len() && !chars[index].is_whitespace() {
                index += 1;
            }
            continue;
        }
        let key: String = chars[key_start..index].iter().collect();
        index += 1;
        if index >= chars.len() {
            break;
        }
        let quote = chars[index];
        if quote != '\'' && quote != '"' {
            let value_start = index;
            while index < chars.len() && !chars[index].is_whitespace() {
                index += 1;
            }
            values.insert(key, chars[value_start..index].iter().collect());
            continue;
        }
        index += 1;
        let mut value = String::new();
        while index < chars.len() {
            if chars[index] == '\\' && index + 1 < chars.len() {
                value.push(chars[index + 1]);
                index += 2;
                continue;
            }
            if chars[index] == quote {
                index += 1;
                break;
            }
            value.push(chars[index]);
            index += 1;
        }
        values.insert(key, value);
    }
    values
}

fn quoted_values(input: &str) -> Vec<String> {
    let mut values = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let quote = chars[index];
        if quote != '\'' && quote != '"' {
            index += 1;
            continue;
        }
        index += 1;
        let mut value = String::new();
        while index < chars.len() {
            if chars[index] == '\\' && index + 1 < chars.len() {
                value.push(chars[index + 1]);
                index += 2;
                continue;
            }
            if chars[index] == quote {
                index += 1;
                break;
            }
            value.push(chars[index]);
            index += 1;
        }
        values.push(value);
    }
    values
}

fn first_quoted(input: &str) -> Option<String> {
    quoted_values(input).into_iter().next()
}

fn parse_apksigner(stdout: &str, stderr: &str, exit_code: Option<i32>) -> ApkSignatureStatus {
    let combined = format!("{}\n{}", stdout, stderr);
    let mut status = ApkSignatureStatus {
        checked: true,
        signed: None,
        verified: None,
        exit_code,
        ..ApkSignatureStatus::default()
    };
    let mut signed = false;
    for line in combined.lines() {
        let line = line.trim();
        if line == "Verifies" {
            status.verified = Some(true);
        } else if line.contains("DOES NOT VERIFY") {
            status.verified = Some(false);
        } else if let Some(value) = line.strip_prefix("Verified using v1 scheme") {
            status.v1 = value.parse_bool_after_colon();
        } else if let Some(value) = line.strip_prefix("Verified using v2 scheme") {
            status.v2 = value.parse_bool_after_colon();
        } else if let Some(value) = line.strip_prefix("Verified using v3 scheme") {
            status.v3 = value.parse_bool_after_colon();
        } else if let Some(value) = line.strip_prefix("Number of signers:") {
            status.signer_count = value.trim().parse::<u32>().ok();
            signed = true;
        } else if let Some(value) = line.strip_prefix("Signer #1 certificate DN:") {
            status.signer_dn = Some(value.trim().to_string());
            signed = true;
        } else if let Some(value) = line.strip_prefix("Signer #1 certificate SHA-256 digest:") {
            status.cert_sha256 = normalize_fingerprint(value.trim());
            if status.cert_sha256.is_some() {
                signed = true;
            }
        } else if line.starts_with("ERROR:") {
            status.errors.push(line.to_string());
        } else if line.starts_with("WARNING:") {
            status.warnings.push(line.to_string());
        }
    }
    if status.verified.is_none() {
        status.verified = exit_code.map(|code| code == 0);
    }
    status.signed =
        Some(signed || status.cert_sha256.is_some() || status.signer_count.unwrap_or(0) > 0);
    status
}

trait BoolAfterColon {
    fn parse_bool_after_colon(&self) -> Option<bool>;
}

impl BoolAfterColon for str {
    fn parse_bool_after_colon(&self) -> Option<bool> {
        let value = self.rsplit(':').next()?.trim();
        match value {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let truncated: String = value.chars().take(max).collect();
    format!("{truncated}\n[truncated]")
}

#[cfg(test)]
mod tests {
    use super::*;

    const BADGING: &str = r#"
package: name='com.example.app' versionCode='42' versionName='1.2.3' compileSdkVersion='35' platformBuildVersionName='15'
sdkVersion:'24'
targetSdkVersion:'35'
application-label:'Example App'
application-label-en:'Example App'
launchable-activity: name='com.example.app.MainActivity'  label='' icon=''
native-code: 'arm64-v8a' 'x86_64'
"#;

    #[test]
    fn parses_aapt2_badging() {
        let info = parse_badging(BADGING);
        assert_eq!(info.package.as_deref(), Some("com.example.app"));
        assert_eq!(info.version_code, Some(42));
        assert_eq!(info.version_name.as_deref(), Some("1.2.3"));
        assert_eq!(info.min_sdk, Some(24));
        assert_eq!(info.target_sdk, Some(35));
        assert_eq!(info.label.as_deref(), Some("Example App"));
        assert_eq!(
            info.launchable_activity.as_deref(),
            Some("com.example.app.MainActivity")
        );
        assert_eq!(info.architectures, vec!["arm64-v8a", "x86_64"]);
    }

    #[test]
    fn parses_apksigner_output() {
        let output = r#"
Verifies
Verified using v1 scheme (JAR signing): true
Verified using v2 scheme (APK Signature Scheme v2): true
Verified using v3 scheme (APK Signature Scheme v3): false
Number of signers: 1
Signer #1 certificate DN: CN=Android Debug, O=Android, C=US
Signer #1 certificate SHA-256 digest: 61:ED:37:7E:85:D3:86:A8:DF:EE:6B:86:4B:D8:5B:0B:FA:A5:AF:81:9B:3F:6B:5E:6B:6B:FF:3A:8E:0E:3F:4F
"#;
        let status = parse_apksigner(output, "", Some(0));
        assert_eq!(status.verified, Some(true));
        assert_eq!(status.signed, Some(true));
        assert_eq!(status.v1, Some(true));
        assert_eq!(status.v2, Some(true));
        assert_eq!(status.v3, Some(false));
        assert_eq!(status.signer_count, Some(1));
        assert!(status.cert_sha256.is_some());
        assert_eq!(status.debug_signed(), Some(true));
    }

    #[test]
    fn parses_unsigned_apk() {
        let status = parse_apksigner(
            "DOES NOT VERIFY\nERROR: Missing AndroidManifest.xml\n",
            "",
            Some(1),
        );
        assert_eq!(status.verified, Some(false));
        assert_eq!(status.signed, Some(false));
        assert!(!status.errors.is_empty());
    }

    #[test]
    fn parse_quoted_values_handles_spaces_and_escapes() {
        let values = quoted_values("'hello world' 'it\\'s fine'");
        assert_eq!(values, vec!["hello world", "it's fine"]);
    }
}
