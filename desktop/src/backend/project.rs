use std::path::{Path, PathBuf};

use super::{DesktopError, Result};
use crate::models::ProjectConfig;

#[derive(Debug, Clone)]
pub struct DiscoveredProject {
    pub root: PathBuf,
    pub config_path: PathBuf,
    pub name: String,
    pub package: Option<String>,
    pub debug_package: Option<String>,
    pub adapter_version: u32,
    pub config: ProjectConfig,
}

impl DiscoveredProject {
    pub fn display_name(&self) -> String {
        self.name.clone()
    }

    pub fn package_label(&self) -> String {
        self.package
            .clone()
            .or_else(|| self.debug_package.clone())
            .unwrap_or_else(|| "no package declared".to_string())
    }

    pub fn conformance_flow(&self) -> Option<&str> {
        self.config.conformance.flow.as_deref()
    }
}

pub fn discover(root: &Path) -> Result<Vec<DiscoveredProject>> {
    let mut projects = Vec::new();
    let mut visited = 0usize;
    discover_inner(root, 0, &mut projects, &mut visited);
    projects.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then_with(|| left.root.cmp(&right.root))
    });
    projects.dedup_by(|left, right| left.root == right.root);
    Ok(projects)
}

pub fn load(path: &Path) -> Result<DiscoveredProject> {
    let (root, config_path) = if path.is_file() {
        (
            path.parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| DesktopError::InvalidPath(path.display().to_string()))?,
            path.to_path_buf(),
        )
    } else {
        let config_path = project_config_path(path).ok_or_else(|| {
            DesktopError::InvalidPath(format!(
                "no project.toml or .androidforge/project.toml in {}",
                path.display()
            ))
        })?;
        (path.to_path_buf(), config_path)
    };
    let text = std::fs::read_to_string(&config_path)?;
    let config: ProjectConfig = toml::from_str(&text)?;
    if config.adapter_version == 0 {
        return Err(DesktopError::InvalidPath(format!(
            "{} does not declare adapter_version",
            config_path.display()
        )));
    }
    let fallback_name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    Ok(DiscoveredProject {
        name: config
            .project
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(fallback_name),
        package: config.project.package.clone(),
        debug_package: config.project.debug_package.clone(),
        adapter_version: config.adapter_version,
        root,
        config_path,
        config,
    })
}

fn discover_inner(
    directory: &Path,
    depth: usize,
    projects: &mut Vec<DiscoveredProject>,
    visited: &mut usize,
) {
    if depth > 3 || *visited > 500 {
        return;
    }
    *visited += 1;
    if let Ok(project) = load(directory) {
        projects.push(project);
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.starts_with('.')
            || matches!(
                name.as_str(),
                "target"
                    | "node_modules"
                    | "dist"
                    | "case-studies"
                    | "crates"
                    | "schema"
                    | "desktop"
                    | "docs"
                    | "vendor"
            )
        {
            continue;
        }
        discover_inner(&path, depth + 1, projects, visited);
    }
}

fn project_config_path(root: &Path) -> Option<PathBuf> {
    let nested = root.join(".androidforge").join("project.toml");
    if nested.is_file() {
        return Some(nested);
    }
    let direct = root.join("project.toml");
    if direct.is_file() {
        return Some(direct);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_nested_project_adapter() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(
            app.join("project.toml"),
            "adapter_version = 1\n[project]\nname = \"Example\"\npackage = \"com.example\"\n",
        )
        .unwrap();
        let projects = discover(temp.path()).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Example");
        assert_eq!(projects[0].package.as_deref(), Some("com.example"));
    }

    #[test]
    fn ignores_directories_without_an_adapter() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("not-a-project")).unwrap();
        assert!(discover(temp.path()).unwrap().is_empty());
    }
}
