# JSON Schemas and Run Layout

AndroidForge has two schema layers:

1. **Machine-checkable schemas** in `schema/`, available through `androidforge schema list|show`.
2. **The run evidence layout**, documented here and produced by every run.

## Schema command

```powershell
androidforge schema list --json
androidforge schema show androidforge.schema.json --json
androidforge schema show adapter.schema.json --json
```

The schema files:

| File | Covers |
| --- | --- |
| `androidforge.schema.json` | JSON envelope (`status`, `command`, `exit_code`, `result`, `error`, `artifacts`). |
| `adapter.schema.json` | `.androidforge/project.toml` / `config.toml`. |
| `fixture.schema.json` | Fixture manifest, declarative `state` (prefs/history/installed_version), steps and action types, including `apply_state`/`reset_state`. |
| `hook.schema.json` | Named hook entries, `run_as`, and `verify` postconditions. |
| `conformance.schema.json` | Conformance flow, expectations and JSON-path checks. |
| `analysis.schema.json` | `report compare` and `report flaky` result payloads. |
| `maintenance.schema.json` | `maintenance summary` and `maintenance clean` result payloads. |
| `context.schema.json` | `context` result sections and next-action argv. |
| `observation.schema.json` | `ui observe` observation and compact node projection. |
| `ui-diff.schema.json` | `ui diff` semantic changes. |
| `diagnosis.schema.json` | `report explain` diagnosis, evidence, actions and fingerprint. |
| `session-ensure.schema.json` | `session ensure` acquisition, readiness and lease metadata. |
| `repair-plan.schema.json` | Read-only `doctor fix --plan` issues and repair commands. |
| `agent-test.schema.json` | `agent test` stages, dry-run fields and final status. |
| `action-receipt.schema.json` | `--receipt` before/after observations, diff, effect, postcondition and completeness. |
| `recurrence.schema.json` | Recurrence fingerprints, statuses, occurrences and resolution metadata. |
| `apk-resolution.schema.json` | `apk resolve` selection rules, candidates and rejection reasons. |
| `apk-diff.schema.json` | Installed/candidate field comparison, overall equivalence and skip safety. |
| `app-verify.schema.json` | `app verify-installed` live/local verification and diff. |
| `app-data.schema.json` | `app data export/import/reset/seed` transfer and pre-import backup metadata, including scoped export (`source`, `in_project`, `in_export_area`). |
| `ui-wait.schema.json` | `ui wait`/`ui assert` selector, text-absence, hierarchy-stability and `for-bounds-stable` results (eight-field `bounds`, `stable_for_ms`). |
| `network-mock.schema.json` | `network mock serve` bound server summary (including the optional `scenario`/`scenario_mode`) and request log path. |
| `visual-diff.schema.json` | Screenshot baseline/region comparison metrics and pass/fail. |
| `session-list.schema.json` | Session classification, age, reason and active/mine filters. |
| `tools-catalog.schema.json` | Complete CLI command catalog used by the desktop tool database and generated MCP tools. |
| `bundle.schema.json` | Diagnostic bundle manifest, checksummed payloads and exclusions. |
| `health.schema.json` | Read-only device health report and unknown reasons. |
| `matrix.schema.json` | Matrix plan/results, `parallel`, per-target health, child reports and cleanup. |
| `performance.schema.json` | `diagnostics performance` startup, jank, memory, CPU, command evidence and unknown reasons. |
| `agent-brief.schema.json` | `agent brief` project/session/device/app/screen/last-run snapshot and next actions. |
| `wireless.schema.json` | Wireless pair/connect/disconnect results (never pairing codes). |
| `frames.schema.json` | `diagnostics frames` gfxinfo frame counters, percentiles, histogram, baseline deltas and thresholds. |
| `capture-launch.schema.json` | `capture launch` bounded screencap sample, first-frame detection, colours and assertion outcome. |
| `network-mock-scenario.schema.json` | Named, loopback-only network mock scenarios and resolved serve/offline configuration. |
| `visual-baseline.schema.json` | Named screenshot baselines and per project/run baseline manifests with hashes and dimensions. |
| `a11y.schema.json` | `ui audit a11y` findings, effective options, severity counts and optional contrast sample metadata. |
| `scenario.schema.json` | Versioned scenario manifests with allowlisted app/UI/fixture steps and repeat/error policy. |
| `report.schema.json` | `report.json` in every run directory, including invocation, status, steps and artifacts. |

Every schema is versioned with `schema_version = 1`, `adapter_version = 1`, fixture `version = 1`, and conformance `version = 1`. Unknown adapter fields are rejected deliberately. Both the 1.0 and 1.1 releases keep the envelope and report schema at `1`: no bump was needed, so older readers and reports stay usable and new fields are additive.

Fixture manifests may be TOML or JSON: `fixture.toml`, `fixture.json` and `*.json` files are discovered and loaded, with malformed JSON reported as an error rather than skipped. `conformance.schema.json` uses a strict `oneOf` so flow-specific fields such as `timeout_secs` are advertised only for the flow types that support them; the conformance `version` remains `1`.

`fixture apply <name>` and `fixture reset <name>` run only the declarative-state actions and return an `action` (`apply_state`/`reset_state`) plus the state `outcome` (`debuggable`, `paths`, `notes`); a dry run reports `dry_run: true` with no device calls. `network mock scenarios --json` returns a read-only `scenarios` array (`name`, `description`, `mode`, `fixture_file`); `network mock serve --scenario <name>` returns the `network-mock.schema.json` summary with `scenario` set, or `stopped_by: "offline"` with zero requests for the offline scenario. Scenario fixtures are bounded, redacted and loopback-only.

`app data export` resolves its destination before any device call: a dry run reports `source`
(`default`/`explicit`), `in_project` and `in_export_area`. A path inside the project root is refused
without `--allow-in-project` and an existing archive without `--overwrite`; `..` traversal is always
refused. Import/reset/seed require `--yes` and a debuggable app.

## Adapter `[build]` settings

`adapter.schema.json` describes the `[build]` table in `.androidforge/project.toml`. The existing
resolution fields are unchanged; the build-execution surface is additive:

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `default_variant` | string | omitted | Variant for `apk resolve` without `--variant`. |
| `default_module` | string | omitted | Module used when several outputs match. |
| `variants` | variant -> string array | `{}` | Project-relative APK paths or bounded glob patterns. |
| `conventional_fallback` | boolean | `true` | Search conventional Gradle outputs when a variant has no patterns. |
| `max_candidates` | integer >= 1 | `64` | Bound on inspected files per resolution. |
| `commands` | variant -> argv array | `{}` | `[build.commands.<variant>]` is a non-empty argv array of non-empty strings. No shell string is interpreted and no command is inferred. Keys are non-empty names of letters, digits, `.`, `_` or `-`. Missing commands remain valid. |
| `cwd` | string | omitted | Project-relative working directory: no `..`, absolute path, drive prefix or UNC root. |
| `timeout_secs` | integer >= 1 | omitted | Build-specific timeout; takes precedence over `[timeouts].build_secs`. |
| `env_passthrough` | string array | `[]` | Environment variable names only (`[A-Za-z_][A-Za-z0-9_]*`); no `=` or NUL. |

Unknown `[build]` keys are rejected (`additionalProperties: false`). These fields declare configuration
only; `androidforge config validate` reports invalid keys, argv, paths, timeouts and names, but this
surface does not execute builds.

### `androidforge build` result

`androidforge build` keeps the standard envelope and adds one `build` object to `result`:

```json
{
  "build": {
    "schema_version": 1,
    "status": "success",
    "variant": "debug",
    "module": null,
    "argv": ["cmd", "/c", "..."],
    "cwd": "A:/project",
    "timeout_ms": 1200000,
    "duration_ms": 1234,
    "exit_code": 0,
    "timed_out": false,
    "log": {
      "path": ".../artifacts/build/debug/build-<token>.log",
      "sha256": "...",
      "bytes": 1234,
      "truncated": false,
      "omitted_bytes": 0
    },
    "artifact": { "...": "ResolvedApk" },
    "warnings": []
  },
  "apk": { "...": "same ResolvedApk shape as apk resolve's selected object" },
  "install": { "...": "existing app install result, present only with --install" },
  "launch": { "...": "existing app launch result, present only with --launch/--cold" }
}
```

`status` is `success`, `failed` (returned as an error with `build_outcome` context) or `dry_run`.
Failures carry stable structured fields: `build_outcome`, `classification`, `variant`,
`failed_task`, `first_error`, `log_path`, `log_sha256`, `duration_ms`, `argv`, `exit_code`.
`--install` reuses the verified install path and its `Risk::Install` gate; `--launch` implies install
and reuses `Risk::AppControl`; `--cold` implies launch and always calls
`AppManager::launch_with_force_stop`, never the old bare force-stop path. Global `--dry-run`
executes nothing and resolves no session, ADB device or built APK; the `install`/`launch` objects
are then planning records only.

`E_BUILD_OUTPUT_TOO_LARGE` decision: truncation alone is a warning (`warnings` plus the persisted
head/tail log with `truncated: true`). The error code is used only when the bounded runner capture
removed the sole failure marker, leaving a generic non-zero exit with neither a failed task nor a
first error, so no usable failure classification remains.

## Agent loop payloads

All agent-facing commands use the normal envelope and put their command-specific object in `result`.
The new schemas cover:

- `context.schema.json` - `project`, `environment`, `session`, `device`, `app`, `screen`, `last_run`,
  `next_actions` and `warnings`. Omitted sections are not present; requested-but-unavailable sections
  have `available: false` and a reason.
- `observation.schema.json` - observation `token` (and backward-compatible `id`), session/device, stable
  `ref`/`role` node handles,
  freshness, display, foreground, focus, interactive `actionable` nodes, non-interactive `context`
  text, scrollables, visible text, per-node `clickable`/`enabled`/`editable`, hierarchy metadata and raw
  XML path. `nodes` appears only with `--full`. Refs are derived from node identity, stay stable while
  that identity is unchanged, and are accepted only together with the same observation token, session
  and device. Context nodes that are actionable through a clickable ancestor carry
  `actionable_via: "ancestor"`. `actionable_total`/`actionable_truncated` and
  `context_total`/`context_truncated` make the compact caps explicit. `ui observe --semantics` adds
  an additive bounded (64) `semantics` array with per-node `test_tag` and derived `role` on top of
  the same stable refs; Android `:id/` resource ids never get a `test_tag`. `semantics_total` and
  `semantics_truncated` report the uncapped count. Without the flag none of these fields appear,
  and stored tokens from either release remain readable.
- `ui-diff.schema.json` - from/to tokens plus compact added, removed, modified, focus, package and
  activity changes, totals and truncation flag. `--full` raises the list cap from 12 to 100. Pure layout
  movement is deliberately not reported as a modification.
- `diagnosis.schema.json` - category, code, confidence, summary, alternatives, evidence references,
  argv next actions, versioned fingerprint, project root and artifacts. Categories include
  `element_reference` for ref identity/actionability failures and `observation` for missing, expired or
  session-mismatched observation tokens, plus `ui_automation` for UIAutomator helper failures.
  `report explain latest` and `report bundle latest` are project-scoped; `latest-failure` selects the
  newest failed/partial/cancelled/interrupted run. `report` and `evidence_excerpts` appear with `--full`.
  The `scope` object exposes the session, device, tag and since filters actually applied during
  latest-selection; explicit run ids report an empty scope.
- `session-ensure.schema.json` - `session_id`, acquisition mode, readiness, lease metadata, recovery and
  warnings. Expiry is advisory (`needs_reconciliation`) and does not imply device termination.
- `repair-plan.schema.json` - read-only flag, issue severities, detection details and repair actions with
  confirmation/network/permission flags. `mutations_performed` must stay empty.
- `agent-test.schema.json` - stage list with status/duration/equivalent argv, installed-build match,
  final status (`success`, `failed`, `dry_run` or `blocked`), failure context, artifacts and report path.
  The inspect stage includes `artifact_kind`, `split_count`, `archive_sha256` and split hashes;
  `compare_build` proves installed split identity before skipping an install.
- `apk-resolution.schema.json` / `apk-diff.schema.json` - artifact `single`/`split_directory`/`split_archive`
  kind, split count/hashes, `.apks` archive SHA-256 and sorted-hash split-set equivalence.
- `performance.schema.json` - explicit cold-start flag, startup `TotalTime`/`WaitTime`/`ThisTime`, gfxinfo
  frame/jank percentiles, meminfo PSS/RSS/heaps, CPU percentages, command/evidence excerpts and
  `unknown_reason` for every unavailable metric. Results also carry `run_id`, `run_dir` and artifact
  paths. Values are measured or explicitly unknown; nothing is inferred from a version string or a
  missing field.
- `agent-brief.schema.json` - the compact project/session/device/app/screen/last-run snapshot returned
  by `agent brief`, with `next_actions` and warnings.
- `frames.schema.json` - `diagnostics frames` frame counts, janky/legacy counters, p50/p90/p95/p99,
  max frame time, bounded histogram, verbatim window excerpt and explicit baseline deltas. Missing
  counters are `null` with a warning; a sample below the quality floor is `insufficient`, never zero.
  A comparable regression is reported as `E_PERFORMANCE_REGRESSION` (exit 11).
- `capture-launch.schema.json` - `capture launch` bounded first-frame evidence: terminal status,
  sample timing, white/black counts, centre/dominant colour, run-relative frame artifacts and the
  optional first-frame assertion outcome. No video decoder and no device temp file are involved.
- `capture assert-color` / `capture contrast` sample one fresh host-only screenshot and return
  `actual`, `expected`, `tolerance` and `max_channel_delta`, or a WCAG `ratio` with `threshold`,
  sampled colours and an a11y finding in the error context. A mismatch is `E_UI_ASSERTION_FAILED`.
- `visual-baseline.schema.json` - `capture baseline save|update|compare|list` keep a validated
  `name`, PNG hash and dimensions in a per project/run manifest. `compare` adds the full
  `compare_png` diff metrics (changed pixels, ratio, mean channel delta, bounding box, optional
  region) and a mismatch is `E_UI_ASSERTION_FAILED`. `save` refuses an existing name; `update`
  replaces it deliberately; no device state changes.
- `a11y.schema.json` - `ui audit a11y` findings with `rule`, `severity`, node reference and bounded
  detail, plus effective `options`, severity/rule counts and optional `contrast` sample metadata.
  The audit is read-only; findings are deterministic and capped by `--max-findings`/`--max-nodes`.
- `ui bounds` resolves one selector through a single hierarchy dump and returns all eight integer
  fields (`left`, `top`, `right`, `bottom`, `width`, `height`, `center_x`, `center_y`).
  `ui assert-bounds` compares `bottom_y` with `--bottom-y`, `--bottom-y-gt` or `--bottom-y-lt`
  (mutually exclusive) and `--tolerance`; the result or `E_UI_ASSERTION_FAILED` context carries
  `expected`, `actual`, `tolerance`, `relation` and `passed`. Both are read-only.

### Host and device-control payloads (batch E1)

These commands use the normal envelope; their `result` objects are additive and versioned:

- `doctor.capacity` - `host` readings (`physical_total`, `physical_available`, `commit_limit`,
  `commit_available`, `disk_free`) each with `value_bytes` (`null` when unknown), `source` and
  `unavailable_reason` for unknowns; `avds`, per-AVD `estimates` (configured RAM, commit estimate,
  headroom, no disk estimate) and `notes`. `read_only` is always true. Unknown values are never
  reported as zero.
- `avd.images` - read-only `images` array with full system-image package `id`, `api_level`, `tag`,
  `abi`, SDK-relative `path` and `source`, plus `count`.
- `avd.create` - exact `argv`, `executable`, `dry_run`, `executed`, `success` and optional
  stdout/stderr/notes. A dry run sets `executed: false`; a real run requires `--yes` and never
  overwrites an existing AVD.
- `ports.list` - one entry per console port in the conventional range with `console_port`,
  `adb_port`, `grpc_port`, `serial`, `state` (`free`/`allocated`/`occupied`), per-port occupancy
  flags and optional `owner`/`session_id`/`avd` from active sessions. `summary` counts each state.
- `emulator.status` - keeps all legacy fields and adds `console_port`, `adb_port`, `grpc_port`,
  `owner`, `session_id` and a conservative `capacity_warning`. A warning is omitted when AVD RAM or
  host capacity is unknown; absence is not proof of safety.
- `devices.scenario` - a `ScenarioPlan` (`serial`, `scenario`, `risk`, `operation`, `restore_keys`,
  `will_mutate`, `dry_run`) plus `applied` and `warnings`. A dry run has `applied: false`,
  `will_mutate: false` and requires no session or device.
- `devices.tag.add|remove|clear|list` - host-only tag changes (`changed`, `tags`, `store_path`) or
  `entries` with one serial and its sorted tags. No device is contacted.
- `devices.shell` - exact `argv`, resolved `serial`, `read_only`, allowlist/deny decisions,
  truncation and redaction flags, stdout/stderr excerpts and risk classification. `--read-only`
  needs no physical grant; without it the result reports `device-mutation` and requires the
  `--allow-physical-mutation` superset. `--dry-run` returns the exact argv without a device call.
- `devices.input.tap` - resolved serial, `x`/`y` device-pixel coordinates, reported display size,
  dispatch status and risk gate. Under the `Risk::Input` consent rule on physical devices;
  `--dry-run` reports coordinates only. JSON `command` stays canonical (`devices.shell`,
  `devices.input.tap`) even when the retained `device` alias is used.
- `agent.matrix` - `device_resolution` maps each `tag:<name>` target to its resolved serials;
  `serials` is sorted and deduplicated and overlaps explicit `--serial` targets.
- `session.gc` - `forget_terminal` with `would_forget` (dry run), `forgotten` and `failures`
  (each with `session_id`, `code` and `message`). Failed cleanup never removes a foreign lock.
- `scenario.validate` - `valid`, `name`, `version`, `source`, `repeat` and an ordered `steps` plan
  (`index`, `kind`, `description`, `audited_operation`, `mutates_device`, `on_error`). Read-only.
- `scenario.run` - `name`, `dry_run`, `plan`, `iterations` and `summary`. Each iteration carries its
  `passed` flag, wall-clock `duration_ms` and per-step records (`index`, `kind`,
  `audited_operation`, `status` of `planned`/`success`/`failed`/`skipped`, optional `error_code` and
  `error_message`, `duration_ms`). `summary` aggregates `iterations`, `passed`, `failed`,
  `pass_rate`, `duration_mean_ms`, `duration_variance_ms2`, `duration_stddev_ms` and a `flake`
  object (`min_runs`, `steps_analyzed`, `flaky_steps`, `classifications`, `heuristic`). A dry run
  plans every step without dispatch, needs no consent and creates no run; a real run records each
  step in the run's `report.json` and attaches screenshot artifacts.

Fingerprints are `sha256:` values over a versioned normalized material string. They intentionally ignore
run ids, timestamps, temporary paths, memory addresses and dynamic numeric ids. See
[AGENT_GUIDE.md](AGENT_GUIDE.md#06-diagnosing-failures) for the agent workflow.

## Envelope

Every meaningful command with `--json` writes exactly one object to stdout:

```json
{
  "schema_version": 1,
  "status": "success",
  "command": "ui.assert",
  "exit_code": 0,
  "run_id": "20261115-121142-14edb404_conformance-run",
  "session_id": "ses-...",
  "device": {"serial": "emulator-5560"},
  "result": {"matched": true},
  "artifacts": [
    {"kind": "screenshot", "path": "screenshots/....png", "sha256": "...", "size_bytes": 384516}
  ],
  "duration_ms": 2132
}
```

Rules:

- `schema_version`, `status`, `command`, `exit_code` and `duration_ms` are always present.
- `profile`, `summary` and `next` appear only with `--output-profile compact|brief` (or equivalent
  config/env defaults). `profile` is `compact` or `brief`; full output remains unchanged.
- `status` is `success`, `error`, `failed` or `blocked`. `failed` and `blocked` are used by pipeline-style
  commands that preserve a structured `result` instead of an error body.
- Success responses put command-specific data in `result`.
- Error responses put the structured failure in `error` and omit `result`.
- Progress, trace and human diagnostics go to stderr, never stdout.
- `run_id`, `session_id` and `device` are present when applicable.
- Artifact paths are relative to the run directory, not the current directory.

### Additive structured-output flags

These are CLI presentation flags. They do not change `schema_version` (still `1`) or the shape of
any command payload when they are absent. They are not MCP tool arguments.

- `--jsonl` implies `--json` and writes one minified JSON object per line (NDJSON). An ordinary
  command emits one envelope line. `scenario run` emits one compact line per iteration
  (`event`, `iteration`, `status`, `duration_ms`, `steps`) before the final envelope. It composes
  with `--output-profile compact|brief`; the profile still shapes the envelope and `result`.
- `--fields a,b,c` keeps only those comma-separated top-level keys of `result`. The envelope keys
  `schema_version`, `status`, `command`, `exit_code`, `result` and `error` are never removed. An
  unknown key fails with `E_INVALID_ARGUMENT` and lists the available result keys; a non-object
  `result` fails with a suggestion. To trim envelope fields, keep using
  `--output-profile compact|brief`.
- `--stats` writes exactly one JSON line to **stderr** after the payload:
  `{"stats":{"schema_version":1,"stdout_bytes":N,"estimated_tokens":M,"token_method":"bytes_div_4","lines":L}}`.
  `stdout_bytes` is the exact number of bytes written to stdout, `estimated_tokens` is
  `ceil(N / 4)` and `lines` counts stdout lines. Stdout is unchanged, so JSON/NDJSON stays
  parseable. Stats are emitted on success and error, including with `--quiet`.

## Run directory

```
.artifacts/runs/<timestamp>-<shortid>_<command>/
  report.json
  metadata.json
  manifest.json
  build/
    install-identity.json
  screenshots/
  ui/
  logs/
  recordings/
  diagnostics/
  junit/
```

`report show latest` and `report path <run>` find the directory. They search the current artifacts
directory, then project-local `.artifacts/runs` directories under the project tree, so a conformance
run can be inspected from the repository root without repeating `--project`. `report show` returns
the report; `report path` returns `report.json`, `metadata.json`, `manifest.json` and the run
directory.

### report.json

```json
{
  "schema_version": 1,
  "run_id": "20261115-121142-14edb404_conformance-run",
  "command": "conformance.run",
  "project": "DiscountHunter",
  "project_root": "A:\\...\\examples\\DiscountHunter",
  "invocation": {
    "argv": ["androidforge", "conformance", "run", "--session", "ses-...", "--json"],
    "cwd": "A:\\Projects\\Active\\AndroidForge",
    "project_root": "A:\\...\\examples\\DiscountHunter",
    "captured_at": "..."
  },
  "status": "passed",
  "exit_code": 0,
  "started_at": "2026-11-15T12:11:42.143Z",
  "ended_at": "2026-11-15T12:12:26.658Z",
  "duration_ms": 44515,
  "session_id": "ses-...",
  "device_serial": "emulator-5560",
  "device": {"serial": "emulator-5560", "api_level": 36, "android_version": "16"},
  "tags": ["release", "smoke"],
  "meta": {"build": "1.2.3", "branch": "main"},
  "build": {
    "package": "com.discounthunter.android.debug",
    "version_name": "1.1.2-release-debug",
    "version_code": 10,
    "apk_sha256": "...",
    "installed_apk_sha256": "...",
    "cert_sha256": "...",
    "debug_signed": true,
    "verified": true
  },
  "steps": [
    {
      "id": "step-001",
      "name": "conformance:release-smoke",
      "command": "conformance release-smoke",
      "status": "success",
      "exit_code": 0,
      "retries": 0,
      "started_at": "...",
      "ended_at": "...",
      "duration_ms": 43714,
      "details": {
        "checks": [],
        "flow_checks": [
          {"name": "app foreground", "passed": true, "detail": {"activity": "MainActivity"}}
        ],
        "build": {"package": "com.discounthunter.android.debug", "verified": true}
      },
      "artifacts": []
    }
  ],
  "artifacts": []
}
```

Run status values are `running`, `passed`, `failed`, `partial`, `cancelled`, `interrupted`, `timed_out` and `abandoned`. `invocation` is optional: v0.6.0 records the exact CLI argv; older reports omit it and bundle generation marks reproduction as non-exact. Step statuses are
`running`, `success`, `failed` and `skipped`.

`tags` (an array of strings) and `meta` (a string-to-string map) are optional fields added in 1.1.0
by `run start --tag` and `run start --meta`; reports written by 1.0.0 omit both and deserialize to
empty values. The same fields are mirrored into `metadata.json`. Tags use AND semantics when passed
to `report list`, `report explain`, `report bundle` and compare baseline selection. `since` accepts
an RFC3339 instant or a relative duration and is resolved at parse time; the applied filter is
returned as `scope` by `report explain`.

`build` is only authoritative after an install observes it. Installs persist the identity in the
session, so later runs and conformance reports in the same session inherit the package, version,
APK/installed hash and certificate. If no install was observed, AndroidForge may fill the installed
package/version from the device once and mark `verified: false`; it never copies expected certificates
into observed fields.

### metadata.json

```json
{
  "schema_version": 1,
  "run_id": "...",
  "command": "app.install",
  "project": "DiscountHunter",
  "project_config": ".../project.toml",
  "project_root": "...",
  "session_id": "ses-...",
  "tags": ["release", "smoke"],
  "meta": {"build": "1.2.3", "branch": "main"},
  "started_at": "...",
  "tool": {"name": "androidforge", "version": "1.1.0", "host_os": "windows", "host_arch": "x86_64"},
  "tool_versions": {"adb": "...", "emulator": "...", "aapt2": "...", "apksigner": "...", "java": "..."},
  "device": {"serial": "emulator-5560", "api_level": 36}
}
```

### manifest.json

```json
{
  "schema_version": 1,
  "run_id": "...",
  "generated_at": "...",
  "files": [
    {"path": "report.json", "size_bytes": 12736, "sha256": "..."},
    {"path": "screenshots/....png", "size_bytes": 129253, "sha256": "..."}
  ]
}
```

The manifest is regenerated as artifacts are added. It is the checksum index for evidence. Verify it before trusting a copied report.

### Artifact references

```json
{
  "kind": "screenshot",
  "path": "screenshots/20261115-121217-394_discount-hunter-alerts.png",
  "sha256": "dc67e1f9...",
  "size_bytes": 384516,
  "description": "discount-hunter-alerts"
}
```

Common kinds: `screenshot`, `screenshot_metadata`, `recording`, `ui_hierarchy`, `logcat_raw`, `logcat_filtered`, `crash_report`, `app_status`, `dumpsys`, `command_output`, `build_identity`, `file_transfer`, `junit`.

## Redaction

Reports and text artifacts are redacted before writing when `[diagnostics].redact = true` (the default). Redaction preserves structure: XML attributes like `password="false"` remain readable booleans, while secret-like values are replaced with `[REDACTED]`. Never rely on redaction as a substitute for keeping secrets out of project files.

`artifacts scan-secrets` is a read-only, host-only scan of `.artifacts` text. Its result carries `files_scanned`, `files_skipped_binary`, `files_skipped_oversized`, `findings_total`, `truncated` and `findings[]`. Each finding has a relative `path`, `line`, `kind`, category-derived `preview`, `start` and `end`; the matched value is never stored or printed. Binary media is disclosed as not scanned and all work is bounded.

`diagnostics logcat tail|peek|query|watch|start` accept repeatable/comma-separated `--redact-patterns`. Every value is a literal token prefix (`AIza`, `sk-`, `ghp_`, `AKIA`, ...), never a regular expression supplied by the caller. Redaction is applied before printing, NDJSON emission and stored capture text. For `start`, prefixes are persisted in a CLI-owned sidecar and applied by `stop` before the capture file is registered with checksums; absence of the flag leaves the original bytes unchanged.

`report bundle --redact-secrets` forces text redaction even when both config switches are off, scans assembled text payloads afterwards, and returns `E_UNSUPPORTED_OPERATION` (exit 3) with counts/kinds only if any suspected secret remains. No partial bundle is written. The bundle manifest adds `redact_secrets` and `secret_scan` additively. Binary screenshots and recordings are opt-in and are not pixel-redacted.

## JUnit import

`report import junit <file> --run <ID>` parses one or more `<testsuite>` elements, writes the source XML under `junit/`, adds a step with counts and failures, and checksums it in the manifest. Invalid XML returns `E_JUNIT_INVALID`. It does not execute Gradle or replace existing test workflows.

## Run analysis

`report compare` returns a `RunComparison` object. `comparable` is false when project or scenario
identity differs; `notes` explains build/environment differences that could be mistaken for a test
regression. Step changes use `regressed`, `fixed`, `failed_both`, `unchanged`, `added`, `removed` or
`unknown`. `report flaky` returns a heuristic report: a step is classified `flaky` only when it has at
least one success and one failure across at least `--min-runs` compatible terminal runs. Cancelled,
interrupted, timed-out and abandoned runs are excluded. Both payloads are described by
`schema/analysis.schema.json`.

With a budget (`--budget <json>` or a `--max-*` threshold flag), `compare_with_budget` adds
`performance`, `budget`, `budget_checks`, `budget_violations` and `budget_passed`. Metrics with no
value on either side are skipped rather than failed. Absolute ceilings (`startup_ms`,
`jank_percent`, `janky_frames`, `max_frame_ms`, `apk_size_bytes`, `failed_tests`) compare the current
value; `*_delta_*` fields cap the positive change from the baseline; `metrics` / `metric_deltas`
apply to named flattened step metrics. Without a budget these fields are absent, preserving the
1.0.0 payload. `--fail-on-regression` maps a failed check to `E_PERFORMANCE_REGRESSION` (exit 11)
and copies the comparison plus `budget_violations` into `error.context`. Baseline selection shares
the report scope (session, device, tags, since).

## Maintenance

`maintenance summary` reports managed paths, byte counts, active/terminal sessions and runs, stale
sessions, largest runs and the terminal-session list. `maintenance clean` defaults to a dry run and
requires `--yes` for deletion; `--sessions`, `--runs`, `--older-than` and `--keep-last` select the
eligible set. `--keep-tag` is repeatable and protects runs carrying any listed tag; using it without
`--yes` forces a dry run. Active sessions, running runs, active-run pointers and protected tags are
always excluded. The result keeps the legacy `runs` summary and adds an additive, versioned
`retention` object with `deletions` (path, relative path, bytes, reason and bounded artifact list),
`kept_runs` with reasons, `reclaimed_bytes`, `deferred`, `requires_confirmation` and `executed`.
Both payloads are described by `schema/maintenance.schema.json`.
