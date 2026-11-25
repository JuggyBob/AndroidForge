//! Bounded local mock HTTP(S) server for app network flows.
//!
//! The server speaks a deliberately small subset of HTTP/1.1: one request per
//! connection, bounded headers and bodies, and no keep-alive. It exists to make
//! app requests observable and reproducible on an emulator, not to replace a
//! general-purpose web framework.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use androidforge_core::error::{ErrorCode, ForgeError, Result};
use androidforge_core::output::CommandSuccess;
use androidforge_core::redact::Redactor;
use androidforge_core::util::{ensure_dir, sha256_hex};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinSet;

use crate::cli::{NetworkMockCommand, NetworkMockServeArgs};
use crate::commands::common::success;
use crate::context::CliContext;

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Default cap on one scenario response body.
pub const DEFAULT_SCENARIO_MAX_RESPONSE_BYTES: usize = 256 * 1024;
/// Default request cap for a scenario run.
pub const DEFAULT_SCENARIO_MAX_REQUESTS: u64 = 64;
/// Default lifetime of a scenario listener, in milliseconds.
pub const DEFAULT_SCENARIO_DURATION_MS: u64 = 30_000;
/// Scenarios never bind anything other than a loopback interface.
pub const DEFAULT_SCENARIO_BIND: &str = "127.0.0.1";

/// Canonical, machine-readable scenario names. `no-APK` keeps its uppercase
/// suffix because it is the published scenario identifier.
pub const SCENARIO_NAMES: [&str; 4] = ["newer-release", "offline", "bad-digest", "no-APK"];

/// Named network mock scenario.
///
/// The payloads are supplied by fixture JSON, never by this enum; the enum only
/// carries the public name and the serving mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioName {
    NewerRelease,
    Offline,
    BadDigest,
    #[serde(rename = "no-APK")]
    NoApk,
}

impl ScenarioName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NewerRelease => "newer-release",
            Self::Offline => "offline",
            Self::BadDigest => "bad-digest",
            Self::NoApk => "no-APK",
        }
    }

    /// Parse a scenario name case-insensitively, returning the canonical name
    /// on success and an actionable error for unknown scenarios.
    pub fn parse(value: &str) -> Result<Self> {
        let trimmed = value.trim();
        let parsed = match trimmed.to_ascii_lowercase().as_str() {
            "newer-release" => Self::NewerRelease,
            "offline" => Self::Offline,
            "bad-digest" => Self::BadDigest,
            "no-apk" => Self::NoApk,
            _ => {
                return Err(ForgeError::invalid_argument(format!(
                    "unknown network mock scenario '{value}'"
                ))
                .with_suggestion(format!(
                    "Available scenarios: {}",
                    SCENARIO_NAMES.join(", ")
                )));
            }
        };
        Ok(parsed)
    }
}

/// How a scenario answers requests. `Offline` deliberately creates no listener.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioMode {
    #[default]
    Serve,
    Offline,
}

impl ScenarioMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Serve => "serve",
            Self::Offline => "offline",
        }
    }
}

/// Bounds and safety defaults applied to every scenario, independently of the
/// CLI arguments used to start the mock server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioDefaults {
    #[serde(default = "default_scenario_bind")]
    pub bind: String,
    #[serde(default = "default_scenario_max_response_bytes")]
    pub max_response_bytes: usize,
    #[serde(default = "default_scenario_max_requests")]
    pub max_requests: u64,
    #[serde(default = "default_scenario_duration_ms")]
    pub duration_ms: u64,
    #[serde(default = "default_true")]
    pub redact: bool,
}

impl Default for ScenarioDefaults {
    fn default() -> Self {
        Self {
            bind: default_scenario_bind(),
            max_response_bytes: default_scenario_max_response_bytes(),
            max_requests: default_scenario_max_requests(),
            duration_ms: default_scenario_duration_ms(),
            redact: true,
        }
    }
}

/// Fixture JSON document backing a scenario. Routes and payloads are data, so
/// projects can override them without changing code or host network settings.
#[derive(Debug, Clone, Deserialize)]
pub struct ScenarioFixture {
    #[serde(default)]
    pub scenario: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mode: ScenarioMode,
    #[serde(default)]
    pub defaults: ScenarioDefaults,
    #[serde(default)]
    pub routes: Vec<MockRoute>,
}

/// One entry of the scenario catalog, exposed by `network mock scenarios --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScenarioInfo {
    pub name: String,
    pub description: String,
    pub mode: ScenarioMode,
    /// Conventional fixture file name for projects that want to override the
    /// built-in payloads.
    pub fixture_file: String,
}

/// A validated scenario definition loaded from fixture JSON.
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioDefinition {
    pub name: ScenarioName,
    pub description: String,
    pub mode: ScenarioMode,
    pub fixture_file: String,
    pub defaults: ScenarioDefaults,
    #[serde(skip)]
    pub fixture_json: &'static str,
}

/// Fully resolved scenario run configuration. This is the typed surface the
/// CLI wiring can consume without knowing any scenario semantics.
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioConfig {
    pub scenario: ScenarioName,
    pub mode: ScenarioMode,
    pub bind: String,
    pub max_response_bytes: usize,
    pub max_requests: u64,
    pub duration_ms: u64,
    pub redact: bool,
    pub routes: Vec<MockRoute>,
    pub fixture: Option<PathBuf>,
}

/// Optional overrides used when resolving a scenario.
#[derive(Debug, Clone, Default)]
pub struct ScenarioOptions {
    /// Explicit fixture JSON file. When absent, the built-in fixture JSON is used.
    pub fixture: Option<PathBuf>,
    pub bind: Option<String>,
    pub max_response_bytes: Option<usize>,
    pub max_requests: Option<u64>,
    pub duration_ms: Option<u64>,
    pub redact: Option<bool>,
}

trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}

/// Route file model. A fixture may also declare the scenario it belongs to and
/// scenario defaults, so the same JSON shape works for raw routes and scenarios.
#[allow(dead_code)] // scenario/defaults are part of the typed config surface used by scenarios.
#[derive(Debug, Clone, Deserialize)]
pub struct RouteFile {
    #[serde(default)]
    pub routes: Vec<MockRoute>,
    #[serde(default)]
    pub scenario: Option<String>,
    #[serde(default)]
    pub defaults: Option<ScenarioDefaults>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockRoute {
    #[serde(default = "any_method")]
    pub method: String,
    pub path: String,
    #[serde(default = "ok_status")]
    pub status: u16,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub delay_ms: u64,
}

fn any_method() -> String {
    "*".to_string()
}

fn ok_status() -> u16 {
    200
}

fn default_scenario_bind() -> String {
    DEFAULT_SCENARIO_BIND.to_string()
}

fn default_scenario_max_response_bytes() -> usize {
    DEFAULT_SCENARIO_MAX_RESPONSE_BYTES
}

fn default_scenario_max_requests() -> u64 {
    DEFAULT_SCENARIO_MAX_REQUESTS
}

fn default_scenario_duration_ms() -> u64 {
    DEFAULT_SCENARIO_DURATION_MS
}

fn default_true() -> bool {
    true
}

// Built-in scenario payloads are fixture JSON documents. They contain generic
// paths and data, never a third-party host, repository or release API shape.
const FIXTURE_NEWER_RELEASE: &str = r#"{
  "scenario": "newer-release",
  "description": "Advertises an update newer than the installed build.",
  "mode": "serve",
  "defaults": {"bind":"127.0.0.1","max_response_bytes":262144,"max_requests":64,"duration_ms":30000,"redact":true},
  "routes": [
    {"method":"GET","path":"/releases/latest","status":200,"headers":{"content-type":"application/json"},"body":"{\"version\":\"9.9.9\",\"artifact\":\"/download/app.apk\",\"notes\":\"fixture update\"}"},
    {"method":"GET","path":"/version","status":200,"body":"9.9.9"}
  ]
}"#;

const FIXTURE_OFFLINE: &str = r#"{
  "scenario": "offline",
  "description": "No listener; requests are deterministically refused without touching host network settings.",
  "mode": "offline",
  "defaults": {"bind":"127.0.0.1","max_response_bytes":262144,"max_requests":1,"duration_ms":1000,"redact":true},
  "routes": []
}"#;

const FIXTURE_BAD_DIGEST: &str = r#"{
  "scenario": "bad-digest",
  "description": "Serves a release manifest with a fixture-controlled digest that does not match the fixture APK.",
  "mode": "serve",
  "defaults": {"bind":"127.0.0.1","max_response_bytes":262144,"max_requests":64,"duration_ms":30000,"redact":true},
  "routes": [
    {"method":"GET","path":"/releases/latest","status":200,"headers":{"content-type":"application/json"},"body":"{\"version\":\"9.9.9\",\"sha256\":\"sha256:0000000000000000000000000000000000000000000000000000000000000000\",\"artifact\":\"/download/app.apk\"}"},
    {"method":"GET","path":"/download/app.apk","status":200,"headers":{"content-type":"application/octet-stream"},"body":"fixture-apk-bytes"}
  ]
}"#;

const FIXTURE_NO_APK: &str = r#"{
  "scenario": "no-APK",
  "description": "Serves a release manifest whose artifact route returns a fixture-controlled 404.",
  "mode": "serve",
  "defaults": {"bind":"127.0.0.1","max_response_bytes":262144,"max_requests":64,"duration_ms":30000,"redact":true},
  "routes": [
    {"method":"GET","path":"/releases/latest","status":200,"headers":{"content-type":"application/json"},"body":"{\"version\":\"9.9.9\",\"artifact\":\"/download/app.apk\"}"},
    {"method":"GET","path":"/download/app.apk","status":404,"headers":{"content-type":"application/json"},"body":"{\"error\":\"artifact unavailable\"}"}
  ]
}"#;

const SCENARIO_BUILTINS: &[(ScenarioName, &str, &str)] = &[
    (
        ScenarioName::NewerRelease,
        ".androidforge/network-mock/newer-release.json",
        FIXTURE_NEWER_RELEASE,
    ),
    (
        ScenarioName::Offline,
        ".androidforge/network-mock/offline.json",
        FIXTURE_OFFLINE,
    ),
    (
        ScenarioName::BadDigest,
        ".androidforge/network-mock/bad-digest.json",
        FIXTURE_BAD_DIGEST,
    ),
    (
        ScenarioName::NoApk,
        ".androidforge/network-mock/no-APK.json",
        FIXTURE_NO_APK,
    ),
];

fn parse_builtin_fixture(json: &str) -> ScenarioFixture {
    serde_json::from_str(json).expect("built-in scenario fixture must be valid JSON")
}

fn builtin(name: ScenarioName) -> &'static (ScenarioName, &'static str, &'static str) {
    SCENARIO_BUILTINS
        .iter()
        .find(|(candidate, _, _)| *candidate == name)
        .expect("every scenario name has a built-in fixture")
}

/// List the named scenarios with their mode and conventional fixture file.
///
/// This backs `network mock scenarios --json`; it does not require a session,
/// device or host network change.
pub fn scenario_catalog() -> Vec<ScenarioInfo> {
    SCENARIO_BUILTINS
        .iter()
        .map(|(_, fixture_file, json)| {
            let fixture = parse_builtin_fixture(json);
            ScenarioInfo {
                name: fixture.scenario.unwrap_or_default(),
                description: fixture.description.unwrap_or_default(),
                mode: fixture.mode,
                fixture_file: (*fixture_file).to_string(),
            }
        })
        .collect()
}

/// Alias used by callers that prefer the shorter list name.
#[allow(dead_code)] // Kept as a stable typed alias for library-style callers.
pub fn list_scenarios() -> Vec<ScenarioInfo> {
    scenario_catalog()
}

/// Look up a built-in scenario definition by name.
pub fn lookup_scenario(name: &str) -> Result<ScenarioDefinition> {
    let parsed = ScenarioName::parse(name)?;
    let (_, fixture_file, json) = builtin(parsed);
    let fixture = parse_builtin_fixture(json);
    Ok(ScenarioDefinition {
        name: parsed,
        description: fixture.description.unwrap_or_default(),
        mode: fixture.mode,
        fixture_file: (*fixture_file).to_string(),
        defaults: fixture.defaults,
        fixture_json: json,
    })
}

/// Read and parse an external scenario fixture JSON file.
pub fn load_scenario_fixture(path: &Path) -> Result<ScenarioFixture> {
    if !path.is_file() {
        return Err(ForgeError::new(
            ErrorCode::ConfigInvalid,
            format!("missing mock scenario fixture file {}", path.display()),
        )
        .with_suggestion(
            "Create the fixture JSON, or omit the fixture override to use the built-in payload.",
        ));
    }
    let text = std::fs::read_to_string(path).map_err(|error| {
        ForgeError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "could not read mock scenario fixture {}: {error}",
                path.display()
            ),
        )
    })?;
    serde_json::from_str(&text).map_err(|error| {
        ForgeError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "invalid mock scenario fixture JSON {}: {error}",
                path.display()
            ),
        )
    })
}

/// Resolve a named scenario into a validated, typed run configuration.
///
/// Without an explicit fixture, the built-in fixture JSON is used. With one,
/// all routes and payloads come from that file, so projects can adapt scenarios
/// without changing the binary or any host network setting.
pub fn resolve_scenario(name: &str, options: &ScenarioOptions) -> Result<ScenarioConfig> {
    let definition = lookup_scenario(name)?;
    let fixture = match &options.fixture {
        Some(path) => load_scenario_fixture(path)?,
        None => parse_builtin_fixture(definition.fixture_json),
    };
    if let Some(declared) = &fixture.scenario
        && ScenarioName::parse(declared).ok() != Some(definition.name)
    {
        return Err(ForgeError::invalid_argument(format!(
            "mock scenario fixture declares '{declared}' but '{name}' was requested"
        )));
    }
    let defaults = fixture.defaults;
    let config = ScenarioConfig {
        scenario: definition.name,
        mode: fixture.mode,
        bind: options.bind.clone().unwrap_or(defaults.bind),
        max_response_bytes: options
            .max_response_bytes
            .unwrap_or(defaults.max_response_bytes),
        max_requests: options.max_requests.unwrap_or(defaults.max_requests),
        duration_ms: options.duration_ms.unwrap_or(defaults.duration_ms),
        redact: options.redact.unwrap_or(defaults.redact),
        routes: fixture.routes,
        fixture: options.fixture.clone(),
    };
    validate_scenario(&config)?;
    Ok(config)
}

/// Validate loopback-only, bounded, host-only, redacted scenario settings and
/// fixture-controlled response sizes.
pub fn validate_scenario(config: &ScenarioConfig) -> Result<()> {
    let loopback = config
        .bind
        .parse::<std::net::IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false);
    if !loopback {
        return Err(ForgeError::invalid_argument(format!(
            "network mock scenario {} requires a loopback host, got '{}'",
            config.scenario.as_str(),
            config.bind
        ))
        .with_suggestion(
            "Scenarios are host-only: use 127.0.0.1 or ::1; never bind an external interface.",
        ));
    }
    if !config.redact {
        return Err(ForgeError::invalid_argument(format!(
            "network mock scenario {} must redact request logs",
            config.scenario.as_str()
        )));
    }
    if config.max_requests == 0 || config.duration_ms == 0 || config.max_response_bytes == 0 {
        return Err(ForgeError::invalid_argument(format!(
            "network mock scenario {} must stay bounded (max_requests, duration_ms and max_response_bytes must be positive)",
            config.scenario.as_str()
        )));
    }
    if config.max_response_bytes > MAX_BODY_BYTES {
        return Err(ForgeError::invalid_argument(format!(
            "network mock scenario {} max_response_bytes exceeds the {} byte server cap",
            config.scenario.as_str(),
            MAX_BODY_BYTES
        )));
    }
    if config.mode == ScenarioMode::Offline {
        if !config.routes.is_empty() {
            return Err(ForgeError::invalid_argument(format!(
                "network mock scenario {} is offline and must not declare routes",
                config.scenario.as_str()
            )));
        }
        return Ok(());
    }
    if config.routes.is_empty() {
        return Err(ForgeError::invalid_argument(format!(
            "network mock scenario {} needs at least one fixture route",
            config.scenario.as_str()
        )));
    }
    for route in &config.routes {
        validate_route(route)?;
        if route.body.len() > config.max_response_bytes {
            return Err(ForgeError::invalid_argument(format!(
                "network mock scenario {} route {} {} has an oversized response ({} bytes > {})",
                config.scenario.as_str(),
                route.method,
                route.path,
                route.body.len(),
                config.max_response_bytes
            )));
        }
    }
    Ok(())
}

/// Deterministic offline outcome: no listener is created and no host network
/// setting is changed. The configured port is reported for schema compatibility.
pub fn offline_outcome(config: &ScenarioConfig, port: u16, log_path: &Path) -> ServerOutcome {
    ServerOutcome {
        bind: config.bind.clone(),
        port,
        tls: false,
        requests: 0,
        log_path: log_path.to_path_buf(),
        routes: 0,
        stopped_by: "offline".to_string(),
        scenario: Some(config.scenario),
    }
}

#[derive(Debug, Serialize)]
struct RequestLog {
    at: String,
    method: String,
    path: String,
    query: Option<String>,
    headers: BTreeMap<String, String>,
    body_bytes: usize,
    body_sha256: String,
    body_truncated: bool,
    body_preview: Option<String>,
    status: u16,
    matched_route: Option<String>,
    duration_ms: u64,
    peer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scenario: Option<String>,
}

#[derive(Debug)]
pub struct ServerOutcome {
    pub bind: String,
    pub port: u16,
    pub tls: bool,
    pub requests: u64,
    pub log_path: PathBuf,
    pub routes: usize,
    pub stopped_by: String,
    pub scenario: Option<ScenarioName>,
}

pub async fn network_mock(
    ctx: &CliContext,
    command: &NetworkMockCommand,
) -> Result<CommandSuccess> {
    match command {
        NetworkMockCommand::Serve(args) => {
            serve_scenario(ctx, args, args.scenario.as_deref()).await
        }
        NetworkMockCommand::Scenarios => {
            let scenarios = scenario_catalog();
            let count = scenarios.len();
            Ok(success(
                "network.mock.scenarios",
                serde_json::json!({"scenarios": scenarios}),
                if scenarios.is_empty() {
                    "No network mock scenarios are available.\n".to_string()
                } else {
                    format!(
                        "{count} loopback-only network mock scenario(s): {}\n",
                        scenarios
                            .iter()
                            .map(|scenario| scenario.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            ))
        }
    }
}

/// Run the mock server, optionally from a named scenario.
///
/// `None` keeps the existing raw route-driven behavior. A scenario resolves its
/// routes from fixture JSON, is validated as loopback-only, bounded, redacted
/// and host-only, and never changes host proxy/profile settings or device state.
/// `network mock serve --scenario` passes the name through to this function.
pub async fn serve_scenario(
    ctx: &CliContext,
    args: &NetworkMockServeArgs,
    scenario: Option<&str>,
) -> Result<CommandSuccess> {
    if args.port == 0 {
        return Err(ForgeError::invalid_argument(
            "network mock serve requires a port between 1 and 65535",
        ));
    }
    let scenario_config = scenario
        .map(|name| {
            let options = ScenarioOptions {
                bind: Some(args.bind.clone()),
                ..ScenarioOptions::default()
            };
            resolve_scenario(name, &options)
        })
        .transpose()?;
    if scenario_config.is_some() && args.reverse {
        return Err(ForgeError::invalid_argument(
            "network mock scenarios are host-only; --reverse is not supported",
        )
        .with_suggestion("Use raw `network mock serve` if you need an adb reverse mapping."));
    }
    if scenario_config.is_none() {
        let loopback = args
            .bind
            .parse::<std::net::IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false);
        if !loopback && !args.allow_external_bind {
            return Err(ForgeError::invalid_argument(
                "network mock serve only binds loopback by default because it has no authentication",
            )
            .with_suggestion(
                "Pass --allow-external-bind only on a trusted network, or keep 127.0.0.1 and use adb reverse.",
            ));
        }
    }
    let routes = if let Some(config) = &scenario_config {
        config.routes.clone()
    } else {
        load_routes(ctx, args)?
    };
    if routes.is_empty()
        && scenario_config
            .as_ref()
            .is_none_or(|config| config.mode != ScenarioMode::Offline)
    {
        return Err(ForgeError::invalid_argument(
            "network mock serve needs at least one route via --route or --routes",
        )
        .with_suggestion(
            "Example: --route 'GET /health=200:{\"ok\":true}' --route 'POST /login=401'",
        ));
    }
    let session = ctx.load_session(true)?;
    let device = if args.reverse {
        Some(ctx.resolve_device(session.as_ref()).await?)
    } else {
        None
    };
    let tls = match (&args.tls_cert, &args.tls_key) {
        (Some(_), Some(_)) => true,
        (None, None) => false,
        _ => {
            return Err(ForgeError::invalid_argument(
                "TLS requires both --tls-cert and --tls-key",
            ));
        }
    };
    let duration = if let Some(config) = &scenario_config {
        Some(Duration::from_millis(config.duration_ms))
    } else {
        args.duration
            .as_deref()
            .map(androidforge_core::util::parse_duration)
            .transpose()
            .map_err(ForgeError::invalid_argument)?
    };
    let bind = scenario_config
        .as_ref()
        .map(|config| config.bind.as_str())
        .unwrap_or(args.bind.as_str());
    let max_requests = scenario_config
        .as_ref()
        .map(|config| config.max_requests)
        .unwrap_or(args.max_requests);
    if ctx.dry_run {
        return Ok(success(
            "network.mock.serve",
            serde_json::json!({
                "dry_run": true,
                "bind": bind,
                "port": args.port,
                "tls": tls,
                "routes": routes.len(),
                "reverse": args.reverse,
                "max_requests": max_requests,
                "duration_ms": duration.map(|value| value.as_millis() as u64),
                "scenario": scenario_config.as_ref().map(|config| config.scenario.as_str()),
                "scenario_mode": scenario_config.as_ref().map(|config| config.mode.as_str()),
            }),
            if let Some(config) = &scenario_config {
                format!(
                    "Would run scenario '{}' ({}) on {}:{} with {} fixture route(s).\n",
                    config.scenario.as_str(),
                    config.mode.as_str(),
                    bind,
                    args.port,
                    routes.len()
                )
            } else {
                format!(
                    "Would serve {} route(s) on {}:{}{}{}\n",
                    routes.len(),
                    args.bind,
                    args.port,
                    if tls { " (TLS)" } else { "" },
                    if args.reverse {
                        format!(" and reverse tcp:{} to the device", args.port)
                    } else {
                        String::new()
                    }
                )
            },
        ));
    }
    let log_override = args.log.as_ref().map(|path| local_path(ctx, path));
    execute_in_run!(
        ctx,
        "network.mock.serve",
        Some("network mock serve"),
        session.as_ref(),
        device.as_ref(),
        true,
        |run| async {
            let log_path = log_override
                .clone()
                .unwrap_or_else(|| run.dir.join("mock").join("requests.ndjson"));
            if let Some(parent) = log_path.parent() {
                ensure_dir(parent)?;
            }
            std::fs::File::create(&log_path).map_err(|error| {
                ForgeError::new(
                    ErrorCode::ArtifactWriteFailed,
                    format!("could not create mock request log: {error}"),
                )
            })?;
            let adb = if args.reverse { Some(ctx.adb()?) } else { None };
            let mut reversed = false;
            if let Some(adb) = adb.as_ref()
                && let Some(device) = device.as_ref()
            {
                adb.reverse_add(
                    &device.serial,
                    &format!("tcp:{}", args.port),
                    &format!("tcp:{}", args.port),
                )
                .await?;
                reversed = true;
            }
            let server = serve_inner(ctx, args, &routes, &log_path, tls, scenario_config.as_ref())
                .await;
            if reversed
                && let Some(adb) = adb.as_ref()
                && let Some(device) = device.as_ref()
                && let Err(error) = adb
                    .reverse_remove(&device.serial, &format!("tcp:{}", args.port))
                    .await
            {
                return Err(error.with_context_string(
                    "reverse_cleanup_error",
                    "adb reverse cleanup failed after the mock server stopped",
                ));
            }
            let outcome = server?;
            let artifact = if log_path.starts_with(&run.dir) {
                run.register_artifact(
                    &log_path,
                    "mock_request_log",
                    Some("NDJSON request log from the local mock server"),
                )?
            } else {
                androidforge_core::artifact::copy_into(
                    &run.dir,
                    &log_path,
                    "mock/requests.ndjson",
                    "mock_request_log",
                    Some("NDJSON request log from the local mock server"),
                )?
            };
            let value = serde_json::json!({
                "bind": outcome.bind,
                "port": outcome.port,
                "tls": outcome.tls,
                "requests": outcome.requests,
                "routes": outcome.routes,
                "stopped_by": outcome.stopped_by,
                "log_path": outcome.log_path.to_string_lossy(),
                "reverse": args.reverse,
                "scenario": outcome.scenario.map(|name| name.as_str()),
                "scenario_mode": scenario_config.as_ref().map(|config| config.mode.as_str()),
            });
            let message = match (outcome.scenario, outcome.stopped_by.as_str()) {
                (Some(scenario), "offline") => format!(
                    "Scenario '{}' created no listener; no host network setting was changed.\nrequest log: {}\n",
                    scenario.as_str(),
                    outcome.log_path.display()
                ),
                _ => format!(
                    "Mock server stopped after {} request(s) on {}:{}{}.\nrequest log: {}\n",
                    outcome.requests,
                    outcome.bind,
                    outcome.port,
                    if outcome.tls { " (TLS)" } else { "" },
                    outcome.log_path.display()
                ),
            };
            let mut command_success = success("network.mock.serve", value, message)
                .with_artifacts(vec![artifact])
                .with_optional_session(session.as_ref().map(|session| session.id.clone()));
            if let Some(device) = device.as_ref() {
                command_success = command_success.with_device(device.serial.clone());
            }
            Ok(command_success)
        },
    )
    .await
}

fn load_routes(ctx: &CliContext, args: &NetworkMockServeArgs) -> Result<Vec<MockRoute>> {
    let mut routes = Vec::new();
    if let Some(path) = &args.routes {
        let path = local_path(ctx, path);
        let text = std::fs::read_to_string(&path).map_err(|error| {
            ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!("could not read mock routes {}: {error}", path.display()),
            )
        })?;
        let file: RouteFile = serde_json::from_str(&text).map_err(|error| {
            ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!("invalid mock routes JSON {}: {error}", path.display()),
            )
        })?;
        routes.extend(file.routes);
    }
    for spec in &args.route {
        routes.push(parse_inline_route(spec)?);
    }
    for route in &routes {
        validate_route(route)?;
    }
    Ok(routes)
}

fn validate_route(route: &MockRoute) -> Result<()> {
    if route.path.is_empty() || !route.path.starts_with('/') {
        return Err(ForgeError::invalid_argument(format!(
            "mock route path '{}' must start with /",
            route.path
        )));
    }
    if !(100..=599).contains(&route.status) {
        return Err(ForgeError::invalid_argument(format!(
            "mock route status {} is not a valid HTTP status",
            route.status
        )));
    }
    for (name, value) in &route.headers {
        if name.contains('\r')
            || name.contains('\n')
            || name.contains(':')
            || value.contains('\r')
            || value.contains('\n')
        {
            return Err(ForgeError::invalid_argument(
                "mock route headers cannot contain CR, LF or a colon in the name",
            ));
        }
    }
    Ok(())
}

fn parse_inline_route(spec: &str) -> Result<MockRoute> {
    let Some((left, right)) = spec.split_once('=') else {
        return Err(ForgeError::invalid_argument(format!(
            "mock route '{spec}' must look like 'METHOD /path=STATUS[:BODY]'"
        )));
    };
    let mut left = left.split_whitespace();
    let method = left.next().unwrap_or_default().to_ascii_uppercase();
    let path = left.next().unwrap_or_default();
    if method.is_empty() || path.is_empty() {
        return Err(ForgeError::invalid_argument(format!(
            "mock route '{spec}' must look like 'METHOD /path=STATUS[:BODY]'"
        )));
    }
    let (status, body) = match right.split_once(':') {
        Some((status, body)) => (status, body.to_string()),
        None => (right, String::new()),
    };
    let status = status.trim().parse::<u16>().map_err(|_| {
        ForgeError::invalid_argument(format!("mock route '{spec}' has an invalid status"))
    })?;
    Ok(MockRoute {
        method,
        path: path.to_string(),
        status,
        headers: BTreeMap::new(),
        body,
        delay_ms: 0,
    })
}

async fn serve_inner(
    ctx: &CliContext,
    args: &NetworkMockServeArgs,
    routes: &[MockRoute],
    log_path: &Path,
    tls: bool,
    scenario: Option<&ScenarioConfig>,
) -> Result<ServerOutcome> {
    let bind = scenario
        .map(|config| config.bind.as_str())
        .unwrap_or(args.bind.as_str());
    // Offline is explicit: no listener, no accept loop and no host network
    // change. The caller still records a bounded run and an NDJSON log.
    if let Some(config) = scenario
        && config.mode == ScenarioMode::Offline
    {
        return Ok(offline_outcome(config, args.port, log_path));
    }
    let listener = TcpListener::bind((bind, args.port))
        .await
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::CommandFailed,
                format!(
                    "could not bind mock server on {bind}:{}: {error}",
                    args.port
                ),
            )
        })?;
    let actual_port = listener
        .local_addr()
        .map(|addr| addr.port())
        .unwrap_or(args.port);
    let tls_acceptor = if tls {
        let cert = args.tls_cert.as_ref().expect("validated");
        let key = args.tls_key.as_ref().expect("validated");
        Some(tls_acceptor(cert, key)?)
    } else {
        None
    };
    let limit = match scenario {
        Some(config) => Some(config.max_requests),
        None if args.max_requests == 0 => None,
        None => Some(args.max_requests),
    };
    let deadline = match scenario {
        Some(config) => Some(Duration::from_millis(config.duration_ms)),
        None => args.duration.as_deref().map(parse_duration).transpose()?,
    };
    let started = Instant::now();
    let deadline_at = deadline.map(|duration| started + duration);
    let counts = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(Notify::new());
    // Scenario logs are always redacted, even if the host config disabled the
    // general redactor; non-scenario behavior is unchanged.
    let redactor = if scenario.is_some() {
        Redactor::default()
    } else {
        ctx.redactor()
    };
    let mut tasks = JoinSet::new();
    let mut stopped_by;
    loop {
        if let Some(limit) = limit
            && counts.load(Ordering::SeqCst) >= limit
        {
            stopped_by = "max_requests".to_string();
            break;
        }
        let accept = listener.accept();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                stopped_by = "signal".to_string();
                break;
            }
            _ = wait_deadline(deadline_at) => {
                stopped_by = "duration".to_string();
                break;
            }
            _ = stop.notified() => {
                stopped_by = "max_requests".to_string();
                break;
            }
            accepted = accept => {
                if tasks.len() >= 64 {
                    let _ = tasks.join_next().await;
                }
                let (stream, peer) = accepted.map_err(|error| {
                    ForgeError::new(
                        ErrorCode::CommandFailed,
                        format!("mock accept failed: {error}"),
                    )
                })?;
                let routes = routes.to_vec();
                let tls_acceptor = tls_acceptor.clone();
                let log_path = log_path.to_path_buf();
                let max_body = args.max_body_bytes;
                let default_status = args.default_status;
                let default_body = args.default_body.clone();
                let delay_ms = args.delay_ms;
                let counts = Arc::clone(&counts);
                let stop = Arc::clone(&stop);
                let redactor = redactor.clone();
                let scenario_name = scenario.map(|config| config.scenario.as_str().to_string());
                tasks.spawn(async move {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(15),
                        handle_connection(
                        stream,
                        peer.to_string(),
                        tls_acceptor,
                        routes,
                        log_path,
                        max_body,
                        default_status,
                        default_body,
                        delay_ms,
                        redactor,
                        scenario_name,
                    ))
                    .await;
                    let total = counts.fetch_add(1, Ordering::SeqCst) + 1;
                    if let Some(limit) = limit
                        && total >= limit
                    {
                        stop.notify_waiters();
                    }
                });
            }
        }
        while tasks.try_join_next().is_some() {}
    }
    while tasks.join_next().await.is_some() {}
    let requests = counts.load(Ordering::SeqCst);
    if limit.is_some() && stopped_by == "signal" && requests >= limit.unwrap_or(u64::MAX) {
        stopped_by = "max_requests".to_string();
    }
    let _ = ctx;
    Ok(ServerOutcome {
        bind: bind.to_string(),
        port: actual_port,
        tls,
        requests,
        log_path: log_path.to_path_buf(),
        routes: routes.len(),
        stopped_by,
        scenario: scenario.map(|config| config.scenario),
    })
}

fn parse_duration(value: &str) -> Result<Duration> {
    androidforge_core::util::parse_duration(value).map_err(ForgeError::invalid_argument)
}

async fn wait_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await,
        None => std::future::pending().await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_connection(
    stream: TcpStream,
    peer: String,
    tls_acceptor: Option<tokio_rustls::TlsAcceptor>,
    routes: Vec<MockRoute>,
    log_path: PathBuf,
    max_body: usize,
    default_status: u16,
    default_body: Option<String>,
    delay_ms: u64,
    redactor: Redactor,
    scenario: Option<String>,
) -> Result<()> {
    let mut stream: Box<dyn AsyncStream> = Box::new(stream);
    if let Some(acceptor) = tls_acceptor {
        let tls = acceptor.accept(stream).await.map_err(|error| {
            ForgeError::new(
                ErrorCode::CommandFailed,
                format!("mock TLS handshake failed: {error}"),
            )
        })?;
        stream = Box::new(tls);
    }
    let started = Instant::now();
    let mut buffer = Vec::new();
    let header_end = read_until_headers(&mut stream, &mut buffer).await?;
    let header = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = header.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_ascii_uppercase();
    let target = parts.next().unwrap_or("/");
    let (path, query) = target
        .split_once('?')
        .map(|(path, query)| (path.to_string(), Some(redactor.redact(query))))
        .unwrap_or_else(|| (target.to_string(), None));
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        respond(
            &mut stream,
            413,
            "Payload Too Large",
            &[("content-type", "application/json")],
            b"{\"error\":\"request body too large\"}",
        )
        .await?;
        log_request(
            &log_path,
            RequestLog {
                at: now(),
                method,
                path,
                query,
                headers: redact_headers(headers),
                body_bytes: 0,
                body_sha256: sha256_hex(b""),
                body_truncated: false,
                body_preview: None,
                status: 413,
                matched_route: None,
                duration_ms: started.elapsed().as_millis() as u64,
                peer: Some(peer),
                scenario: scenario.clone(),
            },
        )?;
        return Ok(());
    }
    while buffer.len() < header_end + content_length {
        let remaining = header_end + content_length - buffer.len();
        let mut chunk = vec![0u8; remaining.min(16 * 1024)];
        let read = stream.read(&mut chunk).await.map_err(|error| {
            ForgeError::new(
                ErrorCode::CommandFailed,
                format!("mock request read failed: {error}"),
            )
        })?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = buffer[header_end..(header_end + content_length).min(buffer.len())].to_vec();
    let route = match_route(&routes, &method, &path);
    let status = route.map(|route| route.status).unwrap_or(default_status);
    let response_body = route
        .map(|route| route.body.clone())
        .or_else(|| default_body.clone())
        .unwrap_or_else(|| format!("{{\"error\":\"no mock route for {method} {path}\"}}"));
    let mut response_headers = route.map(|route| route.headers.clone()).unwrap_or_default();
    if !response_headers
        .keys()
        .any(|key| key.eq_ignore_ascii_case("content-type"))
    {
        response_headers.insert(
            "content-type".to_string(),
            if response_body.trim_start().starts_with('{')
                || response_body.trim_start().starts_with('[')
            {
                "application/json".to_string()
            } else {
                "text/plain; charset=utf-8".to_string()
            },
        );
    }
    let route_delay = route.map(|route| route.delay_ms).unwrap_or(0).max(delay_ms);
    if route_delay > 0 {
        tokio::time::sleep(Duration::from_millis(route_delay)).await;
    }
    let header_pairs = response_headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    respond(
        &mut stream,
        status,
        reason_phrase(status),
        &header_pairs,
        response_body.as_bytes(),
    )
    .await?;
    let _ = stream.shutdown().await;
    let (body_preview, truncated) = redact_preview(&redactor, &body, max_body);
    log_request(
        &log_path,
        RequestLog {
            at: now(),
            method,
            path,
            query,
            headers: redact_headers(headers),
            body_bytes: body.len(),
            body_sha256: sha256_hex(&body),
            body_truncated: truncated,
            body_preview: Some(body_preview),
            status,
            matched_route: route.map(|route| format!("{} {}", route.method, route.path)),
            duration_ms: started.elapsed().as_millis() as u64,
            peer: Some(peer),
            scenario,
        },
    )
}

fn redact_preview(redactor: &Redactor, body: &[u8], max_body: usize) -> (String, bool) {
    let truncated = body.len() > max_body;
    let preview = &body[..body.len().min(max_body)];
    (
        redactor.redact(&String::from_utf8_lossy(preview)),
        truncated,
    )
}

async fn read_until_headers<S: AsyncRead + Unpin>(
    stream: &mut S,
    buffer: &mut Vec<u8>,
) -> Result<usize> {
    loop {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await.map_err(|error| {
            ForgeError::new(
                ErrorCode::CommandFailed,
                format!("mock request read failed: {error}"),
            )
        })?;
        if read == 0 {
            return Err(ForgeError::invalid_argument(
                "mock request ended before headers",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_headers_end(buffer) {
            return Ok(index);
        }
        if buffer.len() > MAX_HEADER_BYTES {
            return Err(ForgeError::invalid_argument(
                "mock request headers exceeded the 64 KiB limit",
            ));
        }
    }
}

fn find_headers_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .or_else(|| {
            bytes
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| index + 2)
        })
}

async fn respond<S: AsyncWrite + Unpin>(
    stream: &mut S,
    status: u16,
    reason: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<()> {
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::CommandFailed,
                format!("mock response write failed: {error}"),
            )
        })?;
    stream.write_all(body).await.map_err(|error| {
        ForgeError::new(
            ErrorCode::CommandFailed,
            format!("mock response write failed: {error}"),
        )
    })?;
    stream.flush().await.ok();
    Ok(())
}

fn match_route<'a>(routes: &'a [MockRoute], method: &str, path: &str) -> Option<&'a MockRoute> {
    routes.iter().find(|route| {
        (route.method == "*" || route.method.eq_ignore_ascii_case(method))
            && (route.path == path || route.path == "*")
    })
}

fn redact_headers(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    const SENSITIVE: &[&str] = &[
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "x-api-key",
        "api-key",
        "x-auth-token",
        "x-access-token",
        "x-csrf-token",
        "x-xsrf-token",
    ];
    headers
        .into_iter()
        .map(|(name, value)| {
            if SENSITIVE.contains(&name.as_str()) {
                (name, "[redacted]".to_string())
            } else {
                (name, value)
            }
        })
        .collect()
}

fn log_request(path: &Path, log: RequestLog) -> Result<()> {
    use std::io::Write;
    let line = serde_json::to_string(&log).map_err(|error| {
        ForgeError::new(
            ErrorCode::Internal,
            format!("could not serialize mock request log: {error}"),
        )
    })?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::ArtifactWriteFailed,
                format!("could not append mock request log: {error}"),
            )
        })?;
    writeln!(file, "{line}").map_err(|error| {
        ForgeError::new(
            ErrorCode::ArtifactWriteFailed,
            format!("could not write mock request log: {error}"),
        )
    })?;
    Ok(())
}

fn tls_acceptor(cert_path: &Path, key_path: &Path) -> Result<tokio_rustls::TlsAcceptor> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certs = load_certs(cert_path)?;
    let key = load_key(key_path)?;
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!("invalid mock TLS certificate/key: {error}"),
            )
        })?;
    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

fn load_certs(path: &Path) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    use rustls::pki_types::pem::PemObject;
    let certs = rustls::pki_types::CertificateDer::pem_file_iter(path)
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!("could not open TLS certificate {}: {error}", path.display()),
            )
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::ConfigInvalid,
                format!(
                    "could not parse TLS certificate {}: {error}",
                    path.display()
                ),
            )
        })?;
    if certs.is_empty() {
        return Err(ForgeError::new(
            ErrorCode::ConfigInvalid,
            format!(
                "TLS certificate {} contains no certificates",
                path.display()
            ),
        ));
    }
    Ok(certs)
}

fn load_key(path: &Path) -> Result<rustls::pki_types::PrivateKeyDer<'static>> {
    use rustls::pki_types::pem::PemObject;
    rustls::pki_types::PrivateKeyDer::from_pem_file(path).map_err(|error| {
        ForgeError::new(
            ErrorCode::ConfigInvalid,
            format!("could not parse TLS key {}: {error}", path.display()),
        )
    })
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Mock Response",
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn local_path(ctx: &CliContext, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        ctx.paths.cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_routes_parse_status_and_body() {
        let route = parse_inline_route("GET /health=200:{\"ok\":true}").unwrap();
        assert_eq!(route.method, "GET");
        assert_eq!(route.path, "/health");
        assert_eq!(route.status, 200);
        assert_eq!(route.body, "{\"ok\":true}");

        let empty = parse_inline_route("POST /login=401").unwrap();
        assert_eq!(empty.method, "POST");
        assert_eq!(empty.status, 401);
        assert!(empty.body.is_empty());
    }

    #[test]
    fn inline_routes_reject_bad_shapes() {
        assert!(parse_inline_route("GET /health").is_err());
        assert!(parse_inline_route("=200").is_err());
        assert!(parse_inline_route("GET /health=abc").is_err());
    }

    #[test]
    fn route_matching_respects_method_and_wildcards() {
        let routes = vec![
            parse_inline_route("GET /health=200").unwrap(),
            parse_inline_route("* /fallback=204").unwrap(),
        ];
        assert_eq!(match_route(&routes, "GET", "/health").unwrap().status, 200);
        assert_eq!(
            match_route(&routes, "GET", "/fallback").unwrap().status,
            204
        );
        assert_eq!(
            match_route(&routes, "POST", "/fallback").unwrap().status,
            204
        );
        assert!(match_route(&routes, "POST", "/health").is_none());
        assert!(match_route(&routes, "GET", "/missing").is_none());
    }

    #[test]
    fn sensitive_headers_are_redacted() {
        let headers = BTreeMap::from([
            ("authorization".to_string(), "Bearer secret".to_string()),
            ("accept".to_string(), "application/json".to_string()),
        ]);
        let redacted = redact_headers(headers);
        assert_eq!(redacted["authorization"], "[redacted]");
        assert_eq!(redacted["accept"], "application/json");
    }

    fn write_fixture(dir: &Path, name: &str, json: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, json).expect("write fixture");
        path
    }

    #[test]
    fn scenario_catalog_lists_the_four_named_scenarios() {
        let catalog = scenario_catalog();
        assert_eq!(list_scenarios(), catalog);
        assert_eq!(catalog.len(), 4);
        let names: Vec<&str> = catalog.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["newer-release", "offline", "bad-digest", "no-APK"]
        );
        assert_eq!(catalog[1].mode, ScenarioMode::Offline);
        assert!(catalog.iter().all(|entry| {
            entry
                .fixture_file
                .starts_with(".androidforge/network-mock/")
                && !entry.description.is_empty()
        }));
    }

    #[test]
    fn scenario_name_parse_is_canonical_and_rejects_unknown() {
        for (input, expected) in [
            ("newer-release", ScenarioName::NewerRelease),
            ("NEWer-Release", ScenarioName::NewerRelease),
            ("no-apk", ScenarioName::NoApk),
            (" NO-APK ", ScenarioName::NoApk),
            ("bad-digest", ScenarioName::BadDigest),
            ("offline", ScenarioName::Offline),
        ] {
            assert_eq!(ScenarioName::parse(input).unwrap(), expected);
        }
        let error = ScenarioName::parse("github-release").unwrap_err();
        assert!(
            error.message().contains("unknown network mock scenario"),
            "{}",
            error.message()
        );
        assert!(lookup_scenario("does-not-exist").is_err());
    }

    #[test]
    fn scenario_lookup_exposes_definition_and_defaults() {
        let definition = lookup_scenario("bad-digest").unwrap();
        assert_eq!(definition.name, ScenarioName::BadDigest);
        assert_eq!(definition.mode, ScenarioMode::Serve);
        assert_eq!(
            definition.fixture_file,
            ".androidforge/network-mock/bad-digest.json"
        );
        assert!(definition.defaults.redact);
        assert_eq!(definition.defaults.max_response_bytes, 256 * 1024);
        assert_eq!(definition.defaults.max_requests, 64);
        assert_eq!(definition.defaults.duration_ms, 30_000);
        assert!(definition.fixture_json.contains("sha256"));
    }

    #[test]
    fn scenario_resolution_generates_fixture_routes_without_host_semantics() {
        let config = resolve_scenario("newer-release", &ScenarioOptions::default()).unwrap();
        assert_eq!(config.scenario, ScenarioName::NewerRelease);
        assert_eq!(config.mode, ScenarioMode::Serve);
        assert_eq!(config.bind, "127.0.0.1");
        assert_eq!(config.routes.len(), 2);
        let latest = config
            .routes
            .iter()
            .find(|route| route.path == "/releases/latest")
            .unwrap();
        assert_eq!(latest.method, "GET");
        assert_eq!(latest.status, 200);
        assert!(latest.body.contains("9.9.9"));
        let serialized = serde_json::to_string(&config).unwrap();
        assert!(!serialized.contains("github"), "{serialized}");
        assert!(!serialized.contains("http://"), "{serialized}");
    }

    #[test]
    fn offline_scenario_has_no_routes_and_a_deterministic_outcome() {
        let config = resolve_scenario("offline", &ScenarioOptions::default()).unwrap();
        assert_eq!(config.mode, ScenarioMode::Offline);
        assert!(config.routes.is_empty());
        assert!(config.redact);
        let outcome = offline_outcome(&config, 8080, Path::new("requests.ndjson"));
        assert_eq!(outcome.stopped_by, "offline");
        assert_eq!(outcome.requests, 0);
        assert_eq!(outcome.routes, 0);
        assert_eq!(outcome.bind, "127.0.0.1");
        assert_eq!(outcome.port, 8080);
        assert_eq!(outcome.scenario, Some(ScenarioName::Offline));
    }

    #[test]
    fn bad_digest_and_no_apk_payloads_come_from_fixture_routes() {
        let bad = resolve_scenario("bad-digest", &ScenarioOptions::default()).unwrap();
        let manifest = bad
            .routes
            .iter()
            .find(|route| route.path == "/releases/latest")
            .unwrap();
        assert!(
            manifest
                .body
                .contains("sha256:000000000000000000000000000000000000")
        );
        let apk = bad
            .routes
            .iter()
            .find(|route| route.path == "/download/app.apk")
            .unwrap();
        assert_eq!(apk.status, 200);
        assert_eq!(apk.body, "fixture-apk-bytes");

        let missing = resolve_scenario("no-APK", &ScenarioOptions::default()).unwrap();
        assert_eq!(missing.scenario.as_str(), "no-APK");
        let apk = missing
            .routes
            .iter()
            .find(|route| route.path == "/download/app.apk")
            .unwrap();
        assert_eq!(apk.status, 404);
        assert!(apk.body.contains("artifact unavailable"));
    }

    #[test]
    fn external_fixture_file_overrides_builtin_routes() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_fixture(
            temp.path(),
            "fixture.json",
            r#"{
                "scenario":"bad-digest",
                "mode":"serve",
                "routes":[
                    {"method":"GET","path":"/releases/latest","status":200,"body":"{\"digest\":\"custom\"}"}
                ]
            }"#,
        );
        let options = ScenarioOptions {
            fixture: Some(path.clone()),
            ..ScenarioOptions::default()
        };
        let config = resolve_scenario("BAD-DIGEST", &options).unwrap();
        assert_eq!(config.fixture, Some(path));
        assert_eq!(config.routes.len(), 1);
        assert_eq!(config.routes[0].body, "{\"digest\":\"custom\"}");
    }

    #[test]
    fn missing_fixture_file_is_an_actionable_error() {
        let temp = tempfile::tempdir().unwrap();
        let options = ScenarioOptions {
            fixture: Some(temp.path().join("missing.json")),
            ..ScenarioOptions::default()
        };
        let error = resolve_scenario("bad-digest", &options).unwrap_err();
        assert!(
            error.message().contains("missing mock scenario fixture"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn scenario_fixture_must_declare_the_requested_scenario() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_fixture(
            temp.path(),
            "wrong.json",
            r#"{"scenario":"offline","mode":"offline","routes":[]}"#,
        );
        let options = ScenarioOptions {
            fixture: Some(path),
            ..ScenarioOptions::default()
        };
        let error = resolve_scenario("bad-digest", &options).unwrap_err();
        assert!(error.message().contains("declares"), "{}", error.message());
    }

    #[test]
    fn non_loopback_hosts_are_rejected_for_scenarios() {
        for host in ["0.0.0.0", "192.168.1.10", "localhost", "example.test"] {
            let options = ScenarioOptions {
                bind: Some(host.to_string()),
                ..ScenarioOptions::default()
            };
            let error = resolve_scenario("newer-release", &options).unwrap_err();
            assert!(error.message().contains("loopback"), "{}", error.message());
        }
        let ipv6 = ScenarioOptions {
            bind: Some("::1".to_string()),
            ..ScenarioOptions::default()
        };
        assert_eq!(
            resolve_scenario("newer-release", &ipv6).unwrap().bind,
            "::1"
        );
    }

    #[test]
    fn oversized_and_unbounded_scenarios_are_rejected() {
        let oversized = ScenarioOptions {
            max_response_bytes: Some(8),
            ..ScenarioOptions::default()
        };
        let error = resolve_scenario("bad-digest", &oversized).unwrap_err();
        assert!(error.message().contains("oversized"), "{}", error.message());

        let unbounded = ScenarioOptions {
            max_requests: Some(0),
            ..ScenarioOptions::default()
        };
        let error = resolve_scenario("newer-release", &unbounded).unwrap_err();
        assert!(error.message().contains("bounded"), "{}", error.message());

        let unredacted = ScenarioOptions {
            redact: Some(false),
            ..ScenarioOptions::default()
        };
        let error = resolve_scenario("newer-release", &unredacted).unwrap_err();
        assert!(error.message().contains("redact"), "{}", error.message());
    }

    #[test]
    fn scenario_request_logs_are_redacted_and_scoped() {
        let config = resolve_scenario("newer-release", &ScenarioOptions::default()).unwrap();
        assert!(config.redact);
        let redactor = Redactor::default();
        let (preview, truncated) = redact_preview(&redactor, b"token=supersecretvalue&note=ok", 64);
        assert!(!preview.contains("supersecretvalue"), "{preview}");
        assert!(preview.contains("[REDACTED]"));
        assert!(!truncated);
        let (preview, truncated) = redact_preview(&redactor, b"abcdefghij", 4);
        assert_eq!(preview, "abcd");
        assert!(truncated);

        let log = RequestLog {
            at: "2026-01-01T00:00:00Z".to_string(),
            method: "GET".to_string(),
            path: "/releases/latest".to_string(),
            query: Some(redactor.redact("token=supersecretvalue")),
            headers: redact_headers(BTreeMap::from([(
                "authorization".to_string(),
                "Bearer secret".to_string(),
            )])),
            body_bytes: 5,
            body_sha256: sha256_hex(b"12345"),
            body_truncated: false,
            body_preview: Some(preview),
            status: 200,
            matched_route: None,
            duration_ms: 1,
            peer: Some("127.0.0.1".to_string()),
            scenario: Some(config.scenario.as_str().to_string()),
        };
        let value = serde_json::to_value(&log).unwrap();
        let text = value.to_string();
        assert_eq!(value["scenario"], "newer-release");
        assert_eq!(value["headers"]["authorization"], "[redacted]");
        assert!(!text.contains("supersecretvalue"), "{text}");
        assert!(!text.contains("Bearer secret"), "{text}");
    }

    #[test]
    fn route_file_and_scenario_fixture_extensions_deserialize() {
        let file: RouteFile = serde_json::from_str(
            r#"{
                "scenario":"offline",
                "defaults":{"redact":true},
                "routes":[{"method":"GET","path":"/health","status":200}]
            }"#,
        )
        .unwrap();
        assert_eq!(file.scenario.as_deref(), Some("offline"));
        assert!(file.defaults.unwrap().redact);
        assert_eq!(file.routes.len(), 1);

        let fixture: ScenarioFixture =
            serde_json::from_str(r#"{"routes":[{"path":"/x"}]}"#).unwrap();
        assert_eq!(fixture.mode, ScenarioMode::Serve);
        assert_eq!(fixture.defaults.bind, DEFAULT_SCENARIO_BIND);
        assert_eq!(
            fixture.defaults.max_response_bytes,
            DEFAULT_SCENARIO_MAX_RESPONSE_BYTES
        );
        assert_eq!(fixture.routes[0].method, "*");
        assert_eq!(fixture.routes[0].status, 200);
    }

    fn schema_validator(name: &str) -> jsonschema::Validator {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("schema")
            .join(name);
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!("read {} failed: {error}", path.display());
        });
        let schema: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!("parse {} failed: {error}", path.display());
        });
        jsonschema::validator_for(&schema).expect("compile schema")
    }

    #[test]
    fn scenario_payloads_validate_against_their_schemas() {
        let validator = schema_validator("network-mock-scenario.schema.json");
        for entry in scenario_catalog() {
            let value = serde_json::to_value(&entry).unwrap();
            assert!(validator.is_valid(&value), "{value}");
        }
        let config = resolve_scenario("no-APK", &ScenarioOptions::default()).unwrap();
        let value = serde_json::to_value(&config).unwrap();
        assert!(validator.is_valid(&value), "{value}");

        let serve_validator = schema_validator("network-mock.schema.json");
        let outcome = offline_outcome(
            &resolve_scenario("offline", &ScenarioOptions::default()).unwrap(),
            8080,
            Path::new("requests.ndjson"),
        );
        let summary = serde_json::json!({
            "bind": outcome.bind,
            "port": outcome.port,
            "tls": outcome.tls,
            "requests": outcome.requests,
            "routes": outcome.routes,
            "stopped_by": outcome.stopped_by,
            "log_path": outcome.log_path.to_string_lossy(),
            "scenario": outcome.scenario.map(|name| name.as_str()),
            "scenario_mode": "offline"
        });
        assert!(serve_validator.is_valid(&summary), "{summary}");
    }
}
