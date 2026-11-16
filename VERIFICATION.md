# Verification record

This document records what was actually run for the `0.3.0` Alpha delivery on 2026-11-16. It separates
automated checks, real Android validation, desktop workflow validation and packaging validation from the
areas that were not exercised.

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
