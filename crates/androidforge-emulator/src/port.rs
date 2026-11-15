use std::net::TcpListener;

use androidforge_core::error::{ErrorCode, ForgeError, Result};

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
    if !(MIN_REQUESTED_PORT..=MAX_REQUESTED_PORT).contains(&port) || port % 2 != 0 {
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
}
