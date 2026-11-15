# Acceptance Flows

These are the flows used to judge whether AndroidForge is useful, not merely compilable.

## CalcSuite — first dogfood

Calculator Suite is the first conformance app because its UI is stable, it needs no seeded state, and it
exercises reliable install, UI interaction, the app's own updater and evidence collection.

```powershell
androidforge doctor --json
androidforge session start --avd androidforge_test --headless --owner CalcSuiteAgent --json
androidforge app install .\CalculatorSuite-v3.0.3.apk --session <ID> --json
androidforge ui dump --session <ID> --json
androidforge ui tap --session <ID> --content-desc "Settings" --json
androidforge ui scroll-to --session <ID> --text "Check for updates" --json
androidforge ui tap --session <ID> --text "Check" --json
androidforge ui wait --session <ID> --contains "You have the newest version" --timeout 30 --json
androidforge ui assert --session <ID> --contains "You have the newest version (3.0.3)." --json
androidforge capture screenshot --session <ID> --json
androidforge diagnostics collect --session <ID> --json
androidforge run --session <ID> --name update-flow -- python my_existing_script.py
androidforge report show <RUN> --json
androidforge session release --session <ID> --owner CalcSuiteAgent --json
```

Done means: one JSON report and one evidence directory, build identity in the report, failure bundle on
failure, `report import junit` merging Gradle XML, and existing Python scripts running unmodified under
`run --`. The shipped example project is in `examples/CalcSuite/`; the real verification run is in
`case-studies/CalcSuite-smoke/` and `VERIFICATION.md`.

## DiscountHunter — the genericity stress test

This flow intentionally uses different state and failure modes:

1. Doctor/capabilities.
2. Headless emulator on an explicit port, or attach to an existing one; claim a session; wake/unlock.
3. Install debug/release APK; verify package, versionCode/versionName, SHA-256 and certificate.
4. Clear data; grant/revoke `POST_NOTIFICATIONS`; inspect AppOps; launch; assert foreground/resumed.
5. Seed a debuggable app-private watch-state file via `app file-push` (run-as).
6. Trigger a background check through a named app hook; do not guess a JobScheduler ID.
7. Exercise semantic UI, text entry, scroll and screenshots.
8. Wait/assert notifications, channels and permission state.
9. Inspect jobs, alarms, Doze, standby, battery, network and raw dumpsys.
10. Reproduce the blocked-notification/watch-state case.
11. Capture automatic failure evidence.
12. Import JUnit and finalize one run report.
13. Release the session.

The actual package and AVD are in `examples/DiscountHunter/project.toml`. Physical-device, MIUI and
vendor-specific behavior remain phase-3 work and must be verified on the target hardware.

## IronPulse — third-app edges

- Existing AVD attachment on a custom port; warm reuse.
- Headless session.
- Database/file fixtures and preserved state.
- Compose-accessibility interaction plus a documented fallback for internal test tags.
- System dialogs/SAF.
- Permissions and logs.
- Process versus task/activity liveness.
- Screen recording for transient animation evidence.
- Reliable cleanup.

See `examples/IronPulse/`.

## Shared adoption gate

The same `androidforge.exe` must run a real flow for each app using only its adapter, fixtures, hooks
and conformance script. No app-specific code in the Rust core. The repo's `genericity` test enforces
the grep-able rule.
