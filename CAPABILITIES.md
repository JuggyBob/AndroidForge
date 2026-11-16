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
| `apk.inspect` | supported | aapt2 | Package, version, SDKs, label, launchable activity, architectures, hash. |
| `apk.verify` | supported | aapt2, apksigner, Java | Local signature verification. No signing automation. |
| `app.install` | supported | adb | Signature-aware diagnostics; never auto-uninstalls. |
| `ui.uiautomator` | supported | adb, uiautomator | Compose semantics only when exposed through accessibility. |
| `capture.screenshot` | supported | adb | Binary PNG via `exec-out`; no console redirection. |
| `capture.recording` | best-effort | adb, screenrecord | Android's 180-second limit; no audio. |
| `diagnostics.logcat` | supported | adb | Never clears the global buffer by default; bounded. |
| `diagnostics.jobs` | best-effort | adb, dumpsys jobscheduler | Raw output preserved; job IDs are never hardcoded. |
| `diagnostics.notifications` | best-effort | adb, dumpsys notification | Formatting varies by Android version. |
| `diagnostics.doze.time` | best-effort | adb, device privileges | Time changes usually require root; AndroidForge never roots. |
| `fixtures` | supported | project adapter | Explicit project hooks and fixture scripts; never automatic. |
| `maestro` | unsupported unless installed | optional Maestro CLI | Local only; no Maestro Cloud, accounts or API keys. |
| `instrumentation` | supported | adb, test APK | Runs installed instrumentation; does not replace Gradle. |
| `snapshots` | best-effort | adb, emulator | Verified at runtime; loading always requires `--yes`. |
| `avd.cloning` | unsupported | none | Not implemented; a renamed profile would not provide isolation. |
| `html.reports` | unsupported | none | JSON and raw artifacts are the primary evidence. |
| `visual.diff` | unsupported | none | Screenshot evidence is supported; automated comparison is deferred. |
| `cloud` | unsupported | none | No telemetry or upload. |

## Desktop cockpit

`androidforge-desktop.exe` is a separate native executable, so it is not reported by
`androidforge capabilities --json`. Its scope is deliberately narrower than the CLI:

- P0 read-only: project selection, environment/doctor summary, devices, sessions, report-backed runs,
  run details/build identity/steps/failure, evidence (screenshots, hierarchy/log text), raw
  `report.json`/`metadata.json`/`manifest.json` and manifest verification.
- P1 controlled actions: session start/attach/release/stop/restart, APK inspect/dry-run/install,
  fixture/conformance/hook launch, streamed output, and cancellation for installs/fixtures/conformance/
  hooks. Session lifecycle operations run to completion.
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
- `session start|attach|claim|list|inspect|status|release|stop|use|current|gc|restart`.
  `session list --active` hides released/failed tombstones; a released tombstone can still stop its
  owned emulator if nobody else owns it. `session stop|release|restart` require the matching `--owner`
  (or `ANDROIDFORGE_OWNER`) unless `--force` is used for a deliberate takeover.

### Apps and APKs

- `apk inspect|verify`.
- `app install|uninstall|clear-data|launch|force-stop|kill|status|info|list|file-push|file-pull|data-backup|data-restore`.
- `permissions check|grant|revoke`.
- `appops get|set|reset`.

### UI

- `ui dump|find|tap|long-press|text|clear|wait|assert|settle|scroll|scroll-to|swipe|back|home|keyevent|hide-keyboard|focused`.

### Evidence and diagnostics

- `capture screenshot`; `capture record start|stop|status`.
- `diagnostics collect|app|logcat tail|start|stop|status|crashes|inspect`.
- `notifications list|wait|assert|channels|tap|clear`.
- `jobs list|run|cancel|wait|assert`.
- `alarms next|list|wait`.
- `doze status|force|unforce`; `standby get|set`; `battery status|unplug|reset`; `network status|airplane|wifi|data`; `time get|set|restore`.

### Automation and reports

- `hook list|run`; hooks can declare `verify` postconditions and `run_as` broadcasts for debuggable
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

The `0.1.0` delivery run exercised the CalcSuite, DiscountHunter and IronPulse smoke flows on a real Android 16 emulator, including install/hash/certificate identity, semantic UI work, fixtures, hooks, screenshots, recording, logcat, diagnostics, one report and clean shutdown. The `0.2.0` delivery added the native desktop and exercised it against CalcSuite. The `0.3.0` delivery exercised the redesigned desktop against a real emulator: onboarding screens, environment diagnostics, session restart, APK inspect/install, conformance run, cancellation and reconciliation, live logcat, device preview and semantic hierarchy actions, run comparison and maintenance preview/cleanup. It also validated the portable ZIP and per-user installer. Physical-device, Maestro, instrumentation-package, snapshot-restore and root-only time paths were not exercised. See [VERIFICATION.md](VERIFICATION.md) for the exact record.
