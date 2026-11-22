# AndroidForge — Portfolio Notes

## The problem

Three Android apps needed the same thing from an AI coding agent: a trustworthy way to acquire a device, install a build, drive the UI, stage state, collect evidence and tell the agent exactly what happened. Existing options were a mix of fragile coordinate scripts, Android Studio's UI, a cloud device farm, or a test framework that only covered one layer.

The apps were different on purpose:

- **CalcSuite** — a stable release app with a simple UI.
- **DiscountHunter** — notifications, permissions, app-private watch state, background checks and failure modes that only appear on a real device.
- **IronPulse** — Compose, onboarding, process/task liveness, widget intents and transient animation evidence.

A solution that only worked for one of them would have been a demo. The delivery goal was one binary that works for all three, with project-specific behavior confined to adapters, fixtures, hooks and scripts.

## What was built

A native Rust 2024 CLI for Windows, no daemon and no cloud:

- device/emulator lifecycle and session ownership;
- verified APK install with package, version, local and installed hash, certificate and debug-signed identity;
- semantic UIAutomator interaction with verified text entry and IME-aware Back;
- screenshots, screen recording, bounded logcat, crashes/ANRs, process/task/foreground state and dumpsys;
- versioned fixtures with app-private file transfer via `run-as`;
- named debug hooks, including real broadcasts and widget action intents;
- thin conformance manifests and one run report with checksummed artifacts;
- structured JSON, stable exit codes and automatic failure evidence.

The same `androidforge.exe` ran CalcSuite, DiscountHunter and IronPulse. The `genericity` test scans runtime crate sources for app names, package ids and fixture tokens.

## Why Rust

The hard part was not parsing XML. It was making failure bounded and understandable:

- every external process has a timeout, output cap and Windows process-tree cleanup;
- JSON stdout and diagnostics stderr are separate by construction;
- one static binary removes runtime installs from the agent's critical path;
- no daemon means no hidden state or port to manage;
- the type system and clippy made error/exit contracts and ownership rules explicit.

Rust also made it practical to ship a single binary with a strict `cargo fmt`, clippy `-D warnings`, workspace tests and a release profile with LTO.

## The DiscountHunter notification story

This was the case that proved the tool was more than a UI clicker.

The app has a real background check that can post a notification when a saved game goes on sale. The check is triggered by an alarm, WorkManager, or a user-visible "Check now" action. A tempting shortcut is to guess a JobScheduler id and claim success when `cmd jobscheduler run` returns. That shortcut is wrong: IDs are not stable, WorkManager owns them, and a successful shell command does not mean the app did any work.

Instead the adapter defines a named hook to the app's real `CHECK_DEALS` receiver. The conformance flow:

1. grants `POST_NOTIFICATIONS`;
2. seeds a valid watch-state baseline through `run-as`;
3. launches the app;
4. navigates to Settings > Deal alerts with semantic selectors;
5. invokes the real receiver through the named hook;
6. waits for the app's own "Last check" line;
7. records `posted 1 notification(s)` in the report;
8. captures a screenshot and diagnostics.

The evidence is in [case-studies/DiscountHunter-smoke](case-studies/DiscountHunter-smoke/). It demonstrates notification permission, app-private state, a real background path and parseable evidence without hardcoded scheduler ids.

## Third-app edges

IronPulse added different realities:

- first-run onboarding and personal profile fields;
- Compose semantics that only expose visible text, not internal test tags;
- process exists, process alive, task exists, foreground and resumed as separate states;
- real widget action intents (`OPEN_STREAK`, `OPEN_PROFILE`);
- a short screen recording for transient animation evidence;
- a debug-only app-private file round-trip;
- a deliberate release-over-debug install that correctly produced `E_INSTALL_SIGNATURE_MISMATCH` and left the debug app installed.

See [case-studies/IronPulse-smoke](case-studies/IronPulse-smoke/).

## Engineering decisions worth discussing

**Sessions, not global device state.** Ownership is explicit, with locks, heartbeats and stale recovery. AndroidForge never stops an unowned emulator.

**Install identity is observed, not declared.** The installed APK is pulled back and hashed; the certificate is read with `apksigner`. A mismatch is diagnosed and never auto-uninstalled. A run that did not install does not copy an expected certificate into an observed field.

**Failures produce evidence.** A failed UI assertion arrives with a screenshot, hierarchy, logcat, crash/ANR state, process/task state and dumpsys paths. Agents do not have to reproduce blind.

**Fixtures are explicit.** There is no hidden test runner and no automatic project behavior. `--dry-run` and `--yes` make destructive intent visible.

**Honest unsupported.** AVD cloning, HTML reports and cloud are absent and report `E_UNSUPPORTED_OPERATION` with reasons and alternatives. Screenshot baseline/region comparison is supported through `capture screenshot --compare`.

**The core stays generic.** Project names, package ids, AVD names and fixture paths live in `examples/` and `.androidforge/`, never in the runtime crates. CI can enforce it with the genericity test.

## What was verified

Real Android 16 emulator runs on 2026-11-15:

- CalcSuite release install and smoke run;
- DiscountHunter debug install, notification permission, watch-state seed, real background-check hook, posted notification, screenshot and diagnostics;
- IronPulse release onboarding, Home, process/task liveness, widget intents, Settings, screenshot, recording and diagnostics;
- IronPulse debug install, app-private file round-trip;
- deliberate signature mismatch with non-destructive diagnosis;
- session start, prepare, release/stop.

See [VERIFICATION.md](VERIFICATION.md) for exact hashes, commands and limits. Physical devices, Maestro, an instrumentation test package, snapshot restore and root-only time changes were not exercised.

## What I would do next

1. Run the same flows on a physical device and a vendor ROM; harden task/notification parsing where formats differ.
2. Add an instrumentation package and merge JUnit evidence into the same run.
3. Test snapshot save/load/restore on pinned emulator versions before claiming isolation.
4. Add AVD cloning only after proving independent writable data.
5. Add a concurrency stress test with several agents and several devices.
6. Keep the capability matrix honest as the surface grows.

## Repository reading order

- [README.md](README.md) — overview and quickstart
- [AGENT_GUIDE.md](AGENT_GUIDE.md) — the agent operating model
- [ARCHITECTURE.md](ARCHITECTURE.md) — crates, sessions, runs, process and evidence model
- [PROJECT_ADAPTER.md](PROJECT_ADAPTER.md) — adapter, fixtures, hooks and conformance
- [VERIFICATION.md](VERIFICATION.md) — what actually ran
- [case-studies/](case-studies/) — checkable evidence from real emulator runs
