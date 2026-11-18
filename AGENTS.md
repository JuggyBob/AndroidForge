# Agent instructions

## Testing delegation (HARD REQUIREMENT)

All testing, verification and test-campaign execution MUST be delegated to subagents using exactly:

```text
opencode-go/hy4-flash-free#high
```

- Always pass `model: "opencode-go/hy4-flash-free#high"` to the `subagent` tool.
- Never use the `none` variant; `#high` is mandatory.
- Do not run test campaigns in the primary session. The primary agent may plan tests, review evidence, consolidate results and run a minimal read-only confirmation of an ambiguous result.
- Use separate subagents for independent test areas or workstreams.
- If the model is unavailable or fails, stop and report; never fall back to another model or variant silently.
- Re-read this file after any compaction or context loss before spawning a test subagent.
- Record the model used for each test workstream in the final report.

## Test safety

- Treat every physical device as production. Do not pass `--allow-physical-mutation` without explicit user approval for that exact operation.
- Use isolated `ANDROIDFORGE_HOME` values and clean up sessions/emulators started by tests.
- Never steal a device from another session owner.
