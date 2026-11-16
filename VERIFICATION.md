# Verification

This document records what was actually run for the `0.2.0` Alpha delivery on 2026-11-16. It separates
verified behavior from implemented-but-not-exercised work. The `0.1.0` record remains in git history.

## Host and toolchain

| Item | Value |
| --- | --- |
| Host | Windows 11 x64 |
| Rust | `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0` |
| MSRV | `1.95` (workspace `rust-version`) |
| Android | Android 16 / API 36 emulator, x86_64, AVD `androidforge_test` |
| Project | `examples/CalcSuite`, CalcSuite 3.0.3 (`com.calcsuite`) |
| Repository | `https://github.com/JuggyBob/AndroidForge`, branch `main` |

## Build and static checks

All commands were run from the repository root on the host above.

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed. |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Passed. |
| `cargo test --workspace` | Passed: 137 tests, 0 failed. |
| `cargo build --workspace --release` | Passed. `androidforge.exe` 9,282,048 bytes; `androidforge-desktop.exe` 10,399,744 bytes. |
| `androidforge --version` | `androidforge 0.2.0`. |

Current stable required let-chain edits in older nested `if` blocks; `cargo clippy --fix` applied only
machine-applicable `collapsible_if` changes across runtime crates, followed by the full test suite above.

### Desktop automated tests

| Suite | Result |
| --- | --- |
| `desktop` unit tests (`cargo test -p androidforge-desktop`) | 14 passed. Includes CLI locator, JSON parser, evidence path safety/XML, project discovery, native process-tree termination. |
| Desktop backend integration tests (`desktop/tests/backend.rs`) | 8 passed. Compiled mock CLI covers version/schema probing, typed devices, structured errors, invalid JSON, timeout kill, streamed output, cancellation and cancelled outcome. |
| Workspace total | 137 passed, 0 failed. |

The cancellation test asserts that the child is terminated, the outcome is marked `cancelled`, success is
false, and the operation ends quickly rather than waiting for the mock CLI's 60-second sleep.

## CLI smoke checks

- `doctor --json`, `devices list --json`, `session list --json`, `config show --json` returned valid
  envelopes.
- `report list`, `report show`, `report path` resolved CalcSuite runs from both project and repository
  roots.
- `fixture list`, `hook list`, `conformance run` returned the expected project artifacts.
- Release binaries were launched from the portable layout and resolved the sidecar CLI.
- `androidforge devices list` after final cleanup returned zero devices and no active session.

## Desktop cockpit validation

The `0.2.0` native desktop was exercised against a real emulator and the CalcSuite project. It used a
release `androidforge.exe` as the authoritative sidecar; the desktop never implemented Android behavior
itself.

### P0 read-only

| Flow | Evidence |
| --- | --- |
| Project discovery | Picker showed CalcSuite, DiscountHunter and IronPulse; CalcSuite selected as `com.calcsuite`, adapter v1. |
| Overview | Showed environment `degraded` only because Maestro was absent; CLI `0.2.0` connected; device/session/run counters updated. |
| Devices | `emulator-5554`, `sdk_gphone64_x86_64`, Android 16, owner `Ivan · ses-4c1e3b4a8c8a`. |
| Sessions | Owner, heartbeat, serial, state, prepared settings and attachment warning rendered. `Select` made a session the action target. |
| Runs | Report-backed table listed install, conformance and hook runs with status, steps, duration and started time. `Open` loaded run details. |
| Run detail | CalcSuite conformance `20261116-060718-ab1205f9_conformance-run` showed package `com.calcsuite`, version `3.0.3 (7)`, matching APK and installed SHA-256, certificate `55071A…9E02`, `verified=yes`, 7 steps and 10 artifacts. |
| Evidence | `20261116-060734-934_run-start.png` rendered at 1080×2400 with Fit/100% controls; `run-logcat.txt` rendered with line numbers and a working search filter; raw files showed `report.json`, manifest 14 ok / 0 missing / 0 mismatched and manifest entries. |
| Error inspector | An attached-session restart returned `E_UNSUPPORTED_OPERATION` with the reason and suggestion; the desktop displayed the code, message, suggestions and raw JSON. |

### P1 controlled actions

| Flow | Evidence |
| --- | --- |
| Session start | GUI started a session on `emulator-5554`; session reached `ready`. |
| APK inspect | Choosing `CalculatorSuite-v3.0.3.apk` showed `com.calcsuite`, `3.0.3 (7)`, 14.2 MB, SHA-256 `ccc2d570…68fe8a`, certificate `55071A…9F02`, signer `CN=Ivan Bizimoski`. |
| Install dry-run | Preview returned `dry_run: true` with APK paths, architectures, version, SDKs and signature state. |
| Install | GUI install created `20261116-063106-2d546e90_app-install`, status `passed`, `verified=yes`; the new run detail opened automatically. |
| Conformance | GUI launched `20261116-060718-ab1205f9_conformance-run` and later `20261116-071813-5e9e6113_conformance-run`, both `passed` with all steps successful. |
| Hook | GUI ran `open-settings`, creating `20261116-072206-057d50ec_hook-run`, status `passed`, 1 step, 1 artifact. |
| Output | Bottom panel streamed the CLI child output, showed elapsed time, exit status and copy action; a completed panel no longer blocked new actions. |
| Cancellation | GUI cancelled conformance run `20261116-081801-7c4e0acd_conformance-run` at ~5 s. Outcome showed `cancelled`, exit `Some(1)`, no final report explanation, and the session/device remained ready. The cancelled run remains `running` in `report list` because its report was not finalized. |
| Release | GUI release changed session `ses-4c1e3b4a8c8a` to `released`; `devices list` still showed `emulator-5554` running. |
| Attach | GUI attach created `ses-83ad725de5d9` (`attached`, `ready`) on `emulator-5554`. |
| Restart rejection | Restarting the attached session returned `E_UNSUPPORTED_OPERATION: restart is only supported for AndroidForge-launched emulators`, with the suggested release/start alternative. |
| Restart success | GUI restarted AndroidForge-launched session `ses-fbbab981c7c8`; operation exited 0 in ~100 s and the session returned to `ready` on `emulator-5554`. |
| Stop | GUI stop moved the owned emulator session to `released`, removed `emulator-5554` from `devices list` and ended the emulator process. |

### Cancellation and emulator safety regression

An earlier implementation placed the desktop child in a Windows job object with
`KILL_ON_JOB_CLOSE`. Verification showed that the detached emulator inherited the job; dismissing a
finished restart operation killed the emulator while leaving the session `ready`. This was a real defect
in the delivery candidate.

The final implementation:

- no longer uses job objects;
- walks the Windows process snapshot and terminates descendants deepest-first, with `taskkill /T /F` as
  a fallback;
- exposes `Cancel` only for installs, fixtures, conformance and hooks;
- runs session lifecycle operations to completion;
- never calls `session stop` or `session release` on cancel.

After the fix, a restart operation was completed and dismissed; `devices list` still reported
`emulator-5554 device`, and a subsequent conformance cancellation left the session and emulator intact.

## Evidence integrity

- Run directories contain `report.json`, `metadata.json`, `manifest.json` and artifacts.
- The desktop manifest view verified 14 entries as `ok`, 0 missing, 0 mismatched for the inspected
  conformance run.
- APK install identity was measured from the local APK and the pulled installed APK; hashes matched.
- Raw case-study run evidence is excluded from the public repository because it contains machine paths,
  device logs and app data. Summaries remain under [case-studies/](case-studies/).

## Issues found and fixed during verification

1. **Desktop reported CLI offline.** The version probe passed `--json` to `--version`; the CLI contract
   is a plain `androidforge 0.2.0`. The probe now invokes `--version` directly.
2. **Run details failed to load.** The desktop called `show`/`path`; the CLI requires `report show` and
   `report path`. Both now include the `report` subcommand.
3. **Row selection did not fire.** Table rows lacked a click sense; explicit `Select`/`Open` buttons were
   added and the table sense set to click.
4. **Install confirmation was hidden.** The install dialog rendered above the confirmation. Dialogs are
   ordered so confirmations/errors render last, and install closes before confirming.
5. **Completed operations blocked actions.** `operation.is_some()` treated finished output as busy. All
   action guards now use `active_operation()`.
6. **Released tombstones exposed actions.** Release/stop/restart/use-as-target/install are disabled for
   released/abandoned sessions.
7. **Long commands overlapped buttons.** Operation output moved the command to a separate truncated line.
8. **Cancellation did not terminate the GUI child.** Replaced `taskkill`-only cancellation with a native
   descendant walk plus fallback, after the job-object regression above.
9. **Job object killed detached emulators.** Removed; see the regression section.
10. **Artifact path checks failed on Windows verbatim paths.** Canonical paths strip `\\?\` before the
    prefix comparison, and XML previews use `root_element()`.
11. **Cancelled runs showed a raw parser error.** The operations panel now explains that no final report
    was produced because the operation was cancelled.

## Implemented but not exercised in this delivery

- Physical devices and vendor ROMs.
- Maestro execution and instrumentation test packages.
- Snapshot load/restore and root-only time changes.
- macOS/Linux hosts (both executables).
- Very large suites, long-duration stress and multiple desktops updating the same project concurrently.
- Desktop restart success on a physical/attached device (attached restart is rejected by design).

## Known limitations

- Alpha software; interfaces and JSON shapes may change before `1.0`.
- The desktop is an operator console, not a project editor, device mirror, HTML report or automation API.
- Cancelling a run can leave an unfinalized `running` report; inspect or prune it deliberately.
- A completed operation occupies the bottom panel until dismissed, but no longer blocks actions.
- Parsers for notifications/jobs/alarms/Doze are best-effort and preserved as raw evidence.

## Reproduce

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --workspace --release

.\target\release\androidforge.exe --version
.\target\release\androidforge.exe doctor --json
.\target\release\androidforge-desktop.exe

# In the desktop: select examples/CalcSuite, select a ready session, inspect a run,
# open Evidence, run a conformance or hook, then cancel a conformance run.
```

## Release assets

| Asset | Purpose |
| --- | --- |
| `androidforge-cli-windows-x86_64.zip` | Standalone CLI. |
| `androidforge-desktop-windows-x86_64.zip` | Portable CLI + desktop cockpit, aligned at `0.2.0`. |
| `SHA256SUMS.txt` | SHA-256 for both archives. |

The assets were downloaded from the published release, extracted and checked against `SHA256SUMS.txt`:

```text
e373b2ca3dc2112b882a0d4f2795ceb8791cc7d5c0e191992461e2bfdd22c7dc  androidforge-cli-windows-x86_64.zip
9a8385658a0f54829ea60c577eb47f088aa072093c58da8e6a79380c352bdecb  androidforge-desktop-windows-x86_64.zip
```

Both extracted `androidforge.exe` binaries reported `androidforge 0.2.0`. The desktop archive contained
`androidforge.exe`, `androidforge-desktop.exe`, `LICENSE` and `PORTABLE.txt`. Release:
`https://github.com/JuggyBob/AndroidForge/releases/tag/v0.2.0`.
