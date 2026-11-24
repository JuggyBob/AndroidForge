# Changelog

All notable changes to AndroidForge are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses semantic versioning;
the 1.0 line is the supported stable contract. Historical pre-1.0 entries are preserved unchanged.

## [Unreleased]

No unreleased changes. See [1.0.0](#100---2026-11-24) for the current release.

## [1.0.0] - 2026-11-24

**The first 1.0 stable release.** It closes the pre-1.0 contract and safety gaps with explicit breaking
changes, JSON fixture manifests, stronger redaction, the official Android robot icon with third-party
notices, and a release-validation gate. Envelope and report schema stay at `1`. Host gate, packaging
inspection and the two-ROM scope are recorded below: POCO X4 GT full approved upgrade/UI plus REDMAGIC
zero-touch read-only coverage. REDMAGIC UI automation and device actions are **NOT RUN**. See
[v1.0.0 release notes](docs/release-notes/v1.0.0.md).

### Added

- **JSON fixture manifests:** `.json` and `fixture.json` are discovered alongside TOML fixtures, and
  malformed JSON is reported instead of silently skipped.
- **JSON and mock query redaction:** JSON-style key/value secrets and mock-server query strings are
  redacted like headers and bodies in reports and request logs.
- **Official robot icon and notices:** the Android robot with hard hat and wrench is the documented
  application icon; `THIRD_PARTY_NOTICES.md` records the Android/Lucide/font attribution and the two
  font OFL license texts ship in the portable ZIP and installer.
- **Release and validation notes:** `docs/release-notes/v1.0.0.md` adds the 1.0 narrative, breaking
  changes, upgrade notes and the physical-ROM verification checklist; CI and the release workflow now
  fail on version drift, catalog/schema count drift and missing release content.
- **Fresh v1.0.0 desktop gallery:** five native cockpit captures in `docs/screenshots/v1.0.0/`,
  published as `androidforge-screenshots-v1.0.0.zip` when the screenshot ZIP is produced.

### BREAKING CHANGES

- **`notifications clear` requires a selector:** a package/id/tag/title/text selector is now mandatory;
  the old "clear everything by default" behavior is removed. Scripts must name what they clear.
- **More MCP tools require `confirm=true`:** `agent test`, `agent matrix`, `notifications tap`,
  `ui dismiss-overlays`, `test maestro`, `test instrumentation` and `conformance run` are now
  device-mutating, in addition to the previously gated tools. MCP callers must pass `confirm=true`.
- **`snapshot save` / `standby set` reclassified:** these moved from `read-only` to `device-mutation`;
  generated MCP wrappers now require `confirm=true`.
- **Physical-device mutation guards:** `devices prepare`/`devices restore` and `conformance run` now
  enforce `--allow-physical-mutation`; `test maestro`, `test instrumentation` and `conformance run`
  require the same guard. MCP strips `allow_physical_mutation`, so physical use fails closed.
- **`emulator status` rejects non-emulator serials** instead of silently reporting a connected physical
  device. Use `devices inspect` for physical devices.
- **`finalize_pass` fails a `Running` step** instead of reporting it as passed with failed steps.
- **Strict conformance schema `oneOf`:** script/maestro/fixture flows no longer accept each other's
  fields (for example `timeout_secs` only applies where supported). Conformance `version` remains `1`.
- **`--run` warns when no step is appended** instead of silently producing an empty passed run.
- **Generated MCP tools forward `--yes`** to the underlying CLI, so confirmed generated calls no
  longer fail on a missing confirmation flag.

**Migration and operational impact:** no state-file or report migration is required. Sessions, adapters,
reports and artifacts remain readable and keep schema version `1`. Automation that relied on selector-
free notification clearing, on `emulator status` against a physical serial, on implicit `read-only`
classification of `snapshot save`/`standby set`, or on MCP calls without `confirm=true` must be updated
before upgrading. Re-run affected flows after upgrade; existing report artifacts remain valid.

### Changed

- Desktop icon is now the official Android robot wearing a compact hard hat between its antennae and
  holding a wrench. The SVG is the source of truth; see `THIRD_PARTY_NOTICES.md` for attribution.

### Fixed

- `logcat watch --package` now keeps the package's process logs plus lines mentioning the package name,
  matching `logcat query`. When the package is absent, it falls back to name matching and emits a note
  on stderr.
- `app status --expect-version-name/--expect-version-code/--expect-apk-sha256` now reports
  `E_PACKAGE_NOT_FOUND` when the package is absent, including on vendor builds where `pm path` exits
  non-zero for a missing package.
- `notifications clear` now requires a package/id/tag/title/text selector instead of clearing every
  notification by default and reports partial progress on failure. The selector-required error still
  references `notifications tap` as the way to interact with a specific notification; it is not a
  substitute for naming what to clear.
- Fixture discovery/loading now accepts `.json` and `fixture.json` alongside TOML, and malformed JSON
  fixtures are reported rather than silently ignored.
- Recording stop verifies the remote `screenrecord` command line and unique output path before SIGINT;
  emulator stop verifies the host process image name before `taskkill`. PID reuse now fails closed.
- The clear-probe path now revalidates focus and field identity, moves to the end before probing, and
  refuses to treat focus loss as a successful clear.
- `ui text` accepts the requested text when an editable field replaces its hint/placeholder text, and
  probes once after a clear to distinguish a revealed hint from real leftover text; real appends are
  still verified.
- `conformance.schema.json` now uses a `oneOf` for script/maestro/fixture flows so `timeout_secs` is
  advertised only for the flow types that support it.
- Redaction now catches JSON-style `"token":"..."` and similar key/value secrets, and mock-server
  query strings are redacted like bodies and headers.
- App-private push paths are shell-quoted before `run-as ... sh -c`, closing device-side command
  injection through file names; control characters are rejected.
- Device preparation records each previous setting before mutating it, so a failed verification still
  leaves a restore path.
- Hook `must_change` can no longer pass when the before-snapshot failed; a missing before state counts
  as unchanged.
- Session GC and prune run under the global state lock, preventing concurrent operation/lease updates
  from being overwritten.
- `.apks` loading rejects an entry whose declared size exceeds the 2 GiB bound before reading or
  writing its payload; extraction reads through a bounded reader.
- UI scroll/tap reject negative bounds instead of wrapping them to huge device coordinates.
- Device-lock filenames percent-encode unsafe serial characters, so distinct serials cannot collide.
  Locks written by 0.13 with `_`-encoded serials are still honored for ownership, release and GC after
  upgrading to `%XX` encoding, so an active legacy session cannot be bypassed or stolen.
- Unreadable device-lock files now fail closed with `E_DEVICE_BUSY` and manual-cleanup guidance instead
  of being deleted, so an unreadable primary lock cannot remove a colliding legacy lock belonging to
  another serial.
- `finalize_pass` reports a failed run instead of a passed run with failed steps.
- `agent test`, `agent matrix`, `notifications tap` and `ui dismiss-overlays` are classified
  device-mutating; `devices prepare`/`devices restore` and `conformance run` now enforce the physical
  mutation guard.
- `--run` now emits a warning when the selected command does not append a step, instead of silently
  producing an empty passed run.
- `test maestro`, `test instrumentation` and `conformance run` now require the physical-device mutation
  guard; the first two are also classified `device-mutation`, so MCP callers must pass `confirm=true`.
  MCP still strips `allow_physical_mutation`, making physical use fail closed.
- `snapshot save` and `standby set` are classified `device-mutation` instead of `read-only`.
- MCP stdio rejects oversized request lines before allocation with bounded buffering and resynchronizes
  at the next newline instead of parsing the remaining bytes as a new request.
- `emulator status` rejects non-emulator serials instead of silently reporting the connected physical
  device.
- `report explain` maps `E_IME_VISIBLE` to UI automation with a `ui back --hide-keyboard` next action.
- Desktop evidence images are capped at 32 MiB before decoding; updater downloads are HTTPS-only.
- Added font OFL notices and shipped the font license texts in the portable ZIP and installer.

### Compatibility

- Envelope (`schema_version`) and run report schema remain `1`; v0.13 and older reports stay readable.
- The desktop requires an exact CLI version match (`1.0.0`) and refuses a mismatched CLI.
- Supported host matrix is Windows 10/11 x86_64 only; Linux and macOS are not validated.
- Release binaries are unsigned. Verify `SHA256SUMS.txt` and expect SmartScreen warnings.
- Maestro remains optional and best-effort; physical-device mutation is excluded from MCP self-service.

### Validation

- **Full host gate passed after the final lock fix:** `cargo fmt --all -- --check` and strict clippy
  (`-D warnings`) clean; `cargo test --workspace --locked` **392 passed / 0 failed** across 27 suites
  (including 4 new unreadable-lock tests); `cargo deny` clean; release build clean. CLI and desktop
  both report `1.0.0`; 191 catalog commands, 192 MCP tools (also confirmed by MCP `initialize` and
  `tools/list`) and 31/31 schemas.
- **Packaged assets and installer:** final hashes matched `dist/SHA256SUMS.txt`; installer
  `FileVersion`/`ProductVersion` `1.0.0`; ZIPs contained all required docs/notices/fonts; extracted CLI
  and desktop both `1.0.0`; silent install/uninstall exit 0, installed EXE hashes matched
  `target/release`, install directory removed and registry/Start Menu clean. Final hashes are recorded
  in the v1.0.0 release notes.
- **POCO X4 GT / HyperOS 2 / Android 14 (API 34):** approved in-place IronPulse 5.1.0/code 15 →
  5.1.2/code 17 with `decision: installed`, `reason: content_different`, same debug certificate and
  installed base APK SHA-256 `2321cd4619e65502409f9d2f87fceff2cdbcc1c576732948b242edfa74c235e4`;
  `app verify-installed` matched and all expectations passed. Data preserved (65 files / 2389 KB before
  and immediately after; 66 files / 2373 KB after launch), package and settings counts unchanged,
  foreground launch, 84-node observation with 29 clickable matches, screenshot/recording evidence and
  clean session release.
- **REDMAGIC 10 Pro / RedMagicOS 11.0.6_EU / Android 16 (API 36):** 8/8 zero-touch read-only checks
  passed (`doctor` warnings non-blocking, `devices inspect`, `devices health`, `app list` 438 packages,
  `app status` 5.1.2/code 17, `notifications list`, bounded `logcat peek`, screenshot). Before/after
  `pm list packages`, `settings global`, `settings secure`, `/data/local/tmp`, `getprop` and `df` were
  identical; no session was created and no app was launched or modified. Screenshot SHA-256
  `cfa4b6d356132645115fd322ea72e3e5b3fe39ef9fb6ed2511e9c82186568148`. **REDMAGIC UI automation and
  all device actions are NOT RUN** by the explicit zero-touch rule. Emulator evidence remains
  baseline/supporting only; full details are in `VERIFICATION.md`.
- Static release gates are in place: CI and release workflows compare `Cargo.toml`, `androidforge
  --version` and installer `ProductVersion`, check catalog counts (191 commands / 192 MCP tools /
  31 schemas), and require `THIRD_PARTY_NOTICES.md`, both font OFL files and `docs/release-notes/v1.0.0.md`.

### Known limitations

- Maestro is optional/best-effort; physical-device Maestro flows and a real instrumentation test APK
  remain unvalidated. Vendor ROM mutation flows remain best-effort.
- `logcat watch --package` samples package PIDs at startup; if the app restarts mid-stream, name
  matching is the fallback. This is also documented in `logcat watch --package --help`.
- Physical-device mutation is deliberately excluded from MCP self-service.
- Snapshot restore, root-only time changes, macOS/Linux hosts and code signing remain unverified.
- The Scoop manifest pins the final v1.0.0 desktop ZIP hash. REDMAGIC UI automation and device actions
  are NOT RUN by the zero-touch rule.

### Release assets

- `androidforge.exe` and `androidforge-desktop.exe` — standalone CLI and desktop cockpit executables,
  byte-identical to the copies inside the ZIPs and installer. Keep them adjacent for the desktop.
- `androidforge-cli-windows-x86_64.zip`, `androidforge-desktop-windows-x86_64.zip` and
  `androidforge-windows-x86_64-setup.exe` — CLI-only, portable desktop + CLI + docs, and per-user installer.
- `SHA256SUMS.txt` — SHA-256 for every published asset, including the two standalone executables.

## [0.13.0] - 2026-11-22

**Optional Maestro, desktop Tool database and a complete MCP command surface.** This release keeps the
truthful verification work from 0.12 and adds discoverability, optional external tooling, a
compatibility policy and a security-hardening pass.

### Added

- **Pinned optional Maestro installer:** `tools install maestro` downloads the official Maestro CLI
  2.12.1 Windows ZIP (about 350 MB), verifies the compile-time pinned SHA-256
  `54dbb0716425626604973cbb374c585ca30272375d9ce15177a1152765ddd9d7`, extracts with zip-slip and size
  limits, and installs under the AndroidForge state tools directory. `tools status maestro` reports the
  active path, source (`androidforge-managed` or `external`), managed version and Java. `tools uninstall
  maestro --yes` removes only a managed copy. `--dry-run` previews URL/hash/target.
- **Desktop Maestro controls:** Setup → Environment shows the Maestro check with an **Install Maestro**
  button, a confirmation dialog listing the pinned version/URL/hash/target, progress state, and the
  active path with Copy path. A CLI installation is detected automatically through the shared managed
  tools directory.
- **Desktop Tool database:** a new Workspace sidebar page with a help icon, search across command paths,
  summaries, arguments and help text, A-Z/Z-A sorting, safety badges, and full detail pages with usage,
  examples, argument tables and generated MCP tool names. Opening a tool uses a short slide/fade.
- **Complete CLI catalog:** `tools catalog --json` derives 191 leaf commands from clap, including
  arguments, required flags, possible values, defaults, examples and safety classes.
  `tools-catalog.schema.json` documents the shape.
- **Complete MCP surface:** the tool catalog now exposes **192 tools**. Existing curated tools keep
  their hand-written schemas; every other bounded, non-unsafe CLI leaf command is generated from the
  same catalog. Raw `adb`/`run exec` and the nested `mcp serve` are excluded; `network mock serve` is
  exposed only through the bounded curated tool. `androidforge://catalog` exposes the full catalog as an MCP resource.
- **Compatibility and deprecation policy:** `COMPATIBILITY.md` defines stable CLI flags, envelopes,
  error/exit codes, report schema 1, curated MCP tools and the deprecation window; generated tools and
  experimental/best-effort integrations are explicitly additive or exempt.
- **Physical-device validation evidence:** install, live verification, same-version byte-different
  `--only-if-changed`, UI dump/find/tap, screenshot, 5s recording and out-of-band refresh were exercised
  on the Xiaomi POCO X4 GT for an isolated debug package, then the package was uninstalled.

### Changed

- Maestro remains optional/best-effort, but AndroidForge now owns a tested, pinned installation path
  instead of requiring a floating global install.
- MCP generated tools use a `--` delimiter for positionals and never auto-forward
  `--allow-physical-mutation`; physical mutation requires an out-of-band CLI/user decision.
- Raw `adb` and `run exec` are classified `unsafe`, hidden from generated MCP tools and shown with a red
  safety badge in the desktop database. They remain available to a human/CLI that intentionally invokes
  them.
- Mock server exposes a confirmation-gated `--allow-external-bind`; loopback remains the default.
- `tools schema` grows from 40 to 192 tools; README/guides use the new count.

### Fixed / security hardening

- Fail-closed device classification: only `emulator-*` serials are treated as unprotected emulators;
  unknown/vendor devices require `--allow-physical-mutation`.
- Physical-device guards added to `app launch`, `broadcast`, `service`, `force-stop` and `kill`.
- Package names are validated before `run-as`/external path interpolation; external push/pull paths
  cannot escape the app directory via `..`.
- App-data restore validates tar members and rejects absolute/parent paths, symlinks, devices and other
  non-regular entries.
- Session GC no longer reclaims a stale session with a live emulator or live operation unless `--stop`
  is explicit; device locks cannot be freed underneath a running operation.
- MCP `tools/call` now accepts only names in the advertised tool set. A hidden catalog entry
  (`androidforge_adb`, `androidforge_run_exec`, long-running servers) is rejected before argv
  construction; the generated fallback enforces the same safety class check.
- MCP `tools/call` has a 600s wall-clock limit, 32 MiB/8 MiB output caps and process-tree termination;
  JSON-RPC request lines are capped at 16 MiB.
- Generated MCP tools forward array-valued `serial` targets (for example `agent matrix`) instead of
  dropping them, and catalog safety classes now require `confirm=true` for `report prune`,
  `app data-restore` and `doze unforce`.
- Mock server has per-connection timeouts, a 64-connection cap, exact `Content-Length` body slicing,
  CRLF header validation and body-log redaction; sensitive headers are expanded.
- Maestro staging cleans up on extraction/target errors; desktop updater uses random temp staging,
  caps executable extraction and requires GitHub release URLs.
- Desktop CLI pipe capture is capped; preview frames over 32 MiB are rejected; run paths must contain
  `report.json` before desktop reads them.
- Desktop Maestro install now goes through the normal confirmation dialog.

### Validation

- `cargo test --workspace`, strict clippy and fmt (delegated final gate).
- `tools catalog`: 191 commands; `tools schema --format mcp`: 192 tools.
- Maestro installer/status/uninstall tested in an isolated home; doctor reports version `2.12.1` and
  the managed path. Passing and deliberately failing Maestro flows were executed on a disposable
  API 36 emulator with `result.version:"2.12.1"` and no duplicate/analytics side effects.
- Mock HTTP/TLS/redaction/timeout smoke, MCP generated tool calls, desktop 15-page smoke and
  multi-emulator + physical session-ownership campaigns are recorded in `VERIFICATION.md`.
- Physical-device evidence: baseline hash `22DDD58E…`, modified hash `198A1EF3…`, same cert, update
  installed with `reason:content_different`, out-of-band refresh reported the baseline hash, cleanup
  removed only `com.aster.localai.debug`.

### Compatibility

- Report schema remains `1`; v0.12 reports remain readable.
- Existing CLI command names, envelope and exit codes are preserved.
- New fields/schemas/MCP tools are additive; see `COMPATIBILITY.md`.
- The desktop and CLI must both be `0.13.0`.
- Desktop screenshots remain the v0.11.0 gallery because the visual system is unchanged; the new Tool
  database is documented with its own page description.

### Known limitations

- Maestro is optional/best-effort. The installer is Windows-only and pins one tested version.
- Physical-device validation covers one Xiaomi HyperOS debug package; other vendor ROM install flows
  remain best-effort.
- Maestro flows were validated end-to-end on a disposable API 36 emulator (passing and deliberately
  failing). A real instrumentation test APK and physical-device Maestro flows were not validated.
- Snapshot restore, root-only time changes, macOS/Linux hosts and code signing remain unverified.
- Raw `adb`, `run exec`, and physical-device mutation are deliberately excluded from MCP self-service.

## [0.12.1] - 2026-11-22

**Dependency and supply-chain patch for the v0.12.0 stable line.** This is a normal stable release and
supersedes the v0.12.0 assets. It contains no intentional feature changes.

### Changed

- Updated the transitive `url` crate to `2.5.9`, which fixes RUSTSEC-2026-0592 (stack overflow in
  `Url::origin()` on nested `blob:` URLs). The vulnerable function is not called by AndroidForge or its
  `webbrowser` use path, but the published v0.12.0 desktop bundle contained `url 2.5.8`.
- Replaced the unmaintained `rustls-pemfile` crate with the PEM parser built into `rustls-pki-types`.
  Mock TLS certificate/key loading keeps the same behavior and error messages.
- Workspace, CLI, desktop, installer and compatibility version are `0.12.1`.

### Fixed

- `cargo deny check advisories bans licenses sources` is clean again. The v0.12.0 tag predates
  RUSTSEC-2026-0592, so its published assets retain the advisory; v0.12.1 is the corrected build.

### Validation

- `cargo fmt --all -- --check`, strict clippy and `cargo test --workspace` pass (347 tests, 0 failed).
- Local `cargo deny check advisories bans licenses sources`: advisories, bans, licenses and sources
  all report `ok`.
- Mock TLS was re-tested with a self-signed certificate after the PEM parser migration.
- Package/ZIP/installer versions report `0.12.1`; published checksums are recorded in
  [VERIFICATION.md](VERIFICATION.md) and below.
- Release workflow succeeded: [run 45023428623](https://github.com/JuggyBob/AndroidForge/actions/runs/45023428623).

Published asset checksums:

```text
23663c21218c3916c520edadf81f390af2a46c2ff8cb31693fa0ce7f7262fc09  androidforge-cli-windows-x86_64.zip
0d808b6130543f990fb8bc92ce6603e6b43bae31b76c420406389379568447ed  androidforge-desktop-windows-x86_64.zip
b7fbe33ada7334ded37027fa8229e84ac01441ea5bf1ffb24210d0fda70f2371  androidforge-windows-x86_64-setup.exe
```

GitHub release: <https://github.com/JuggyBob/AndroidForge/releases/tag/v0.12.1>. A normal stable release,
not a prerelease.

## [0.12.0] - 2026-11-22

**Truthful Verification & Automation Gap Closure — the first stable (pre-1.0) release.** This release
closes the gaps found during the IronPulse 5.1.2 device campaign. The central rule is that AndroidForge
must prove what is actually on the device instead of trusting cached install records, version strings or
a process exit code.

### Added

- **Live install verification:** `app verify-installed --apk <path>` pulls the live base APK, hashes it,
  reads live package metadata and compares package, version, certificate and split identity without
  installing anything. Mismatches exit with `E_INSTALL_VERIFICATION_FAILED` and a full diff.
- **Install expectations:** `app status <package> --expect-version-name`,
  `--expect-version-code` and `--expect-apk-sha256` evaluate live device state and return structured
  pass/fail records. `--assert-task-exists`, `--assert-task-absent` and `--assert-process-stable`
  remain available alongside them.
- **Live session build refresh:** `session status` and `session inspect` re-read `pm path`, version and
  base APK SHA-256 on every call. The refreshed identity is persisted so later reports do not inherit a
  stale install record after an out-of-band `adb install -r`.
- **Compose test-tag selector:** selectors accept `--test-tag` as an alias for the accessibility
  resource id. A missing tag produces an actionable suggestion naming the app-side
  `testTagsAsResourceId` debug setting; `ui find` remains a non-failing search.
- **Configurable long-press:** `ui long-press --duration <ms>` replaces the fixed 900 ms hold (default
  remains 900 ms). Referenced long-presses accept the same duration.
- **Drag gestures:** `ui drag` performs a real press-move-release drag with `--from`/`--to` selector
  expressions (`text=`, `contains=`, `desc=`, `id=`, `tag=`, `class=`), explicit coordinates, text/id
  flags, `--duration`, `--hold-before` and `--hold-after`.
- **Mid-gesture screenshots:** `--capture-mid <png>` captures while the finger is still down during a
  long-press or drag. The frame and SHA-256 are returned in the action result.
- **Wait modes:** `ui wait --for-text-gone <TEXT>` (repeatable) waits until every supplied string is
  absent and reports lingering matches with bounds. `--for-idle` waits for a byte-identical hierarchy
  dump; `--for-stable --stable-for <duration>` requires a stable hierarchy hash over a window.
- **Text absence assertion:** `ui assert-no-text --text/--contains` fails with
  `E_UI_ASSERTION_FAILED` and the matching nodes when the text exists; success includes attempts,
  duration and package evidence.
- **App data fixtures:** `app data export`, `app data import`, `app data reset` and `app data seed`.
  Export/import/seed use `run-as` and require a debuggable build. Import always writes and registers a
  `data/pre-import-backup.tar` before restoring. Reset clears data, optionally saves
  `--backup-current`, then restores `--baseline`. Seed writes one file to an app-private relative path,
  reads it back and reports `remote_sha256`/`remote_hash_verified`.
- **ADB reverse management:** `network reverse add/list/remove/clear` manages per-device mappings and
  records them in the session for cleanup.
- **Local mock HTTP/TLS server:** `network mock serve` binds loopback by default with configurable port,
  inline `METHOD /path=STATUS[:BODY]` routes or a JSON routes file, default status/body, delay, request
  budget, duration and optional TLS certificate/key. `--reverse` adds and later removes an
  `adb reverse` mapping automatically.
- **Mock request logs:** NDJSON records method, path, query, redacted headers, bounded body preview and
  SHA-256, status, matched route, duration and peer. Sensitive headers (`authorization`, cookies,
  API keys) are redacted. The log is registered as a run artifact.
- **Visual regression:** `capture screenshot --compare <baseline> [--region x1,y1,x2,y2]
  [--max-diff-ratio N] [--diff-threshold N]` reports compared/changed pixels, ratio, mean and maximum
  channel delta and the changed bounding box. A diff PNG is stored as an artifact; failure exits with
  `E_UI_ASSERTION_FAILED`.
- **Tap evidence:** `ui tap --capture-before <png> --capture-after <png>` links before/after frames in
  the action result or receipt.
- **Session hygiene filters:** `session list --mine --active [--owner NAME]` filters by the
  `ANDROIDFORGE_OWNER`/OS owner and active states. Each entry carries `classification`
  (active/busy/stale/released/failed/abandoned), age and a human-readable `reason`.
- **GC guidance:** `session gc --reclaim --dry-run` adds `would_reclaim` and `next_commands`, leaves every
  session file untouched and never reports a live or tombstoned session as reclaimed.
- **Schemas:** `app-verify.schema.json`, `app-data.schema.json`, `ui-wait.schema.json`,
  `network-mock.schema.json`, `visual-diff.schema.json` and `session-list.schema.json`.
- **MCP/tool catalog:** nine new tools cover verify-installed, app status/data, drag, wait,
  assert-no-text, reverse, bounded mock serve and session list. `--test-tag` and screenshot comparison
  flags are exposed through the selector/screenshot schemas.

### Changed

- **`--only-if-changed` is byte-truthful:** a rebuild with the same package/version but different base
  APK bytes is installed (`reason:content_different`). Installation is skipped only when package,
  version code, base APK SHA-256 and the sorted split hashes all match. Missing hashes are `unknown` and
  never safe to skip.
- `ui tap` captures are explicit and bounded; they are not silently added to every action.
- `ui long-press`, `ui drag`, `ui wait` and `ui assert-no-text` emit structured action evidence,
  including capture hashes and lingering-match bounds.
- `app data seed` verifies the remote file through a `run-as` read-back instead of trusting only the
  push exit code.
- Session list JSON omits `owner` when unfiltered and includes it only for `--mine`/`--owner`.
- Session GC distinguishes dry-run `would_reclaim` from applied reclamation.
- Release workflow now publishes stable releases without the `--prerelease` flag by default. Existing
  v0.11.0 remains a prerelease; v0.12.0 is a normal release.
- Workspace, CLI, desktop, installer and compatibility version are `0.12.0`.

### Fixed

- `session status` could report a stale install path/SHA-256 after an out-of-band `adb install -r`.
- `app install --only-if-changed` could skip a same-version rebuilt APK whose bytes had changed.
- Empty candidate/installed hashes were treated as comparable values; they are now `unknown` and block
  skip decisions.
- `network reverse list` labels the ADB transport token as `transport` instead of pretending it is a
  device serial.
- Mock-serve rejects port 0, validates status codes and route paths, and cleans up an automatically
  added reverse mapping after the server stops.
- `ui assert-no-text` and text-gone waits omit absent `hierarchy_package` instead of emitting `null`.
- Session-list schema accepts `owner: null` from older reports while the current CLI omits the field.
- Visual-diff payloads validate against their schema, including region-only comparisons.

### Validation

- `cargo test --workspace`: **347 passed / 0 failed / 0 ignored** across 27 test executables.
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- Live install truth: after `adb install -r`, `session status` returned `live_build_refreshed:true`
  with the live base APK SHA-256.
- `--only-if-changed`: a same-package/same-version signed rebuild with an added asset installed with
  `skipped:false`, `reason:content_different`; re-installing the identical baseline skipped.
- `app verify-installed` reported `overall:same` and `safe_to_skip_install:true`; `app status
  --expect-apk-sha256` passed against the live hash.
- Disposable-emulator campaign passed for long-press duration, drag/mid-capture, text-absence tests,
  idle/stable waits, app-data export/seed/import/reset, adb reverse, screenshot comparison and tap
  before/after captures.
- Mock HTTP returned 200/401 and 404 fallback, honored `--max-requests`, redacted `authorization` and
  wrote the request log. Self-signed TLS served successfully with `curl -k`.
- Session hygiene was tested with fabricated fresh/stale/released/failed sessions: classification,
  owner filters, dry-run immutability and reclaim-only-stale behavior passed.
- No physical device was mutated; the disposable emulator was released and stopped.

### Compatibility

- Report schema remains `1`; v0.11.0 reports remain readable.
- Existing command names, JSON envelopes, exit codes and session ownership behavior are preserved.
- The `--only-if-changed` skip condition is stricter. A build that previously skipped on matching
  version metadata now installs when hashes differ; this is the intended fix.
- New JSON result fields are additive; new schema files are additive.
- The desktop and CLI must be on the same `0.12.0` version line.
- The desktop UI is unchanged from v0.11.0; the v0.11.0 screenshot gallery remains representative.

### Known limitations

- Physical-device installs and vendor ROM mutation paths remain incompletely validated. The physical
  Xiaomi device was present but not mutated.
- Maestro and instrumentation execution remain optional/best-effort; a real Maestro flow and test APK
  were not exercised in this release.
- Snapshot restore and root-only time changes remain incompletely validated.
- Real Tesseract accuracy is engine- and language-data-dependent.
- MCP is local stdio only; no HTTP/SSE transport is shipped.
- macOS and Linux hosts are not validated.
- Compose tags require the app-side `testTagsAsResourceId` setting; AndroidForge does not inject it.
- TLS mock serving requires the app/device to trust the supplied certificate through its own
  network-security or user-CA configuration.
- The desktop is Windows-focused; release binaries are unsigned.

### Release assets

- `androidforge-cli-windows-x86_64.zip` — standalone `androidforge.exe`.
- `androidforge-desktop-windows-x86_64.zip` — portable CLI + desktop, license and documentation.
- `androidforge-windows-x86_64-setup.exe` — per-user Windows installer.
- `SHA256SUMS.txt` — checksums for the published assets.

Published asset checksums:

```text
d68f10a43991c4cbd3e40aa2d0a06d00126023ba7edf0748c59b461265552e67  androidforge-cli-windows-x86_64.zip
d7dfe246c6bf5e28beddf7a13a8ec57931e79d0eee2fa99f8c13a19f9e53d181  androidforge-desktop-windows-x86_64.zip
b89cd32077d0d18dd60d10c16aebc8c868e3034f87196cfd8dc24b64a566e872  androidforge-windows-x86_64-setup.exe
```

GitHub release: <https://github.com/JuggyBob/AndroidForge/releases/tag/v0.12.0>. A normal stable
release, not a prerelease. All assets are unsigned.



## [0.11.0] - 2026-11-19

**Desktop & Agent Onboarding beta.** The CLI contract is unchanged. This release rebuilds the native
cockpit around consistent cards, rows and typography, completes Setup with a copyable agent handoff,
and fixes the run-resume failure-history edge case.

### Added

- **Setup Agent Handoff page:** an eighth and final wizard step shows a ready-to-send prompt containing
  the full paths to `AGENT_GUIDE.md`, `USER_GUIDE.md`, `CAPABILITIES.md`, `README.md`, `EXIT_CODES.md`
  and `PROJECT_ADAPTER.md`, the CLI executable, the workspace and the recommended agent loop. The
  prompt is copyable with one button; the documentation folder and CLI path are also copyable/openable.
- **Setup animations:** switching wizard sections now fades and slides the content pane and glides the
  active-step highlight in the left rail. Whole-row step clicks, Back/Continue and direct step jumps
  share the animation.
- **33-font catalog:** embedded JetBrains Mono plus installed Windows fonts including Segoe UI, Aptos,
  Bahnschrift, Calibri, Candara, Cascadia Code/Mono, Comic Sans, Consolas, Constantia, Corbel,
  Courier New, Ebrima, Franklin Gothic, Gabriola, Georgia, Impact, Ink Free, Lucida fonts, Microsoft
  Sans Serif, Palatino Linotype, Segoe Print/Script, Sylfaen, Tahoma, Times New Roman, Trebuchet and
  Verdana. The picker keeps live `Lorem Ipsum` previews and persists the choice.
- **Recursive Inspector cards:** each hierarchy object is a row card. Expanding a row turns it into a
  card containing child row cards; children recursively expand into deeper cards. Clicking anywhere on
  a row works, the body slides open/closed, and collapsing returns to the initial state.
- **Screenshot gallery:** 16 current screenshots covering every major page, Setup Agent Handoff and
  light mode, published under `docs/screenshots/v0.11.0/` and as a release asset.
- **Documentation navigation:** the release notes and README link the screenshot gallery and the agent
  handoff flow.

### Changed

- **Desktop visual overhaul:** GitHub-dark and light palettes, reusable card gutters, rounded cards,
  thin status pills with larger text, taller row tables, a lighter raised surface behind every row,
  solid right-side scrollbars and a short page slide-up transition.
- **All row tables:** Sessions, Ownership, Runs, Overview recent runs, Maintenance lists and Inspector
  use the same raised rounded row card, aligned columns and hover/selection treatment.
- **Topbar:** Settings moved to the sidebar; Refresh is a compact rounded button with a hand-painted
  circular-arrow icon, hover highlight and UI Automation Invoke support; the Online/Offline pill is
  height-matched to Refresh.
- **Bottom-left CLI card:** compact two-line layout, path is middle-truncated and clipped inside the
  card, full path appears in the hover tooltip, and the whole card is one accessible button that opens
  a persistent `cmd /K` console running the CLI.
- **Settings page:** connection, workspace, owner, theme, accent, font, updates and about are now one
  scrollable page. The color wheel, brightness slider and font picker use the same visible slider and
  hover styling as the Inspector.
- **Light mode:** selected sidebar text, setup step labels, font-picker rows and segmented controls use
  theme-aware foreground colors; nav group and unselected nav colors are darker for readability.
- **Setup finish flow:** the old summary page now leads to Agent Handoff; the footer shows Step 8 of 8
  and Finish setup. Changing sections resets the page scroll to the top.
- **Sessions loading:** the page shows a spinner instead of a false “No sessions” state while the CLI
  list is in flight.
- **Inspector refresh:** manual Refresh is ignored while an automatic frame/hierarchy dump is already
  running, preventing concurrent UIAutomator failures.
- **Matrix copy:** `agent matrix --help` and the desktop help text describe the implemented parallel
  worker model instead of calling parallel execution unvalidated.
- Workspace, CLI, desktop, installer and compatibility version are `0.11.0`.
- Repository: local-only `SPEC-v*.md` drafts are no longer tracked or published; they remain on the
  developer machine and are ignored by `.gitignore`.

### Fixed

- `run resume --from-checkpoint` preserves a `failure_history` record when the previous attempt was
  finalized with `run end --status failed` (or another non-success terminal status) without a
  structured step failure. The record carries the terminal status and exit code.
- CLI card opened the executable in a console that immediately closed; it now runs `cmd /K` with a
  correctly quoted path and remains at a prompt.
- CLI card path overflowed the card; it is now clipped and middle-truncated with a full-path tooltip.
- Topbar Refresh was unnecessarily wide because of leading spaces; the icon and label are now laid out
  compactly at 98×37 px versus the 83×27 px Online pill, with the icon painted instead of using a
  missing font glyph.
- Light-mode selected sidebar text was invisible; selected and unselected nav colors are now readable.
- Overview warning rows rendered with zero gap (`maestronot found`) because the two-column layout set
  horizontal item spacing to zero; spacing is restored without changing the column widths.
- Brightness and Inspector interval sliders could be invisible on dark surfaces; both now use a 6 px
  rail, visible handle, accent trailing fill and themed inactive background.
- Run resume failure history, setup scroll reset, matrix help copy, and concurrent Inspector refresh
  races are covered by the updated regression suite.

### Release assets

- `androidforge-cli-windows-x86_64.zip` — standalone `androidforge.exe` for PATH and automation.
- `androidforge-desktop-windows-x86_64.zip` — portable `androidforge.exe` + `androidforge-desktop.exe`,
  license and documentation.
- `androidforge-windows-x86_64-setup.exe` — per-user Windows installer with both executables, Start Menu
  shortcuts and uninstaller; user data outside the install directory is preserved.
- `androidforge-screenshots-v0.11.0.zip` — 16 PNG screenshots of the v0.11.0 desktop UI.
- `SHA256SUMS.txt` — SHA-256 checksums for the CLI, desktop and installer assets.

Published asset checksums:

```text
6e200c25093a1e1d089dc23b8cbef9688b258c5d0e9084f304882af147bb9a2b  androidforge-cli-windows-x86_64.zip
c4f08e41b3e7e20e568b815505a6f9e998f2c8c21852040124fd1d06a58bc7bf  androidforge-desktop-windows-x86_64.zip
67660a091905a1c69f19d651800d24a3e0ea7bbb18591de5064decf5e2f8b98d  androidforge-screenshots-v0.11.0.zip
c24b2d234546f2d37de58ca9a2dc56aaff6cfaa060681930bb5412e3b7a8076d  androidforge-windows-x86_64-setup.exe
```

All binaries are intentionally unsigned. Windows SmartScreen may warn on first launch.

### Validation

- `cargo test --workspace`: **335 passed / 0 failed / 0 ignored**, 27 test executables.
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- Desktop smoke: **14/14 pages PASS**, 0 blank pages, seeded Sessions/Ownership/Runs data, correct
  breadcrumbs, no leftover processes.
- Setup Agent Handoff: clipboard prompt verified with all six full documentation paths, CLI path,
  workspace and command loop; 8-step footer progression verified.
- Setup animations: measured content fade/slide and step-rail highlight glide over ~320 ms.
- Recursive Inspector: live emulator test progressed **1 → 2 → 3 nested card levels → 1**; whole-row
  clicks and animated collapse verified; isolated emulator and session cleaned up.
- Light mode: selected sidebar item readable; settings left unchanged.
- Topbar Refresh: 98×37 px button vs 83×27 px Online pill, hover fill/border change, UIA Invoke
  confirmed by the workspace-health timer reset and CLI refresh calls.
- CLI card: path text right edge 12 px inside the card border; full path available in the tooltip;
  persistent console showed CLI help and the command prompt.
- Screenshot gallery: 16 images from the v0.11.0 release binary.

### Known limitations

- Physical devices, vendor ROM installs, Maestro execution, a real instrumentation package, snapshot
  restore and root-only time changes remain incompletely validated.
- Real Tesseract accuracy is engine- and language-data-dependent.
- MCP is local stdio only; no HTTP/SSE transport is shipped.
- macOS and Linux hosts are not validated.
- The desktop is Windows-focused; code signing is not included.
- Screenshots and test evidence may expose local project paths and emulator labels; review them before
  publishing in a different environment.

## [0.10.0] - 2026-11-18

Agent-surface release. Seven capabilities requested for the next minor line, delivered together and
verified on live emulators.

### Added

- `mcp serve`, a local stdio Model Context Protocol server (JSON-RPC 2025-06-18). It exposes 31 tools
  and 29 resources, shells out to the same CLI and refuses destructive tools without `confirm=true`.
- `tools schema --format mcp|openai|json` exports the same tool catalog for other function-calling
  clients.
- Selector v2: `--role`, `--text-regex`, `--region`, `--within`, `--above`, `--below`, `--left-of` and
  `--right-of`. OCR fallback via `--ocr-fallback`, using `tesseract` by default and
  `ANDROIDFORGE_OCR_COMMAND` as an override.
- Resumable runs: `run checkpoint`, `run resume --from-checkpoint`, append-only `events.ndjson`,
  `run events [--follow]` and `report repro`. Reports keep `checkpoints`, `resume_count`,
  `resumed_from_checkpoint` and `failure_history`.
- Structured `diagnostics logcat query` with level/package/time/regex filters and
  `diagnostics logcat watch` NDJSON streaming for log, crash and ANR events.
- Generic intents: `app launch --component/--action/--data/--mime-type/--category/--extra/--flag/
  --wait/--force-stop`, `app broadcast` and `app service`.
- `network proxy get|set|clear`, `network profile list|apply` and `network restore`. Profiles are
  `default`, `offline`, `wifi-only`, `data-only` and `no-proxy`; state is recorded per session.
- `session ensure --wait --timeout` for busy devices and `agent matrix --parallel 1..32`, which runs
  one independent child matrix per target, assigns ports under a cross-process lock, writes
  per-target child logs and reports, and aggregates partial results.
- `tests/mcp-smoke.ps1`, a real stdio MCP handshake, tools, resources and shutdown harness.

### Fixed

- Parallel `session ensure` calls can no longer select the same emulator port before either session
  record exists. Port probing and session creation are serialized, and live session ports are treated
  as occupied.
- Parallel matrix workers can no longer hang the parent on inherited stdout/stderr pipe handles.
  Child output is file-backed, the direct child is awaited with a timeout, and a timed-out child is
  killed.
- No-wait `am start` output containing `Starting: Intent` is accepted as success, so
  `app launch --component` works without `--wait`.
- `network profile list` no longer requires a connected device.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.10.0`.
- `matrix.schema.json` documents `parallel` and `child_report`.
- Capability reporting adds MCP, tool schema, selector v2, OCR fallback, run resume, structured
  logcat, intents, network profiles and parallel matrix.

### Validation

- MCP smoke: **10/10 PASS**, 31 tools, 29 resources, protocol `2025-06-18`, clean shutdown.
- Selector/OCR: 21 UI tests pass; live fake-OCR find/tap succeeded; missing OCR engine returned
  `E_TOOL_NOT_FOUND`.
- Runs: checkpoint, failure, resume, NDJSON events and exact-argv repro verified live; core test
  covers the full sequence.
- Logcat: injected `AF_WATCH_SENTINEL` was returned by both query and watch.
- Intents: action start, explicit component (no-wait and wait) and broadcast passed; service stop
  returned the expected command failure for a missing service.
- Network: proxy set/get/clear, `wifi-only` apply and restore passed.
- Devices: busy-device wait returned `E_DEVICE_BUSY` after ~6.8s for a 6s timeout.
- Matrix: two AVDs, `--parallel 2`, **2 passed / 0 failed** in 7.2s with distinct serials and released
  sessions.

### Not validated

- Real Tesseract accuracy. OCR was exercised with a deterministic local test engine; the default
  `tesseract` binary was not installed on the verification host.
- MCP over HTTP/SSE. Only local stdio is implemented.

## [0.9.4] - 2026-11-18

Safety and repeatability release. Closes the v0.9.3 long-horizon certification finding and the
follow-up recommendations made before final UI polish.

### Added

- `--allow-physical-mutation` (global). Data-changing commands now refuse physical devices by
  default with `E_PHYSICAL_DEVICE_MUTATION_BLOCKED`; `--dry-run` reports
  `requires_physical_mutation` without changing anything.
- `ui dismiss-overlays` for the SystemUI immersive "Got it" overlay on fresh AVDs. It only taps
  clickable `com.android.systemui` nodes labelled `Got it`, `OK`, `Dismiss` or `Close`.
- `report bundle --metadata-only` excludes every run artifact while keeping the manifest, summary
  and explanation.
- `session start|ensure --snapshot-save` allows manual emulator snapshots by omitting the default
  `-no-snapshot-save` flag.
- `tests/long-horizon.ps1`, `tests/real-device.ps1`, `tests/upgrade-rollback.ps1`,
  `tests/fault-injection.ps1` and `tests/supply-chain.ps1`, plus `tests/README.md`.
- `deny.toml` and a CI `supply-chain` job for advisories, licenses, bans and sources.
- `docs/LONG_HORIZON_TESTING.md`, `docs/REAL_DEVICE_COMPATIBILITY.md` and `docs/SUPPLY_CHAIN.md`.

### Fixed

- `app uninstall` now requires `--yes`, matching `clear-data` and `data-restore`.
- Action receipts collect complete actionable/context lists before classification, so changes
  outside the compact cap are not reported as `no_visible_change`.
- `snapshot list` parses the emulator's tabular output; `snapshot save` polls for the asynchronous
  snapshot to appear instead of failing immediately.
- `capture record stop` reports the actual recording duration instead of the stop/pull duration.
- Windows state replacement uses `MoveFileExW` with replace/write-through semantics, closing the
  remove-before-rename crash window.
- Empty and system-overlay hierarchies remain transient/retryable, and overlay dismissal is now an
  explicit safe action.
- Workspace packages declare `license = "MIT"` so the license inventory has no unlicensed entries.

### Changed

- `session status`/`session inspect` are documented as the heartbeat source; there is no standalone
  `session heartbeat` subcommand.
- Maestro and instrumentation are reported as `best-effort` and not end-to-end validated.
- Snapshot support is documented as requiring `--snapshot-save` on AndroidForge-launched emulators.
- Workspace, CLI, desktop, installer and compatibility version are `0.9.4`.

### Validation

- Full workspace tests, fmt, clippy and release build passed.
- Long-horizon harness: 0 failures with screenshots, a recording, performance samples and a
  checksum-verified bundle.
- Real-device harness on Xiaomi HyperOS: 23 PASS, 0 FAIL, 1 N/A; debug install blocked by vendor
  policy and recorded as BLOCKED.
- Upgrade/rollback from 0.9.2 to 0.9.4: state and run history preserved; uninstall clean.
- Fault injection: stale locks, corrupt session/report JSON, invalid adapter fields and concurrent
  run writers all handled with structured outcomes.
- Supply chain: 525 packages, 0 unlicensed; CI runs cargo-deny.

## [0.9.3] - 2026-11-17

Contract and coverage release. All eight pre-UI hardening items were delivered together.

### Added

- Formal JSON Schema validation in `cargo test --workspace`: every released schema document is
  compiled as Draft 2020-12 and representative context, agent brief, agent test, observation,
  action receipt, UI diff, performance, diagnosis and bundle payloads are validated.
- `[storage] observation_retention` in the project adapter, global config, generated defaults and
  `schema/adapter.schema.json`. Default `20`, valid range `2..=1000`.
- Cross-process `StateLock` in `androidforge-core`; observation token allocation, writes and pruning
  now hold it so parallel CLI processes cannot reuse tokens or race retention.
- `desktop/tests/smoke.ps1` and `desktop/tests/README.md`: a local release gate that launches all 13
  desktop pages, captures screenshots, checks rendering, records the Windows UI Automation tree and
  invokes Refresh on overview.
- `EXIT_CODES.md` consistency test that checks every documented code, exit number and recoverability
  flag against `ErrorCode::ALL`.

### Fixed

- Empty UIAutomator hierarchies are now classified as transient capture failures and retried instead
  of returning a blank successful screen.
- Nested adapter sections and hook entries reject unknown fields instead of silently ignoring them.
  `config validate` and `context` return `E_CONFIG_INVALID` naming the offending field.
- `schema/agent-test.schema.json` allows `run_id: null`, matching the product before a run exists.
- Brief `agent test` stages keep the schema-required `command_argv` field as an empty array instead of
  removing it.
- Observation retention is configurable and still never evicts the record just written.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.9.3`.
- The 26 checks previously recorded as `NOT RUN` in the certification matrix were exercised against a
  clean API 36 AVD and are recorded in `VERIFICATION.md`.
- `session stop` is documented as owner-guarded, not `--yes`-guarded, in the user and agent guides.

### Not validated

- **Physical device (item 4): BLOCKED.** The Poco X4 GT / HyperOS 2 device was not connected during
  this release. No physical-device behavior is claimed.

## [0.9.2] - 2026-11-17

Patch for the six product findings from the independent v0.9.1 certification run.

### Fixed

- `ui tap --ref` now resolves `actionable_via: "ancestor"` context nodes correctly when the
  observation was captured with `--full`. The full-hierarchy copy carries the same readiness
  metadata, and ref lookup prefers the compact context node.
- Commands that load a session now refuse a session bound to a different project with
  `E_SESSION_PROJECT_MISMATCH`. `context` ignores the foreign session and returns an explicit
  warning instead of silently combining projects.
- `hook run` now returns `verified` and `verification_state`. A hook with no `verify` block is
  reported as `unverified` with a note that shell exit 0 is not route proof. The shipped IronPulse
  hooks verify `CURRENT STREAK` and `Appearance`.
- `diagnostics logcat start` follows from the current logcat tail (`-T 1`) and treats a bounded
  capture that reaches its byte cap as `completed` (`completed_reason: "max_bytes"`) instead of
  failing with "logcat capture process exited immediately". `stop` finalizes the completed capture.
- Action receipts with a satisfied postcondition and an empty semantic diff now add explicit
  evidence that the expected state may already have been visible before the action.
  CalcSuite's live preview makes `=` correctly `no_visible_change`.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.9.2`.
- Error taxonomy includes `E_SESSION_PROJECT_MISMATCH` (configuration).
- `hook run` result payload includes `verified` and `verification_state`; existing fields remain.

## [0.9.1] - 2026-11-17

Stability patch for the issues found by the CalcSuite, DiscountHunter and IronPulse v0.9.0
validations.

### Fixed

- Observation retention now orders by recorded `observed_at` (then mtime) instead of random token
  filenames, prunes before writing, and never evicts the observation just written. A store that
  cannot evict fails explicitly instead of returning an unusable token.
- Refs no longer include the observation token. Unchanged nodes keep the same ref across repeated
  observations; changed identity still produces a different ref.
- `ui tap`/`ui long-press --ref` accept context text inside a clickable ancestor and dispatch on that
  ancestor. Observations expose `actionable_via: "ancestor"` for those nodes.
- `--clickable` matches a clickable ancestor again instead of requiring the node itself to be
  clickable.
- `E_SELECTOR_NOT_FOUND` prioritizes the requested label and reports `visible_labels_total` and
  `visible_labels_truncated`.
- `report explain` classifies `E_ELEMENT_REF_*`, `E_ELEMENT_IDENTITY_CHANGED`,
  `E_ELEMENT_NOT_ACTIONABLE`, `E_ELEMENT_DEVICE_MISMATCH` as `element_reference` and
  `E_OBSERVATION_*` as `observation`, with targeted next actions. Observation-store failures are not
  auto-resolved by an unrelated passing run.
- `report explain` and `report bundle` use one project-scoped selector: `latest` is the newest
  completed run and `latest-failure` is the newest failed/partial run. Runs from other projects are
  never selected.
- Brief `agent test` keeps compact stage entries and `next_actions`.
- The bundle `manifest.json` records `checksums_verified:true` after successful verification.
- `diagnostics performance` returns `run_id`, `run_dir` and artifact paths; the brief summary includes
  the jank frame sample.
- Hook verification supports `ui_text`, `ui_activity` and `ui_package` postconditions. The IronPulse
  example hooks use them.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.9.1`.
- `schema/observation.schema.json`, `schema/diagnosis.schema.json`, `schema/hook.schema.json` and
  `schema/performance.schema.json` document the new fields.
- `report explain latest` on a passing run adds a `latest-failure` next action.
- Docs and examples use `latest-failure` when the intent is the newest failure.

## [0.9.0] - 2026-11-17

Agent Mode. Adds compact/brief output profiles, a start-of-turn brief, shorter handles and command
aliases without changing full JSON output.

### Added

- `--output-profile full|compact|brief`, `--compact`, `--brief`, `ANDROIDFORGE_PROFILE`,
  `[agent] profile`, `--max-items` and `--max-text-chars`. Full remains the default.
  Compact/brief output is minified, caps lists with explicit totals/truncation metadata, removes
  optional repetition, and adds envelope `profile`, `summary` and `next` fields.
- `agent brief` returns one small project/session/device/app/screen/last-run snapshot with the current
  observation token and short next commands.
- Compact observations, diffs and action receipts: node payloads are capped, optional duplicate node
  fields are removed, and receipts keep effect/completeness/identity while dropping redundant target
  and action payloads.
- Brief projections for `context`, `agent test`, `report explain`, `diagnostics performance`, APK
  install/diff results and errors. Comparable summaries preserve package, status, failure stage,
  metrics, split counts and evidence pointers.
- Short handles: element refs and observation tokens use 12 hex characters, with collision detection
  that extends refs deterministically. Existing longer observation tokens remain readable.
- Command aliases: `ui o/d/t/w`, `app i/l`, `diagnostics perf`, `report why`, `agent b`, `context ctx`.
  Selector aliases: `--id` (`--resource-id`), `--desc` (`--content-desc`) and `--token`
  (`--expect-token`).
- `schema/agent-brief.schema.json`; envelope `profile`/`summary`/`next`; adapter `[agent]` section.

### Changed

- `ui observe`, `ui diff`, action receipts, `agent test`, `context`, `report explain` and performance
  carry compact/brief projections when requested. Full output is unchanged.
- `schema list|show` includes `agent-brief.schema.json`.
- Workspace, CLI, desktop, installer and compatibility version are `0.9.0`.

## [0.8.0] - 2026-11-17

Build & Signal. Adds honest split-artifact identity, stable refs for the remaining semantic actions,
and explicit startup/jank/memory/CPU evidence.

### Added

- `app install` accepts explicit `--split` APKs, split directories containing `splits/` or
  `standalones/`, and `.apks` archives. Split sets use `adb install-multiple`; archives are extracted
  as provided without bundletool or automatic device-specific split selection. Package and version
  identity are checked before installation. Standalone/universal entries are not mixed into split sets,
  and ambiguous multi-standalone archives are rejected.
- `apk diff --split` and split-aware installed identity. Split sets are compared by sorted SHA-256;
  `app install --only-if-changed` skips only when the base APK and complete split set match. Unknown
  split identity never skips installation.
- `apk inspect` and `apk resolve` report artifact kind, split count, split hashes and `.apks` archive
  SHA-256. `apk resolve` warns that split paths are temporary extraction paths.
- `agent test` now proves installed split hashes before claiming the build is current; an unproven or
  changed split set requires explicit `--install` instead of being silently skipped.
- Stable refs for `ui text`, `ui clear`, `ui scroll`, `ui wait` and `ui assert`:
  `--ref <ref> --expect-token <observation>`. Text/clear require an editable field, scroll requires a
  scrollable node, and wait/assert may reference context text. Identity is re-checked before dispatch;
  stale/changed refs fail closed with no text or coordinate fallback.
- `diagnostics performance [package] [--activity ...] [--cold] [--settle ...] [--timeout ...]` with
  startup timing from `am start -W`, gfxinfo frame/jank statistics, meminfo PSS/RSS, and CPU from
  `dumpsys cpuinfo` or `/proc/<pid>/stat` sampling. Cold start requires the explicit `--cold` flag.
  Missing metrics carry unknown reasons; a `performance/report.json` artifact is written.
- Desktop Inspector performance panel with cold-start opt-in and startup/jank/memory/CPU summaries.
- `schema/performance.schema.json`; split fields in `apk-resolution.schema.json` and
  `apk-diff.schema.json`.

### Changed

- `app install` records the supplied artifact path (APK, directory or `.apks`) as the build source
  instead of a temporary extraction path.
- Workspace, CLI, desktop, installer and compatibility version are `0.8.0`.

## [0.7.1] - 2026-11-17

Patch to the v0.7.0 Matrix & Device Control line.

### Added

- Desktop Logcat now requests bounded read-only retries and surfaces the CLI retry report in its notes
  (attempt count and transient-failure indication), completing the v0.7.0 desktop retry presentation.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.7.1`.

## [0.7.0] - 2026-11-17

Seventh alpha, extending AndroidForge from reliable single-device orchestration to controlled
multi-device validation, stronger device interaction and safer transient recovery.

### Added

- `agent matrix` for sequential execution across multiple independent AVDs or serials, with a dry-run
  plan, per-target sessions and artifacts, duplicate/ownership checks, health preflight, fail-fast,
  cleanup reporting and aggregate `passed`/`failed`/`partial` status. Parallel mode is explicitly
  rejected until independently validated.
- Stable semantic element references: deterministic `ref` values on observed actionable/context nodes,
  `ui tap --ref ... --expect-token ...`, `ui long-press` reference support, identity re-validation
  immediately before dispatch and explicit stale/unknown/wrong-device/not-actionable errors. No
  text/coordinate fallback.
- Wireless ADB: `devices pair` (stdin-safe pairing code), `devices connect`, `devices disconnect` and
  endpoint validation. Pairing codes are never stored, logged or returned.
- `devices health`: read-only battery, thermal, storage, memory, network, screen, uptime, Android/API
  and ADB state, with explicit unknown reasons, raw evidence and configurable warning thresholds.
- Bounded read-only retries: `--retries 0..5` on `ui observe`, fresh `ui diff`, logcat `peek/tail`, with
  transient-error classification, exponential backoff, a 20-second total window and per-attempt
  reporting. State-changing operations never retry.
- Desktop Matrix page, per-device health and wireless controls, reference-copy actions in Context,
  action receipt details in Triage, and stdin-backed pairing-code handling.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.7.0`.
- `observation` nodes now include `ref`/`role`; old consumers that ignore unknown fields remain compatible.
- Matrix execution calls the existing `agent test` pipeline as a child process, preserving one
  backend/JSON contract.

### Safety

- Matrix never steals an occupied device and never releases a pre-existing session; it releases only
  sessions created by that matrix with the matrix owner. Emulators are not stopped.
- Element references fail closed on a changed path, identity, visibility or enabled state.
- Wireless pairing never enables networking or silently falls back to insecure `adb tcpip`.
- Retries are opt-in, bounded and restricted to a central allow-list of read-only operations.

## [0.6.0] - 2026-11-17

Sixth alpha, focused on returning more trustworthy evidence per agent command and remembering failures
across local runs.

### Added

- Action receipts for `ui tap`, `ui long-press`, `ui text` and `ui scroll` via `--receipt` and
  `--receipt-full`: before/after observation tokens, compact semantic diff, focus/package/activity
  transitions, deterministic effect classification, postcondition result and explicit completeness.
- Failure recurrence memory derived from existing run reports: `report recurrences`, `report history
  --fingerprint`, recurrence data in `report explain`, and `new`/`recurring`/`regressed`/`unknown`
  classification with conservative scenario comparability.
- Optional `[build]` variant configuration, conventional Gradle output fallback, `apk resolve`,
  `agent test --variant`, `apk diff installed:<package>` and `app install --only-if-changed`.
- `report bundle` for redacted, size-bounded, checksummed diagnostic ZIPs plus Markdown/JSON summaries,
  dry-run inclusion previews, evidence/logcat/hierarchy excerpts and explicit media/full-log opt-ins.
- Desktop Context and Ownership pages, recurrence history and related runs in Triage, and a diagnostic
  bundle inclusion preview/creation workflow.
- Schema files for action receipts, recurrence, APK resolution/diff and diagnostic bundles.
- Run reports now record the exact CLI invocation argv when created.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.6.0`.
- `report recurrences` is project-scoped by default; `--all-projects` aggregates every discovered local
  artifact store.
- Installed APK comparison reports `same`, `different` or `unknown` per field instead of inferring
  equality from version strings.

### Safety

- Receipts never retry a state-changing action when a postcondition is absent or times out.
- `--only-if-changed` never uninstalls, clears data or downgrades; unverifiable identity installs and
  records the reason.
- Bundles never modify the source run, never include keystores/credentials, reject traversal, and
  require opt-in for screenshots, recordings and full raw logs.

## [0.5.0] - 2026-11-16

Fifth alpha. Fixes every issue confirmed by the three independent app verification agents and adds
regression coverage for each corrected behavior.

### Fixed

- Failed `agent test` pipelines now emit envelope `status: "failed"` instead of `status: "success"` with
  a nonzero exit code. Blocked dry runs emit `status: "blocked"`.
- `session ensure` no longer chooses a console port already associated with a connected ADB emulator.
  Added `--port` to `session ensure` and the agent pipeline, with explicit conflict errors.
- `session start`, `session attach` and `session ensure` now bind the project root on creation, so the
  first `ensure` no longer emits a misleading legacy-project warning.
- A stuck UIAutomator helper is detected, killed and retried automatically. Persistent failure returns
  `E_UI_AUTOMATOR_UNAVAILABLE` with attempts, last error and recovery context.
- `report explain` classifies UIAutomator infrastructure failures as `ui_automation` rather than
  `unknown`, including legacy `E_COMMAND_FAILED` reports containing exit 137.
- `report explain latest` selects the newest failed/partial/cancelled/interrupted run when one exists,
  instead of explaining a newer passed run.
- `report explain <RUN>` and `report show/path` search the current directory and ancestor artifact
  stores, so runs created without `--project` remain discoverable from a project-scoped invocation.
- `report list --all` discovers runs across project and ancestor artifact stores.
- `ui observe` now exposes `token` alongside the backward-compatible `id`, separates interactive
  `actionable` nodes from non-interactive `context` text, always includes `clickable` and `enabled`,
  filters empty resource ids, and adds an `editable` flag.
- `ui diff` returns compact node summaries with a default 12-node cap per category and totals/truncation
  flags. `--full` raises the cap to 100.
- Selector errors now suggest `ui scroll-to` for below-the-fold targets and `--clickable` when candidate
  matches differ in interactivity.
- `doctor` counts active sessions and stale active sessions, reporting tombstones separately.
- `session release` ownership errors now name the actual subcommands and mention `ANDROIDFORGE_OWNER`.
- The desktop executable answers `--help` and `--version` non-interactively instead of launching the GUI.
- IronPulse's conformance flow dismisses the debug update dialog. DiscountHunter's Settings tap uses
  `--clickable` to avoid the header/bottom-nav ambiguity.

### Added

- Regression tests for failed/blocked agent envelope status, UIAutomator classification, project-scoped
  run discovery, observation actionable/context separation, compact diff caps, session owner parsing,
  and report scoping.
- `ui diff --full`.
- `session ensure --port`.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.5.0`.
- Observation compact default caps actionable nodes at 24 and context nodes at 16; context includes at
  most 12 and 8 respectively.
- `ui diff` defaults to compact nodes; use `--full` for larger lists.

## [0.4.0] - 2026-11-16

Fourth alpha, focused on reducing the number of commands, tokens and manual reasoning an AI coding
agent needs to operate an Android device.

### Added

- `androidforge context` - one compact, section-filterable snapshot of project, environment, session,
  device, application, screen observation, last run and deterministic next actions. Supports `--full`,
  `--refresh-screen`, `--no-screen` and `--include`; returns partial results when a device or optional
  section is unavailable.
- `androidforge report explain <RUN|latest>` - deterministic failure categories, confidence,
  evidence references, stable versioned fingerprints and exact `argv` next actions. Supports `--full`
  and `--markdown`; never calls an LLM or network service.
- `androidforge ui observe` - compact semantic observation with persisted observation tokens, freshness,
  foreground/focus/display metadata, actionable/scrollable nodes, visible text and `--full` hierarchy
  mode. Observation tokens are session-scoped, boot-id-scoped, retained per session and rejected after
  24 hours, cross-session use or a device reboot.
- `androidforge ui diff --from <TOKEN> [--to <TOKEN>]` - semantic added/removed/modified/focus/package/
  activity changes, duplicate resource-id pairing, meaningful bounds-change filtering and explicit
  truncation counts.
- UI postconditions for `tap`, `long-press`, `text` and `scroll`: `--wait-for-text`,
  `--wait-for-absent-text`, `--wait-for-value`, `--wait-for-package`, `--wait-for-activity` and
  `--timeout`. Actions execute once; only observations are polled. Failures return
  `E_POSTCONDITION_TIMEOUT` with the current foreground state, postcondition results, observation token
  and failure evidence.
- `androidforge session ensure` - idempotent session acquisition by project/owner/target, safe reuse,
  warm-emulator attachment, project binding, advisory TTL/lease metadata, expiry reconciliation without
  killing devices, serialized concurrent ensures and ownership-safe alternatives.
- `androidforge doctor fix --plan|--check` - strictly read-only repair planning for SDK, platform-tools,
  ADB, emulator, build-tools, Java, Maestro, licenses, SDK/platform gaps, invalid paths, conflicting SDK
  environment variables and project configuration. No package installation, PATH edit, registry edit or
  download is performed.
- `androidforge agent test` - transparent composition of config validation, session ensure, device
  readiness, APK inspection, installed-build comparison, optional install, optional fixture, optional
  conformance, failure explanation and a consolidated inline/on-disk report. `--dry-run` lists planned
  stages, confirmations and state-changing operations without executing them.
- Stable error codes `E_POSTCONDITION_TIMEOUT`, `E_OBSERVATION_NOT_FOUND`, `E_OBSERVATION_EXPIRED`,
  `E_OBSERVATION_SESSION_MISMATCH` and `E_LEASE_EXPIRED`.
- JSON Schemas for context, observation, UI diff, diagnosis, session ensure, repair plan and agent test.
- Desktop integration: deterministic explanation and copyable next actions in Triage, read-only repair
  plan in Setup, and lease/project binding in Sessions.

### Changed

- Workspace, CLI, desktop, installer and compatibility version are `0.4.0`.
- UI observation defaults to a compact actionable view with explicit `actionable_total` and
  `actionable_truncated`; `--full` returns the complete parsed hierarchy and all actionable nodes.
- Failure explanations prefer the operation-specific step failure over an aggregate command failure,
  classify signature rejection messages even when reported as `E_INSTALL_FAILED`, and surface failed
  conformance check names.
- `report explain` curates up to ten relevant evidence references by default; all artifacts remain in
  `artifacts` and expanded excerpts are available with `--full`.
- APK install failures now record the candidate `apk_path` so diagnosis and next actions reference the
  APK that actually failed rather than a stale session build.
- Session metadata now records `project_root`, `lease_expires_at` and `lease_ttl_seconds`; older
  sessions remain readable.

### Known limitations

- Observation tokens live in the local AndroidForge state directory. They survive normal CLI
  invocations but not manual deletion of that state or a different `ANDROIDFORGE_STATE_DIR`.
- Lease expiry is advisory metadata; it never stops an emulator. Expired leases must be renewed or
  reconciled explicitly.
- `doctor fix --plan` does not repair environments automatically; this is deliberate.
- `agent test` supports one APK, one fixture and one conformance flow per invocation.
- Physical devices, Maestro execution, instrumentation-package validation, snapshot restore, root-only
  operations, multiple DPI scales and macOS/Linux hosts remain unvalidated.

## [0.3.0] - 2026-11-16

Third alpha, released as one integrated desktop and backend update.

### Added

- First-run setup wizard covering CLI discovery and compatibility, actionable environment diagnostics,
  workspace/project selection, a safe device verification step and a final readiness summary. The
  wizard does not install APKs or reset user data.
- Actionable `doctor` fields: `required`, `blocking`, `impact` and `suggestion`, so missing or broken
  components explain why they matter and what to do next.
- GitHub release update checks and a verified CLI repair path. Downloads are explicit, checked against
  the published SHA-256, staged, validated and rolled back on failure. The previous executable is kept.
- Failure triage with failure metadata, build identity, artifacts, related live views and copyable
  Markdown/JSON reports.
- Compatible run comparison with project/scenario matching, build/environment change warnings, step
  regressions, new/resolved failures, duration deltas and artifact differences.
- Flaky-step analysis with a transparent minimum-run heuristic and per-step observation history.
- Run lifecycle outcomes `cancelled`, `interrupted`, `timed_out` and `abandoned`, creator PID/host
  metadata, `report reconcile`, and startup reconciliation of unfinished runs.
- Read-only live capture contracts: `capture frame`, `diagnostics logcat peek --since`, `ui inspect`,
  plus `report compare`, `report flaky`, `session prune` and `maintenance summary|clean`.
- Native live logcat panel with bounded buffers, incremental tailing, package/level/search filters,
  pause/resume, follow-latest, crash/ANR highlighting, disconnect state and export.
- Native periodic device preview and semantic UIAutomator inspector with hierarchy tree expansion,
  search, raw XML, bounds highlighting, selector copying, tap/long-press/type/scroll/back/home actions
  and a clearly labelled coordinate fallback. Preview is documented as periodic capture, not video.
- Maintenance center with storage accounting, dry-run previews, explicit confirmation, active-session
  protection and terminal-only pruning. Preview state is cleared after a successful cleanup.
- Portable ZIP and per-user Inno Setup installer, including both executables, license, documentation,
  Start Menu/optional desktop shortcuts and an uninstaller. User configuration and artifacts are kept
  outside the install directory and are not removed by upgrade or uninstall.
- Embedded Inter and JetBrains Mono fonts with OFL license files.
- Comprehensive desktop redesign: navigation groups, typography scale, semantic colors, reusable
  cards/tables/badges/empty states, sticky wizard footer, keyboard shortcuts, responsive layouts and
  automatic Windows DPI handling.
- JSON schemas for analysis and maintenance results, plus expanded run/product documentation.

### Changed

- Workspace version is `0.3.0`; the desktop refuses a CLI whose version or envelope schema does not match.
- CLI locator also checks the per-user install directory and `PATH`.
- Emulator detached-process handling now keeps an invisible console for hidden children, avoiding a
  stray visible Windows Terminal/QEMU console window on headless starts.
- Cancellation no longer opens an error dialog; it reports the cancelled outcome in the operation panel
  and reconciles the run as `cancelled` while leaving the session intact.
- Operation duration freezes at the final measured duration instead of continuing to climb.

### Fixed

- Settings icon renders as a painted gear rather than relying on an unsupported font glyph.
- Long paths in cards truncate with copy/open actions instead of spilling under adjacent cards.
- Logcat tag columns clip with ellipsis and no longer overlap message text.
- Logcat treats an empty package filter as all processes, and incremental tailing uses the previous
  device timestamp instead of suppressing repeated identical lines.
- Maintenance preview no longer remains visible after data has been removed.
- Inspector duplicate error banners are deduplicated and disconnected previews show an overlay while
  retaining the last captured frame.
- Sticky setup footer keeps Back/Continue/Finish reachable at 1280×720 and smaller layouts.

### Known limitations

- Binaries are unsigned. Windows SmartScreen may warn; no publisher identity is claimed.
- Device preview is periodic screenshots (roughly 1–5 FPS), not a low-latency video stream.
- Physical-device, Maestro, instrumentation-package, snapshot-restore and root-only paths remain
  unvalidated in this release.
- The update checker only offers a compatible CLI sidecar. It does not silently install a new desktop
  build.

## [0.2.0] - 2026-11-16

Second alpha. Adds the native AndroidForge Desktop cockpit while keeping the CLI as the authoritative
automation surface.

### Added

- `androidforge-desktop.exe`, a native Rust egui/eframe companion with the AndroidForge icon embedded in
  the window and executable. It uses the glow renderer and has no WebView, browser, HTML, JavaScript,
  Node or frontend build.
- P0 read-only cockpit: project discovery, environment overview, devices, sessions, report-backed runs,
  run details, build identity, steps, failure details, evidence tabs, raw report/metadata/manifest
  views and checksum-manifest verification.
- Evidence viewer for screenshots (fit/100%/zoom), hierarchy and log previews with line numbers and
  search, safe artifact joining and external-open shortcuts.
- P1 controlled actions: emulator session start, device attach, session release/stop/restart, APK
  selection and inspection, install dry-run and verified install, fixture/conformance/hook launch,
  streamed output and process-tree cancellation.
- Session-target selection, prepared-settings inspection and explicit destructive confirmations.
- Native Windows process-tree termination for cancellable desktop operations (installs, fixtures,
  conformance and hooks); session lifecycle operations run to completion because they must not leave a
  half-owned emulator. Cancelling a run never stops, restarts or releases the emulator, and the UI says
  so. A cancelled run leaves an incomplete report and a clear explanation instead of a parser error.
- CLI locator with explicit path, sidecar, workspace-build and `PATH` resolution, compatibility probing,
  schema refusal for unknown contracts, structured CLI error display and in-app Settings.
- Desktop integration tests with a compiled mock CLI covering version probing, typed results, structured
  errors, invalid JSON, timeouts, streamed output and cancellation.
- Documentation for the desktop cockpit, including architecture, user guide, capabilities, verification
  and portable distribution.

### Changed

- Workspace version is `0.2.0`; MSRV is `1.95`.
- CI builds and uploads both executables. Release assets now include a standalone CLI zip, a combined
  desktop+CLI portable zip and `SHA256SUMS.txt`.
- Busy-state checks use only a running operation, so completed output no longer disables new actions.
- Session actions are disabled for released/abandoned tombstones.
- Clippy `collapsible_if` edits across the workspace for the current stable toolchain.

### Fixed

- Desktop version probing invokes `androidforge --version` directly, matching the CLI contract.
- Run detail requests use `report show` and `report path` instead of ambiguous top-level verbs.
- Confirmations render above settings/install/run dialogs; installing closes the install dialog before
  confirming and reopens it with the measured result.
- Table row selection works reliably and explicit `Select`/`Open` buttons are available.
- Long commands no longer overlap operation buttons.
- Cancelled operations explain the missing final JSON report instead of showing a raw parser error.
- Artifact path checks normalize Windows verbatim prefixes and report XML uses the document root
  element.

## [0.1.0] - 2026-11-15

First public alpha of the local-first AndroidForge CLI.

### Added

- Rust 2024 workspace with nine runtime crates plus test support; one native `androidforge.exe`.
- Global `--json`, `--trace`, `--dry-run`, `--yes`, `--session`, `--serial`, `--project`, `--run`, `--quiet` and `--no-failure-capture`.
- Stable exit codes, structured errors with suggestions/context/artifacts, and command suggestions for misspellings.
- `doctor`, `capabilities`, `schema`, `init`, `config show|validate|path`.
- ADB discovery, robust `adb devices -l` parsing, device inspection, wait, prepare and restore.
- AVD listing, headless emulator start, boot wait, safe port checks, status and stop. Unowned emulators are never stopped.
- Session start/attach/claim/list/inspect/status/release/stop/use/current/gc/restart with heartbeat, operation locks, dead-operation recovery and explicit stale reclaim.
- Snapshot list/save/load/delete with confirmation for destructive loads and honest unsupported errors when the emulator refuses.
- APK inspect/verify via `aapt2` and `apksigner`.
- App install with package, versionName/versionCode, local and installed APK SHA-256, certificate SHA-256, debug-signed status and verification.
- Signature-mismatch diagnosis without automatic uninstall.
- App launch/status/force-stop/kill/uninstall/clear-data/list/info and `run-as` file push/pull for debuggable apps.
- UIAutomator hierarchy dump/find/focused, semantic selectors, ambiguity errors, verified text entry, clear, wait, assert, settle, scroll, scroll-to, swipe, Back/Home/keyevent and IME-aware Back.
- Screenshots with display metadata and SHA-256; screen recording with Android's 180-second limit and reliable stop; bounded one-shot and continuous logcat.
- Crash/ANR detection, process/task/activity/foreground separation, dumpsys collection and automatic failure bundles.
- Notifications, jobs, alarms, permissions, AppOps, Doze, standby, battery, network and time commands, with best-effort labels where parsing varies.
- Versioned fixtures with phases, retries, timeouts, failure policy, capture policy, destructive confirmation, dry-run, environment contract and app-private file round-trips.
- Named project hooks: broadcast, deep link, shell, instrumentation and command.
- Thin conformance manifests with script, Maestro and fixture flows, artifact globs and JSON expectations.
- Optional Maestro wrapper and instrumentation wrapper; no Gradle replacement.
- One-run model with `report.json`, `metadata.json`, checksum `manifest.json`, active runs, failure bundles, `report show|path|prune`, and `report import junit`.
- Bounded process runner with timeout, output caps, UTF-8 replacement and Windows process-tree cleanup.
- Example adapters and real conformance flows for CalcSuite, DiscountHunter and IronPulse.
- Case-study evidence from real Android 16 emulator runs.

### Fixed

- `permissions check/grant/revoke` now accepts short names such as `POST_NOTIFICATIONS` and normalizes them to `android.permission.*`; Android 16's removal of `pm check-permission` falls back to `dumpsys package` without noisy false failures.
- APK install accepts both the release and debug application ids declared by an adapter. An explicit `--expected-package` still narrows the check. `--expected-cert-sha256` adds an ad-hoc certificate check.
- APK install enforces `[project].expected_cert_sha256` before installing, so a build signed by an unexpected key fails with `E_SIGNATURE_VERIFICATION_FAILED` and the actual fingerprint.
- APK install `--dry-run` now inspects the APK and reports package/version/hash/certificate plus planned constraints. `clear-data --dry-run` no longer requires `--yes`.
- Fixture templates now resolve variables that themselves contain `${PROJECT_ROOT}` or `${ENV:NAME}` to a fixpoint.
- `capture record stop` no longer finalizes a caller-owned run; it only finalizes a run AndroidForge created implicitly for the recording.
- Run pre-install build identity uses the debug package when configured and no longer copies an expected certificate into an observed field before an install verifies it. Installs persist identity in the session, so later runs/conformance reports inherit the measured build.
- `run end` derives status from failed steps, refuses an implicit `passed` result, and exits non-zero for failed/partial runs.
- `ui clear` verifies an empty field when Android reports the empty text as absent, instead of returning `E_TEXT_VERIFICATION_FAILED`.
- `session start` no longer lets the detached emulator inherit the caller's stdout/stderr pipe; captured invocation returns after Ready.
- `session stop` can stop a released tombstone's owned emulator if no new session took ownership; `session list --active` hides tombstones.
- `report list` was added; `report import junit` accepts multiple files or a Gradle result directory.
- Hook `verify` postconditions and `run_as` broadcast delivery were added. ADB exit 0 is no longer treated as proof that the app changed state.
- `notifications channels` parses the full `AppSettings` channel catalog, not only active notification records.
- `jobs list` parses Android 16 named/WorkManager namespace headers and package `Source` lines; `alarms next` parses `origWhen`/`Alarm{...}` forms.
- `doctor` now reports actual free space and says “no project adapter found” rather than validating defaults as if an adapter existed.
- `avd clone` now has an explicit command that returns `E_UNSUPPORTED_OPERATION` and supported alternatives instead of falling through to an unknown-command error.
- `--json` appended to a raw `adb` command is moved before `adb`; interior occurrences get a clear ordering error. `capture screenshot` exposes `result.path`.
- Example conformance scripts treat `ui find` as a search (empty `matches` is not "found"), handle real first-run onboarding, and CalcSuite now checks the 3.0.3 updater.
- `run end --run <ID>` clears the session's active-run pointer when it names that run. Commands no longer append to a finished run after an explicit end; stale finished-run pointers are cleared automatically.
- `session stop`, `session release` and `session restart` require the owner label recorded at start (or `ANDROIDFORGE_OWNER`); `--force` is the explicit cross-owner takeover. This closes the path where one agent could stop another agent's emulator by session id.
- `ui clear` reads UIAutomator `hint`/`hint-text` and treats an exact hint as empty before typing and after clearing; Android Settings search now clears without a false failure. Clear with no selector reports `E_SELECTOR_NOT_FOUND` instead of a text-verification error when nothing editable is focused.
- `report show`/`report path` fall back to project-local `.artifacts/runs`, so conformance evidence is inspectable from the repository root without repeating `--project`.
- Conformance command results now include the measured build identity and `flow_checks` parsed from the external script's final JSON `checks` array; the conformance step details carry the same checks.
- Local install preflight failures (expected package/certificate) skip the full failure bundle because the device was not touched.
- `app launch` retries `am start` once after a transient package/launcher race and reports the retry as a warning.
- Human-mode errors summarize embedded run reports, JUnit summaries and external results instead of printing the full JSON context.
- Active-run pointers without a session include the project root, so two projects cannot overwrite each other's active run.

### Known limitations

- Physical devices, Maestro execution, a real instrumentation package, snapshot restore and time changes requiring root were not exercised in this delivery.
- AVD cloning and named-profile isolation are not implemented and return `E_UNSUPPORTED_OPERATION`.
- Compose test tags are visible only when exposed through accessibility semantics; use instrumentation for internal tags.
- Notification/job/alarm/Doze parsing is version-dependent and remains best-effort with raw evidence preserved.
- No GUI, daemon, cloud, telemetry, HTML reports, visual diff, release publishing or signing automation.

[Unreleased]: https://github.com/JuggyBob/AndroidForge/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v1.0.0
[0.13.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.13.0
[0.12.1]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.12.1
[0.12.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.12.0
[0.11.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.11.0
[0.10.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.10.0
[0.9.4]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.9.4
[0.9.3]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.9.3
[0.9.2]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.9.2
[0.9.1]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.9.1
[0.9.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.9.0
[0.8.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.8.0
[0.7.1]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.7.1
[0.7.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.7.0
[0.6.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.6.0
[0.5.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.5.0
[0.4.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.4.0
[0.3.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.3.0
[0.2.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.2.0
[0.1.0]: https://github.com/JuggyBob/AndroidForge/releases/tag/v0.1.0
