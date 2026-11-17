# Project Adapter, Fixtures, Hooks and Conformance

## Adapter location

The primary location is `.androidforge/project.toml` (or `.androidforge/config.toml`). For portfolio
examples, a root `project.toml` containing `adapter_version` is also accepted.

Configuration precedence: CLI/profile values > environment contract > project adapter > global config >
defaults.

```toml
adapter_version = 1
min_androidforge_version = "0.1.0"

[project]
name = "Example App"
package = "com.example.app"
debug_package = "com.example.app.debug"
expected_cert_sha256 = ["...64 hex..."]
default_avd = "Pixel_8_API_35"
artifact_dir = ".artifacts"
retention_days = 14
verify_installed_apk = true

[fixtures]
directory = ".androidforge/fixtures"
default = "clean-install"

[conformance]
flow = ".androidforge/conformance/release-smoke.toml"

[build]
default_variant = "debug"
default_module = "app"
conventional_fallback = true

[build.variants]
debug = ["app/build/outputs/apk/debug/*.apk"]
"freeDebug" = ["app/build/outputs/apk/free/debug/*.apk"]
release = ["app/build/outputs/apk/release/app-release.apk"]

[ui]
backend = "uiautomator"
default_timeout_secs = 15
poll_interval_ms = 300
settle_interval_ms = 400
text_verify = true
tap_clickable_ancestor = true

[maestro]
enabled = true
binary = "maestro"
timeout_secs = 900

[diagnostics]
logcat_tail_lines = 2000
max_log_bytes = 5242880
capture_on_failure = true
redact = true
include_dumps = true
record_max_seconds = 180
logcat_max_seconds = 900

[hooks.open-settings]
type = "shell"
action = "am start -a android.settings.SETTINGS"
timeout_secs = 30
idempotent = true
```

`expected_cert_sha256` is enforced before an install proceeds; list both release and debug
certificates when both builds are installed. SDK paths, build-tools, timeouts and redaction patterns
may also be set. Run `androidforge config validate --json`; unknown keys are rejected deliberately.

`[build.variants]` powers `apk resolve` and `agent test --variant`. Values are project-relative
explicit paths or bounded glob patterns (`*`, `?`, `**`). `apk resolve` never runs Gradle. When no explicit
patterns are configured, conventional `app/build/outputs/apk` locations are tried unless
`conventional_fallback = false`. `default_module`
filters candidates to that module directory; `max_candidates` bounds inspection. Equal timestamps with
different content are reported as ambiguity rather than guessed.

## Fixture format

A fixture is a versioned manifest, either `.androidforge/fixtures/<name>.toml` or
`.androidforge/fixtures/<name>/fixture.toml`.

```toml
version = 1
name = "clean-install"
description = "One deterministic starting state"
default_timeout_secs = 300
requires_confirmation = false

[vars]
SEED = "fixtures/seed.sql"

[[steps]]
name = "install debug build"
phase = "pre-install"
action = { type = "install", apk = "${ANDROIDFORGE_APK}", fresh = false }

[[steps]]
name = "push seed"
phase = "post-install"
action = { type = "app_file_push", package = "com.example.app", local = "${SEED}", remote = "databases/seed.db" }

[[steps]]
name = "grant notification permission"
phase = "pre-test"
action = { type = "grant", permission = "android.permission.POST_NOTIFICATIONS" }

[[steps]]
name = "launch"
phase = "pre-test"
action = { type = "launch", package = "com.example.app" }

[[steps]]
name = "wait for shell"
phase = "pre-test"
action = { type = "assert", text_contains = "Home", timeout_secs = 20 }

[[steps]]
name = "cleanup notification permission"
phase = "cleanup"
action = { type = "revoke", permission = "android.permission.POST_NOTIFICATIONS", on_failure = "continue" }
```

Step fields: `name`, `phase`, `timeout_secs`, `retries` (0–20), `on_failure` (`stop`, `continue`,
`cleanup`), `capture` (`none`, `on-failure`, `always`), `destructive`, `action`.

Action types: `adb`, `shell`, `install`, `uninstall`, `clear_data`, `launch`, `force_stop`, `grant`,
`revoke`, `app_op`, `push`, `pull`, `app_file_push`, `app_file_pull`, `settings`, `wait`, `assert`,
`script`, `backup_app_data`, `restore_app_data`. Each has a timeout and runs through the shared
process/ADB runner.

Rules:

- Nothing runs automatically just because the config exists. `fixture run` is explicit.
- `--dry-run` prints the plan and records skipped steps.
- `--yes` is required for destructive steps (`clear_data`, `uninstall`, `restore_app_data`, fresh install).
- `--var KEY=VALUE` supplies values. `${PROJECT_ROOT}` is available. `${ENV:NAME}` resolves a local secret
  and is not saved to artifacts.
- Fixture scripts receive `ANDROIDFORGE_SESSION`, `ANDROIDFORGE_SERIAL`, `ANDROIDFORGE_PACKAGE`,
  `ANDROIDFORGE_APK`, `ANDROIDFORGE_ARTIFACTS`, `ANDROIDFORGE_PHASE`, `ANDROIDFORGE_ADB`,
  `ANDROIDFORGE_ROOT`, `ANDROIDFORGE_RUN_ID`, `ANDROIDFORGE_PROJECT`, `ANDROIDFORGE_EXE`.
- App-private file access requires a debuggable app and uses `run-as`; AndroidForge never roots the
  device. External app files use `--external`.
- App data backup/restore uses `run-as` + `tar` for debuggable apps and is capped; verify on your device.

## Hooks

Hooks are named debug triggers provided by the project. Types: `broadcast`, `deep-link`, `shell`,
`instrumentation`, `command`. The core does not interpret the hook name.

```toml
[hooks.check-background-work]
type = "broadcast"
action = "com.example.DEBUG_RUN_CHECK"
component = "com.example.app/.DebugReceiver"
timeout_secs = 30
idempotent = false

# Postcondition: ADB exit 0 is not proof that the app did work.
[hooks.check-background-work.verify]
shell = "run-as com.example.app cat files/status.json"
must_change = true
timeout_secs = 30
```

`androidforge hook list --json` / `androidforge hook run <name> --session <ID> --json` are explicit.
Hooks are timeout-bounded and return structured results. If an app has no deterministic trigger, say so
instead of guessing a dynamic job ID. JobScheduler IDs are never hardcoded and `jobs run` is
best-effort; the app hook is the reliable path.

For a non-exported broadcast receiver on a debuggable build, set `run_as = true`. AndroidForge then
sends the explicit broadcast as the app UID (`run-as <package> am broadcast --user 0 ...`), which the
app can receive without weakening its manifest. This does not work for release builds.

Hook verification supports:

- `shell` or `command` to read app state;
- `must_change` to require output different from the pre-hook baseline;
- `expect_contains` / `expect_not_contains`;
- `timeout_secs` and `env`.

The verification command is polled until it passes or times out. A failed postcondition returns
`E_HOOK_VERIFICATION_FAILED` and prevents the conformance from treating the hook as proof.

## Conformance

A conformance is a thin manifest, not a scripting language:

```toml
version = 1
name = "release-smoke"
description = "One real UI assertion"
setup_fixture = "clean-install"

[flow]
type = "script"          # script | maestro | fixture
command = ["python", "conformance/release_smoke.py"]
timeout_secs = 300

[expect]
status = "success"
exit_code = 0
artifacts = ["screenshots/*.png", "logs/**/logcat*.txt"]
# json_paths = { "build.package" = "com.example.app" }
```

The script receives the environment contract and can call `androidforge` recursively; it uses the same
run via `ANDROIDFORGE_RUN_ID`. AndroidForge records the conformance step, the external output, and
checks expected status/exit/artifact/JSON fields.

If the script's final JSON line contains a `checks` array, each entry's `name`, `passed`, optional
`expected`, optional `actual` and optional `detail` become `flow_checks` in the command result and in
the conformance step's details. The flow exit code remains the pass/fail authority; a reported failed
check adds a warning. The conformance result also carries the run's measured `build` identity (package,
version, APK/installed hash and certificate when known).

## Adapter versioning

`adapter_version = 1` is frozen for this release. Unsupported versions fail with `E_ADAPTER_INVALID`.
`min_androidforge_version` is checked before commands run. Future versions require migration.
