# Glossary

**ADB** — Android Debug Bridge, the device control transport used by AndroidForge.

**Adapter** — Project-specific `.androidforge/project.toml` (or `config.toml`) containing package IDs, AVD defaults, fixtures, hooks, timeouts and UI settings. The Rust core contains no project branches.

**Agent** — An autonomous program driving AndroidForge through JSON and exit codes.

**APK identity** — The observed package, versionName, versionCode, local and installed APK SHA-256, signing certificate SHA-256 and debug-signed status recorded after install.

**Artifact** — A file written under a run directory and referenced by a path, kind, size and SHA-256. Screenshots, recordings, logcat, dumps, reports and transcripts are artifacts.

**Attached session** — A session that claims an already-running emulator or physical device instead of starting an emulator.

**AVD** — Android Virtual Device, an emulator definition managed by the Android SDK.

**Conformance** — A thin project manifest (`script`, `maestro` or `fixture`) with expected status, exit code, artifact globs and optional JSON paths. It is not a custom test DSL.

**Device preparation** — Recording and changing reversible settings such as animation scales, stay-awake, locale, timezone, density or orientation, then restoring them on release.

**Failure bundle** — Automatic evidence captured when a meaningful command fails: screenshot, UI hierarchy, focused nodes, logcat tail, crash/ANR state, process/task/foreground state and dumpsys.

**Fixture** — A versioned, explicit project state manifest with phases, ordered steps, retries, timeouts, failure policy and confirmation rules. Fixtures never run automatically.

**Hook** — A named, explicit project debug trigger (`broadcast`, `deep-link`, `shell`, `instrumentation`, `command`). AndroidForge does not interpret the name. A hook can declare a `verify` postcondition; ADB exit 0 alone is not treated as proof that the app changed state. `run_as` sends a broadcast as the debuggable app UID.

**Manifest (run)** — `manifest.json` in a run directory: every run file with size and SHA-256.

**Metadata (run)** — `metadata.json`: tool version, host, project root, toolchain paths, device and session at run start.

**Report** — `report.json`: one versioned JSON record of a run, its steps, status, build identity, artifacts and exit code.

**Run** — The unit of evidence. A run owns a directory, can span native commands and external scripts, and ends with one report.

**Session** — The unit of device ownership. It binds an owner, a device serial, optional emulator process/port, heartbeat and prepared settings. Locks are cooperative between AndroidForge processes. `session stop`, `release` and `restart` require the recorded owner label or an explicit `--force` takeover.

**Stale session** — A session past its heartbeat threshold, or whose owner process is gone. `session gc --reclaim` is explicit; AndroidForge never silently steals a live device.

**UIAutomator hierarchy** — Android's XML dump of accessibility nodes. AndroidForge parses it and selects by text, content description, resource id, class, index and state.

**Verified text entry** — Focusing an editable field, optionally clearing with batched deletes, sending text, re-reading the hierarchy and comparing. Password fields report unverified rather than claiming success.

**`E_UNSUPPORTED_OPERATION`** — A structured, non-stub answer for a capability that is intentionally absent, with the reason and supported alternatives.
