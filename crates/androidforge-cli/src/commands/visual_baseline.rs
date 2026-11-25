//! Named screenshot baseline storage.
//!
//! This complements [`super::visual`]: `compare_png` still does the pixel work,
//! while this module gives a screenshot a stable, validated name and keeps a
//! small manifest per project/run. Names are validated to a conservative ASCII
//! character set and used only as a file name below the store root; nothing
//! here invokes a shell and no user text is ever interpreted as a path
//! component. All reads and writes are size-bounded before decoding.

use std::fs;
use std::io::BufReader;
use std::path::{Component, Path, PathBuf};

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::paths::ForgePaths;
use androidforge_core::util::{now_rfc3339, sha256_hex, write_atomic, write_json_atomic};
use serde::{Deserialize, Serialize};

use super::visual::{VisualDiff, VisualRegion, compare_png};

pub const SCHEMA_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "manifest.json";
pub const MAX_NAME_LEN: usize = 64;
pub const MAX_BASELINE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_BASELINE_PIXELS: u64 = 16 * 1024 * 1024;
pub const MAX_BASELINES: usize = 256;
pub const MAX_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;

/// One saved screenshot. These exact fields make up a `visual-baseline`
/// record in `schema/visual-baseline.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineEntry {
    pub name: String,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub created_at: String,
    /// Original PNG path the baseline was saved from (never used to locate
    /// the stored baseline).
    pub source_path: String,
    /// Store-relative path of the retained PNG.
    pub path: String,
    pub size_bytes: u64,
}

/// Persisted manifest for one project/run. The root is chosen by the caller,
/// so a project-wide store or a per-run store are both supported; `project`
/// and `run_id` record the scope that produced each update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineManifest {
    pub schema_version: u32,
    pub project: String,
    pub run_id: String,
    pub updated_at: String,
    pub baselines: Vec<BaselineEntry>,
}

/// A verified baseline and the path of its retained PNG.
#[derive(Debug, Clone)]
pub struct LoadedBaseline {
    pub entry: BaselineEntry,
    pub path: PathBuf,
}

/// Structured result of comparing a current screenshot with a named baseline.
#[derive(Debug, Clone, Serialize)]
pub struct BaselineComparison {
    pub name: String,
    pub baseline: BaselineEntry,
    pub current: String,
    pub passed: bool,
    pub diff: VisualDiff,
}

/// Resource bounds applied before any file is decoded or a manifest is read.
#[derive(Debug, Clone, Copy)]
pub struct BaselineLimits {
    pub max_file_bytes: u64,
    pub max_pixels: u64,
    pub max_baselines: usize,
}

impl Default for BaselineLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: MAX_BASELINE_BYTES,
            max_pixels: MAX_BASELINE_PIXELS,
            max_baselines: MAX_BASELINES,
        }
    }
}

/// Validates a baseline name. The same rule is used for saving and loading,
/// so a name that cannot be written can never be looked up either.
///
/// Accepted: 1-64 ASCII letters/digits/`.`/`_`/`-`, starting with a letter or
/// digit, not ending in `.`, not a Windows device name. This excludes path
/// separators, `..`, leading dots, shell metacharacters and non-ASCII input.
pub fn validate_name(name: &str) -> Result<&str> {
    let reject = || {
        ForgeError::invalid_argument(format!(
            "baseline name '{name}' must be 1-{MAX_NAME_LEN} ASCII characters starting with a letter or digit and using only letters, digits, '.', '_' or '-'"
        ))
        .with_suggestion("Use a simple name such as 'home_screen' or 'settings-v2'.")
    };
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return Err(reject());
    }
    let bytes = name.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() || bytes.last() == Some(&b'.') {
        return Err(reject());
    }
    if !bytes
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(reject());
    }
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(reject());
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return Err(ForgeError::invalid_argument(format!(
            "baseline name '{name}' is a reserved device name"
        ))
        .with_suggestion("Choose a different baseline name."));
    }
    Ok(name)
}

#[derive(Debug, Clone)]
pub struct BaselineStore {
    root: PathBuf,
    project: String,
    run_id: String,
    limits: BaselineLimits,
}

impl BaselineStore {
    pub fn new(root: PathBuf, project: impl Into<String>, run_id: impl Into<String>) -> Self {
        Self {
            root,
            project: project.into(),
            run_id: run_id.into(),
            limits: BaselineLimits::default(),
        }
    }

    /// Project-scoped store rooted below the configured artifact directory.
    pub fn for_project(paths: &ForgePaths, run_id: &str) -> Self {
        let project = paths
            .project_root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_string());
        Self::new(
            paths.artifacts_dir.join("visual-baselines"),
            project,
            run_id,
        )
    }

    pub fn with_limits(mut self, limits: BaselineLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join(MANIFEST_FILE)
    }

    fn png_path(&self, name: &str) -> PathBuf {
        self.root.join(format!("{name}.png"))
    }

    /// Reads the manifest. A missing manifest is an empty store, which makes
    /// `list` safe to call before the first save.
    pub fn manifest(&self) -> Result<BaselineManifest> {
        let path = self.manifest_path();
        if !path.exists() {
            return Ok(BaselineManifest {
                schema_version: SCHEMA_VERSION,
                project: self.project.clone(),
                run_id: self.run_id.clone(),
                updated_at: String::new(),
                baselines: Vec::new(),
            });
        }
        let bytes = fs::read(&path).map_err(|error| {
            ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!(
                    "could not read baseline manifest {}: {error}",
                    path.display()
                ),
            )
        })?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!(
                    "baseline manifest is {} bytes; the limit is {MAX_MANIFEST_BYTES}",
                    bytes.len()
                ),
            ));
        }
        let manifest: BaselineManifest = serde_json::from_slice(&bytes).map_err(|error| {
            ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!(
                    "could not parse baseline manifest {}: {error}",
                    path.display()
                ),
            )
            .with_suggestion("Restore the manifest or save the baselines again.")
        })?;
        if manifest.schema_version != SCHEMA_VERSION {
            return Err(ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!(
                    "baseline manifest version {} is unsupported (expected {SCHEMA_VERSION})",
                    manifest.schema_version
                ),
            )
            .with_suggestion("Regenerate the baselines with this AndroidForge version."));
        }
        if manifest.baselines.len() > self.limits.max_baselines {
            return Err(ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!(
                    "baseline manifest lists {} entries; the limit is {}",
                    manifest.baselines.len(),
                    self.limits.max_baselines
                ),
            ));
        }
        // A hand-edited manifest must not be able to plant an unsafe name.
        for entry in &manifest.baselines {
            validate_name(&entry.name)?;
            if entry.path != format!("{}.png", entry.name) {
                return Err(ForgeError::new(
                    ErrorCode::ConfigInvalid,
                    format!(
                        "baseline '{}' has an unexpected stored path '{}'",
                        entry.name, entry.path
                    ),
                ));
            }
        }
        Ok(manifest)
    }

    /// Lists the saved baseline metadata, sorted by name.
    pub fn list(&self) -> Result<Vec<BaselineEntry>> {
        let mut manifest = self.manifest()?;
        manifest
            .baselines
            .sort_by(|left, right| left.name.cmp(&right.name));
        Ok(manifest.baselines)
    }

    /// Saves a new named baseline. An existing name is refused; use `update`
    /// to replace it deliberately.
    pub fn save(&self, name: &str, png: &Path) -> Result<BaselineEntry> {
        self.save_inner(name, png, false)
    }

    /// Saves or replaces a named baseline (explicit update policy).
    pub fn update(&self, name: &str, png: &Path) -> Result<BaselineEntry> {
        self.save_inner(name, png, true)
    }

    fn save_inner(&self, name: &str, png: &Path, allow_update: bool) -> Result<BaselineEntry> {
        let name = validate_name(name)?;
        let metadata = fs::metadata(png).map_err(|error| {
            ForgeError::new(
                ErrorCode::FileNotFound,
                format!("could not read baseline source {}: {error}", png.display()),
            )
        })?;
        if !metadata.is_file() {
            return Err(ForgeError::invalid_argument(format!(
                "baseline source {} is not a file",
                png.display()
            )));
        }
        if metadata.len() == 0 || metadata.len() > self.limits.max_file_bytes {
            return Err(ForgeError::invalid_argument(format!(
                "baseline source is {} bytes; the limit is {}",
                metadata.len(),
                self.limits.max_file_bytes
            ))
            .with_suggestion("Use a smaller screenshot or re-encode it as an optimized PNG."));
        }

        let (width, height) = png_dimensions(png)?;
        check_dimensions(width, height, self.limits.max_pixels)?;

        let bytes = fs::read(png).map_err(|error| {
            ForgeError::new(
                ErrorCode::FileNotFound,
                format!("could not read baseline source {}: {error}", png.display()),
            )
        })?;
        if bytes.is_empty() || bytes.len() as u64 > self.limits.max_file_bytes {
            return Err(ForgeError::invalid_argument(format!(
                "baseline source changed while reading; {} bytes exceeds the limit of {}",
                bytes.len(),
                self.limits.max_file_bytes
            )));
        }

        let mut manifest = self.manifest()?;
        let is_update = manifest.baselines.iter().any(|entry| entry.name == name);
        if is_update && !allow_update {
            return Err(ForgeError::new(
                ErrorCode::InvalidArgument,
                format!("baseline '{name}' already exists"),
            )
            .with_context(
                "existing",
                serde_json::to_value(
                    manifest
                        .baselines
                        .iter()
                        .find(|entry| entry.name == name)
                        .expect("checked above"),
                )
                .unwrap_or_default(),
            )
            .with_suggestion(
                "Use `capture baseline update --name <name>` to replace it, or choose a new name.",
            ));
        }
        if !is_update && manifest.baselines.len() >= self.limits.max_baselines {
            return Err(ForgeError::invalid_argument(format!(
                "baseline store already holds {} entries; the limit is {}",
                manifest.baselines.len(),
                self.limits.max_baselines
            )));
        }

        let path = format!("{name}.png");
        write_atomic(&self.png_path(name), &bytes)?;
        let entry = BaselineEntry {
            name: name.to_string(),
            sha256: sha256_hex(&bytes),
            width,
            height,
            created_at: now_rfc3339(),
            source_path: png.to_string_lossy().into_owned(),
            size_bytes: bytes.len() as u64,
            path,
        };
        manifest.baselines.retain(|existing| existing.name != name);
        manifest.baselines.push(entry.clone());
        manifest
            .baselines
            .sort_by(|left, right| left.name.cmp(&right.name));
        manifest.schema_version = SCHEMA_VERSION;
        manifest.project = self.project.clone();
        manifest.run_id = self.run_id.clone();
        manifest.updated_at = now_rfc3339();
        write_json_atomic(&self.manifest_path(), &manifest)?;
        Ok(entry)
    }

    /// Loads and verifies a named baseline (existence, size, sha256 and PNG
    /// dimensions) before it is used for a comparison.
    pub fn load(&self, name: &str) -> Result<LoadedBaseline> {
        let name = validate_name(name)?;
        let manifest = self.manifest()?;
        let entry = manifest
            .baselines
            .into_iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| {
                ForgeError::new(
                    ErrorCode::FileNotFound,
                    format!("baseline '{name}' is not saved"),
                )
                .with_context_string("name", name)
                .with_suggestion("Run `androidforge capture baseline save --name <name>` first.")
            })?;
        let path = self.png_path(name);
        let metadata = fs::metadata(&path).map_err(|error| {
            ForgeError::new(
                ErrorCode::FileNotFound,
                format!(
                    "baseline '{name}' PNG is missing at {}: {error}",
                    path.display()
                ),
            )
            .with_suggestion("Re-save the named baseline from a trusted screenshot.")
        })?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > self.limits.max_file_bytes
        {
            return Err(ForgeError::new(
                ErrorCode::UiAssertionFailed,
                format!(
                    "baseline '{name}' PNG at {} has an unexpected size ({} bytes)",
                    path.display(),
                    metadata.len()
                ),
            ));
        }
        let actual = sha256_hex(&fs::read(&path).map_err(|error| {
            ForgeError::new(
                ErrorCode::FileNotFound,
                format!(
                    "could not read baseline '{name}' at {}: {error}",
                    path.display()
                ),
            )
        })?);
        if actual != entry.sha256 {
            return Err(ForgeError::new(
                ErrorCode::UiAssertionFailed,
                format!(
                    "baseline '{name}' is corrupt: manifest sha256 {} but file sha256 {actual}",
                    entry.sha256
                ),
            )
            .with_context_string("name", name)
            .with_context_string("path", path.to_string_lossy().into_owned())
            .with_suggestion("Re-save the named baseline from a trusted screenshot."));
        }
        let (width, height) = png_dimensions(&path)?;
        if width != entry.width || height != entry.height {
            return Err(ForgeError::new(
                ErrorCode::UiAssertionFailed,
                format!(
                    "baseline '{name}' declares {}x{} but its PNG is {width}x{height}",
                    entry.width, entry.height
                ),
            )
            .with_suggestion("Re-save the named baseline so the manifest matches its PNG."));
        }
        Ok(LoadedBaseline { entry, path })
    }

    /// Compares a screenshot with a named baseline via `compare_png`, returning
    /// both the baseline metadata and the structured diff metrics.
    pub fn compare(
        &self,
        name: &str,
        current: &Path,
        region: Option<VisualRegion>,
        diff_threshold: u8,
        max_diff_ratio: f64,
        diff_output: Option<&Path>,
    ) -> Result<BaselineComparison> {
        let loaded = self.load(name)?;
        let diff = compare_png(
            &loaded.path,
            current,
            region,
            diff_threshold,
            max_diff_ratio,
            diff_output,
        )?;
        Ok(BaselineComparison {
            name: loaded.entry.name.clone(),
            baseline: loaded.entry,
            current: current.to_string_lossy().into_owned(),
            passed: diff.passed,
            diff,
        })
    }
}

fn check_dimensions(width: u32, height: u32, max_pixels: u64) -> Result<()> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > max_pixels {
        return Err(ForgeError::invalid_argument(format!(
            "screenshot {width}x{height} has {pixels} pixels; the baseline limit is {max_pixels}"
        ))
        .with_suggestion("Capture a smaller screenshot or raise the baseline limit."));
    }
    Ok(())
}

fn png_dimensions(path: &Path) -> Result<(u32, u32)> {
    let file = std::fs::File::open(path).map_err(|error| {
        ForgeError::new(
            ErrorCode::FileNotFound,
            format!("could not open PNG {}: {error}", path.display()),
        )
    })?;
    let decoder = png::Decoder::new(BufReader::new(file));
    let reader = decoder.read_info().map_err(|error| {
        ForgeError::new(
            ErrorCode::UiAssertionFailed,
            format!("{} is not a valid PNG: {error}", path.display()),
        )
    })?;
    Ok((reader.info().width, reader.info().height))
}

#[cfg(test)]
mod tests {
    use androidforge_core::error::ErrorCode;
    use jsonschema::validator_for;
    use serde_json::Value;

    use super::*;

    fn write_png(path: &Path, width: u32, height: u32, pixels: &[[u8; 3]]) {
        let file = fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let bytes = pixels
            .iter()
            .flat_map(|pixel| pixel.iter().copied())
            .collect::<Vec<_>>();
        writer.write_image_data(&bytes).unwrap();
    }

    fn test_store(temp: &Path) -> BaselineStore {
        BaselineStore::new(temp.join("baselines"), "TestProject", "run-1")
    }

    #[test]
    fn project_store_is_rooted_below_the_artifact_directory() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ForgePaths::discover(Some(temp.path().to_path_buf()), None).unwrap();
        let store = BaselineStore::for_project(&paths, "run-7");
        assert_eq!(store.root(), paths.artifacts_dir.join("visual-baselines"));
        let manifest = store.manifest().unwrap();
        assert_eq!(
            manifest.project,
            paths
                .project_root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        );
        assert_eq!(manifest.run_id, "run-7");
    }

    fn validate_schema(schema_json: &str, value: &Value) {
        let schema: Value = serde_json::from_str(schema_json).unwrap();
        let validator = validator_for(&schema).unwrap();
        if let Err(error) = validator.validate(value) {
            panic!("schema rejected payload: {error}\npayload: {value:#}");
        }
    }

    fn validate_baseline(value: &Value) {
        validate_schema(
            include_str!("../../../../schema/visual-baseline.schema.json"),
            value,
        );
    }

    #[test]
    fn save_load_and_list_named_baselines() {
        let temp = tempfile::tempdir().unwrap();
        let store = test_store(temp.path());
        let first = temp.path().join("first.png");
        let second = temp.path().join("second.png");
        write_png(&first, 2, 2, &[[1u8, 2, 3]; 4]);
        write_png(&second, 1, 1, &[[9, 9, 9]]);

        let saved = store.save("home_screen", &first).unwrap();
        let other = store.save("settings.v2", &second).unwrap();
        assert_eq!(saved.width, 2);
        assert_eq!(saved.height, 2);
        assert_eq!(saved.path, "home_screen.png");
        assert_eq!(saved.size_bytes, fs::metadata(&first).unwrap().len());
        assert_eq!(saved.sha256, sha256_hex(&fs::read(&first).unwrap()));
        assert!(saved.created_at.ends_with('Z'));
        assert_eq!(saved.source_path, first.to_string_lossy());
        assert_eq!(other.width, 1);

        let names = store
            .list()
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["home_screen", "settings.v2"]);

        let loaded = store.load("home_screen").unwrap();
        assert_eq!(loaded.entry, saved);
        assert_eq!(loaded.path, store.root().join("home_screen.png"));

        let manifest = store.manifest().unwrap();
        assert_eq!(manifest.schema_version, SCHEMA_VERSION);
        assert_eq!(manifest.project, "TestProject");
        assert_eq!(manifest.run_id, "run-1");
        assert_eq!(manifest.baselines.len(), 2);
        validate_baseline(&serde_json::to_value(&manifest).unwrap());
        validate_baseline(&serde_json::to_value(&saved).unwrap());
    }

    #[test]
    fn duplicate_save_is_refused_and_update_replaces_it() {
        let temp = tempfile::tempdir().unwrap();
        let store = test_store(temp.path());
        let first = temp.path().join("first.png");
        let second = temp.path().join("second.png");
        write_png(&first, 1, 1, &[[1, 2, 3]]);
        write_png(&second, 1, 1, &[[4, 5, 6]]);

        let original = store.save("home", &first).unwrap();
        let error = store.save("home", &second).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
        assert!(error.message().contains("already exists"));
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(store.load("home").unwrap().entry, original);

        let updated = store.update("home", &second).unwrap();
        assert_ne!(updated.sha256, original.sha256);
        assert_eq!(updated.name, "home");
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(store.load("home").unwrap().entry, updated);
    }

    #[test]
    fn unsafe_names_are_rejected_without_writing_anything() {
        let temp = tempfile::tempdir().unwrap();
        let store = test_store(temp.path());
        let png = temp.path().join("png.png");
        write_png(&png, 1, 1, &[[0, 0, 0]]);
        let long = "a".repeat(MAX_NAME_LEN + 1);
        let candidates = [
            "",
            ".",
            "..",
            "../escape",
            "a/b",
            "a\\b",
            "a;b",
            "a$b",
            " leading",
            "trailing ",
            "CON",
            "com1",
            "ünicode",
            long.as_str(),
        ];
        for name in candidates {
            let error = store.save(name, &png).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidArgument, "name: {name:?}");
            assert!(store.load(name).is_err(), "load must also reject {name:?}");
        }
        assert!(store.list().unwrap().is_empty());
        assert!(!temp.path().join("baselines").join("png.png").exists());
        assert!(!temp.path().join("escape.png").exists());
    }

    #[test]
    fn missing_baseline_is_reported() {
        let temp = tempfile::tempdir().unwrap();
        let store = test_store(temp.path());
        assert!(store.list().unwrap().is_empty());
        let error = store.load("absent").unwrap_err();
        assert_eq!(error.code(), ErrorCode::FileNotFound);
        assert!(error.message().contains("not saved"));
    }

    #[test]
    fn corrupt_or_missing_baseline_png_is_detected() {
        let temp = tempfile::tempdir().unwrap();
        let store = test_store(temp.path());
        let png = temp.path().join("png.png");
        write_png(&png, 1, 1, &[[7, 7, 7]]);
        store.save("home", &png).unwrap();
        let stored = store.root().join("home.png");

        fs::write(&stored, b"not a png at all").unwrap();
        let error = store.load("home").unwrap_err();
        assert_eq!(error.code(), ErrorCode::UiAssertionFailed);
        assert!(error.message().contains("sha256"));

        fs::remove_file(&stored).unwrap();
        let error = store.load("home").unwrap_err();
        assert_eq!(error.code(), ErrorCode::FileNotFound);
        assert!(error.message().contains("missing"));
    }

    #[test]
    fn corrupt_or_unsupported_manifest_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let store = test_store(temp.path());
        let png = temp.path().join("png.png");
        write_png(&png, 1, 1, &[[7, 7, 7]]);
        store.save("home", &png).unwrap();

        fs::write(store.manifest_path(), b"{ not json }").unwrap();
        let error = store.load("home").unwrap_err();
        assert_eq!(error.code(), ErrorCode::ConfigInvalid);

        let mut manifest = BaselineManifest {
            schema_version: SCHEMA_VERSION,
            project: "TestProject".into(),
            run_id: "run-1".into(),
            updated_at: now_rfc3339(),
            baselines: Vec::new(),
        };
        manifest.schema_version = SCHEMA_VERSION + 1;
        write_json_atomic(&store.manifest_path(), &manifest).unwrap();
        let error = store.load("home").unwrap_err();
        assert_eq!(error.code(), ErrorCode::ConfigInvalid);
        assert!(error.message().contains("version"));
    }

    #[test]
    fn invalid_source_and_bounds_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let store = test_store(temp.path());
        let directory = temp.path().join("not-a-file");
        fs::create_dir_all(&directory).unwrap();
        assert_eq!(
            store.save("home", &directory).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );
        let garbage = temp.path().join("garbage.png");
        fs::write(&garbage, b"not a png").unwrap();
        assert_eq!(
            store.save("home", &garbage).unwrap_err().code(),
            ErrorCode::UiAssertionFailed
        );

        let limited = test_store(temp.path()).with_limits(BaselineLimits {
            max_file_bytes: 16,
            max_pixels: 2,
            max_baselines: 1,
        });
        let png = temp.path().join("two.png");
        write_png(&png, 2, 2, &[[0u8, 0, 0]; 4]);
        assert_eq!(
            limited.save("small", &png).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );

        let limited = test_store(temp.path()).with_limits(BaselineLimits {
            max_file_bytes: 1_000_000,
            max_pixels: 2,
            max_baselines: 1,
        });
        let error = limited.save("small", &png).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
        assert!(error.message().contains("pixels"));

        let limited = test_store(temp.path()).with_limits(BaselineLimits {
            max_file_bytes: MAX_BASELINE_BYTES,
            max_pixels: MAX_BASELINE_PIXELS,
            max_baselines: 1,
        });
        let one = temp.path().join("one.png");
        write_png(&one, 1, 1, &[[1, 1, 1]]);
        limited.save("one", &one).unwrap();
        assert_eq!(
            limited.save("two", &png).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn compare_named_baseline_passes_and_fails_with_structured_results() {
        let temp = tempfile::tempdir().unwrap();
        let store = test_store(temp.path());
        let baseline = temp.path().join("baseline.png");
        let matching = temp.path().join("matching.png");
        let changed = temp.path().join("changed.png");
        let mut pixels = [[0u8, 0, 0]; 4];
        write_png(&baseline, 2, 2, &pixels);
        write_png(&matching, 2, 2, &pixels);
        pixels[3] = [255, 255, 255];
        write_png(&changed, 2, 2, &pixels);
        let saved = store.save("home", &baseline).unwrap();

        let pass = store
            .compare("home", &matching, None, 0, 0.0, None)
            .unwrap();
        assert!(pass.passed);
        assert_eq!(pass.name, "home");
        assert_eq!(pass.baseline, saved);
        assert_eq!(pass.diff.changed_pixels, 0);
        assert_eq!(pass.diff.width, 2);
        assert_eq!(pass.diff.height, 2);
        let value = serde_json::to_value(&pass).unwrap();
        validate_baseline(&value["baseline"]);
        validate_schema(
            include_str!("../../../../schema/visual-diff.schema.json"),
            &value["diff"],
        );

        let fail = store
            .compare(
                "home",
                &changed,
                Some(VisualRegion {
                    x: 1,
                    y: 1,
                    width: 1,
                    height: 1,
                }),
                0,
                0.0,
                Some(&temp.path().join("diff.png")),
            )
            .unwrap();
        assert!(!fail.passed);
        assert_eq!(fail.diff.changed_pixels, 1);
        assert_eq!(fail.diff.changed_ratio, 1.0);
        assert_eq!(fail.diff.changed_bounds.unwrap().x, 1);
        assert!(fail.diff.diff_image.is_some());
        assert!(temp.path().join("diff.png").is_file());
    }

    #[test]
    fn compare_reports_image_size_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let store = test_store(temp.path());
        let small = temp.path().join("small.png");
        let large = temp.path().join("large.png");
        write_png(&small, 2, 2, &[[0u8, 0, 0]; 4]);
        write_png(&large, 3, 3, &[[0u8, 0, 0]; 9]);
        store.save("home", &small).unwrap();

        let error = store
            .compare("home", &large, None, 0, 0.0, None)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::UiAssertionFailed);
        assert!(error.message().contains("sizes differ"));
    }
}
