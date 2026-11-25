//! Android emulator, AVD, port and device-preparation management.

mod avd;
mod manager;
mod port;
mod prepare;

pub use avd::{
    AvdInfo, CreateAvdOptions, CreateAvdRequest, CreateAvdResult, SystemImageInfo, create_avd,
    create_avd_args, find_avdmanager, list_avds, list_system_images, parse_system_image_parts,
    validate_avd_name, validate_device_id, validate_image_id,
};
pub use manager::{
    CapacitySnapshot, EmulatorManager, EmulatorStatus, SnapshotInfo, StartedEmulator,
    StatusContext, StatusMetadata, StopResult, capacity_warning, console_port_from_serial,
    status_metadata,
};
pub use port::{
    PortEntry, PortOwnerInfo, PortProbe, PortReservation, PortState, allocate_port,
    is_port_occupied, list_ports, port_entry, probe_port,
};
pub use prepare::{
    DevicePrepareOptions, DevicePreparer, PrepareChange, PrepareResult, RestoreResult,
};
