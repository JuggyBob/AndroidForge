# Security and Trust Boundaries

AndroidForge is a local developer tool. It is not a sandbox and does not claim to protect a host from code the project itself asks it to run.

## Local-first by design

- No cloud service, telemetry, analytics, crash upload, account, API key or LLM integration.
- No network listener or daemon.
- External network access happens only through Android tools, project scripts and hooks that the user explicitly invokes.
- Reports and artifacts stay in the project artifact directory and the local state directory.

## Secrets

- AndroidForge never reads keystores, signing passwords or `gradle.properties` secrets.
- A fixture or hook value may reference `${ENV:NAME}`. The referenced value is resolved at run time and is not written to reports or artifacts.
- Logcat, dumps and error output pass through redaction by default. Common token/password/authorization shapes are replaced with `[REDACTED]`; structural XML booleans such as `password="false"` are preserved.
- Redaction is best-effort. A secret embedded in an app's own custom log line can still escape if it does not match a known shape. Treat artifacts as sensitive test data.
- Do not put secrets directly in `.androidforge/project.toml`, fixtures, hook actions or conformance scripts. Use environment references and a local secret manager.

## APK signing

- AndroidForge verifies signatures and records certificate SHA-256 values. It does not sign APKs, manage keystores or publish releases.
- A package/signature mismatch is diagnosed with both identities. AndroidForge never uninstalls the existing app automatically.
- `--fresh --yes` is an explicit request to replace the app and delete its data.

## Project code execution

Fixtures, hooks and conformance flows execute project-authored commands with the same OS privileges as the user running AndroidForge. Review them like any build script.

- Fixture scripts receive the documented `ANDROIDFORGE_*` environment contract.
- Hooks can run arbitrary shell commands, broadcasts, deep links or instrumentation.
- `androidforge adb` is explicitly unsafe: it preserves argument boundaries but bypasses AndroidForge's higher-level safety and returns raw output.
- `--dry-run` is the intended way to inspect a fixture before execution.

## Desktop cockpit

- The desktop has no network listener, cloud sync, telemetry or embedded browser. It invokes the local
  CLI and renders its files.
- The CLI path is user-configurable and is executed with the user's privileges. Point it only at an
  `androidforge.exe` you trust.
- Evidence artifacts are resolved inside the selected run directory; parent-directory escapes are
  rejected. Image decoding uses the native `image` crate and text previews are bounded.
- Cancellation terminates the child process tree for cancellable runs and never issues a session stop or
  release. Session lifecycle operations are not cancellable.

## Device ownership

- Session and device locks coordinate AndroidForge processes only. They do not stop an unrelated ADB client, IDE or script from using the same device.
- AndroidForge stops only emulators it started, or an attached emulator when `--stop-attached` is explicit.
- Do not "fix" `E_DEVICE_BUSY` by killing the emulator. Inspect sessions and reclaim only genuinely dead owners.
- Physical devices may contain personal data. Use dedicated test devices or emulators, and prefer fixture-scoped app data.

## Filesystem boundaries

- Artifacts are written under the configured artifact directory. Artifact paths are validated against traversal.
- App-private file operations use `run-as` only for debuggable apps and require relative remote paths without `..`.
- Data backup/restore is capped and requires explicit confirmation. Verify backups before destructive flows.

## Reporting a security issue

Do **not** open a public issue for a suspected vulnerability.

Use GitHub's private vulnerability reporting on the repository's **Security** tab. Include:

- AndroidForge version and binary SHA-256.
- Host OS and Rust version.
- The command, `run_id` and structured JSON error if available.
- A minimal reproduction and the affected trust boundary.
- Whether the issue requires a malicious project adapter, fixture, hook or conformance script.

Do not include real signing secrets, keystores, personal device data or production credentials.
Redact paths, serial numbers, package names and app data where possible.

## Supported versions

AndroidForge is a pre-1.0 stable release line. Security fixes target the latest published release on `main`.
Older pre-release builds may not receive fixes.

## Scope

In scope: path traversal, unsafe artifact writes, command/argument injection in AndroidForge itself,
unintended device ownership bypass, secret leakage in reports, desktop artifact path escapes or unsafe
CLI-path execution, and privilege boundaries around `run-as`, hooks, fixtures and raw ADB.

Out of scope: behavior caused by a project-authored command that the user explicitly runs, an
unrelated ADB client modifying the same device, an already-compromised host, and physical access to an
unlocked test device.
