# Contributing to AndroidForge

Thanks for taking the time to contribute. AndroidForge is a pre-1.0 stable release line. Bug reports,
reproduction cases and small, focused pull requests are especially valuable.

## Before you start

- Read [README.md](README.md), [ARCHITECTURE.md](ARCHITECTURE.md) and [AGENT_GUIDE.md](AGENT_GUIDE.md).
- Search existing issues and discussions before opening a new one.
- For security issues, **do not open a public issue**. Follow [SECURITY.md](SECURITY.md).
- Never include signing secrets, keystores, real user data or personal device dumps in an issue or PR.

## Reporting bugs

Use the bug report template and include:

- AndroidForge version (`androidforge --version`) and binary SHA-256 if you built it yourself.
- Host OS, Rust version, Android SDK/build-tools versions, device or emulator image.
- The exact command and whether `--json`, `--trace` or `--project` was used.
- The structured JSON error, exit code and `run_id` if one was created.
- A minimal reproduction. Redact paths, serial numbers, package names and app data when necessary.

## Suggesting features

Use the feature request template. Explain the workflow, who benefits and what the tool should do when
it cannot complete the operation. Unsupported capabilities must return a useful
`E_UNSUPPORTED_OPERATION` rather than a stub or a silent no-op.

## Development setup

Requirements:

- Rust `1.95` or newer, Edition 2024 (see `rust-toolchain.toml`).
- Windows 10/11 for the currently supported host workflow.
- Android SDK platform-tools, emulator and build-tools for device/APK features.

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --workspace --release
```

A real device is not required for the unit and CLI test suite. If you change device behavior, include
the emulator image and a short validation note in the PR. The desktop can be run with
`cargo run -p androidforge-desktop`; it uses the workspace CLI build when present.

## Design rules

- Keep the Rust core generic. App names, package ids, fixture paths and project branches belong in
  `examples/` or a user's `.androidforge/` adapter, not in runtime crates.
- Every meaningful command supports `--json`, stable exit codes and structured errors.
- Progress and diagnostics go to stderr; JSON goes to stdout.
- Long-running work must be bounded by an explicit timeout and must not leak child processes.
- Destructive operations need explicit confirmation (`--yes`) and a meaningful `--dry-run` where possible.
- A feature that is not implemented must say so honestly, with alternatives.
- Evidence is part of the product: a failure should explain what happened and where to look.
- The desktop is a sidecar client. It must not reimplement Android behavior or invent a second report
  model; add CLI commands/results first, then consume them from `desktop/`.

## Branches and commits

- Branch from `main`.
- Use a short descriptive branch name: `fix/session-owner-guard`, `feat/report-list`,
  `docs/quickstart`, `chore/ci`.
- Keep commits focused. Use an imperative subject, for example:
  - `fix: clear active run pointer on explicit end`
  - `feat: include flow checks in conformance results`
  - `docs: describe session ownership`
- Reference an issue in the PR description, not in every commit.
- Rebase or squash before merging when the history is noisy.

## Pull requests

Use the pull request template. A good PR:

- Explains the user-visible problem and the chosen behavior.
- Includes tests for the fix or feature.
- Updates the relevant documentation and `CHANGELOG.md` for notable changes.
- Passes the CI checks on Windows.
- Avoids unrelated formatting or refactoring churn.

Maintainers may ask for a narrower scope. Large features should start as an issue or design
discussion before implementation.

## Code of conduct

Be direct, technical and respectful. Harassment, personal attacks and publishing another person's
private data are not acceptable. Maintainers may remove content or block accounts that violate this
rule.

## License

By contributing, you agree that your contribution is licensed under the repository's MIT license.
