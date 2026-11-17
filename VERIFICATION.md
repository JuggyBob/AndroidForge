# Verification record

This document records what was actually run for the `0.6.0` Alpha delivery, followed by the `0.5.0`,
`0.4.0` and `0.3.0` records. It separates automated checks, real Android validation, desktop workflow
validation and packaging validation from areas that were not exercised.

## v0.6.0 verification (2026-11-17)

### Scope

Agent Memory: action receipts, failure recurrence memory, APK/build resolution and comparison,
diagnostic bundles, and desktop Context/Ownership/Triage integration. The starting v0.5.0 state is
preserved by the pre-v0.6.0 backup at
`A:\Projects\Archive\Backups\AndroidForge\20261117-071338`.

### Automated checks

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Passed. |
| `cargo test --workspace` | 219 passed, 0 failed across 25 test executables. |
| `cargo build --workspace --release` | Passed. |
| New unit tests | 10 recurrence, 7 receipt classification, 6 bundle/archive safety, 4 APK glob resolution, 4 APK diff, 2 desktop model/context, plus CLI integration coverage for recurrence/history/explain/bundle and malformed APK resolution. |
| Compatibility tests | Existing v0.5.0 CLI tests, desktop backend tests and workspace tests all passed. Plain UI actions without `--receipt` still return their previous shape. |

### Real Android validation

Environment: Android 16 / API 36 emulator `emulator-5554`, session `ses-a4465e206322`, owner
`v06-verify`, CalcSuite installed (`com.calcsuite` 3.0.3 / versionCode 7).

- `ui tap --content-desc Settings --receipt` returned `effect=content_changed`,
  `completeness=complete`, distinct before/after tokens and `added=26 removed=25`.
- `ui text "1+2" --wait-for-value "1+2" --receipt` returned `postcondition.result=satisfied` and
  `completeness=complete`.
- `ui text "7" --wait-for-value never-this-value --timeout 1s --receipt` exited 7 with
  `E_POSTCONDITION_TIMEOUT`, `dispatched=true`, `postcondition.result=timed_out`,
  `completeness=incomplete` and an after token.
- A missing selector with `--receipt` exited 7 with `E_SELECTOR_NOT_FOUND`, `dispatched=false`,
  `completeness=incomplete`, `effect=unknown` and a before token.
- The same tap without `--receipt` returned the legacy keys only (`matched`, `tap_x`, `tap_y`,
  `used_clickable_ancestor`, `display_size`, `rotation`) with no `receipt` field.
- `apk diff installed:com.calcsuite <pulled installed base.apk>` reported `overall=same`,
  `safe_to_skip_install=true`, with package/version/hash/certificate/SDK/activity/split fields all
  `same`.
- `app install <pulled installed base.apk> --only-if-changed` returned `skipped=true`,
  `decision=skipped`, `diff=same` and the documented reason.
- `app install <DiscountHunter release APK> --only-if-changed --expected-package ...` installed with
  `decision=installed`, `reason=not_installed`; the test package was explicitly uninstalled afterwards.
- `report bundle` on the real failed run `20261117-065835-a65d76ff_ui-tap` produced a 24,403-byte ZIP
  with 9 payloads, 10 explicit exclusions, `verified=true`; the ZIP reopened and manifest/
  summary/explanation/evidence/logcat entries were present.
- Recurrence integration tests covered new, recurring, fixed-then-regressed, unrelated success,
  non-exercising success, cross-project isolation, version changes and status/fingerprint filters.

### Desktop validation

- Desktop built and launched with `title=AndroidForge`, `responding=True`, and closed gracefully.
- The Context page rendered real project/session/device/app data from the CLI (`CalcSuite`,
  `ses-a4465e206322`, `emulator-5554`, `com.calcsuite` 3.0.3).
- The Ownership page rendered active sessions, owners, device ownership, stale active count and
  tombstones with copyable safe commands. It has no stop/release control.
- Triage rendered the deterministic explanation, recurrence panel and diagnostic bundle panel. Windows
  UI Automation invoked `Preview inclusion` against the real CLI; the page displayed 9 payloads,
  16,405 bytes and 10 exclusions.
- `androidforge-desktop.exe --page <name>` was added to open a page directly for diagnostics; invalid
  page names exit 2.

### Packaging and publication (pre-publication)

- `.\\packaging\\package.ps1 -SkipBuild` produced:
  - `androidforge-cli-windows-x86_64.zip` (4,759,890 bytes), SHA-256
    `a275dff8d70eda0efcc29eb7493f5daa7dc33572b20f26a5d6a1ae6237ec0b7f`.
  - `androidforge-desktop-windows-x86_64.zip` (11,452,541 bytes), SHA-256
    `76d9fc1e66d5480792f1fce7f649151a08334c6a5079e65a37b3440dc2d2f1f8`.
  - `androidforge-windows-x86_64-setup.exe` (9,465,475 bytes), SHA-256
    `31c26e4569d4bb4e4128ed2849123f0036919af524a7eeb04192abbef082b5fc`.
- The portable ZIP was extracted into a clean directory; CLI and desktop reported `0.6.0`; new help
  pages loaded; the desktop launched with title `AndroidForge` and stayed responsive.
- The installer installed CLI/desktop `0.6.0` and uninstalled cleanly with the install directory removed.
- All three local asset SHA-256 values matched `SHA256SUMS.txt`. CI produces independent binaries and
  hashes; published-asset download results are recorded below.

### Published release

- Release: <https://github.com/JuggyBob/AndroidForge/releases/tag/v0.6.0>
- Title: `AndroidForge v0.6.0 - Agent Memory`; not a prerelease.
- Tag: `v0.6.0`, tag object `2066615dc4484748fea88f391cbc1d265541658b`, commit
  `26f837660d53e38575d50a2d28aafc6f832acce4`.
- CI Release workflow `44039413660` passed and uploaded the assets.
- The published assets were downloaded and matched the published `SHA256SUMS.txt`:
  - `androidforge-cli-windows-x86_64.zip` (4,758,822 bytes),
    `f072ab10174d6fb82e6b876a7bb29e1ff8cf647672e862b31d2b0f52c44b5381`.
  - `androidforge-desktop-windows-x86_64.zip` (11,452,997 bytes),
    `7c73ae7b1d3021bd6bf2b299ba8f6613bbda4d041c6f270b42cd4257f0c3371e`.
  - `androidforge-windows-x86_64-setup.exe` (9,464,168 bytes),
    `f2d5b90d579f4dc419cb149aef8918f15f31ed72e51150d952cdab05e35e91fa`.
- The downloaded portable ZIP reported CLI/desktop `0.6.0`; new bundle/APK help pages loaded; the
  desktop launched directly on Ownership with title `AndroidForge`.
- The downloaded installer installed CLI/desktop `0.6.0` and uninstalled cleanly with the install
  directory removed.

### Not exercised

- Physical Android devices, wireless ADB, Maestro, a real instrumentation package, snapshot restore,
  root-only time changes, vendor ROMs, and desktop layouts other than the tested Windows scale.
- Receipt classification for system permission dialogs on a non-Android-16 vendor ROM.
- Split APK installs: comparison correctly reports their identity as unknown and refuses to skip.
- Screenshot/recording pixel redaction when explicitly included in a bundle.

## v0.5.0 verification (2026-11-16)

### Scope

Three independent app agents reported `PASS WITH ISSUES` for IronPulse, DiscountHunter and CalcSuite.
AndroidForge v0.5.0 fixes every confirmed AndroidForge issue plus the app/conformance test-asset issues
inside the AndroidForge example adapters. Two claims were checked and not reproduced as stated:

- `session release --owner` already exists and parses. A CLI parser regression test now pins it. The
  ownership error text was made more specific.
- The shared-host `session ensure` failure was real but the cause was automatic port allocation choosing
  an occupied ADB emulator port before the device lock was checked.

### Automated checks

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Passed. |
| `cargo test --workspace` | Passed. |
| `cargo build --workspace --release` | Passed. |
| `androidforge --version` | `androidforge 0.5.0` after version bump. |

### Regression tests added

- Failed and blocked `agent test` envelope status.
- UIAutomator failure classification for the new code and legacy `E_COMMAND_FAILED`/exit 137 reports.
- Project-scoped `report explain latest` preferring a failed run and finding an unprojected run by id.
- Observation `actionable`/`context` separation, `token`, always-present `clickable`/`enabled` and
  password redaction.
- Compact diff truncation at 12 and `--full` at 100.
- `session release --owner` parsing.
- Envelope preservation of `status: "failed"` and nonzero exit code.

### Live checks

| Scenario | Result |
| --- | --- |
| Existing `emulator-5554` running, ensure `androidforge_test` | Launched `emulator-5556` in 27.9 s, not 5554. |
| `session start`/`attach` project binding | Newly attached session showed the CalcSuite project root; release with `--owner` succeeded. |
| First ensure after session creation | No legacy project-binding warning. |
| `ui observe` new shape | `token` equals `id`; `clickable`/`enabled` present; interactive and context lists separated; empty resource ids omitted. |
| `ui diff` compact | CalcSuite Settings-to-Home: 8.4 KB with 12/33 added and 12/34 removed truncated lists, versus ~21.6 KB before the compact change. |
| `report explain latest --project examples/CalcSuite` | Selected the failed run `20261116-211609-...`, not the newer passed run. |
| `report explain <unprojected-id> --project examples/CalcSuite` | Found and explained the root-artifact run. |
| `report explain` on DiscountHunter UIAutomator failure | Category `ui_automation`, high confidence, observe/diagnostics/session next actions. |
| `report list --all` | Merged project and ancestor stores. |
| `agent test --apk missing --dry-run` | Envelope `status: "failed"`, exit 6, `result.final_status: "failed"`. |
| Desktop `--version` / `--help` | Printed text and exited 0 without opening the GUI. |
| Desktop normal launch | Window titled `AndroidForge` remained responsive. |

### Packaging validation

- `.\packaging\package.ps1 -SkipBuild` produced:
  - `androidforge-cli-windows-x86_64.zip` (3,877,887 bytes), SHA-256
    `011e9fef8f57ae27a484214b547b0198031446901d882139aafa803943e5c9af`.
  - `androidforge-desktop-windows-x86_64.zip` (10,488,877 bytes), SHA-256
    `0464d36b57dc6eb2a7f7dc46a9fe0269789003ac7e08a8f5f5a61fb052ea4772`.
  - `androidforge-windows-x86_64-setup.exe` (8,978,868 bytes), SHA-256
    `2e0fbf6361ac2e37ab682c9080118153249abcf075f374f7bb0e505de8e99ef2`.
- The portable ZIP was extracted into a clean directory; CLI and desktop both reported `0.5.0`; the new
  `--port`, `ui observe` and `ui diff` help pages loaded; the desktop launched with title
  `AndroidForge` and stayed responsive.
- The installer ran silently, installed CLI/desktop `0.5.0`, and uninstalled cleanly while removing the
  install directory.
- All three packaged asset SHA-256 values matched `SHA256SUMS.txt`.

### Published release

- Release: <https://github.com/JuggyBob/AndroidForge/releases/tag/v0.5.0>
- Tag: `v0.5.0`, tag object `61bf9bd9afa180472130bfc6c94796f4f19aacfa`, commit
  `dccd6be733817287055a5e364b8d6525fab7a636`.
- The tag-triggered Release workflow (`43961438287`) passed and uploaded the CI-built assets.
- All three assets were downloaded from the published release and matched `SHA256SUMS.txt`.
- The downloaded portable ZIP reported CLI/desktop `0.5.0`; `--help`/`--version` were non-interactive;
  the desktop launched with title `AndroidForge`.
- The downloaded installer installed CLI/desktop `0.5.0` and uninstalled cleanly.

### Published release

- Release: <https://github.com/JuggyBob/AndroidForge/releases/tag/v0.6.0>
- Title: `AndroidForge v0.6.0 - Agent Memory`; not a prerelease.
- Tag: `v0.6.0`, tag object `2066615dc4484748fea88f391cbc1d265541658b`, commit
  `26f837660d53e38575d50a2d28aafc6f832acce4`.
- CI Release workflow `44039413660` passed and uploaded the assets.
- The published assets were downloaded and matched the published `SHA256SUMS.txt`:
  - `androidforge-cli-windows-x86_64.zip` (4,758,822 bytes),
    `f072ab10174d6fb82e6b876a7bb29e1ff8cf647672e862b31d2b0f52c44b5381`.
  - `androidforge-desktop-windows-x86_64.zip` (11,452,997 bytes),
    `7c73ae7b1d3021bd6bf2b299ba8f6613bbda4d041c6f270b42cd4257f0c3371e`.
  - `androidforge-windows-x86_64-setup.exe` (9,464,168 bytes),
    `f2d5b90d579f4dc419cb149aef8918f15f31ed72e51150d952cdab05e35e91fa`.
- The downloaded portable ZIP reported CLI/desktop `0.6.0`; new bundle/APK help pages loaded; the
  desktop launched directly on Ownership with title `AndroidForge`.
- The downloaded installer installed CLI/desktop `0.6.0` and uninstalled cleanly with the install
  directory removed.

### Not exercised

- UIAutomator helper recovery was not forced into exit 137; classification and cleanup paths are unit
  tested, and the live observation path passed.
- The full three-app `agent test` matrix was not repeated after every fix; existing v0.4.0 evidence was
  reused for the unaffected app flows, and the corrected report/observation/session paths were tested
  directly.
- Physical devices, Maestro, instrumentation packages, snapshot restore, root-only operations and
  multiple DPI scales remain unvalidated.

## v0.4.0 verification (2026-11-16)

### Environment

- Windows 11 x86_64 host, 2560x1440 display at 125% scaling.
- Android 16 / API 36 emulator, serial `emulator-5554`, AVD `dh36` (attached/reused for all three apps).
- AndroidForge CLI and desktop built from the same `0.4.0` workspace.
- Real targets:
  - CalcSuite `CalculatorSuite-v3.0.3.apk`, package `com.calcsuite`, version `3.0.3` (7), certificate
    `55071A2393348B9C2F44DDE98D21277F1666A56040DECD51C0B83F377A6F9E02`.
  - DiscountHunter debug APK, package `com.discounthunter.android.debug`, debug certificate
    `C1FD411DC1DA096FD12001EC5B71DA212C91E13442BA289D2C8F36A72F0ED691`.
  - IronPulse debug APK `5.0.0` (13), package `com.aistudio.fitness.ironpulse`, debug certificate
    `C1FD...D691`; release `5.0.1` used to exercise signature mismatch, release `4.4.0` used to exercise
    downgrade rejection.

### Automated checks

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Passed. |
| `cargo test --workspace` | Passed: core unit tests (including diagnosis categories, fingerprint stability, evidence filtering, lease behavior and session ownership), CLI unit/integration tests (context partial results, read-only repair plan, stable explain output, ensure dry-run, missing-session errors, dry-run pipeline, diff semantics, token scoping) and desktop tests. |
| `cargo build --workspace --release` | Passed. |
| `androidforge --version` | `androidforge 0.4.0`. |

### Real Android validation

| Scenario | Result |
| --- | --- |
| `session ensure --avd dh36 --owner AgentTest --ttl 2h` | Launched a ready session in 30.1 s. |
| Repeated identical ensure | Reused the same session in 1.05 s; mode `reused_session`. |
| Ensure as `OtherAgent` | Refused with `E_DEVICE_BUSY` and the owner/session/alternatives; no takeover. |
| Ensure after simulated expired lease with `--ttl 30m` | Renewed the lease and reported `expired_lease_renewed`. |
| Ensure after simulated expired lease without `--ttl` | Returned `ready: true`, `lease_expired: true`, `needs_reconciliation: true` and a warning; did not stop the device. |
| `context --session` cold/warm | ~3.0 s cold, ~0.68 s warm, ~17 KB JSON including app and screen; reused the recent observation. |
| `context --include project,environment` | ~0.34 s, ~2.8 KB, omitted sections in one compact call. |
| `ui observe` compact / `--full` | ~2.7 s; ~14 KB compact, ~56 KB full; foreground/focus/actionables correct. |
| `ui observe --since TOKEN` | No changes reported on an unchanged screen. |
| `ui diff --from TOKEN` | Correctly reported package/activity changes and semantic added/removed nodes when Chrome/Gmail replaced the launcher. |
| Reboot then `ui diff --from TOKEN` | Failed with `E_OBSERVATION_EXPIRED` (exit 4), proving boot-id invalidation. |
| Tap Chrome with `--wait-for-package com.android.chrome` | Action executed once; postcondition satisfied in 7.1 s. |
| Tap Gmail with impossible text and `--timeout 3` | Failed with `E_POSTCONDITION_TIMEOUT`, `action_executed: true`, foreground `com.google.android.gm`, observation token and screenshot/logcat/hierarchy evidence. |
| IronPulse update dialog: tap `Close` with `--wait-for-absent-text "Update available"` | Postcondition satisfied; no duplicate tap. |
| CalcSuite Settings: tap `Back` with `--wait-for-absent-text "Check for updates"` | Postcondition satisfied; diff showed the screen change. |
| `report explain latest` on a postcondition timeout | Classified `assertion` / `E_POSTCONDITION_TIMEOUT`, high confidence, 12 evidence refs, exact observe/screenshot/report commands, 125 ms. |
| `report explain` on an interrupted CalcSuite run | Classified `interrupted` / `E_INTERRUPTED`, high confidence, stable fingerprint. |
| `report explain` on IronPulse conformance failure | Selected the step's `E_UI_ASSERTION_FAILED` instead of the aggregate command exit, surfaced failed checks (`status, exit_code, artifact:recordings/*.mp4`) and produced an assertion diagnosis. |
| `report explain` repeated for the same run | Identical fingerprint. |
| `doctor fix --plan` with a valid SDK | Read-only, `mutations_performed: []`, optional Maestro/adapter issues only. |
| `doctor fix --plan` with invalid `ANDROID_HOME` | Detected the invalid path and kept the project files unchanged; no environment modification. |
| `agent test` CalcSuite with `--install --flow conformance` | Success in 39.6 s; install and conformance stages passed; 8 artifacts. |
| `agent test` CalcSuite again without `--install` | Success in 35.5 s; install skipped because the recorded SHA-256 matched (`proof: sha256`). |
| `agent test` DiscountHunter with `--install --flow conformance` | Success in 50.7 s; debug signature, install and conformance passed. |
| `agent test` IronPulse debug `5.0.0` with conformance | Reached conformance; the existing flow failed because `recordings/*.mp4` was not produced. The pipeline returned the assertion explanation and failed checks instead of a generic command error. |
| IronPulse release `5.0.1` on a debug-signed install | Failed with `E_INSTALL_SIGNATURE_MISMATCH`; explanation classified `signature`, referenced the candidate APK, and offered a destructive `--fresh --yes` action with a warning. No automatic uninstall occurred. |
| IronPulse release `4.4.0` over `5.0.0` | Failed with `E_INSTALL_FAILED` version downgrade; explanation offered `--downgrade`. |
| `agent test --dry-run` | Listed planned stages and `--install`/`--flow` confirmations; created no session or agent-test report. |

### Desktop validation

- `cargo check -p androidforge-desktop` passed after adding typed explanation, repair-plan and lease
  models.
- The release desktop was built with both executables from the same workspace. The new Triage, Setup and
  Sessions panels compile and use only CLI JSON results.
- The packaged portable desktop launched successfully; no screenshot-based visual review at multiple DPI
  scales was performed.

### Packaging validation

- `.\packaging\package.ps1 -SkipBuild` produced:
  - `androidforge-cli-windows-x86_64.zip` (3,836,839 bytes), SHA-256
    `261aa5b5b0658ecef19fb9883852a21d091227868e6708e23c581362c7ce5c26`.
  - `androidforge-desktop-windows-x86_64.zip` (10,442,902 bytes), SHA-256
    `63898187634964b1117fba90df8f601d9366eba24b935154d320d45ac8a4362b`.
  - `androidforge-windows-x86_64-setup.exe` (8,944,319 bytes), SHA-256
    `cfaaa1c0046900e015f8adcea9520fe45859cdd85f760192e079429c742708c0`.
  - `SHA256SUMS.txt` matching all three assets.
- The portable ZIP was extracted into a clean directory. `androidforge.exe --version` reported
  `androidforge 0.4.0`; `doctor fix --plan --json` reported `read_only: true`; the adjacent
  `androidforge-desktop.exe` launched with window title `AndroidForge` and stayed running until closed.
- The installer ran silently with `/VERYSILENT /CURRENTUSER`, installed both executables at version
  `0.4.0`, reported `androidforge 0.4.0`, and uninstalled cleanly with `unins000.exe`; the install
  directory was removed afterward.
- Desktop interactive validation in this delivery was limited to launching the packaged executable and
  confirming it stays responsive; the new Triage/Setup/Sessions panels were compile- and unit-tested but
  not screenshot-reviewed at multiple DPI scales.

### Published release

- Release: <https://github.com/JuggyBob/AndroidForge/releases/tag/v0.4.0>
- Tag: `v0.4.0`, tag object `83fb053083c479316ab74b311193ef27be0f01ee`, commit
  `34f2706df37a128fb2573258ee6380fd216b04fa`.
- The tag-triggered Release workflow (`43943405606`) passed and uploaded CI-built assets.
- Assets attached: `androidforge-cli-windows-x86_64.zip`,
  `androidforge-desktop-windows-x86_64.zip`, `androidforge-windows-x86_64-setup.exe`,
  `SHA256SUMS.txt`.
- All three assets were downloaded from the published release and their SHA-256 values matched
  `SHA256SUMS.txt`.
- The downloaded portable ZIP was extracted to a clean directory; `androidforge --version` reported
  `0.4.0`, all new command help pages loaded, and the adjacent desktop launched with window title
  `AndroidForge`.
- The downloaded installer ran silently, installed CLI/desktop `0.4.0`, and uninstalled cleanly while
  removing the install directory.

## Environment

- Windows 11 x86_64 host, 2560×1440 display at 125% scaling.
- Android 16 / API 36 emulator, serial `emulator-5554`, AVD `androidforge_test`.
- AndroidForge CLI and desktop built from the same `0.3.0` workspace.
- Real target app: `CalculatorSuite-v3.0.3.apk`, package `com.calcsuite`, version `3.0.3` (7),
  certificate `55071A2393348B9C2F44DDE98D21277F1666A56040DECD51C0B83F377A6F9E02`.

## Automated checks

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Passed with no warnings. |
| `cargo test --workspace` | Passed. Includes unit tests for analysis/comparison/flaky heuristics, session pruning, maintenance, diagnostics, run lifecycle and desktop contracts, plus compiled-mock-CLI desktop integration tests. |
| `cargo build --workspace --release` | Passed. |
| `androidforge --version` | `androidforge 0.3.0`; both distributed executables come from this build. |

The desktop compatibility probe requires an exact version match. Its integration test mock now derives
the reported version from the workspace package version so the contract cannot silently drift.

## Real Android validation

The release CLI was exercised against the Android 16 emulator and CalcSuite:

- Session start and restart on `emulator-5554`; headless process launch produced no stray console window
  after the detached-process fix.
- `app install` of `CalculatorSuite-v3.0.3.apk` with `verified=true`, package/version identity, APK
  SHA-256 and certificate recorded in the run report.
- Conformance flow `release-smoke` passed twice (34.1s and 42.9s observed) with build identity inherited
  from the install run.
- `capture frame` and `ui inspect` returned a 1080×2400 screenshot and a parsed hierarchy (32–108 nodes
  depending on the foreground screen).
- Semantic tap on the Settings icon opened the Settings app; node bounds highlighting and tree
  expansion/search were exercised in the desktop.
- `diagnostics logcat peek` returned bounded device logs; the desktop streamed ~4,000 lines, paused,
  filtered and recovered after the device was disconnected and the session restarted.
- Intentional emulator termination showed `E_DEVICE_NOT_FOUND` in logcat/inspector and a disconnected
  preview overlay; `session restart` restored the same session and the live views recovered.
- Cancelling a conformance operation killed its process tree, left the session `ready`, and reconciled
  the run as `cancelled` (12.1s, 3/9 steps) rather than passed or permanently running.
- Maintenance preview identified only old test tombstones/runs. Confirmed cleanup removed those records,
  preserved all current runs and left the active session untouched.

## Desktop workflow validation

The released native desktop was launched and exercised with keyboard/mouse against the real emulator:

- First-launch detection opened Setup on a fresh settings profile; CLI, environment and project steps
  rendered real doctor/project data.
- Sticky wizard footer, sidebar scrolling and the aligned metric/table layouts were checked at
  1280×720 and larger logical sizes.
- Overview, Devices, Sessions, Runs, run details/evidence, Triage, Compare, Logcat, Inspector,
  Maintenance and Settings all rendered real backend data.
- Failure triage displayed failure metadata, build identity, artifacts and flaky analysis.
- Comparison showed build changes, two regressions, current/baseline statuses and duration deltas.
- Live logcat filters, pause/resume, follow, export and disconnect state were exercised.
- Device preview, hierarchy tree, raw XML, bounds highlight and semantic tap were exercised; the
  coordinate fallback is clearly labelled and non-default.
- APK inspect/install, conformance, session restart and controlled cancellation were exercised.
- Maintenance preview and confirmed cleanup were exercised; the preview clears after success.
- Operation duration freezes at the final measured value; the custom-painted Settings gear and sticky
  footer were verified after fixes.
- The overview cards now share a minimum height; path rows reserve space for Copy/Open so text cannot
  spill under the adjacent card.

### Window sizes

The application was launched at client sizes corresponding to 1280×720, 1366×768 and 1920×1080 logical
pixels (physical 1600×900, 1708×960 and 2400×1350 at 125% Windows scaling). Layout clipping, sidebar
reachability, card alignment and table column fit were checked. Final screenshots are stored in
[`docs/screenshots/v0.3.0/`](screenshots/v0.3.0/).

## Packaging validation

- Portable ZIP extracted into a clean temporary directory. Both executables were present, the CLI
  reported `0.3.0`, and the desktop found the adjacent CLI.
- Per-user installer installed silently to a separate test directory without administrator rights.
  Both executables, license/docs, Start Menu shortcuts and uninstaller were present; installed CLI
  reported `0.3.0`.
- A second install over the same directory (upgrade simulation) kept a user-data sentinel file outside
  the install directory.
- Uninstall removed the application directory and shortcuts while preserving the sentinel and all
  AndroidForge user state.
- `SHA256SUMS.txt` was generated for the CLI ZIP, portable ZIP and installer and used to verify the
  portable and installed assets.

## Published release verification

Published as `v0.3.0` at
<https://github.com/JuggyBob/AndroidForge/releases/tag/v0.3.0> (release tag commit `8945b21`).
The release workflow completed successfully on GitHub Actions.

Downloaded assets and verified their published SHA-256 values:

| Asset | SHA-256 |
| --- | --- |
| `androidforge-cli-windows-x86_64.zip` | `c660cf5bf6533880fa2b547bc75f084272b77960833d5d3744ceff424fb5d914` |
| `androidforge-desktop-windows-x86_64.zip` | `7c2706b083bf3937b87d67a170bb4b465340dad6d76d5e585b1f701e163b0e39` |
| `androidforge-windows-x86_64-setup.exe` | `103ba045930c71075bdb02a23a048f54950dd19a1ac60a06b31f142cbd009a8c` |

The downloaded portable ZIP extracted with both executables, docs and license; its CLI reported
`androidforge 0.3.0`. Launching the published portable desktop showed `CLI connected` and resolved the
adjacent published `androidforge.exe` at version `0.3.0`. The downloaded installer installed silently,
reported CLI `0.3.0`, preserved a sentinel user-data file and uninstalled cleanly while preserving that
data.

## Not validated

- Physical Android devices and vendor ROMs.
- Maestro execution, instrumentation test APKs, snapshot restore and root-only time changes.
- macOS and Linux hosts.
- Live CLI update repair against a published release carrying `0.3.0` (the release created from this
  build is the first usable target); version comparison and checksum parsing are unit-tested.
- Very large suites, multi-hour stress runs and every Windows DPI/monitor combination.
- Complete first-run wizard finish path on a second fresh profile after the sticky-footer fix; the
  steps and their backend calls were exercised, but the final Finish/Persist sequence was not repeated
  end-to-end after that UI change.

Validation results are only claimed where the operation above was actually performed. Unsupported
features return structured `E_UNSUPPORTED_OPERATION` errors rather than fabricated success.

