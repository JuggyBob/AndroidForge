//! Applications, APKs, permissions, AppOps and app-private files.

mod apk;
mod appops;
mod files;
mod install;
mod package;
mod permissions;

pub use apk::{
    ApkInfo, ApkInspector, ApkSignatureStatus, ApkVerificationReport, VerifyCheck, VerifyOptions,
};
pub use appops::{AppOpState, AppOps};
pub use files::{AppFileTransfer, AppFiles};
pub use install::{InstallOptions, InstallResult, InstalledPackage, SignatureDiagnosis};
pub use package::{
    AppLaunch, AppManager, AppStateExpectation, AppStates, PackageInfo, ProcessState,
};
pub use permissions::{PermissionState, Permissions};
