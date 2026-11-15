# Architecture

AndroidForge is a single-process, no-daemon Rust CLI. Every invocation performs discovery, claims or checks a session, does bounded work, records evidence, and exits. Long-lived things (emulators, screen recordings, continuous logcat) are explicit managed subprocesses with status and stop commands.

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
| `androidforge-test-support` | Test-only helpers (not part of the runtime dependency graph). |

Dependency direction is acyclic: `cli` is the composition root; feature crates depend on `core`, `process`, `adb` and each other only in the direction of their responsibility. No feature crate depends on `cli`.

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
