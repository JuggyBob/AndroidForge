# Capabilities

`androidforge capabilities --json` is the machine-readable authority for this build. It reports implementation/toolchain state, not a promise that every path was exercised on the current host. The `runtime_verified` field stays `false` unless a delivery-specific verification process sets it; see [VERIFICATION.md](VERIFICATION.md) for what was actually run.

## Status vocabulary

| Status | Meaning |
| --- | --- |
| `supported` | Implemented and normally works when the required tools/device are present. |
| `best-effort` | Implemented, but the Android/vendor output is variable. Raw evidence is preserved; do not over-trust parsed fields. |
| `unsupported` | Deliberately absent. Expect `E_UNSUPPORTED_OPERATION`, a reason and alternatives. |

## Reported capabilities

| Capability | Status | Requires | Notes |
| --- | --- | --- | --- |
| `devices.list` | supported | adb | Parses `adb devices -l`, enriches operational devices. |
| `emulator.lifecycle` | supported | adb, emulator | Legacy emulator executable; headless start, boot wait, stop, snapshots. |
| `session.ownership` | supported | filesystem locks | Coordinates AndroidForge sessions only; external ADB users are not prevented. |
| `session.ensure` | supported | filesystem locks | Idempotent reuse/attach/launch with advisory lease metadata and bounded `--wait`; never steals another owner's device. |
| `session.hygiene` | supported | filesystem locks | `session list --mine/--active` with classification/reason/age; `gc --reclaim --dry-run` lists would-reclaim ids and the exact apply command. |
| `context` | supported | local state, optional adb | Compact project/session/device/app/screen/run snapshot with partial results and argv next actions. |
| `ui.observation` | supported | adb, uiautomator | Compact semantic observations, session/boot-scoped tokens, semantic diffs and `--full` hierarchy. `--semantics` adds an additive bounded (64) full-hierarchy view with Compose `test_tag` values and derived roles; without the flag no semantics fields appear. |
| `ui.geometry` | supported | adb, uiautomator | Read-only `ui bounds` and `ui assert-bounds` reuse semantic selectors and return all eight integer bounds; `ui wait --for-bounds-stable` re-resolves a selector across polls. |
| `ui.a11y` | supported | adb, uiautomator | `ui audit a11y` is a read-only hierarchy audit (names, touch targets, duplicate labels); `--contrast` adds one host-only screenshot sample and WCAG findings. |
| `ui.postconditions` | supported | adb, uiautomator | One action plus observation polling; no hidden action retries. |
| `ui.gestures` | supported | adb | Configurable long-press holds, press-move-release drags with holds and optional mid-gesture screenshots. |
| `ui.wait_modes` | supported | adb, uiautomator | `--for-text-gone`, `--for-idle`, `--for-stable` and `ui assert-no-text` with bounded attempts and lingering matches. |
| `ui.compose_test_tags` | best-effort | adb, uiautomator, debug app exposing testTagsAsResourceId | `--test-tag` aliases the accessibility resource id; tags are invisible unless the app enables `testTagsAsResourceId`. |
| `ui.receipts` | supported | adb, uiautomator | Before/after tokens, semantic diff, deterministic effect classification and explicit completeness. |
| `ui.element_refs` | supported | adb, uiautomator | Opaque refs scoped to one observation token, with strict identity validation and no fallback for tap, long-press, text, clear, scroll, wait and assert. |
| `ui.overlay_dismissal` | best-effort | adb, uiautomator | `ui dismiss-overlays` handles only clickable SystemUI nodes labelled Got it, OK, Dismiss or Close; it never taps app content or permission decisions. |
| `diagnosis.explain` | supported | local reports | Deterministic categories, confidence, evidence, stable fingerprints and exact argv next actions. No LLM. |
| `diagnosis.recurrence` | supported | local reports | Project-scoped new/recurring/regressed/unknown memory derived from existing run reports; no second database. |
| `reports.tagging` | supported | local reports | `run start --tag/--meta` adds additive tags and string metadata; `report list|explain|bundle` and compare baseline selection accept repeatable `--tag` (AND) plus `--since`. |
| `reports.budgets` | supported | local reports | `report compare --budget/--max-*` adds budget checks/violations and `budget_passed`; `--fail-on-regression` maps a violation to `E_PERFORMANCE_REGRESSION` (exit 11). Absent budgets preserve the 1.0.0 payload. |
| `maintenance.retention` | supported | local filesystem and reports | `maintenance clean` is host-only and dry-run by default; repeatable `--keep-tag` protects tagged runs and forces a dry run without `--yes`. The additive `retention` object records deletions, kept runs and reasons. |
| `diagnostics.bundle` | supported | local reports | Redacted, size-bounded, checksummed ZIP plus Markdown/JSON evidence summaries and dry-run preview. `--metadata-only` excludes every run artifact; `--redact-secrets` refuses a bundle with any remaining suspected secret. |
| `artifacts.secret_scan` | supported | local filesystem | `artifacts scan-secrets` is a bounded, host-only read of `.artifacts` text with category-derived previews; matched values are never stored or printed and binary media is disclosed as unscanned. |
| `devices.health` | supported | adb | Read-only battery/thermal/storage/memory/network/screen/uptime report with unknown reasons. |
| `devices.shell` | supported | adb | `devices shell --read-only` is an allowlisted, bounded, redacted diagnostic path with no physical grant; without the flag it requires the master `--allow-physical-mutation` only. |
| `devices.input` | supported | adb | `devices input tap X Y` uses `UiManager::tap_xy` under the existing `Risk::Input` consent gate and reports display size; `--dry-run` prints coordinates only. |
| `devices.wireless` | supported | adb | Standard pair/connect/disconnect wrappers; pairing codes are never logged or returned. |
| `devices.scenario` | supported | adb, session | `devices scenario --night|--locale|--font|--network` uses the existing prepare/network primitives, records restore state in the session, honors the data/network risk gates and has a pure dry-run. |
| `devices.tags` | supported | local state | Host-only safe-tag manifest (`devices tag add/remove/list/clear`) and `agent matrix --device tag:<name>` with deterministic, deduplicated serial expansion. No device access. |
| `avd.images` | supported | SDK layout | `avd images` lists installed SDK system images from the directory layout; no `sdkmanager` invocation. |
| `avd.create` | supported | avdmanager | `avd create` requires `--yes`, validates names/images/devices, never uses `--force` to overwrite, and `--dry-run` prints exact argv without executing. |
| `host.capacity` | supported | Windows API, filesystem | `doctor --capacity` reports RAM/commit/disk readings with sources and unknown reasons; `emulator status` adds ports, owner/session and a conservative capacity warning. Unknown values are never fabricated as zero. |
| `ports.list` | supported | local sockets | `ports list` maps console/ADB/gRPC ports with free/allocated/occupied state and active-session owner metadata; no device required. |
| `session.terminal_cleanup` | supported | filesystem locks | `session gc --forget-terminal` releases locks of terminal tombstones, keeps their history, is dry-run by default and never steals a live lock. Owner match or `--force` required. |
| `retry.read_only` | supported | none | Opt-in bounded retries (max 5, 20s window) for observe/diff/logcat reads only. |
| `repair.plan` | supported | local toolchain | Strictly read-only environment repair planning; does not install or edit anything. |
| `agent.pipeline` | supported | adb, project adapter | Transparent ensure/inspect/install/fixture/conformance/explain stages with dry-run. |
| `agent.brief` | supported | local state, optional adb | One small project/session/device/app/screen snapshot with next commands. |
| `agent.output_profiles` | supported | none | Full, compact and brief JSON with minification, caps, counts, summaries and next commands. |
| `agent.structured_output_flags` | supported | none | CLI-only `--jsonl` (NDJSON, with `scenario run` iteration lines), `--fields` (trim `result` keys) and `--stats` (exact stdout bytes/lines plus `ceil(bytes/4)` tokens on stderr). Envelope stays schema 1; presentation flags are never MCP arguments. |
| `agent.matrix` | supported | adb, emulator, project adapter | One independent child matrix per target, health preflight, fail-fast and partial aggregates. `--parallel 1-32`; `--fail-fast` is best-effort because running targets finish. |
| `mcp.server` | supported | none | Local stdio JSON-RPC MCP server (`mcp serve`) that shells out to the same CLI. Counts are measured per batch; destructive tools require `confirm=true`; unsafe `adb`/`run exec` and the nested `mcp serve` are excluded, while the curated mock-server tool is bounded by `max_requests`/`duration` unless a built-in scenario supplies the bounds. |
| `tools.schema` | supported | none | `tools schema --format mcp|openai|json` exports the current MCP tool catalog for function-calling clients. |
| `tools.catalog` | supported | none | `tools catalog --json` exports every CLI leaf command, argument, safety class and example for the desktop tool database and MCP generation. |
| `ui.selector_v2` | supported | adb, uiautomator | `--role`, `--text-regex`, `--region`, `--within`, `--above`, `--below`, `--left-of` and `--right-of` in addition to the v1 selectors. |
| `ui.ocr_fallback` | best-effort | adb, optional local OCR engine | `--ocr-fallback` runs only after UIAutomator finds nothing. Default engine `tesseract`; override with `ANDROIDFORGE_OCR_COMMAND`. Images stay local. |
| `runs.resume` | supported | local reports | `run checkpoint`, `run resume --from-checkpoint`, failure history and append-only `events.ndjson`; `report repro` emits exact argv. |
| `diagnostics.logcat.query` | supported | adb | Bounded structured threadtime records with level/grep/time filters. `logcat watch` emits log/crash/ANR NDJSON. |
| `intents` | supported | adb | Action/deep-link starts, typed extras, categories, flags, broadcasts and service start/stop. |
| `network.profiles` | supported | adb | `default`, `offline`, `wifi-only`, `data-only` and `no-proxy`, global proxy get/set/clear and session-scoped restore. |
| `network.reverse` | supported | adb | Per-session add/list/remove/clear of `adb reverse` mappings; recorded for cleanup. |
| `network.mock_http` | supported | local TCP listener | Bounded local HTTP/1.1 mock server with inline/JSON routes, request logs, redacted headers and body limits. |
| `network.mock_tls` | supported | local TLS certificate/key | Optional local TLS with a user-provided PEM cert/key; Android trust must be configured separately; no public CA claim. |
| `apk.inspect` | supported | aapt2 | Package, version, SDKs, label, launchable activity, architectures, hash. |
| `apk.verify` | supported | aapt2, apksigner, Java | Local signature verification. No signing automation. |
| `apk.resolve` | supported | aapt2, project config | Deterministic newest valid variant output; never runs Gradle; ambiguity is explicit. |
| `apk.diff` | supported | adb, aapt2 | Pulls the installed base APK and reports same/different/unknown including split limitations. |
| `app.install` | supported | adb | Signature-aware diagnostics; `--only-if-changed` requires proven package/version/base-hash/split-hash equivalence; never auto-uninstalls. Physical devices require `--allow-physical-install` or `--allow-physical-mutation`. |
| `app.verify_installed` | supported | adb, aapt2 | Pulls the live installed base APK, hashes it and compares package/version/certificate/splits without installing. |
| `app.data_fixtures` | supported | adb, debuggable app | Export/import/reset/seed app-private data through `run-as`; imports always capture a pre-import archive; physical devices require `--allow-physical-data` or `--allow-physical-mutation`. |
| `app.uninstall` | supported | adb | Requires `--yes`; `--keep-data` keeps app data. Physical devices require `--allow-physical-install` or `--allow-physical-mutation` too. |
| `build.execution` | supported | configured argv, aapt2 | `build` runs the explicit `[build.commands.<variant>]` argv, resolves the APK, and optionally installs/launches through the audited paths. No inferred command, no shell string; global `--dry-run` touches no device. |
| `ui.uiautomator` | supported | adb, uiautomator | Compose semantics only when exposed through accessibility. |
| `capture.screenshot` | supported | adb | Binary PNG via `exec-out`; optional baseline/region pixel comparison and diff artifact. |
| `capture.launch` | supported | adb | Bounded host-only screencap sampling after an explicit launch, with first-frame, splash and colour assertions; no video decoder or device temp file. |
| `capture.baseline` | supported | local PNG decoder | Named screenshot baselines (`capture baseline save|update|compare|list`) with hashes, dimensions and a per project/run manifest. |
| `capture.color` | supported | adb | `capture assert-color` and `capture contrast` sample a fresh host-only screenshot for RGB and WCAG checks. |
| `capture.recording` | best-effort | adb, screenrecord | Android's 180-second limit; no audio. |
| `diagnostics.logcat` | supported | adb | Never clears the global buffer by default; bounded. Repeatable literal-prefix `--redact-patterns` apply to `tail|peek|query|watch|start` before printing or storage; `start` persists them for `stop`. |
| `diagnostics.performance` | best-effort | adb, dumpsys, /proc | Explicit startup/jank/memory/CPU evidence; cold start requires `--cold`; unknown values carry reasons. Startup measurement launches the app, so physical devices require `--allow-physical-app-control` (or `--allow-physical-mutation`); `--wake` is a separate, explicit screen-state opt-in. Screen-off fails closed (exit 4) unless `--wake` is explicit; jank needs `--min-frames 30` by default. |
| `diagnostics.frames` | best-effort | adb, dumpsys gfxinfo | Bounded frame counters/percentiles/histogram with optional baseline deltas; a comparable regression is exit 11, partial or non-comparable evidence exit 10. |
| `diagnostics.jobs` | best-effort | adb, dumpsys jobscheduler | Raw output preserved; job IDs are never hardcoded. |
| `diagnostics.notifications` | best-effort | adb, dumpsys notification | Formatting varies by Android version. |
| `diagnostics.doze.time` | best-effort | adb, device privileges | Time changes usually require root; AndroidForge never roots. |
| `fixtures` | supported | project adapter, debuggable app for state | Explicit project hooks and fixture scripts; never automatic. Declarative `state` (`prefs`/`history`/`installed_version`) is applied only through `fixture apply`/`fixture reset` or `apply_state`/`reset_state` steps, with explicit data-mutation consent and no root access. Physical devices require `--allow-physical-data` or `--allow-physical-mutation`. |
| `scenario` | supported | adb, project adapter for fixtures | `scenario validate` is a read-only manifest parser; `scenario run` dispatches an allowlisted step list (launch, UI, bounds, colour, screenshot, fixture state, install) to the existing audited operations, with per-step run reporting, repeats, pass rate, duration variance and flake summary. No shell/adb/arbitrary step is possible. `--dry-run` needs no device or consent; real mutating runs need `--yes` and the exact scoped physical flag for each risk. Physical capability flags are never exposed or forwarded over MCP. |

| `network.mock` | supported | none (host loopback) | `network mock serve --scenario <name>` serves bounded, redacted, loopback-only built-ins (`newer-release`, `offline`, `bad-digest`, `no-APK`); `network mock scenarios --json` is a read-only catalog. `--reverse` remains a device-network mutation with its own physical gate and is rejected for scenarios. |
| `maestro` | best-effort when installed | optional Maestro CLI | Local only; no Maestro Cloud, accounts or API keys. Emulator pass/fail flows were validated in 0.13.0; physical-device flows remain unverified. Missing Maestro returns `E_MAESTRO_NOT_FOUND`. |
| `maestro.installer` | supported | Windows, Java 17+ | `tools install maestro` downloads the pinned Maestro CLI 2.12.1 ZIP, verifies its SHA-256 and installs under the AndroidForge state directory. `tools status` reports the active path/source. |
| `instrumentation` | best-effort | adb, test APK | Runs installed instrumentation; does not replace Gradle. Requires a test APK and was not end-to-end validated in this release. |
| `snapshots` | best-effort | adb, emulator | Loading always requires `--yes`. AndroidForge-launched emulators default to `-no-snapshot-save`; pass `session start|ensure --snapshot-save` to permit manual snapshots. Save waits for the emulator's asynchronous snapshot list. |
| `physical.safety` | supported | adb | Physical-device changes require an explicit opt-in. Scoped flags cover app control, data, network, install and input; `--allow-physical-mutation` remains the superset. Dry-runs report the requirement, and MCP strips every physical flag. |
| `avd.cloning` | unsupported | none | Not implemented; a renamed profile would not provide isolation. |
| `html.reports` | unsupported | none | JSON and raw artifacts are the primary evidence. |
| `visual.diff` | supported | local PNG decoder | Baseline/full or region screenshot comparison with changed-pixel ratio, bounding box and diff PNG artifact. |
| `cloud` | unsupported | none | No telemetry or upload. |

## Desktop cockpit

`androidforge-desktop.exe` is a separate native executable, so it is not reported by
`androidforge capabilities --json`. Its scope is deliberately narrower than the CLI:

- P0 read-only: project selection, environment/doctor summary, Context, Ownership, devices, sessions,
  report-backed runs, run details/build identity/steps/failure, evidence (screenshots, hierarchy/log
  text), raw `report.json`/`metadata.json`/`manifest.json` and manifest verification.
- P1 controlled actions: session start/attach/release/stop/restart, APK inspect/dry-run/install,
  fixture/conformance/hook launch, streamed output, and cancellation for installs/fixtures/conformance/
  hooks. Session lifecycle operations run to completion. Triage also previews and creates diagnostic
  bundles through the CLI; recurrence is shown with the explanation.
- CLI resolution and schema probing: explicit path, sidecar, workspace build, then `PATH`; unknown report
  schemas are refused.
- Native Rust/egui/eframe/glow only; no WebView, HTML, JavaScript, Node or bundled browser.
- Windows-validated. It is not a project editor, device mirror, HTML report or automation API.

## Command coverage

### Environment and configuration

- `doctor` — host, SDK, ADB, emulator, build tools, Java, apksigner, Maestro, AVDs, virtualization, licenses, artifact writability, sessions, configuration. `doctor --capacity` adds read-only RAM/commit/disk readings with sources and per-AVD estimates; unknown values stay unknown, never zero.
- `capabilities` — support matrix and toolchain.
- `ports list` — console/ADB/gRPC port map with free/allocated/occupied state and active-session owner metadata; no device required.
- `schema list|show` — versioned JSON Schemas.
- `tools schema|catalog|install|status|uninstall` — MCP/function-calling export, complete CLI catalog and optional Maestro installer.
- `init` — create a project adapter or global config; `--dry-run` supported.
- `config show|validate|path` — merged config, validation issues, resolved paths.

### Devices, emulators, sessions

- `devices list|inspect|wait|prepare|restore`; the 1.0.0 `device` alias remains available for all of them.
- `devices shell --read-only` — allowlisted, bounded, redacted shell diagnostics via the typed ADB path;
  no physical-device grant. Without `--read-only` it requires master `--allow-physical-mutation` only
  and is recorded as `device-mutation`. `devices input tap` uses `UiManager::tap_xy` under the existing
  `Risk::Input` consent gate and reports display size. `--dry-run` prints exact argv/coordinates only.
  Both are also reachable as `device shell` / `device input tap` through the retained alias.
- `avd list`; `avd images` is a read-only inventory of installed SDK system images; `avd create` requires `--yes` (or `--dry-run` for exact argv) and never overwrites an existing AVD. `avd clone` is present only to return a structured `E_UNSUPPORTED_OPERATION` with alternatives.
- `devices tag add|remove|list|clear` are host-only safe tags; `agent matrix --device tag:<name>` expands tags to deterministic, deduplicated serials. No device access for tags.
- `emulator start|status|stop`; `emulator status` adds ports, owner/session and a conservative `capacity_warning`.
- `snapshot list|save|load|delete`.
- `devices pair|connect|disconnect|health`.
- `session start|ensure|attach|claim|list|inspect|status|release|stop|use|current|gc|restart`.
  `session list --active --mine [--owner NAME]` hides tombstones and filters by owner; each entry has a
  `classification` (active/busy/stale/released/failed/abandoned), age and reason. A released tombstone
  can still stop its owned emulator if nobody else owns it. `session stop|release|restart` require the
  matching `--owner` (or `ANDROIDFORGE_OWNER`) unless `--force` is used for a deliberate takeover.
  `session gc --reclaim --dry-run` lists `would_reclaim` and the exact apply command without mutating;
  `session gc --forget-terminal` releases terminal-tombstone locks, keeps their history and is dry-run
  by default.
- `session ensure` reuses a compatible owned session, attaches to a warm unowned emulator or launches the
  requested AVD. `--ttl` records an advisory lease; expiry marks `needs_reconciliation` rather than
  stopping a device.
- `context` - project, environment, session, device, app, screen, last run and next actions; supports
  `--include`, `--full`, `--refresh-screen`, `--no-screen` and partial results.

### Apps and APKs

- `build --variant <name> [--module M] [--timeout 20m] [--install|--launch|--cold]` runs the configured argv, resolves the APK and reuses the audited install/launch paths; global `--dry-run` executes nothing and contacts no device.
- `apk inspect|verify|resolve|diff`; split sets via repeated `--split` and `.apks`/split directories.
- `app install|uninstall|clear-data|launch|force-stop|kill|status|info|list|file-push|file-pull|verify-installed|data-backup|data-restore|data export|data import|data reset|data seed`.
- `permissions check|grant|revoke`.
- `appops get|set|reset`.

### UI

- `ui dump|inspect|observe|diff|find|tap|long-press|drag|text|clear|wait|assert|assert-no-text|dismiss-overlays|settle|scroll|scroll-to|swipe|back|home|keyevent|hide-keyboard|focused`.
- `ui observe` returns compact semantics, a session/boot-scoped token and stable `ref` values; `ui diff` compares tokens. `--semantics` adds an additive bounded (64) full-hierarchy view with Compose `test_tag` values and derived roles, with stable refs; absent flag leaves the observation unchanged. `--test-tag` on `ui tap`/`ui assert` maps to the audited resource-id selector.
- `ui bounds` returns all eight integer fields for one semantic selector; `ui assert-bounds` compares `bottom_y` with equality/greater/less relations and tolerance. `ui audit a11y` is a read-only hierarchy audit with effective options, bounded findings and severity/rule counts; `--contrast` samples one host-only screenshot. All are read-only and require no physical consent flag.
- `tap`, `long-press`, `drag`, `text` and `scroll` accept `--wait-for-text`, `--wait-for-absent-text`,
  `--wait-for-value`, `--wait-for-package`, `--wait-for-activity` and `--timeout` postconditions.
- The same actions accept `--receipt` / `--receipt-full` for before/after tokens, semantic diff,
  deterministic effect classification and explicit completeness. Tap also accepts
  `--capture-before`/`--capture-after`; long-press and drag accept `--capture-mid`.
- `ui wait` adds `--for-text-gone`, `--for-idle` and `--for-stable --stable-for`; `ui assert-no-text`
  fails with lingering matches. `--test-tag` aliases the accessibility resource id (app must expose
  Compose `testTagsAsResourceId`).

### Agent loop and repair

- `report explain <run|latest|latest-failure>` with `--full` and `--markdown`; deterministic categories, evidence,
  fingerprints, recurrence and exact argv next actions.
- `report history --fingerprint`, `report recurrences` with project/package/status/device/version/since
  filters, and `report bundle` with ZIP/Markdown/JSON, dry-run preview and explicit media opt-ins.
- `doctor fix --plan|--check`; strictly read-only repair planning.
- `agent test` with `--dry-run`, `--apk`, `--variant`, `--session-id`, `--avd`, `--serial`, `--owner`,
  `--profile`, `--ttl`, `--install`, `--flow` and `--fixture`.
- `agent brief` for the start-of-turn snapshot.
- `agent matrix` with repeated `--avd`/`--serial`, `--flow`, `--fixture`, `--apk`, `--variant`,
  `--install`, `--fail-fast`, `--dry-run`, `--parallel 1-32` and cleanup reporting.
- `ui observe|diff` and logcat peek/tail accept `--retries`; `ui tap|long-press|text|clear|scroll|wait|assert`
  accept `--ref` and `--expect-token`.
- CLI-only presentation flags: `--jsonl` (NDJSON envelopes plus `scenario run` iteration lines),
  `--fields a,b,c` (trim top-level `result` keys) and `--stats` (exact stdout bytes/lines and
  `ceil(bytes/4)` tokens on stderr). Envelope stays schema 1 and these flags are never MCP arguments.

### Evidence and diagnostics

- `capture screenshot [--compare BASELINE --region x1,y1,x2,y2 --max-diff-ratio N]`; `capture record start|stop|status`.
- `capture baseline save|update|compare|list` keeps validated named baselines and a per project/run manifest (hashes and dimensions); `capture contrast` performs a WCAG check and `capture assert-color` checks an RGB sample. Both sample a fresh host-only screenshot.
- `capture launch <PACKAGE> [--fps N --duration Ns --expect-first-frame --no-splash --expected-color #RRGGBB]` launches the app and samples bounded screencap PNGs (`capture-launch/frame-NNNN.png`) to detect the first real frame.
- `diagnostics collect|app|logcat tail|start|stop|status|crashes|inspect|performance|frames`. `diagnostics frames` writes `frames/report.json` with counts, percentiles, histogram, verbatim excerpt and optional baseline deltas (exit 11 on a comparable regression).
- `diagnostics performance` fails closed with `E_SCREEN_OFF` (exit 4) on a sleeping display; wake is only `--wake`. Jank needs `--min-frames` (default 30) and the report carries `sample_quality` (`jank`/`cpu`), never an inferred zero. Because a real run launches the app (and `--cold` force-stops it), its catalog/MCP classification is `device-mutation` (`diagnostics frames` stays read-only); MCP requires `confirm=true` unless `dry_run=true`. Physical CLI use still requires `--allow-physical-app-control` (or the `--allow-physical-mutation` master flag), and MCP never forwards physical capability flags.
- `diagnostics logcat tail|peek|query|watch|start` accept literal-prefix `--redact-patterns`; `start`
  persists them for `stop` so captured text is redacted before it is stored and checksummed.
- `artifacts scan-secrets` — bounded, host-only scan of `.artifacts` text; category-derived previews
  only, binary media disclosed as not scanned.
- `report bundle --redact-secrets` — forces redaction and refuses any bundle with a remaining suspected
  secret (`E_UNSUPPORTED_OPERATION`, exit 3), with counts/kinds only. Binary screenshots/recordings are
  not pixel-redacted.
- `notifications list|wait|assert|channels|tap|clear`.
- `jobs list|run|cancel|wait|assert`.
- `alarms next|list|wait`.
- `doze status|force|unforce`; `standby get|set`; `battery status|unplug|reset`; `network status|airplane|wifi|data|reverse add|list|remove|clear|mock serve|mock scenarios`; `time get|set|restore`.

### Automation and reports

- `hook list|run`; hooks can declare shell/command/UI `verify` postconditions (`ui_text`, `ui_activity`, `ui_package`) and `run_as` broadcasts for debuggable
  non-exported receivers.
- `fixture list|validate|run|apply|reset` (`apply`/`reset` dispatch only declarative-state actions).
- `conformance run`.
- `scenario validate|run` (`run` supports `--repeat`, `--dry-run`, `--yes` and the scoped physical flags).
- `test maestro|instrumentation`.
- `run start|end|status`, `run exec` / `run -- <program>`. `run start --tag/--meta` adds additive labels
  and string metadata to `report.json` and `metadata.json`.
- `report list|explain|bundle` and compare baseline selection accept repeatable `--tag` (AND) and
  `--since`; `report compare --budget/--max-*` adds budget checks with `--fail-on-regression` mapped to
  exit 11. `maintenance clean --keep-tag` protects tagged runs and forces a dry run without `--yes`.
- `report list|show|path|prune|import junit`; JUnit accepts files, multiple paths or a directory tree.
  `report show|path` fall back to project-local `.artifacts/runs` directories, so a conformance run can
  be inspected from the repository root without `--project`.
- Conformance results carry build identity and `flow_checks` parsed from the external flow's final JSON
  `checks` array.
- `adb <args...>` — explicit raw passthrough.

## Not implemented

- A TUI, plugin system or automation API beyond the CLI and the native desktop cockpit.
- A mandatory daemon or background scheduler.
- Cloud device farms, telemetry, crash upload, accounts or LLM calls.
- Release publishing, APK signing, keystore/password handling.
- AVD cloning, named-profile isolation or parallel AVD provisioning.
- HTML reports. Screenshot baselines/regions are supported by `capture screenshot --compare` and by
  named baselines through `capture baseline save|update|compare|list`.
- A replacement for Gradle, JUnit, Robolectric, Compose UI tests or Espresso.
- iOS or non-Android devices.

## Verified versus unverified

The current **1.1.0** build passed the full gate: fmt and strict clippy clean, `cargo deny` passed,
and `cargo test --workspace --locked` **784 passed / 0 failed across 29 suites**. After the final two
safety fixes (physical `diagnostics performance` requires `Risk::AppControl`; the command is
catalog/MCP `device-mutation` with MCP `confirm=true` unless `dry_run=true`), the post-fix CLI gate
reported **375 passed / 0 failed**. The catalog exposes **219 commands**, MCP exports **221 tools**,
and **38/38 schemas** validate. The curated `androidforge_ui_observe` tool gained only the optional
`semantics` field; `--jsonl`, `--fields` and `--stats` remain CLI presentation flags and never appear
in any MCP tool schema or argv. The envelope and report schemas stay at `1`.

The supported 1.1.0 scope is complete. On the approved POCO X4 GT / Android 14 (API 34), a delegated
read-only campaign passed 12/12 steps with no mutations and no NOT RUN, and a delegated approved
mutation campaign covered app launch, input tap and Back; both sessions were released. Packaged
assets were verified by SHA-256 (CLI ZIP `92bf048f0d0e022fbb947ba351314c5ff98ade4d45cc9d1a4b177133f8225a0b`,
desktop ZIP `7a8d2325aa416f701fbf27fc42cb3f926889a63359153a6c7174acf6a4157c90`, installer
`71dc973b084349d5c5676e96d3ca62971d566a166459bf3f627d287cc3b9ca5e`, CLI EXE
`6e49c9f995c35a2b5968fc2b5615677179b532663d590166b54742c5de852aa4`, desktop EXE
`977cf28495aacb14f381af747c96add619ad9ae8147f707365e4ab4f227d24ad`), installer `FileVersion` /
`ProductVersion` 1.1.0, ZIP contents and the Scoop hash; `dist/SHA256SUMS.txt` matches all five files.

**NOT RUN for 1.1.0:** physical `diagnostics performance` and the night-mode device scenario (not
approved), silent install/uninstall (installed Desktop release directory untouched), standalone
`run repeat --count`, cross-device geometry/colour comparison, a v1.1.0 screenshot gallery,
interactive MCP checks, desktop binary `--version` verification, other vendor ROMs, macOS/Linux hosts
and code signing. These are not claimed as verified.

Historical 1.0.0 record: the full host gate was fmt and strict clippy clean,
`cargo test --workspace --locked` **392 passed / 0 failed** across 27 suites (including 4 new
unreadable-lock tests), clean `cargo deny` and release build. CLI and desktop both reported `1.0.0`,
with 191 catalog commands, 192 MCP tools (also confirmed by MCP `initialize`/`tools/list`) and
31/31 schemas. For 1.0.0, the final package bundled documentation matching the workspace and was verified by checksum, installer
version metadata, bundled docs/notices/fonts, silent install/uninstall and installed EXE hashes. On the
user-approved POCO X4 GT / HyperOS 2 / Android 14 (API 34) device, the packaged CLI performed an
in-place IronPulse 5.1.0 → 5.1.2 upgrade with `content_different`, the same debug certificate and
preserved app data, plus passing live verification, foreground launch, UI observation and evidence
capture. On REDMAGIC 10 Pro / Android 16 (API 36) / RedMagicOS 11.0.6_EU, the packaged CLI passed 8/8
zero-touch read-only checks with no session, no device mutation and byte-identical before/after system
and package inventories; screenshot SHA-256
`cfa4b6d356132645115fd322ea72e3e5b3fe39ef9fb6ed2511e9c82186568148`. The two-ROM scope is complete
for that coverage, but REDMAGIC UI automation and all device actions are **NOT RUN** by the explicit
zero-touch rule. Final asset hashes are recorded in the v1.0.0 release notes.

The `0.12.1` patch updated `url` to 2.5.9 for RUSTSEC-2026-0592 and replaced unmaintained `rustls-pemfile` with `rustls-pki-types` PEM parsing; the full 347-test suite, strict clippy/fmt, `cargo deny` and mock TLS smoke passed. The `0.12.0` stable pre-1.0 release exercised live install truth after an out-of-band `adb install -r`, byte-truthful `--only-if-changed` with a same-version/different-byte signed rebuild, `app verify-installed` and live version/hash expectations, configurable long-press and drag with mid-gesture capture, text-absence/idle/stability waits, `assert-no-text`, app-data export/seed/import/reset with pre-import backup and remote hash verification, `adb reverse`, bounded mock HTTP and self-signed TLS serving with redacted request logs, screenshot baseline comparison, tap before/after captures and session-hygiene classification/reclaim on a disposable API 36 emulator. The workspace test suite passed 347/347 and strict clippy/fmt were clean. No physical device was mutated; Maestro, real instrumentation, snapshot restore, root-only time changes, macOS/Linux hosts and code signing remained unverified in 0.12. The 0.13.0 line subsequently validated Maestro pass/fail flows on a disposable emulator; instrumentation and physical-device Maestro remain unverified.

The `0.1.0` delivery run exercised the CalcSuite, DiscountHunter and IronPulse smoke flows on a real Android 16 emulator, including install/hash/certificate identity, semantic UI work, fixtures, hooks, screenshots, recording, logcat, diagnostics, one report and clean shutdown. The `0.2.0` delivery added the native desktop and exercised it against CalcSuite. The `0.3.0` delivery exercised the redesigned desktop against a real emulator: onboarding screens, environment diagnostics, session restart, APK inspect/install, conformance run, cancellation and reconciliation, live logcat, device preview and semantic hierarchy actions, run comparison and maintenance preview/cleanup. The `0.4.0` delivery introduced and exercised the agent loop: compact context, semantic observations and diffs, postconditions, session ensure, read-only repair planning, deterministic explanations and the agent pipeline. The `0.5.0` delivery fixed the issues reported by three independent app agents: shared-host port selection, UIAutomator recovery, pipeline status, project-scoped run discovery, observation shape, compact diffs, selector guidance, desktop help/version and session project binding. The `0.6.0` delivery added and exercised action receipts on a real emulator, real installed-APK diff/only-if-changed, recurrence classification, redacted bundle creation/verification and the Context/Ownership desktop pages. The `0.7.0` delivery added and exercised a two-AVD matrix (API 36 and 34), stable refs with stale rejection, device health, disconnect/recovery and read-only retry reporting, plus the Matrix and device health desktop workflows. The `0.8.0` delivery added and exercised split/`.apks` install and hash equivalence, refs for text/clear/scroll/wait/assert with stale refusal, explicit startup/jank/memory/CPU evidence and the desktop performance panel. The `0.9.0` delivery added and exercised full/compact/brief output profiles, `agent brief`, shorter refs/tokens, command aliases and measured byte reductions without changing the full JSON contract. The `0.9.1` delivery fixed the defects found by three independent app validations: deterministic observation retention, identity-stable refs, context refs through clickable ancestors, `--clickable` ancestor matching, selector label truncation, reference/observation diagnosis, project-scoped `latest`/`latest-failure`, verified bundle manifests, performance run identity and hook UI postconditions. The `0.9.2` delivery closed the independent certification findings: full-observation context refs, cross-project session refusal, honest hook verification state, bounded logcat byte-cap completion and clearer receipt evidence. Physical-device, Maestro, instrumentation-package, snapshot-restore, root-only time paths and non-125% DPI layouts were not exercised. See [VERIFICATION.md](VERIFICATION.md) for the exact record.
