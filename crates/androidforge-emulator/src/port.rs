use std::collections::BTreeMap;
use std::net::TcpListener;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use serde::{Deserialize, Serialize};

pub const MIN_CONSOLE_PORT: u16 = 5554;
pub const MAX_CONSOLE_PORT: u16 = 5682;
/// The emulator documents 5554-5584, but current builds accept other even
/// console ports. Explicit requests are allowed outside the conventional range;
/// automatic allocation still starts at 5554.
pub const MIN_REQUESTED_PORT: u16 = 1024;
pub const MAX_REQUESTED_PORT: u16 = 65532;
pub const GRPC_PORT_OFFSET: u16 = 3000;

#[derive(Debug)]
pub struct PortReservation {
    pub console_port: u16,
    listeners: Vec<TcpListener>,
}

impl PortReservation {
    pub fn serial(&self) -> String {
        format!("emulator-{}", self.console_port)
    }

    pub fn adb_port(&self) -> u16 {
        self.console_port + 1
    }

    pub fn grpc_port(&self) -> Option<u16> {
        self.console_port.checked_add(GRPC_PORT_OFFSET)
    }

    pub fn release(self) {
        drop(self.listeners);
    }
}

/// Reserve a valid emulator console port pair. The emulator's console port must
/// be even; ADB uses the next odd port. The gRPC port (console + 3000) is also
/// checked because modern emulators bind it on startup.
pub fn allocate_port(requested: Option<u16>) -> Result<PortReservation> {
    if let Some(port) = requested {
        validate_port(port)?;
        return reserve(port);
    }
    let mut last_error = None;
    for port in (MIN_CONSOLE_PORT..=MAX_CONSOLE_PORT).step_by(2) {
        match reserve(port) {
            Ok(reservation) => return Ok(reservation),
            Err(error) => last_error = Some(error),
        }
    }
    let mut error = ForgeError::new(
        ErrorCode::DeviceBusy,
        "no free Android emulator console port was found",
    )
    .with_suggestion(format!(
        "Ports {MIN_CONSOLE_PORT}-{MAX_CONSOLE_PORT} are reserved for emulators."
    ))
    .with_suggestion(
        "Stop an unused emulator or run `androidforge session list` to inspect sessions.",
    );
    if let Some(last) = last_error {
        error = error.with_context_string("last_port_error", last.message());
    }
    Err(error)
}

fn validate_port(port: u16) -> Result<()> {
    if !(MIN_REQUESTED_PORT..=MAX_REQUESTED_PORT).contains(&port) || !port.is_multiple_of(2) {
        return Err(ForgeError::invalid_argument(format!(
            "emulator console port must be an even number between {MIN_REQUESTED_PORT} and {MAX_REQUESTED_PORT}; got {port}"
        ))
        .with_suggestion(
            "The conventional range is 5554-5682; for example --port 5556 gives emulator-5556.",
        ));
    }
    Ok(())
}

fn reserve(port: u16) -> Result<PortReservation> {
    let mut listeners = Vec::new();
    for candidate in [port, port + 1] {
        match TcpListener::bind(("0.0.0.0", candidate)) {
            Ok(listener) => listeners.push(listener),
            Err(error) => {
                return Err(ForgeError::new(
                    ErrorCode::DeviceBusy,
                    format!("port {candidate} is already in use: {error}"),
                )
                .with_suggestion(format!(
                    "Choose a different even --port; androidforge checked {port} and its ADB port {}.",
                    port + 1
                ))
                .with_suggestion("Run `androidforge doctor` or `netstat -ano` to inspect port usage."));
            }
        }
    }
    if let Some(grpc) = port.checked_add(GRPC_PORT_OFFSET) {
        // The gRPC listener is best-effort: some emulator versions do not bind
        // it, but when they do a collision makes startup fail.
        if let Ok(listener) = TcpListener::bind(("0.0.0.0", grpc)) {
            listeners.push(listener);
        }
    }
    Ok(PortReservation {
        console_port: port,
        listeners,
    })
}

/// Lifecycle state reported by `ports list`.
///
/// `allocated` means an AndroidForge session claims the port but no listener
/// was observed; `occupied` means a listener is present (and takes precedence
/// when both pieces of information exist).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortState {
    Free,
    Allocated,
    Occupied,
}

impl PortState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Allocated => "allocated",
            Self::Occupied => "occupied",
        }
    }
}

/// Session metadata supplied by the caller (AndroidForge session files).
/// The port module stays independent of session storage and never guesses an
/// owner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PortOwnerInfo {
    pub owner: Option<String>,
    pub session_id: Option<String>,
    pub avd: Option<String>,
}

impl PortOwnerInfo {
    pub fn has_metadata(&self) -> bool {
        self.owner.is_some() || self.session_id.is_some() || self.avd.is_some()
    }
}

/// Result of a bounded bind probe for one console port and its paired ports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PortProbe {
    pub console: bool,
    pub adb: bool,
    pub grpc: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortEntry {
    pub console_port: u16,
    pub adb_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grpc_port: Option<u16>,
    pub serial: String,
    pub state: PortState,
    pub console_occupied: bool,
    pub adb_occupied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grpc_occupied: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avd: Option<String>,
}

/// Probe a single port with a short-lived bind, the same primitive used by
/// `allocate_port`. A failed bind is reported as occupied; no process is killed
/// and no port is retained.
pub fn is_port_occupied(port: u16) -> bool {
    TcpListener::bind(("0.0.0.0", port)).is_err()
}

pub fn probe_port(console_port: u16) -> PortProbe {
    let grpc = console_port.checked_add(GRPC_PORT_OFFSET);
    PortProbe {
        console: is_port_occupied(console_port),
        adb: console_port.checked_add(1).is_some_and(is_port_occupied),
        grpc: grpc.is_some_and(is_port_occupied),
    }
}

/// Pure mapping from probe results and optional session metadata to a report
/// row. Keeping this separate from socket probing makes owner/state mapping
/// testable without a device or a particular host network state.
pub fn port_entry(console_port: u16, probe: PortProbe, owner: Option<&PortOwnerInfo>) -> PortEntry {
    let allocated = owner.is_some_and(PortOwnerInfo::has_metadata);
    let occupied = probe.console || probe.adb || probe.grpc;
    let state = if occupied {
        PortState::Occupied
    } else if allocated {
        PortState::Allocated
    } else {
        PortState::Free
    };
    PortEntry {
        console_port,
        adb_port: console_port.saturating_add(1),
        grpc_port: console_port.checked_add(GRPC_PORT_OFFSET),
        serial: format!("emulator-{console_port}"),
        state,
        console_occupied: probe.console,
        adb_occupied: probe.adb,
        grpc_occupied: console_port
            .checked_add(GRPC_PORT_OFFSET)
            .map(|_| probe.grpc),
        owner: owner.and_then(|info| info.owner.clone()),
        session_id: owner.and_then(|info| info.session_id.clone()),
        avd: owner.and_then(|info| info.avd.clone()),
    }
}

/// List console ports in the conventional automatic-allocation range.
/// Owner metadata is keyed by console port; callers build it from sessions.
pub fn list_ports(owners: &BTreeMap<u16, PortOwnerInfo>) -> Vec<PortEntry> {
    (MIN_CONSOLE_PORT..=MAX_CONSOLE_PORT)
        .step_by(2)
        .map(|port| port_entry(port, probe_port(port), owners.get(&port)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_odd_and_out_of_range_ports() {
        assert!(validate_port(5555).is_err());
        assert!(validate_port(1000).is_err());
        assert!(validate_port(5554).is_ok());
        assert!(validate_port(5540).is_ok());
        assert!(validate_port(65533).is_err());
    }

    #[test]
    fn serial_uses_console_port() {
        let reservation = allocate_port(Some(5556)).unwrap();
        assert_eq!(reservation.serial(), "emulator-5556");
        assert_eq!(reservation.adb_port(), 5557);
        reservation.release();
    }

    #[test]
    fn port_entry_maps_free_allocated_and_occupied_states() {
        let free = port_entry(5554, PortProbe::default(), None);
        assert_eq!(free.state, PortState::Free);
        assert_eq!(free.serial, "emulator-5554");
        assert_eq!(free.adb_port, 5555);
        assert_eq!(free.grpc_port, Some(8554));
        assert_eq!(free.grpc_occupied, Some(false));

        let owner = PortOwnerInfo {
            owner: Some("agent-a".to_string()),
            session_id: Some("ses-1".to_string()),
            avd: Some("Pixel_7".to_string()),
        };
        let allocated = port_entry(5556, PortProbe::default(), Some(&owner));
        assert_eq!(allocated.state, PortState::Allocated);
        assert_eq!(allocated.owner.as_deref(), Some("agent-a"));
        assert_eq!(allocated.session_id.as_deref(), Some("ses-1"));
        assert_eq!(allocated.avd.as_deref(), Some("Pixel_7"));

        let occupied = port_entry(
            5558,
            PortProbe {
                console: false,
                adb: true,
                grpc: false,
            },
            Some(&owner),
        );
        assert_eq!(occupied.state, PortState::Occupied);
        assert!(occupied.adb_occupied);
        assert_eq!(occupied.owner.as_deref(), Some("agent-a"));
    }

    #[test]
    fn port_entry_handles_grpc_overflow_without_fabricating_values() {
        let entry = port_entry(65532, PortProbe::default(), None);
        assert_eq!(entry.adb_port, 65533);
        assert_eq!(entry.grpc_port, None);
        assert_eq!(entry.grpc_occupied, None);
    }

    #[test]
    fn port_state_has_stable_serialized_names() {
        assert_eq!(PortState::Free.as_str(), "free");
        assert_eq!(PortState::Allocated.as_str(), "allocated");
        assert_eq!(PortState::Occupied.as_str(), "occupied");
        assert_eq!(
            serde_json::to_string(&PortState::Allocated).unwrap(),
            "\"allocated\""
        );
    }
}
