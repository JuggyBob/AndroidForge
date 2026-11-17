# AndroidForge — Agent Guide

This is the front door for an autonomous coding agent. Read this before issuing commands.

## 0. The v0.9.0 agent loop

The shortest reliable loop is:

```powershell
androidforge doctor --json
androidforge agent brief --project . --json
androidforge session ensure --project . --avd Pixel_8_API_35 --owner MyAgent --ttl 1h --json
androidforge ui observe --session ses-... --output-profile compact --json
androidforge ui tap --session ses-... --text "Settings" --wait-for-text "Preferences" --receipt --output-profile compact --json
androidforge report explain latest --json
androidforge report recurrences --project . --json
androidforge report bundle latest --output .\\evidence.zip --json
androidforge ui observe --session ses-... --json
androidforge ui tap --session ses-... --ref el-... --expect-token obs-... --json
androidforge ui text "hello" --session ses-... --ref el-... --expect-token obs-... --json
androidforge ui wait --session ses-... --ref el-... --expect-token obs-... --timeout 5s --json
androidforge app install .\bundle.apks --only-if-changed --session ses-... --json
androidforge diagnostics performance com.example.app --cold --session ses-... --output-profile brief --json
androidforge agent matrix --avd Pixel_8_API_35 --avd Pixel_8_API_36 --flow conformance --json
androidforge context --session ses-... --json
androidforge session release --session ses-... --owner MyAgent --json
```

The commands answer, in order: is the host ready, what is the project/device state, which session do I
own, what is on screen, did my action produce the expected result and what changed, why did the last
run fail, whether that failure is new/recurring/regressed, and where the portable evidence bundle is.

### 0.1 Command safety classes

Output-profile flags (`--output-profile`, `--compact`, `--brief`, `--max-items`, `--max-text-chars`)
only change the JSON presentation; they never change which device actions are allowed or performed.

Read-only commands (safe to run for discovery; `context` may still capture and persist a screen
observation unless `--dry-run` is used):

- `doctor`, `capabilities`, `devices list|inspect|health`, `session list|inspect|status`, `context`,
  `ui observe`, `ui diff`, `ui inspect`, `ui dump`, `report list|show|path|explain|history|recurrences`,
  `report bundle --dry-run`, `doctor fix --plan|--check`, `apk inspect|verify|resolve|diff`,
  `diagnostics logcat peek`, `agent matrix --dry-run`. `apk diff` pulls the installed base APK and
  splits but does not change the device.
- `session ensure --dry-run` and `agent test --dry-run` make no device or session changes.

Device-modifying commands:

- `session start|ensure|attach|restart`, `app install` (plain install preserves app data), `ui
  tap|long-press|text|clear|scroll|swipe|back|home|keyevent` (including `--ref` actions), `fixture run`,
  `conformance run`, `agent test` without `--dry-run`, `agent matrix` without `--dry-run`,
  `diagnostics performance` (it launches the app; `--cold` also force-stops it), and
  `devices pair|connect|disconnect`.

Potentially destructive commands (require `--yes` or an explicit confirmation flag):

- `app install --fresh --yes` deletes application data.
- `app clear-data`, `app uninstall`, `app data-restore`, `snapshot load`, `session prune`, `session
  gc --reclaim --stop`, and `maintenance clean`.
- `agent test` never passes `--fresh`, never clears data and never uninstalls. It installs only when
  `--install` is supplied and only with a normal, data-preserving install.

### 0.2 `context`

```powershell
androidforge context --project . --json
androidforge context --session ses-... --json
androidforge context --session ses-... --include project,session,device,app --json
androidforge context --session ses-... --refresh-screen --json
androidforge context --session ses-... --full --json
```

`context` returns `project`, `environment`, `session`, `device`, `app`, `screen`, `last_run` and
`next_actions`. Missing sections are explicit objects with `available: false` and a reason, never
silently omitted. The screen section carries `observed_at`, `age_ms` and `stale`; a screen observation
younger than five seconds is reused unless `--refresh-screen` is passed. Use `--include` to fetch only
the sections you need. `--full` returns complete available data, including the parsed hierarchy.

Every `next_actions` entry is a suggestion:

```json
{
  "reason": "The screen has not been observed recently.",
  "priority": "high",
  "command": { "argv": ["androidforge", "ui", "observe", "--session", "ses-...", "--json"] }
}
```

Never execute an action blindly. The argv is exact, but it is not run automatically.

### 0.2.1 Agent brief and output profiles

```powershell
androidforge agent brief --project . --json
androidforge agent brief --session ses-... --json
androidforge ui observe --session ses-... --output-profile compact --json
androidforge diagnostics performance com.example.app --cold --session ses-... --brief --json
```

`agent brief` is one small project/session/device/app/screen snapshot for the start of a turn. It
returns the current observation token, compact actionable/context nodes and short next commands.

Full JSON remains the default and unchanged. Choose a profile with one of:

- `--output-profile compact` or `--compact`: minified JSON, capped lists with explicit totals, optional
  repetition removed, one-line `summary`, short `next` commands.
- `--output-profile brief` or `--brief`: the smallest decision-oriented view. It is especially effective
  for `context`, `agent test`, `report explain`, `diagnostics performance`, observations, diffs and
  action receipts.
- `--output-profile full`: explicitly override a compact default from `[agent]` or the environment.

Defaults can come from `[agent] profile` in `.androidforge/project.toml` or
`ANDROIDFORGE_PROFILE=full|compact|brief`. `--max-items` and `--max-text-chars` bound compact output.
Full detail remains available by re-running without the profile or with `--output-profile full`.

Common aliases: `ui o/d/t/w` (observe/diff/tap/wait), `app i/l` (install/launch), `diagnostics perf`,
`report why` (explain), `agent b` (brief), `context ctx`. Selector aliases include `--id`
(`--resource-id`), `--desc` (`--content-desc`) and `--token` (`--expect-token`).
### 0.3 Session acquisition and leases

```powershell
androidforge session ensure --project . --avd Pixel_8_API_35 --owner MyAgent --ttl 1h --json
androidforge session ensure --project . --avd Pixel_8_API_35 --port 5562 --owner MyAgent --json
androidforge session ensure --project . --serial emulator-5554 --owner MyAgent --json
```

`session ensure` is idempotent for the same owner/project/target. It reuses a healthy compatible
session, attaches to a warm unowned emulator instead of starting a second one, or launches the
requested AVD. It never steals a device owned by another session. The response includes
`session_id`, `owner`, `device_serial`, `avd`, `acquisition_mode`, `readiness`, `lease_expires_at`,
`lease_expired`, `needs_reconciliation`, `recovery` and `warnings`. Automatic port allocation skips
ports already used by connected ADB emulators; `--port` requests an explicit even console port.

TTL is advisory metadata. Expiry never stops an emulator or interrupts an active operation. Without
`--ttl`, an expired lease is reported as `needs_reconciliation: true`; with `--ttl`, ensure renews it
and reports `expired_lease_renewed`. Release a session explicitly when finished:

```powershell
androidforge session release --session ses-... --owner MyAgent --json
```

### 0.4 Semantic observation and diffs

```powershell
androidforge ui observe --session ses-... --json
androidforge ui observe --session ses-... --package com.example.app --json
androidforge ui observe --session ses-... --since obs-... --json
androidforge ui observe --session ses-... --full --json
androidforge ui diff --session ses-... --from obs-... --json
androidforge ui diff --session ses-... --from obs-... --to obs-... --json
androidforge ui diff --session ses-... --from obs-... --full --json
```

A compact observation returns the foreground package/activity, display, focus, interactive
`actionable` nodes, non-interactive `context` text, scrollable nodes, visible text, freshness and a
preferred `token` (with `id` retained as an alias). Every node includes `clickable` and `enabled`;
`editable` is present when true. Default output is intentionally filtered and capped;
`actionable_total`/`actionable_truncated` and `context_total`/`context_truncated` tell you whether more
nodes exist. `--full` returns the complete parsed hierarchy.

Tokens are scoped to one session and one device boot. They expire after 24 hours, after the per-session
retention limit, or when the device reboots. Cross-session use returns
`E_OBSERVATION_SESSION_MISMATCH`; a reboot or expiry returns `E_OBSERVATION_EXPIRED`. Create a fresh
observation instead of guessing.

`ui diff` reports compact semantic added, removed and modified nodes, focus/package/activity changes
and meaningful bounds changes. Pure pixel movement is ignored. Lists are capped at 12 by default with
explicit totals/`truncated`; pass `--full` to raise the cap to 100.

### 0.4.1 Action receipts

```powershell
androidforge ui tap --session ses-... --text "Save" --receipt --json
androidforge ui text "Example" --session ses-... --wait-for-value "Example" --receipt --json
androidforge ui long-press --session ses-... --text "Item" --receipt-full --json
androidforge ui scroll --session ses-... --direction down --receipt --json
```

A receipt replaces the tap/observe/diff/infer loop. `result.receipt` contains:

- `receipt_id`, action, normalized selector, session and device; `dispatch_state` is
  `not_dispatched`, `dispatched` or `unknown`;
- `before` and `after` observation tokens, foreground package/activity and focus;
- a compact semantic diff (`changes`) with exact totals;
- `effect`: `no_visible_change`, `content_changed`, `navigated`, `dialog_appeared`,
  `dialog_disappeared`, `element_disappeared`, `package_changed` or `unknown`;
- `effect_evidence`, the postcondition definition/result, warnings and `completeness`.

The action is dispatched at most once. Postcondition polling only refreshes observations. If the
before observation fails, the action is **not** dispatched and the error carries a partial receipt. If
the device disconnects after dispatch, the receipt says `dispatched: true` and the outcome is
`unknown`/incomplete — never "did not happen". Without `--receipt`, the action response keeps its old
shape.

### 0.5 UI actions with postconditions

```powershell
androidforge ui tap --session ses-... --text "Save" --wait-for-text "Saved" --timeout 10 --json
androidforge ui tap --session ses-... --text "Settings" --wait-for-package com.example.app --json
androidforge ui tap --session ses-... --text "Item" --wait-for-absent-text "Item" --json
androidforge ui type "Example query" --session ses-... --wait-for-value "Example query" --json
androidforge ui scroll --session ses-... --direction down --wait-for-text "More" --json
```

Multiple conditions are ANDed. The action is executed once; only observations are polled. Success
means both the action executed and the postcondition was satisfied. A timeout returns
`E_POSTCONDITION_TIMEOUT` (exit 7) with `action_executed: true`, the unsatisfied conditions, current
foreground package/activity, timeout, observation token and failure artifacts. Do not repeat a
potentially destructive action just because the postcondition timed out; inspect the observation and
evidence first.

### 0.6 Diagnosing failures

```powershell
androidforge report explain latest --json
androidforge report explain <RUN_ID> --json
androidforge report explain <RUN_ID> --full --json
androidforge report explain <RUN_ID> --markdown
```

The explanation contains `diagnosis.category`, `diagnosis.code`, `diagnosis.confidence`, a summary,
curated evidence references, alternatives, `next_actions` with exact argv, a stable `sha256:` fingerprint
and `project_root`. Categories include environment, device, emulator, install, signature, application,
process, task, ANR, selector, selector_ambiguous, ui_automation, timeout, assertion, permission,
configuration, cancelled, interrupted and unknown. Confidence is `high`, `medium`, `low` or
`insufficient_evidence`; there is no fake percentage.

`report explain latest` selects the newest failed/partial/cancelled/interrupted run when one exists,
falling back to the newest run. Run lookup searches the current project, the current directory and
ancestor artifact stores, so runs created without `--project` remain discoverable. Use
`androidforge report list --all` to list runs across those stores.

Run next actions from `project_root`, or pass that path with `--project`. The same failure produces the
same fingerprint across timestamps, run IDs, temporary paths and dynamic numeric IDs. Different
failures are not collapsed by a generic timeout or message.

### 0.6.1 Failure recurrence memory

```powershell
androidforge report recurrences --project . --json
androidforge report recurrences --project . --since 30d --package com.example.app --status regressed --json
androidforge report history --fingerprint sha256:<VALUE> --json
```

Recurrence is derived from existing local `report.json` files; there is no second database. Statuses
are:

- `new` — no earlier matching fingerprint in the project scope;
- `recurring` — earlier occurrences exist and no comparable success intervened;
- `regressed` — a comparable passing run exercised the same failing step between two occurrences;
- `unknown` — evidence is too incomplete for a confident classification.

A passing run only proves resolution when it belongs to the same project, command/scenario and package
and contains the same failing step with a successful status. An unrelated pass never marks a failure
fixed. Entries include first/last seen, occurrence count, run ids, devices, app versions, evidence
references and `currently_resolved`. `report explain` adds the same recurrence object.

### 0.6.2 Diagnostic bundles

```powershell
androidforge report bundle latest --output .\evidence.zip --json
androidforge report bundle latest --dry-run --json
androidforge report bundle latest --markdown --json
androidforge report bundle latest --format json --json
```

A ZIP bundle contains a run/explanation summary, Markdown explanation, exact recorded argv (or a
legacy-run fallback), selected evidence excerpts, bounded logcat, hierarchy XML, a checksummed payload
manifest and an explicit exclusion list. Redaction is on by default and applies to text payloads. Media
and full raw logs are opt-in. The tool refuses paths that escape the run directory, bounds per-file and
total size, writes atomically and reopens the ZIP to verify every checksum before reporting success.
`--dry-run` is the inclusion preview and writes nothing.

### 0.7 Repair planning

```powershell
androidforge doctor fix --plan --json
androidforge doctor fix --check --json
```

This is strictly read-only. It never installs SDK packages, edits PATH or the registry, downloads
executables or changes Java/emulator configuration. Each issue reports its severity, detected state,
component, reason, repair command (when one can be stated exactly), confirmation, network and
permission requirements. `mutations_performed` is always empty.

### 0.8 Transparent `agent test`

```powershell
androidforge --project . agent test --apk .\app-debug.apk --flow conformance --install --json
androidforge --project . agent test --apk .\app-debug.apk --flow conformance --dry-run --json
```

Without `--install`, the pipeline compares the candidate APK with the recorded installed build. If it
cannot prove the installed build is current, it stops at `install_apk` and tells you to pass
`--install`; it does not silently modify the device. A plain install preserves app data. `--dry-run`
lists every planned stage, required confirmations, target session/target acquisition and state-changing
operations without executing them. The result records per-stage status, duration, equivalent command
argv, failure stage/code, explanation and artifact paths, plus a consolidated report path.

A failed pipeline returns envelope `status: "failed"` with `exit_code` set to the failure code and the
full structured `result` preserved. A blocked dry run returns `status: "blocked"` and exit code 8.
Do not treat a nonzero exit code as an empty error: inspect `result.failure` and the report.

### 0.8.1 APK resolution and comparison

```powershell
androidforge apk resolve --variant debug --json
androidforge apk resolve --variant freeDebug --module app --json
androidforge agent test --project . --variant debug --flow conformance --json
androidforge apk diff installed:com.example.app .\app-debug.apk --session ses-... --json
androidforge apk diff installed:com.example.app .\app-debug.apk --split .\config.en.apk --session ses-... --json
androidforge app install .\app-debug.apk --only-if-changed --json
androidforge app install .\bundle.apks --only-if-changed --session ses-... --json
androidforge app install .\base.apk --split .\config.arm64_v8a.apk --session ses-... --json
```

`apk resolve` reads optional `[build.variants]` patterns and uses conventional Gradle output locations when no
explicit patterns are configured. It never runs Gradle. The newest valid artifact wins; malformed candidates are rejected,
equal timestamps with different hashes are an explicit ambiguity error, and the selected rule, size,
mtime, SHA-256 and certificate are returned. An explicit `--apk` always takes precedence over
`--variant`.

`apk diff` pulls the installed base APK and every installed split and reports `same`, `different` or
`unknown` per field. Split sets are compared by sorted SHA-256 hashes. `--only-if-changed` skips
installation only when package, version code, base APK SHA-256 and the complete split set prove
equivalence; unknown identity installs rather than silently skipping. It never uninstalls, clears data
or downgrades automatically. `agent test --variant` performs the same installed-identity proof before
claiming a build is current. `.apks` archives are extracted as provided; AndroidForge does not run
bundletool or select device-specific splits.

### 0.9 Stable element references

```powershell
androidforge ui observe --session ses-... --json
androidforge ui tap --session ses-... --ref el-... --expect-token obs-... --json
androidforge ui long-press --session ses-... --ref el-... --expect-token obs-... --receipt --json
androidforge ui text "hello" --session ses-... --ref el-... --expect-token obs-... --json
androidforge ui clear --session ses-... --ref el-... --expect-token obs-... --json
androidforge ui scroll --session ses-... --ref el-... --expect-token obs-... --direction down --json
androidforge ui wait --session ses-... --ref el-... --expect-token obs-... --timeout 10 --json
androidforge ui assert --session ses-... --ref el-... --expect-token obs-... --json
```

Every observed actionable/context/scrollable node has a deterministic opaque `ref` (12 hex characters by default). A ref is valid only
with its observation token and the same session/device. Immediately before dispatch, the CLI re-reads the
hierarchy and verifies path, class, package, resource id, text, content description, bounds, enabled and
actionable state. It refuses with explicit errors when the token expired, the node disappeared or
changed, the session/device is wrong, or the node is no longer actionable. Text/clear require an
editable field, scroll requires a scrollable node, and wait/assert may target context text. **There is
no fallback to text, resource id or coordinates.** Re-observe and choose a new ref. `--expect-token` is
required.

### 0.10 Sequential device matrix

```powershell
androidforge agent matrix --project . --avd Pixel_8_API_35 --avd Pixel_8_API_36 --flow conformance --json
androidforge agent matrix --project . --serial emulator-5554 --serial emulator-5556 --flow conformance --json
androidforge agent matrix --project . --avd Pixel_8_API_35 --avd Pixel_8_API_36 --dry-run --json
```

The matrix validates every AVD/serial, rejects duplicates, resolves the build once, and runs targets
sequentially. Each target gets its own owned session and calls the existing `agent test` pipeline.
`--fail-fast` stops after the first failure; otherwise remaining independent targets still run. The
result aggregates `passed`, `failed`, `partial` or `failed`, includes per-target sessions, API/model,
health, run ids, failures, artifacts and cleanup state. Only sessions created by this matrix with the
matrix owner are released; pre-existing sessions and emulators are not stopped. `--parallel` >1 currently
returns `E_UNSUPPORTED_OPERATION` because parallel isolation is not validated.

### 0.11 Wireless ADB and device health

```powershell
echo 123456 | androidforge devices pair 192.168.1.20:37123 --code-stdin --json
androidforge devices connect 192.168.1.20:37521 --json
androidforge devices disconnect 192.168.1.20:37521 --json
androidforge devices health --session ses-... --json
```

Pairing is separate from connecting; the ports differ. Pairing codes are never stored, logged, reported
or bundled. Prefer `--code-stdin`; `--code` may be visible to other local processes. AndroidForge does not
enable wireless debugging or `adb tcpip` automatically.

`devices health` is read-only. It reports battery, charging, temperature, thermal status, storage,
memory, network, screen, uptime, Android/API and ADB state. Unsupported values include a reason. Raw
command evidence is included unless `--no-evidence` is passed. Warning thresholds live under `[health]`.

### 0.12 Bounded read-only retries

```powershell
androidforge ui observe --session ses-... --retries 2 --json
androidforge ui diff --session ses-... --from obs-... --retries 2 --json
androidforge diagnostics logcat peek --session ses-... --retries 3 --json
```

Only read-only operations accept `--retries` (maximum 5). Retryable errors are device disconnect/offline,
timeouts and UIAutomator helper failures; selector, configuration, permission and install errors are not
retried. Backoff is 250ms, 500ms, 1s, 2s, bounded by a 20-second total window. The result reports every
attempt, code, retryability, delay and total time. Taps, text, scrolls, installs, uninstalls, data
operations, fixtures and device changes never retry.

### 0.13 Performance evidence

```powershell
androidforge diagnostics performance com.example.app --session ses-... --json
androidforge diagnostics performance com.example.app --cold --settle 2s --session ses-... --json
```

This is an explicit measurement, not a passive status command. It launches the app and writes
`performance/report.json` with startup timing, gfxinfo jank/frame statistics, meminfo PSS/RSS and CPU
usage. `--cold` is the only way to request a force-stop first; without it the running process is
measured as-is. Every unavailable metric is `null` with an `unknown_reason`, the command exits 10 when
the report is partial, and no value is estimated. The default post-launch settle is 2 seconds.
## 1. The operating model

Every meaningful test is a **run** with a **session** and a **device**.

1. `doctor --json` — confirm the SDK, ADB and emulator are discovered.
2. `capabilities --json` — see what this host can actually do.
3. `session start` (headless) or `session attach` — claim one device.
4. Do work against that session.
5. Collect evidence.
6. `session stop` or `session release` — do not leave ownership or emulators behind.

A session is the safe unit of ownership. Do not touch a device owned by another AndroidForge session.
AndroidForge coordinates AndroidForge sessions; it cannot prevent an unrelated process from using ADB.

## 2. JSON, stderr and exit codes

Use `--json` on every command. Stdout then contains exactly one JSON envelope. Progress, trace and
human diagnostics go to stderr. `--trace` also sends external argv/timeouts/exit codes to stderr.

```json
{
  "schema_version": 1,
  "status": "success",
  "command": "ui.assert",
  "exit_code": 0,
  "run_id": "20261115-111132-...",
  "session_id": "ses-...",
  "device": {"serial": "emulator-5560"},
  "result": {"matched": true},
  "artifacts": [],
  "duration_ms": 340
}
```

Branch on **numeric exit codes** (see [EXIT_CODES.md](EXIT_CODES.md)), not prose. Do not assume a command
worked because it printed a line. In JSON mode never parse human output from stdout.

## 3. Starting a device

```powershell
androidforge session start --avd <AVD> --port 5556 --headless --owner <AgentName> --json
```

- Headless is the default; use `--windowed` only when a human needs to see it.
- `--reuse` attaches to an already-running emulator for the AVD if it is unowned.
- New emulators are started with `-no-snapshot-save`; they do not silently preserve test state.
- `--port` must be an even console port. Automatic allocation starts at 5554; explicit requests
  accept even 1024–65532, including environment-reserved ports below the conventional range. ADB uses
  port + 1.
- On success, keep the returned session ID. You can export `ANDROIDFORGE_SESSION` or run
  `androidforge session use <ID>` to make it current for this project.
- `session attach --serial <SERIAL>` claims an existing emulator or physical device.
- `session start --avd ... --prepare` also disables animations and keeps the screen awake; `device
  restore` or `session stop` restores the settings it changed.
- `session release` releases ownership but keeps an owned emulator running; a later `session stop
  --session <ID>` can still stop that released tombstone if no new session owns the device.
- `session list --active` hides released/failed/abandoned tombstones.
- `session stop`, `session release` and `session restart` verify the session's `owner` label. Pass the
  same `--owner` used at start (or export `ANDROIDFORGE_OWNER`) or use `--force` for a deliberate
  takeover. This blocks one agent from stopping another agent's emulator by ID; the owner is a
  coordination label, not authentication.

If the device is owned by another session, you get `E_DEVICE_BUSY`. Do not work around that by killing
the emulator. Inspect `session list`; use `session gc --reclaim` only if the owner is genuinely gone.

## 4. Installing a build

```powershell
androidforge app install .\app-debug.apk --session <ID> --json
```

The result includes package, versionName, versionCode, local APK SHA-256, installed APK SHA-256, signing
certificate SHA-256 and debug-signed status. Installation fails if the installed package does not match
the requested package. A signing mismatch is diagnosed, **never** auto-uninstalled:

- `E_INSTALL_SIGNATURE_MISMATCH` lists both certificates and the installed version.
- To replace the app and delete its data, do it explicitly: `app install <apk> --fresh --yes`.
- To preserve debuggable app data first: `app data-backup <pkg> <file>` (debuggable apps only), then
  uninstall/install deliberately.

Never put signing secrets in a project or fixture. AndroidForge does not read keystores.

## 5. UI work

Prefer semantics over coordinates.

```powershell
androidforge ui dump --session <ID> --json
androidforge ui find --session <ID> --contains "Save" --json
androidforge ui tap --session <ID> --text "Save" --json
androidforge ui wait --session <ID> --text "Saved" --timeout 20 --json
androidforge ui assert --session <ID> --text "Saved" --json
androidforge ui assert --session <ID> --text "Something went wrong" --not-exists --json
androidforge ui settle --session <ID> --json
androidforge ui scroll-to --session <ID> --text "Finance" --json
androidforge ui text "search words" --session <ID> --class android.widget.EditText --clear --hide-keyboard --json
androidforge ui back --session <ID> --hide-keyboard --json
```

Rules:

- Exact matching is the default. Use `--contains` deliberately.
- `ui find` is a search and succeeds with `matches: []` when nothing matches; `ui assert`, `ui tap`
  and `ui wait` fail when the condition is absent.
- If a selector matches more than one element, AndroidForge refuses to guess. Use `--index N` from the
  listed candidates or make the selector more specific.
- Coordinates are real device pixels in the current orientation. Never scale from an unrelated
  screenshot.
- Text entry verifies an editable focused field, clears explicitly when requested, sends batched
  deletions, re-reads the field, and fails if the text does not match. Empty strings verify as empty
  fields. Android Settings and some OEM fields expose their hint/placeholder as `text`; AndroidForge
  reads `hint` and treats an exact hint as empty before typing and after clearing. Passwords cannot be
  read back; result says `verified=false` rather than claiming success.
- `ui clear` with no selector requires a focused editable field. If none is focused it returns
  `E_SELECTOR_NOT_FOUND` with the focused nodes and tells you to tap the field or pass a selector,
  rather than a misleading text-verification failure.
- `ui back` refuses to run while the soft keyboard is up unless `--hide-keyboard` is provided. Avoid
  fixed sleeps; use `ui wait`, `ui settle` and `ui scroll-to`.
- `ui assert` is deliberately a quick one-attempt check (default 1s). Pass `--timeout` or use `ui
  wait` first when the screen is still animating or loading.
- Compose test tags are visible only if the app exposes them as accessibility resource IDs
  (`testTagsAsResourceId`) or equivalent semantics. UIAutomator cannot see arbitrary internal Compose
  test nodes. Use instrumentation when precise internals are required.

## 6. Evidence and failure capture

Screenshots are written as binary through Rust file I/O, never terminal redirection:

```powershell
androidforge capture screenshot --session <ID> --label checkpoint --json
```

Automatic failure capture is on by default for app/UI/fixture/conformance steps. It records a screenshot,
UI hierarchy with focused nodes, logcat tail, crash/ANR state, process/task/foreground state and
`dumpsys` evidence. The error JSON includes artifact paths. Disable it only with
`--no-failure-capture`.

Logcat is never cleared by default. One-shot tails are bounded; continuous captures have explicit
time and byte caps and a `diagnostics logcat stop` lifecycle.

## 7. Runs and reports

For a multi-step flow, keep one report:

```powershell
androidforge run start --session <ID> --name smoke --json
# ... commands using the same session/run ...
androidforge run end --session <ID> --json
androidforge report show <RUN_ID> --json
androidforge report path <RUN_ID> --json
androidforge report list --json
```

`run start` makes the run active for the session and project. Every run has `report.json`,
`metadata.json` and a checksum `manifest.json`. Native commands append steps. `ANDROIDFORGE_RUN_ID` is
injected into scripts; a child script can call `androidforge` and its steps join the same run.

`run end` derives the status from its steps: any failed step makes the run failed and exits non-zero.
Use `--status failed` to make that explicit, or `--allow-failures --status passed` for a deliberate
override. `run end --run <ID>` clears the active pointer when that run is the session's active run, so
the next command starts a new run instead of appending to a finished one. A stale pointer to an
already-finished run is ignored and cleared automatically.

A run that did not install inherits the session's observed build identity; if none is known, it reads
the installed package/version once and marks hashes/certificate unverified rather than inventing them.

`report show` and `report path` also search project-local `.artifacts/runs` directories under the
current project tree. A conformance run under `examples/<App>/.artifacts/runs` can therefore be
inspected from the repository root without repeating `--project`.

For a single external process, use:

```powershell
androidforge run --session <ID> --name update-flow --capture-screen --capture-logcat -- python my_script.py
```

The external process gets the documented `ANDROIDFORGE_*` environment contract. Its stdout/stderr are
artifacts, and its exit status is in the report. AndroidForge exits with a stable failure code if the
external process failed.

`report import junit <file> --run <ID>` merges Gradle/JUnit XML into the same run. It does not replace
Gradle, Robolectric, Compose instrumentation or your existing unit tests.

Use run analysis without a second report format:

```powershell
androidforge report reconcile --dry-run --json
androidforge report reconcile --run <RUN_ID> --outcome interrupted --json
androidforge report compare <RUN_ID> --baseline <BASELINE_ID> --json
androidforge report flaky --limit 30 --min-runs 3 --json
```

`report reconcile` only finalizes a `running` report when its creator PID is gone (or when an explicit
outcome is requested). `report compare` warns when project/scenario or build/environment identity differs;
`report flaky` never classifies a step from a single failure. Both analysis payloads are documented by
`schema/analysis.schema.json`.

Live read-only captures for agents and clients:

```powershell
androidforge capture frame --session <ID> --output .\frame.png --json
androidforge diagnostics logcat peek --session <ID> --package com.example.app --level warn --since "11-16 18:00:00.000" --json
androidforge ui inspect --session <ID> --json
```

`capture frame` and `logcat peek` do not create runs; `ui inspect` returns the parsed hierarchy without
writing an artifact. `logcat peek` treats an omitted package as all processes and never clears the device
buffer.

Managed storage:

```powershell
androidforge maintenance summary --json
androidforge maintenance clean --dry-run --older-than 30d --keep-last 10 --sessions --runs --json
androidforge maintenance clean --yes --older-than 30d --keep-last 10 --sessions --runs --json
androidforge session prune --dry-run --older-than 7d --keep-last 3 --json
```

Cleanup only considers terminal sessions and runs under AndroidForge-managed paths. Active sessions and
running runs are never removed. Preview first; `--yes` is required for deletion.

## 8. Fixtures, hooks and conformance

Project-specific behavior belongs in `.androidforge/project.toml`, fixtures, hooks and scripts. Never
add app-specific code to the core.

```powershell
androidforge --project examples/CalcSuite fixture list --json
androidforge --project examples/CalcSuite fixture validate smoke --json
androidforge --project examples/CalcSuite fixture run smoke --session <ID> --json
androidforge --project examples/CalcSuite hook list --json
androidforge --project examples/CalcSuite hook run open-settings --session <ID> --json
androidforge --project examples/CalcSuite conformance run --session <ID> --json
```

A conformance result includes the same build identity as `report.json`. If the external flow prints a
final JSON object with a `checks` array, those named checks become `flow_checks` in the command result
and in the conformance step's details; the flow exit code remains the pass/fail authority. Local
install preflight failures (expected package/certificate) skip the full screenshot/logcat/dumpsys
bundle because the device was never touched.

Fixtures support phases (`pre-install`, `post-install`, `pre-test`, `post-test`, `cleanup`), retries,
per-step timeouts, `on_failure`, capture policy, explicit destructive steps, file transfer, app-private
files via `run-as` only for debuggable apps, and external scripts. They never run automatically just
because a config exists. Use `--dry-run` to inspect a plan and `--yes` for destructive steps.

Hooks are named, explicit debug triggers. AndroidForge does not interpret the hook name. Give a hook a
`verify` postcondition when ADB exit 0 is not proof that the app did the work. For a non-exported
receiver on a debuggable build, use `run_as = true` so the explicit broadcast runs as the app UID; a
plain shell broadcast can report `Broadcast completed: result=0` without delivery. If the postcondition
does not change or match, the hook fails with `E_HOOK_VERIFICATION_FAILED`. If an app cannot expose a
deterministic trigger, prefer an honest message over guessing a JobScheduler ID.

`jobs list` accepts both numeric and named/WorkManager namespaces on Android 16; parsed output remains
best-effort and raw dumps stay the source of truth.

## 9. Safe, reversible and destructive operations

**Safe/read-only:** `doctor`, `devices list/inspect`, `ui dump/find/focused`, `apk inspect/verify`,
`report show/path`, `diagnostics inspect`, `get` commands, `app status` without assertions.

**Reversible (AndroidForge records restoration):** `device prepare` (settings), `session start --prepare`,
`devices restore`, `session release`/`session stop` (restores settings before stopping).

**Destructive or disruptive:** `app clear-data` (`--yes`), `app data-restore` (`--yes`), `app install
--fresh` (`--yes`), `snapshot load` (`--yes`), `time set` (`--yes`), `report prune` (`--yes` unless
`--dry-run`), `doze force`/network airplane (`--yes`), `session stop --attached` for an externally
started emulator. Never assume these are safe because a JSON envelope came back.

## 10. Error recovery

- `E_DEVICE_AMBIGUOUS` — choose `--serial` or `--session`; run `devices list`.
- `E_DEVICE_BUSY` - another session owns the device; do not kill it.
- `E_SESSION_OWNERSHIP_MISMATCH` - pass the owner used at `session start` or use `--force` only for a deliberate takeover.
- `E_ADB_NOT_FOUND` / `E_SDK_NOT_FOUND` - run `doctor`; set `ANDROID_HOME` or `--sdk` config.
- `E_SELECTOR_NOT_FOUND` — `ui dump`, use `--contains` or a more specific resource ID; `ui wait` if loading.
- `E_SELECTOR_AMBIGUOUS` — inspect candidates and pass `--index N`.
- `E_COMMAND_TIMEOUT` — inspect device state and retry with a larger explicit timeout; do not loop forever.
- `E_DEVICE_OFFLINE` / `E_DEVICE_DISCONNECTED` — reconnect or restart only an emulator you own.
- `E_INSTALL_SIGNATURE_MISMATCH` — read the diagnosis; do not auto-uninstall.
- `E_SESSION_EXPIRED` — start/attach a new session.
- `E_UNSUPPORTED_OPERATION` — read the capability matrix; use the suggested alternative.

## 11. End-to-end example

```powershell
androidforge doctor --json
androidforge session start --avd androidforge_test --headless --owner CalcSuiteAgent --json
# ses-3457532120af, emulator-5560

androidforge app install .\CalculatorSuite-v3.0.3.apk --session ses-3457532120af --json
androidforge app launch com.calcsuite --session ses-3457532120af --json
androidforge ui assert --session ses-3457532120af --text "Calc Suite" --json
androidforge ui tap --session ses-3457532120af --text "Algebra" --json
androidforge capture screenshot --session ses-3457532120af --label algebra --json
androidforge diagnostics collect --session ses-3457532120af --package com.calcsuite --json
androidforge app status com.calcsuite --session ses-3457532120af --expect foreground --json

androidforge run start --session ses-3457532120af --name release-smoke --json
androidforge ui assert --session ses-3457532120af --text "Calc Suite" --json
androidforge --project examples/CalcSuite fixture run smoke --session ses-3457532120af --json
androidforge run end --session ses-3457532120af --json
androidforge report show latest --json

androidforge session stop --session ses-3457532120af --owner CalcSuiteAgent --json
```

The actual command surface and current verified status are in [CAPABILITIES.md](CAPABILITIES.md) and
[VERIFICATION.md](VERIFICATION.md). If a command is not listed as supported, expect a structured
`E_UNSUPPORTED_OPERATION` rather than a fake success.

## 12. Desktop cockpit

`androidforge-desktop.exe` is the human operator console. Agents should continue to use the CLI and JSON
contract; the desktop is not a second automation API and does not expose a plugin or RPC surface.

If a human may have the desktop open:

- Do not assume you are the only actor. The desktop uses the same sessions, owner label, device locks,
  runs and artifacts as the CLI. `E_DEVICE_BUSY` means a real session owns the device.
- Do not kill an emulator owned by another session. Use `session list`, `session inspect` and the
  documented owner/reclaim paths.
- A desktop operation runs the CLI as a child process. `session start`/`attach`/`release`/`stop`/`restart`
  cannot be cancelled from the desktop and run to completion.
- Cancelling a desktop install/fixture/conformance/hook terminates the child tree and leaves the
  emulator/session intact, but may leave an unfinalized `running` report. Reconcile with `report list`,
  `report show` and `session status`.
- Closing the desktop does not release or stop a session. Clean up with the CLI and the recorded owner.
- Use the aligned desktop/CLI pair. The desktop checks the report schema and refuses an unknown contract
  instead of guessing.

The desktop reads the same `report.json`, `metadata.json`, `manifest.json` and artifact paths described in
[SCHEMA.md](SCHEMA.md); it never produces a separate report format.




