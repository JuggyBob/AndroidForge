//! Diagnostics, screenshots, recordings and evidence collection.

mod alarms;
mod capture;
mod collect;
mod device_state;
mod jobs;
mod logcat;
mod notifications;

pub use alarms::{AlarmInfo, Alarms, NextAlarms};
pub use capture::{
    FrameResult, Recorder, RecordingOutcome, RecordingState, RecordingStatus, ScreenshotResult,
    capture_frame, capture_screenshot,
};
pub use collect::{CollectionResult, DiagnosticDump, capture_failure_bundle, collect_diagnostics};
pub use device_state::{
    BatteryState, DeviceStateManager, DozeState, NetworkState, StandbyState, TimeState,
};
pub use jobs::{JobInfo, JobList, JobRunResult, JobSelector, JobWaitResult, Jobs};
pub use logcat::{
    CrashEvent, CrashReport, LogcatCapture, LogcatPeek, LogcatResult, LogcatStatus,
    LogcatStopResult, LogcatStream, collect_logcat_peek, collect_logcat_tail, detect_crashes,
};
pub use notifications::{NotificationInfo, NotificationList, NotificationSelector, Notifications};
