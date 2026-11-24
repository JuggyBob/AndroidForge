//! Diagnostics, screenshots, recordings and evidence collection.

mod alarms;
mod capture;
mod collect;
mod device_state;
mod health;
mod jobs;
mod logcat;
mod notifications;
mod performance;

pub use alarms::{AlarmInfo, Alarms, NextAlarms};
pub use capture::{
    FrameResult, Recorder, RecordingOutcome, RecordingState, RecordingStatus, ScreenshotResult,
    capture_frame, capture_screenshot,
};
pub use collect::{CollectionResult, DiagnosticDump, capture_failure_bundle, collect_diagnostics};
pub use device_state::{
    BatteryState, DeviceStateManager, DozeState, NetworkState, ProxyState, StandbyState, TimeState,
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
    CpuMeasurement, JankMeasurement, MemoryMeasurement, PERFORMANCE_SCHEMA_VERSION,
    PerformanceCommand, PerformanceOptions, PerformanceReport, StartupMeasurement,
    collect_performance, parse_am_start, parse_cpuinfo, parse_gfxinfo, parse_meminfo,
};
