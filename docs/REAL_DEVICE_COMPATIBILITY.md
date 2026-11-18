# Real-device compatibility

AndroidForge distinguishes emulators from physical devices. Physical-device data
is not disposable, so data-changing commands are blocked by default.

## Safety policy

Data-changing commands return `E_PHYSICAL_DEVICE_MUTATION_BLOCKED` on a physical
device unless `--allow-physical-mutation` is passed. This covers app install,
uninstall, clear-data, file-push, data-restore, permission/AppOps changes,
Doze/standby/battery/network/time changes, notification clearing and job triggers.
`--dry-run` works without the flag and reports `requires_physical_mutation`.

The guard is a safety default, not authentication. A deliberate test can still
opt in.

## Harness

```powershell
.\tests\real-device.ps1 -Serial <SERIAL> -SoakCycles 3
```

The harness:

1. verifies ADB authorization and keyguard state;
2. records manufacturer, model, Android/SDK, build fingerprint and HyperOS/MIUI
   properties;
3. attaches a temporary session;
4. runs context, observe, dump, screenshot and diagnostics logcat/collect;
5. launches Settings, swipes, goes Back and Home;
6. runs safe launch/observe/screenshot soak cycles;
7. releases the session and confirms the device is still online and unchanged.

It never factory-resets, clears data, uninstalls a pre-existing app, restores data,
loads a snapshot, or changes security/network/time settings.

## Optional test APK

```powershell
.\tests\real-device.ps1 -Serial <SERIAL> `
  -AllowTestApk `
  -TestApk .\app-debug.apk `
  -TestPackage com.example.debug
```

The harness refuses to continue if that package is already installed. If the
vendor blocks installation, it records `BLOCKED`; it does not change "Install via
USB" or "USB debugging (Security settings)".

## Xiaomi / HyperOS findings (v0.9.3)

- Model `22041216G`, codename `xaga`, Android 14 / SDK 34, HyperOS OS2.0.
- Read-only observation, screenshots (1080x2460), UI swipe/back/home, diagnostics
  and performance passed.
- Debug APK installation was blocked with
  `INSTALL_FAILED_USER_RESTRICTED: Install canceled by user`. This is a vendor
  policy result, not a product pass. Enabling the vendor's USB install permission
  is a user decision.
- No destructive command was run. The device was left online and unchanged.
