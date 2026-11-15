# Verification

This document records what was actually run for the `0.1.0` delivery on 2026-11-15. It separates verified behavior from implemented-but-not-exercised and unsupported/deferred work.

## Host and toolchain

- Host: Windows (10.0.26300), x86_64.
- Rust: stable Edition 2024 workspace, `rust-toolchain.toml` pinned; `rustc 1.100.0` toolchain (the exact compiler output is in the transcript below the build).
- Android SDK: `%LOCALAPPDATA%\Android\Sdk` on Windows.
- ADB: 1.0.41.
- Emulator: 37.1.11.0 (build 15917651), WHPX acceleration.
- Build tools: 36.0.0 (`aapt2` 2.20-13193326, `apksigner`).
- Java: OpenJDK 17.0.20.1 (SDK bundled JDK).
- Maestro: not installed.
- Device: AVD `androidforge_test`, Android 16 / API 36, x86_64, 1080x2400, headless on console port 5560 (ADB `emulator-5560`).
- Final verification session: `ses-020b98fd05ca`, owner `FinalReview2`, started with `--prepare`.
  Earlier delivery sessions `ses-65dc10884032`, `ses-67817710ae9e` and `ses-ee3682e48390` were also used.

A pre-existing `ip_shots` emulator on `emulator-5556` was left untouched throughout. The dedicated `androidforge_test` AVD was left in place (stopped) for reproduction; remove it with the SDK AVD manager if it is not wanted.

## Build and static checks

```text
cargo fmt --all -- --check                         pass
cargo clippy --workspace --all-targets --all-features -- -D warnings   pass
cargo test --workspace                             115 tests, 0 failed
cargo build --workspace --release                  pass
```

The 115 tests include unit, integration, process-runner, CLI, and genericity tests. The genericity test scans runtime crate sources for app names, package ids and fixture tokens. New tests cover `run end` status derivation, active-pointer cleanup, session owner guards, Android 16 jobs/alarm formats, the full notification-channel catalog, hint/placeholder clearing, conformance flow checks and port 5540.

Release binary:

- `target\release\androidforge.exe`
- size `9,282,048` bytes
- SHA-256 `9832C11B86C5BD213EE6CED2A5A3963B1A04F980938443FC470BD5CB2AAE4F35`

## CLI smoke checks

All run with the release binary:

| Command | Result |
| --- | --- |
| `--version` | `androidforge 0.1.0` |
| `--help` | exit 0 |
| `doctor --json` | exit 0; required tools found; disk free space measured; warnings for missing Maestro and (at the repo root) no project adapter |
| `capabilities --json` | exit 0; `runtime_verified: false` |
| `devices list --json` | exit 0; saw the test emulator only |
| unknown command `--json` | exit 2, `E_INVALID_COMMAND` with a suggestion; stdout contained only JSON |
| `config validate --json` for all three examples | exit 0, valid |
| `schema list --json` | exit 0, five schemas |
| `avd clone ... --json` | exit 3, `E_UNSUPPORTED_OPERATION` with reason and alternatives |
| `apk inspect` on three real APKs | exit 0, package/version/SDK/hash/certificate |
| `report import junit` | exit 0, parsed 1 suite / 2 cases; a two-file directory import parsed 2 files / 2 cases |
| `report list --json` | exit 0, discovered runs without a known run id |
| `run end` with a failed step | exit 6, report `failed`, `E_COMMAND_FAILED`; explicit `--status passed` refused with exit 2 |
| `session start ... --json` piped to a file | returned in 24.8s with a 2,579-byte JSON envelope while the emulator stayed running |
| `session stop` from `ANDROIDFORGE_OWNER=IntruderAgent` | exit 4, `E_SESSION_OWNERSHIP_MISMATCH`, session owner and caller owner in context; emulator untouched |
| `session release` then `session stop --owner <owner>` | release left the emulator running; owner-matched stop on the released tombstone stopped it |
| `session list --active` | exit 0, hid released/failed tombstones |
| `run end --run <ID>` then `capture screenshot` | exit 0; screenshot created a new run, not an append to the finished one |
| `ui clear` with no focused editable field | exit 7, `E_SELECTOR_NOT_FOUND` with a focused-node explanation |
| `ui clear` on Android Settings search after typing `wifi` | exit 0, `cleared: true`, `verified: true`, hint `Search settings` treated as empty |
| `report show`/`path` for a project-local run from the repo root | exit 0, found the run under `examples/<App>/.artifacts/runs` without `--project` |
| conformance result and report | result carried build identity and parsed `flow_checks`; report step details carried the same checks |
| wrong `--expected-cert-sha256` inside a run | exit 7, 0 artifacts, 5,323 bytes on disk (no screenshot/logcat/dumpsys bundle) |
| `app install --dry-run` | exit 0, reported APK package/version/hash/certificate and planned constraints |
| `app clear-data --dry-run` | exit 0 without `--yes`, reported package and required confirmation |
| `app install --expected-cert-sha256` with a wrong value | exit 7, `E_SIGNATURE_VERIFICATION_FAILED` with the actual fingerprint |
| `--json` appended to raw `adb` | moved before `adb`; JSON envelope returned |
| `capture screenshot` | result includes `path`, dimensions and checksum |
| `doctor --json` after cleanup | exit 0, `degraded` for missing Maestro and no root adapter, 0 sessions, disk `ok` with 1.0 TiB free |

Warm startup measurements on this host (single run, not a benchmark):

| Command | Wall time |
| --- | ---: |
| `--version` | 15 ms |
| `--help` | 11 ms |
| `capabilities --json` | 92 ms |
| `doctor --json` | 401 ms |
| `devices list --json` | 371 ms |

The device commands include ADB work, so they are not pure process startup.

## Real app runs

### CalcSuite — first dogfood, current 3.0.3 updater

- APK: `CalculatorSuite-v3.0.3.apk` (release).
- Install run `20261115-165050-0dc26b94_app-install`: package `com.calcsuite`, versionName `3.0.3`, versionCode `7`, local and installed APK SHA-256 `ccc2d570010409ebaa2277ac65ba8f78008a84e10b719046b6b488362868fe6a`, certificate SHA-256 `55071A2393348B9C2F44DDE98D21277F1666A56040DECD51C0B83F377A6F9E02`, `debug_signed: false`, `verified: true`.
- Fixture run: force-stopped/launched the app, asserted `Calc Suite` and `Categories`, captured the category screenshot.
- Conformance run `20261115-165054-dc1b9d28_conformance-run` with the release binary: status `passed`, exit 0. It opened Settings, scrolled to `Check for updates`, tapped `Check`, waited for and asserted `You have the newest version (3.0.3).`, and captured a screenshot.
- The conformance report inherits the install's full build identity.
- Evidence: [case-studies/CalcSuite-smoke](case-studies/CalcSuite-smoke/).

### DiscountHunter — genericity and the notification case study

- APK: debug `app-debug.apk`, package `com.discounthunter.android.debug`, versionName `1.1.2-release-debug`, versionCode `10`.
- Install run `20261115-165135-e118856a_app-install`: local and installed APK SHA-256 `6e7a01bebcef656f118061fa85c0e7a145ac52677070d28b124077c0f16cc996`, debug certificate SHA-256 `C1FD411DC1DA096FD12001EC5B71DA212C91E13442BA289D2C8F36A72F0ED691`, `debug_signed: true`, `verified: true`.
- App data was cleared, so conformance `20261115-165138-b1a909b8_conformance-run` exercised the real intro and first-run path.
- Fixture: granted `POST_NOTIFICATIONS`, pushed a valid `files/alerts/watch-state.json` through `run-as`, launched, asserted the app shell.
- Conformance: navigated Settings > Deal alerts with semantic selectors and `scroll-to`, checked notification permission, sent `CHECK_DEALS` as the app UID through the `run_as` hook, required the app's `status.json` `checkedAt` to change, waited for the app's check result, captured a screenshot and collected diagnostics. It also returned both `discount_hunter_free` and `discount_hunter_sales` channels and six parsed `flow_checks`.
- The current run found nothing new to announce; the retained `notification-run` (`20261115-131907-b3ca2d45_conformance-run`) records `posted 1 notification(s)` and is explained as historical notification evidence.
- Status `passed`, exit 0. See [case-studies/DiscountHunter-smoke](case-studies/DiscountHunter-smoke/).

### IronPulse — third app, first-run, liveness, recording

- Release APK 4.4.0 install run `20261115-165451-63437e87_app-install`: package `com.aistudio.fitness.ironpulse`, versionCode `12`, APK SHA-256 `58e0ad70496080454ee62af51d0e0b53bbf2c892255b8c9d0cb065e7c4fd3464`, release certificate SHA-256 `46893F8E6C463E01B7B849AEF18098331E4C162325E906D86762F16A6430BB2C`, `verified: true`.
- App data was cleared; conformance run `20261115-165457-2029a77c_conformance-run` used the release binary and completed the real first-run onboarding (Skip, profile fields, Start training), then Home. The report carries the inherited full build identity and eight parsed `flow_checks`.
- Verified: foreground process/task/resumed state, `CURRENT STREAK` home surface, `focus-streak` and `open-profile` widget action intents, Settings surface, screenshot, 2-second screen recording and diagnostics app collection.
- Status `passed`, exit 0. See [case-studies/IronPulse-smoke](case-studies/IronPulse-smoke/).

### App-private file round-trip

Debug build 4.3.0 (`3233645e08f8ec3d4a5f63edfe1e12d4e749d66c481218330496eb16f308c8a9`, debug cert) was installed, then fixture `debug-file-roundtrip` run `20261115-123512-d4b90e19_fixture-run` pushed a JSON marker to `files/androidforge-fixture-marker.json`, pulled it back and matched the bytes. Release builds are not debuggable and correctly fail this path with `E_APP_NOT_DEBUGGABLE`.

### Expected certificate enforcement

A temporary adapter declared an all-zero `expected_cert_sha256`. Installing the real CalcSuite APK
failed with `E_SIGNATURE_VERIFICATION_FAILED` (exit 7) and reported the actual certificate
`55071A2393348B9C2F44DDE98D21277F1666A56040DECD51C0B83F377A6F9E02`. The normal CalcSuite adapter
then installed the same APK successfully with `verified: true`. This proves the adapter's declared
certificate is enforced, not merely displayed.

### Signature mismatch is non-destructive

With the debug build installed, installing the release 4.4.0 APK produced `E_INSTALL_SIGNATURE_MISMATCH` (exit 6). The error listed incoming and installed certificate SHA-256 values and installed version `4.3.0`; the app remained installed. The local evidence was inspected but raw run files are not published.

## Evidence integrity

During verification, every copied case-study run was checked against its `manifest.json`: all listed files existed and every SHA-256 matched. The public repository intentionally withholds raw run directories because they can contain machine paths, device logs and app data. The case-study READMEs record the run ids, build identities and results.

## Issues found and fixed during verification

These were found by real runs, not only by compilation:

1. **`ui find` empty results.** `ui find` intentionally succeeds with `matches: []`; two example scripts treated exit 0 as found. They now inspect `matches`. This is documented in the user guide.
2. **Debug package expectation.** Installing a debug APK through an adapter that also declared a release package failed because only `package` was expected. `InstallOptions` now accepts both configured package ids; explicit `--expected-package` still wins.
3. **Recording stop finalized a caller's run.** Stopping a recording that began inside a conformance run marked the whole run passed, so later steps failed with "already Passed." Recording state now distinguishes an implicitly created recording run from a caller-owned run.
4. **Fixture templates with nested `${PROJECT_ROOT}`.** A fixture variable containing `${PROJECT_ROOT}` was left unresolved. Template expansion now iterates to a fixpoint.
5. **Android 16 `pm check-permission` removal.** Permission checks now fall back to `dumpsys package` without a false failure note, and short names like `POST_NOTIFICATIONS` are normalized to `android.permission.*`.
6. **Run build identity.** A run that had not installed an app copied the adapter's expected certificate into the observed `build.cert_sha256` field and used the release package even when a debug package was configured. It now reports the declared debug/release target without inventing observed certificate data; an install overrides it with measured identity.
7. **IronPulse 4.3.0 widget intent crash.** A debug 4.3.0 attempt with the app still on its intro crashed inside `NavController.getGraph()` before `setGraph()`. The tool captured the crash, process death and ANR evidence. Release 4.4.0 passed the same flow. This is an app-version defect, recorded rather than hidden.
8. **Test expectation for process timeout.** The runner returns bounded stdout/stderr with `timed_out=true` for evidence; the test now checks `require_success()` for the timeout error rather than expecting `run()` itself to return `Err`.
9. **Captured `session start` hang.** The detached emulator could inherit the caller's stdout/stderr pipe. `spawn_detached` now clears the inherit bit on standard handles around `CreateProcess`; captured start returns in 28.9s while the emulator keeps running. The captured-start smoke check is listed above.
10. **`run end` false green.** Status was fixed to `passed` by default even with failed steps. It is now derived, explicit `passed` overrides require `--allow-failures`, and failed/partial runs exit non-zero.
11. **`ui clear` false failure.** Empty text is represented as absent by UIAutomator; clear now compares `None` to the empty expected string and verifies correctly.
12. **Hook false positive.** A plain shell broadcast to a non-exported receiver can return `result=0` without delivery. Hooks now support `run_as` and `verify`; the DiscountHunter hook requires the app's `status.json` `checkedAt` to change. The old plain broadcast failed the new postcondition and was replaced with an app-UID broadcast.
13. **Build identity not inherited.** Installs persist measured identity in the session; later runs and conformance reports inherit it. Identity is ignored when it belongs to a different configured package, then refreshed from the installed app.
14. **Empty Android 16 parsers.** Notification channels are read from the full `AppSettings` catalog; jobs parse named/WorkManager headers and `Source: pkg=` lines; alarms parse `origWhen` and `Alarm{...}` forms.
15. **Discovery and preview gaps.** Added `report list`, multi-file/directory JUnit import, `session list --active`, meaningful `app install --dry-run`, `--expected-cert-sha256`, `capture screenshot.result.path`, and a released-tombstone stop path.
16. **Raw ADB global flag ordering.** A trailing `--json` is moved before `adb`; remaining interior occurrences get a clear ordering error.
17. **CalcSuite case study was stale.** Rerun against the current 3.0.3 release and the app's real updater assertion.
18. **Invalid default expectations.** The process timeout test expected `run()` to return `Err`; the API intentionally returns bounded output with `timed_out`, and `require_success()` was the correct assertion.
19. **`run end --run` left the active pointer.** Ending the session's active run by explicit id finalized the report but left the pointer, so the next command failed with "already Passed". `run end` now clears the pointer only when it names that run; stale pointers to finished runs are ignored and cleared by later commands.
20. **Destructive session commands accepted any owner.** `session stop`, `session release` and `session restart` could target another agent's session id. They now require the recorded owner label (or `ANDROIDFORGE_OWNER`) and return `E_SESSION_OWNERSHIP_MISMATCH`; `--force` is the explicit takeover. This was reproduced by stopping another agent's session before the fix.
21. **`ui clear` placeholder text.** Android Settings exposes the hint `Search settings` as the field text after clearing, so clear failed with `E_TEXT_VERIFICATION_FAILED`; text entry also appended to the hint. UIAutomator `hint`/`hint-text` is now parsed and an exact hint is treated as empty before typing and after clearing. Clear with no selector reports `E_SELECTOR_NOT_FOUND` when nothing editable is focused.
22. **Project-local reports were undiscoverable.** `report show`/`path` only searched the current artifacts directory. They now fall back to project-local `.artifacts/runs` directories under the project tree, with ambiguity checking.
23. **Conformance identity and flow checks were missing from the result.** The command result had `build: null` and only exit/status checks, even though the report carried identity and the script printed named checks. The result now includes the measured build identity and `flow_checks`; step details carry the same checks.
24. **Preflight install failures produced heavy evidence.** A local expected-certificate mismatch wrote screenshot, hierarchy, logcat and dumpsys bundles even though the device was untouched. Preflight-tagged errors skip that bundle; the run directory for the check above was 5,323 bytes with zero artifacts.
25. **Transient `am start` failure after install.** One launch immediately after install exited 6 on an ADB 255/launcher-bootstrap race. `app launch` now retries `am start` once after 700 ms and reports the retry as a warning; a persistent failure still fails with both outputs.
26. **Human error output embedded whole reports.** Human mode printed the full report JSON (including artifact arrays) in error context. It now summarizes run status/failed steps, JUnit counts and external exit/timing; JSON mode still carries the full object.
27. **Active-run pointer collision without a session.** The global pointer used the key `default` for every project. Without a session it now includes a stable project-root hash, so two projects cannot overwrite each other's active run.

## Final state

- No AndroidForge sessions or active-run pointers remain.
- No AndroidForge-owned emulator processes remain; the pre-existing `emulator-5556` (`ip_shots`) was untouched.
- Root and example `.artifacts` directories were removed after the case-study copies were checksum-verified.
- The `androidforge_test` AVD remains stopped for reproduction.

## Implemented but not exercised in this delivery

- Physical Android devices, USB authorization and reconnect behavior.
- Maestro execution (Maestro is not installed; the capability is reported as unsupported).
- A real instrumentation test APK and `test instrumentation` output.
- Snapshot load/restore (snapshot list/save/load/delete are implemented; load remains best-effort and `--yes`-gated).
- Time changes requiring root (`time get` is safe; `time set` is implemented but was not run without root).
- Vendor-specific MIUI/Doze/notification behavior.
- Parallel multi-agent/multi-device stress; concurrent commands on the same device interfere by design.
- AVD cloning and named-profile isolation (unsupported, not faked).

## Known limitations

- Notification, JobScheduler, AlarmManager and vendor dumpsys formats vary. Parsed fields are labelled best-effort; raw evidence is preserved.
- Compose internal test tags are not visible through UIAutomator unless the app exposes them as accessibility resource ids. Use instrumentation for tag-level assertions.
- Raw ADB is explicitly unsafe and can use or modify state outside a session.
- Session locks coordinate AndroidForge processes only, not external ADB/IDE users. The owner label on `session stop|release|restart` prevents accidents between agents; it is not authentication, and `--force` deliberately bypasses it.
- Active runs are scoped to a project. Start a run with the same `--project` used by its steps; a root-level `run start` followed by `--project examples/App` creates a separate project-local implicit run.
- Reports contain absolute paths from the machine that generated them. Raw case-study reports are withheld from the public repository for that reason; use relative paths in new automation.
- No HTML report, visual diff, cloud execution, daemon, signing automation or release publishing.

## Reproduce

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --workspace --release

$exe = ".\target\release\androidforge.exe"
& $exe doctor --json
& $exe session start --avd androidforge_test --port 5560 --headless --owner ReproAgent --prepare --json
& $exe --project .\examples\CalcSuite fixture run smoke --session <ID> --json
& $exe --project .\examples\CalcSuite conformance run --session <ID> --json
& $exe --project .\examples\DiscountHunter fixture run smoke --session <ID> --json
& $exe --project .\examples\DiscountHunter conformance run --session <ID> --json
& $exe --project .\examples\IronPulse conformance run --session <ID> --json
& $exe session stop --session <ID> --owner ReproAgent --json
```

The installed APK builds and package identities are project inputs, not part of this repository. Install the matching APK before each flow. Do not run flows against the same device concurrently.
