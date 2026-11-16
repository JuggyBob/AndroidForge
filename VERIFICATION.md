# Verification record

This document records what was actually run for the `0.4.0` Alpha delivery on 2026-11-16, followed by
the historical `0.3.0` record. It separates automated checks, real Android validation, desktop workflow
validation and packaging validation from areas that were not exercised.

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
  - `androidforge-cli-windows-x86_64.zip` (3,837,067 bytes), SHA-256
    `3737ec9ad2950fc0bee861f7490128f2e93d5f063689f8a4e105ef328a873255`.
  - `androidforge-desktop-windows-x86_64.zip` (10,442,651 bytes), SHA-256
    `9aeb3bf84a22822b2eed068b5debd83d239ceda84d6e2866adf45b9e63088859`.
  - `androidforge-windows-x86_64-setup.exe` (8,948,509 bytes), SHA-256
    `f8245889c8903261fdd50a6770c5e73c6a839950747023ac402613b76859ce5f`.
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



