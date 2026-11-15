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

Every schema is versioned with `schema_version = 1`, `adapter_version = 1`, fixture `version = 1`, and conformance `version = 1`. Unknown adapter fields are rejected deliberately.

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
- `status` is `success` or `error`.
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

Run status values are `running`, `passed`, `failed`, `partial` and `interrupted`. Step statuses are
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
