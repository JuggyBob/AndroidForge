# SPEC v2 — Third-App and Hard Primitives

v2 is deferred. These areas must be implemented honestly, with runtime capability checks, and must not
be stubbed.

## Scope candidates

- Physical-device claim/release, reconnect, vendor restrictions, MIUI/Doze edge cases.
- SAF pickers, installer dialogs, permission dialogs, notification shade interactions.
- Notification channel/tag/text assertions across Android versions.
- JobScheduler and AlarmManager structured state, with best-effort labels.
- Doze, standby buckets, battery, network toggles and restoration guarantees.
- Time control on rooted/userdebug devices, with mandatory prior-state recording and restore.
- Maestro flows, instrumentation tests and JUnit evidence unification.
- Snapshot/clone isolation only after proving independent writable data on the installed emulator.
- Concurrency hardening across many simultaneous agents.

## Explicit non-goals remain

No GUI, daemon, cloud, telemetry, signing automation, release publishing, Gradle replacement, or
pretence that an unsupported API is reliable.
