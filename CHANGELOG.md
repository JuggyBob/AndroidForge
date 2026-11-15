# Changelog

All notable changes to AndroidForge are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses semantic versioning
within its alpha release line.

## [Unreleased]

No changes yet.

## [0.1.0] - 2026-11-15

First public alpha of the local-first AndroidForge CLI.

### Added

- Rust 2024 workspace with nine runtime crates plus test support; one native `androidforge.exe`.
- Global `--json`, `--trace`, `--dry-run`, `--yes`, `--session`, `--serial`, `--project`, `--run`, `--quiet` and `--no-failure-capture`.
- Stable exit codes, structured errors with suggestions/context/artifacts, and command suggestions for misspellings.
- `doctor`, `capabilities`, `schema`, `init`, `config show|validate|path`.
- ADB discovery, robust `adb devices -l` parsing, device inspection, wait, prepare and restore.
- AVD listing, headless emulator start, boot wait, safe port checks, status and stop. Unowned emulators are never stopped.
- Session start/attach/claim/list/inspect/status/release/stop/use/current/gc/restart with heartbeat, operation locks, dead-operation recovery and explicit stale reclaim.
- Snapshot list/save/load/delete with confirmation for destructive loads and honest unsupported errors when the emulator refuses.
- APK inspect/verify via `aapt2` and `apksigner`.
- App install with package, versionName/versionCode, local and installed APK SHA-256, certificate SHA-256, debug-signed status and verification.
- Signature-mismatch diagnosis without automatic uninstall.
- App launch/status/force-stop/kill/uninstall/clear-data/list/info and `run-as` file push/pull for debuggable apps.
- UIAutomator hierarchy dump/find/focused, semantic selectors, ambiguity errors, verified text entry, clear, wait, assert, settle, scroll, scroll-to, swipe, Back/Home/keyevent and IME-aware Back.
- Screenshots with display metadata and SHA-256; screen recording with Android's 180-second limit and reliable stop; bounded one-shot and continuous logcat.
- Crash/ANR detection, process/task/activity/foreground separation, dumpsys collection and automatic failure bundles.
- Notifications, jobs, alarms, permissions, AppOps, Doze, standby, battery, network and time commands, with best-effort labels where parsing varies.
- Versioned fixtures with phases, retries, timeouts, failure policy, capture policy, destructive confirmation, dry-run, environment contract and app-private file round-trips.
- Named project hooks: broadcast, deep link, shell, instrumentation and command.
- Thin conformance manifests with script, Maestro and fixture flows, artifact globs and JSON expectations.
- Optional Maestro wrapper and instrumentation wrapper; no Gradle replacement.
- One-run model with `report.json`, `metadata.json`, checksum `manifest.json`, active runs, failure bundles, `report show|path|prune`, and `report import junit`.
- Bounded process runner with timeout, output caps, UTF-8 replacement and Windows process-tree cleanup.
- Example adapters and real conformance flows for CalcSuite, DiscountHunter and IronPulse.
- Case-study evidence from real Android 16 emulator runs.

### Fixed

- `permissions check/grant/revoke` now accepts short names such as `POST_NOTIFICATIONS` and normalizes them to `android.permission.*`; Android 16's removal of `pm check-permission` falls back to `dumpsys package` without noisy false failures.
- APK install accepts both the release and debug application ids declared by an adapter. An explicit `--expected-package` still narrows the check. `--expected-cert-sha256` adds an ad-hoc certificate check.
- APK install enforces `[project].expected_cert_sha256` before installing, so a build signed by an unexpected key fails with `E_SIGNATURE_VERIFICATION_FAILED` and the actual fingerprint.
- APK install `--dry-run` now inspects the APK and reports package/version/hash/certificate plus planned constraints. `clear-data --dry-run` no longer requires `--yes`.
- Fixture templates now resolve variables that themselves contain `${PROJECT_ROOT}` or `${ENV:NAME}` to a fixpoint.
- `capture record stop` no longer finalizes a caller-owned run; it only finalizes a run AndroidForge created implicitly for the recording.
- Run pre-install build identity uses the debug package when configured and no longer copies an expected certificate into an observed field before an install verifies it. Installs persist identity in the session, so later runs/conformance reports inherit the measured build.
- `run end` derives status from failed steps, refuses an implicit `passed` result, and exits non-zero for failed/partial runs.
- `ui clear` verifies an empty field when Android reports the empty text as absent, instead of returning `E_TEXT_VERIFICATION_FAILED`.
- `session start` no longer lets the detached emulator inherit the caller's stdout/stderr pipe; captured invocation returns after Ready.
- `session stop` can stop a released tombstone's owned emulator if no new session took ownership; `session list --active` hides tombstones.
- `report list` was added; `report import junit` accepts multiple files or a Gradle result directory.
- Hook `verify` postconditions and `run_as` broadcast delivery were added. ADB exit 0 is no longer treated as proof that the app changed state.
- `notifications channels` parses the full `AppSettings` channel catalog, not only active notification records.
- `jobs list` parses Android 16 named/WorkManager namespace headers and package `Source` lines; `alarms next` parses `origWhen`/`Alarm{...}` forms.
- `doctor` now reports actual free space and says “no project adapter found” rather than validating defaults as if an adapter existed.
- `avd clone` now has an explicit command that returns `E_UNSUPPORTED_OPERATION` and supported alternatives instead of falling through to an unknown-command error.
- `--json` appended to a raw `adb` command is moved before `adb`; interior occurrences get a clear ordering error. `capture screenshot` exposes `result.path`.
- Example conformance scripts treat `ui find` as a search (empty `matches` is not "found"), handle real first-run onboarding, and CalcSuite now checks the 3.0.3 updater.
- `run end --run <ID>` clears the session's active-run pointer when it names that run. Commands no longer append to a finished run after an explicit end; stale finished-run pointers are cleared automatically.
- `session stop`, `session release` and `session restart` require the owner label recorded at start (or `ANDROIDFORGE_OWNER`); `--force` is the explicit cross-owner takeover. This closes the path where one agent could stop another agent's emulator by session id.
- `ui clear` reads UIAutomator `hint`/`hint-text` and treats an exact hint as empty before typing and after clearing; Android Settings search now clears without a false failure. Clear with no selector reports `E_SELECTOR_NOT_FOUND` instead of a text-verification error when nothing editable is focused.
- `report show`/`report path` fall back to project-local `.artifacts/runs`, so conformance evidence is inspectable from the repository root without repeating `--project`.
- Conformance command results now include the measured build identity and `flow_checks` parsed from the external script's final JSON `checks` array; the conformance step details carry the same checks.
- Local install preflight failures (expected package/certificate) skip the full failure bundle because the device was not touched.
- `app launch` retries `am start` once after a transient package/launcher race and reports the retry as a warning.
- Human-mode errors summarize embedded run reports, JUnit summaries and external results instead of printing the full JSON context.
- Active-run pointers without a session include the project root, so two projects cannot overwrite each other's active run.

### Known limitations

- Physical devices, Maestro execution, a real instrumentation package, snapshot restore and time changes requiring root were not exercised in this delivery.
- AVD cloning and named-profile isolation are not implemented and return `E_UNSUPPORTED_OPERATION`.
- Compose test tags are visible only when exposed through accessibility semantics; use instrumentation for internal tags.
- Notification/job/alarm/Doze parsing is version-dependent and remains best-effort with raw evidence preserved.
- No GUI, daemon, cloud, telemetry, HTML reports, visual diff, release publishing or signing automation.

[Unreleased]: https://github.com/JuggyBob/AndroidForge/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.1.0
