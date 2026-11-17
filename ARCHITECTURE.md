# Architecture

AndroidForge is a single-process, no-daemon Rust CLI plus a separate native desktop sidecar. Every CLI invocation performs discovery, claims or checks a session, does bounded work, records evidence, and exits. Long-lived things (emulators, screen recordings, continuous logcat) are explicit managed subprocesses with status and stop commands. The desktop never owns state of its own beyond UI preferences; it reads and writes through the CLI.

## Workspace

| Crate | Responsibility |
| --- | --- |
| `androidforge-cli` | Binary, argv preprocessing, clap command tree, output, command handlers, run scope. |
| `androidforge-core` | Errors/exit codes, config/adapter, paths, sessions/locks, runs/reports, artifacts, redaction, toolchain discovery. |
| `androidforge-process` | Bounded process execution, UTF-8-safe capture, Windows process-tree cleanup, detached processes. |
| `androidforge-adb` | ADB discovery, `adb devices -l` parsing, device state, shell/pull/push helpers. |
| `androidforge-emulator` | AVD discovery, safe ports, headless launch, boot wait, stop, snapshots, device preparation/restore. |
| `androidforge-app` | APK inspection/signature verification, install diagnosis, package/app files, permissions, AppOps. |
| `androidforge-ui` | UIAutomator XML parsing, selectors, actions, waits, settle, verified text entry. |
| `androidforge-diagnostics` | Screenshots, recordings, logcat lifecycle, crashes/ANRs, dumpsys, failure bundle. |
| `androidforge-automation` | Fixtures, hooks, conformance, Maestro, instrumentation, external processes. |
| `androidforge-desktop` | Native egui/eframe cockpit, CLI locator/runner, typed models, evidence viewer, P1 operations. |
| `androidforge-test-support` | Test-only helpers (not part of the runtime dependency graph). |

Dependency direction is acyclic: `cli` is the composition root; feature crates depend on `core`, `process`, `adb` and each other only in the direction of their responsibility. No feature crate depends on `cli`. `androidforge-desktop` depends on none of the runtime feature crates; it invokes the CLI executable through its JSON contract.

## Startup

`main` runs the async runtime on a dedicated 32 MiB-stack worker thread. The orchestration future nests process capture, UI waits, fixture steps and diagnostic capture; the Windows main-thread default stack was too small for deeply nested async state machines. The runtime is current-thread Tokio: the CLI is not a server and does not need a worker pool.

`preprocess` allows ergonomic forms such as `androidforge run -- python ...` and `androidforge diagnostics logcat ...` without a custom full parser. It only inserts `exec`/`tail` where the top-level command makes the intent unambiguous; all remaining parsing is clap.

Project discovery walks upward from the current directory (or `--project`) looking for:
1. `.androidforge/project.toml`
2. `.androidforge/config.toml`
3. a root `project.toml` containing `adapter_version` (portfolio/example layout)

The merged config precedence is CLI/profile > environment contract > project adapter > global config > defaults. Unknown adapter keys are rejected.

## Output model

All meaningful commands build a `CommandSuccess` with a command name, JSON `result`, human text and optional artifacts. The output layer serializes one envelope when `--json` is active; otherwise it prints human text. Diagnostics/progress/trace go to stderr.

Errors carry a stable `ErrorCode`, message, recoverability, suggestions, context map, optional command/device/session and artifacts. The process exit code comes from the code, not from a generic `1`.

## Agent loop

The v0.7.0 agent-facing layer is a composition of existing primitives, not a second execution model.

### Context

`context` assembles sections from project config, toolchain discovery, session metadata, a single
device inspection, app status, a recent or fresh observation and run summaries. It does not require a
device to return project/environment/session metadata; unavailable sections are explicit. A screen
observation younger than five seconds is reused unless `--refresh-screen` is passed. Observations are
written by the same observation token store used by `ui observe`, so `context` never invents freshness.

### Observations and tokens

`ui observe` parses one UIAutomator hierarchy, then derives a compact semantic projection. Tokens are
written atomically under `<state>/observations/<session>/<token>.json`, with the raw XML in the same
directory. Records include the session id, boot id, timestamp and parsed projection. The compact node model records `token`, `clickable`, `enabled` and `editable`; empty resource ids are omitted. Reads validate
session scope, boot id and the 24-hour maximum age; the per-session retention limit is 20 records. The
store is filesystem-only and requires no daemon. Diffs pair nodes by resource id when available and by
path/class/package otherwise, pair duplicate ids in order, ignore sub-threshold bounds movement and cap
the returned lists with explicit totals. The default answer separates interactive `actionable` nodes from non-interactive `context` text and caps diff node lists at 12 unless `--full` is requested.

### Deterministic diagnosis

`androidforge-core::diagnosis` maps stable error codes, then normalized failure messages, to categories
and confidence. It uses no network, model, telemetry or probabilistic service. Evidence is curated from
stored report/step artifacts (command output, hierarchy, crash reports, app status, screenshots and
filtered logcat); raw dumpsys/logcat remain available through the full artifact list. Next actions are
built as argv arrays for implemented commands. Fingerprints hash a versioned, normalized material string
(category, code, operation, package, selector, step and normalized message) and exclude run ids,
timestamps, temporary paths, memory addresses and dynamic numeric ids.

### Action receipts

Receipts reuse the observation store and semantic diff rather than adding a parallel model. Before an
action, the CLI captures a normal observation. The action is dispatched once through the existing UI
manager. Postcondition polling refreshes observations only. After the action (or postcondition) the CLI
captures another observation, computes the same diff used by `ui diff`, and classifies the effect with
fixed precedence: unknown/inconsistent, package change, dialog appearance/disappearance, activity
navigation, removal-only disappearance, content change, then no visible change. Completeness is
`complete`, `before_missing`, `after_missing` or `incomplete`; dispatch failure is stored separately
from postcondition failure.

### Failure recurrence

`androidforge-core::recurrence` derives memory from existing `report.json` files. Failures are grouped
by project scope + fingerprint. A passing run counts as a resolution proof only when it is later than
the failure, shares project/command/package identity and contains the same failing step as a successful
step. Unrelated or non-exercising successes are ignored. Classification is `new`, `recurring`,
`regressed` or `unknown`; the store remains the existing run artifact tree, so no migration or second
database is required.

### APK resolution and comparison

`[build.variants]` maps variant names to project-relative paths or bounded glob patterns. Resolution
walks only the project, rejects traversal and symlinks, applies deterministic sorting (mtime, then
relative path), rejects malformed APKs and treats equal-timestamp/different-hash candidates as an
ambiguity error. It never invokes Gradle. `apk diff` pulls the installed base APK to a temporary
directory, inspects it with the same APK inspector and compares package, versions, hashes, certificate,
SDKs, launchable activity and split shape. `--only-if-changed` skips only on a proven byte-equivalent
base APK with no split ambiguity.

### Diagnostic bundles

`report bundle` builds a payload plan from the stored explanation, evidence references and artifacts.
Text payloads are redacted, per-file and total size limits are applied deterministically, paths are
canonicalized inside the run directory, names are generated by AndroidForge, and the ZIP is built in
memory, written atomically and reopened for checksum verification. The manifest is written last and is
not included in its own checksum list. Screenshots, recordings and full logs require explicit opt-in.

### Stable element references

Every observation assigns opaque `el-...` refs to the nodes it returns. The ref material includes the
observation token, path, semantic attributes and bounds, so equivalent nodes are deterministic within
one observation but not reused across observations. The stored observation is authoritative. Before an
action, the CLI loads the token, checks session/boot/device identity, finds the ref, then calls the UI
manager with a `NodeExpectation`. The manager performs one fresh hierarchy dump and refuses dispatch if
the path, identity, bounds, visibility, enabled state or actionability changed. No selector or coordinate
fallback exists.

### Matrix execution

`agent matrix` validates targets, resolves the build once and acquires one owned session per target.
Execution is sequential and each target calls the existing `agent test` pipeline as a child process, so
the matrix does not duplicate install/conformance logic. Health is collected before the child run. Only
sessions created by the matrix with the matrix owner are released; pre-existing sessions and emulators
are not stopped. Parallel mode is rejected until independent-device isolation is validated.

### Wireless ADB and health

Wireless pairing/connect/disconnect are thin semantic wrappers over standard ADB commands. Pairing codes
are validated as six digits, sent through stdin when requested, redacted from output and never placed in
the result. Health is read-only: it runs bounded `dumpsys`/`/proc`/`ip` queries and records a reason for
every unavailable metric. Thresholds are configuration, not hardcoded device policy, and only produce
warnings so an unknown noncritical metric cannot reject a test.

### Read-only retries

`androidforge-core::retry` owns the allow-list, transient classification, maximum attempt count, backoff
and total deadline. `common::retry_read_only` applies it only at read-only call sites. A retry report is
attached to success or error results; attempts are never concealed. State-changing operations have no
retry path.

### Orchestration

`session ensure` serializes identical ensure requests with a filesystem lock, then reuses a compatible
session, attaches to a warm unowned emulator or launches the target. Project/owner/target compatibility
is checked before reuse; ownership is enforced by the existing device lock. Lease fields are advisory
metadata only.

`agent test` is an explicit stage machine in the CLI: config validation, session resolution, device
check, APK inspection, installed-build comparison, optional install, optional fixture, optional
conformance, explanation and consolidated report. It calls the same internal operations as the
corresponding CLI commands and records the equivalent argv, duration and status for each stage. It never
passes `--fresh`, never clears data and never uninstalls. A dry run stops before any session or device
mutation and reports required confirmations.

## Desktop cockpit

`androidforge-desktop` is a separate binary, not a library dependency of the CLI. It has four layers:

1. **Backend** — CLI resolution, compatibility probe, runner, JSON envelope parser, evidence reader,
   project discovery and process manager. All CLI calls go through `std::process::Command` with argv
   arrays, hidden consoles, bounded timeouts and typed envelopes.
2. **Models/state** — serde models of CLI results, cached `DataState` values, selection, forms,
   confirmations, toasts and operation state. The desktop does not invent success: a run passes only when
   the report says it passed.
3. **Workers** — background threads run CLI calls and send typed events over `mpsc`; the egui frame only
   drains events and renders. Refreshes are interval-driven and pinned while an operation is running.
4. **Views/dialogs** - Setup/onboarding with read-only repair planning, Overview, Devices, Sessions with
   lease/project binding, Runs, Triage with deterministic explanations and copyable next actions,
   Compare, Logcat, Inspector, Maintenance, run details/evidence and operation output. Confirmation and
   error dialogs render above ordinary windows.

CLI resolution order is explicit settings path, executable beside the desktop, workspace `target/debug`
or `target/release` during development, then `PATH`. The compatibility probe checks `--version` and the
report schema before enabling actions. Unknown schemas are refused with a desktop error instead of
mis-parsing.

Read-only surfaces are environment/runs/evidence plus logcat, preview, hierarchy inspection, analysis and
maintenance previews. Controlled actions are start/attach/release/stop/restart, APK inspect/dry-run/install,
fixture/conformance/hook launch, semantic UI actions, confirmed cleanup, output streaming and cancellation.
Every destructive action has an explicit confirmation. Install closes the install window before
confirming and reopens it with measured identity; run details request `report show`/`report path` and
never infer from process exit codes. Update checks use the public GitHub release; a CLI repair download is
verified against the published SHA-256 and staged before replacement.

Cancellation is limited to operations that can be terminated safely: installs, fixtures, conformance flows and
hooks. Session lifecycle operations (start/attach/release/stop/restart) run to completion because killing
their child tree could leave a half-owned emulator or an inconsistent lock. For cancellable operations the
desktop walks the Windows process snapshot, terminates descendants deepest-first, and falls back to
`taskkill /T /F`. It never calls `session stop` or `session release`; the emulator is not a child of a
cancelled run and stays alive. The UI reconciles the unfinished run as `cancelled`; startup reconciliation
marks abandoned `running` reports `interrupted` instead of leaving them running forever.

The desktop uses eframe 0.37 with the glow renderer and embedded PNG/ICO assets. There is no WebView,
HTML, JavaScript, Node, React or browser frontend, and no network service.

## Runs and reports

A run is the evidence unit. `RunContext` owns:
- `report.json` — status, command, project, device, build, steps, artifacts, exit code;
- `metadata.json` — tool/host/toolchain/project/session/device at start;
- `manifest.json` — every file with size and SHA-256.

Native commands use the `execute_in_run!` macro. It begins a run (explicit `--run`/`ANDROIDFORGE_RUN_ID`, an active run for the session, or a new implicit run), starts a step, executes, then finalizes implicit runs or saves explicit ones.

`run start` creates an explicit open run and records it as active for the session/project. Subsequent commands join it. `run end` finalizes it. `run exec` / `run --` runs one external program with the environment contract, captures stdout/stderr/logcat/screenshots, and adds its exit status to the same report. External scripts can recursively call `androidforge` because `ANDROIDFORGE_RUN_ID` is injected.

`report import junit` parses JUnit XML and appends a step to the run; it never executes tests.

Run start/stop pairs (recording, logcat) deliberately leave a run open while the capture is active. `capture record stop` finalizes a run only when AndroidForge created it implicitly for that recording; inside a caller-owned run it saves and leaves the run open. This prevents a recording from closing a conformance report.

## Sessions and ownership

A session is a JSON record under the local state directory containing:
- id, owner, kind (emulator/attached), state, serial, AVD, port, headless, emulator PID/args;
- created/heartbeat/updated timestamps, creator PID, current operation lock;
- recorded device settings for restoration;
- snapshot history and notes.

State directory:
- Windows: `%LOCALAPPDATA%\AndroidForge` (or `ANDROIDFORGE_HOME`/`ANDROIDFORGE_STATE_DIR`);
- global config: `%APPDATA%\AndroidForge\config.toml` (or `ANDROIDFORGE_HOME`/`ANDROIDFORGE_CONFIG_DIR`);
- project state pointer: `.androidforge/current-session`.

Ownership is cooperative between AndroidForge processes:
1. A global `fs2` exclusive lock serializes device-lock creation.
2. A per-device lock file records the owning session.
3. A different live session gets `E_DEVICE_BUSY`.
4. Released/failed/abandoned owners, or a starting/stopping session whose operation PID is dead past a grace period, can be cleaned up.
5. `session gc --reclaim` is the explicit stale recovery path; it is never automatic for a live owner.

A heartbeat is updated by session-mutating operations. A session is stale after two hours without a heartbeat for GC purposes. The process cannot prevent unrelated ADB clients from touching the same device; the locks only coordinate AndroidForge.

Device preparation records previous values before changing animation scales, stay-awake, locale, timezone, density, orientation, font scale or dark mode. `devices restore`, `session release` and `session stop` restore what was changed. Only settings AndroidForge changed are restored.

## Emulator lifecycle

- Discovery uses the SDK emulator executable and `avd list`.
- Console ports must be even in the documented range; ADB uses port + 1. AndroidForge checks console, ADB and gRPC collisions before launch.
- New emulators default to headless with `-no-audio -no-boot-anim -no-snapshot-save -no-window`.
- Boot wait polls `sys.boot_completed`, device state and emulator process liveness with a bounded timeout.
- Stop is allowed only for an emulator owned by the session. Attached emulators require explicit `--stop-attached`.
- Snapshots list/save/load/delete are best-effort because emulator builds differ. Load is destructive and requires `--yes`.

## APK install and identity

The install pipeline is deliberately evidence-first:

1. `aapt2 dump badging` obtains package, versions, SDKs, label, launchable activity, architectures.
2. SHA-256 is computed locally.
3. `apksigner verify --print-certs` obtains signer certificates and schemes. Java is required for the apksigner JAR.
4. `adb install` runs with a bounded timeout; failure output is parsed into a stable reason where possible.
5. The installed base APK is located with `pm path` and pulled.
6. The pulled APK is hashed and compared with the local APK.
7. The installed package/version and certificate are compared with the requested build.
8. The result reports package, versionName/versionCode, local and installed APK SHA-256, certificate SHA-256, debug-signed and `verified`.

A certificate mismatch is `E_INSTALL_SIGNATURE_MISMATCH` with both identities and the installed version. It is never auto-uninstalled. `--fresh --yes` is the explicit destructive escape hatch. Expected certificates in an adapter are declarations, not observed data; a run that did not install does not copy them into `build.cert_sha256`.

## UI automation

1. Dump the hierarchy with `uiautomator dump` to a device temp path and pull the XML.
2. Parse with `roxmltree` into nodes with text, content description, resource id, class, bounds, clickable/enabled/focused/scrollable/checked/visible and package.
3. Selectors default to exact text; `--contains`, content description, resource id, class, index and state flags narrow matches.
4. Zero matches is `E_SELECTOR_NOT_FOUND`; multiple matches is `E_SELECTOR_AMBIGUOUS` with candidates and an `--index` suggestion.
5. Taps use real bounds and optionally walk to the nearest clickable ancestor. Coordinates are current-device pixels, never scaled from an unrelated screenshot.
6. Text entry validates the focused editable field, clears explicitly with batched deletes when asked, sends text, re-reads and compares. Password fields report `verified=false` because Android does not expose their contents.
7. `ui back` refuses while the IME is visible unless `--hide-keyboard` is explicit.
8. `ui wait`, `ui settle` and `ui scroll-to` poll hierarchy state with bounded intervals; fixed sleeps are not the core pattern.

Compose nodes are visible only when the app exposes semantics through accessibility (`testTagsAsResourceId` or equivalent). Internal test tags are not magically visible; instrumentation is the correct fallback.

## Diagnostics

- Screenshots use `adb exec-out` and are written as binary; terminal redirection is never used.
- Screen recordings use Android `screenrecord`, cap at 180 seconds, run detached, and are stopped robustly, pulled and checksummed.
- One-shot logcat is bounded by line count and bytes and never clears the global buffer by default.
- Continuous logcat runs on-device with `timeout` and `head -c`, plus a local state file and stop/status lifecycle.
- Crash/ANR detection is labelled best-effort; raw buffers are preserved.
- Process exists/alive, task exists, activity exists, foreground and resumed are separate fields, because they fail differently.
- A failure bundle captures a screenshot, hierarchy/focused nodes, logcat tail, crash state, app state and dumpsys evidence with paths in the error.

## Fixtures, hooks and conformance

Fixtures are versions of project state, not hidden test code. The runner resolves templates, merges adapter and `--var` values, selects phases, enforces confirmation for destructive actions, retries bounded steps and records each step. `${PROJECT_ROOT}` and `${ENV:NAME}` expand to a fixpoint, so variables may themselves contain templates.

Hooks are named explicit triggers: broadcast, deep link, shell, instrumentation or local command. The name has no meaning to AndroidForge. JobScheduler IDs are never guessed.

Conformance is a thin manifest with a script, Maestro flow or fixture, plus expectations for status, exit code, artifact globs and JSON paths. The script receives the environment contract and uses the same run. AndroidForge records the external output and checks the expectations, but does not invent a test DSL.

## Process control

`androidforge-process`:
- passes argv as an array (no shell concatenation) and preserves arguments for raw ADB;
- applies a timeout and output caps;
- decodes UTF-8 losslessly with replacement instead of failing on device bytes;
- kills the process tree on Windows on timeout/failure paths;
- spawns detached children for emulator/logcat/recording with explicit state and stop commands.

## Redaction

Text artifacts and report strings pass through a default redactor for password/secret/token/authorization shapes, bearer tokens, private-key blocks and common cloud/GitHub token prefixes. Boolean structural attributes such as `password="false"` are preserved. Extra regexes can be configured, and `[diagnostics].redact = false` disables the default; the latter is not recommended.

## Generic core

The `genericity` integration test scans every runtime crate's `src/` for app names, package IDs and project-specific tokens. Project behavior belongs in adapters, fixtures, hooks, conformance scripts and project data. CI should run this test, `cargo fmt --check`, clippy with `-D warnings`, and the full workspace test suite.
