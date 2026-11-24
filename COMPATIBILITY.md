# Compatibility and deprecation policy

This policy applies to the **stable 1.0.x line** and to future releases until this document is
replaced. It answers one question: what may an agent or script rely on, and how will AndroidForge tell
them when that changes?

AndroidForge is at 1.0. The stable tiers below are the compatibility promise for the supported host
matrix; the 1.0 release also ships documented breaking safety and schema changes (see the
`BREAKING CHANGES` section of `CHANGELOG.md` and [v1.0.0 release notes](docs/release-notes/v1.0.0.md)).

## 1. Stability tiers

| Surface | Tier | Promise |
| --- | --- | --- |
| Command names and documented flags | **stable** | Not removed or renamed without a deprecation cycle. |
| JSON envelope (`status`, `command`, `exit_code`, `result`, `error`, `artifacts`) | **stable** | Existing fields keep their meaning. New fields may be added. |
| Error codes (`E_*`) and exit codes | **stable** | Not renamed or repurposed without a deprecation cycle. |
| Report schema `1` fields | **stable** | Readers must ignore unknown fields; removed fields require a schema bump. |
| Observation/ref tokens and their lifetime rules | **stable contract** | Tokens remain session/boot-scoped; invalid tokens fail closed. |
| Curated MCP tool names and input fields | **stable** | Existing names/fields remain usable; new optional fields are additive. |
| Generated MCP tools (`androidforge_<command_path>`) | **additive** | Names follow the CLI path; if a CLI command is deprecated, its generated tool follows the same cycle. |
| `best-effort` integrations (Maestro, instrumentation, vendor dumpsys parsing) | **best-effort** | May change when the external tool or Android output changes; failures must be explicit. |
| `experimental` features | **experimental** | May change or be removed in any release; help text and docs label them. |
| Internal modules, Rust APIs and on-disk state files | **internal** | Not a public compatibility surface. Use the CLI/JSON contract. |

`CAPABILITIES.md` is the authority for whether a capability is supported, best-effort or unsupported in
a given build.

## 2. Semantic versioning from 1.0

- **Patch (1.0.x):** bug fixes, security/dependency fixes, documentation and additive fields that do
  not change documented behavior.
- **Minor (1.x.0):** new commands/features, additive schema fields, new MCP tools, or a deprecation
  that is announced in the release notes.
- **Major (x.0.0):** reserved for incompatible contract changes. The 1.0 release makes the stable
  tiers above the compatibility promise for the supported host matrix. Breaking changes that were
  required to close safety gaps before 1.0 are listed once in `CHANGELOG.md`; later major releases
  follow the deprecation process in section 3.

The version is reported by `androidforge --version`, `androidforge capabilities --json` and every run
report. The desktop and CLI on the same machine must be the same version, including the patch version;
the desktop refuses a mismatched CLI rather than guessing.

## 3. Deprecation process

When a stable command, flag, JSON field, error code, exit code or curated MCP tool must change:

1. **Announce:** add the deprecation to `CHANGELOG.md` and `docs/release-notes/<version>.md`.
2. **Warn:** keep the old spelling working and emit a structured warning in JSON
   (`warnings` or an additive `deprecated` field) plus a human warning on stderr. Never put warnings on
   stdout in JSON mode.
3. **Document the replacement:** name the new command/flag/field and show a before/after example.
4. **Keep an alias for at least one minor release** (normally 90 days). A security fix may shorten this
   window when keeping the old behavior would be unsafe; the release notes must say why.
5. **Remove only in a major release**, or in a minor release after the announced window and with an
   explicit `BREAKING` section in the changelog. Removal must produce a clear error naming the
   replacement, not an unknown-command parser dump.

Experimental features, best-effort external integrations and internal state files are exempt. They may
change at any time, but the change must still be documented.

## 4. JSON envelope

Every command in JSON mode emits one object:

```json
{
  "schema_version": 1,
  "status": "success",
  "command": "app.status",
  "exit_code": 0,
  "result": {},
  "error": null,
  "artifacts": [],
  "duration_ms": 12
}
```

Rules:

- `schema_version` describes the envelope, not the command payload. It is currently `1`.
- `status` is `success` or `error`; a non-zero `exit_code` accompanies an error.
- `error` is present on failure with `code`, `message`, `recoverable`, `suggestions`, `context` and any
  artifacts. New context keys may be added.
- `result` is command-specific. New fields are additive. Consumers must ignore fields they do not know.
- `artifacts` entries have stable `kind`, `path`, `size_bytes` and `sha256` fields; extra fields are
  additive.
- Human mode may change wording at any time. Scripts must use `--json`.
- Diagnostics and progress go to stderr; stdout contains only the envelope in JSON mode.

## 5. Report schema

Run reports use `schema_version: 1` in `report.json`, `metadata.json` and `manifest.json`.

- New optional fields may be added in a patch/minor release.
- Existing field meanings and types do not change within schema `1`.
- A removed or incompatible field requires a new report schema version and a documented migration.
- Readers must tolerate unknown fields and preserve raw JSON when forwarding evidence.
- The `androidforge report` commands remain able to read older reports on the same schema.
- The 1.0 release deliberately keeps both the envelope and report schema at `1`; existing readers do
  not need a migration.

## 6. Exit codes and error codes

`EXIT_CODES.md` is the authoritative list. Exit codes are part of the automation contract:

- `0` success; `2` invalid argument/command; `3` toolchain/SDK; `4` device/session; `5` timeout;
  `6` command/package/install failure; `7` UI assertion; `8` configuration/adapter/fixture; `9` internal
  or artifact failure; `10` partial evidence; `130` interrupted.
- Error codes are stable identifiers. New codes may be added. A code is never reused for a different
  meaning without a deprecation cycle.
- Error `context` and `suggestions` are additive; consumers must not parse human messages for logic.
- `--dry-run` reports intent and requirements without mutating the device or project.

## 7. MCP compatibility

The MCP server shells out to the same CLI. Its tool catalog has two layers:

- **Curated tools** (`androidforge_app_install`, `androidforge_ui_tap`, etc.) have hand-written,
  safety-reviewed schemas. Their names and existing fields are stable.
- **Generated tools** (`androidforge_<command_path>`) cover every remaining bounded, non-unsafe CLI
  leaf command. They are additive and mirror CLI argument names. A generated tool is a convenience
  wrapper, not a second implementation; CLI behavior and safety rules win.
- Destructive and device-mutating tools require `confirm=true`. `allow_physical_mutation` is not part
  of any MCP tool schema and is never forwarded from an MCP call; physical mutation must be authorized
  on the CLI, outside the MCP request.
- Raw `adb` and `run exec` are classified `unsafe` and excluded from generated MCP tools, as are
  long-running catalog entries. They remain CLI-only escape hatches and are shown with an unsafe badge
  in the desktop tool database. The one server wrapper exposed through MCP is the curated
  `androidforge_network_mock_serve`, which requires `max_requests` or `duration` and returns.
- Tool schema changes are additive. Removing a curated tool or required field follows the deprecation
  process. Adding a new CLI command may add a generated tool without a major release.
- The server refuses unknown tools and malformed arguments. It never falls back to a shell string.

## 8. Supported platforms

For the 1.0.x stable line, **Windows 10/11 x86_64 is the tested host**. Linux and macOS are not
validated for the 1.0 line. The CLI is written without Windows-only logic where practical, but no
compatibility promise is made for untested hosts. Release binaries are unsigned; SmartScreen warnings
are expected and are not a compatibility defect.

Android support is validated on API 36 emulators and a Xiaomi HyperOS physical device for read-only and
approved mutation paths. Vendor ROM behavior remains best-effort.

## 9. Security and emergency changes

Security fixes may require a faster change. AndroidForge will:

- prefer a backwards-compatible fix;
- document the affected versions and the minimum safe version;
- use a patch release;
- state any unavoidable behavior change in the changelog and release notes.

## 10. What is not covered

- Human-readable output formatting.
- Internal Rust APIs, crate layout, file names inside state directories, and log wording.
- Exact ordering of list results unless documented on the command.
- Best-effort third-party parsers (vendor dumpsys, Maestro output, OCR accuracy).
- Release asset byte-for-byte reproducibility across build hosts.
- Unsigned-binary warnings from Windows SmartScreen.

## 11. How to report a compatibility problem

Open a GitHub issue with:

1. `androidforge --version` and `androidforge doctor --json`;
2. the exact `--json` command and the envelope;
3. the version that last worked;
4. whether a script, MCP client or desktop client is affected.

Do not include keystores, tokens, `.env` files or device pairing codes.
