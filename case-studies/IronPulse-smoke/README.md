# IronPulse Smoke — Compose, Liveness, Widget Intents and Recording

This case study summarizes a real Android 16 emulator run from 2026-11-15. IronPulse is the third
application and stresses first-run onboarding, process/task liveness, real widget action intents,
Settings navigation and transient screen recording. Raw run artifacts are intentionally not published
because they can contain machine paths, device logs and app data.

## Build identity

Release build (`20261115-165451-63437e87_app-install`) recorded:

- package `com.aistudio.fitness.ironpulse`
- versionName `4.4.0`, versionCode `12`
- local and installed APK SHA-256 `58e0ad70496080454ee62af51d0e0b53bbf2c892255b8c9d0cb065e7c4fd3464`
- release certificate SHA-256 `46893F8E6C463E01B7B849AEF18098331E4C162325E906D86762F16A6430BB2C`
- `debug_signed: false`, `verified: true`

A debug 4.3.0 build was also used for the app-private file round-trip:

- versionName `4.3.0`, versionCode `11`
- APK SHA-256 `3233645e08f8ec3d4a5f63edfe1e12d4e749d66c481218330496eb16f308c8a9`
- debug certificate SHA-256 `C1FD411DC1DA096FD12001EC5B71DA212C91E13442BA289D2C8F36A72F0ED691`
- `debug_signed: true`, `verified: true`

## Conformance

Run `20261115-165457-2029a77c_conformance-run` finished with status `passed`, exit code `0`, with the
release binary and a freshly cleared app (real first-run onboarding). The report inherits the install's
full build identity: package, `4.4.0` / code `12`, APK and installed hash, release certificate,
`verified: true`, and eight parsed `flow_checks`.

The external flow's eight named checks were:

- app foreground;
- Home surface visible (`Welcome to IronPulse, ...`);
- foreground process, task exists and resumed state verified together;
- `focus-streak` widget action intent accepted by `MainActivity`;
- `open-profile` widget action intent accepted;
- Settings surface visible (`Your account, appearance, tools and data`);
- screenshot evidence;
- screen recording produced and checksummed.

The conformance starts with a cleared app, taps through the real intro, fills the profile fields
(`AndroidForge`, 30, 180, 80, 75), starts training, navigates Home, then runs the liveness and widget
checks. The MP4 recording proves the recording lifecycle, not visual quality.

The flow documents a real limitation: Compose `testTag`s are internal to the app and are not exposed
as UIAutomator resource ids because `testTagsAsResourceId` is not set. The flow uses visible semantics
and says so instead of pretending tags are selectable.

## App-private file round-trip

A debug-only fixture run force-stopped the app, pushed a JSON marker into
`files/androidforge-fixture-marker.json` through `run-as`, pulled it back to the project artifact
directory and matched the bytes. This proves the file primitive and its debuggable-only boundary. It
does not claim to seed Room or DataStore.

## Signature mismatch

With the debug build installed, installing the release 4.4.0 APK produced
`E_INSTALL_SIGNATURE_MISMATCH` (exit 6). The error listed incoming and installed certificate SHA-256
values and the installed version; the app remained installed. The check was non-destructive.

## Limits

The public case study targets release 4.4.0 / debug 4.3.0 because those were the builds present in the
verification workspace. A later IronPulse 4.8.2 retest was performed separately; its APK is not part of
this repository. SAF/system-dialog exposure and internal Compose test-tag interaction remain future
work; instrumentation is the correct path for tag-level assertions. Raw evidence is withheld from the
public repository; the run ids above identify the local records.
