# Cookbook — AndroidForge 1.1.0

Six practical recipes for repeatable agent runs. Every command prints a JSON envelope with
`command`, `exit_code`, `result`, `artifacts`, `session_id` and `error`; branch on `exit_code` and
`error.code`, not on prose. See [USER_GUIDE.md](../USER_GUIDE.md),
[SCHEMA.md](../SCHEMA.md) (run layout and schemas), [CAPABILITIES.md](../CAPABILITIES.md)
(status vocabulary) and [EXIT_CODES.md](../EXIT_CODES.md#complete-code-table).

All 1.1.0 surfaces used below are wired and covered by the green host gate (784 passed / 0 failed
across 29 suites; fmt, strict clippy and cargo-deny clean) and the post-fix CLI gate (375 passed /
0 failed). Confirm exact leaf paths and flags with `androidforge tools catalog --json`. Packaging and
the physical-device campaign are complete for the approved scope: packaged assets and hashes are
recorded in [VERIFICATION.md](../VERIFICATION.md), and a delegated POCO X4 GT / Android 14 (API 34)
campaign passed 12/12 read-only steps plus explicitly approved launch/input/back mutations, with both
sessions released. Do not assume physical `diagnostics performance` or the device night-mode scenario
is covered — both are **NOT RUN** (not approved). Also **NOT RUN:** silent install/uninstall
(installed Desktop release directory untouched), standalone `run repeat --count`, cross-device
geometry/colour comparison and a v1.1.0 screenshot gallery.

Never use `androidforge adb` / raw `adb` for these flows: it bypasses redaction, risk gates and
evidence capture. Prefer an isolated emulator and a unique `ANDROIDFORGE_HOME`; never take a device
owned by another session.

---

## 1. Theme / dark / AMOLED and splash / first-frame verification

**Goal.** Force night mode, confirm the app renders the expected background, and prove the first
app frame is real content rather than a splash or a blank launch frame.

**Prerequisites.** A session (`session ensure` or `session attach`), a debuggable build if the app
sets its own theme, and a screen that is on. If you assert Compose `--test-tag`, the app must expose
`testTagsAsResourceId = true` in a debug build; AndroidForge does not inject it.

```powershell
androidforge session ensure --project . --avd Pixel_8_API_35 --owner <OWNER> --json
androidforge devices prepare --session <ID> --dark-mode true --keep-awake --json
androidforge ui dismiss-overlays --session <ID> --json
androidforge ui observe --session <ID> --package com.example.app --json
androidforge capture assert-color --session <ID> --rgb #000000 --region 0,0,1080,240 --majority --tolerance 4 --json
androidforge capture launch com.example.app --session <ID> --fps 10 --duration 5s --expect-first-frame --no-splash --expected-color #101010 --color-sample dominant --tolerance 12 --json
androidforge devices restore --session <ID> --json
```

**Expected evidence/artifacts.** A JSON result with `actual`, `expected`, `tolerance` and
`max_channel_delta` from `capture assert-color`; a `capture launch` result with `first_frame_ms`,
observed focus and sampled colours, plus `capture-launch/frame-NNNN.png` PNGs in the run directory
(`result.run_dir` / `result.run_id`). `devices prepare` records previous values so `devices restore`
reverses only what AndroidForge changed. Payload shapes:
`androidforge schema show capture-launch.schema.json --json`.

**Failure interpretation.** `E_UI_ASSERTION_FAILED` (exit 7) means the sampled colour or first-frame
assertion did not hold — inspect the sampled PNGs and `actual` before changing app code. No first
frame with `--expect-first-frame`, or a blank frame with `--no-splash`, points to a splash/theme
transition; add `--ui-dump-package` if focus did not identify the app. `E_SCREEN_OFF` (4) means the
display was asleep; re-run with an explicitly awake screen. `E_DEVICE_AMBIGUOUS` (4) means pass
`--session` or `--serial`. A `devices prepare` failure is a device-setting change, not a theme bug.

**Safety.** `devices prepare --dark-mode` changes device settings and requires
`--allow-physical-data` (or `--allow-physical-mutation`) on a physical device; use a throwaway AVD.
`capture launch` launches the app, so a physical device also needs `--allow-physical-app-control`.
`capture assert-color` is host-only (`exec-out screencap -p`, no device temp file). No raw adb.

---

## 2. DataStore / prefs fixture seeding without rebuilding

**Goal.** Give a debuggable app deterministic prefs, a bounded history payload and a fake installed
version without changing or reinstalling the APK.

**Prerequisites.** A debuggable app, a project adapter with `[project].package` (or `package` on
each state-bearing step), and a fixture file under `[fixtures].directory` (default
`.androidforge/fixtures/`). A host-side writer script owns the DataStore format: AndroidForge never
parses DataStore protobuf.

`.androidforge/fixtures/update-state.json`:

```json
{
  "version": 1,
  "name": "update-state",
  "state": {
    "prefs": {
      "path": "files/androidforge/fixture-prefs.txt",
      "values": { "theme": "amoled", "onboarding_done": "true" }
    },
    "history": {
      "count": 25,
      "file": "files/androidforge/history.bin",
      "command": ["python", "tools/write_history.py"]
    },
    "installed_version": {
      "mode": "set",
      "version_name": "1.0.0",
      "version_code": 100,
      "path": "files/androidforge/installed-version.txt"
    }
  },
  "steps": [
    { "name": "seed", "phase": "pre-test",
      "action": { "type": "apply_state", "package": "com.example.app" } },
    { "name": "clean", "phase": "cleanup",
      "action": { "type": "reset_state", "package": "com.example.app" } }
  ]
}
```

The history writer receives `ANDROIDFORGE_FIXTURE_HISTORY_COUNT`,
`ANDROIDFORGE_FIXTURE_HISTORY_OUTPUT` and `ANDROIDFORGE_FIXTURE_HISTORY_FILE` as environment
variables; it writes the payload at the output path.

```powershell
androidforge schema show fixture.schema.json --json
androidforge fixture list --json
androidforge fixture validate update-state --json
androidforge fixture run update-state --phase pre-test --session <ID> --yes --dry-run --json
androidforge fixture run update-state --phase pre-test --session <ID> --yes --json
androidforge app data export com.example.app .\before-clean.tar --session <ID> --json
androidforge fixture run update-state --phase cleanup --session <ID> --yes --json
```

**Expected evidence/artifacts.** `fixture validate` returns `{valid, name, errors}`; `fixture run`
returns step statuses, warnings and artifact refs. `apply_state` writes
`fixtures/state/NNN-prefs.txt`, `fixtures/state/NNN-history.bin` and `fixtures/state/NNN-version.txt`
as local copies in the run, plus writer stdout/stderr under `logs/`. `installed_version.mode = "set"`
stamps `version_name=` / `version_code=` lines into a fixture-owned file; it does **not** change the
package manager. Use `mode = "assert"` when you want the package-manager version checked instead.
`app data export` gives a recoverable pre-change archive.

**Failure interpretation.** `E_FIXTURE_INVALID` (8) means the manifest or a path is invalid (no
absolute paths, `..`, backslashes or control characters; max 1000 history items).
`E_APP_NOT_DEBUGGABLE` (6) means `run-as` cannot write the state — use a debug build.
`E_FIXTURE_FAILED` (6) means a step or the user-supplied writer failed; read the run's stdout/stderr
artifacts. `E_PARTIAL_FAILURE` (10) means `on_failure: continue` steps failed; inspect the report.
Always preview with `--dry-run` before a destructive `reset_state`.

**Safety.** Declarative state is debug-only and data-changing: physical devices require
`--allow-physical-data` or `--allow-physical-mutation`; emulators do not. `--yes` is required for
destructive reset/cleanup steps. Fixtures are explicit, never automatic. Keep writer scripts and
payloads inside the project; do not use `adb shell` or raw adb to push fixture files.

---

## 3. Updater popup and installed-version scenarios

**Goal.** Reproduce the in-app updater popup for a newer release, an offline check, a bad digest and
a missing APK, and confirm the installed-version assertion shown to the user.

**Prerequisites.** A session, the app configured to query the mock endpoint, and a loopback mock.
The shipped route form works today; the named scenarios are a 1.1.0 surface.

Routes JSON (shipped `network mock serve --routes` form):

```json
{"routes":[
  {"method":"GET","path":"/releases/latest","status":200,
   "headers":{"content-type":"application/json"},
   "body":"{\"version\":\"9.9.9\",\"artifact\":\"/download/app.apk\"}"},
  {"method":"GET","path":"/download/app.apk","status":404,
   "headers":{"content-type":"application/json"},"body":"{\"error\":\"artifact unavailable\"}"}
]}
```

```powershell
androidforge network reverse add tcp:8080 --session <ID> --json
androidforge network mock serve --session <ID> --port 8080 --routes .\update-routes.json --reverse --max-requests 8 --duration 30s --json
androidforge network mock serve --session <ID> --scenario newer-release --json
androidforge network mock serve --session <ID> --scenario offline --json
androidforge network mock serve --session <ID> --scenario bad-digest --json
androidforge network mock serve --session <ID> --scenario no-APK --json
androidforge ui wait --session <ID> --text "Update available" --timeout 15 --json
androidforge ui find --session <ID> --test-tag update_dialog --json
androidforge capture screenshot --session <ID> --label update-popup --json
androidforge app verify-installed --apk .\app-debug.apk --session <ID> --json
# Cleanup applies only to the raw --routes + --reverse run above; named scenarios reject --reverse.
androidforge network reverse clear --session <ID> --json
```

**Expected evidence/artifacts.** The mock writes a bounded, redacted NDJSON request log in the run
(method, path, redacted headers, body preview, SHA-256, status, matched route); for the raw
`--routes` form with `--session`, `--reverse` records the mapping for cleanup. Scenarios carry
their own loopback bounds and reject `--reverse`. The popup is evidenced by a `ui find` match and a
labelled screenshot artifact. `app verify-installed` reports installed vs candidate package,
version, certificate and split hashes without installing. Scenario payloads are fixture JSON under
`.androidforge/network-mock/`; see `androidforge schema show network-mock-scenario.schema.json --json`.

**Failure interpretation.** `E_SELECTOR_NOT_FOUND` (7) means the popup did not appear — check the
request log for the manifest request and confirm the app's endpoint. `--test-tag` only resolves when
the debug app exposes `testTagsAsResourceId`; otherwise use text/resource-id selectors.
`E_UI_ASSERTION_FAILED` (7)
means the dialog text/state differs from the expectation. `E_INSTALL_VERIFICATION_FAILED` (6) or
`E_INSTALL_SIGNATURE_MISMATCH` (6) from `app verify-installed` means the installed build is not the
candidate; AndroidForge never auto-uninstalls. Unknown `--scenario` names are `E_INVALID_ARGUMENT` (2)
with the four canonical names. `E_DEVICE_HEALTH_UNAVAILABLE`/`E_DEVICE_OFFLINE` (4) means the
reverse mapping or session target is wrong.

**Safety.** Loopback bind is the default; never pass `--allow-external-bind` (the mock has no
authentication). `adb reverse` and network state changes require `--allow-physical-network` on a
physical device, or `--allow-physical-mutation`; only the raw `--routes` run adds a reverse mapping, so
clean it up with `network reverse clear` / `network restore`. Named scenarios never add one.
`--scenario offline` creates no listener and does not change host network settings. Installed-version
`mode = "set"` is a debug-only data fixture. No raw adb.

---

## 4. OEM install-confirmation dialog handling

**Goal.** Install an APK on an OEM device that shows an on-device confirmation prompt, without
blindly tapping unknown buttons.

**Prerequisites.** A session, an APK, and an explicit decision about which localized confirm label
is acceptable. The opt-in install flags are a 1.1.0 surface; the manual UI fallback uses shipped
commands.

```powershell
androidforge apk inspect .\app-release.apk --json
androidforge app install .\app-release.apk --session <ID> --expected-package com.example.app --json
# On E_INSTALL_FAILED whose text indicates an OEM confirmation:
androidforge ui dump --session <ID> --json
androidforge ui find --session <ID> --text "Install" --json
androidforge ui tap --session <ID> --text "Install" --json
androidforge app status com.example.app --session <ID> --json
androidforge app install .\app-release.apk --session <ID> --wait-for-confirmation --confirm-text "Install" --confirm-text "安装" --json
```

**Expected evidence/artifacts.** The failure envelope for `E_INSTALL_FAILED` (6) carries installer
output; `ui dump`/`ui find` capture the dialog hierarchy and candidate buttons. A handled flow
returns an `oem_confirmation` result with `outcome` (`confirmed`/`skipped`/`disabled`/`timeout`),
`reason`, `label`, tap coordinates, elapsed time and attempt count, and the caller re-runs the
install once. `app status` shows the resulting installed version. Screenshots captured with
`ui tap --capture-before/--capture-after` are optional evidence.

**Failure interpretation.** A trigger that is not an OEM prompt returns `reason =
"confirmation_not_matched"`; a vanished or auto-denied dialog returns
`"confirmation_dialog_timeout"` or an auto-denial reason. Multiple matching buttons are rejected
as `E_SELECTOR_AMBIGUOUS` (7) — narrow with `--confirm-text` or `--index`, never tap an arbitrary
candidate. `E_INSTALL_SIGNATURE_MISMATCH` (6) is not a confirmation problem; AndroidForge never
auto-uninstalls. A non-zero exit after the tap means re-check `app status` rather than retrying
indefinitely.

**Safety.** No automatic tap without an explicit opt-in flag. On a physical device, input needs
`--allow-physical-input` and install needs `--allow-physical-install` (or
`--allow-physical-mutation`); approval must name the exact operation. Read the dialog label before
tapping — never tap `Cancel`, `Uninstall`, permission or OEM "deny" controls. Do not script this
with raw adb input.

---

## 5. Compose frame / jank verification (`diagnostics frames`)

**Goal.** Collect a bounded gfxinfo frame sample, optionally compare it with a saved baseline, and
get an explicit pass/regression/partial signal.

**Prerequisites.** A session and a screen that stays on during the sample. For Compose-specific
assertions, the app must expose semantics through accessibility; test tags require a debug build
with `testTagsAsResourceId = true`.

```powershell
androidforge diagnostics frames com.example.app --session <ID> --dry-run --json
androidforge diagnostics frames com.example.app --session <ID> --reset --duration 5s --json
androidforge diagnostics frames com.example.app --session <ID> --reset --duration 5s --baseline .\runs\old\frames\report.json --max-janky 2 --max-p99 5 --max-frame-ms 10 --json
androidforge diagnostics performance com.example.app --cold --settle 2s --session <ID> --json
androidforge schema show frames.schema.json --json
```

**Expected evidence/artifacts.** `frames/report.json` in the run: frame counts, janky/legacy
counters, p50/p90/p95/p99, max frame time, a bounded histogram, a verbatim header excerpt,
`sample_status`, warnings, and — with `--baseline` — structured deltas with `comparable`,
`regression` and a reason. Counters the device does not report are `null`, never `0.0`. A comparable
regression exits **11** (`E_PERFORMANCE_REGRESSION`); partial or non-comparable evidence exits **10**
with warnings; clean runs exit 0. `diagnostics performance` complements this with startup/memory/CPU
evidence under `performance/report.json`.

**Failure interpretation.** Exit 11 means one of the allowed increases
(`--max-janky` percentage points, `--max-p99` ms, `--max-frame-ms`) was exceeded; inspect the delta
entries, not just the exit code. Exit 10 with `janky_percent: null` means an insufficient or
incomparable sample — use `--reset --duration` and keep the app in the measured state. `E_FILE_NOT_FOUND`
(6) means the baseline path is wrong; a non-frame-report baseline is `E_INVALID_ARGUMENT` (2).
`E_SCREEN_OFF` (4) from `diagnostics performance` means wake explicitly with `--wake`; never assume
implicit wake. Unknown metrics stay `null` with a reason.

**Safety.** `--reset` clears gfxinfo counters on the device; only use it on a session you own.
Frame collection is read-only apart from that reset. `--duration` bounds the sample; `--dry-run`
needs no device and changes nothing. Do not substitute raw `adb shell dumpsys gfxinfo`; the command
keeps raw evidence, nulls and exit semantics. No physical mutation flag is added for frames.

---

## 6. Memory-aware emulator / AVD capacity checks

**Goal.** Decide whether the host can start another AVD before launching it, and keep unknown
capacity values explicitly unknown rather than zero.

**Prerequisites.** Android SDK/emulator tooling discoverable by `androidforge doctor --json`; an
existing AVD for `avd list --json`. Capacity estimates are read-only and conservative.

```powershell
androidforge doctor --json
androidforge avd list --json
androidforge emulator status --serial emulator-5554 --json
androidforge devices health --serial emulator-5554 --json
androidforge doctor --capacity --json
androidforge avd images --json
androidforge avd create <NAME> --image "system-images;android-35;google_apis;x86_64" --device pixel_7 --yes --json
androidforge emulator start --avd <NAME> --port 5560 --memory 2048 --headless --prepare --json
```

**Expected evidence/artifacts.** `doctor --json` reports host/SDK/ADB/emulator/disk state;
`avd list --json` exposes per-AVD `ram_mb` when the AVD config declares it; `devices health --json`
is a read-only device memory/storage/thermal report with explicit `unknown_reason` fields. The
1.1.0 capacity surface adds host RAM/commit/disk readings with sources and a per-AVD
`capacity_warning` in `emulator status`, plus image catalog and `avd create` results. Absent values
are `null`/omitted with a reason, not estimated. `emulator start` records the chosen port and
session ownership.

**Failure interpretation.** `E_SDK_NOT_FOUND`/`E_ADB_NOT_FOUND` (3) means tooling discovery failed;
fix the SDK path before judging capacity. `E_EMULATOR_NOT_FOUND` (4) means the AVD/process is absent.
`E_DEVICE_HEALTH_UNAVAILABLE` (4) means health collection failed — do not infer memory from a partial
report. `E_DEVICE_BUSY` (4) means another session owns the target. A missing `ram_mb` or a capacity
reading marked `unknown` suppresses the warning by design; it is not proof that capacity is safe.
A collision on `--port` is reported before launch, not after a boot timeout (`E_EMULATOR_BOOT_TIMEOUT`, 5).

**Safety.** Capacity checks are read-only. `avd create` is a host-side mutation with an explicit
`--yes`; `emulator start` consumes host RAM and must use a free even console port and an owned
session. Never start or stop an emulator owned by another session; `session release`/`stop` need the
matching owner. No physical-device mutation and no raw adb are needed for this recipe.
