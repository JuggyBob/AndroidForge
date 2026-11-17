# AndroidForge

**Local-first Android testing and device orchestration for AI coding agents.**

[![CI](https://github.com/JuggyBob/AndroidForge/actions/workflows/ci.yml/badge.svg)](https://github.com/JuggyBob/AndroidForge/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Status: Alpha](https://img.shields.io/badge/status-alpha-orange.svg)](#project-status)

> **Alpha `v0.7.0`.** AndroidForge is usable today for emulator-driven development and agent
> workflows, but it is not a finished product. Windows is the tested host, and physical devices,
> Maestro, instrumentation, snapshot restore and root-only operations are not yet validated. Read
> [Project status](#project-status) before relying on it.

AndroidForge is a local-first Rust toolkit: a single native CLI for automation plus an optional native
desktop cockpit for humans. The CLI gives an AI coding agent a reliable way to acquire an Android
device, install a build, inspect and drive the UI, prepare fixture state, collect evidence and return
machine-readable results — without Android Studio, fragile coordinate scripts, a daemon or a cloud
service. The desktop app is a separate `androidforge-desktop.exe` that drives the same CLI and its JSON
contract; it never reimplements or replaces the CLI.

The same CLI binary runs every project. Project-specific behavior lives in `.androidforge/project.toml`,
fixtures, hooks, conformance manifests and scripts. The Rust core contains no app names, package ids or
project branches.

## Why this exists

Agent-driven Android testing usually fails at the same places:

- a device is already in use, or the agent cannot tell which device it owns;
- an APK installs, but nobody checks what was actually installed;
- UI taps work by coordinates until the screen changes;
- a failed assertion leaves no screenshot, hierarchy or logcat;
- a test reports success because `adb` exited 0, even though the app did nothing;
- the agent has to parse human text instead of a stable JSON contract.

AndroidForge makes those cases explicit. Sessions coordinate device ownership. Installs record
package, version, APK hash and signing certificate. UI actions use the accessibility hierarchy. Runs
produce a JSON report and checksum manifest. Failures attach evidence. Hooks can declare postconditions.
Unsupported operations say so with `E_UNSUPPORTED_OPERATION` and alternatives.

## Designed for AI agents

- **JSON on stdout, diagnostics on stderr.** No progress bars or human text mixed into machine output.
- **Stable exit codes and structured errors.** Every error includes a code, message, context,
  suggestions and artifact paths where available.
- **Bounded execution.** External processes have timeouts and process-tree cleanup. Long-lived
  operations have explicit status and stop commands.
- **Device ownership.** A session binds an owner, device, optional emulator, heartbeat and prepared
  settings. Another agent gets `E_DEVICE_BUSY` instead of silently interfering.
- **One run, one report.** `run start`/`run end`, `ANDROIDFORGE_RUN_ID` and `report show` make a
  multi-command flow inspectable after the fact.
- **Evidence by default.** Failed app/UI/conformance steps capture a screenshot, hierarchy, logcat,
  process state and relevant dumps.
- **Project context, not hidden magic.** `--project`, `--session`, `--serial`, `--run`, `--json`,
  `--trace`, `--dry-run` and `--yes` are explicit.
- **Unmodified scripts still work.** `run -- python script.py` injects the environment contract and
  joins the existing report.

If you are wiring an agent, start with [AGENT_GUIDE.md](AGENT_GUIDE.md) and
[EXIT_CODES.md](EXIT_CODES.md). The machine-readable schemas are in [schema/](schema/).

## Features

| Area | What AndroidForge provides |
| --- | --- |
| Devices | ADB discovery, `devices list/inspect/wait/health`, wireless pair/connect/disconnect, preparation and restore |
| Emulators | AVD listing, safe port allocation, headless start, boot wait, status, stop, snapshots |
| Sessions | Claim/attach/release/stop/restart, owner guards, heartbeats, stale-session GC |
| APKs | `aapt2`/`apksigner` inspection, signature and package checks, install identity verification |
| Apps | Launch, status, force-stop, uninstall, clear data, file push/pull for debuggable builds |
| UI | Hierarchy dump/find/assert/tap/text/clear/back/scroll/settle/wait, semantic selectors, stable element refs |
| Evidence | Screenshots, screen recording, logcat, crashes/ANRs, dumpsys, artifact manifests |
| Diagnostics | Notifications, channels, jobs, alarms, permissions, AppOps, Doze, battery, network, time |
| Automation | Versioned fixtures, hooks with postconditions, conformance manifests, JUnit import |
| Agent context | `context`, compact observation tokens, `ui diff`, stable refs, action receipts, postconditions and exact next-action argv |
| Diagnosis | Deterministic failure categories, confidence, evidence, stable fingerprints, recurrence memory, `report explain/history/recurrences` and diagnostic bundles |
| Orchestration | Idempotent `session ensure`, read-only `doctor fix --plan`, transparent `agent test`, APK resolution, `--only-if-changed` and sequential `agent matrix` |
| Desktop | Native egui/eframe cockpit: onboarding, context, matrix, ownership, devices with health/wireless, sessions, runs, triage with explanations/receipts/recurrence/bundle previews, comparison, live logcat, inspector, repair planning, maintenance and updates |
| Analysis | Failure triage, compatible run comparison, flaky-step analysis, copyable Markdown/JSON reports |
| Storage safety | Dry-run cleanup previews, terminal-only pruning, active-session protection, stale-session GC |
| Integration | Raw ADB passthrough, external `run --` scripts, project adapters, JSON schemas |

## What is new in v0.7.0

- **Sequential device matrix:** `agent matrix` plans and runs the same flow on multiple independent AVDs/serials, with one owned session per target, per-device health, artifacts, aggregate `passed`/`failed`/`partial` status, fail-fast and explicit cleanup. Parallel execution is rejected until validated.
- **Stable semantic refs:** `ui observe` returns `ref` values. `ui tap --ref ... --expect-token ...` (and long-press) validates session, device, observation token and current node identity immediately before dispatch. Stale or changed refs fail closed without text/coordinate fallback.
- **Wireless ADB and health:** `devices pair` (stdin-safe code), `connect`, `disconnect` and a read-only `devices health` report covering battery, thermal, storage, memory, network, screen, uptime and unknown reasons.
- **Bounded read-only retries:** `--retries 0..5` on `ui observe`, fresh `ui diff` and logcat peek/tail, with transient classification, backoff, a 20-second window and transparent per-attempt results. No state-changing retries.
- **Desktop:** Matrix page, per-device health and wireless controls, element-ref copy actions, action receipt details and stdin-backed pairing-code handling.

## What is new in v0.6.0

- **Action receipts:** `ui tap`, `ui long-press`, `ui text` and `ui scroll` accept `--receipt` / `--receipt-full`. One command returns before/after observation tokens, a compact semantic diff, focus/package/activity transitions, a deterministic effect classification, postcondition result and explicit completeness. The action is dispatched at most once.
- **Failure recurrence memory:** `report recurrences`, `report history --fingerprint` and recurrence data in `report explain` classify failures as `new`, `recurring`, `regressed` or `unknown`. Memory is derived from existing local run reports; a passing run proves resolution only when it exercised the same project, scenario and failing step.
- **APK/build resolution:** optional `[build.variants]` patterns, conventional Gradle output fallback, `apk resolve`, `agent test --variant`, `apk diff installed:<package>` and `app install --only-if-changed`. Resolution never runs Gradle and never claims equality from version strings alone.
- **Diagnostic bundles:** `report bundle` creates a redacted, checksummed ZIP with summary, explanation, reproduction argv, evidence excerpts, bounded logcat, manifest and exclusion list. Markdown and JSON forms are available; screenshots/recordings and full logs are explicit opt-ins.
- **Desktop integration:** new Context and Ownership pages, recurrence history in Triage, and a diagnostic-bundle inclusion preview/creation workflow driven entirely by the CLI.
- **Safer persistence:** run reports now record the exact invocation argv when available; older reports remain readable and are marked as lacking exact reproduction data.

## What is new in v0.5.0

- **Honest pipeline status:** a failed `agent test` now returns envelope `status: "failed"` with its nonzero exit code, while preserving the full stage result. Blocked dry runs use `status: "blocked"`.
- **Shared-host session acquisition:** `session ensure` avoids ports already used by connected emulators, adds `--port`, binds the project on start/attach, and no longer creates misleading failed tombstones for port collisions.
- **UIAutomator recovery:** a stuck helper is detected, cleaned up and retried automatically; persistent failures use the specific `E_UI_AUTOMATOR_UNAVAILABLE` code.
- **Better diagnosis:** `report explain` classifies UIAutomator infrastructure failures as `ui_automation`, prefers the newest failed run for `latest`, and searches current/ancestor artifact stores so project-scoped calls can find unprojected runs.
- **Clearer observations:** `ui observe` now exposes `token`, separates interactive `actionable` nodes from non-interactive `context` text, always includes `clickable` and `enabled`, filters empty resource ids, and redacts password text.
- **Smaller diffs:** `ui diff` returns compact node summaries capped at 12 per category by default; `--full` raises the cap to 100. Totals and truncation remain explicit.
- **CLI/desktop polish:** `report list --all` discovers runs across artifact stores, selector errors suggest scrolling or `--clickable`, `doctor` counts active sessions instead of noisy tombstones, and the desktop answers `--help`/`--version` without opening its GUI.

## What is new in v0.4.0

- **`context`**: one compact snapshot of project, environment, owned session, device, app, screen and last run, plus exact next-action argv. Partial results and freshness are explicit.
- **`report explain`**: deterministic failure category, confidence, evidence, stable fingerprint and exact next commands. No LLM, network or invented cause.
- **`ui observe` and `ui diff`**: compact semantic observations with session-scoped, reboot-aware tokens; semantic added/removed/modified changes without pixel-noise.
- **UI postconditions**: `--wait-for-text`, `--wait-for-value`, `--wait-for-package`, `--wait-for-activity`, `--wait-for-absent-text` and `--timeout` turn action-plus-verify into one command. Actions execute once.
- **`session ensure`**: idempotent ownership-safe acquisition with warm-emulator attachment, project binding, advisory lease metadata and clear reconciliation.
- **`doctor fix --plan`**: a strictly read-only repair plan with exact component names and commands. Nothing is installed or edited.
- **`agent test`**: a transparent ensure/inspect/install/fixture/conformance/explain pipeline with `--dry-run`, stage timings, exact commands, failure evidence and no hidden destructive actions.
- **Desktop integration**: Triage shows deterministic explanations and copyable next commands, Setup shows the read-only repair plan, Sessions shows lease and project binding.

Previous desktop work from v0.3.0 is unchanged: setup, runs, triage, comparison, live logcat, device preview, semantic inspector, maintenance and update repair.

Screenshots of the desktop application are in [`docs/screenshots/v0.3.0/`](docs/screenshots/v0.3.0/); the v0.7.0 desktop keeps the same design system with Context, Matrix, Ownership, health/wireless and receipt panels.

![AndroidForge desktop overview](docs/screenshots/v0.3.0/overview-1280x720.png)

## Requirements

- **Host:** Windows 10/11 is the tested host. macOS and Linux are not yet validated.
- **Rust:** `1.95` or newer with Edition 2024 support (`rust-toolchain.toml`). The desktop crate uses
  eframe 0.37 and the glow renderer; there is no WebView, browser, Node or frontend toolchain.
- **Android SDK:** platform-tools, emulator and build-tools (`aapt2`, `apksigner`).
- **Java:** a JDK usable by `apksigner` (the SDK-bundled JDK is fine).
- **Device:** an emulator or a USB-debugging Android device.

`androidforge doctor --json` reports what is missing. Missing required SDK/ADB tooling is an
environment failure; missing Maestro is a warning.

## Install

### Download a release

1. Open [Releases](https://github.com/JuggyBob/AndroidForge/releases).
2. Choose an asset and verify it with `SHA256SUMS.txt`:
   - `androidforge-desktop-windows-x86_64.zip` - portable `androidforge.exe` + `androidforge-desktop.exe`, docs and license;
   - `androidforge-windows-x86_64-setup.exe` - per-user Windows installer with both executables, Start Menu shortcut and uninstaller;
   - `androidforge-cli-windows-x86_64.zip` - standalone `androidforge.exe` for PATH/automation.
3. For the installer, run the setup executable. No administrator rights are required. The application installs under your user profile.
4. For the portable build, extract the ZIP and launch `androidforge-desktop.exe`. Keep `androidforge.exe` beside it.
5. For the CLI only, extract the CLI zip, put `androidforge.exe` on `PATH`, and run:

```powershell
androidforge --version
androidforge doctor --json
```

The desktop searches for the CLI beside itself, then a configured path, then the development build, then `PATH`. Open Settings to point at a different CLI or workspace.

Both executables follow the same `0.7.0` version line. The desktop refuses a CLI whose version or JSON schema it does not understand rather than guessing.

> **Unsigned binaries.** The release assets are intentionally unsigned. Windows SmartScreen may warn on first launch. AndroidForge does not include a code-signing certificate or claim a verified publisher identity. Verify the published SHA-256 checksums before running downloaded files.

### Build from source

```powershell
git clone https://github.com/JuggyBob/AndroidForge.git
cd AndroidForge
cargo build --workspace --release
.\target\release\androidforge.exe doctor --json
.\target\release\androidforge-desktop.exe
```

The workspace has no runtime dependency on Python, Node, .NET or PowerShell. Java is only needed by
the Android build tools.

## Quick start

```powershell
# 1. Health check.
androidforge doctor --json
androidforge devices list --json

# 2. Start an isolated emulator session. Use the same owner for cleanup.
androidforge session start --avd Pixel_8_API_35 --port 5556 --headless --owner MyAgent --prepare --json

# 3. Install and verify a build.
androidforge app install .\app\build\outputs\apk\debug\app-debug.apk --session ses-... --json

# 4. Drive the UI semantically.
androidforge ui dump --session ses-... --json
androidforge ui wait --session ses-... --text "Welcome" --timeout 20 --json
androidforge ui tap --session ses-... --text "Continue" --json
androidforge ui text "hello world" --session ses-... --class android.widget.EditText --clear --hide-keyboard --json
androidforge ui assert --session ses-... --text "Saved" --json

# 5. Keep evidence.
androidforge capture screenshot --session ses-... --json
androidforge diagnostics collect --session ses-... --package com.example.app --json

# 6. Run a project flow.
androidforge --project .\examples\CalcSuite conformance run --session ses-... --json

# 7. Stop and release. The owner label is required.
androidforge session stop --session ses-... --owner MyAgent --json
```

### Agent loop quick start (v0.7.0)

```powershell
# One snapshot instead of several discovery commands.
androidforge context --project . --json

# Acquire or reuse one owned session, idempotently.
androidforge session ensure --project . --avd Pixel_8_API_35 --owner MyAgent --ttl 1h --json

# Observe compact semantics and keep the token.
androidforge ui observe --session ses-... --json

# Act, verify and explain the effect in one command. The tap happens once.
androidforge ui tap --session ses-... --text "Settings" --wait-for-text "Preferences" --receipt --json

# Name the semantic change since the token.
androidforge ui observe --session ses-... --since obs-... --json
androidforge ui diff --session ses-... --from obs-... --json

# Explain a failure, see whether it is new/recurring/regressed, and build evidence.
androidforge report explain latest --json
androidforge report recurrences --project . --json
androidforge report bundle latest --output .\\evidence.zip --json

# Use a stable element reference, then run a sequential device matrix.
androidforge ui observe --session ses-... --json
androidforge ui tap --session ses-... --ref el-... --expect-token obs-... --json
androidforge agent matrix --avd Pixel_8_API_35 --avd Pixel_8_API_36 --flow conformance --json

# Resolve an existing APK without running Gradle, then install only if it changed.
androidforge apk resolve --variant debug --json
androidforge app install build\\app-debug.apk --only-if-changed --json

# Plan repairs without changing the environment.
androidforge doctor fix --plan --json

# Compose the same primitives transparently, or preview them first.
androidforge --project . agent test --apk .\app-debug.apk --flow conformance --install --json
androidforge --project . agent test --apk .\app-debug.apk --flow conformance --dry-run --json
```

For a multi-step report:

```powershell
androidforge run start --session ses-... --name smoke --json
androidforge ui assert --session ses-... --text "Welcome" --json
androidforge run end --session ses-... --json
androidforge report show latest --json
```

`run end` derives `passed`/`failed` from its steps. A failed step exits non-zero; an explicit
`--status passed` is refused unless `--allow-failures` is deliberate.

## Desktop cockpit

`androidforge-desktop.exe` is an optional native companion for people who want to see and control the
same sessions, runs and evidence the CLI produces. It is built with Rust, egui/eframe and glow; there is
no WebView, JavaScript, HTML, Node or browser runtime.

The cockpit has four pages:

- **Overview** — environment health, device/session counts, recent runs and project context.
- **Devices** — serial, model, Android version, boot state, battery and current owner.
- **Sessions** — owner, heartbeat, prepared settings and validated `Release`, `Stop` and `Restart`
  actions behind explicit confirmations.
- **Runs** — report-backed run table, build identity, steps, failure details, screenshots, hierarchy,
  logs, raw files and checksum-manifest verification.

P1 actions are intentionally narrow: start an emulator, attach a device, choose and inspect an APK,
dry-run or install it, run fixtures/conformance flows/hooks, stream output and cancel a run. Cancellation
terminates the AndroidForge child process tree (never a session command) and never stops or releases the
emulator; the session reminder stays visible. Session lifecycle operations cannot be cancelled and run to
completion. A completed operation still occupies the output panel until dismissed, but no longer blocks
the next action.

The desktop is a sidecar client, not a second implementation. Every refresh and action invokes
`androidforge --json`, parses the typed envelope and shows the CLI's own error code, message,
suggestions and artifacts. Closing the desktop does not stop the CLI, an emulator or a session.

See [USER_GUIDE.md](USER_GUIDE.md#3-desktop-cockpit) for the workflow and
[ARCHITECTURE.md](ARCHITECTURE.md#desktop-cockpit) for the design.

## Configuration

A project opts into AndroidForge with `.androidforge/project.toml`:

```toml
adapter_version = 1

[project]
name = "ExampleApp"
package = "com.example.app"
debug_package = "com.example.app.debug"
expected_cert_sha256 = "AA:BB:..." # optional release certificate pin
default_avd = "Pixel_8_API_35"

[conformance]
flow = ".androidforge/conformance/release-smoke.toml"

[build.variants]
debug = ["app/build/outputs/apk/debug/*.apk"]
```

Fixtures, hooks, conformance manifests and the environment contract are documented in
[PROJECT_ADAPTER.md](PROJECT_ADAPTER.md). The adapter is optional for `doctor`, `devices`, `avd` and raw
ADB; it is required for package fallbacks, fixtures, hooks and conformance.

Precedence is CLI > environment contract > project adapter > global config > defaults. `--project`
walks upward for `.androidforge/project.toml` when omitted.

## How it works

| Crate | Responsibility |
| --- | --- |
| `androidforge-cli` | Command surface, human/JSON output, exit codes |
| `androidforge-core` | Errors, config, sessions/locks, artifacts, runs, schemas |
| `androidforge-process` | Bounded process execution, UTF-8 capture, process-tree cleanup |
| `androidforge-adb` | ADB discovery, `adb devices -l` parsing, device inspection |
| `androidforge-emulator` | AVDs, ports, headless launch, boot wait, snapshots, device prep |
| `androidforge-app` | APK inspection, install diagnosis, app/package operations |
| `androidforge-ui` | UIAutomator hierarchy, selectors, semantic actions, waits |
| `androidforge-diagnostics` | Logcat, crashes/ANRs, dumpsys, screenshots, recordings, evidence |
| `androidforge-automation` | Fixtures, hooks, conformance, Maestro, instrumentation, run exec |
| `androidforge-desktop` | Native egui cockpit, CLI locator/sidecar runner, typed models, evidence viewer, P1 actions |

Reports live under `.artifacts/runs/<run-id>/` by default and contain `report.json`, `metadata.json`
and a checksummed `manifest.json`. See [ARCHITECTURE.md](ARCHITECTURE.md) and [SCHEMA.md](SCHEMA.md).

## Safety model

- Explicit session ownership; another session gets `E_DEVICE_BUSY`.
- `session stop`, `release` and `restart` require the recorded owner or an explicit `--force`.
- Destructive app/device operations require `--yes`.
- `--dry-run` previews fixtures, installs and other meaningful operations.
- APK signatures are verified; AndroidForge never signs or auto-uninstalls on mismatch.
- The desktop confirms release/stop/restart/install/fixture/conformance/hook actions and shows the
  destructive parts in plain language.
- Cancelling a desktop run walks and terminates the AndroidForge child process tree; it does not stop,
  restart or release the emulator or session. Session lifecycle operations are not cancellable.
- No cloud, telemetry, account, API key or LLM integration.
- Raw ADB is intentionally unsafe and labelled as such.

See [SECURITY.md](SECURITY.md) for the full threat model.

## Project status

**Alpha `v0.7.0`.** The CLI has been exercised against real apps on Android 16 and Android 14
emulators, including a two-AVD matrix. v0.7.0 adds sequential device matrices, stable element refs,
wireless ADB/health and bounded read-only retries on top of v0.6.0 agent memory. See
[VERIFICATION.md](VERIFICATION.md) and [case-studies/](case-studies/).

Validated:

- Windows 10/11 host, Android 16 / API 36 emulator, x86_64.
- APK install and signature/version/hash verification.
- Semantic UI navigation, text entry, scrolling, waits and assertions.
- Session ownership and multi-agent refusal behavior.
- Fixtures, hooks with postconditions, conformance flows, JUnit import.
- Screenshots, logcat, dumpsys, notifications/jobs/alarms evidence.
- Build identity inheritance from installs into later runs.
- Native desktop onboarding, triage, comparison, flaky analysis, logcat, preview/inspector, maintenance
  and controlled P1 actions, including process-tree cancellation and run reconciliation.
- Compact `context`, semantic `ui observe`/`ui diff`, action postconditions, observation-token security,
  deterministic `report explain`, idempotent `session ensure`, read-only `doctor fix --plan` and the
  `agent test` dry-run/execute pipeline.
- v0.4.0 pipeline validation on CalcSuite, DiscountHunter and IronPulse (including a real install-skip
  decision based on recorded SHA-256 identity).
- v0.5.0 regression validation of the agent-reported issues: port selection on a shared host,
  UIAutomator recovery/classification, project-scoped run discovery, failed pipeline envelope status,
  observation token/actionable split, compact diffs, session owner flags, and desktop help/version.
- v0.6.0 real-emulator validation: receipt effect/postcondition/timeout/selector-failure cases, real
  installed APK diff and `--only-if-changed` skip/install, redacted bundle creation and verification,
  Context/Ownership desktop pages, and Triage bundle preview.
- v0.7.0 real-emulator validation: two independent AVDs (API 36 and 34), aggregate pass/partial,
  duplicate and occupancy rejection, stable-ref success/stale/wrong-token, device health, device
  disconnect/recovery, read-only retry reporting and desktop Matrix/health workflows.
- Portable ZIP and per-user installer packaging; installer upgrade preserves user data outside the
  install directory.

Not yet validated:

- Physical devices and vendor ROMs (MIUI, Samsung, etc.).
- Maestro execution and instrumentation test APKs.
- Snapshot load/restore and root-only time changes.
- macOS and Linux hosts (the desktop is Windows-validated; the CLI is not validated there either).
- Very large test suites or long-duration stress runs.
- Live update download/repair against a matching published CLI release (discovery and checksum logic are
  implemented and unit-tested; the public release carrying this version is the first usable target).
- Desktop editing of project adapters/fixtures. The cockpit is an operator console, not an IDE.

Unsupported features return `E_UNSUPPORTED_OPERATION` with a reason and alternatives. A capability
that was not exercised is not claimed as verified.

## Documentation

- [AGENT_GUIDE.md](AGENT_GUIDE.md) — operating model for autonomous agents
- [USER_GUIDE.md](USER_GUIDE.md) — setup, devices, sessions, diagnostics, troubleshooting
- [ARCHITECTURE.md](ARCHITECTURE.md) — crates, data flow, locking, process and report model
- [desktop/README.md](desktop/README.md) — building and testing the native desktop cockpit
- [PROJECT_ADAPTER.md](PROJECT_ADAPTER.md) — project config, fixtures, hooks and conformance
- [SCHEMA.md](SCHEMA.md) — report, metadata, manifest and JSON shapes
- [EXIT_CODES.md](EXIT_CODES.md) — stable exit codes and error codes
- [CAPABILITIES.md](CAPABILITIES.md) — the honest capability matrix
- [GLOSSARY.md](GLOSSARY.md) — terms used across the docs
- [VERIFICATION.md](VERIFICATION.md) — exactly what was run, including limitations
- [ACCEPTANCE.md](ACCEPTANCE.md) and [PORTFOLIO.md](PORTFOLIO.md) — acceptance flows and app coverage
- [CHANGELOG.md](CHANGELOG.md) — release history
- [SECURITY.md](SECURITY.md) — trust boundaries and private reporting

## Development

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

Run the desktop in development with `cargo run -p androidforge-desktop`. It discovers the workspace CLI
debug build first, so no extra configuration is needed. CI runs formatting, clippy, tests and a release
build for both executables on Windows. See [CONTRIBUTING.md](CONTRIBUTING.md) for branch naming, PR
expectations and design rules.

## Contributing

Bug reports, focused pull requests and real-app reproductions are welcome. Read
[CONTRIBUTING.md](CONTRIBUTING.md) first. For security issues, use the private reporting path in
[SECURITY.md](SECURITY.md), not a public issue.

## License

MIT. See [LICENSE](LICENSE).

Copyright (c) 2026 JuggyBob.
