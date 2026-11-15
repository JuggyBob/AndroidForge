# AndroidForge — Vision and Non-Goals

## Mission

AndroidForge is local-first infrastructure for AI coding agents that work on Android applications. It
removes the repetitive, fragile work around devices: acquiring an emulator or physical device, keeping
ownership explicit, installing builds, inspecting and driving the UI, preparing test state, collecting
diagnostic evidence, and returning a result an agent can act on.

The product is a single native Rust CLI. It is not a dashboard, an Android IDE, a cloud test service or
a new scripting language.

## The three workflows that define the product

### IronPulse
An existing AVD, headless execution, warm reuse, named test state, seeded databases, Compose-accessible
UI interaction, SAF/system dialogs, logcat and crash evidence, and process-versus-task liveness. The
emulator must not be restarted for every test.

### DiscountHunter
Multiple emulators and physical devices, explicit ownership, disconnect recovery, port collision
avoidance, certificate-aware installation, package/version inspection, permissions, AppOps, background
jobs, alarms, Doze/standby, app-private files, time inspection/control, cross-application UI, and raw
ADB when the abstraction is insufficient.

### CalcSuite
A simple scriptable interface: install, UI hierarchy dump, XML parsing, semantic selectors, text entry,
waiting, assertions, screenshots, multi-step runs, and JSON evidence. No Maestro dependency. Existing
Gradle unit tests and Python/PowerShell integration scripts remain untouched.

## Core product rules

- **Generic core, project adapters.** The Rust core has no app names, package IDs, AVD names, fixture
  paths or project branches. A second project must run from the same binary with only adapter, fixture
  and script changes.
- **Earn abstractions.** Write generically from day one; freeze a shared abstraction only after a
  second real project needs it. Until then, project-specific behavior lives in that project's adapter.
- **Phase gates.** Core + one real vertical slice first; second app proves genericity; third app adds
  hard primitives. A feature is not complete just because a command exists.
- **No stubs.** An unsupported capability returns `E_UNSUPPORTED_OPERATION` with the reason and an
  alternative. Fake success is worse than a missing command.
- **Real evidence.** Prefer a small tested capability over a broad speculative one.

## Non-negotiable technology constraints

- Genuine Rust, Edition 2024, Cargo.
- Native Windows CLI executable (`androidforge.exe`).
- No GUI (no WinUI, WPF, egui, Tauri, Slint).
- No mandatory daemon or service.
- Local only: no cloud execution, telemetry, accounts, subscription, AI API, or automatic upload.
- External Android utilities (ADB, emulator, AAPT2, APK Signer, Maestro) are orchestrated through
  native process management; they do not turn the product into a script wrapper.
- Never read, display, upload or persist signing passwords, keystores or `keystore.properties`.

## Explicit non-goals

- Android Studio replacement, custom emulator, custom ADB, Gradle replacement.
- Robolectric or Compose instrumentation replacement.
- GitHub Actions replacement or release publishing.
- LLM integration, cloud infrastructure, mandatory Maestro, mandatory HTML reports.
- Visual-diff testing as a core dependency.
- A complicated custom test language.
- Pretending that AndroidForge can prevent non-AndroidForge programs from using ADB.
- Pretending a renamed profile is isolation when the AVD is shared.

## Honest limitations

Android APIs restrict what a shell-level tool can observe. Task state, notification state and
JobScheduler state are parsed best-effort and raw evidence is preserved. Compose internal nodes are not
magically visible; apps must expose semantics. Time changes usually require root. Snapshot behavior
depends on the emulator build. Physical-device behavior depends on vendor restrictions. These are
reported as limitations, not hidden behind false success.

## Definition of a trustworthy delivery

The same executable can run real smoke/regression flows for multiple Android apps through project
adapters and fixtures, producing one JSON report and an evidence directory per run, with stable exit
codes and honest capability reporting. The first real use matters more than the README.
