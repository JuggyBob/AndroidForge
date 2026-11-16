# Changelog

All notable changes to AndroidForge are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses semantic versioning
within its alpha release line.

## [Unreleased]

No changes yet.

## [0.4.0] - 2026-11-16

Fourth alpha, focused on reducing the number of commands, tokens and manual reasoning an AI coding
agent needs to operate an Android device.

### Added

- `androidforge context` - one compact, section-filterable snapshot of project, environment, session,
  device, application, screen observation, last run and deterministic next actions. Supports `--full`,
  `--refresh-screen`, `--no-screen` and `--include`; returns partial results when a device or optional
  section is unavailable.
- `androidforge report explain <RUN|latest>` - deterministic failure categories, confidence,
  evidence references, stable versioned fingerprints and exact `argv` next actions. Supports `--full`
  and `--markdown`; never calls an LLM or network service.
- `androidforge ui observe` - compact semantic observation with persisted observation tokens, freshness,
  foreground/focus/display metadata, actionable/scrollable nodes, visible text and `--full` hierarchy
  mode. Observation tokens are session-scoped, boot-id-scoped, retained per session and rejected after
  24 hours, cross-session use or a device reboot.
- `androidforge ui diff --from <TOKEN> [--to <TOKEN>]` - semantic added/removed/modified/focus/package/
  activity changes, duplicate resource-id pairing, meaningful bounds-change filtering and explicit
  truncation counts.
- UI postconditions for `tap`, `long-press`, `text` and `scroll`: `--wait-for-text`,
  `--wait-for-absent-text`, `--wait-for-value`, `--wait-for-package`, `--wait-for-activity` and
  `--timeout`. Actions execute once; only observations are polled. Failures return
  `E_POSTCONDITION_TIMEOUT` with the current foreground state, postcondition results, observation token
  and failure evidence.
- `androidforge session ensure` - idempotent session acquisition by project/owner/target, safe reuse,
  warm-emulator attachment, project binding, advisory TTL/lease metadata, expiry reconciliation without
  killing devices, serialized concurrent ensures and ownership-safe alternatives.
- `androidforge doctor fix --plan|--check` - strictly read-only repair planning for SDK, platform-tools,
  ADB, emulator, build-tools, Java, Maestro, licenses, SDK/platform gaps, invalid paths, conflicting SDK
  environment variables and project configuration. No package installation, PATH edit, registry edit or
  download is performed.
- `androidforge agent test` - transparent composition of config validation, session ensure, device
  readiness, APK inspection, installed-build comparison, optional install, optional fixture, optional
  conformance, failure explanation and a consolidated inline/on-disk report. `--dry-run` lists planned
  stages, confirmations and state-changing operations without executing them.
- Stable error codes `E_POSTCONDITION_TIMEOUT`, `E_OBSERVATION_NOT_FOUND`, `E_OBSERVATION_EXPIRED`,
  `E_OBSERVATION_SESSION_MISMATCH` and `E_LEASE_EXPIRED`.
- JSON Schemas for context, observation, UI diff, diagnosis, session ensure, repair plan and agent test.
- Desktop integration: deterministic explanation and copyable next actions in Triage, read-only repair
  plan in Setup, and lease/project binding in Sessions.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.4.0`.
- UI observation defaults to a compact actionable view with explicit `actionable_total` and
  `actionable_truncated`; `--full` returns the complete parsed hierarchy and all actionable nodes.
- Failure explanations prefer the operation-specific step failure over an aggregate command failure,
  classify signature rejection messages even when reported as `E_INSTALL_FAILED`, and surface failed
  conformance check names.
- `report explain` curates up to ten relevant evidence references by default; all artifacts remain in
  `artifacts` and expanded excerpts are available with `--full`.
- APK install failures now record the candidate `apk_path` so diagnosis and next actions reference the
  APK that actually failed rather than a stale session build.
- Session metadata now records `project_root`, `lease_expires_at` and `lease_ttl_seconds`; older
  sessions remain readable.

### Known limitations

- Observation tokens live in the local AndroidForge state directory. They survive normal CLI
  invocations but not manual deletion of that state or a different `ANDROIDFORGE_STATE_DIR`.
- Lease expiry is advisory metadata; it never stops an emulator. Expired leases must be renewed or
  reconciled explicitly.
- `doctor fix --plan` does not repair environments automatically; this is deliberate.
- `agent test` supports one APK, one fixture and one conformance flow per invocation.
- Physical devices, Maestro execution, instrumentation-package validation, snapshot restore, root-only
  operations, multiple DPI scales and macOS/Linux hosts remain unvalidated.

## [0.3.0] - 2026-11-16

Third alpha, released as one integrated desktop and backend update.

### Added

- First-run setup wizard covering CLI discovery and compatibility, actionable environment diagnostics,
  workspace/project selection, a safe device verification step and a final readiness summary. The
  wizard does not install APKs or reset user data.
- Actionable `doctor` fields: `required`, `blocking`, `impact` and `suggestion`, so missing or broken
  components explain why they matter and what to do next.
- GitHub release update checks and a verified CLI repair path. Downloads are explicit, checked against
  the published SHA-256, staged, validated and rolled back on failure. The previous executable is kept.
- Failure triage with failure metadata, build identity, artifacts, related live views and copyable
  Markdown/JSON reports.
- Compatible run comparison with project/scenario matching, build/environment change warnings, step
  regressions, new/resolved failures, duration deltas and artifact differences.
- Flaky-step analysis with a transparent minimum-run heuristic and per-step observation history.
- Run lifecycle outcomes `cancelled`, `interrupted`, `timed_out` and `abandoned`, creator PID/host
  metadata, `report reconcile`, and startup reconciliation of unfinished runs.
- Read-only live capture contracts: `capture frame`, `diagnostics logcat peek --since`, `ui inspect`,
  plus `report compare`, `report flaky`, `session prune` and `maintenance summary|clean`.
- Native live logcat panel with bounded buffers, incremental tailing, package/level/search filters,
  pause/resume, follow-latest, crash/ANR highlighting, disconnect state and export.
- Native periodic device preview and semantic UIAutomator inspector with hierarchy tree expansion,
  search, raw XML, bounds highlighting, selector copying, tap/long-press/type/scroll/back/home actions
  and a clearly labelled coordinate fallback. Preview is documented as periodic capture, not video.
- Maintenance center with storage accounting, dry-run previews, explicit confirmation, active-session
  protection and terminal-only pruning. Preview state is cleared after a successful cleanup.
- Portable ZIP and per-user Inno Setup installer, including both executables, license, documentation,
  Start Menu/optional desktop shortcuts and an uninstaller. User configuration and artifacts are kept
  outside the install directory and are not removed by upgrade or uninstall.
- Embedded Inter and JetBrains Mono fonts with OFL license files.
- Comprehensive desktop redesign: navigation groups, typography scale, semantic colors, reusable
  cards/tables/badges/empty states, sticky wizard footer, keyboard shortcuts, responsive layouts and
  automatic Windows DPI handling.
- JSON schemas for analysis and maintenance results, plus expanded run/product documentation.

### Changed

- Workspace version is `0.3.0`; the desktop refuses a CLI whose version or envelope schema does not match.
- CLI locator also checks the per-user install directory and `PATH`.
- Emulator detached-process handling now keeps an invisible console for hidden children, avoiding a
  stray visible Windows Terminal/QEMU console window on headless starts.
- Cancellation no longer opens an error dialog; it reports the cancelled outcome in the operation panel
  and reconciles the run as `cancelled` while leaving the session intact.
- Operation duration freezes at the final measured duration instead of continuing to climb.

### Fixed

- Settings icon renders as a painted gear rather than relying on an unsupported font glyph.
- Long paths in cards truncate with copy/open actions instead of spilling under adjacent cards.
- Logcat tag columns clip with ellipsis and no longer overlap message text.
- Logcat treats an empty package filter as all processes, and incremental tailing uses the previous
  device timestamp instead of suppressing repeated identical lines.
- Maintenance preview no longer remains visible after data has been removed.
- Inspector duplicate error banners are deduplicated and disconnected previews show an overlay while
  retaining the last captured frame.
- Sticky setup footer keeps Back/Continue/Finish reachable at 1280×720 and smaller layouts.

### Known limitations

- Binaries are unsigned. Windows SmartScreen may warn; no publisher identity is claimed.
- Device preview is periodic screenshots (roughly 1–5 FPS), not a low-latency video stream.
- Physical-device, Maestro, instrumentation-package, snapshot-restore and root-only paths remain
  unvalidated in this release.
- The update checker only offers a compatible CLI sidecar. It does not silently install a new desktop
  build.

## [0.2.0] - 2026-11-16

Second alpha. Adds the native AndroidForge Desktop cockpit while keeping the CLI as the authoritative
automation surface.

### Added

- `androidforge-desktop.exe`, a native Rust egui/eframe companion with the AndroidForge icon embedded in
  the window and executable. It uses the glow renderer and has no WebView, browser, HTML, JavaScript,
  Node or frontend build.
- P0 read-only cockpit: project discovery, environment overview, devices, sessions, report-backed runs,
  run details, build identity, steps, failure details, evidence tabs, raw report/metadata/manifest
  views and checksum-manifest verification.
- Evidence viewer for screenshots (fit/100%/zoom), hierarchy and log previews with line numbers and
  search, safe artifact joining and external-open shortcuts.
- P1 controlled actions: emulator session start, device attach, session release/stop/restart, APK
  selection and inspection, install dry-run and verified install, fixture/conformance/hook launch,
  streamed output and process-tree cancellation.
- Session-target selection, prepared-settings inspection and explicit destructive confirmations.
- Native Windows process-tree termination for cancellable desktop operations (installs, fixtures,
  conformance and hooks); session lifecycle operations run to completion because they must not leave a
  half-owned emulator. Cancelling a run never stops, restarts or releases the emulator, and the UI says
  so. A cancelled run leaves an incomplete report and a clear explanation instead of a parser error.
- CLI locator with explicit path, sidecar, workspace-build and `PATH` resolution, compatibility probing,
  schema refusal for unknown contracts, structured CLI error display and in-app Settings.
- Desktop integration tests with a compiled mock CLI covering version probing, typed results, structured
  errors, invalid JSON, timeouts, streamed output and cancellation.
- Documentation for the desktop cockpit, including architecture, user guide, capabilities, verification
  and portable distribution.

### Changed

- Workspace version is `0.2.0`; MSRV is `1.95`.
- CI builds and uploads both executables. Release assets now include a standalone CLI zip, a combined
  desktop+CLI portable zip and `SHA256SUMS.txt`.
- Busy-state checks use only a running operation, so completed output no longer disables new actions.
- Session actions are disabled for released/abandoned tombstones.
- Clippy `collapsible_if` edits across the workspace for the current stable toolchain.

### Fixed

- Desktop version probing invokes `androidforge --version` directly, matching the CLI contract.
- Run detail requests use `report show` and `report path` instead of ambiguous top-level verbs.
- Confirmations render above settings/install/run dialogs; installing closes the install dialog before
  confirming and reopens it with the measured result.
- Table row selection works reliably and explicit `Select`/`Open` buttons are available.
- Long commands no longer overlap operation buttons.
- Cancelled operations explain the missing final JSON report instead of showing a raw parser error.
- Artifact path checks normalize Windows verbatim prefixes and report XML uses the document root
  element.

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

[Unreleased]: https://github.com/JuggyBob/AndroidForge/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.4.0
[0.3.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.3.0
[0.2.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.2.0
[0.1.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.1.0
