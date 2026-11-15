# CalcSuite Smoke — Current 3.0.3 Updater Flow

This case study summarizes a real Android 16 emulator run from 2026-11-15 against the current
Calculator Suite `3.0.3` release build. Raw run artifacts are intentionally not published because they
can contain machine paths, device logs and app data.

## Install

The install run `20261115-165050-0dc26b94_app-install` recorded:

- package `com.calcsuite`
- versionName `3.0.3`, versionCode `7`
- local and installed APK SHA-256 `ccc2d570010409ebaa2277ac65ba8f78008a84e10b719046b6b488362868fe6a`
- release certificate SHA-256 `55071A2393348B9C2F44DDE98D21277F1666A56040DECD51C0B83F377A6F9E02`
- `debug_signed: false`, `verified: true`
- installed base APK path and data directory
- a compact `build/install-identity.json` artifact

## Conformance

Conformance run `20261115-165054-dc1b9d28_conformance-run` finished with status `passed`, exit code
`0`, using the release binary.

The flow:

1. force-stopped and launched CalcSuite;
2. asserted the visible title `Calc Suite` and the `Categories` shell;
3. opened Settings through its content description;
4. scrolled to `Check for updates`;
5. tapped `Check`;
6. waited for and asserted `You have the newest version (3.0.3).`;
7. captured the `calcsuite-updater` screenshot.

The conformance report carries the observed build identity inherited from the install in the same
session: package `com.calcsuite`, versionName `3.0.3`, versionCode `7`, APK/installed hash and
certificate, `verified: true`. This is the same identity recorded by the install run.

## What this proves

- The same binary installs a current release APK and records build identity.
- The installed APK hash is pulled back and compared, not merely trusted from the local file.
- UIAutomator hierarchy parsing, semantic navigation by content description and text, scrolling and
  exact/contains assertions work on the current app.
- The app's own updater path runs against a real network/device and reports the current version.
- Fixtures, external conformance scripts and checksummed reports work together.
- A later run in the same session inherits the installed build identity.

## Limits

This flow covers the home shell, Settings navigation and the updater. It does not exercise physical
devices, Maestro, instrumentation, AVD cloning or release publishing. Raw evidence is withheld from the
public repository; the run ids above identify the local verification records.
