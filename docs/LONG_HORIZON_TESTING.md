# Long-horizon testing

`tests/long-horizon.ps1` is the repeatable version of the v0.9.3 independent
long-horizon run. It starts a disposable headless AVD session and repeats a
bounded workload while sampling session, device and observation-store health.

## What it exercises

- session start, ownership, heartbeat and project binding;
- APK inspect, install and `--only-if-changed`;
- app launch, force-stop, observe, screenshot, swipe and home;
- SystemUI overlay dismissal;
- diagnostics performance every five cycles;
- a 15-second screen recording;
- report list and a checksum-verified evidence bundle;
- final session stop and cleanup.

## Usage

```powershell
cargo build --workspace --release
.\tests\long-horizon.ps1 `
  -Avd af_cert_clean `
  -Project A:\Projects\Active\CalcSuite\Internal `
  -Apk A:\Projects\Active\CalcSuite\Internal\app\build\outputs\apk\debug\app-debug.apk `
  -MaxCycles 500 `
  -DurationMinutes 120
```

The script stops at the first of `-MaxCycles` or `-DurationMinutes`. A cap reached
before the wall-clock target is reported as `N/A`, not as a pass. Use an isolated
`ANDROIDFORGE_HOME` (the script creates one under the evidence directory).

Results:

- `long-horizon-summary.json` with pass/fail/N/A counts and metrics;
- `TIMELINE.txt`;
- one transcript per command;
- screenshots, recording and `evidence.zip` under the evidence directory.

The script never targets a physical device and never shuts down the host.
