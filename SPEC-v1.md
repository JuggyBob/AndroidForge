# SPEC v1 — Second-App Proving Ground

v1 is not implemented in this delivery as a separate phase. It is the next integration gate.

## Goal

Run a second Android application against the same `androidforge.exe`, changing only:

- `.androidforge/project.toml`
- fixtures and fixture scripts
- conformance flow
- app debug hooks
- optional Maestro/instrumentation assets

No app name, package ID, AVD name, branch or fixture path may be added to the Rust core.

## Candidate second-app additions

- Cross-application file picker / SAF navigation primitives that are generic across apps.
- More specialised package/version/diagnostic parsing if the second app exposes gaps.
- Additional physical-device connection handling if the second app's hardware reveals it.
- Richer task/activity evidence where `dumpsys` formats differ.
- More notification/job/alarm parsing, still labelled best-effort with raw evidence.

## Acceptance

A second conformance flow passes with the same binary and no core changes. If the core must change, the
new abstraction must be justified by both apps, not just one.
