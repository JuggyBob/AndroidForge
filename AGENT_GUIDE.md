# AndroidForge — Agent Guide

This is the front door for an autonomous coding agent. Read this before issuing commands.

## 1. The operating model

Every meaningful test is a **run** with a **session** and a **device**.

1. `doctor --json` — confirm the SDK, ADB and emulator are discovered.
2. `capabilities --json` — see what this host can actually do.
3. `session start` (headless) or `session attach` — claim one device.
4. Do work against that session.
5. Collect evidence.
6. `session stop` or `session release` — do not leave ownership or emulators behind.

A session is the safe unit of ownership. Do not touch a device owned by another AndroidForge session.
AndroidForge coordinates AndroidForge sessions; it cannot prevent an unrelated process from using ADB.

## 2. JSON, stderr and exit codes

Use `--json` on every command. Stdout then contains exactly one JSON envelope. Progress, trace and
human diagnostics go to stderr. `--trace` also sends external argv/timeouts/exit codes to stderr.

```json
{
  "schema_version": 1,
  "status": "success",
  "command": "ui.assert",
  "exit_code": 0,
  "run_id": "20261115-111132-...",
  "session_id": "ses-...",
  "device": {"serial": "emulator-5560"},
  "result": {"matched": true},
  "artifacts": [],
  "duration_ms": 340
}
```

Branch on **numeric exit codes** (see [EXIT_CODES.md](EXIT_CODES.md)), not prose. Do not assume a command
worked because it printed a line. In JSON mode never parse human output from stdout.

## 3. Starting a device

```powershell
androidforge session start --avd <AVD> --port 5556 --headless --owner <AgentName> --json
```

- Headless is the default; use `--windowed` only when a human needs to see it.
- `--reuse` attaches to an already-running emulator for the AVD if it is unowned.
- New emulators are started with `-no-snapshot-save`; they do not silently preserve test state.
- `--port` must be an even console port. Automatic allocation starts at 5554; explicit requests
  accept even 1024–65532, including environment-reserved ports below the conventional range. ADB uses
  port + 1.
- On success, keep the returned session ID. You can export `ANDROIDFORGE_SESSION` or run
  `androidforge session use <ID>` to make it current for this project.
- `session attach --serial <SERIAL>` claims an existing emulator or physical device.
- `session start --avd ... --prepare` also disables animations and keeps the screen awake; `device
  restore` or `session stop` restores the settings it changed.
- `session release` releases ownership but keeps an owned emulator running; a later `session stop
  --session <ID>` can still stop that released tombstone if no new session owns the device.
- `session list --active` hides released/failed/abandoned tombstones.
- `session stop`, `session release` and `session restart` verify the session's `owner` label. Pass the
  same `--owner` used at start (or export `ANDROIDFORGE_OWNER`) or use `--force` for a deliberate
  takeover. This blocks one agent from stopping another agent's emulator by ID; the owner is a
  coordination label, not authentication.

If the device is owned by another session, you get `E_DEVICE_BUSY`. Do not work around that by killing
the emulator. Inspect `session list`; use `session gc --reclaim` only if the owner is genuinely gone.

## 4. Installing a build

```powershell
androidforge app install .\app-debug.apk --session <ID> --json
```

The result includes package, versionName, versionCode, local APK SHA-256, installed APK SHA-256, signing
certificate SHA-256 and debug-signed status. Installation fails if the installed package does not match
the requested package. A signing mismatch is diagnosed, **never** auto-uninstalled:

- `E_INSTALL_SIGNATURE_MISMATCH` lists both certificates and the installed version.
- To replace the app and delete its data, do it explicitly: `app install <apk> --fresh --yes`.
- To preserve debuggable app data first: `app data-backup <pkg> <file>` (debuggable apps only), then
  uninstall/install deliberately.

Never put signing secrets in a project or fixture. AndroidForge does not read keystores.

## 5. UI work

Prefer semantics over coordinates.

```powershell
androidforge ui dump --session <ID> --json
androidforge ui find --session <ID> --contains "Save" --json
androidforge ui tap --session <ID> --text "Save" --json
androidforge ui wait --session <ID> --text "Saved" --timeout 20 --json
androidforge ui assert --session <ID> --text "Saved" --json
androidforge ui assert --session <ID> --text "Something went wrong" --not-exists --json
androidforge ui settle --session <ID> --json
androidforge ui scroll-to --session <ID> --text "Finance" --json
androidforge ui text "search words" --session <ID> --class android.widget.EditText --clear --hide-keyboard --json
androidforge ui back --session <ID> --hide-keyboard --json
```

Rules:

- Exact matching is the default. Use `--contains` deliberately.
- `ui find` is a search and succeeds with `matches: []` when nothing matches; `ui assert`, `ui tap`
  and `ui wait` fail when the condition is absent.
- If a selector matches more than one element, AndroidForge refuses to guess. Use `--index N` from the
  listed candidates or make the selector more specific.
- Coordinates are real device pixels in the current orientation. Never scale from an unrelated
  screenshot.
- Text entry verifies an editable focused field, clears explicitly when requested, sends batched
  deletions, re-reads the field, and fails if the text does not match. Empty strings verify as empty
  fields. Android Settings and some OEM fields expose their hint/placeholder as `text`; AndroidForge
  reads `hint` and treats an exact hint as empty before typing and after clearing. Passwords cannot be
  read back; result says `verified=false` rather than claiming success.
- `ui clear` with no selector requires a focused editable field. If none is focused it returns
  `E_SELECTOR_NOT_FOUND` with the focused nodes and tells you to tap the field or pass a selector,
  rather than a misleading text-verification failure.
- `ui back` refuses to run while the soft keyboard is up unless `--hide-keyboard` is provided. Avoid
  fixed sleeps; use `ui wait`, `ui settle` and `ui scroll-to`.
- `ui assert` is deliberately a quick one-attempt check (default 1s). Pass `--timeout` or use `ui
  wait` first when the screen is still animating or loading.
- Compose test tags are visible only if the app exposes them as accessibility resource IDs
  (`testTagsAsResourceId`) or equivalent semantics. UIAutomator cannot see arbitrary internal Compose
  test nodes. Use instrumentation when precise internals are required.

## 6. Evidence and failure capture

Screenshots are written as binary through Rust file I/O, never terminal redirection:

```powershell
androidforge capture screenshot --session <ID> --label checkpoint --json
```

Automatic failure capture is on by default for app/UI/fixture/conformance steps. It records a screenshot,
UI hierarchy with focused nodes, logcat tail, crash/ANR state, process/task/foreground state and
`dumpsys` evidence. The error JSON includes artifact paths. Disable it only with
`--no-failure-capture`.

Logcat is never cleared by default. One-shot tails are bounded; continuous captures have explicit
time and byte caps and a `diagnostics logcat stop` lifecycle.

## 7. Runs and reports

For a multi-step flow, keep one report:

```powershell
androidforge run start --session <ID> --name smoke --json
# ... commands using the same session/run ...
androidforge run end --session <ID> --json
androidforge report show <RUN_ID> --json
androidforge report path <RUN_ID> --json
androidforge report list --json
```

`run start` makes the run active for the session and project. Every run has `report.json`,
`metadata.json` and a checksum `manifest.json`. Native commands append steps. `ANDROIDFORGE_RUN_ID` is
injected into scripts; a child script can call `androidforge` and its steps join the same run.

`run end` derives the status from its steps: any failed step makes the run failed and exits non-zero.
Use `--status failed` to make that explicit, or `--allow-failures --status passed` for a deliberate
override. `run end --run <ID>` clears the active pointer when that run is the session's active run, so
the next command starts a new run instead of appending to a finished one. A stale pointer to an
already-finished run is ignored and cleared automatically.

A run that did not install inherits the session's observed build identity; if none is known, it reads
the installed package/version once and marks hashes/certificate unverified rather than inventing them.

`report show` and `report path` also search project-local `.artifacts/runs` directories under the
current project tree. A conformance run under `examples/<App>/.artifacts/runs` can therefore be
inspected from the repository root without repeating `--project`.

For a single external process, use:

```powershell
androidforge run --session <ID> --name update-flow --capture-screen --capture-logcat -- python my_script.py
```

The external process gets the documented `ANDROIDFORGE_*` environment contract. Its stdout/stderr are
artifacts, and its exit status is in the report. AndroidForge exits with a stable failure code if the
external process failed.

`report import junit <file> --run <ID>` merges Gradle/JUnit XML into the same run. It does not replace
Gradle, Robolectric, Compose instrumentation or your existing unit tests.

## 8. Fixtures, hooks and conformance

Project-specific behavior belongs in `.androidforge/project.toml`, fixtures, hooks and scripts. Never
add app-specific code to the core.

```powershell
androidforge --project examples/CalcSuite fixture list --json
androidforge --project examples/CalcSuite fixture validate smoke --json
androidforge --project examples/CalcSuite fixture run smoke --session <ID> --json
androidforge --project examples/CalcSuite hook list --json
androidforge --project examples/CalcSuite hook run open-settings --session <ID> --json
androidforge --project examples/CalcSuite conformance run --session <ID> --json
```

A conformance result includes the same build identity as `report.json`. If the external flow prints a
final JSON object with a `checks` array, those named checks become `flow_checks` in the command result
and in the conformance step's details; the flow exit code remains the pass/fail authority. Local
install preflight failures (expected package/certificate) skip the full screenshot/logcat/dumpsys
bundle because the device was never touched.

Fixtures support phases (`pre-install`, `post-install`, `pre-test`, `post-test`, `cleanup`), retries,
per-step timeouts, `on_failure`, capture policy, explicit destructive steps, file transfer, app-private
files via `run-as` only for debuggable apps, and external scripts. They never run automatically just
because a config exists. Use `--dry-run` to inspect a plan and `--yes` for destructive steps.

Hooks are named, explicit debug triggers. AndroidForge does not interpret the hook name. Give a hook a
`verify` postcondition when ADB exit 0 is not proof that the app did the work. For a non-exported
receiver on a debuggable build, use `run_as = true` so the explicit broadcast runs as the app UID; a
plain shell broadcast can report `Broadcast completed: result=0` without delivery. If the postcondition
does not change or match, the hook fails with `E_HOOK_VERIFICATION_FAILED`. If an app cannot expose a
deterministic trigger, prefer an honest message over guessing a JobScheduler ID.

`jobs list` accepts both numeric and named/WorkManager namespaces on Android 16; parsed output remains
best-effort and raw dumps stay the source of truth.

## 9. Safe, reversible and destructive operations

**Safe/read-only:** `doctor`, `devices list/inspect`, `ui dump/find/focused`, `apk inspect/verify`,
`report show/path`, `diagnostics inspect`, `get` commands, `app status` without assertions.

**Reversible (AndroidForge records restoration):** `device prepare` (settings), `session start --prepare`,
`devices restore`, `session release`/`session stop` (restores settings before stopping).

**Destructive or disruptive:** `app clear-data` (`--yes`), `app data-restore` (`--yes`), `app install
--fresh` (`--yes`), `snapshot load` (`--yes`), `time set` (`--yes`), `report prune` (`--yes` unless
`--dry-run`), `doze force`/network airplane (`--yes`), `session stop --attached` for an externally
started emulator. Never assume these are safe because a JSON envelope came back.

## 10. Error recovery

- `E_DEVICE_AMBIGUOUS` — choose `--serial` or `--session`; run `devices list`.
- `E_DEVICE_BUSY` - another session owns the device; do not kill it.
- `E_SESSION_OWNERSHIP_MISMATCH` - pass the owner used at `session start` or use `--force` only for a deliberate takeover.
- `E_ADB_NOT_FOUND` / `E_SDK_NOT_FOUND` - run `doctor`; set `ANDROID_HOME` or `--sdk` config.
- `E_SELECTOR_NOT_FOUND` — `ui dump`, use `--contains` or a more specific resource ID; `ui wait` if loading.
- `E_SELECTOR_AMBIGUOUS` — inspect candidates and pass `--index N`.
- `E_COMMAND_TIMEOUT` — inspect device state and retry with a larger explicit timeout; do not loop forever.
- `E_DEVICE_OFFLINE` / `E_DEVICE_DISCONNECTED` — reconnect or restart only an emulator you own.
- `E_INSTALL_SIGNATURE_MISMATCH` — read the diagnosis; do not auto-uninstall.
- `E_SESSION_EXPIRED` — start/attach a new session.
- `E_UNSUPPORTED_OPERATION` — read the capability matrix; use the suggested alternative.

## 11. End-to-end example

```powershell
androidforge doctor --json
androidforge session start --avd androidforge_test --headless --owner CalcSuiteAgent --json
# ses-3457532120af, emulator-5560

androidforge app install .\CalculatorSuite-v3.0.3.apk --session ses-3457532120af --json
androidforge app launch com.calcsuite --session ses-3457532120af --json
androidforge ui assert --session ses-3457532120af --text "Calc Suite" --json
androidforge ui tap --session ses-3457532120af --text "Algebra" --json
androidforge capture screenshot --session ses-3457532120af --label algebra --json
androidforge diagnostics collect --session ses-3457532120af --package com.calcsuite --json
androidforge app status com.calcsuite --session ses-3457532120af --expect foreground --json

androidforge run start --session ses-3457532120af --name release-smoke --json
androidforge ui assert --session ses-3457532120af --text "Calc Suite" --json
androidforge --project examples/CalcSuite fixture run smoke --session ses-3457532120af --json
androidforge run end --session ses-3457532120af --json
androidforge report show latest --json

androidforge session stop --session ses-3457532120af --owner CalcSuiteAgent --json
```

The actual command surface and current verified status are in [CAPABILITIES.md](CAPABILITIES.md) and
[VERIFICATION.md](VERIFICATION.md). If a command is not listed as supported, expect a structured
`E_UNSUPPORTED_OPERATION` rather than a fake success.
