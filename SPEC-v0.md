# SPEC v0 — The Narrow, Must-Be-Excellent Core

This specification describes the implementation delivered here. Later-scope features are listed in
`SPEC-v1.md` and `SPEC-v2.md` and return a structured unsupported error if not implemented.

## Phase contract

- **v0.1** — one real vertical slice on one app: AVD lifecycle, sessions/ownership, APK install and
  build identity, app launch/status, UI dump/selectors/actions/waits, screenshots, logcat, process/task
  state, fixtures v0, artifacts and JSON report, failure evidence, `doctor`, `config validate`,
  `--trace`, `--dry-run`, `--json`.
- **v0.2** — a second app using the same binary and only adapter/fixture changes. If the core must change,
  the abstraction is wrong.
- **v0.3** — a third app and hard primitives (physical devices, SAF/system dialogs, notifications,
  jobs, alarms, Doze/standby, permissions/AppOps, time, Maestro/instrumentation, snapshots when
  proven reliable).
- **v1.0** — concurrency hardening, packaging, full documentation, three conformance profiles,
  case studies, prebuilt binary.

This repository implements the v0 core and selected v0.3 primitives where they are honest and
testable. A command is not considered complete merely because it compiles; see `VERIFICATION.md`.

## Required capabilities

### Discovery and environment

- `doctor` / `doctor --json`: SDK, ADB, emulator, build-tools, Java, APK Signer, Maestro, AVDs,
  virtualization, licenses, writable artifacts, sessions and config validation.
- `capabilities --json`: supported / best-effort / unsupported, with host requirements and an honest
  `runtime_verified` flag.
- `schema list|show`, `init`, `config show|validate|path`.

### Devices and emulators

- `devices list|inspect|wait|prepare|restore`.
- Robust `adb devices -l` parsing; explicit target when multiple devices; offline/unauthorized/booting
  detection.
- `avd list`; `emulator start|status|stop`; headless by default; even console ports, ADB port and gRPC
  collision checks; boot wait; crash detection; no unowned stops.
- Session start/attach/claim/list/inspect/status/release/stop/use/current/gc/restart.
- Locks with PID/heartbeat metadata, dead-operation recovery, deliberate stale-session reclaim, and
  ownership checks on devices.
- Device preparation with previous-value recording and restore (animations, stay awake, wake/unlock,
  locale, timezone, font scale, dark mode, orientation, density).
- Snapshots list/save/load/delete with explicit `--yes` for destructive load and honest unsupported
  results when the emulator version refuses.

### Applications and APKs

- `app install/uninstall/clear-data/launch/force-stop/kill/status/info/list`.
- Distinguish `am force-stop` from `am kill`.
- Build identity after every install: package, versionName, versionCode, local APK SHA-256, installed
  APK SHA-256, certificate SHA-256, debug-signed, verified.
- Signature-mismatch diagnosis without automatic uninstall.
- `apk inspect/verify --expected-cert-sha256 --expected-package --min-version-code`.
- Permissions and AppOps: check, grant, revoke, get/set/reset, special permission/AppOp fallback.
- App-private file push/pull via `run-as` only for debuggable apps; external files; data backup/restore
  with explicit confirmation.

### UI and input

- `ui dump/find/tap/long-press/text/clear/wait/assert/settle/scroll/scroll-to/swipe/back/home/keyevent/
  hide-keyboard/focused`.
- Exact selectors by default; contains, content description, resource ID, class, index, clickable,
  enabled, focused, scrollable, checked, visible.
- Ambiguity is an error with all candidates and an `--index` suggestion.
- Tappable-center calculation from real hierarchy bounds; clickable ancestors; no scaling from
  unrelated screenshots.
- Verified text entry: focused editable field, explicit clear, batched deletes, re-read/compare,
  unsupported character reporting, keyboard-aware Back behavior.
- Compose accessibility where exposed; documented limitations for internal test tags.
- Waits and settle polling; no fixed sleeps in the core.

### Evidence and diagnostics

- Screenshot PNG via `adb exec-out`; dimensions/display metadata; SHA-256; artifact registration.
- Screen recording start/stop/status with Android's 180-second limit, reliable stop, pull/cleanup and
  no audio claim.
- Logcat one-shot tail and continuous capture with time/byte caps; never clear the global buffer.
- Crash, ANR, native crash and process-death detection, best-effort and labelled.
- Process exists/alive, task exists, activity exists, foreground, resumed as separate states.
- Jobs, alarms, notifications, activity, package, battery, deviceidle, connectivity dumps; raw preserved.
- Automatic failure bundle: screenshot, hierarchy/focused node, logcat, crashes, app state, dumpsys.
- `diagnostics collect|app|logcat|crashes|inspect`.

### Fixtures, hooks, conformance, external work

- Versioned fixture manifest with phases, steps, retries, timeout, `on_failure`, capture policy,
  destructive confirmation, `--dry-run`, `--yes`, env contract and `fixture validate`.
- Named explicit debug hooks; no automatic hooks.
- Thin conformance manifest (script/Maestro/fixture) with expected status, exit code, artifacts and
  JSON paths. No custom test DSL.
- Optional Maestro with no cloud requirement.
- Instrumentation wrapper, no Gradle replacement.
- `run start|end|status` and `run -- <program>` with one report across steps.
- `report show|path|prune` and `report import junit`.
- Raw ADB passthrough with argument boundaries preserved and unsafe status explicit.

### CLI contract

- `--json` on every meaningful command; stdout is JSON only.
- `--trace`, `--dry-run` where meaningful, `--yes` for destructive operations.
- Stable exit codes and structured errors with code, message, recoverability, suggestions, context and
  artifacts.
- Command suggestions for misspellings.
- Bounded external commands with process-tree cleanup on Windows.

## Cut/deferred from v0

- Full named-profile cloning/isolation and AVD cloning.
- Visual diff and mandatory HTML reports.
- Cloud execution, telemetry, LLM integration, release publishing.
- A mandatory daemon.
- “Newer Android CLI emulator management” as a required dependency; legacy emulator path primary.
