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
| `context` | supported | local state, optional adb | Compact project/session/device/app/screen/run snapshot with partial results and argv next actions. |
| `ui.observation` | supported | adb, uiautomator | Compact semantic observations, session/boot-scoped tokens, semantic diffs and `--full` hierarchy. |
| `ui.postconditions` | supported | adb, uiautomator | One action plus observation polling; no hidden action retries. |
| `ui.receipts` | supported | adb, uiautomator | Before/after tokens, semantic diff, deterministic effect classification and explicit completeness. |
| `ui.element_refs` | supported | adb, uiautomator | Opaque refs scoped to one observation token, with strict identity validation and no fallback for tap, long-press, text, clear, scroll, wait and assert. |
| `ui.overlay_dismissal` | best-effort | adb, uiautomator | `ui dismiss-overlays` handles only clickable SystemUI nodes labelled Got it, OK, Dismiss or Close; it never taps app content or permission decisions. |
| `diagnosis.explain` | supported | local reports | Deterministic categories, confidence, evidence, stable fingerprints and exact argv next actions. No LLM. |
| `diagnosis.recurrence` | supported | local reports | Project-scoped new/recurring/regressed/unknown memory derived from existing run reports; no second database. |
| `diagnostics.bundle` | supported | local reports | Redacted, size-bounded, checksummed ZIP plus Markdown/JSON evidence summaries and dry-run preview. `--metadata-only` excludes every run artifact. |
| `devices.health` | supported | adb | Read-only battery/thermal/storage/memory/network/screen/uptime report with unknown reasons. |
| `devices.wireless` | supported | adb | Standard pair/connect/disconnect wrappers; pairing codes are never logged or returned. |
| `retry.read_only` | supported | none | Opt-in bounded retries (max 5, 20s window) for observe/diff/logcat reads only. |
| `repair.plan` | supported | local toolchain | Strictly read-only environment repair planning; does not install or edit anything. |
| `agent.pipeline` | supported | adb, project adapter | Transparent ensure/inspect/install/fixture/conformance/explain stages with dry-run. |
| `agent.brief` | supported | local state, optional adb | One small project/session/device/app/screen snapshot with next commands. |
| `agent.output_profiles` | supported | none | Full, compact and brief JSON with minification, caps, counts, summaries and next commands. |
| `agent.matrix` | supported | adb, emulator, project adapter | One independent child matrix per target, health preflight, fail-fast and partial aggregates. `--parallel 1-32`; `--fail-fast` is best-effort because running targets finish. |
| `mcp.server` | supported | none | Local stdio JSON-RPC MCP server (`mcp serve`) that shells out to the same CLI. Destructive tools require `confirm=true`. |
| `tools.schema` | supported | none | `tools schema --format mcp|openai|json` exports the tool catalog for MCP and function-calling clients. |
| `ui.selector_v2` | supported | adb, uiautomator | `--role`, `--text-regex`, `--region`, `--within`, `--above`, `--below`, `--left-of` and `--right-of` in addition to the v1 selectors. |
| `ui.ocr_fallback` | best-effort | adb, optional local OCR engine | `--ocr-fallback` runs only after UIAutomator finds nothing. Default engine `tesseract`; override with `ANDROIDFORGE_OCR_COMMAND`. Images stay local. |
| `runs.resume` | supported | local reports | `run checkpoint`, `run resume --from-checkpoint`, failure history and append-only `events.ndjson`; `report repro` emits exact argv. |
| `diagnostics.logcat.query` | supported | adb | Bounded structured threadtime records with level/grep/time filters. `logcat watch` emits log/crash/ANR NDJSON. |
| `intents` | supported | adb | Action/deep-link starts, typed extras, categories, flags, broadcasts and service start/stop. |
| `network.profiles` | supported | adb | `default`, `offline`, `wifi-only`, `data-only` and `no-proxy`, global proxy get/set/clear and session-scoped restore. |
| `apk.inspect` | supported | aapt2 | Package, version, SDKs, label, launchable activity, architectures, hash. |
| `apk.verify` | supported | aapt2, apksigner, Java | Local signature verification. No signing automation. |
| `apk.resolve` | supported | aapt2, project config | Deterministic newest valid variant output; never runs Gradle; ambiguity is explicit. |
| `apk.diff` | supported | adb, aapt2 | Pulls the installed base APK and reports same/different/unknown including split limitations. |
| `app.install` | supported | adb | Signature-aware diagnostics; `--only-if-changed` requires proven equivalence; never auto-uninstalls. Physical devices require `--allow-physical-mutation`. |
| `app.uninstall` | supported | adb | Requires `--yes`; `--keep-data` keeps app data. Physical devices require `--allow-physical-mutation` too. |
| `ui.uiautomator` | supported | adb, uiautomator | Compose semantics only when exposed through accessibility. |
| `capture.screenshot` | supported | adb | Binary PNG via `exec-out`; no console redirection. |
| `capture.recording` | best-effort | adb, screenrecord | Android's 180-second limit; no audio. |
| `diagnostics.logcat` | supported | adb | Never clears the global buffer by default; bounded. |
| `diagnostics.performance` | best-effort | adb, dumpsys, /proc | Explicit startup/jank/memory/CPU evidence; cold start requires `--cold`; unknown values carry reasons. |
| `diagnostics.jobs` | best-effort | adb, dumpsys jobscheduler | Raw output preserved; job IDs are never hardcoded. |
| `diagnostics.notifications` | best-effort | adb, dumpsys notification | Formatting varies by Android version. |
| `diagnostics.doze.time` | best-effort | adb, device privileges | Time changes usually require root; AndroidForge never roots. |
| `fixtures` | supported | project adapter | Explicit project hooks and fixture scripts; never automatic. |
| `maestro` | best-effort when installed | optional Maestro CLI | Local only; no Maestro Cloud, accounts or API keys. Not end-to-end validated in this release; missing Maestro returns `E_MAESTRO_NOT_FOUND`. |
| `instrumentation` | best-effort | adb, test APK | Runs installed instrumentation; does not replace Gradle. Requires a test APK and was not end-to-end validated in this release. |
| `snapshots` | best-effort | adb, emulator | Loading always requires `--yes`. AndroidForge-launched emulators default to `-no-snapshot-save`; pass `session start|ensure --snapshot-save` to permit manual snapshots. Save waits for the emulator's asynchronous snapshot list. |
| `physical.safety` | supported | adb | Data-changing commands refuse physical devices unless `--allow-physical-mutation`; dry-runs report the requirement. |
| `avd.cloning` | unsupported | none | Not implemented; a renamed profile would not provide isolation. |
| `html.reports` | unsupported | none | JSON and raw artifacts are the primary evidence. |
| `visual.diff` | unsupported | none | Screenshot evidence is supported; automated comparison is deferred. |
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

- `doctor` — host, SDK, ADB, emulator, build tools, Java, apksigner, Maestro, AVDs, virtualization, licenses, artifact writability, sessions, configuration.
- `capabilities` — support matrix and toolchain.
- `schema list|show` — versioned JSON Schemas.
- `init` — create a project adapter or global config; `--dry-run` supported.
- `config show|validate|path` — merged config, validation issues, resolved paths.

### Devices, emulators, sessions

- `devices list|inspect|wait|prepare|restore`.
- `avd list`; `avd clone` is present only to return a structured `E_UNSUPPORTED_OPERATION` with alternatives.
- `emulator start|status|stop`.
- `snapshot list|save|load|delete`.
- `devices pair|connect|disconnect|health`.
- `session start|ensure|attach|claim|list|inspect|status|release|stop|use|current|gc|restart`.
  `session list --active` hides released/failed tombstones; a released tombstone can still stop its
  owned emulator if nobody else owns it. `session stop|release|restart` require the matching `--owner`
  (or `ANDROIDFORGE_OWNER`) unless `--force` is used for a deliberate takeover.
- `session ensure` reuses a compatible owned session, attaches to a warm unowned emulator or launches the
  requested AVD. `--ttl` records an advisory lease; expiry marks `needs_reconciliation` rather than
  stopping a device.
- `context` - project, environment, session, device, app, screen, last run and next actions; supports
  `--include`, `--full`, `--refresh-screen`, `--no-screen` and partial results.

### Apps and APKs

- `apk inspect|verify|resolve|diff`; split sets via repeated `--split` and `.apks`/split directories.
- `app install|uninstall|clear-data|launch|force-stop|kill|status|info|list|file-push|file-pull|data-backup|data-restore`.
- `permissions check|grant|revoke`.
- `appops get|set|reset`.

### UI

- `ui dump|inspect|observe|diff|find|tap|long-press|text|clear|wait|assert|dismiss-overlays|settle|scroll|scroll-to|swipe|back|home|keyevent|hide-keyboard|focused`.
- `ui observe` returns compact semantics, a session/boot-scoped token and stable `ref` values; `ui diff` compares tokens.
- `tap`, `long-press`, `text` and `scroll` accept `--wait-for-text`, `--wait-for-absent-text`,
  `--wait-for-value`, `--wait-for-package`, `--wait-for-activity` and `--timeout` postconditions.
- The same actions accept `--receipt` / `--receipt-full` for before/after tokens, semantic diff,
  deterministic effect classification and explicit completeness.

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
  `--install`, `--fail-fast`, `--dry-run` and cleanup reporting. `--parallel` >1 is explicitly rejected.
- `ui observe|diff` and logcat peek/tail accept `--retries`; `ui tap|long-press|text|clear|scroll|wait|assert`
  accept `--ref` and `--expect-token`.

### Evidence and diagnostics

- `capture screenshot`; `capture record start|stop|status`.
- `diagnostics collect|app|logcat tail|start|stop|status|crashes|inspect|performance`.
- `notifications list|wait|assert|channels|tap|clear`.
- `jobs list|run|cancel|wait|assert`.
- `alarms next|list|wait`.
- `doze status|force|unforce`; `standby get|set`; `battery status|unplug|reset`; `network status|airplane|wifi|data`; `time get|set|restore`.

### Automation and reports

- `hook list|run`; hooks can declare shell/command/UI `verify` postconditions (`ui_text`, `ui_activity`, `ui_package`) and `run_as` broadcasts for debuggable
  non-exported receivers.
- `fixture list|validate|run`.
- `conformance run`.
- `test maestro|instrumentation`.
- `run start|end|status`, `run exec` / `run -- <program>`.
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
- HTML reports and automated visual diff.
- A replacement for Gradle, JUnit, Robolectric, Compose UI tests or Espresso.
- iOS or non-Android devices.

## Verified versus unverified

The `0.1.0` delivery run exercised the CalcSuite, DiscountHunter and IronPulse smoke flows on a real Android 16 emulator, including install/hash/certificate identity, semantic UI work, fixtures, hooks, screenshots, recording, logcat, diagnostics, one report and clean shutdown. The `0.2.0` delivery added the native desktop and exercised it against CalcSuite. The `0.3.0` delivery exercised the redesigned desktop against a real emulator: onboarding screens, environment diagnostics, session restart, APK inspect/install, conformance run, cancellation and reconciliation, live logcat, device preview and semantic hierarchy actions, run comparison and maintenance preview/cleanup. The `0.4.0` delivery introduced and exercised the agent loop: compact context, semantic observations and diffs, postconditions, session ensure, read-only repair planning, deterministic explanations and the agent pipeline. The `0.5.0` delivery fixed the issues reported by three independent app agents: shared-host port selection, UIAutomator recovery, pipeline status, project-scoped run discovery, observation shape, compact diffs, selector guidance, desktop help/version and session project binding. The `0.6.0` delivery added and exercised action receipts on a real emulator, real installed-APK diff/only-if-changed, recurrence classification, redacted bundle creation/verification and the Context/Ownership desktop pages. The `0.7.0` delivery added and exercised a two-AVD matrix (API 36 and 34), stable refs with stale rejection, device health, disconnect/recovery and read-only retry reporting, plus the Matrix and device health desktop workflows. The `0.8.0` delivery added and exercised split/`.apks` install and hash equivalence, refs for text/clear/scroll/wait/assert with stale refusal, explicit startup/jank/memory/CPU evidence and the desktop performance panel. The `0.9.0` delivery added and exercised full/compact/brief output profiles, `agent brief`, shorter refs/tokens, command aliases and measured byte reductions without changing the full JSON contract. The `0.9.1` delivery fixed the defects found by three independent app validations: deterministic observation retention, identity-stable refs, context refs through clickable ancestors, `--clickable` ancestor matching, selector label truncation, reference/observation diagnosis, project-scoped `latest`/`latest-failure`, verified bundle manifests, performance run identity and hook UI postconditions. The `0.9.2` delivery closed the independent certification findings: full-observation context refs, cross-project session refusal, honest hook verification state, bounded logcat byte-cap completion and clearer receipt evidence. Physical-device, Maestro, instrumentation-package, snapshot-restore, root-only time paths and non-125% DPI layouts were not exercised. See [VERIFICATION.md](VERIFICATION.md) for the exact record.
