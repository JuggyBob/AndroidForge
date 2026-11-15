use std::env;
use std::path::{Path, PathBuf};

use crate::config::ForgeConfig;
use crate::error::{ErrorCode, ForgeError, Result};

#[derive(Debug, Clone, Default)]
pub struct Toolchain {
    pub sdk_root: Option<PathBuf>,
    pub adb: Option<PathBuf>,
    pub emulator: Option<PathBuf>,
    pub build_tools: Option<PathBuf>,
    pub aapt2: Option<PathBuf>,
    pub apksigner: Option<PathBuf>,
    pub zipalign: Option<PathBuf>,
    pub apkanalyzer: Option<PathBuf>,
    pub java: Option<PathBuf>,
    pub maestro: Option<PathBuf>,
    pub avd_home: Option<PathBuf>,
}

impl Toolchain {
    pub fn discover(config: &ForgeConfig) -> Self {
        let sdk_root = discover_sdk_root(config);
        let adb = first_existing([
            config.file.sdk.adb.clone(),
            env_path("ANDROIDFORGE_ADB"),
            sdk_root
                .as_ref()
                .map(|root| root.join("platform-tools").join(platform_exe("adb"))),
            which("adb"),
        ]);
        let adb = adb.or_else(|| {
            sdk_root
                .as_ref()
                .and_then(|root| {
                    derive_sdk_root_from_adb(&root.join("platform-tools").join(platform_exe("adb")))
                })
                .and_then(|root| {
                    let path = root.join("platform-tools").join(platform_exe("adb"));
                    path.is_file().then_some(path)
                })
        });

        let emulator = first_existing([
            config.file.sdk.emulator.clone(),
            env_path("ANDROIDFORGE_EMULATOR"),
            sdk_root
                .as_ref()
                .map(|root| root.join("emulator").join(platform_exe("emulator"))),
            which("emulator"),
        ]);

        let build_tools = discover_build_tools(config, sdk_root.as_deref());
        let aapt2 = first_existing([
            config.file.sdk.aapt2.clone(),
            env_path("ANDROIDFORGE_AAPT2"),
            build_tools
                .as_ref()
                .map(|dir| dir.join(platform_exe("aapt2"))),
            which("aapt2"),
        ]);
        // On Windows Build-Tools ship apksigner.bat plus lib/apksigner.jar.
        // Prefer the jar so AndroidForge can invoke Java directly instead of
        // routing an internal operation through cmd.exe.
        let apksigner = first_existing([
            config.file.sdk.apksigner.clone(),
            env_path("ANDROIDFORGE_APKSIGNER"),
            build_tools
                .as_ref()
                .map(|dir| dir.join(platform_exe("apksigner"))),
            build_tools
                .as_ref()
                .map(|dir| dir.join("lib").join("apksigner.jar")),
            which("apksigner"),
        ]);
        let zipalign = first_existing([
            config.file.sdk.zipalign.clone(),
            build_tools
                .as_ref()
                .map(|dir| dir.join(platform_exe("zipalign"))),
            which("zipalign"),
        ]);
        let apkanalyzer = first_existing([
            config.file.sdk.apkanalyzer.clone(),
            sdk_root.as_ref().map(|root| {
                root.join("cmdline-tools")
                    .join("latest")
                    .join("bin")
                    .join(platform_exe("apkanalyzer"))
            }),
            which("apkanalyzer"),
        ]);
        let java = first_existing([
            config.file.sdk.java.clone(),
            sdk_root
                .as_ref()
                .map(|root| root.join("jbr").join("bin").join(platform_exe("java"))),
            sdk_root.as_ref().and_then(|root| find_sdk_java(root)),
            env_path("JAVA_HOME").map(|home| home.join("bin").join(platform_exe("java"))),
            which("java"),
        ]);
        let maestro = first_existing([
            config.file.sdk.maestro.clone(),
            env_path("ANDROIDFORGE_MAESTRO"),
            which(&config.file.maestro.binary),
            env_path("USERPROFILE").map(|home| {
                home.join(".maestro")
                    .join("bin")
                    .join(platform_exe("maestro"))
            }),
            env_path("HOME").map(|home| {
                home.join(".maestro")
                    .join("bin")
                    .join(platform_exe("maestro"))
            }),
            which("maestro"),
        ]);

        let avd_home = env_path("ANDROID_AVD_HOME")
            .or_else(|| env_path("USERPROFILE").map(|home| home.join(".android").join("avd")))
            .or_else(|| env_path("HOME").map(|home| home.join(".android").join("avd")));

        Self {
            sdk_root,
            adb,
            emulator,
            build_tools,
            aapt2,
            apksigner,
            zipalign,
            apkanalyzer,
            java,
            maestro,
            avd_home,
        }
    }

    pub fn require_adb(&self) -> Result<PathBuf> {
        self.adb.clone().ok_or_else(|| {
            ForgeError::new(ErrorCode::AdbNotFound, "ADB was not found")
                .with_suggestion("Install Android SDK Platform-Tools.")
                .with_suggestion("Set ANDROID_HOME or ANDROIDFORGE_ADB to the adb executable.")
                .with_suggestion("Run `androidforge doctor` to inspect tool discovery.")
        })
    }

    pub fn require_emulator(&self) -> Result<PathBuf> {
        self.emulator.clone().ok_or_else(|| {
            ForgeError::new(
                ErrorCode::EmulatorNotFound,
                "the Android Emulator executable was not found",
            )
            .with_suggestion("Install the Android SDK Emulator package.")
            .with_suggestion("Set ANDROID_HOME or ANDROIDFORGE_EMULATOR.")
            .with_suggestion("Run `androidforge doctor` to inspect tool discovery.")
        })
    }

    pub fn require_aapt2(&self) -> Result<PathBuf> {
        self.aapt2.clone().ok_or_else(|| {
            ForgeError::new(ErrorCode::ToolNotFound, "AAPT2 was not found")
                .with_suggestion("Install Android SDK Build-Tools (34.0.0 or newer).")
                .with_suggestion(
                    "Set [sdk].aapt2 in AndroidForge configuration if it is in a custom location.",
                )
                .with_suggestion("Run `androidforge doctor` to inspect tool discovery.")
        })
    }

    pub fn require_apksigner(&self) -> Result<PathBuf> {
        self.apksigner.clone().ok_or_else(|| {
            ForgeError::new(ErrorCode::ToolNotFound, "APK Signer was not found")
                .with_suggestion("Install Android SDK Build-Tools (34.0.0 or newer).")
                .with_suggestion("Set [sdk].apksigner in AndroidForge configuration if it is in a custom location.")
                .with_suggestion("Run `androidforge doctor` to inspect tool discovery.")
        })
    }

    pub fn require_java(&self) -> Result<PathBuf> {
        self.java.clone().ok_or_else(|| {
            ForgeError::new(ErrorCode::ToolNotFound, "Java was not found")
                .with_suggestion("Install a JDK (Android Studio bundles one).")
                .with_suggestion("Set [sdk].java or JAVA_HOME.")
        })
    }

    pub fn require_maestro(&self) -> Result<PathBuf> {
        self.maestro.clone().ok_or_else(|| {
            ForgeError::new(ErrorCode::MaestroNotFound, "Maestro was not found")
                .with_suggestion("Maestro is optional. Install it from https://maestro.mobile.dev if you want to run Maestro flows.")
                .with_suggestion("Set [sdk].maestro or [maestro].binary in configuration if it is in a custom location.")
                .with_suggestion("Native `androidforge ui` commands do not require Maestro.")
        })
    }

    pub fn sdk_or_error(&self) -> Result<&Path> {
        self.sdk_root.as_deref().ok_or_else(|| {
            ForgeError::new(ErrorCode::SdkNotFound, "Android SDK was not found")
                .with_suggestion("Set ANDROID_HOME or ANDROID_SDK_ROOT.")
                .with_suggestion("Run `androidforge doctor` to inspect SDK discovery.")
        })
    }

    pub fn licenses_dir(&self) -> Option<PathBuf> {
        self.sdk_root.as_ref().map(|root| root.join("licenses"))
    }

    pub fn build_tools_versions(&self) -> Vec<String> {
        let Some(root) = &self.sdk_root else {
            return Vec::new();
        };
        let dir = root.join("build-tools");
        let mut versions = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    versions.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
        versions.sort_by_key(|left| version_key(left));
        versions.reverse();
        versions
    }
}

fn discover_sdk_root(config: &ForgeConfig) -> Option<PathBuf> {
    let configured = config.file.sdk.root.clone();
    first_existing_dir([
        configured,
        env_path("ANDROIDFORGE_SDK_ROOT"),
        env_path("ANDROID_HOME"),
        env_path("ANDROID_SDK_ROOT"),
        env_path("LOCALAPPDATA").map(|local| local.join("Android").join("Sdk")),
        env_path("USERPROFILE").map(|home| {
            home.join("AppData")
                .join("Local")
                .join("Android")
                .join("Sdk")
        }),
        env_path("HOME").map(|home| home.join("Android").join("Sdk")),
    ])
}

fn discover_build_tools(config: &ForgeConfig, sdk_root: Option<&Path>) -> Option<PathBuf> {
    if let Some(configured) = &config.file.sdk.build_tools {
        let path = Path::new(configured);
        if path.is_absolute() && path.is_dir() {
            return Some(path.to_path_buf());
        }
        if let Some(sdk) = sdk_root {
            let candidate = sdk.join("build-tools").join(configured);
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    let root = sdk_root?.join("build-tools");
    let mut versions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            if entry.path().join(platform_exe("aapt2")).is_file()
                || entry.path().join(platform_exe("apksigner")).is_file()
            {
                versions.push(entry.path());
            }
        }
    }
    versions.sort_by(|left, right| {
        version_key(&left.file_name().unwrap_or_default().to_string_lossy()).cmp(&version_key(
            &right.file_name().unwrap_or_default().to_string_lossy(),
        ))
    });
    versions.pop()
}

fn first_existing<const N: usize>(candidates: [Option<PathBuf>; N]) -> Option<PathBuf> {
    candidates.into_iter().flatten().find(|path| path.is_file())
}

fn first_existing_dir<const N: usize>(candidates: [Option<PathBuf>; N]) -> Option<PathBuf> {
    candidates.into_iter().flatten().find(|path| path.is_dir())
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

fn platform_exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn derive_sdk_root_from_adb(adb: &Path) -> Option<PathBuf> {
    // <sdk>/platform-tools/adb.exe
    adb.parent()?.parent().map(Path::to_path_buf)
}

fn find_sdk_java(sdk_root: &Path) -> Option<PathBuf> {
    let jdk = sdk_root.join("jdk");
    let direct = jdk.join("bin").join(platform_exe("java"));
    if direct.is_file() {
        return Some(direct);
    }
    let entries = std::fs::read_dir(&jdk).ok()?;
    let mut candidates = entries
        .flatten()
        .map(|entry| entry.path().join("bin").join(platform_exe("java")))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.pop()
}

fn version_key(value: &str) -> Vec<u64> {
    value
        .split(['.', '-'])
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

pub fn which(program: &str) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() || program.contains('\\') || program.contains('/') {
        return path.is_file().then(|| path.to_path_buf());
    }
    let path_var = env::var_os("PATH")?;
    let extensions: Vec<String> = if cfg!(windows) {
        let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let mut extensions = Vec::new();
        if Path::new(program).extension().is_some() {
            extensions.push(String::new());
        }
        for extension in pathext.split(';') {
            if !extension.is_empty() {
                extensions.push(extension.to_ascii_lowercase());
                extensions.push(extension.to_ascii_uppercase());
            }
        }
        extensions
    } else {
        vec![String::new()]
    };
    for directory in env::split_paths(&path_var) {
        for extension in &extensions {
            let candidate = directory.join(format!("{program}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_sort_is_numeric() {
        assert!(version_key("36.0.0") > version_key("9.0.0"));
        assert!(version_key("34.0.0") < version_key("35.0.0"));
    }

    #[test]
    fn derives_sdk_root_from_platform_tools() {
        let root = derive_sdk_root_from_adb(Path::new("C:/Sdk/platform-tools/adb.exe")).unwrap();
        assert_eq!(root, PathBuf::from("C:/Sdk"));
    }
}
