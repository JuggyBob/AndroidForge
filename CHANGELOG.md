# Changelog

All notable changes to AndroidForge are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses semantic versioning
within its alpha release line.

## [Unreleased]

No changes yet.

## [0.9.2] - 2026-11-17

Patch for the six product findings from the independent v0.9.1 certification run.

### Fixed

- `ui tap --ref` now resolves `actionable_via: "ancestor"` context nodes correctly when the
  observation was captured with `--full`. The full-hierarchy copy carries the same readiness
  metadata, and ref lookup prefers the compact context node.
- Commands that load a session now refuse a session bound to a different project with
  `E_SESSION_PROJECT_MISMATCH`. `context` ignores the foreign session and returns an explicit
  warning instead of silently combining projects.
- `hook run` now returns `verified` and `verification_state`. A hook with no `verify` block is
  reported as `unverified` with a note that shell exit 0 is not route proof. The shipped IronPulse
  hooks verify `CURRENT STREAK` and `Appearance`.
- `diagnostics logcat start` follows from the current logcat tail (`-T 1`) and treats a bounded
  capture that reaches its byte cap as `completed` (`completed_reason: "max_bytes"`) instead of
  failing with "logcat capture process exited immediately". `stop` finalizes the completed capture.
- Action receipts with a satisfied postcondition and an empty semantic diff now add explicit
  evidence that the expected state may already have been visible before the action.
  CalcSuite's live preview makes `=` correctly `no_visible_change`.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.9.2`.
- Error taxonomy includes `E_SESSION_PROJECT_MISMATCH` (configuration).
- `hook run` result payload includes `verified` and `verification_state`; existing fields remain.

## [0.9.1] - 2026-11-17

Stability patch for the issues found by the CalcSuite, DiscountHunter and IronPulse v0.9.0
validations.

### Fixed

- Observation retention now orders by recorded `observed_at` (then mtime) instead of random token
  filenames, prunes before writing, and never evicts the observation just written. A store that
  cannot evict fails explicitly instead of returning an unusable token.
- Refs no longer include the observation token. Unchanged nodes keep the same ref across repeated
  observations; changed identity still produces a different ref.
- `ui tap`/`ui long-press --ref` accept context text inside a clickable ancestor and dispatch on that
  ancestor. Observations expose `actionable_via: "ancestor"` for those nodes.
- `--clickable` matches a clickable ancestor again instead of requiring the node itself to be
  clickable.
- `E_SELECTOR_NOT_FOUND` prioritizes the requested label and reports `visible_labels_total` and
  `visible_labels_truncated`.
- `report explain` classifies `E_ELEMENT_REF_*`, `E_ELEMENT_IDENTITY_CHANGED`,
  `E_ELEMENT_NOT_ACTIONABLE`, `E_ELEMENT_DEVICE_MISMATCH` as `element_reference` and
  `E_OBSERVATION_*` as `observation`, with targeted next actions. Observation-store failures are not
  auto-resolved by an unrelated passing run.
- `report explain` and `report bundle` use one project-scoped selector: `latest` is the newest
  completed run and `latest-failure` is the newest failed/partial run. Runs from other projects are
  never selected.
- Brief `agent test` keeps compact stage entries and `next_actions`.
- The bundle `manifest.json` records `checksums_verified:true` after successful verification.
- `diagnostics performance` returns `run_id`, `run_dir` and artifact paths; the brief summary includes
  the jank frame sample.
- Hook verification supports `ui_text`, `ui_activity` and `ui_package` postconditions. The IronPulse
  example hooks use them.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.9.1`.
- `schema/observation.schema.json`, `schema/diagnosis.schema.json`, `schema/hook.schema.json` and
  `schema/performance.schema.json` document the new fields.
- `report explain latest` on a passing run adds a `latest-failure` next action.
- Docs and examples use `latest-failure` when the intent is the newest failure.

## [0.9.0] - 2026-11-17

Agent Mode. Adds compact/brief output profiles, a start-of-turn brief, shorter handles and command
aliases without changing full JSON output.

### Added

- `--output-profile full|compact|brief`, `--compact`, `--brief`, `ANDROIDFORGE_PROFILE`,
  `[agent] profile`, `--max-items` and `--max-text-chars`. Full remains the default.
  Compact/brief output is minified, caps lists with explicit totals/truncation metadata, removes
  optional repetition, and adds envelope `profile`, `summary` and `next` fields.
- `agent brief` returns one small project/session/device/app/screen/last-run snapshot with the current
  observation token and short next commands.
- Compact observations, diffs and action receipts: node payloads are capped, optional duplicate node
  fields are removed, and receipts keep effect/completeness/identity while dropping redundant target
  and action payloads.
- Brief projections for `context`, `agent test`, `report explain`, `diagnostics performance`, APK
  install/diff results and errors. Comparable summaries preserve package, status, failure stage,
  metrics, split counts and evidence pointers.
- Short handles: element refs and observation tokens use 12 hex characters, with collision detection
  that extends refs deterministically. Existing longer observation tokens remain readable.
- Command aliases: `ui o/d/t/w`, `app i/l`, `diagnostics perf`, `report why`, `agent b`, `context ctx`.
  Selector aliases: `--id` (`--resource-id`), `--desc` (`--content-desc`) and `--token`
  (`--expect-token`).
- `schema/agent-brief.schema.json`; envelope `profile`/`summary`/`next`; adapter `[agent]` section.

### Changed

- `ui observe`, `ui diff`, action receipts, `agent test`, `context`, `report explain` and performance
  carry compact/brief projections when requested. Full output is unchanged.
- `schema list|show` includes `agent-brief.schema.json`.
- Workspace, CLI, desktop, installer and compatibility version are `0.9.0`.

## [0.8.0] - 2026-11-17

Build & Signal. Adds honest split-artifact identity, stable refs for the remaining semantic actions,
and explicit startup/jank/memory/CPU evidence.

### Added

- `app install` accepts explicit `--split` APKs, split directories containing `splits/` or
  `standalones/`, and `.apks` archives. Split sets use `adb install-multiple`; archives are extracted
  as provided without bundletool or automatic device-specific split selection. Package and version
  identity are checked before installation. Standalone/universal entries are not mixed into split sets,
  and ambiguous multi-standalone archives are rejected.
- `apk diff --split` and split-aware installed identity. Split sets are compared by sorted SHA-256;
  `app install --only-if-changed` skips only when the base APK and complete split set match. Unknown
  split identity never skips installation.
- `apk inspect` and `apk resolve` report artifact kind, split count, split hashes and `.apks` archive
  SHA-256. `apk resolve` warns that split paths are temporary extraction paths.
- `agent test` now proves installed split hashes before claiming the build is current; an unproven or
  changed split set requires explicit `--install` instead of being silently skipped.
- Stable refs for `ui text`, `ui clear`, `ui scroll`, `ui wait` and `ui assert`:
  `--ref <ref> --expect-token <observation>`. Text/clear require an editable field, scroll requires a
  scrollable node, and wait/assert may reference context text. Identity is re-checked before dispatch;
  stale/changed refs fail closed with no text or coordinate fallback.
- `diagnostics performance [package] [--activity ...] [--cold] [--settle ...] [--timeout ...]` with
  startup timing from `am start -W`, gfxinfo frame/jank statistics, meminfo PSS/RSS, and CPU from
  `dumpsys cpuinfo` or `/proc/<pid>/stat` sampling. Cold start requires the explicit `--cold` flag.
  Missing metrics carry unknown reasons; a `performance/report.json` artifact is written.
- Desktop Inspector performance panel with cold-start opt-in and startup/jank/memory/CPU summaries.
- `schema/performance.schema.json`; split fields in `apk-resolution.schema.json` and
  `apk-diff.schema.json`.

### Changed

- `app install` records the supplied artifact path (APK, directory or `.apks`) as the build source
  instead of a temporary extraction path.
- Workspace, CLI, desktop, installer and compatibility version are `0.8.0`.

## [0.7.1] - 2026-11-17

Patch to the v0.7.0 Matrix & Device Control line.

### Added

- Desktop Logcat now requests bounded read-only retries and surfaces the CLI retry report in its notes
  (attempt count and transient-failure indication), completing the v0.7.0 desktop retry presentation.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.7.1`.

## [0.7.0] - 2026-11-17

Seventh alpha, extending AndroidForge from reliable single-device orchestration to controlled
multi-device validation, stronger device interaction and safer transient recovery.

### Added

- `agent matrix` for sequential execution across multiple independent AVDs or serials, with a dry-run
  plan, per-target sessions and artifacts, duplicate/ownership checks, health preflight, fail-fast,
  cleanup reporting and aggregate `passed`/`failed`/`partial` status. Parallel mode is explicitly
  rejected until independently validated.
- Stable semantic element references: deterministic `ref` values on observed actionable/context nodes,
  `ui tap --ref ... --expect-token ...`, `ui long-press` reference support, identity re-validation
  immediately before dispatch and explicit stale/unknown/wrong-device/not-actionable errors. No
  text/coordinate fallback.
- Wireless ADB: `devices pair` (stdin-safe pairing code), `devices connect`, `devices disconnect` and
  endpoint validation. Pairing codes are never stored, logged or returned.
- `devices health`: read-only battery, thermal, storage, memory, network, screen, uptime, Android/API
  and ADB state, with explicit unknown reasons, raw evidence and configurable warning thresholds.
- Bounded read-only retries: `--retries 0..5` on `ui observe`, fresh `ui diff`, logcat `peek/tail`, with
  transient-error classification, exponential backoff, a 20-second total window and per-attempt
  reporting. State-changing operations never retry.
- Desktop Matrix page, per-device health and wireless controls, reference-copy actions in Context,
  action receipt details in Triage, and stdin-backed pairing-code handling.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.7.0`.
- `observation` nodes now include `ref`/`role`; old consumers that ignore unknown fields remain compatible.
- Matrix execution calls the existing `agent test` pipeline as a child process, preserving one
  backend/JSON contract.

### Safety

- Matrix never steals an occupied device and never releases a pre-existing session; it releases only
  sessions created by that matrix with the matrix owner. Emulators are not stopped.
- Element references fail closed on a changed path, identity, visibility or enabled state.
- Wireless pairing never enables networking or silently falls back to insecure `adb tcpip`.
- Retries are opt-in, bounded and restricted to a central allow-list of read-only operations.

## [0.6.0] - 2026-11-17

Sixth alpha, focused on returning more trustworthy evidence per agent command and remembering failures
across local runs.

### Added

- Action receipts for `ui tap`, `ui long-press`, `ui text` and `ui scroll` via `--receipt` and
  `--receipt-full`: before/after observation tokens, compact semantic diff, focus/package/activity
  transitions, deterministic effect classification, postcondition result and explicit completeness.
- Failure recurrence memory derived from existing run reports: `report recurrences`, `report history
  --fingerprint`, recurrence data in `report explain`, and `new`/`recurring`/`regressed`/`unknown`
  classification with conservative scenario comparability.
- Optional `[build]` variant configuration, conventional Gradle output fallback, `apk resolve`,
  `agent test --variant`, `apk diff installed:<package>` and `app install --only-if-changed`.
- `report bundle` for redacted, size-bounded, checksummed diagnostic ZIPs plus Markdown/JSON summaries,
  dry-run inclusion previews, evidence/logcat/hierarchy excerpts and explicit media/full-log opt-ins.
- Desktop Context and Ownership pages, recurrence history and related runs in Triage, and a diagnostic
  bundle inclusion preview/creation workflow.
- Schema files for action receipts, recurrence, APK resolution/diff and diagnostic bundles.
- Run reports now record the exact CLI invocation argv when created.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.6.0`.
- `report recurrences` is project-scoped by default; `--all-projects` aggregates every discovered local
  artifact store.
- Installed APK comparison reports `same`, `different` or `unknown` per field instead of inferring
  equality from version strings.

### Safety

- Receipts never retry a state-changing action when a postcondition is absent or times out.
- `--only-if-changed` never uninstalls, clears data or downgrades; unverifiable identity installs and
  records the reason.
- Bundles never modify the source run, never include keystores/credentials, reject traversal, and
  require opt-in for screenshots, recordings and full raw logs.

## [0.5.0] - 2026-11-16

Fifth alpha. Fixes every issue confirmed by the three independent app verification agents and adds
regression coverage for each corrected behavior.

### Fixed

- Failed `agent test` pipelines now emit envelope `status: "failed"` instead of `status: "success"` with
  a nonzero exit code. Blocked dry runs emit `status: "blocked"`.
- `session ensure` no longer chooses a console port already associated with a connected ADB emulator.
  Added `--port` to `session ensure` and the agent pipeline, with explicit conflict errors.
- `session start`, `session attach` and `session ensure` now bind the project root on creation, so the
  first `ensure` no longer emits a misleading legacy-project warning.
- A stuck UIAutomator helper is detected, killed and retried automatically. Persistent failure returns
  `E_UI_AUTOMATOR_UNAVAILABLE` with attempts, last error and recovery context.
- `report explain` classifies UIAutomator infrastructure failures as `ui_automation` rather than
  `unknown`, including legacy `E_COMMAND_FAILED` reports containing exit 137.
- `report explain latest` selects the newest failed/partial/cancelled/interrupted run when one exists,
  instead of explaining a newer passed run.
- `report explain <RUN>` and `report show/path` search the current directory and ancestor artifact
  stores, so runs created without `--project` remain discoverable from a project-scoped invocation.
- `report list --all` discovers runs across project and ancestor artifact stores.
- `ui observe` now exposes `token` alongside the backward-compatible `id`, separates interactive
  `actionable` nodes from non-interactive `context` text, always includes `clickable` and `enabled`,
  filters empty resource ids, and adds an `editable` flag.
- `ui diff` returns compact node summaries with a default 12-node cap per category and totals/truncation
  flags. `--full` raises the cap to 100.
- Selector errors now suggest `ui scroll-to` for below-the-fold targets and `--clickable` when candidate
  matches differ in interactivity.
- `doctor` counts active sessions and stale active sessions, reporting tombstones separately.
- `session release` ownership errors now name the actual subcommands and mention `ANDROIDFORGE_OWNER`.
- The desktop executable answers `--help` and `--version` non-interactively instead of launching the GUI.
- IronPulse's conformance flow dismisses the debug update dialog. DiscountHunter's Settings tap uses
  `--clickable` to avoid the header/bottom-nav ambiguity.

### Added

- Regression tests for failed/blocked agent envelope status, UIAutomator classification, project-scoped
  run discovery, observation actionable/context separation, compact diff caps, session owner parsing,
  and report scoping.
- `ui diff --full`.
- `session ensure --port`.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.5.0`.
- Observation compact default caps actionable nodes at 24 and context nodes at 16; context includes at
  most 12 and 8 respectively.
- `ui diff` defaults to compact nodes; use `--full` for larger lists.

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

[Unreleased]: https://github.com/JuggyBob/AndroidForge/compare/v0.9.2...HEAD
[0.9.2]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.9.2
[0.9.1]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.9.1
[0.9.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.9.0
[0.8.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.8.0
[0.7.1]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.7.1
[0.7.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.7.0
[0.6.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.6.0
[0.5.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.5.0
[0.4.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.4.0
[0.3.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.3.0
[0.2.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.2.0
[0.1.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.1.0


