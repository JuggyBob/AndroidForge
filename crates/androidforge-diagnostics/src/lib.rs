//! Diagnostics, screenshots, recordings and evidence collection.

mod alarms;
mod capture;
mod capture_launch;
mod collect;
mod device_state;
mod frames;
mod health;
mod jobs;
mod logcat;
mod notifications;
mod performance;
mod secrets;

pub use alarms::{AlarmInfo, Alarms, NextAlarms};
pub use capture::{
    FrameResult, Recorder, RecordingOutcome, RecordingState, RecordingStatus, ScreenshotResult,
    capture_frame, capture_screenshot,
};
pub use capture_launch::{
    ABSOLUTE_MAX_FRAME_BYTES, ABSOLUTE_MAX_FRAMES, ABSOLUTE_MAX_TOTAL_ARTIFACT_BYTES,
    AdbLaunchFrameSource, CAPTURE_LAUNCH_SCHEMA_VERSION, CaptureLaunchOptions, ColorSample,
    ColorSampleKind, DEFAULT_DURATION_MS, DEFAULT_FPS, DEFAULT_FRAME_TIMEOUT_MS,
    DEFAULT_MAX_FRAME_BYTES, DEFAULT_MAX_FRAMES, DEFAULT_MAX_TOTAL_ARTIFACT_BYTES,
    DEFAULT_TOLERANCE, FirstFrameAssertion, FirstFrameExpectation, FrameAnalysisInput,
    LaunchAnalysis, LaunchArtifactSink, LaunchCancellation, LaunchCaptureResult,
    LaunchCaptureStatus, LaunchFrame, LaunchFrameArtifact, LaunchFrameSource, MAX_DURATION_MS,
    MAX_FPS, MAX_FRAME_TIMEOUT_MS, MIN_DURATION_MS, MIN_FPS, analyze_launch_frames,
    assert_first_frame, sample_launch_capture,
};
pub use collect::{CollectionResult, DiagnosticDump, capture_failure_bundle, collect_diagnostics};
pub use device_state::{
    BatteryState, DeviceStateManager, DozeState, NetworkState, ProxyState, StandbyState, TimeState,
};
pub use frames::{
    FRAMES_SCHEMA_VERSION, FramesComparison, FramesDeltaStatus, FramesHistogramBucket,
    FramesMetricDelta, FramesReport, FramesSample, FramesThresholds, FramesWindow,
    compare_frames_reports, compare_frames_reports_default, frames_report_from_gfxinfo,
    parse_frames_gfxinfo, parse_frames_gfxinfo_with, parse_frames_histogram, parse_frames_window,
};
pub use health::{
    BatteryHealth, DeviceHealth, HealthEvidence, HealthThresholds, MemoryHealth, NetworkHealth,
    ScreenHealth, StorageHealth, ThermalHealth, collect_health,
};
pub use jobs::{JobInfo, JobList, JobRunResult, JobSelector, JobWaitResult, Jobs};
pub use logcat::{
    CrashEvent, CrashReport, LogcatCapture, LogcatPeek, LogcatRecord, LogcatResult, LogcatStatus,
    LogcatStopResult, LogcatStream, collect_logcat_peek, collect_logcat_tail, detect_crashes,
    logcat_line_matches_package, parse_threadtime_line,
};
pub use notifications::{NotificationInfo, NotificationList, NotificationSelector, Notifications};
pub use performance::{
    CpuMeasurement, DEFAULT_MIN_FRAMES, JankMeasurement, MemoryMeasurement,
    PERFORMANCE_SCHEMA_VERSION, PerformanceCommand, PerformanceOptions, PerformanceReport,
    StartupMeasurement, collect_performance, parse_am_start, parse_cpuinfo, parse_gfxinfo,
    parse_meminfo,
};
pub use secrets::{
    SECRET_SCAN_MAX_BYTES, SECRET_SCAN_MAX_FINDINGS, SecretFinding, SecretKind, scan_secrets,
};
