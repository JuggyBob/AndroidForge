# AndroidForge Desktop

`androidforge-desktop` is the optional native cockpit for AndroidForge. It is a separate executable built
with Rust, egui/eframe and the glow renderer. There is no WebView, HTML, JavaScript, Node or browser
frontend.

The CLI is authoritative. The desktop locates `androidforge.exe`, invokes it with argv arrays, parses the
JSON envelope and renders the CLI's own sessions, runs, errors and artifacts. It does not reimplement
device, install, UI, fixture or report logic.

## Build and run

```powershell
cargo run -p androidforge-desktop
```

The development locator checks the executable beside the desktop, a configured CLI path, the workspace
`target/debug` and `target/release` builds, then `PATH`. Use Settings to change the CLI path, workspace,
owner or theme.

## Tests

```powershell
cargo test -p androidforge-desktop
```

Unit tests cover CLI resolution, JSON contracts, evidence path safety, project discovery and native
process-tree termination. Integration tests compile a mock CLI with `rustc` and cover version/schema
probing, typed results, structured errors, invalid JSON, timeouts, streamed output and cancellation.

## Validation

`cargo fmt --check`, clippy with `-D warnings`, the workspace test suite and a release build are run in
CI. GUI flows were validated manually on Windows against the CalcSuite project; see
[../VERIFICATION.md](../VERIFICATION.md).

## Cancellation

`Cancel` is shown only for installs, fixtures, conformance and hooks. It walks the Windows process
snapshot and terminates the child tree deepest-first, with `taskkill /T /F` as a fallback. It never issues
`session stop` or `session release`. Session lifecycle operations run to completion because killing them
could leave a half-owned emulator or an inconsistent lock.

## Assets

- `assets/icon.svg` — original scalable identity.
- `assets/icon.png` — window icon source.
- `assets/icon.ico` — embedded executable icon through `build.rs` and `embed-resource`.
