# DiscountHunter Smoke — Notification and Background-Check Case Study

This case study summarizes real evidence from the `com.discounthunter.android.debug` build on an
Android 16 emulator on 2026-11-15. It is the genericity and notification/background-work story, not a
mock. Raw run artifacts are intentionally not published because they can contain machine paths, device
logs and app data.

## Build identity

Install run `20261115-165135-e118856a_app-install` recorded:

- package `com.discounthunter.android.debug`
- versionName `1.1.2-release-debug`, versionCode `10`
- local and installed APK SHA-256 `6e7a01bebcef656f118061fa85c0e7a145ac52677070d28b124077c0f16cc996`
- debug certificate SHA-256 `C1FD411DC1DA096FD12001EC5B71DA212C91E13442BA289D2C8F36A72F0ED691`
- `debug_signed: true`, `verified: true`

The conformance report inherits the same identity from the install in the session.

## Current conformance (verified hook)

Run `20261115-165138-b1a909b8_conformance-run` finished with status `passed`, exit code `0`, using the
release binary. The command result included the full build identity and six parsed `flow_checks`; the
same checks were recorded in the conformance step details.

The fixture:

1. force-stopped the app;
2. granted `android.permission.POST_NOTIFICATIONS`;
3. pushed a valid `files/alerts/watch-state.json` baseline through `run-as`;
4. launched the debug app and asserted its shell.

The conformance script:

1. handled the intro/permission UI if present;
2. navigated `Deals -> Settings -> Deal alerts` with semantic selectors and `scroll-to`;
3. asserted the `Deal alerts` section;
4. checked notification permission through `dumpsys package`;
5. invoked the app's real background-check receiver through the named `check-deals` hook;
6. the hook's `verify.must_change` postcondition read `files/alerts/status.json` before and after
   through `run-as` and required `checkedAt` to change;
7. waited for and asserted the app's own `Last check:` line;
8. captured a screenshot and diagnostics;
9. reported all notification channels, including both `discount_hunter_free` and
   `discount_hunter_sales`.

The hook uses `run_as = true` because the receiver is not exported: AndroidForge sends the explicit
broadcast as the app UID, which is a debuggable-build-only path. The earlier plain shell broadcast
returned exit 0 without delivering; the postcondition turns that false positive into
`E_HOOK_VERIFICATION_FAILED`.

The current run found no new deal to announce. A separate historical notification run
`20261115-131907-b3ca2d45_conformance-run` recorded `posted 1 notification(s)` and is retained as
notification evidence. It predates the hook postcondition and therefore also demonstrates why the
postcondition matters: only the later run proves the app changed state.

## Why this matters

This flows through the same binary as CalcSuite and IronPulse; only the adapter, fixture, hook and
conformance script changed. It exercised:

- a debug application id different from the release application id;
- a configurable expected debug certificate;
- runtime notification permission;
- a machine-written app-private file seeded without root;
- a real, non-exported receiver reached through `run-as`;
- a declared hook postcondition that distinguishes "ADB exited 0" from "the app did work";
- notification/channel evidence and diagnostics;
- build identity in a conformance report that did not itself install.

## Limits

`run_as` broadcasts require a debuggable build. A release build needs an app-provided exported debug
receiver, a deep-link/activity trigger or instrumentation. Notification channel parsing remains
version-dependent and is labelled best-effort. This was not a physical-device or MIUI/vendor test.
Raw evidence is withheld from the public repository; the run ids above identify the local records.
