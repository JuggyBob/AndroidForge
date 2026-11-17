//! Explicit, reproducible performance evidence.
//!
//! Values are reported only when the device output actually contains them.
//! Missing metrics are recorded as unknown with a reason; AndroidForge never
//! invents a startup time, frame count or CPU percentage. Cold measurement
//! requires an explicit request because it force-stops the application.

use std::time::{Duration, Instant};

use androidforge_adb::AdbClient;
use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::util::now_rfc3339;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const PERFORMANCE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceCommand {
    pub name: String,
    pub command: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StartupMeasurement {
    pub requested: bool,
    pub cold: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_time_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_time_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub this_time_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JankMeasurement {
    pub reset_before_launch: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_frames: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub janky_frames: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub janky_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentile_50_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentile_90_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentile_95_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentile_99_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missed_vsync: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryMeasurement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_pss_kb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_rss_kb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_heap_pss_kb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_heap_pss_kb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CpuMeasurement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceReport {
    pub schema_version: u32,
    pub package: String,
    pub serial: String,
    pub collected_at: String,
    pub cold_start_requested: bool,
    pub settle_ms: u64,
    pub foreground: Option<bool>,
    pub activity: Option<String>,
    pub startup: StartupMeasurement,
    pub jank: JankMeasurement,
    pub memory: MemoryMeasurement,
    pub cpu: CpuMeasurement,
    pub commands: Vec<PerformanceCommand>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PerformanceOptions<'a> {
    pub package: &'a str,
    pub activity: Option<&'a str>,
    pub cold: bool,
    pub settle: Duration,
    pub timeout: Duration,
}

pub async fn collect_performance(
    adb: &AdbClient,
    serial: &str,
    options: &PerformanceOptions<'_>,
) -> Result<PerformanceReport> {
    let package = options.package;
    let mut warnings = Vec::new();
    let mut commands = Vec::new();

    let install_path = shell(
        adb,
        serial,
        &["pm".to_string(), "path".to_string(), package.to_string()],
        options.timeout,
    )
    .await?;
    commands.push(command(
        "verify_install",
        &format!("pm path {package}"),
        &install_path,
        1200,
    ));
    if !install_path.text.contains("package:") {
        return Err(ForgeError::new(
            ErrorCode::PackageNotFound,
            format!("package {package} is not installed on {serial}"),
        )
        .with_suggestion("Install the application with `androidforge app install <apk>`.")
        .device(serial.to_string()));
    }

    let component = if let Some(activity) = options.activity {
        Some(normalize_component(package, activity))
    } else {
        let resolved = shell(
            adb,
            serial,
            &[
                "cmd".to_string(),
                "package".to_string(),
                "resolve-activity".to_string(),
                "--brief".to_string(),
                package.to_string(),
            ],
            options.timeout,
        )
        .await
        .ok();
        if let Some(resolved) = &resolved {
            commands.push(command(
                "resolve_activity",
                &format!("cmd package resolve-activity --brief {package}"),
                resolved,
                2000,
            ));
        }
        resolved.and_then(|output| {
            output.text.lines().rev().find_map(|line| {
                let line = line.trim();
                (line.contains('/') && line.contains('.')).then(|| line.to_string())
            })
        })
    };
    if component.is_none() {
        warnings.push(
            "could not resolve a launcher activity; startup timing will be unknown because monkey does not report it"
                .to_string(),
        );
    }

    if options.cold {
        let stopped = shell(
            adb,
            serial,
            &[
                "am".to_string(),
                "force-stop".to_string(),
                package.to_string(),
            ],
            options.timeout,
        )
        .await;
        match &stopped {
            Ok(output) => commands.push(command(
                "force_stop",
                &format!("am force-stop {package}"),
                output,
                1000,
            )),
            Err(error) => {
                warnings.push(format!("cold-start force-stop failed: {}", error.message()));
                commands.push(PerformanceCommand {
                    name: "force_stop".to_string(),
                    command: format!("am force-stop {package}"),
                    status: "failed".to_string(),
                    excerpt: None,
                    reason: Some(error.message().to_string()),
                });
            }
        }
        tokio::time::sleep(Duration::from_millis(750)).await;
    }

    let mut jank = JankMeasurement::default();
    match shell(
        adb,
        serial,
        &[
            "dumpsys".to_string(),
            "gfxinfo".to_string(),
            package.to_string(),
            "reset".to_string(),
        ],
        options.timeout,
    )
    .await
    {
        Ok(output) => {
            jank.reset_before_launch = true;
            commands.push(command(
                "gfxinfo_reset",
                &format!("dumpsys gfxinfo {package} reset"),
                &output,
                1200,
            ));
        }
        Err(error) => {
            jank.reset_before_launch = false;
            warnings.push(format!(
                "could not reset gfxinfo counters; jank data covers an unknown window: {}",
                error.message()
            ));
            commands.push(PerformanceCommand {
                name: "gfxinfo_reset".to_string(),
                command: format!("dumpsys gfxinfo {package} reset"),
                status: "failed".to_string(),
                excerpt: None,
                reason: Some(error.message().to_string()),
            });
        }
    }

    let launch_argv = if let Some(component) = &component {
        vec![
            "am".to_string(),
            "start".to_string(),
            "-W".to_string(),
            "-n".to_string(),
            component.clone(),
        ]
    } else {
        vec![
            "monkey".to_string(),
            "-p".to_string(),
            package.to_string(),
            "-c".to_string(),
            "android.intent.category.LAUNCHER".to_string(),
            "1".to_string(),
        ]
    };
    let launch = shell(adb, serial, &launch_argv, options.timeout).await?;
    commands.push(command("startup", &launch_argv.join(" "), &launch, 4000));
    let mut startup = parse_am_start(&launch.text);
    startup.requested = true;
    startup.cold = options.cold;
    startup.activity = component.clone();
    if component.is_none() {
        startup.unknown_reason = Some("monkey launch does not report startup timing".to_string());
    }
    if let Some(reason) = &startup.unknown_reason {
        warnings.push(format!("startup timing is unknown: {reason}"));
    }

    if !options.settle.is_zero() {
        tokio::time::sleep(options.settle).await;
    }

    let foreground = adb.current_focus(serial).await;
    let foreground_matches = foreground
        .as_ref()
        .map(|focus| focus.package == package)
        .unwrap_or(false);
    if foreground.is_some() && !foreground_matches {
        warnings.push(format!(
            "the package is not the current focus (focus: {})",
            foreground
                .as_ref()
                .map(|focus| focus.package.as_str())
                .unwrap_or("unknown")
        ));
    }

    match shell(
        adb,
        serial,
        &[
            "dumpsys".to_string(),
            "gfxinfo".to_string(),
            package.to_string(),
        ],
        options.timeout,
    )
    .await
    {
        Ok(output) => {
            let parsed = parse_gfxinfo(&output.text);
            let reset = jank.reset_before_launch;
            jank = parsed;
            jank.reset_before_launch = reset;
            commands.push(command(
                "gfxinfo",
                &format!("dumpsys gfxinfo {package}"),
                &output,
                2500,
            ));
            if let Some(reason) = &jank.unknown_reason {
                warnings.push(format!("jank metrics are unknown: {reason}"));
            }
        }
        Err(error) => {
            jank.unknown_reason = Some(error.message().to_string());
            warnings.push(format!("could not read gfxinfo: {}", error.message()));
            commands.push(PerformanceCommand {
                name: "gfxinfo".to_string(),
                command: format!("dumpsys gfxinfo {package}"),
                status: "failed".to_string(),
                excerpt: None,
                reason: Some(error.message().to_string()),
            });
        }
    }

    let memory = match shell(
        adb,
        serial,
        &[
            "dumpsys".to_string(),
            "meminfo".to_string(),
            package.to_string(),
        ],
        options.timeout,
    )
    .await
    {
        Ok(output) => {
            commands.push(command(
                "meminfo",
                &format!("dumpsys meminfo {package}"),
                &output,
                2500,
            ));
            let parsed = parse_meminfo(&output.text);
            if let Some(reason) = &parsed.unknown_reason {
                warnings.push(format!("memory metrics are unknown: {reason}"));
            }
            parsed
        }
        Err(error) => {
            warnings.push(format!("could not read meminfo: {}", error.message()));
            commands.push(PerformanceCommand {
                name: "meminfo".to_string(),
                command: format!("dumpsys meminfo {package}"),
                status: "failed".to_string(),
                excerpt: None,
                reason: Some(error.message().to_string()),
            });
            MemoryMeasurement {
                unknown_reason: Some(error.message().to_string()),
                ..MemoryMeasurement::default()
            }
        }
    };

    let mut cpu = match shell(
        adb,
        serial,
        &["dumpsys".to_string(), "cpuinfo".to_string()],
        options.timeout,
    )
    .await
    {
        Ok(output) => {
            commands.push(command("cpuinfo", "dumpsys cpuinfo", &output, 2500));
            let parsed = parse_cpuinfo(&output.text, package);
            if let Some(reason) = &parsed.unknown_reason {
                warnings.push(format!("CPU metrics are unknown: {reason}"));
            }
            parsed
        }
        Err(error) => {
            warnings.push(format!("could not read cpuinfo: {}", error.message()));
            commands.push(PerformanceCommand {
                name: "cpuinfo".to_string(),
                command: "dumpsys cpuinfo".to_string(),
                status: "failed".to_string(),
                excerpt: None,
                reason: Some(error.message().to_string()),
            });
            CpuMeasurement {
                unknown_reason: Some(error.message().to_string()),
                ..CpuMeasurement::default()
            }
        }
    };

    if cpu.total_percent.is_none()
        && let Some(pid) = memory.pid
    {
        match sample_proc_cpu(adb, serial, pid, options.timeout).await {
            Ok((sample, ticks, first, second)) => {
                cpu = sample;
                commands.push(PerformanceCommand {
                        name: "proc_cpu".to_string(),
                        command: format!("cat /proc/{pid}/stat (two samples)"),
                        status: "ok".to_string(),
                        excerpt: Some(format!(
                            "utime+stime samples {first} -> {second} over a 500 ms window; CLK_TCK={ticks}"
                        )),
                        reason: None,
                    });
                warnings.retain(|warning| !warning.starts_with("CPU metrics are unknown"));
            }
            Err(error) => {
                cpu.unknown_reason = Some(format!(
                    "{}; /proc/{pid}/stat sampling failed: {}",
                    cpu.unknown_reason
                        .as_deref()
                        .unwrap_or("dumpsys cpuinfo did not report the package"),
                    error.message()
                ));
            }
        }
    }
    Ok(PerformanceReport {
        schema_version: PERFORMANCE_SCHEMA_VERSION,
        package: package.to_string(),
        serial: serial.to_string(),
        collected_at: now_rfc3339(),
        cold_start_requested: options.cold,
        settle_ms: options.settle.as_millis() as u64,
        foreground: foreground.map(|_| foreground_matches),
        activity: component,
        startup,
        jank,
        memory,
        cpu,
        commands,
        warnings,
    })
}

pub fn parse_am_start(text: &str) -> StartupMeasurement {
    let mut measurement = StartupMeasurement {
        requested: true,
        status: value_after(text, "Status:"),
        total_time_ms: millis_after(text, "TotalTime:"),
        wait_time_ms: millis_after(text, "WaitTime:"),
        this_time_ms: millis_after(text, "ThisTime:"),
        raw_output: Some(excerpt(text, 4000)),
        ..StartupMeasurement::default()
    };
    if measurement.total_time_ms.is_none() {
        measurement.unknown_reason =
            Some("am start output did not include a TotalTime value".to_string());
    }
    measurement
}
pub fn parse_gfxinfo(text: &str) -> JankMeasurement {
    let total_frames = capture_u64(r"(?m)^\s*Total frames rendered:\s*(\d+)", text);
    let (janky_frames, janky_percent) = if let Some(caps) =
        Regex::new(r"(?m)^\s*Janky frames:\s*(\d+)\s*\(([\d.]+)%\)")
            .expect("jank regex")
            .captures(text)
    {
        (
            caps.get(1).and_then(|value| value.as_str().parse().ok()),
            caps.get(2).and_then(|value| value.as_str().parse().ok()),
        )
    } else {
        (None, None)
    };
    let mut measurement = JankMeasurement {
        total_frames,
        janky_frames,
        janky_percent,
        percentile_50_ms: percentile(text, 50),
        percentile_90_ms: percentile(text, 90),
        percentile_95_ms: percentile(text, 95),
        percentile_99_ms: percentile(text, 99),
        missed_vsync: capture_u64(r"(?m)^\s*Number Missed Vsync:\s*(\d+)", text),
        ..JankMeasurement::default()
    };
    if measurement.total_frames.is_none() {
        measurement.unknown_reason = Some(if text.contains("No process") {
            "gfxinfo reported no process for the package".to_string()
        } else if text.trim().is_empty() {
            "gfxinfo produced no output".to_string()
        } else {
            "gfxinfo did not report frame statistics".to_string()
        });
    }
    measurement
}
pub fn parse_meminfo(text: &str) -> MemoryMeasurement {
    let mut measurement = MemoryMeasurement {
        pid: Regex::new(r"\*\* MEMINFO in pid (\d+)")
            .expect("meminfo pid regex")
            .captures(text)
            .and_then(|captures| captures.get(1))
            .and_then(|value| value.as_str().parse().ok()),
        total_pss_kb: capture_u64(r"TOTAL PSS:\s*(\d+)", text),
        total_rss_kb: capture_u64(r"TOTAL RSS:\s*(\d+)", text),
        java_heap_pss_kb: capture_u64(r"(?m)^\s*Java Heap:\s*(\d+)", text),
        native_heap_pss_kb: capture_u64(r"(?m)^\s*Native Heap:\s*(\d+)", text),
        unknown_reason: None,
    };
    if measurement.total_pss_kb.is_none() && measurement.total_rss_kb.is_none() {
        measurement.unknown_reason = Some(if text.contains("No process") {
            "meminfo reported no process for the package".to_string()
        } else if text.trim().is_empty() {
            "meminfo produced no output".to_string()
        } else {
            "meminfo did not report TOTAL PSS or TOTAL RSS".to_string()
        });
    }
    measurement
}
pub fn parse_cpuinfo(text: &str, package: &str) -> CpuMeasurement {
    let mut measurement = CpuMeasurement::default();
    let regex = Regex::new(
        r"(?m)^\s*([\d.]+)%\s+(\d+)/(\S+):\s+([\d.]+)%\s+user\s+\+\s+([\d.]+)%\s+kernel",
    )
    .expect("cpuinfo regex");
    let mut exact = None;
    let mut prefixed: Option<(f64, CpuMeasurement)> = None;
    for caps in regex.captures_iter(text) {
        let Some(process) = caps.get(3).map(|value| value.as_str().to_string()) else {
            continue;
        };
        let total = caps
            .get(1)
            .and_then(|value| value.as_str().parse::<f64>().ok());
        let user = caps
            .get(4)
            .and_then(|value| value.as_str().parse::<f64>().ok());
        let kernel = caps
            .get(5)
            .and_then(|value| value.as_str().parse::<f64>().ok());
        let candidate = CpuMeasurement {
            process: Some(process.clone()),
            total_percent: total,
            user_percent: user,
            kernel_percent: kernel,
            unknown_reason: None,
        };
        if process == package {
            exact = Some(candidate.clone());
            break;
        }
        if process.starts_with(&format!("{package}:")) {
            let rank = total.unwrap_or(0.0);
            if prefixed.as_ref().is_none_or(|(best, _)| rank > *best) {
                prefixed = Some((rank, candidate));
            }
        }
    }
    if let Some(exact) = exact {
        return exact;
    }
    if let Some((_, prefixed)) = prefixed {
        return prefixed;
    }
    measurement.unknown_reason =
        Some("the package process was not present in dumpsys cpuinfo".to_string());
    measurement
}

async fn sample_proc_cpu(
    adb: &AdbClient,
    serial: &str,
    pid: u32,
    timeout: Duration,
) -> Result<(CpuMeasurement, f64, u64, u64)> {
    let ticks = shell(
        adb,
        serial,
        &["getconf".to_string(), "CLK_TCK".to_string()],
        timeout,
    )
    .await
    .ok()
    .and_then(|output| output.text.trim().parse::<f64>().ok())
    .filter(|ticks| *ticks > 0.0)
    .unwrap_or(100.0);
    let first = proc_cpu_ticks(adb, serial, pid, timeout).await?;
    let started = Instant::now();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let second = proc_cpu_ticks(adb, serial, pid, timeout).await?;
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    let user = (second.0.saturating_sub(first.0)) as f64 / ticks / elapsed * 100.0;
    let kernel = (second.1.saturating_sub(first.1)) as f64 / ticks / elapsed * 100.0;
    Ok((
        CpuMeasurement {
            process: Some(format!("pid {pid}")),
            total_percent: Some(user + kernel),
            user_percent: Some(user),
            kernel_percent: Some(kernel),
            unknown_reason: None,
        },
        ticks,
        first.0.saturating_add(first.1),
        second.0.saturating_add(second.1),
    ))
}

async fn proc_cpu_ticks(
    adb: &AdbClient,
    serial: &str,
    pid: u32,
    timeout: Duration,
) -> Result<(u64, u64)> {
    let output = shell(
        adb,
        serial,
        &["cat".to_string(), format!("/proc/{pid}/stat")],
        timeout,
    )
    .await?;
    parse_proc_stat(&output.text).ok_or_else(|| {
        ForgeError::new(
            ErrorCode::CommandFailed,
            format!("could not parse /proc/{pid}/stat"),
        )
        .with_context_string("output", excerpt(&output.text, 1200))
        .device(serial.to_string())
    })
}

fn parse_proc_stat(text: &str) -> Option<(u64, u64)> {
    let rest = text.rsplit_once(')')?.1.trim();
    let fields = rest.split_whitespace().collect::<Vec<_>>();
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    Some((utime, stime))
}
fn normalize_component(package: &str, activity: &str) -> String {
    if activity.contains('/') {
        activity.to_string()
    } else {
        format!("{package}/{activity}")
    }
}

struct ShellOutput {
    text: String,
}

async fn shell(
    adb: &AdbClient,
    serial: &str,
    args: &[String],
    timeout: Duration,
) -> Result<ShellOutput> {
    let output = adb
        .shell_args(serial, args, Some(timeout), Some(4 * 1024 * 1024))
        .await?;
    Ok(ShellOutput {
        text: output.combined_text(),
    })
}

fn command(
    name: &str,
    command_text: &str,
    output: &ShellOutput,
    excerpt_limit: usize,
) -> PerformanceCommand {
    PerformanceCommand {
        name: name.to_string(),
        command: command_text.to_string(),
        status: "ok".to_string(),
        excerpt: Some(excerpt(&output.text, excerpt_limit)),
        reason: None,
    }
}

fn value_after(text: &str, prefix: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix(prefix)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn millis_after(text: &str, prefix: &str) -> Option<u64> {
    value_after(text, prefix)?
        .trim_end_matches("ms")
        .trim()
        .parse()
        .ok()
}

fn percentile(text: &str, percentile: u32) -> Option<f64> {
    let pattern = format!(
        r"(?m)^\s*{}(?:st|nd|rd|th) percentile:\s*([\d.]+)\s*ms",
        percentile
    );
    Regex::new(&pattern)
        .ok()?
        .captures(text)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

fn capture_u64(pattern: &str, text: &str) -> Option<u64> {
    Regex::new(pattern)
        .ok()?
        .captures(text)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

fn excerpt(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let mut result = trimmed.chars().take(limit).collect::<String>();
    result.push_str("\n[truncated]");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const AM_START: &str = "\
Starting: Intent { cmp=com.example/.MainActivity }
Status: ok
LaunchState: COLD
Activity: com.example/.MainActivity
TotalTime: 842
WaitTime: 855
ThisTime: 410
";

    const GFXINFO: &str = "\
Applications Graphics Acceleration Info:
Uptime: 123456 Realtime: 123456
** Graphics info for pid 1234 [com.example] **
Total frames rendered: 120
Janky frames: 9 (7.50%)
50th percentile: 8ms
90th percentile: 16ms
95th percentile: 24ms
99th percentile: 40ms
Number Missed Vsync: 3
";

    const MEMINFO: &str = "\
** MEMINFO in pid 1234 [com.example] **
                   Pss  Private  Private  SwapPss      Rss
         Java Heap:   12345    10000        0        0    20000
       Native Heap:   23456    20000        0        0    30000
           TOTAL:  456789   400000        0        0   500000
 TOTAL PSS:   456789            TOTAL RSS:   500000       TOTAL SWAP PSS:    123
";

    const CPUINFO: &str = "\
Load: 1.2 / 1.1 / 1.0
CPU usage from 1000ms to 0ms ago:
  12.5% 1234/com.example: 8.0% user + 4.5% kernel + 0.0% iowait
   2.0% 5678/com.example:remote: 1.0% user + 1.0% kernel
   1.0% 999/system_server: 0.5% user + 0.5% kernel
";

    #[test]
    fn parses_am_start_timings() {
        let startup = parse_am_start(AM_START);
        assert_eq!(startup.status.as_deref(), Some("ok"));
        assert_eq!(startup.total_time_ms, Some(842));
        assert_eq!(startup.wait_time_ms, Some(855));
        assert_eq!(startup.this_time_ms, Some(410));
        assert!(startup.unknown_reason.is_none());
    }

    #[test]
    fn missing_startup_time_is_explicitly_unknown() {
        let startup = parse_am_start("Status: ok\n");
        assert_eq!(startup.total_time_ms, None);
        assert!(
            startup
                .unknown_reason
                .as_deref()
                .unwrap()
                .contains("TotalTime")
        );
    }

    #[test]
    fn parses_gfxinfo_frames_and_percentiles() {
        let jank = parse_gfxinfo(GFXINFO);
        assert_eq!(jank.total_frames, Some(120));
        assert_eq!(jank.janky_frames, Some(9));
        assert_eq!(jank.janky_percent, Some(7.5));
        assert_eq!(jank.percentile_90_ms, Some(16.0));
        assert_eq!(jank.percentile_99_ms, Some(40.0));
        assert_eq!(jank.missed_vsync, Some(3));
        assert!(jank.unknown_reason.is_none());
    }

    #[test]
    fn empty_gfxinfo_is_unknown() {
        let jank = parse_gfxinfo("No process found for com.example\n");
        assert_eq!(jank.total_frames, None);
        assert!(jank.unknown_reason.is_some());
    }

    #[test]
    fn parses_meminfo_totals_and_heaps() {
        let memory = parse_meminfo(MEMINFO);
        assert_eq!(memory.pid, Some(1234));
        assert_eq!(memory.total_pss_kb, Some(456789));
        assert_eq!(memory.total_rss_kb, Some(500000));
        assert_eq!(memory.java_heap_pss_kb, Some(12345));
        assert_eq!(memory.native_heap_pss_kb, Some(23456));
        assert!(memory.unknown_reason.is_none());
    }

    #[test]
    fn parses_exact_and_remote_cpu_processes() {
        let cpu = parse_cpuinfo(CPUINFO, "com.example");
        assert_eq!(cpu.process.as_deref(), Some("com.example"));
        assert_eq!(cpu.total_percent, Some(12.5));
        assert_eq!(cpu.user_percent, Some(8.0));
        assert_eq!(cpu.kernel_percent, Some(4.5));
    }

    #[test]
    fn parses_proc_stat_cpu_ticks() {
        let line =
            "1234 (com.example app) S 1 1234 0 0 -1 4194560 100 0 0 0 42 17 0 0 20 0 12 0 100 0 0";
        assert_eq!(parse_proc_stat(line), Some((42, 17)));
        assert_eq!(parse_proc_stat("not a stat line"), None);
    }

    #[test]
    fn unknown_cpu_process_is_not_guessed() {
        let cpu = parse_cpuinfo(CPUINFO, "com.other");
        assert_eq!(cpu.total_percent, None);
        assert!(cpu.unknown_reason.is_some());
    }
}
