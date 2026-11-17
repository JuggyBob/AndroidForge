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
| `fixture.schema.json` | Fixture manifest, steps and action types. |
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
| `bundle.schema.json` | Diagnostic bundle manifest, checksummed payloads and exclusions. |

Every schema is versioned with `schema_version = 1`, `adapter_version = 1`, fixture `version = 1`, and conformance `version = 1`. Unknown adapter fields are rejected deliberately.

## Agent loop payloads

All agent-facing commands use the normal envelope and put their command-specific object in `result`.
The new schemas cover:

- `context.schema.json` - `project`, `environment`, `session`, `device`, `app`, `screen`, `last_run`,
  `next_actions` and `warnings`. Omitted sections are not present; requested-but-unavailable sections
  have `available: false` and a reason.
- `observation.schema.json` - observation `token` (and backward-compatible `id`), session/device,
  freshness, display, foreground, focus, interactive `actionable` nodes, non-interactive `context`
  text, scrollables, visible text, per-node `clickable`/`enabled`/`editable`, hierarchy metadata and raw
  XML path. `nodes` appears only with `--full`. `actionable_total`/`actionable_truncated` and
  `context_total`/`context_truncated` make the compact caps explicit.
- `ui-diff.schema.json` - from/to tokens plus compact added, removed, modified, focus, package and
  activity changes, totals and truncation flag. `--full` raises the list cap from 12 to 100. Pure layout
  movement is deliberately not reported as a modification.
- `diagnosis.schema.json` - category, code, confidence, summary, alternatives, evidence references,
  argv next actions, versioned fingerprint, project root and artifacts. Categories include `ui_automation`
  for UIAutomator helper failures. `report` and `evidence_excerpts` appear with `--full`.
- `session-ensure.schema.json` - `session_id`, acquisition mode, readiness, lease metadata, recovery and
  warnings. Expiry is advisory (`needs_reconciliation`) and does not imply device termination.
- `repair-plan.schema.json` - read-only flag, issue severities, detection details and repair actions with
  confirmation/network/permission flags. `mutations_performed` must stay empty.
- `agent-test.schema.json` - stage list with status/duration/equivalent argv, installed-build match,
  final status (`success`, `failed`, `dry_run` or `blocked`), failure context, artifacts and report path.

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
- `status` is `success`, `error`, `failed` or `blocked`. `failed` and `blocked` are used by pipeline-style
  commands that preserve a structured `result` instead of an error body.
- Success responses put command-specific data in `result`.
- Error responses put the structured failure in `error` and omit `result`.
- Progress, trace and human diagnostics go to stderr, never stdout.
- `run_id`, `session_id` and `device` are present when applicable.
- Artifact paths are relative to the run directory, not the current directory.

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
  "started_at": "...",
  "tool": {"name": "androidforge", "version": "0.1.0", "host_os": "windows", "host_arch": "x86_64"},
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

## Maintenance

`maintenance summary` reports managed paths, byte counts, active/terminal sessions and runs, stale
sessions, largest runs and the terminal-session list. `maintenance clean` defaults to a dry run and
requires `--yes` for deletion; `--sessions`, `--runs`, `--older-than` and `--keep-last` select the
eligible set. Active sessions and running runs are always excluded. The result lists every pruned
session and deleted run. Both payloads are described by `schema/maintenance.schema.json`.
