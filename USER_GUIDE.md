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
- Optional: Maestro CLI, an instrumentation test APK.

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
resolved CLI version and connection state. Settings lets you change the CLI path, workspace, owner and
light/dark/system theme.

If the CLI is offline, run `androidforge doctor --json` in the same environment. The desktop refuses a
CLI with an unknown report schema rather than guessing. A missing or incompatible CLI is shown in the
Overview error banner and in Settings.

### Pages

- **Overview** — environment status, devices/sessions/runs counters, recent runs and CLI resolution.
- **Devices** — selectable device table with serial, model, Android version, state, battery and owner;
  details include boot state, AVD, port, architecture and resolution.
- **Sessions** — selectable session table with owner, heartbeat and state; details show prepared
  settings and expose `Release`, `Stop`, `Restart`, `Use as target` and `Install APK`. Released/abandoned
  tombstones are read-only.
- **Runs** — report-backed runs, project hooks and `Run project flow`. Selecting `Open` or a row shows
  build identity, steps, failure details and evidence.

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

## 6. Emulators and snapshots

```powershell
androidforge avd list --json
androidforge emulator start --avd androidforge_test --port 5560 --headless --json
androidforge emulator status --serial emulator-5560 --json
androidforge emulator stop --session <ID> --json
```

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

`session use` writes `.androidforge/current-session` for the project. `ANDROIDFORGE_SESSION` is the environment alternative. `session release` releases ownership and restores prepared settings; `session stop` also stops an emulator the session owns, or an attached emulator with `--attached`. `session release --stop` stops an owned emulator; attached emulators additionally require `--stop-attached`. If you release an owned emulator without stopping it, the released tombstone can still be stopped with `session stop --session <ID>` as long as no new session owns the device. `session list --active` hides released/failed tombstones.

`session stop`, `session release` and `session restart` are owner-guarded. Pass the same `--owner` used at `session start` (or export `ANDROIDFORGE_OWNER`) or use `--force` for a deliberate takeover. Without a matching owner they return `E_SESSION_OWNERSHIP_MISMATCH` with both labels and do not touch the emulator. This is a coordination guard, not authentication: anyone who can read the local session file can still use `--force`.

If another session owns the device, you get `E_DEVICE_BUSY`. Do not kill the emulator. Check `session list`; use `session gc --reclaim` only when the owner is genuinely dead. `session gc --help` also has `--stop` for stale owned emulators.

## 8. APKs

```powershell
androidforge apk inspect .\app-debug.apk --json
androidforge apk verify .\app-debug.apk --expected-cert-sha256 <64-hex> --expected-package com.example.app --min-version-code 10 --json
```

Inspection reports package, versions, min/target SDK, label, launchable activity, architectures, size, SHA-256 and signature details. Verification returns `E_SIGNATURE_VERIFICATION_FAILED` on a bad certificate and `E_APK_INVALID` on an unreadable APK.

### Install and build identity

```powershell
androidforge app install .\app-debug.apk --session <ID> --json
androidforge app install .\app-release.apk --session <ID> --expected-package com.example.app --json
```

The result includes package, versionName, versionCode, local APK SHA-256, installed APK SHA-256, signing certificate SHA-256, debug-signed and `verified`. A mismatch between the requested and installed package, hash or version fails with `E_INSTALL_VERIFICATION_FAILED`. If `[project].expected_cert_sha256` is set and the APK uses a different signer, install fails before touching the device with `E_SIGNATURE_VERIFICATION_FAILED` and reports the actual fingerprint. Add `--expected-cert-sha256 <64-hex>` for an ad-hoc fingerprint check, or `--expected-package` for an ad-hoc package check.

`--dry-run` inspects the APK locally and reports its package/version/hash/certificate plus the planned
fresh, downgrade, package and certificate constraints without installing.

A signing mismatch is `E_INSTALL_SIGNATURE_MISMATCH`, listing both certificates and the installed version. AndroidForge never auto-uninstalls. To replace the app and delete its data:

```powershell
androidforge app install .\app-release.apk --session <ID> --fresh --yes --json
```

Back up debuggable data first if it matters:

```powershell
androidforge app data-backup com.example.app .\backup.tar --session <ID> --json
```

Use `--no-verify` only when you explicitly need to skip post-install hash verification; it weakens evidence.

## 9. App control

```powershell
androidforge app launch com.example.app --session <ID> --json
androidforge app status com.example.app --session <ID> --expect foreground --json
androidforge app status com.example.app --session <ID> --expect foreground --assert-task-exists --assert-process-stable --stable-for 2s --json
androidforge app force-stop com.example.app --session <ID> --json
androidforge app kill com.example.app --session <ID> --json
androidforge app clear-data com.example.app --session <ID> --yes --json
androidforge app uninstall com.example.app --session <ID> --yes --json
androidforge app list --session <ID> --json
androidforge app info com.example.app --session <ID> --json
```

`force-stop` removes the task/process and stops scheduled work. `kill` is the softer `am kill`. `clear-data` and `uninstall` are destructive and require `--yes`. `run-as` file operations require a debuggable build:

```powershell
androidforge app file-push .\seed.json files/seed.json --package com.example.app.debug --session <ID> --json
androidforge app file-pull files/seed.json .\pulled.json --package com.example.app.debug --session <ID> --json
```

Remote paths are relative to the app's private data directory and may not contain `..`.

## 10. UI work

Start with a dump or a find, not coordinates.

```powershell
androidforge ui dump --session <ID> --json
androidforge ui find --session <ID> --text "Save" --json
androidforge ui find --session <ID> --contains "Save" --json
androidforge ui find --session <ID> --resource-id com.example.app:id/save --json
androidforge ui focused --session <ID> --json
```

Actions:

```powershell
androidforge ui tap --session <ID> --text "Save" --json
androidforge ui long-press --session <ID> --content-desc "Delete" --json
androidforge ui text "hello world" --session <ID> --class android.widget.EditText --clear --hide-keyboard --json
androidforge ui clear --session <ID> --json
androidforge ui back --session <ID> --hide-keyboard --json
androidforge ui home --session <ID> --json
androidforge ui keyevent KEYCODE_ENTER --session <ID> --json
androidforge ui swipe --session <ID> --x1 500 --y1 1800 --x2 500 --y2 600 --json
```

Waiting and assertions:

```powershell
androidforge ui wait --session <ID> --text "Saved" --timeout 20 --json
androidforge ui wait --session <ID> --contains "Loading" --not-exists --timeout 20 --json
androidforge ui settle --session <ID> --timeout 10 --json
androidforge ui assert --session <ID> --contains "Welcome" --json
androidforge ui assert --session <ID> --text "Error" --not-exists --json
androidforge ui scroll-to --session <ID> --text "Settings" --json
```

Rules:

- Exact matching is the default; use `--contains` deliberately.
- `ui find` is a search: it succeeds with `matches: []` when nothing matches. `ui assert`, `ui tap` and `ui wait` fail with `E_SELECTOR_NOT_FOUND` when the condition is absent.
- `ui assert` is a quick one-attempt check (default 1s). Pass `--timeout` or use `ui wait` first for animated/loading screens.
- `ui clear` with no selector clears the focused editable field. If none is focused it returns `E_SELECTOR_NOT_FOUND` with the focused nodes and a selector suggestion. Android Settings and some OEM fields expose their hint as `text`; AndroidForge treats an exact hint as empty before typing and after clearing, so clearing a search field does not false-fail.
- Ambiguity is an error with candidates. Add `--index N` from the candidate list or narrow the selector.
- Taps use current device pixels and optionally a clickable ancestor.
- Text entry verifies the focused editable field, clears with batched deletes when asked, re-reads and compares. Password fields report unverified.
- `ui back` refuses while the IME is up unless `--hide-keyboard` is explicit.
- Avoid fixed sleeps. Use `wait`, `settle` and `scroll-to`.
- Compose test tags are not visible unless the app exposes them as accessibility resource IDs. Use instrumentation for internal tags.

## 11. Evidence

### Screenshots and recordings

```powershell
androidforge capture screenshot --session <ID> --label before --json
androidforge capture record start --session <ID> --time-limit 30 --json
androidforge capture record status --session <ID> --json
androidforge capture record stop --session <ID> --json
```

Screenshots include dimensions, display size, timestamp and SHA-256. Recordings are MP4, no audio, and Android caps standard `screenrecord` at 180 seconds. A recording started inside a caller-owned run does not finalize that run on stop.

### Logcat

```powershell
androidforge diagnostics logcat --session <ID> --package com.example.app --lines 2000 --json
androidforge diagnostics logcat start --session <ID> --package com.example.app --json
androidforge diagnostics logcat status --session <ID> --json
androidforge diagnostics logcat stop --session <ID> --json
```

One-shot tails are bounded. Continuous capture runs on-device with `[diagnostics].logcat_max_seconds` and `max_log_bytes` caps. AndroidForge never clears the global buffer by default.

### Diagnostics collection

```powershell
androidforge diagnostics collect --session <ID> --package com.example.app --json
androidforge diagnostics app com.example.app --session <ID> --json
androidforge diagnostics crashes com.example.app --session <ID> --json
androidforge diagnostics inspect activity --session <ID> --json
```

`collect` captures screenshot, hierarchy, logcat, crash/ANR state, app state and dumpsys. `--full` includes every supported dump; `--log-lines` controls the tail. Crash/ANR detection is best-effort and says so.

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
androidforge time get --session <ID> --json
```

Short permission names are normalized to `android.permission.*`. `SCHEDULE_EXACT_ALARM` and similar special permissions may need AppOps or user consent; a failed grant reports what remains. Notification/job/alarm/Doze parsers are best-effort across Android versions; raw dumps are preserved.

Interactive toggles (`doze force`, `network airplane`, `time set`) require `--yes`. Time changes usually require root, which AndroidForge will not provide.

## 12. Runs and reports

For a multi-step flow:

```powershell
androidforge run start --session <ID> --name smoke --json
# subsequent commands with --session <ID> join the active run
androidforge app install .\app-debug.apk --session <ID> --json
androidforge ui assert --session <ID> --text "Welcome" --json
androidforge run end --session <ID> --json
androidforge report show latest --json
androidforge report path latest --json
androidforge report list --json
```

`run end` derives the run status from its steps. Any failed step makes the run `failed`, prints the
failure, and exits with the matching non-zero code. Use `--status failed` explicitly or
`--allow-failures --status passed` only for a deliberate override. `run end --run <ID>` clears the
active pointer when that run is the session's active run, so a later command starts a new run instead
of appending to a finished one; a stale pointer to a finished run is ignored and cleared. A run that
did not install inherits the build identity observed in the session; if none is known it reads the
installed package/version once and marks hash/certificate verification false.

`report show` and `report path` search the current artifacts directory first, then project-local
`.artifacts/runs` directories under the project tree. This makes a conformance run under
`examples/<App>/.artifacts/runs` inspectable from the repository root without `--project`.

For one existing script:

```powershell
androidforge run exec --session <ID> --name legacy-flow --capture-screen --capture-logcat -- python .\scripts\smoke.py --json
# or: androidforge run --session <ID> --name legacy-flow -- python .\scripts\smoke.py
```

The script receives `ANDROIDFORGE_SESSION`, `ANDROIDFORGE_SERIAL`, `ANDROIDFORGE_PACKAGE`, `ANDROIDFORGE_RUN_ID`, `ANDROIDFORGE_ARTIFACTS`, `ANDROIDFORGE_ADB`, `ANDROIDFORGE_ROOT`, `ANDROIDFORGE_PROJECT`, `ANDROIDFORGE_EXE` and `ANDROIDFORGE_PHASE`. It can call `androidforge` recursively and its steps join the same report.

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

## 13. Fixtures, hooks and conformance

```powershell
androidforge --project .\examples\DiscountHunter fixture list --json
androidforge --project .\examples\DiscountHunter fixture validate smoke --json
androidforge --project .\examples\DiscountHunter fixture run smoke --session <ID> --json
androidforge --project .\examples\DiscountHunter hook list --json
androidforge --project .\examples\DiscountHunter hook run check-deals --session <ID> --json
androidforge --project .\examples\DiscountHunter conformance run --session <ID> --json
```

Use `--dry-run` to inspect a fixture plan and `--yes` for destructive steps. Fixtures never run
automatically. Hooks are explicit debug triggers; do not guess a JobScheduler ID when the app has no
deterministic hook.

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
| Desktop refuses the CLI schema | The desktop and CLI are version-mismatched; use the aligned `0.3.0` pair or a compatible release. |
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
