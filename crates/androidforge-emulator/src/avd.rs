use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::toolchain::Toolchain;
use androidforge_process::{CommandSpec, Runner};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvdInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_mb: Option<u32>,
}

pub async fn list_avds(toolchain: &Toolchain, trace: bool) -> Result<Vec<AvdInfo>> {
    let configs = read_avd_configs(toolchain);
    if let Some(emulator) = &toolchain.emulator {
        let runner = Runner::new(trace);
        let spec = CommandSpec::new(emulator.to_string_lossy().into_owned())
            .args(["-list-avds"])
            .timeout(Duration::from_secs(20))
            .trace(trace)
            .hide_window(true);
        if let Ok(output) = runner.run(spec).await
            && output.success
        {
            let names = output
                .stdout_text()
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with("INFO"))
                .map(str::to_string)
                .collect::<Vec<_>>();
            if !names.is_empty() {
                return Ok(names
                    .into_iter()
                    .map(|name| {
                        configs.get(&name).cloned().unwrap_or_else(|| AvdInfo {
                            name: name.clone(),
                            path: toolchain.avd_home.as_ref().map(|home| {
                                home.join(format!("{name}.avd"))
                                    .to_string_lossy()
                                    .into_owned()
                            }),
                            target: None,
                            abi: None,
                            device: None,
                            tag: None,
                            ram_mb: None,
                        })
                    })
                    .collect());
            }
        }
    }
    if configs.is_empty() {
        return Err(
            ForgeError::new(ErrorCode::EmulatorNotFound, "no Android AVDs were found")
                .with_suggestion("Create an AVD with Android Studio or `avdmanager`.")
                .with_suggestion(
                    "Run `androidforge doctor` to inspect emulator and AVD discovery.",
                ),
        );
    }
    let mut avds = configs.into_values().collect::<Vec<_>>();
    avds.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(avds)
}

fn read_avd_configs(toolchain: &Toolchain) -> BTreeMap<String, AvdInfo> {
    let Some(home) = &toolchain.avd_home else {
        return BTreeMap::new();
    };
    let mut avds = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(home) else {
        return avds;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("ini") {
            continue;
        }
        let name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let ini = std::fs::read_to_string(&path).unwrap_or_default();
        let avd_path = parse_ini(&ini)
            .get("path")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(format!("{name}.avd")));
        let config_path = avd_path.join("config.ini");
        let config = std::fs::read_to_string(&config_path)
            .ok()
            .map(|text| parse_ini(&text))
            .unwrap_or_default();
        let target = parse_ini(&ini).get("target").cloned();
        avds.insert(
            name.clone(),
            AvdInfo {
                name,
                path: Some(avd_path.to_string_lossy().into_owned()),
                target,
                abi: config.get("abi.type").cloned(),
                device: config.get("hw.device.name").cloned(),
                tag: config.get("tag.id").cloned(),
                ram_mb: config
                    .get("hw.ramSize")
                    .and_then(|value| value.parse::<u32>().ok()),
            },
        );
    }
    avds
}

fn parse_ini(text: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            values.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ini() {
        let values =
            parse_ini("avd.ini.encoding=UTF-8\npath=C:\\Users\\me\\.android\\avd\\Pixel.avd\n");
        assert_eq!(values.get("avd.ini.encoding").unwrap(), "UTF-8");
        assert!(values.get("path").unwrap().contains("Pixel.avd"));
    }
}
