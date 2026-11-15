use std::env;
use std::path::{Path, PathBuf};

use crate::error::{ForgeError, Result};
use crate::util::canonicalize_or_self;

#[derive(Debug, Clone)]
pub struct ForgePaths {
    pub cwd: PathBuf,
    pub project_root: PathBuf,
    pub project_config_path: Option<PathBuf>,
    pub global_config_path: PathBuf,
    pub global_dir: PathBuf,
    pub state_dir: PathBuf,
    pub artifacts_dir: PathBuf,
}

impl ForgePaths {
    pub fn discover(cwd: Option<PathBuf>, project_override: Option<PathBuf>) -> Result<Self> {
        let cwd = match cwd {
            Some(cwd) => cwd,
            None => env::current_dir().map_err(|error| {
                ForgeError::internal(format!("could not determine current directory: {error}"))
            })?,
        };
        let cwd = canonicalize_or_self(&cwd);

        let (project_root, project_config_path) = match project_override {
            Some(path) => {
                let root = canonicalize_or_self(&path);
                if !root.is_dir() {
                    return Err(ForgeError::invalid_argument(format!(
                        "project path {} is not a directory",
                        root.display()
                    )));
                }
                let config = find_project_config(&root);
                (root, config)
            }
            None => {
                let mut current = cwd.clone();
                loop {
                    if let Some(config) = find_project_config(&current) {
                        break (current, Some(config));
                    }
                    if !current.pop() {
                        break (cwd.clone(), None);
                    }
                }
            }
        };

        let global_dir = global_config_dir();
        let state_dir = state_dir();
        let artifacts_dir = project_root.join(".artifacts");

        Ok(Self {
            cwd,
            project_root,
            project_config_path,
            global_config_path: global_dir.join("config.toml"),
            global_dir,
            state_dir,
            artifacts_dir,
        })
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.state_dir.join("sessions")
    }

    pub fn locks_dir(&self) -> PathBuf {
        self.state_dir.join("locks")
    }

    pub fn active_runs_dir(&self) -> PathBuf {
        self.state_dir.join("active-runs")
    }

    pub fn session_pointer_path(&self) -> PathBuf {
        self.project_root
            .join(".androidforge")
            .join("current-session")
    }

    pub fn androidforge_dir(&self) -> PathBuf {
        self.project_root.join(".androidforge")
    }

    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.artifacts_dir.join("runs").join(run_id)
    }

    pub fn with_artifact_dir(&self, dir: &Path) -> Self {
        let mut clone = self.clone();
        clone.artifacts_dir = if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            self.project_root.join(dir)
        };
        clone
    }
}

fn find_project_config(root: &Path) -> Option<PathBuf> {
    let dir = root.join(".androidforge");
    let project = dir.join("project.toml");
    if project.is_file() {
        return Some(project);
    }
    let legacy = dir.join("config.toml");
    if legacy.is_file() {
        return Some(legacy);
    }
    // Some example/portfolio layouts use examples/<project>/project.toml.
    // Only accept it when it clearly is an AndroidForge adapter, not an
    // unrelated file with the same name.
    let root_project = root.join("project.toml");
    if root_project.is_file() {
        if let Ok(text) = std::fs::read_to_string(&root_project) {
            if text.contains("adapter_version") {
                return Some(root_project);
            }
        }
    }
    None
}

fn global_config_dir() -> PathBuf {
    if let Ok(home) = env::var("ANDROIDFORGE_HOME") {
        return PathBuf::from(home);
    }
    if let Ok(dir) = env::var("ANDROIDFORGE_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    {
        if let Ok(appdata) = env::var("APPDATA") {
            return PathBuf::from(appdata).join("AndroidForge");
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join("androidforge");
        }
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home).join(".config").join("androidforge");
        }
    }
    PathBuf::from(".androidforge-global")
}

fn state_dir() -> PathBuf {
    if let Ok(home) = env::var("ANDROIDFORGE_HOME") {
        return PathBuf::from(home).join("state");
    }
    if let Ok(dir) = env::var("ANDROIDFORGE_STATE_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    {
        if let Ok(local) = env::var("LOCALAPPDATA") {
            return PathBuf::from(local).join("AndroidForge");
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = env::var("XDG_STATE_HOME") {
            return PathBuf::from(xdg).join("androidforge");
        }
        if let Ok(home) = env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("state")
                .join("androidforge");
        }
    }
    PathBuf::from(".androidforge-state")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_project_config_from_subdirectory() {
        let temp = tempfile::tempdir().unwrap();
        let androidforge = temp.path().join(".androidforge");
        std::fs::create_dir_all(&androidforge).unwrap();
        std::fs::write(androidforge.join("project.toml"), "adapter_version = 1\n").unwrap();
        let nested = temp.path().join("app").join("src");
        std::fs::create_dir_all(&nested).unwrap();
        let paths = ForgePaths::discover(Some(nested), None).unwrap();
        assert_eq!(paths.project_root, canonicalize_or_self(temp.path()));
        assert_eq!(
            paths.project_config_path,
            Some(canonicalize_or_self(&androidforge.join("project.toml")))
        );
    }

    #[test]
    fn project_override_wins() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir_all(project.join(".androidforge")).unwrap();
        let paths =
            ForgePaths::discover(Some(temp.path().to_path_buf()), Some(project.clone())).unwrap();
        assert_eq!(paths.project_root, canonicalize_or_self(&project));
    }
}
