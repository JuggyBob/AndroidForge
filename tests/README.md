# Repeatable test harnesses

These scripts are local release gates. They are not unit tests and are not run by
`cargo test`; they exercise a real emulator or physical device and write evidence
to `target/` by default.

| Script | Purpose | Device |
| --- | --- | --- |
| `long-horizon.ps1` | Sustained force-stop/launch/observe/screenshot/swipe workload, health samples, performance, recording, reports and bundles. | Disposable headless AVD |
| `real-device.ps1` | Read-only/safe compatibility pass on a physical device, including optional test-APK install and soak cycles. | Physical device |
| `upgrade-rollback.ps1` | Installs an older release, creates state, upgrades in place, verifies preservation, uninstalls and checks state survives. | Installer only |
| `fault-injection.ps1` | Stale locks, corrupt session/report JSON, invalid adapter fields and concurrent run writers. | None |
| `supply-chain.ps1` | License inventory, duplicate versions, wildcard scan and optional audit/deny checks. | None |

Examples:

```powershell
cargo build --workspace --release

.\tests\long-horizon.ps1 -Avd af_cert_clean -MaxCycles 25 -DurationMinutes 20
.\tests\real-device.ps1 -Serial <SERIAL> -SoakCycles 3
.\tests\upgrade-rollback.ps1 -FromVersion 0.9.3 -ToInstaller .\dist\androidforge-windows-x86_64-setup.exe -ToVersion 0.9.4
.\tests\fault-injection.ps1
.\tests\supply-chain.ps1
```

Safety:

- `real-device.ps1` refuses emulator serials and never factory-resets, clears app
  data, uninstalls a pre-existing app, restores data, loads a snapshot, or changes
  security/network/time settings. It installs a test APK only with
  `-AllowTestApk -TestApk <path> -TestPackage <package>` and only if that package
  is not already installed.
- `long-horizon.ps1` starts and stops only the session it created. It does not
  shut down the host.
- Evidence defaults to `target\long-horizon`, `target\real-device`,
  `target\upgrade-rollback`, `target\fault-injection` and `target\supply-chain`.
