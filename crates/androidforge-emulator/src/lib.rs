//! Android emulator, AVD, port and device-preparation management.

mod avd;
mod manager;
mod port;
mod prepare;

pub use avd::{AvdInfo, list_avds};
pub use manager::{EmulatorManager, EmulatorStatus, SnapshotInfo, StartedEmulator, StopResult};
pub use port::{PortReservation, allocate_port};
pub use prepare::{
    DevicePrepareOptions, DevicePreparer, PrepareChange, PrepareResult, RestoreResult,
};
