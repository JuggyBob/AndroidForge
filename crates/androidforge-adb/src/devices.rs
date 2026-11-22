use std::collections::BTreeMap;

use androidforge_core::model::{Device, DeviceKind, DeviceState};

/// Parse `adb devices -l` without assuming the first line or the first device is
/// the target. Unknown states are preserved rather than treated as success.
pub fn parse_devices_output(output: &str) -> Vec<Device> {
    let mut devices = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("List of devices attached")
            || line.starts_with("* daemon")
            || line.starts_with("adb server")
            || line.starts_with("error:")
            || line.starts_with("adb:")
        {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(serial) = parts.next() else {
            continue;
        };
        let Some(state_token) = parts.next() else {
            continue;
        };
        let mut state_text = state_token.to_string();
        if state_text == "no" {
            let mut rest = Vec::new();
            for part in parts.by_ref() {
                if part.contains(':') {
                    rest.push(part);
                } else {
                    state_text.push(' ');
                    state_text.push_str(part);
                }
            }
            let mut device = Device::basic(serial, DeviceState::Unknown);
            device.product = None;
            device.boot_state = Some(state_text);
            // Attribute tokens after "no permissions (...)" are still useful.
            for token in rest {
                apply_attribute(&mut device, token);
            }
            devices.push(device);
            continue;
        }

        let state = DeviceState::from_adb(&state_text);
        let mut device = Device::basic(serial, state);
        for token in parts {
            apply_attribute(&mut device, token);
        }
        device.kind = kind_for(&device);
        devices.push(device);
    }
    devices
}

fn apply_attribute(device: &mut Device, token: &str) {
    let Some((key, value)) = token.split_once(':') else {
        return;
    };
    match key {
        "model" => device.model = Some(value.to_string()),
        "product" => device.product = Some(value.to_string()),
        "device" => {
            if device.model.is_none() {
                device.model = Some(value.to_string());
            }
        }
        "transport_id" => device.transport_id = Some(value.to_string()),
        _ => {}
    }
}

fn kind_for(device: &Device) -> DeviceKind {
    // Only the adb emulator transport is accepted as positive proof. Model
    // strings are vendor-controlled and must not be able to downgrade a
    // physical device into the unprotected emulator class.
    if device.serial.starts_with("emulator-") {
        DeviceKind::Emulator
    } else {
        DeviceKind::Physical
    }
}

/// Parse the output of `adb shell getprop`. Values may contain spaces, brackets
/// and other punctuation.
pub fn parse_getprop_output(output: &str) -> BTreeMap<String, String> {
    let mut properties = BTreeMap::new();
    for line in output.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((key, value)) = rest.split_once("]: [") else {
            continue;
        };
        let value = value.strip_suffix(']').unwrap_or(value);
        properties.insert(key.to_string(), value.to_string());
    }
    properties
}

/// Return the effective display size (override wins) from `wm size`.
pub fn parse_wm_size(output: &str) -> Option<(u32, u32)> {
    let mut physical = None;
    let mut override_size = None;
    for line in output.lines() {
        let Some((label, value)) = line.split_once(':') else {
            continue;
        };
        let Some((width, height)) = parse_dimensions(value.trim()) else {
            continue;
        };
        if label.trim().eq_ignore_ascii_case("override size") {
            override_size = Some((width, height));
        } else if label.trim().eq_ignore_ascii_case("physical size") {
            physical = Some((width, height));
        }
    }
    override_size.or(physical)
}

/// Return the effective display density (override wins) from `wm density`.
pub fn parse_wm_density(output: &str) -> Option<u32> {
    let mut physical = None;
    let mut override_density = None;
    for line in output.lines() {
        let Some((label, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let Ok(density) = value.parse::<u32>() else {
            continue;
        };
        if label.trim().eq_ignore_ascii_case("override density") {
            override_density = Some(density);
        } else if label.trim().eq_ignore_ascii_case("physical density") {
            physical = Some(density);
        }
    }
    override_density.or(physical)
}

fn parse_dimensions(value: &str) -> Option<(u32, u32)> {
    let (width, height) = value.split_once('x')?;
    let width = width.trim().parse::<u32>().ok()?;
    let height = height.trim().parse::<u32>().ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

pub fn boot_completed(properties: &BTreeMap<String, String>) -> bool {
    properties
        .get("sys.boot_completed")
        .map(|value| value == "1")
        .unwrap_or(false)
        || properties
            .get("dev.bootcomplete")
            .map(|value| value == "1")
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
* daemon not running; starting now at tcp:5037
* daemon started successfully
List of devices attached
emulator-5554          device product:sdk_gphone64_x86_64 model:sdk_gphone64_x86_64 device:emu64xa transport_id:1
R58M123456             unauthorized usb:1-1 product:beyond1q model:SM_G973F device:beyond1 transport_id:2
emulator-5556          offline transport_id:3

"#;

    #[test]
    fn parses_multiple_device_states() {
        let devices = parse_devices_output(SAMPLE);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].serial, "emulator-5554");
        assert_eq!(devices[0].state, DeviceState::Device);
        assert_eq!(devices[0].kind, DeviceKind::Emulator);
        assert_eq!(devices[0].model.as_deref(), Some("sdk_gphone64_x86_64"));
        assert_eq!(devices[0].transport_id.as_deref(), Some("1"));
        assert_eq!(devices[1].state, DeviceState::Unauthorized);
        assert_eq!(devices[1].kind, DeviceKind::Physical);
        assert_eq!(devices[2].state, DeviceState::Offline);
    }

    #[test]
    fn ignores_daemon_noise_and_empty_output() {
        assert!(parse_devices_output("List of devices attached\n\n").is_empty());
        assert!(parse_devices_output("* daemon started successfully\n").is_empty());
    }

    #[test]
    fn parses_no_permissions_state() {
        let devices =
            parse_devices_output("abcd no permissions (user in plugdev group); see [http://x]\n");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].state, DeviceState::Unknown);
        assert!(
            devices[0]
                .boot_state
                .as_deref()
                .unwrap()
                .contains("no permissions")
        );
    }

    #[test]
    fn parses_getprop_output() {
        let props = parse_getprop_output(
            "[ro.product.model]: [Pixel 8]\n[ro.build.version.sdk]: [35]\n[weird]: [a]: [b]]\n",
        );
        assert_eq!(props.get("ro.product.model").unwrap(), "Pixel 8");
        assert_eq!(props.get("ro.build.version.sdk").unwrap(), "35");
        assert_eq!(props.get("weird").unwrap(), "a]: [b]");
    }

    #[test]
    fn wm_override_wins() {
        let size = parse_wm_size("Physical size: 1080x2400\nOverride size: 720x1600\n").unwrap();
        assert_eq!(size, (720, 1600));
        assert_eq!(
            parse_wm_size("Physical size: 1080x2400").unwrap(),
            (1080, 2400)
        );
        assert_eq!(
            parse_wm_density("Physical density: 420\nOverride density: 560\n").unwrap(),
            560
        );
        assert_eq!(parse_wm_size("garbage"), None);
    }

    #[test]
    fn boot_props() {
        let mut props = BTreeMap::new();
        assert!(!boot_completed(&props));
        props.insert("sys.boot_completed".to_string(), "1".to_string());
        assert!(boot_completed(&props));
    }
}
