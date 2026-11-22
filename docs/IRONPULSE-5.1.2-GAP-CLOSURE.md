# IronPulse 5.1.2 Gap Closure Checklist

> **Released in AndroidForge v0.12.0 (2026-11-22).** All items below are implemented and recorded with
> delegated verification evidence. Release notes: `docs/release-notes/v0.12.0.md`.

Source issue report: `A:\Projects\Archive\Docs\AndroidForge\Issues\IronPulse-5.1.2-AndroidForge-Issues.md`

Status values: **NOT STARTED**, **IN PROGRESS**, **TO VERIFY**, **IMPLEMENTED**.

Rules for this work:

- Fix data-truth bugs before adding new surface area.
- Every item gets a failing reproduction or a focused test where practical.
- Human output, JSON output, schemas/MCP mappings and docs are updated together.
- Testing runs are delegated to `opencode-go/hy4-flash-free#high` only.
- Do not mark an item IMPLEMENTED until its evidence is recorded here.

## Phase 0 - Baseline and hotfix carry-over

| ID | Work item | Status | Evidence / notes |
| --- | --- | --- | --- |
| 0.1 | Record source backup before the gap-closure work | IMPLEMENTED | `A:\Backups\AndroidForge-source-pre-ironpulse-gaps-20261122-112523` |
| 0.2 | Carry the desktop CLI-launcher PATH hotfix into the working tree | IMPLEMENTED | `desktop/src/app.rs` prepends the CLI directory to the spawned console PATH; local hotfix installed separately |
| 0.3 | Keep the v0.11.0 release artifacts and tests baseline intact | IMPLEMENTED | 335 tests / 27 executables on v0.11.0; release assets unchanged |

## Phase 1 - Truth bugs first

| ID | Work item | Acceptance | Status | Evidence / notes |
| --- | --- | --- | --- | --- |
| 1.1 | `session status` re-reads live installed APK metadata | After an out-of-band `adb install -r`, `session status --json` reports the live `pm path`, version and base APK SHA-256, not the cached install record | IMPLEMENTED | Delegated emulator proof: after `adb -s emulator-5554 install -r <baseline>`, `session status` returned `live_build_refreshed:true` and `build.installed_apk_sha256=2785092a…68bee` matching the live APK |
| 1.2 | `app install --only-if-changed` is byte-truthful | A rebuilt APK with the same versionCode but different base bytes is installed; only exact package/version/hash/split identity skips | IMPLEMENTED | Delegated emulator proof: same package/version signed rebuild with added asset (`7b96823a…718e`) installed with `skipped:false`, `reason:content_different`; repeating the baseline skipped with byte-identity reason. Unit tests cover different/unknown hashes |
| 1.3 | Compose testTag bridge / `ui tap --test-tag` | `--test-tag` is accepted by selectors, maps to the exposed accessibility resource id, and fails with an actionable diagnostic when the app has not enabled `testTagsAsResourceId` | IMPLEMENTED | Delegated emulator proof: `ui tap --test-tag definitely_missing_tag` returned exit 7 `E_SELECTOR_NOT_FOUND` with the suggestion `Compose testTag values are visible only when the debug app sets testTagsAsResourceId = true in its semantics.`; `ui find` returned zero matches without error |

## Phase 2 - Truthful install and version verification

| ID | Work item | Acceptance | Status | Evidence / notes |
| --- | --- | --- | --- | --- |
| 2.1 | `app verify-installed --apk <path>` | Compares the live installed base APK bytes/hash to the local APK and reports package, version, expected/actual SHA-256, certificate and a clear exit code | IMPLEMENTED | Delegated emulator proof: `overall:same`, `safe_to_skip_install:true`, all 9 compared fields `same`; mismatch path returns `E_INSTALL_VERIFICATION_FAILED` |
| 2.2 | `app status --expect-version-name/--expect-version-code/--expect-apk-sha256` | Returns a structured pass/fail for each expectation against live device state; non-zero exit on mismatch | IMPLEMENTED | Delegated emulator proof: `--expect-apk-sha256 2785092a…68bee` returned `passed:true` with actual live hash; JSON includes `install_expectations` and `live_install` |
| 2.3 | Live build truth exposed in session/runs | `session status`, `session inspect` and subsequent run reports use refreshed live build identity | IMPLEMENTED | Delegated emulator proof: out-of-band install was reflected by `session status` and persisted (`live_build_refreshed:true`); status/inspect share the refreshing path |

## Phase 3 - Gestures, animation waiting and assertions

| ID | Work item | Acceptance | Status | Evidence / notes |
| --- | --- | --- | --- | --- |
| 3.1 | First-class long-press duration | `ui long-press --duration <ms>` uses a configurable hold instead of a fixed 900ms; default remains safe | IMPLEMENTED | Emulator `ui long-press --duration 1200` executed against Settings (matched EditText, tap 603,222); delegated smoke passed |
| 3.2 | Drag-and-drop with hold duration | `ui drag --from <selector> --to <selector> --hold-before/--hold-after/--duration` performs a real drag with optional holds; coordinates/selectors supported | IMPLEMENTED | Emulator `ui drag --from-x 300 --from-y 1200 --to-x 800 --to-y 1200 --duration 800` returned `steps:12`; delegated smoke passed |
| 3.3 | Mid-gesture screenshots | `--capture-mid <path>` (or equivalent) captures a screenshot during a long-press/drag without ending the gesture | IMPLEMENTED | Emulator drag returned `mid_capture` with SHA-256 `29c5f8…9fc3e6`; file existed; delegated smoke passed |
| 3.4 | `ui wait --for-text-gone` | Waits until matching text is absent; can be combined with existing waits | IMPLEMENTED | Emulator: absent text passed; present "Search Settings" produced exit 7 `E_UI_ASSERTION_FAILED` with lingering TextView bounds, proving the wait is real |
| 3.5 | `ui wait --for-idle` | Waits for UI hierarchy/touch idle using bounded hierarchy stability polling | IMPLEMENTED | Emulator `ui wait --for-idle --timeout 10` returned `stable:true`, `attempts:2`; delegated smoke passed |
| 3.6 | `ui wait --for-stable` | Waits for a stable hierarchy hash over a configurable interval | IMPLEMENTED | Emulator `ui wait --for-stable --stable-for 2s --timeout 15` returned `stable:true`, `attempts:2`, XML SHA-256 |
| 3.7 | `ui assert-no-text` | Fails if matching visible text exists; passes with evidence when absent | IMPLEMENTED | Emulator `ui assert-no-text --contains "Definitely absent text"` returned `absent:true`, `attempts:1`; delegated smoke passed |

## Phase 4 - App data and fixture control

| ID | Work item | Acceptance | Status | Evidence / notes |
| --- | --- | --- | --- | --- |
| 4.1 | `app data export` | Pulls app-private data using `run-as` for debuggable apps into a portable archive/directory; refuses non-debuggable apps with a clear error | IMPLEMENTED | Delegated emulator proof: exported `com.remerer.mobiledeck` to 5,632-byte tar with SHA-256 `537ef7c7…68fd3`; `run-as` path used |
| 4.2 | `app data import` | Restores exported app-private data with `--yes`, validating package/debuggable state and preserving a pre-import backup | IMPLEMENTED | Delegated emulator proof: import created `data/pre-import-backup.tar` (6,656 B) with `app_data_pre_import` artifact before a successful restore |
| 4.3 | Per-test data reset/restore | `app data reset --seed <fixture>` or equivalent resets to a recorded baseline, supports dry-run and confirmation | IMPLEMENTED | Delegated emulator proof: reset cleared data, wrote `before-reset.tar` (6,656 B) and restored the baseline tar; `--dry-run` supported |
| 4.4 | Database/DataStore seeding | `app data seed --file <path> --remote <path>` for debuggable apps, with SQLite/DataStore-aware guidance and checksum verification | IMPLEMENTED | Delegated emulator proof: seed returned `remote_hash_verified:true` and remote SHA-256 equal to local; size also verified |

## Phase 5 - Network and provider mocking

| ID | Work item | Acceptance | Status | Evidence / notes |
| --- | --- | --- | --- | --- |
| 5.1 | `adb reverse` management | `network reverse add/list/remove/clear` manages per-session reverse mappings and records cleanup | IMPLEMENTED | Emulator add/list/remove tcp:18080 passed; subsequent list empty; `AdbClient::reverse_*`; mappings stored as `network:reverse:<remote>` |
| 5.2 | Local mock HTTP server | `network mock serve` starts a local bounded HTTP server with configurable port/routes and session mapping | IMPLEMENTED | Delegated smoke: 200/401 routes, 404 fallback, `requests:2`, `stopped_by:max_requests`, log artifact; loopback default |
| 5.3 | TLS options | Mock server supports optional local TLS certificate/key and a documented trust path | IMPLEMENTED | Delegated smoke: self-signed cert + `--tls-cert/--tls-key`, `curl -k` returned 200, result `tls:true`; docs state user-CA/network-security trust, no public CA claim |
| 5.4 | Request logs | Mock server records method/path/status/headers/body metadata and exposes request log artifact/JSON | IMPLEMENTED | NDJSON log showed method/path/status/matched route/duration; `authorization: [redacted]` confirmed |

## Phase 6 - Visual regression

| ID | Work item | Acceptance | Status | Evidence / notes |
| --- | --- | --- | --- | --- |
| 6.1 | `capture screenshot --compare baseline` | Compares current screenshot to a baseline with full/region diff metrics and pass/fail threshold | IMPLEMENTED | Emulator: two identical 1080x2400 captures compared with `--compare`, `changed_pixels:0`, `compared_pixels:2592000`, `passed:true`; 4 visual unit tests pass |
| 6.2 | Region diffs | `--region x1,y1,x2,y2` limits comparison; result includes changed-pixel count/ratio and bounds | IMPLEMENTED | Region/threshold unit tests pass (`parse_region`, changed bounds, tolerance); region validated against image bounds |
| 6.3 | Before/after tap screenshots | `ui tap --receipt` (or explicit flag) captures before/after screenshots and links them in the receipt | IMPLEMENTED | Emulator tap on Settings returned `before_screenshot`/`after_screenshot` with SHA-256, both files present; delegated smoke passed |

## Phase 7 - Session hygiene and guidance

| ID | Work item | Acceptance | Status | Evidence / notes |
| --- | --- | --- | --- | --- |
| 7.1 | `session list --mine --active` | Filters to the current owner and active states; `--mine` defaults to configured owner | IMPLEMENTED | Delegated fabricated-session test: `--mine`/`ANDROIDFORGE_OWNER` returned only alice sessions; `--owner bob` returned bob only; `--active` excluded released/failed |
| 7.2 | Clear stale/tombstone presentation | List/status distinguishes active, stale, released, failed and abandoned; JSON includes reasons and ages | IMPLEMENTED | Fresh→active, stale→stale with 10808s reason, released→released, failed→failed with last_error; after GC stale→abandoned |
| 7.3 | `session gc --reclaim --dry-run` guidance | Dry-run is explicit, lists would-reclaim sessions/devices and gives exact next command | IMPLEMENTED | Dry run reported `would_reclaim:["ses-stale-bob"]` and `next_commands:["androidforge session gc --reclaim"]`; all four session hashes unchanged. Applying reclaim changed only the stale session |

## Phase 8 - Documentation, schema, MCP and tests

| ID | Work item | Acceptance | Status | Evidence / notes |
| --- | --- | --- | --- | --- |
| 8.1 | Update `USER_GUIDE.md`, `AGENT_GUIDE.md`, `CAPABILITIES.md`, `SCHEMA.md` | Every new command/flag and safety class is documented | IMPLEMENTED | Added sections for truth-first installs, gestures, waits, data fixtures, reverse/mocks, visual diffs, session hygiene and Compose tags; removed the stale "visual diff unsupported" claim and updated command/capability lists |
| 8.2 | Update JSON schemas | New result shapes validate; old reports remain readable | IMPLEMENTED | Delegated check: 30/30 `schema/*.json` parse; `schema list` includes all six new schemas; existing safety/schema tests pass |
| 8.3 | Update MCP/tool schema mappings | New commands/flags are exposed or explicitly excluded with reason | IMPLEMENTED | Delegated check: MCP catalog is 40 tools and includes verify-installed, app status/data, drag, wait, assert-no-text, reverse, bounded mock serve, session list, screenshot comparison |
| 8.4 | Regression tests | Focused tests cover live install truth, only-if-changed bytes, new selectors, gestures, waits, data, network, visual diffs and sessions | IMPLEMENTED | Unit tests cover hash mismatch/unknown, drag selectors, reverse specs, visual diff + schema, mock routes/redaction; delegated emulator campaign covered out-of-band refresh, same-version byte-different install, baseline skip, verify/status expectations, gestures, waits, data export/seed/import/reset, reverse, visual compare and session hygiene |
| 8.5 | Final delegated verification | fmt, clippy, full tests, targeted smoke and asset checks pass on the final binary | IMPLEMENTED | Final gate: `cargo fmt --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo test --workspace` 347 passed/0 failed, visual schema test 5/5, debug binary newer than all 166 source files, 30 schemas. Mock HTTP/TLS/404/help smoke and emulator campaign passed. No release assets changed (no release requested) |
