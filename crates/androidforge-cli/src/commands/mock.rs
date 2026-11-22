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

trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}

#[derive(Debug, Clone, Deserialize)]
struct RouteFile {
    #[serde(default)]
    routes: Vec<MockRoute>,
}

#[derive(Debug, Clone, Deserialize)]
struct MockRoute {
    #[serde(default = "any_method")]
    method: String,
    path: String,
    #[serde(default = "ok_status")]
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    delay_ms: u64,
}

fn any_method() -> String {
    "*".to_string()
}

fn ok_status() -> u16 {
    200
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
}

#[derive(Debug)]
struct ServerOutcome {
    bind: String,
    port: u16,
    tls: bool,
    requests: u64,
    log_path: PathBuf,
    routes: usize,
    stopped_by: String,
}

pub async fn network_mock(
    ctx: &CliContext,
    command: &NetworkMockCommand,
) -> Result<CommandSuccess> {
    match command {
        NetworkMockCommand::Serve(args) => serve(ctx, args).await,
    }
}

async fn serve(ctx: &CliContext, args: &NetworkMockServeArgs) -> Result<CommandSuccess> {
    if args.port == 0 {
        return Err(ForgeError::invalid_argument(
            "network mock serve requires a port between 1 and 65535",
        ));
    }
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
    let routes = load_routes(ctx, args)?;
    if routes.is_empty() {
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
    let duration = args
        .duration
        .as_deref()
        .map(androidforge_core::util::parse_duration)
        .transpose()
        .map_err(ForgeError::invalid_argument)?;
    if ctx.dry_run {
        return Ok(success(
            "network.mock.serve",
            serde_json::json!({
                "dry_run": true,
                "bind": args.bind,
                "port": args.port,
                "tls": tls,
                "routes": routes.len(),
                "reverse": args.reverse,
                "max_requests": args.max_requests,
                "duration_ms": duration.map(|value| value.as_millis() as u64),
            }),
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
            ),
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
            let adb = ctx.adb()?;
            let mut reversed = false;
            if args.reverse
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
            let server = serve_inner(ctx, args, &routes, &log_path, tls).await;
            if reversed
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
            });
            let mut command_success = success(
                "network.mock.serve",
                value,
                format!(
                    "Mock server stopped after {} request(s) on {}:{}{}.\nrequest log: {}\n",
                    outcome.requests,
                    outcome.bind,
                    outcome.port,
                    if outcome.tls { " (TLS)" } else { "" },
                    outcome.log_path.display()
                ),
            )
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
    }
    Ok(routes)
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
) -> Result<ServerOutcome> {
    let listener = TcpListener::bind((args.bind.as_str(), args.port))
        .await
        .map_err(|error| {
            ForgeError::new(
                ErrorCode::CommandFailed,
                format!(
                    "could not bind mock server on {}:{}: {error}",
                    args.bind, args.port
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
    let limit = if args.max_requests == 0 {
        None
    } else {
        Some(args.max_requests)
    };
    let deadline = args.duration.as_deref().map(parse_duration).transpose()?;
    let started = Instant::now();
    let deadline_at = deadline.map(|duration| started + duration);
    let counts = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(Notify::new());
    let redactor = ctx.redactor();
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
        bind: args.bind.clone(),
        port: actual_port,
        tls,
        requests,
        log_path: log_path.to_path_buf(),
        routes: routes.len(),
        stopped_by,
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
        .map(|(path, query)| (path.to_string(), Some(query.to_string())))
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
    let truncated = body.len() > max_body;
    let preview_bytes = &body[..body.len().min(max_body)];
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
            body_preview: Some(redactor.redact(&String::from_utf8_lossy(preview_bytes))),
            status,
            matched_route: route.map(|route| format!("{} {}", route.method, route.path)),
            duration_ms: started.elapsed().as_millis() as u64,
            peer: Some(peer),
        },
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
}
