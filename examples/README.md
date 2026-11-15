# Example Adapters

These are project-specific adapters, fixtures, hooks and conformance flows. They are not linked into the Rust binary. The same `androidforge.exe` runs all three by loading different configuration.

| Example | Package | Primary build used here | What it stresses |
| --- | --- | --- | --- |
| `CalcSuite` | `com.calcsuite` | release `3.0.3` | Current Settings updater, semantic navigation, simple evidence. |
| `DiscountHunter` | `com.discounthunter.android` / `.debug` | debug `1.1.2-release-debug` | Notification permission, app-private watch state via `run-as`, semantic Settings flow, real broadcast hook, evidence. |
| `IronPulse` | `com.aistudio.fitness.ironpulse` | release `4.4.0`, debug `4.3.0` | Compose-visible text, first-run onboarding, process/task liveness, widget intents, Settings, screen recording. |

## Common workflow

```powershell
$exe = ".\target\release\androidforge.exe"
$project = ".\examples\DiscountHunter"

& $exe --project $project config validate --json
& $exe --project $project fixture list --json
& $exe --project $project hook list --json
& $exe --project $project fixture run smoke --session <ID> --json
& $exe --project $project conformance run --session <ID> --json
```

Use a session you own. Do not run two examples against the same device at the same time; UIAutomator and app state will interfere.

## CalcSuite

```powershell
& $exe app install <path>\CalculatorSuite-v3.0.3.apk --session <ID> --json
& $exe --project .\examples\CalcSuite fixture run smoke --session <ID> --json
& $exe --project .\examples\CalcSuite conformance run --session <ID> --json
```

Expected identity for the current build used in this delivery:

- package `com.calcsuite`, versionName `3.0.3`, versionCode `7`
- APK SHA-256 `ccc2d570010409ebaa2277ac65ba8f78008a84e10b719046b6b488362868fe6a`
- release certificate SHA-256 `55071A2393348B9C2F44DDE98D21277F1666A56040DECD51C0B83F377A6F9E02`

The fixture force-stops, launches, and asserts the visible title and `Categories` shell. The conformance script opens Settings, scrolls to `Check for updates`, taps `Check`, asserts `You have the newest version (3.0.3).`, captures a screenshot and emits a JSON check summary. The conformance report inherits the install's build identity.

## DiscountHunter

```powershell
& $exe app install <path>\app-debug.apk --session <ID> --json
& $exe --project .\examples\DiscountHunter fixture run smoke --session <ID> --json
& $exe --project .\examples\DiscountHunter hook run check-deals --session <ID> --json
& $exe --project .\examples\DiscountHunter conformance run --session <ID> --json
```

The default adapter targets the debug id `com.discounthunter.android.debug`; the declared certificate is the standard Android debug certificate. Add the release certificate to `[project].expected_cert_sha256` when testing a release APK.

The fixture grants `POST_NOTIFICATIONS`, pushes a valid `files/alerts/watch-state.json` baseline into the debuggable app, launches and asserts the app shell. The conformance handles the intro and notification prompt, navigates to Settings > Deal alerts, checks permission, runs the real `CHECK_DEALS` receiver hook, captures a screenshot and collects diagnostics.

The `check-deals` hook is a real broadcast to the app's non-exported receiver. It uses `run_as = true` so the explicit broadcast is sent as the app UID on a debuggable build; it is not a guessed JobScheduler id. Its `verify` postcondition reads `files/alerts/status.json` through `run-as` and requires `checkedAt` to change, so an undelivered broadcast cannot pass. `open-deal-link` uses the Steam deep link declared in the manifest. `open-notification-settings` opens the package's notification settings for channel diagnosis.

## IronPulse

```powershell
& $exe app install <path>\app-release.apk --session <ID> --json
& $exe --project .\examples\IronPulse fixture run smoke --session <ID> --json
& $exe --project .\examples\IronPulse hook run focus-streak --session <ID> --json
& $exe --project .\examples\IronPulse conformance run --session <ID> --json
```

The fixture force-stops, launches and asserts an IronPulse surface. The conformance detects first-run onboarding, fills the real profile fields when needed, verifies Home, checks foreground/process/task liveness, invokes the real widget action intents (`OPEN_STREAK`, `OPEN_PROFILE`), asserts Settings, captures a screenshot, records a short MP4 and collects diagnostics.

Compose `testTag`s are internal to the app and are not exposed as UIAutomator resource ids. The flow uses visible semantics and records that limitation instead of pretending internal tags are selectable. Use instrumentation for tag-level assertions.

`debug-file-roundtrip` is a debug-only fixture that pushes a marker through `run-as`, pulls it back and verifies the bytes. It does not claim to seed Room or DataStore state.

## What belongs in the core

Nothing app-specific. If a new app needs a primitive that is genuinely generic, add it to the appropriate crate with tests and document it. If only one app needs it, keep it in the adapter, hook, fixture or conformance script.

The workspace's `genericity` test fails if app names, package ids or project tokens appear in runtime crate sources.
