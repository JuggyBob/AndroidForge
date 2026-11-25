# User Guide

This guide is for a human setting up AndroidForge and for scripts that prefer a stable CLI over prose. Add `--json` to any command when another program consumes the result.

## 1. Requirements

- Windows 10/11 x64 for the delivered binaries. The Rust workspace is portable in principle; process-tree cleanup is Windows-focused.
- Rust `1.95` or newer with Edition 2024 support (`rust-toolchain.toml` pins the channel).
- Android SDK with:
  - platform-tools (`adb`)
  - emulator
  - at least one AVD, if you use emulators
  - build-tools with `aapt2` and `apksigner`
  - a JDK 17 for `apksigner` (the SDK's bundled JDK is ideal)
- Optional: Maestro CLI (AndroidForge can install the pinned Windows build), an instrumentation test APK.

AndroidForge discovers the SDK from `ANDROIDFORGE_SDK_ROOT`, `ANDROID_HOME` or `ANDROID_SDK_ROOT`, then common install locations such as `%LOCALAPPDATA%\Android\Sdk`. Java is discovered from `[sdk].java`, `ANDROIDFORGE_JAVA`, `JAVA_HOME`, the SDK's `jdk` directory, then `PATH`.

## 2. Build and first check

```powershell
cargo build --workspace --release
.\target\release\androidforge.exe --version
.\target\release\androidforge.exe doctor --json
.\target\release\androidforge.exe capabilities --json
.\target\release\androidforge.exe schema list --json
.\target\release\androidforge-desktop.exe
```

`doctor` exits `0` when required tooling is present, even if optional Maestro or a missing project adapter is a warning. Disk free space is measured; less than 1 GiB is a warning. Missing ADB/SDK is an environment failure (`3`). Read `checks[].status` rather than assuming every warning is fatal. The envelope's `status: success` means the doctor command ran; `result.status` is the health summary.

### Optional Maestro installation

Maestro is optional. Native `androidforge ui` commands do not require it. If a project has Maestro flows, AndroidForge can download a pinned, checksum-verified Windows build instead of relying on a floating global install:

```powershell
androidforge tools install maestro --dry-run --json
androidforge tools install maestro --yes --json
androidforge tools status maestro --json
androidforge tools uninstall maestro --yes --json
```

- Pinned version: **Maestro CLI 2.12.1**.
- Download: the official `cli-2.12.1/maestro.zip` release asset.
- Verified SHA-256: `54dbb0716425626604973cbb374c585ca30272375d9ce15177a1152765ddd9d7`.
- Install location: `%LOCALAPPDATA%\AndroidForge\tools\maestro\2.12.1` (or the equivalent under
  `ANDROIDFORGE_HOME` / `ANDROIDFORGE_STATE_DIR`).
- Java 17 or newer is required. AndroidForge uses the JDK it already discovered from the SDK or
  `JAVA_HOME`.
- The desktop Setup → Environment page shows a **Maestro** check with an **Install Maestro** button,
  the active path and a Copy path action. A CLI-installed copy is detected automatically because both
  use the same managed tools directory.
- `tools install` requires `--yes` because it downloads about 350 MB. `--dry-run` prints the URL, hash
  and target without downloading.

Maestro remains **best-effort/optional** in the 1.1 line. The installer pins one tested version; a
manually installed copy on `PATH` is still detected through `[maestro].binary`, `[sdk].maestro` or
`ANDROIDFORGE_MAESTRO`.

## 2.1 Agent loop commands (v0.9.2)

These commands reduce repeated discovery and inspection. They keep the same JSON envelope, exit codes
and evidence rules as the rest of the CLI.

### Agent brief and output profiles

```powershell
androidforge agent brief --project . --json
androidforge agent brief --session <ID> --json
androidforge ui observe --session <ID> --output-profile compact --json
androidforge diagnostics performance com.example.app --cold --session <ID> --brief --json
```

`agent brief` is one small snapshot for the start of a turn: project, session, device, app, screen
observation, last run and deterministic next actions. It uses the brief profile automatically.

Full JSON remains the default. `--output-profile compact` / `--compact` minifies JSON, caps lists with
explicit totals and truncation flags, removes optional repetition, and adds a one-line `summary` plus
short `next` commands. `--output-profile brief` / `--brief` returns the smallest decision-oriented view
for context, observations, diffs, receipts, `agent test`, `report explain` and performance. Use
`--output-profile full` to override a compact default. Defaults can also come from `[agent] profile`
in `.androidforge/project.toml` or `ANDROIDFORGE_PROFILE=full|compact|brief`. `--max-items` and
`--max-text-chars` control the caps.

For line-oriented consumers, add `--jsonl` (implies `--json`) to emit one minified JSON object per
line; `scenario run` also writes one short iteration line before the final envelope. `--fields a,b,c`
keeps only those top-level keys of `result` and never removes envelope fields; envelope trimming
remains `--output-profile compact|brief`. `--stats` adds one JSON stats line to stderr with the exact
stdout byte count, line count and `ceil(bytes/4)` token estimate; stdout stays parseable and `--quiet`
does not suppress stats. These flags are CLI-only and are never MCP tool arguments.

Refs and observation tokens are 12 hex characters by default. Common aliases are `ui o/d/t/w`,
`app i/l`, `diagnostics perf`, `report why`, `agent b` and `context ctx`; selector aliases include
`--id`, `--desc` and `--token`.

### Context
```powershell
androidforge context --project . --json
androidforge context --session <ID> --json
androidforge context --session <ID> --include project,environment,session,device,app --json
androidforge context --session <ID> --refresh-screen --json
androidforge context --session <ID> --full --json
```

The compact result includes project, environment, session, device, app, recent screen observation, last
run and deterministic next actions. Missing sections are explicit (`available: false` with a reason).
The screen section reports `observed_at`, `age_ms` and `stale`. Observations younger than five seconds
are reused unless `--refresh-screen` is passed. `--dry-run` avoids capturing a new screen observation.

### Session ensure

```powershell
androidforge session ensure --project . --avd Pixel_8_API_35 --owner MyAgent --ttl 1h --json
androidforge session ensure --project . --avd Pixel_8_API_35 --port 5562 --owner MyAgent --json
androidforge session ensure --project . --serial emulator-5554 --owner MyAgent --json
androidforge session ensure --project . --avd Pixel_8_API_35 --owner MyAgent --wait --timeout 10m --json
```

`ensure` is idempotent for the same owner/project/target. `--wait` retries only while the device is
busy (`E_DEVICE_BUSY`); other errors fail immediately. On timeout it returns `E_DEVICE_BUSY` with the
elapsed wait and never steals the device. It reuses a healthy compatible session,
attaches to a warm unowned emulator, or starts the requested AVD. It refuses to steal a device from
another owner. `lease_expires_at` and `lease_ttl_seconds` are advisory: expiry never stops a device or
interrupts work. An expired lease is reported as `needs_reconciliation: true`; supplying `--ttl` to
ensure renews it.

### Observations and diffs

```powershell
androidforge ui observe --session <ID> --json
androidforge ui observe --session <ID> --package com.example.app --json
androidforge ui observe --session <ID> --since obs-... --json
androidforge ui observe --session <ID> --full --json
androidforge ui observe --session <ID> --semantics --json
androidforge ui observe --session <ID> --semantics --full --json
androidforge ui diff --session <ID> --from obs-... --json
androidforge ui diff --session <ID> --from obs-... --full --json
```

Default observations contain interactive `actionable` nodes, non-interactive `context` text, focus,
foreground package/activity, display and visible text. Each node includes `clickable` and `enabled`;
`token` is the preferred 12-hex observation token and `id` is a backward-compatible alias.
`actionable_total`/`actionable_truncated` and `context_total`/`context_truncated` state whether the
compact view was capped. `--full` returns the complete parsed hierarchy plus all nodes. Add
`--semantics` for a bounded (64-node) semantic view of the full hierarchy: it carries Compose test
tags as `test_tag` while leaving `resource_id` unchanged, adds `semantics_total`/
`semantics_truncated`, and gives those nodes stable refs usable by `ui tap --ref`. Omitting the flag
leaves the observation unchanged. Compose test tags can also be selected directly with
`--test-tag <tag>` on `ui tap` and `ui assert`. Observation
tokens are session-scoped, boot-id-scoped, kept in the local AndroidForge state directory and expire
after 24 hours or the per-session retention limit. The retention limit is
`[storage] observation_retention` (default 20, range 2-1000); eviction is oldest-first and never
removes the observation just written. A reboot or cross-session token returns
`E_OBSERVATION_EXPIRED` or `E_OBSERVATION_SESSION_MISMATCH`.

### Action receipts and postconditions

```powershell
androidforge ui tap --session <ID> --text "Save" --wait-for-text "Saved" --timeout 10 --json
androidforge ui tap --session <ID> --text "Settings" --wait-for-package com.example.app --json
androidforge ui type "Example" --session <ID> --wait-for-value "Example" --json
androidforge ui scroll --session <ID> --direction down --wait-for-text "More" --json
```

Conditions are ANDed. The action executes once; only observations are polled. A timeout returns
`E_POSTCONDITION_TIMEOUT` with `action_executed: true`, unsatisfied conditions, current foreground state,
the timeout and evidence. Actions are never retried automatically.

Add `--receipt` to tap, long-press, text or scroll to get the before/after observation tokens, compact
semantic diff, effect classification and completeness in the same response. `--receipt-full` expands
node lists. Receipt-enabled timeouts preserve the action result in `error.context.receipt`; a selector
failure shows `dispatched: false`.

### Failure recurrence and diagnostic bundles

```powershell
androidforge report recurrences --project . --json
androidforge report recurrences --project . --since 30d --package com.example.app --status regressed --json
androidforge report history --fingerprint sha256:<VALUE> --json
androidforge report bundle latest --output .\evidence.zip --json
androidforge report bundle latest-failure --output .\failure-evidence.zip --json
androidforge report bundle latest --dry-run --json
androidforge report bundle latest --markdown --json
androidforge report bundle latest-failure --tag smoke --since 7d --json
```

Recurrence memory derives `new`, `recurring`, `regressed` or `unknown` from existing local reports.
A passing run counts as resolution only when it exercised the same project/scenario/failing step.
Bundles include a redacted summary, explanation, reproduction argv, evidence excerpts, bounded logcat,
hierarchy XML, checksums and an exclusion list. Screenshots, recordings and full raw logs are opt-in.
ZIPs are written atomically and reopened for checksum verification before success is reported.
`latest` and `latest-failure` selection can be narrowed with repeatable `--tag` (all tags must be
present), the global `--session` / `--serial`, and `--since` (an RFC3339 instant or a relative
duration such as `7d`, `2h` or `500ms`). `report list` and the compare baseline selector accept the
same filters, so a smoke bundle cannot accidentally pick up an untagged or older run.

### APK resolution and comparison

```powershell
androidforge apk resolve --variant debug --json
androidforge apk resolve --variant freeDebug --module app --json
androidforge agent test --project . --variant debug --flow conformance --json
androidforge apk diff installed:com.example.app .\app-debug.apk --session <ID> --json
androidforge app install .\app-debug.apk --only-if-changed --json
```

Configuration is optional:

```toml
[build]
default_variant = "debug"

[build.variants]
debug = ["app/build/outputs/apk/debug/*.apk"]
"freeDebug" = ["app/build/outputs/apk/free/debug/*.apk"]

[agent]
profile = "compact"
max_items = 40
max_text_chars = 240

[storage]
observation_retention = 20

[health]
battery_warn_percent = 15
storage_warn_bytes = 536870912
thermal_warn_code = 3
matrix_preflight = true
```

Resolution never runs Gradle. It selects the newest valid APK and reports the matched rule, mtime,
SHA-256 and certificate. Equal timestamps with different content are an ambiguity error. An explicit
`--apk` wins over `--variant`. `--only-if-changed` skips only when package, version code, base APK hash
and every split SHA-256 prove equivalence; unknown split identity installs rather than silently skipping.

### Build, install and launch in one command

`androidforge build` runs the explicit argv configured for a variant and then resolves the produced
APK. It never infers a command or interprets a shell string. Build alone is host-side: it needs no
session and touches no device.

```powershell
androidforge build --variant debug --json
androidforge build --variant debug --module app --timeout 20m --expected-package com.example.app --json
androidforge build --variant debug --install --only-if-changed --session <ID> --json
androidforge build --variant debug --launch --activity .MainActivity --session <ID> --json
androidforge build --variant debug --cold --session <ID> --json
```

- `--variant` defaults to `[build].default_variant`; `--module` narrows APK resolution;
  `--timeout` overrides `[build].timeout_secs` (for example `20m`).
- `--install` reuses `app install` and its identity checks. `--fresh` requires `--yes`, cannot be
  combined with `--only-if-changed`, and `--downgrade` / `--no-verify` forward to the install
  options. Physical devices still require `--allow-physical-install` (or the superset).
- `--launch` implies install and reuses `app launch`, gated by `Risk::AppControl` (physical devices
  require `--allow-physical-app-control`). `--activity` requires `--launch` or `--cold`.
- `--cold` implies launch and uses `AppManager::launch_with_force_stop` even when `--activity` is
  given, so the launch verifies a genuinely new PID instead of reusing the old force-stop path.
- Global `--dry-run` executes nothing: no build process, no session lookup, no device contact and
  no log artifact. Install/launch flags become planning records in the result.
- A successful result adds a `build` object alongside `apk` (the resolved APK), `install` and
  `launch` when those were requested. Failures use `E_BUILD_*` codes with structured context
  (`classification`, `failed_task`, `first_error`, `log_path`, `log_sha256`, `argv`, `exit_code`).
- `E_BUILD_OUTPUT_TOO_LARGE` is reserved for the rare case where bounded output capture removed the
  only failure marker and left a generic non-zero exit; ordinary log truncation is a warning with a
  retained head and tail.

### Stable element references

```powershell
androidforge ui observe --session <ID> --json
androidforge ui tap --session <ID> --ref el-... --expect-token obs-... --json
androidforge ui long-press --session <ID> --ref el-... --expect-token obs-... --receipt --json
androidforge ui text "hello" --session <ID> --ref el-... --expect-token obs-... --json
androidforge ui clear --session <ID> --ref el-... --expect-token obs-... --json
androidforge ui scroll --session <ID> --ref el-... --expect-token obs-... --direction down --json
androidforge ui wait --session <ID> --ref el-... --expect-token obs-... --timeout 10 --json
androidforge ui assert --session <ID> --ref el-... --expect-token obs-... --json
```

A `ref` is a stable handle (12 hex characters) derived from node identity and scoped to one observation token, session and device. AndroidForge re-reads
the hierarchy and validates the node identity immediately before dispatch. Expired tokens, removed or
changed nodes, wrong devices and non-actionable nodes are explicit errors; there is no text or
coordinate fallback. Text/clear require an editable field, scroll requires a scrollable node, and
wait/assert may reference context text that is not itself actionable. Context text inside a clickable
row is marked `actionable_via: "ancestor"` and can be tapped or long-pressed through that ancestor.
Observe again after navigation; unchanged nodes keep the same ref.

### Device matrix (sequential or parallel)

```powershell
androidforge agent matrix --project . --avd AVD_1 --avd AVD_2 --flow conformance --json
androidforge agent matrix --project . --serial emulator-5554 --serial emulator-5556 --flow conformance --parallel 2 --json
androidforge agent matrix --project . --avd AVD_1 --avd AVD_2 --flow conformance --dry-run --json
```

Targets are validated up front and duplicates are rejected. Each target gets a separate owned session
and runs the normal `agent test` pipeline. `--parallel 1..32` launches one independent child matrix per
target; port probing and session creation are serialized so workers cannot claim the same emulator
port. Aggregate status is `passed`, `failed` or `partial`; `parallel`, per-target API/model, health,
session, child report and cleanup are recorded. `--fail-fast` is best-effort in parallel mode because
already-running targets finish. Only sessions created by the matrix with the matrix owner are released;
emulators and pre-existing sessions are untouched.

### Wireless ADB and health

```powershell
# Prefer stdin so the pairing code is not in shell history.
123456 | androidforge devices pair 192.168.1.20:37123 --code-stdin --json
androidforge devices connect 192.168.1.20:37521 --json
androidforge devices disconnect 192.168.1.20:37521 --json
androidforge devices health --session <ID> --json
```

Pairing and connecting use different endpoints/ports. Pairing codes are six digits, are never stored,
logged or returned, and are not included in bundles. AndroidForge never enables wireless debugging or
legacy `adb tcpip` by itself. Health is read-only and reports battery, thermal, storage, memory, network,
screen, uptime, Android/API and ADB state; unavailable values include a reason. Thresholds are configured
under `[health]` and `--no-evidence` omits raw command excerpts.

### Read-only retries

```powershell
androidforge ui observe --session <ID> --retries 2 --json
androidforge ui diff --session <ID> --from obs-... --retries 2 --json
androidforge diagnostics logcat peek --session <ID> --retries 3 --json
androidforge diagnostics logcat tail --session <ID> --retries 2 --json
```

Retries are opt-in, bounded to 0-5 and limited to read-only operations. Transient device/UIAutomator/
timeout errors retry with bounded exponential backoff; selector/configuration/permission/install errors do
not. The total retry window is 20 seconds, and every attempt is reported. Actions that change device or
application state are never retried.

### Failure explanations

```powershell
androidforge report explain latest --json
androidforge report explain latest-failure --json
androidforge report explain <RUN_ID> --full --json
androidforge report explain <RUN_ID> --markdown
androidforge report explain latest --tag release --tag smoke --since 2026-01-01T00:00:00Z --json
```

Explanations are deterministic and offline. They include category, stable error code, confidence,
curated evidence, alternatives, exact `argv` next actions, a stable fingerprint and the project root.
`latest` is the newest completed run in this project; `latest-failure` is the newest failed, partial,
cancelled, interrupted or abandoned run. `report bundle` uses the same selection. Both are strictly
scoped to the project's recorded root, so a newer run from another app is never selected.
Repeatable `--tag` and `--since` narrow `latest`/`latest-failure` further; the applied filter is
returned as `result.scope`. Explicit run ids bypass latest-selection filters. `report list --all`
lists runs across stores; run commands from `project_root` or pass `--project`.

### Read-only repair planning

```powershell
androidforge doctor fix --plan --json
androidforge doctor fix --check --json
```

`--plan` never installs packages, downloads tools, edits PATH or the registry, or changes Java/emulator
configuration. Every issue lists severity, detected state, component, reason and (when exact) a repair
command with confirmation/network/permission flags. `mutations_performed` is empty.

### MCP server and tool schema

```powershell
androidforge mcp serve
androidforge tools schema --format mcp --json
androidforge tools schema --format openai --json
```

`mcp serve` is a local stdio JSON-RPC server. It writes only protocol lines to stdout; logs go to
stderr. It exposes the same tools as `tools schema` (count measured per batch), shells out to the same
CLI and requires `confirm=true` for destructive tools. Raw `adb`, `run exec` and generated
long-running server entries are deliberately excluded; the curated mock-server tool remains bounded
by `max_requests`/`duration` unless a built-in scenario supplies its own bounds.
Physical-device mutation cannot be self-approved from MCP arguments. There is no HTTP
listener, account or key. `tools catalog --json` exposes the complete command catalog, and
`androidforge://catalog` serves it as an MCP resource. Use `tools schema` to feed the same catalog to
another function-calling client.

### Transparent agent pipeline

```powershell
androidforge --project . agent test --apk .\app-debug.apk --flow conformance --install --json
androidforge --project . agent test --apk .\app-debug.apk --flow conformance --dry-run --json
```

Stages are validate config, resolve/ensure session, verify device, inspect APK, compare the recorded
installed build, optionally install, optionally run a fixture, optionally run conformance, explain a
failure and write a consolidated report. Without `--install`, a mismatch stops at `install_apk`; the
pipeline never runs a `--fresh` install, clears data or uninstalls. `--dry-run` performs no session,
install, fixture or conformance changes and lists required confirmations.

A failed pipeline returns envelope `status: "failed"` with a nonzero `exit_code` and the full structured
result. A blocked dry run returns `status: "blocked"` and exit code 8. The desktop executable also
answers `--help` and `--version` without opening its GUI.

## 3. Desktop cockpit

`androidforge-desktop.exe` is the optional native cockpit. It is a separate executable built with Rust,
egui/eframe and glow; it does not embed a browser, WebView, HTML, JavaScript or Node runtime. The CLI
remains authoritative: every screen and action invokes `androidforge --json` and reads the same sessions,
reports and evidence.

### Launch and connection

```powershell
.\target\release\androidforge-desktop.exe
```

On startup it looks for the CLI in this order: the executable beside the desktop, an explicit path saved
in Settings, the workspace `target/debug` or `target/release` build, then `PATH`. The top bar shows the
resolved CLI version and connection state. The Settings page lets you change the CLI path, workspace,
owner, light/dark/system theme, accent color, and UI font. It is a normal page in the main window, not
a separate dialog.

If the CLI is offline, run `androidforge doctor --json` in the same environment. The desktop refuses a
CLI with an unknown report schema rather than guessing. A missing or incompatible CLI is shown in the
Overview error banner and in Settings.

### Pages

- **Overview** — environment status, devices/sessions/runs counters, recent runs and CLI resolution.
- **Context** — one read-only CLI `context` snapshot: project, active session/owner, device, installed
  app, current screen, last run, warnings and copyable next commands. Observed nodes expose stable
  `ref` values with copyable `ui tap --ref ... --expect-token ...` commands. Refresh is explicit and the
  snapshot age is shown.
- **Matrix** - run a sequential or parallel conformance matrix against one or more AVDs entered on
  separate lines, with owner, install, fail-fast, worker count and result/health/cleanup display.
  `--parallel` and the desktop worker field use one independent child matrix per target; `--fail-fast`
  is best-effort because already-running targets finish.
- **Devices** — selectable device table with serial, model, Android version, state, battery and owner;
  details include boot state, AVD, port, architecture, resolution, read-only health and wireless
  pair/connect/disconnect controls. Pairing codes are password fields sent through stdin and cleared
  after use.
- **Sessions** - selectable session table with owner, heartbeat, state, advisory lease and project
  binding; details show prepared settings and expose `Release`, `Stop`, `Restart`, `Use as target` and
  `Install APK`. Released/abandoned tombstones are read-only.
- **Ownership** — read-only dashboard of active sessions, device owners, stale active leases, tombstones
  and safe alternative commands when a device is occupied. It never stops or releases another owner's
  session.
- **Runs** - report-backed runs, project hooks and `Run project flow`. Selecting `Open` or a row shows
  build identity, steps, failure details and evidence.
- **Triage** - failure metadata, evidence, flaky analysis, deterministic explanation with recurrence
  memory and stored occurrences, action receipt details (effect, tokens, semantic changes, postcondition,
  reference validation), plus a diagnostic-bundle inclusion preview and ZIP creation. Category,
  confidence, summary, fingerprint, curated evidence and copyable next-action commands are shown. The
  commands are never executed by the desktop.
- **Setup** - the full eight-step wizard: welcome, CLI, environment, project, device, updates, finish
  summary and **Agent Handoff**. Agent Handoff shows the full paths to the agent/user/capability/readme/
  exit-code/project-adapter documentation, the CLI executable and workspace, and a copyable prompt with
  the recommended agent loop and safety rules. Section changes fade/slide and the step highlight glides.
- **Tool database** - searchable catalog of every CLI command generated from the same clap parser:
  alphabetical sort, safety badges, usage, examples and an argument table. Selecting a row opens a full
  detail page with a slide/fade transition and an MCP tool name. Search matches command paths, summaries,
  argument names and help text.
- **Compare** - compatible run comparison with per-step current/baseline status, duration delta and
  change classification.
- **Logcat** - bounded local log view with filters, clean rows and a `Follow latest` button that eases
  to the newest line instead of jumping abruptly.
- **Inspector** - periodic device preview plus a recursive semantic hierarchy: top-level rows expand
  into cards of child rows, children expand further, whole-row activation works and collapsing returns
  to the initial list. Node details and actions remain available.
- **Maintenance** - storage summary, cleanup preview and terminal-session/run inspection.
- **Settings** - connection, left-aligned CLI path, color wheel with visible brightness slider,
  searchable 33-font catalog with embedded JetBrains Mono and `Lorem Ipsum` previews, theme mode, update
  checks and About information. Settings fills the content area like every other page.

The cockpit uses GitHub-dark and light palettes, consistent card gutters, raised rounded row cards,
thin status pills with larger text, solid right-side scrollbars, a sliding sidebar highlight and a short
slide-up page transition. The native Windows caption bar is colored to match the active theme where DWM
supports it.

The project picker refreshes discovered adapters. Use `Refresh` or wait for the automatic interval; a
stale timestamp is shown when the last refresh failed.

### Evidence

The run detail has three tabs:

- **Summary** — build identity (package/version, APK and installed SHA-256, certificate, verification),
  session/device, failure code/suggestions and every step.
- **Evidence** — filterable artifact list. Screenshots render with Fit/100%/zoom; hierarchy and log text
  render with line numbers and a search filter. `Open` opens the artifact with the OS, and the path can
  be copied.
- **Raw files** — `report.json`, `metadata.json`, raw `manifest.json` and manifest verification counts
  (`ok`, `missing`, `mismatched`). Artifact paths are joined safely inside the run directory.

### Controlled actions

- **APK install** — choose an APK, inspect package/version/size/SHA-256/certificate, choose a ready or
  attached target session, optionally `Dry run`, then `Install...`. Fresh installs are explicit and
  destructive. The install dialog closes for confirmation and reopens with the measured result.
- **Run project flow** — fixture or conformance, with the project/target summary, destructive-step
  checkbox and confirmation. Hooks have their own `Run...` confirmation.
- **Session lifecycle** — start, attach, release, stop and restart, with a confirmation that names the
  session, device and owner. Stop and restart explain what is lost. Attached sessions show the attached
  stop warning.
- **Output** — the bottom panel streams stdout/stderr, shows elapsed time, exit status, truncation and a
  copy button. `Dismiss` clears a finished panel.

Session lifecycle operations run to completion and do not show `Cancel`. Cancellation is limited
to installs, fixtures, conformance and hooks: it terminates the AndroidForge child process tree and never
calls `session stop` or `session release`. The emulator remains available. The desktop reconciles the
unfinished run as `cancelled`, and startup reconciliation marks abandoned runs `interrupted` rather than
leaving them `running` forever.

### Safety rules for desktop users

- Read the confirmation target before clicking; the desktop uses the session selected in the picker.
- A released session is a tombstone: start or attach a new session before running actions.
- Closing the desktop does not stop the CLI, emulator or session. Use the CLI or the Sessions page to
  release/stop explicitly.
- The desktop is an operator console, not an editor for adapters, fixtures or scripts.

## 4. Project adapter

Run `init` inside a project, or create `.androidforge/project.toml` manually:

```powershell
androidforge init --name "My App" --package com.example.app --avd Pixel_8_API_35
androidforge config validate --json
androidforge config show --json
androidforge config path --json
```

Minimal adapter:

```toml
adapter_version = 1

[project]
name = "My App"
package = "com.example.app"
debug_package = "com.example.app.debug"
default_avd = "Pixel_8_API_35"
artifact_dir = ".artifacts"
verify_installed_apk = true

[fixtures]
directory = ".androidforge/fixtures"
default = "smoke"

[conformance]
flow = ".androidforge/conformance/release-smoke.toml"

[build]
default_variant = "debug"

[build.variants]
debug = ["app/build/outputs/apk/debug/*.apk"]
"freeDebug" = ["app/build/outputs/apk/free/debug/*.apk"]

[agent]
profile = "compact"
max_items = 40
max_text_chars = 240

[storage]
observation_retention = 20

[health]
battery_warn_percent = 15
storage_warn_bytes = 536870912
thermal_warn_code = 3
matrix_preflight = true
```

The adapter is optional for `doctor`, `devices`, `avd` and raw ADB, but required for package fallbacks, fixtures, hooks and conformance. See [PROJECT_ADAPTER.md](PROJECT_ADAPTER.md) for every section.

Configuration precedence: CLI > environment contract > project adapter > global config > defaults. `--project <dir>` chooses a project explicitly. Without it, AndroidForge walks upward for `.androidforge/project.toml`, `.androidforge/config.toml`, or a root `project.toml` containing `adapter_version`.

## 5. Devices

```powershell
androidforge devices list --json
androidforge devices inspect emulator-5554 --json
androidforge devices wait --serial emulator-5554 --timeout 120 --json
```

When exactly one operational device exists, many commands can infer it. With two or more, pass `--serial` or `--session`; otherwise expect `E_DEVICE_AMBIGUOUS`.

Reversible preparation:

```powershell
androidforge devices prepare --session <ID> --animations --keep-awake --wake --json
androidforge devices restore --session <ID> --json
```

Preparation records previous values. `devices restore`, `session release` and `session stop` restore only what AndroidForge changed. Use `--keep-prepared` if you deliberately want to keep test settings.

One device setting at a time is available through the typed scenario API:

```powershell
androidforge devices scenario --night yes --session <ID> --json
androidforge devices scenario --locale fr-FR --session <ID> --json
androidforge devices scenario --font 1.15 --session <ID> --json
androidforge devices scenario --network offline --session <ID> --json
androidforge devices scenario --network offline --dry-run --json
```

Exactly one flag is accepted. Night/locale/font changes reuse `DevicePreparer` and record their
previous values in the session; network changes reuse the named network profiles and their restore
recorder. Settings changes require `--allow-physical-data` on a physical device, network changes
require `--allow-physical-network`, and a dry run only prints a plan with `will_mutate: false`.

Host-only tags label serials for matrix planning. They never contact a device:

```powershell
androidforge devices tag add --serial emulator-5554 --tag nightly --json
androidforge devices tag list --json
androidforge devices tag remove --serial emulator-5554 --tag nightly --json
androidforge devices tag clear --serial emulator-5554 --json
androidforge agent matrix --device tag:nightly --flow conformance --dry-run --json
```

Tags are lowercase safe tokens. `--device tag:<name>` resolves to concrete serials deterministically,
and a serial that is also passed with `--serial` is counted once.

## 6. Emulators and snapshots

```powershell
androidforge avd list --json
androidforge avd images --json
androidforge doctor --capacity --json
androidforge avd create androidforge_test --image "system-images;android-35;google_apis;x86_64" --device pixel_7 --dry-run --json
androidforge avd create androidforge_test --image "system-images;android-35;google_apis;x86_64" --device pixel_7 --yes --json
androidforge ports list --json
androidforge emulator start --avd androidforge_test --port 5560 --headless --json
androidforge emulator status --serial emulator-5560 --json
androidforge emulator stop --session <ID> --json
```

`avd images` is a read-only inventory of installed SDK system images. `avd create` is a host-side
mutation: `--dry-run` prints the exact `avdmanager` argv and executes nothing, and a real run requires
explicit `--yes`; an existing AVD is never overwritten. `doctor --capacity` reports host RAM, commit
and disk readings with their sources plus conservative per-AVD commit estimates; unknown values are
`null`/unknown with a reason, never a fabricated zero. `ports list` maps every conventional console
port to its ADB and gRPC ports, shows free/allocated/occupied state and attaches active-session
owner/session/AVD metadata from session files. `emulator status` adds console/ADB/gRPC ports,
owner/session and a conservative `capacity_warning` while keeping the existing fields.

- New emulators are headless by default. Use `--windowed` only when a human needs to watch.
- `--port` is an even console port; ADB uses port + 1. Automatic allocation starts at the conventional
  5554 range, while explicit requests accept any even 1024–65532 so environment-reserved ports work.
  AndroidForge checks collisions before launch.
- `--cold-boot`, `--memory`, `--cores`, `--prepare`, `--reuse` and `--timeout` are available.
- AndroidForge never stops an emulator it did not start, unless the session attached to it and you pass the explicit attached-stop flag.

Snapshots:

```powershell
androidforge snapshot list --session <ID> --json
androidforge snapshot save <name> --session <ID> --json
androidforge snapshot load <name> --session <ID> --yes --json
androidforge snapshot delete <name> --session <ID> --json
```

Snapshot support varies by emulator build. Load is destructive and always requires `--yes`.

## 7. Sessions

A session is the unit of ownership and the most reliable way to target a device.

```powershell
androidforge session start --avd androidforge_test --port 5560 --headless --owner MyAgent --prepare --json
androidforge session start --avd androidforge_test --port 5560 --headless --owner MyAgent --snapshot-save --json
androidforge session attach --serial emulator-5556 --owner MyAgent --json
androidforge session claim --serial emulator-5556 --json
androidforge session list --json
androidforge session status --session ses-... --json
androidforge session inspect --session ses-... --json
androidforge session use ses-... --json
androidforge session current --json
androidforge session release --session ses-... --owner MyAgent --json
androidforge session stop --session ses-... --owner MyAgent --json
```

`session use` writes `.androidforge/current-session` for the project. `ANDROIDFORGE_SESSION` is the environment alternative. `session release` releases ownership and restores prepared settings; `session stop` also stops an emulator the session owns, or an attached emulator with `--attached`. `session release --stop` stops an owned emulator; attached emulators additionally require `--stop-attached`. If you release an owned emulator without stopping it, the released tombstone can still be stopped with `session stop --session <ID>` as long as no new session owns the device. `session list --active` hides released, failed and abandoned tombstones. `session list --mine` filters to the current owner (`ANDROIDFORGE_OWNER`, or the OS user), and `--owner <name>` selects an explicit owner. Each list entry carries a `classification` (`active`, `busy`, `stale`, `released`, `failed`, `abandoned`), its heartbeat age and a `reason` so tombstones are never mistaken for live sessions.

`session gc --reclaim --dry-run` is the safe preview. It reports `would_reclaim`, leaves every session file untouched, and prints the exact command to apply the change. Reclaim never touches a session with a current heartbeat or another owner's live operation.

`session gc --forget-terminal` releases device locks left by failed/released/abandoned tombstones without deleting their history. It is dry-run by default when the flag is used: preview `would_forget` first, then apply with the same flags plus `--owner <OWNER>` or `--force`. `--owner` (or `ANDROIDFORGE_OWNER`) must match the recorded owner unless `--force` is passed. Live sessions and locks belonging to another session are always refused, even with `--force`.

`session status` and `session inspect` report the heartbeat and advisory lease. There is no standalone `session heartbeat` subcommand; use `session status --session <ID> --json` when an agent needs a liveness sample.

AndroidForge-launched emulators default to `-no-snapshot-save` so test state is not silently preserved. Pass `--snapshot-save` to `session start` or `session ensure` when you intend to use `snapshot save|load`. Attached emulators keep whatever launch flags they already have.

`session stop`, `session release` and `session restart` are owner-guarded. Pass the same `--owner` used at `session start` (or export `ANDROIDFORGE_OWNER`) or use `--force` for a deliberate takeover. Without a matching owner they return `E_SESSION_OWNERSHIP_MISMATCH` with both labels and do not touch the emulator. This is a coordination guard, not authentication: anyone who can read the local session file can still use `--force`.

`session stop` is **not** `--yes`-guarded; ownership is the safety gate. `--yes` is reserved for app uninstall, data-clearing, restore and other destructive app operations.

If another session owns the device, you get `E_DEVICE_BUSY`. Do not kill the emulator. Check `session list`; use `session gc --reclaim` only when the owner is genuinely dead. `session gc --help` also has `--stop` for stale owned emulators.

## 8. APKs

```powershell
androidforge apk inspect .\app-debug.apk --json
androidforge apk verify .\app-debug.apk --expected-cert-sha256 <64-hex> --expected-package com.example.app --min-version-code 10 --json
```

Inspection reports package, versions, min/target SDK, label, launchable activity, architectures, size, SHA-256 and signature details. Verification returns `E_SIGNATURE_VERIFICATION_FAILED` on a bad certificate and `E_APK_INVALID` on an unreadable APK.

### Variant resolution, diff and only-if-changed

```powershell
androidforge apk resolve --variant debug --json
androidforge apk resolve --variant freeDebug --module app --json
androidforge apk diff installed:com.example.app .\app-debug.apk --session <ID> --json
androidforge apk diff installed:com.example.app .\app-debug.apk --split .\config.arm64_v8a.apk --session <ID> --json
androidforge app install .\app-debug.apk --session <ID> --only-if-changed --json
androidforge app install .\outputs\bundle.apks --session <ID> --only-if-changed --json
```

`apk resolve` is deterministic and read-only. It never invokes Gradle. Configure explicit outputs, or
rely on conventional `app/build/outputs/apk` layouts when no variant patterns are configured:

```toml
[build]
default_variant = "debug"

[build.variants]
debug = ["app/build/outputs/apk/debug/*.apk"]
"freeDebug" = ["app/build/outputs/apk/free/debug/*.apk"]

[agent]
profile = "compact"
max_items = 40
max_text_chars = 240

[storage]
observation_retention = 20

[health]
battery_warn_percent = 15
storage_warn_bytes = 536870912
thermal_warn_code = 3
matrix_preflight = true
```

The newest valid APK wins. Malformed APKs are rejected with reasons; equal timestamps and different
hashes return an ambiguity error; the selected rule, mtime, hash and certificate are reported. An
explicit `--apk` always wins over `--variant`.

`apk diff` reports each field as `same`, `different` or `unknown`. It pulls the installed base APK and
every installed split to a temporary directory and cleans them up. Split sets are compared by their
sorted SHA-256 hashes, not names. `--only-if-changed` skips installation only when package, version code,
base APK SHA-256 and the complete split set prove equivalence; otherwise it installs and records the
reason. It never uninstalls, clears data or downgrades automatically.

### Install and build identity

```powershell
androidforge app install .\app-debug.apk --session <ID> --json
androidforge app install .\app-release.apk --session <ID> --expected-package com.example.app --json
androidforge app install .\base.apk --split .\config.arm64_v8a.apk --split .\config.en.apk --session <ID> --json
androidforge app install .\outputs\bundle.apks --session <ID> --json
```

Split artifacts are installed exactly as provided. A directory with `splits/` uses those entries; a
directory with one `standalones/` APK uses that standalone. An `.apks` archive selects `splits/` entries,
or a single `universal.apk`/standalone; multiple standalones are rejected because AndroidForge will not
guess which one targets the device. Archives are extracted with bounded entry/size limits. AndroidForge
does not run bundletool and does not select device-specific splits. Every base/split pair must declare
the same package and version code before installation. Split installs use `adb install-multiple` and
record the supplied artifact path in the build identity.

The result includes package, versionName, versionCode, local APK SHA-256, installed APK SHA-256, signing certificate SHA-256, debug-signed and `verified`. A mismatch between the requested and installed package, hash or version fails with `E_INSTALL_VERIFICATION_FAILED`. If `[project].expected_cert_sha256` is set and the APK uses a different signer, install fails before touching the device with `E_SIGNATURE_VERIFICATION_FAILED` and reports the actual fingerprint. Add `--expected-cert-sha256 <64-hex>` for an ad-hoc fingerprint check, or `--expected-package` for an ad-hoc package check.

`--dry-run` inspects the APK locally and reports its package/version/hash/certificate plus the planned
fresh, downgrade, package and certificate constraints without installing.

Some OEM skins answer their own "allow installation" prompt before `adb install` returns, leaving `INSTALL_FAILED_USER_RESTRICTED`. `--wait-for-confirmation` opts into watching that dialog through the audited UIAutomator snapshot path. If a single unambiguous confirm button is found, AndroidForge taps it through `UiManager` identity validation and retries the install once; a second failure is returned as-is. Repeatable `--confirm-text <label>` overrides the built-in localized label list, and `--confirmation-timeout` bounds the wait (default 15s). Auto-denial, an ambiguous dialog or a timeout fail with `E_INSTALL_FAILED` carrying the exact observed dialog text, candidate buttons and a stable `reason`. The normal `Risk::Install` gate and all identity checks are unchanged; without the flag this path is never entered.

A signing mismatch is `E_INSTALL_SIGNATURE_MISMATCH`, listing both certificates and the installed version. AndroidForge never auto-uninstalls. To replace the app and delete its data:

```powershell
androidforge app install .\app-release.apk --session <ID> --fresh --yes --json
```

Back up debuggable data first if it matters:

```powershell
androidforge app data-backup com.example.app .\backup.tar --session <ID> --json
```

Use `--no-verify` only when you explicitly need to skip post-install hash verification; it weakens evidence.

Two read-only commands answer "what is actually installed?" without trusting cached session metadata:

```powershell
androidforge app verify-installed --apk .\app-debug.apk --session <ID> --json
androidforge app verify-installed --apk .\base.apk --split .\config.arm64_v8a.apk --session <ID> --json
androidforge app status com.example.app --session <ID> --expect-version-name 5.1.2 --expect-version-code 512 --expect-apk-sha256 <64-hex> --json
```

`app verify-installed` pulls the live base APK, hashes it, reads the live package metadata and compares package, version, certificate and split identity. It exits non-zero with `E_INSTALL_VERIFICATION_FAILED` when the device does not match. `app status` applies each expectation to live state and returns a structured pass/fail record; a mismatch is an assertion failure, not a warning.

`session status --json` also refreshes the cached build identity from `pm path` on every call, so an out-of-band `adb install -r` cannot leave a stale `installed_apk_sha256` in the session. The refresh is persisted for later reports.

## 9. App control

```powershell
androidforge app launch com.example.app --session <ID> --json
androidforge app launch com.example.app --action android.intent.action.VIEW --data "myapp://checkout/42" --wait --session <ID> --json
androidforge app launch com.example.app --component com.example.app/.MainActivity --extra "coupon:s:SAVE10" --session <ID> --json
androidforge app broadcast --action com.example.app.PING --extra "n:i:7" --session <ID> --json
androidforge app service --package com.example.app --service .SyncService --stop --session <ID> --json
androidforge app status com.example.app --session <ID> --expect foreground --json
androidforge app status com.example.app --session <ID> --expect foreground --assert-task-exists --assert-process-stable --stable-for 2s --json
androidforge app force-stop com.example.app --session <ID> --json
androidforge app kill com.example.app --session <ID> --json
androidforge app clear-data com.example.app --session <ID> --yes --json
androidforge app uninstall com.example.app --session <ID> --yes --json
androidforge app list --session <ID> --json
androidforge app info com.example.app --session <ID> --json
```

`app launch` accepts `--component`, `--action`, `--data` (deep links), `--mime-type`, `--category`,
`--extra`, `--flag`, `--wait` and `--force-stop`. Extras use `key:type:value` with `s`, `i`, `l`, `f`,
`z`, `esa` and `eia`; values may contain colons. `app broadcast` adds `--receiver-permission` and
`--asynchronous`; `app service` starts or stops a service intent. A supplied package must be installed.

`force-stop` removes the task/process and stops scheduled work. `kill` is the softer `am kill`. `clear-data` and `uninstall` are destructive and require `--yes`. `run-as` file operations require a debuggable build:

AndroidForge protects physical devices by default. Data-changing commands such as `app install`,
`app uninstall`, `app clear-data`, `app file-push`, `app data-restore`, `permissions grant|revoke`,
`appops set|reset`, `doze force`, `standby set`, `battery unplug|reset`, `network` changes,
`time set|restore`, `notifications clear` and `jobs run|cancel` return
`E_PHYSICAL_DEVICE_MUTATION_BLOCKED` on a physical device without an explicit opt-in. Emulators do
not need a flag. `--dry-run` remains available and shows `requires_physical_mutation` without
changing anything. This is a safety guard, not authentication; a deliberate test can still opt in.

Opt-ins are scoped, so granting one capability never grants another:

| Flag | Physical-device capability it grants |
| --- | --- |
| `--allow-physical-app-control` | `app launch`, `app force-stop`, `app broadcast`, `app service`, `app kill` and explicit intents |
| `--allow-physical-data` | app-private data operations (`clear-data`, `file-push`, `data import/restore/reset/seed`), fixture/hook/conformance/test runs, permissions/AppOps/Doze/standby/battery/time changes, and device prepare/restore/snapshot |
| `--allow-physical-network` | airplane mode, Wi-Fi, mobile data, proxy, network profiles/restore, `adb reverse` mappings and `network mock serve --reverse` |
| `--allow-physical-install` | `app install` and `app uninstall` |
| `--allow-physical-input` | input injection (`ui tap`, `ui long-press`, `ui drag`, `ui text`, `ui clear`, `ui scroll`, `ui scroll-to`, `ui swipe`, `ui back`, `ui home`, `ui keyevent`, `ui hide-keyboard`, `ui dismiss-overlays`, `notifications tap`) |

`--allow-physical-mutation` remains the superset and grants every risk above; it preserves the
existing behavior for callers that use it. For example, `--allow-physical-input` still blocks a
data change, and `--allow-physical-data` still blocks an install. MCP never exposes or forwards any
of these flags: mutating MCP tools still require `confirm=true` and cannot self-approve a physical
device operation.

```powershell
androidforge app file-push .\seed.json files/seed.json --package com.example.app.debug --session <ID> --json
androidforge app file-pull files/seed.json .\pulled.json --package com.example.app.debug --session <ID> --json
```

Remote paths are relative to the app's private data directory and may not contain `..`.

### App data fixtures

```powershell
# Safe default: writes under the managed artifacts/export area.
androidforge app data export com.example.app --session <ID> --json
# Explicit destination (named flag or legacy positional path).
androidforge app data export com.example.app --output .\data.tar --session <ID> --json
androidforge app data export com.example.app .\data.tar --session <ID> --json
# In-project destination and replacing an existing archive are explicit opt-ins.
androidforge app data export com.example.app --output .\data.tar --allow-in-project --overwrite --session <ID> --json
androidforge app data import com.example.app .\data.tar --session <ID> --yes --json
androidforge app data reset com.example.app --baseline .\seeded.tar --backup-current .\before.tar --session <ID> --yes --json
androidforge app data seed com.example.app --file .\fixture.db --remote databases/app.db --session <ID> --yes --json
```

`app data export` resolves its destination before touching the device: the default lives under
`<artifacts_dir>/export`, outside the project's source tree. An explicit destination inside the
project root is refused unless `--allow-in-project` is passed; an existing file is refused unless
`--overwrite` is passed; `..` traversal is always refused. `--dry-run` reports `source`
(`default`/`explicit`), `in_project`, and `in_export_area` so the chosen path can be audited.

`app data export` and `app data import` use `run-as` and therefore require a debuggable build; a non-debuggable package fails with `E_APP_NOT_DEBUGGABLE` and a clear suggestion. `app data import` always captures a pre-import archive into the run before restoring, so an overwrite is recoverable. `app data reset` clears app data, optionally saves `--backup-current`, then restores `--baseline`. `app data seed` writes one file to a path relative to the app-private data directory. Import, reset and seed require `--yes`; physical devices additionally require `--allow-physical-data` (or the superset `--allow-physical-mutation`). Use `--dry-run` to preview all paths.

## 10. UI work

Start with a dump or a find, not coordinates.

```powershell
androidforge ui dump --session <ID> --json
androidforge ui find --session <ID> --text "Save" --json
androidforge ui find --session <ID> --contains "Save" --json
androidforge ui find --session <ID> --resource-id com.example.app:id/save --json
androidforge ui find --session <ID> --role button --text-regex "Save|Apply" --json
androidforge ui find --session <ID> --text "Continue" --within "Checkout" --json
androidforge ui find --session <ID> --text "Continue" --region 0,800,1080,1600 --json
androidforge ui find --session <ID> --text "Continue" --ocr-fallback --json
androidforge ui focused --session <ID> --json
```

Read-only geometry checks reuse the same semantic selectors:

```powershell
androidforge ui bounds --session <ID> --text "Save" --json
androidforge ui assert-bounds --session <ID> --text "Save" --bottom-y 300 --tolerance 2 --json
androidforge ui assert-bounds --session <ID> --resource-id com.example:id/save --bottom-y-gt 800 --json
androidforge ui assert-bounds --session <ID> --content-desc "Save changes" --bottom-y-lt 500 --json
androidforge ui wait --session <ID> --text "Save" --for-bounds-stable --stable-for 2s --timeout 15 --json
```

`ui audit a11y` is a read-only audit of one hierarchy dump. It reports missing accessible names, tiny touch targets and duplicate labels, with effective options, bounded findings and severity/rule counts:

```powershell
androidforge ui audit a11y --session <ID> --json
androidforge ui audit a11y --session <ID> --density 2.75 --min-touch-target-dp 48 --json
androidforge ui audit a11y --session <ID> --contrast --text-rgb #FFFFFF --json
```

`--contrast` captures one screenshot (host-only `screencap -p`) and samples the majority background behind each visible text node, merging low-contrast findings into the same report. `--large-text` applies the WCAG 3:1 exception; `--text-rgb` supplies the known text colour because UIAutomator does not expose it. The `--no-content-descriptions`, `--no-touch-targets` and `--no-duplicate-labels` flags disable individual rules. Findings are deterministic and capped by `--max-findings`/`--max-nodes`.

`ui bounds` resolves one selector through a single hierarchy dump and returns all eight
integer fields (`left`, `top`, `right`, `bottom`, `width`, `height`, `center_x`, `center_y`).
`ui assert-bounds` compares `bottom_y` with `--bottom-y` (equality within `--tolerance`,
default 0), `--bottom-y-gt` (`bottom_y > expected + tolerance`) or `--bottom-y-lt`
(`bottom_y < expected - tolerance`); the three value flags are mutually exclusive. A failed
comparison is `E_UI_ASSERTION_FAILED` with `expected`, `actual`, `tolerance`, `relation`,
`passed` and the full comparison in its error context. All three commands are read-only:
they never inject input and never require a physical-device consent flag.

Actions:

```powershell
androidforge ui tap --session <ID> --text "Save" --json
androidforge ui tap --session <ID> --test-tag save_button --json
androidforge ui long-press --session <ID> --content-desc "Delete" --duration 1500 --json
androidforge ui long-press --session <ID> --text "Card" --duration 1200 --capture-mid .\held.png --json
androidforge ui drag --session <ID> --from "text=Item" --to "id=com.example.app:id/trash" --duration 800 --hold-before 200 --hold-after 200 --json
androidforge ui drag --session <ID> --from-x 200 --from-y 600 --to-x 800 --to-y 600 --capture-mid .\dragging.png --json
androidforge ui text "hello world" --session <ID> --class android.widget.EditText --clear --hide-keyboard --json
androidforge ui clear --session <ID> --json
androidforge ui back --session <ID> --hide-keyboard --json
androidforge ui home --session <ID> --json
androidforge ui keyevent KEYCODE_ENTER --session <ID> --json
androidforge ui swipe 500 1800 500 600 --session <ID> --json
androidforge ui dismiss-overlays --session <ID> --dry-run --json
androidforge ui dismiss-overlays --session <ID> --max 3 --json
```

`ui dismiss-overlays` handles the common SystemUI immersive "Viewing full screen / Got it" overlay
that appears on a fresh AVD. It only taps clickable `com.android.systemui` nodes labelled `Got it`,
`OK`, `Dismiss` or `Close`; it never taps app content or permission decisions. Use `--dry-run` to
preview the candidates.

Waiting and assertions:

```powershell
androidforge ui wait --session <ID> --text "Saved" --timeout 20 --json
androidforge ui wait --session <ID> --contains "Loading" --not-exists --timeout 20 --json
androidforge ui wait --session <ID> --for-text-gone "Loading" --for-text-gone "Saving" --timeout 20 --json
androidforge ui wait --session <ID> --for-idle --timeout 10 --json
androidforge ui wait --session <ID> --for-stable --stable-for 2s --timeout 15 --json
androidforge ui settle --session <ID> --timeout 10 --json
androidforge ui assert --session <ID> --contains "Welcome" --json
androidforge ui assert --session <ID> --text "Error" --not-exists --json
androidforge ui assert-no-text --session <ID> --contains "Loading" --timeout 5 --json
androidforge ui scroll-to --session <ID> --text "Settings" --json
```

`--for-text-gone` polls visible text and content descriptions until every supplied string is absent; on timeout it reports every lingering match with bounds. `--for-idle` waits for one byte-identical hierarchy dump; `--for-stable` requires the hierarchy hash to remain unchanged for `--stable-for` (default 2s). These modes are mutually exclusive with selectors and `--not-exists`. `--for-bounds-stable` instead re-resolves a semantic selector on every poll and succeeds when the same bounds persist for `--stable-for`; a lost or ambiguous element fails closed. It requires a selector, rejects `--ref`/`--not-exists`, and is mutually exclusive with the other wait modes. `ui assert --for-bounds-stable` runs the same check with the shorter default assertion timeout.

Rules:

- Exact matching is the default; use `--contains` deliberately.
- Selector v2 adds `--role`, `--text-regex`, `--region`, `--within`, `--above`, `--below`, `--left-of`
  and `--right-of`. `--ocr-fallback` runs only after UIAutomator finds nothing; it supports text,
  regex, region and index and refuses conditions OCR cannot prove (resource id, class, role,
  relative nodes). The default engine is `tesseract`; override with `ANDROIDFORGE_OCR_COMMAND`.
- `ui find` is a search: it succeeds with `matches: []` when nothing matches. `ui assert`, `ui tap` and `ui wait` fail with `E_SELECTOR_NOT_FOUND` when the condition is absent.
- `ui assert` is a quick one-attempt check (default 1s). Pass `--timeout` or use `ui wait` first for animated/loading screens.
- `ui clear` with no selector clears the focused editable field. If none is focused it returns `E_SELECTOR_NOT_FOUND` with the focused nodes and a selector suggestion. Android Settings and some OEM fields expose their hint as `text`; AndroidForge treats an exact hint as empty before typing and after clearing, so clearing a search field does not false-fail.
- Ambiguity is an error with candidates. Add `--index N` from the candidate list or narrow the selector.
- Taps use current device pixels and optionally a clickable ancestor.
- `ui tap --capture-before <png>` and `--capture-after <png>` capture evidence around the action and include `before_screenshot`/`after_screenshot` in the result or receipt. They are explicit because screenshots add device round-trips.
- Text entry verifies the focused editable field, clears with batched deletes when asked, re-reads and compares. Password fields report unverified.
- `ui back` refuses while the IME is up unless `--hide-keyboard` is explicit.
- On a fresh AVD, run `ui dismiss-overlays` before app assertions so the SystemUI immersive cling
  cannot hide the app hierarchy.
- Avoid fixed sleeps. Use `wait`, `settle` and `scroll-to`.
- Compose test tags are not visible unless the app exposes them as accessibility resource IDs. `--test-tag` is an alias for the resource id and fails with an actionable suggestion when the tag is absent. In a debug build, set `Modifier.semantics { testTagsAsResourceId = true }` on the relevant composable (or the root semantics) so UIAutomator can see `testTag` values. This is an app-side setting; AndroidForge does not inject it and does not claim tags exist when the app has not enabled them.

## 11. Evidence

### Screenshots and recordings

```powershell
androidforge capture screenshot --session <ID> --label before --json
androidforge capture screenshot --session <ID> --compare .\baseline.png --json
androidforge capture screenshot --session <ID> --compare .\baseline.png --region 0,120,1080,900 --max-diff-ratio 0.002 --diff-output .\diff.png --json
androidforge capture assert-color --session <ID> --rgb #1A73E8 --at 540,1200 --tolerance 8 --json
androidforge capture assert-color --session <ID> --rgb #FFFFFF --region 0,0,1080,200 --majority --json
androidforge capture launch com.example.app --session <ID> --fps 10 --duration 5s --expect-first-frame --no-splash --expected-color #1A73E8 --json
androidforge capture record start --session <ID> --time-limit 30 --json
androidforge capture record status --session <ID> --json
androidforge capture record stop --session <ID> --json
```

Screenshots include dimensions, display size, timestamp and SHA-256. With `--compare`, AndroidForge decodes both PNGs, limits comparison to `--region x1,y1,x2,y2` when supplied, applies an optional per-channel `--diff-threshold`, and reports changed pixels, ratio, mean channel delta and the changed bounding box. The check fails with `E_UI_ASSERTION_FAILED` when the ratio exceeds `--max-diff-ratio`; a diff visualization is stored as a run artifact. Baselines may be a direct path or a path relative to the artifact directory. Recordings are MP4, no audio, and Android caps standard `screenrecord` at 180 seconds. A recording started inside a caller-owned run does not finalize that run on stop.

`capture assert-color` takes a fresh screenshot with `exec-out screencap -p` (host-only, no device file) and checks one colour against `--rgb #RRGGBB`. By default it samples the image centre; `--at x,y` selects one device pixel, and `--region x,y,w,h` samples that rectangle (average colour, or the most frequent colour with `--majority`). `--tolerance` is the inclusive maximum per-channel difference (default 16). Success returns `actual`, `expected`, `tolerance` and `max_channel_delta`; a mismatch is `E_UI_ASSERTION_FAILED` with the same structured values. `--dry-run` needs no device session and changes nothing.

`capture contrast` uses the same host-only screenshot sampling for a WCAG text/background check. `--rgb` is the text colour; the background comes from `--at x,y` (one pixel), `--region x,y,w,h` (majority, or average without `--majority`), or the whole image when neither is given. `--threshold` overrides the default 4.5:1 (3:1 with `--large-text`). A failure is `E_UI_ASSERTION_FAILED` with the ratio, threshold, sampled colours and an a11y finding in its context.

Named baselines keep a validated name and a per project/run manifest under the artifact directory:

```powershell
androidforge capture baseline save --name home_screen --session <ID> --json
androidforge capture baseline update --name home_screen --session <ID> --json
androidforge capture baseline compare --name home_screen --max-diff-ratio 0.002 --json
androidforge capture baseline list --json
```

`save` refuses an existing name; `update` replaces it deliberately. `--source <png>` saves a local PNG instead of capturing a fresh screenshot, and `compare --current <png>` does the same for the current side. Compare uses `compare_png` with the optional `--region x1,y1,x2,y2`, `--diff-threshold` and `--diff-output`; a mismatch is `E_UI_ASSERTION_FAILED` carrying the baseline entry and the complete diff metrics. These commands are host-side (read-only in the catalog): they never change device state.

`capture launch` deliberately launches the package, then samples bounded screencap frames (no video decoder, no device temp file) to find the first frame whose focus or `--ui-dump-package` hierarchy matches the package. `--fps` (1-30) and `--duration` (e.g. `5s`) bound the sample. `--expect-first-frame`, `--no-splash` and `--expected-color` with `--color-sample centre|dominant` and `--tolerance` are explicit assertions; a failure is `E_UI_ASSERTION_FAILED` carrying the observed values. Every sampled PNG is written to the run as `capture-launch/frame-NNNN.png`. On a physical device the launch follows the existing scoped `--allow-physical-app-control` consent rule; no new mutation flag is added.

### Logcat

```powershell
androidforge diagnostics logcat --session <ID> --package com.example.app --lines 2000 --json
androidforge diagnostics logcat query --session <ID> --grep "FATAL|ANR" --level warn --json
androidforge diagnostics logcat watch --session <ID> --crash --anr --timeout 30s
androidforge diagnostics logcat start --session <ID> --package com.example.app --json
androidforge diagnostics logcat status --session <ID> --json
androidforge diagnostics logcat stop --session <ID> --json
# Redact suspected tokens before printing, NDJSON emission or storage:
androidforge diagnostics logcat peek --session <ID> --redact-patterns AIza,sk-,ghp_,AKIA --json
```

One-shot tails are bounded. `query` returns structured threadtime records filtered by package,
level, time and regex. `watch` streams raw NDJSON events (`log`, `crash`, `anr`) and stops at
`--timeout` or `--max-events`. Continuous capture runs on-device with `[diagnostics].logcat_max_seconds`
and `max_log_bytes` caps. AndroidForge never clears the global buffer by default.

`--redact-patterns` is repeatable and comma-separated. Each value is a **literal prefix**, not a regex:
`sk-` redacts the whole following token to `[REDACTED]`. It applies to `tail`, `peek`, `query`, `watch`
and `start`; for `start` the prefixes are persisted and applied by `stop` before captured text is
registered, so the background capture needs no new trust. Without the flag output is unchanged.

### Diagnostics collection

```powershell
androidforge diagnostics collect --session <ID> --package com.example.app --json
androidforge diagnostics app com.example.app --session <ID> --json
androidforge diagnostics crashes com.example.app --session <ID> --json
androidforge diagnostics inspect activity --session <ID> --json
```

`collect` captures screenshot, hierarchy, logcat, crash/ANR state, app state and dumpsys. `--full` includes every supported dump; `--log-lines` controls the tail. Crash/ANR detection is best-effort and says so.

### Performance evidence

```powershell
androidforge diagnostics performance com.example.app --session <ID> --json
androidforge diagnostics performance com.example.app --cold --settle 2s --session <ID> --json
androidforge diagnostics performance com.example.app --no-screen-check --min-frames 60 --frame-window 5s --warmup 2s --session <ID> --json
```

`diagnostics performance` measures each dimension explicitly and writes `performance/report.json`.
Because startup timing requires launching the app (`am start -W`, and `am force-stop` with `--cold`),
measurement on a physical device requires `--allow-physical-app-control` (or the
`--allow-physical-mutation` superset), just like `capture launch` and `app launch`. Emulators need no
grant, and `--dry-run` neither requires consent nor touches the device. The catalog classifies `diagnostics performance` as `device-mutation` (whereas `diagnostics frames` stays read-only), and MCP requires `confirm=true` unless `dry_run=true`; MCP never forwards physical capability flags, so the physical grant above is required on the CLI side:

- startup from `am start -W` (`TotalTime`, `WaitTime`, `ThisTime`, status and activity);
- jank from `dumpsys gfxinfo` after a pre-launch reset (frames, janky frames/percent, percentiles,
  missed vsync);
- memory from `dumpsys meminfo` (PSS, RSS, Java/native heap and PID);
- CPU from `dumpsys cpuinfo`, falling back to two `/proc/<pid>/stat` samples when the package is not in
  the dumpsys window.

`--cold` is required to force-stop the app first; without it the existing process is measured as-is.
Force-stopping preserves app data. `--settle` controls the delay before post-launch sampling (default
2s), and `--timeout` bounds each device command. Values that the device does not report are `null` with
an `unknown_reason`, and the command exits 10 when evidence is partial. Nothing is estimated.

The screen-on preflight is **on by default**: a sleeping display makes jank and startup evidence
invalid, so measurement fails with `E_SCREEN_OFF` instead of reporting misleading numbers. Use
`--no-screen-check` to skip that preflight when you accept sleeping-display evidence. Waking the
device is a separate, explicit screen-state opt-in via `--wake` only; it is never implicit, including
with `--cold`, and the app-control grant never implies it.

A jank percentage is reported only when the sample contains at least `--min-frames` rendered frames
(default 30). Below that floor the sample is `insufficient`: `jank.janky_percent` is `null`,
`jank.sample_status` is `insufficient`, `jank.comparable` is `false` and a comparison reason plus a
warning explain the shortfall. An absent gfxinfo frame count is `unavailable`, not zero; a stale or
PID-mismatched CPU sample is likewise rejected rather than reported as a percentage. `--frame-window`
records an explicit sampling window (for example `5s`) and `--warmup` adds a delay before frame
sampling; both are opt-in and make samples easier to compare across runs.

### Frame reports and baselines

```powershell
androidforge diagnostics frames com.example.app --session <ID> --json
androidforge diagnostics frames com.example.app --session <ID> --reset --duration 5s --json
androidforge diagnostics frames com.example.app --session <ID> --reset --duration 5s --baseline .\runs\old\frames\report.json --max-janky 2 --max-p99 5 --max-frame-ms 10 --json
```

`diagnostics frames` collects `dumpsys gfxinfo <package>` once and writes `frames/report.json` as a
run artifact. `--reset` clears gfxinfo counters first and `--duration` waits for an explicit sampling
window before collection; without a reset the window is unknown and is not inferred. The report keeps
frame counts, janky/legacy counters, p50/p90/p95/p99, max frame time, a bounded histogram and a
verbatim header excerpt. Counters the device does not print stay `null` with a warning, never zero.
With `--baseline`, the two reports are compared on jank percentage points, p99 and max frame time using
`--max-janky`, `--max-p99` and `--max-frame-ms` as allowed increases (default 0, so any increase is a
regression). A comparable regression is `E_PERFORMANCE_REGRESSION` (exit 11) with structured deltas;
partial or non-comparable evidence exits 10. `--dry-run` prints the plan without a device.
### Notifications, jobs, alarms, system state

```powershell
androidforge notifications list --package com.example.app --session <ID> --json
androidforge notifications wait --package com.example.app --text "sale" --timeout 30 --json
androidforge notifications assert --package com.example.app --channel deals --title "Sale" --json
androidforge notifications channels --package com.example.app --session <ID> --json
androidforge jobs list --package com.example.app --session <ID> --json
androidforge alarms next --package com.example.app --session <ID> --json
androidforge permissions check POST_NOTIFICATIONS --package com.example.app --session <ID> --json
androidforge permissions grant POST_NOTIFICATIONS --package com.example.app --session <ID> --json
androidforge appops get com.example.app POST_NOTIFICATION --session <ID> --json
androidforge doze status --session <ID> --json
androidforge standby get com.example.app --session <ID> --json
androidforge battery status --session <ID> --json
androidforge network status --session <ID> --json
androidforge network proxy set http://127.0.0.1:8080 --session <ID> --json
androidforge network proxy get --session <ID> --json
androidforge network proxy clear --session <ID> --json
androidforge network profile list --json
androidforge network profile apply wifi-only --session <ID> --json
androidforge network restore --session <ID> --json
androidforge time get --session <ID> --json
```

Short permission names are normalized to `android.permission.*`. `SCHEDULE_EXACT_ALARM` and similar special permissions may need AppOps or user consent; a failed grant reports what remains. Notification/job/alarm/Doze parsers are best-effort across Android versions; raw dumps are preserved.

Interactive toggles (`doze force`, `network airplane`, `time set`) require `--yes`. Time changes usually require root, which AndroidForge will not provide.

Network profiles are `default`, `offline`, `wifi-only`, `data-only` and `no-proxy`. Applying one
records the previous airplane/Wi-Fi/data/proxy state in the session; `network restore` puts it back
and removes the record. Profiles do not simulate latency, loss or bandwidth. `network profile list`
works without a device.

### Reverse mappings and local mocks

```powershell
androidforge network reverse add tcp:8080 --session <ID> --json
androidforge network reverse list --session <ID> --json
androidforge network reverse remove tcp:8080 --session <ID> --json
androidforge network reverse clear --session <ID> --json
androidforge network mock serve --session <ID> --port 8080 --route 'GET /health=200:{"ok":true}' --route 'POST /login=401' --reverse --max-requests 2 --json
androidforge network mock serve --session <ID> --port 8443 --routes .\routes.json --tls-cert .\cert.pem --tls-key .\key.pem --duration 30s --json
androidforge network mock scenarios --json
androidforge network mock serve --scenario newer-release --port 8080 --json
androidforge network mock serve --scenario offline --port 8080 --json
```

Reverse mappings are per device and recorded in the session; `clear` removes them all. `network mock serve` binds loopback by default and never exposes the listener to the LAN. Routes are exact method/path matches (`*` is a wildcard), configured inline as `METHOD /path=STATUS[:BODY]` or with a JSON file:

```json
{"routes":[{"method":"GET","path":"/api/user","status":200,"headers":{"content-type":"application/json"},"body":"{\"name\":\"Ada\"}"}]}
```

Pass `--reverse` to add and later remove `adb reverse tcp:PORT tcp:PORT` automatically. This device half remains gated by `--allow-physical-network` (or `--allow-physical-mutation`) on a physical device. Named scenarios are read-only to list with `network mock scenarios --json` and are always loopback-only, bounded, redacted and host-only: `--reverse` is rejected with a scenario. `--scenario offline` deliberately creates no listener and changes no host proxy/profile setting; the other built-ins (`newer-release`, `bad-digest`, `no-APK`) serve fixture-controlled routes from the scenario catalog. An optional `--routes` file can override the built-in payloads. `--max-requests` or `--duration` bounds non-interactive raw runs; otherwise stop with Ctrl+C. Every request appends method, path, query, redacted headers, bounded body preview and SHA-256, status, matched route, duration and peer to an NDJSON artifact. Authorization, proxy-authorization, cookie, set-cookie and API-key headers are redacted. TLS is a local certificate/key pair only: Android must trust the certificate through its own user-CA/network-security configuration. AndroidForge never claims public CA trust and does not install a CA.

## 12. Runs and reports

For a multi-step flow:

```powershell
androidforge run start --session <ID> --name smoke --json
androidforge run start --name smoke --tag release --tag smoke --meta build=1.2.3 --meta branch=main --json
# subsequent commands with --session <ID> join the active run
androidforge app install .\app-debug.apk --session <ID> --json
androidforge ui assert --session <ID> --text "Welcome" --json
androidforge run end --session <ID> --json
androidforge report show latest --json
androidforge report path latest --json
androidforge report list --json
androidforge report list --tag release --since 7d --json
```

`run end` derives the run status from its steps. Any failed step makes the run `failed`, prints the
failure, and exits with the matching non-zero code. Use `--status failed` explicitly or
`--allow-failures --status passed` only for a deliberate override. `run end --run <ID>` clears the
active pointer when that run is the session's active run, so a later command starts a new run instead
of appending to a finished one; a stale pointer to a finished run is ignored and cleared. A run that
did not install inherits the build identity observed in the session; if none is known it reads the
installed package/version once and marks hash/certificate verification false.

`run start --tag` attaches repeatable labels and `--meta KEY=VALUE` attaches repeatable string
metadata to `report.json` and `metadata.json`. Keys accept letters, digits, `_`, `-` and `.` (at most
64 characters) and values are limited to 1024 characters; keys that look like secrets (`password`,
`token`, `api_key`, `credential`, and similar) are rejected because metadata is stored in reports.
`report list --tag` and `--since` use the same filters as `latest` selection. Reports written before
1.1.0 have no `tags`/`meta` fields and remain readable; they behave as untagged.

### Compare budgets

```powershell
androidforge report compare <RUN_ID> --json
androidforge report compare <RUN_ID> --max-startup-ms 600 --max-jank-percent 10 --json
androidforge report compare <RUN_ID> --max-p99-ms 50 --max-apk-size-bytes 2000000 --max-failed-tests 0 --json
androidforge report compare <RUN_ID> --budget '{"startup_delta_ms":100,"jank_percent_delta":2}' --json
androidforge report compare <RUN_ID> --max-startup-ms 600 --fail-on-regression --json
```

Without a budget, `report compare` keeps the original behavior: structural differences and
performance deltas are reported, but no threshold is evaluated. Adding any `--budget` or `--max-*`
flag evaluates `budget_checks` and adds `budget`, `budget_passed` and `budget_violations` to the
result; missing metrics are skipped rather than failed. `--fail-on-regression` turns any violation
into `E_PERFORMANCE_REGRESSION` (exit 11) with the full comparison and `budget_violations` under
`error.context`. The convenience flags cover absolute ceilings; the `--budget` JSON object also
supports delta ceilings and named metrics. `--max-p99-ms` and `--max-frame-ms` both map to the
`max_frame_ms` budget because p99/longest-frame evidence shares one canonical metric; use `--budget`
when both need independent checks. Baseline selection can be narrowed with `--tag` and `--since`.

### Checkpoints, resume and events

```powershell
androidforge run checkpoint after-login --run <ID> --note "session ready" --json
androidforge run end --run <ID> --status failed --json
androidforge run resume <ID> --from-checkpoint after-login --json
androidforge run events <ID>
androidforge run events <ID> --follow --timeout 5m
androidforge report repro <ID> --json
```

A checkpoint records the current step index and session. Resuming reopens a terminal run, moves the
previous failure into `failure_history`, increments `resume_count` and keeps appending to the same
report. `events` prints raw NDJSON (`run_started`, `step_started`, `step_finished`, `checkpoint`,
`artifact`, `run_finalized`, `resumed`) without the usual JSON envelope. `report repro` emits a
PowerShell script from the run's exact recorded argv.

`report show` and `report path` search the current artifacts directory first, then project-local
`.artifacts/runs` directories under the project tree. This makes a conformance run under
`examples/<App>/.artifacts/runs` inspectable from the repository root without `--project`.

### Recurrence memory

```powershell
androidforge report recurrences --project . --json
androidforge report recurrences --project . --since 30d --json
androidforge report recurrences --project . --package com.example.app --device emulator-5554 --json
androidforge report recurrences --project . --status regressed --json
androidforge report history --fingerprint sha256:<VALUE> --json
```

Recurrence is derived from stored run reports, not a second database. It reports first/last seen,
occurrence count, run ids, devices, app versions, evidence references and `currently_resolved`.
Statuses are `new`, `recurring`, `regressed` and `unknown`. A passing run only proves resolution when it
ran after the failure in the same project/scenario and contains the same failing step as a success. Use
`--all-projects` to aggregate every discovered local artifact store.

### Diagnostic bundles

```powershell
androidforge report bundle latest --output .\evidence.zip --json
androidforge report bundle <RUN_ID> --dry-run --json
androidforge report bundle <RUN_ID> --markdown --json
androidforge report bundle <RUN_ID> --format json --json
androidforge report bundle <RUN_ID> --include-screenshots --include-recordings --json
androidforge report bundle <RUN_ID> --metadata-only --format json --json
```

`--metadata-only` emits the manifest, summary and explanation only; every run artifact is listed as
excluded with the reason `metadata-only bundle requested`. Use it when the report may contain
sensitive screen content and only workflow metadata is safe to share.

The ZIP contains a summary, Markdown explanation, exact invocation argv when the run recorded it,
selected evidence excerpts, bounded logcat, hierarchy XML, `manifest.json` checksums and excluded
artifacts with reasons. Text is redacted; pairing codes are removed explicitly. Media and full raw logs
are opt-in. The original run directory is never modified. The ZIP is written atomically and reopened to
verify every payload checksum before success is reported. `--dry-run` is the inclusion preview.

`--redact-secrets` forces text redaction even when `[redaction].enabled` or `[diagnostics].redact` is
false. After assembly, every text payload is rescanned; if a suspected secret remains, the command
returns `E_UNSUPPORTED_OPERATION` (exit 3) with counts and categories only and writes no output file.
The manifest records `redact_secrets` and safe `secret_scan` counts. Binary screenshots and recordings
are **not pixel-redacted**; only explicitly included as files with their own checksums.

For one existing script:

```powershell
androidforge run exec --session <ID> --name legacy-flow --capture-screen --capture-logcat -- python .\scripts\smoke.py --json
# or: androidforge run --session <ID> --name legacy-flow -- python .\scripts\smoke.py
```

The script receives `ANDROIDFORGE_SESSION`, `ANDROIDFORGE_SERIAL`, `ANDROIDFORGE_PACKAGE`, `ANDROIDFORGE_RUN_ID`, `ANDROIDFORGE_ARTIFACTS`, `ANDROIDFORGE_ADB`, `ANDROIDFORGE_ROOT`, `ANDROIDFORGE_PROJECT`, `ANDROIDFORGE_EXE` and `ANDROIDFORGE_PHASE`. It can call `androidforge` recursively and its steps join the same report.

### Safe artifact inspection and narrow device commands

```powershell
androidforge artifacts scan-secrets --project . --json
androidforge artifacts scan-secrets --path .\\.artifacts --json
# Read-only shell: conservative allowlist; no physical-device grant:
androidforge devices shell --read-only --session <ID> --json -- dumpsys gfxinfo com.example
androidforge devices shell --read-only --allow "cmd package" --deny "package list" --session <ID> --json -- cmd package path com.example
# Exact coordinates use the gated input path (Risk::Input on physical devices):
androidforge devices input tap 540 1200 --session <ID> --json
# The 1.0.0 `device` alias still works for every devices subcommand:
androidforge device list --json
androidforge device shell --read-only --session <ID> --json -- getprop ro.build.version.release
```

`devices` is the canonical command; `device` is the retained 1.0.0 alias, and JSON `command` values
stay canonical (`devices.list`, `devices.shell`, `devices.input.tap`). `artifacts scan-secrets` is
host-only and read-only; it never resolves a session or contacts a device, and its findings never
contain matched values. `devices shell --read-only` requires every command prefix to match an allowlist
(a conservative default list covers common diagnostics) and no deny prefix; it rejects mutating tokens
such as `reset`, `set` and `clear`, and also checks whole-command shapes: bare `date`, `date -u`/`--utc`
and one `+FORMAT`, or exactly `wm size`/`wm density`. `date <digits>`, `date -s`, `wm size WxH` and
`wm density N` are refused before ADB. `devices shell` without `--read-only` requires the master
`--allow-physical-mutation` only, never a scoped flag. Both shell forms record the exact argv,
truncation and redaction flags in a run step. `devices input tap` uses `UiManager::tap_xy`, keeps the
existing narrow physical-input consent gate and returns the device display size. `--dry-run` prints the
exact argv or coordinates and resolves no device.

Import Gradle/JUnit XML without replacing Gradle:

```powershell
# Point at the Gradle result directory; every XML file is imported recursively.
androidforge report import junit .\app\build\test-results\testDebugUnitTest --run <RUN> --json
# Individual files and multiple paths are also accepted.
androidforge report import junit .\test-results\TEST-One.xml .\test-results\TEST-Two.xml --run <RUN> --json
```

Multiple paths/directories are accepted. The summary aggregates suites, tests, failures, errors,
skipped and cases, and each source XML is checksummed under `junit/`.

Prune old runs deliberately:

```powershell
androidforge report prune --older-than 30d --keep-last 20 --dry-run --json
androidforge report prune --older-than 30d --keep-last 20 --yes --json
```

### Maintenance and tag-aware retention

`maintenance` is host-only: it never needs a device session or a physical-consent flag.

```powershell
androidforge maintenance summary --json
androidforge maintenance clean --runs --keep-tag release --json
androidforge maintenance clean --runs --older-than 30d --keep-last 10 --keep-tag release --yes --json
androidforge maintenance clean --sessions --dry-run --json
```

`maintenance clean` is destructive. `--yes` is required to delete; `--dry-run` previews the plan.
Using `--keep-tag` without `--yes` automatically becomes a dry run. `--keep-tag` is repeatable and
protects any run carrying one of the listed tags (OR semantics), on top of `--older-than` and
`--keep-last`. Running runs, runs with an active pointer, and unreadable or unsafe run directories
are always kept. The result retains the legacy `runs` summary and adds an additive, versioned
`retention` plan with every deletion, every protected run and the reason for each decision.

## 13. Fixtures, hooks and conformance

```powershell
androidforge --project .\examples\DiscountHunter fixture list --json
androidforge --project .\examples\DiscountHunter fixture validate smoke --json
androidforge --project .\examples\DiscountHunter fixture run smoke --session <ID> --json
androidforge --project .\examples\DiscountHunter fixture apply state-demo --session <ID> --json
androidforge --project .\examples\DiscountHunter fixture reset state-demo --yes --session <ID> --json
androidforge --project .\examples\DiscountHunter hook list --json
androidforge --project .\examples\DiscountHunter hook run check-deals --session <ID> --json
androidforge --project .\examples\DiscountHunter conformance run --session <ID> --json
```

Use `--dry-run` to inspect a fixture plan and `--yes` for destructive steps. Fixtures never run
automatically. Hooks are explicit debug triggers; do not guess a JobScheduler ID when the app has no
deterministic hook.

A fixture may declare a `state` block (`prefs`, `history`, `installed_version`). It is debug-only:
AndroidForge writes through `run-as` and refuses non-debuggable apps. `fixture apply` and
`fixture reset` are explicit aliases that dispatch **only** the `apply_state` / `reset_state`
actions; no other fixture step runs through them. `--dry-run` validates the pack and reports the
planned action without contacting a device or writing app data. Real runs pass data-mutation
consent to the fixture core only after the physical-device gate accepts the emulator or
`--allow-physical-data` / `--allow-physical-mutation`; `fixture reset` additionally requires
`--yes`. Use `--package` when the adapter has no package. AndroidForge never roots the device, and
`history` uses a host argv writer rather than parsing app-specific protobuf.

A hook can declare a `verify` postcondition. For example, a non-exported debug receiver can be
reached with `run_as = true`; the verification command then reads app state before/after and requires
it to change:

```toml
[hooks.check-deals]
type = "broadcast"
action = "com.example.action.CHECK"
component = "com.example.debug/com.example.DebugReceiver"
run_as = true

[hooks.check-deals.verify]
shell = "run-as com.example.debug cat files/status.json"
must_change = true
timeout_secs = 30
```

If the postcondition is not met, `hook run` fails with `E_HOOK_VERIFICATION_FAILED`; exit 0 from ADB
is not treated as proof.

A conformance result includes the report's build identity (package, version, APK/installed hash,
certificate) and `flow_checks` parsed from the flow script's final JSON `checks` array. The same
checks appear in the conformance step's details in `report.json`; the flow exit code still decides
pass/fail. Local install preflight failures (expected package or certificate) skip the full device
evidence bundle because no device state changed.

### Declarative scenarios

`scenario` is an allowlisted, repeatable wrapper over the audited operations above. A manifest is a
versioned list of steps (`launch`, `tap`, `text`, `wait`, `assert`, `bounds`, `color`, `screenshot`,
`fixture_apply`, `fixture_reset`, `install`). JSON (`.json` or a JSON body) and TOML are both
accepted; unknown step kinds such as `shell`/`adb` are rejected before any device work.

```powershell
androidforge scenario validate .\smoke.toml --json
androidforge scenario run .\smoke.toml --dry-run --json
androidforge scenario run .\smoke.toml --session <ID> --yes --allow-physical-input --json
androidforge scenario run .\smoke.json --repeat 3 --session <ID> --yes --json
```

`scenario validate` is read-only. `scenario run --dry-run` builds the plan only: it dispatches no
step, needs no session and requires no consent. A real run requires `--yes` for any manifest with a
device-mutating step, plus the narrow physical flag for each risk the scenario exercises
(`--allow-physical-app-control` for launch, `--allow-physical-input` for tap/text,
`--allow-physical-data` for fixtures, `--allow-physical-install` for install). Emulators need no
physical flag. Physical capability flags are never exposed or forwarded over MCP.

Each step maps to its existing audited operation and appears as a step in the current run's
`report.json` (screenshots are attached as run artifacts). `on_error = "continue"` records a failed
step and keeps going; the default `"fail"` skips the rest of that iteration. Repeats are aggregated
into a pass rate, duration mean/variance/stddev and a flake summary built by the existing flaky
analysis helper. Exit code is 1 when any step fails.

## 14. Raw ADB

```powershell
androidforge adb -s emulator-5554 shell dumpsys battery --json
```

Raw ADB preserves argument boundaries and returns stdout/stderr/exit status. It is explicitly unsafe and bypasses AndroidForge's higher-level checks. Prefer a typed command when one exists.

## 15. Environment variables

| Variable | Purpose |
| --- | --- |
| `ANDROIDFORGE_SESSION` | Default session id. |
| `ANDROIDFORGE_SERIAL` | Default device serial for some commands. |
| `ANDROIDFORGE_RUN_ID` | Append commands/scripts to a run. |
| `ANDROIDFORGE_OWNER` | Default session owner. |
| `ANDROIDFORGE_HOME` | Overrides global config and state root (state under `<home>/state`). |
| `ANDROIDFORGE_CONFIG_DIR` | Global config directory. |
| `ANDROIDFORGE_STATE_DIR` | State directory. |
| `ANDROIDFORGE_SDK_ROOT` / `ANDROID_HOME` / `ANDROID_SDK_ROOT` | SDK discovery. |
| `ANDROIDFORGE_ADB`, `ANDROIDFORGE_EMULATOR`, `ANDROIDFORGE_AAPT2`, `ANDROIDFORGE_APKSIGNER`, `ANDROIDFORGE_MAESTRO` | Explicit tool paths. |
| `ANDROIDFORGE_APK` | Fixture install APK when no `--apk` is passed. |
| `JAVA_HOME` | JDK discovery when the adapter does not set `[sdk].java`. |

Fixture/conformance processes also receive the documented `ANDROIDFORGE_*` contract above.

## 16. Troubleshooting

| Symptom | Action |
| --- | --- |
| `E_ADB_NOT_FOUND` / `E_SDK_NOT_FOUND` | Run `doctor`; set `ANDROID_HOME` or `[sdk].root`; verify platform-tools. |
| `E_DEVICE_AMBIGUOUS` | `devices list`, then pass `--serial` or `--session`. |
| `E_DEVICE_BUSY` | Another session owns it. Do not kill it; inspect `session list`; reclaim only a dead owner. |
| `E_EMULATOR_BOOT_TIMEOUT` | Check virtualization, AVD health and emulator log under the session directory. |
| `E_SELECTOR_NOT_FOUND` | `ui dump`, then use `--contains`, content description or resource id; `ui wait` for loading. |
| `E_SELECTOR_AMBIGUOUS` | Inspect `matches`; use `--index N` or a narrower selector. |
| `E_TEXT_VERIFICATION_FAILED` | Confirm the field is editable/unfocused by the IME; use `--clear`, or `--no-verify` only for known read-back limitations. |
| `E_IME_VISIBLE` | Run `ui hide-keyboard` or `ui back --hide-keyboard`. |
| `E_INSTALL_SIGNATURE_MISMATCH` | Use a build signed with the installed key, or explicitly back up and `--fresh --yes`; never assume auto-uninstall. |
| `E_SESSION_EXPIRED` | Start/attach a new session. |
| `E_HOOK_NOT_FOUND` | `hook list`; the adapter must define it. |
| `E_UNSUPPORTED_OPERATION` | Read the reason/alternatives; check `capabilities --json`. |
| Doctor says `degraded` | A warning such as unavailable Maestro or disk-free measurement, not necessarily a failure. Read `checks`. |
| Desktop says CLI offline | Run `doctor`; set the CLI path in Settings or put `androidforge.exe` beside the desktop/on `PATH`. |
| Desktop refuses the CLI schema | The desktop and CLI are version-mismatched; use the matching `1.1.0` pair or a compatible release. |
| Desktop has no `Cancel` button | Session lifecycle operations are not cancellable. Run installs/fixtures/conformance/hooks to use cancellation. |

## 17. Safety checklist for agents

1. `doctor --json` and `capabilities --json`.
2. Claim one session; never borrow another owner's device.
3. Install with verification; keep the returned build identity.
4. Use the run model for multi-step work.
5. Prefer semantic selectors and waits.
6. Keep evidence; failure bundles are automatic.
7. Release the session and stop only owned emulators.
8. Treat `--yes`, `--fresh`, `clear-data`, snapshot load, time/network changes and raw ADB as destructive.
9. In the desktop, check the selected project/session in every confirmation; closing the app does not
   release or stop anything. Cancelling a run preserves the emulator.
