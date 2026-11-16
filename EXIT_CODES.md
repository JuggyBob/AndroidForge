# Exit Codes and Structured Errors

Agents must branch on the process exit code and on `error.code` in JSON. Do not parse English prose.

## Success

| Code | Meaning |
| --- | --- |
| `0` | The command completed and its assertions/expectations passed. |
| `10` | Partial success: some independent steps completed, others failed. Inspect `error.context` and the run report. |

## Failure ranges

| Code | Family |
| --- | --- |
| `2` | Invalid command or argument. Usually recoverable by fixing argv. |
| `3` | Environment/tooling/capability gap: SDK, ADB, Java, missing hook, unsupported operation. |
| `4` | Device, emulator or session selection/ownership problem. |
| `5` | Timeout or boot timeout. |
| `6` | A command, install, package, fixture or permission operation failed. |
| `7` | UI selector, assertion, text verification, hook postcondition or signature verification failed. |
| `8` | Configuration, adapter, fixture, conformance or JUnit input is invalid. |
| `9` | Internal error or artifact write failure. |
| `130` | Interrupted (Ctrl+C / cancellation). |

## Complete code table

| Code | Exit | Recoverable | Meaning |
| --- | ---: | --- | --- |
| `E_INVALID_COMMAND` | 2 | yes | Unknown or malformed command. Suggestions may name the intended command. |
| `E_INVALID_ARGUMENT` | 2 | yes | Argument combination or value is invalid. |
| `E_DEVICE_NOT_FOUND` | 4 | yes | No device/emulator matches the target. |
| `E_DEVICE_AMBIGUOUS` | 4 | yes | More than one device is connected; pass `--serial` or `--session`. |
| `E_DEVICE_OFFLINE` | 4 | yes | ADB reports the device offline. |
| `E_DEVICE_UNAUTHORIZED` | 4 | yes | USB debugging authorization is missing. |
| `E_DEVICE_BUSY` | 4 | yes | Another AndroidForge session owns the device. |
| `E_DEVICE_DISCONNECTED` | 4 | yes | The device disappeared during an operation. |
| `E_EMULATOR_NOT_FOUND` | 4 | yes | The AVD or emulator process was not found. |
| `E_EMULATOR_BOOT_TIMEOUT` | 5 | yes | The emulator did not finish booting in time. |
| `E_SDK_NOT_FOUND` | 3 | yes | The Android SDK root could not be located. |
| `E_ADB_NOT_FOUND` | 3 | yes | `adb` could not be located. |
| `E_TOOL_NOT_FOUND` | 3 | yes | A required build tool or executable is missing. |
| `E_MAESTRO_NOT_FOUND` | 3 | yes | Optional Maestro CLI was requested but is unavailable. |
| `E_COMMAND_TIMEOUT` | 5 | yes | A bounded external command timed out. |
| `E_COMMAND_FAILED` | 6 | yes | An external command exited unsuccessfully. |
| `E_PACKAGE_NOT_FOUND` | 6 | yes | The requested package is not installed. |
| `E_FILE_NOT_FOUND` | 6 | yes | A local or device file is missing. |
| `E_APP_NOT_DEBUGGABLE` | 6 | yes | `run-as` file access requires a debuggable build. |
| `E_INSTALL_FAILED` | 6 | yes | `adb install` failed; context includes the installer output. |
| `E_INSTALL_SIGNATURE_MISMATCH` | 6 | yes | Installed and requested APK certificates differ. AndroidForge never auto-uninstalls. |
| `E_INSTALL_VERIFICATION_FAILED` | 6 | yes | The installed package/hash/version did not match the requested APK. |
| `E_APK_INVALID` | 6 | no | The file is not a readable APK. |
| `E_SIGNATURE_VERIFICATION_FAILED` | 7 | no | APK signature verification failed. |
| `E_SELECTOR_NOT_FOUND` | 7 | yes | No UI node matched the selector, or `ui clear` had no focused editable field. |
| `E_UI_HIERARCHY_INVALID` | 6 | yes | UIAutomator returned an unreadable hierarchy. |
| `E_SELECTOR_AMBIGUOUS` | 7 | yes | Multiple nodes matched; inspect candidates and add `--index` or a narrower selector. |
| `E_POSTCONDITION_TIMEOUT` | 7 | yes | The action executed but its requested postcondition was not satisfied before the timeout. The action is not retried. Context includes current foreground state and an observation token. |
| `E_UI_ASSERTION_FAILED` | 7 | yes | The requested UI condition did not hold. |
| `E_TEXT_VERIFICATION_FAILED` | 7 | yes | Entered text did not read back as requested. |
| `E_IME_VISIBLE` | 7 | yes | Back was refused while the soft keyboard was up; pass `--hide-keyboard`. |
| `E_PERMISSION_DENIED` | 6 | yes | Permission grant/revoke did not take effect or device permissions blocked the command. |
| `E_SNAPSHOT_NOT_FOUND` | 4 | yes | The emulator snapshot does not exist. |
| `E_SESSION_EXPIRED` | 4 | yes | Session metadata is unreadable, stale, or the device is gone. |
| `E_SESSION_OWNERSHIP_MISMATCH` | 4 | yes | The session is not owned by the caller/owner, including destructive `session stop|release|restart`. |
| `E_SESSION_NOT_FOUND` | 4 | yes | The session id does not exist. |
| `E_OBSERVATION_NOT_FOUND` | 4 | yes | The observation token does not exist for this session or was pruned by retention. |
| `E_OBSERVATION_EXPIRED` | 4 | yes | The observation is older than 24 hours or the device rebooted. Capture a fresh observation. |
| `E_OBSERVATION_SESSION_MISMATCH` | 4 | yes | The token belongs to another session. Use a token from the target session. |
| `E_LEASE_EXPIRED` | 4 | yes | A session lease is expired and requires reconciliation or renewal. Expiry never stops a device by itself. |
| `E_ARTIFACT_WRITE_FAILED` | 9 | yes | Evidence could not be written. Check disk/path permissions. |
| `E_CONFIG_INVALID` | 8 | no | Global or merged configuration is invalid. |
| `E_ADAPTER_INVALID` | 8 | no | Project adapter is invalid or uses an unsupported `adapter_version`. |
| `E_FIXTURE_INVALID` | 8 | no | Fixture manifest, template or action is invalid. |
| `E_FIXTURE_FAILED` | 6 | yes | A fixture step failed or destructive steps were not confirmed. |
| `E_HOOK_NOT_FOUND` | 3 | yes | The named hook is not defined or is incomplete. |
| `E_HOOK_VERIFICATION_FAILED` | 7 | yes | The hook ran but its declared app-state postcondition was not met. |
| `E_JUNIT_INVALID` | 8 | no | JUnit XML could not be parsed. |
| `E_UNSUPPORTED_OPERATION` | 3 | yes | The operation is deliberately not implemented. Read the reason and alternatives. |
| `E_PARTIAL_FAILURE` | 10 | yes | Independent work partially completed; inspect the report. |
| `E_INTERRUPTED` | 130 | yes | The operation was interrupted. |
| `E_INTERNAL` | 9 | no | An unexpected internal invariant failed. Include the command and trace in a report. |

## JSON error shape

```json
{
  "schema_version": 1,
  "status": "error",
  "command": "ui.assert",
  "exit_code": 7,
  "session_id": "ses-...",
  "device": {"serial": "emulator-5560"},
  "error": {
    "code": "E_UI_ASSERTION_FAILED",
    "message": "assertion failed",
    "recoverable": true,
    "suggestions": ["Run `androidforge ui dump` and inspect the current hierarchy."],
    "context": {"selector": {"text": "Saved"}},
    "command": "ui.assert",
    "device_serial": "emulator-5560",
    "session_id": "ses-...",
    "artifacts": [
      {"kind": "screenshot", "path": "screenshots/....png", "sha256": "..."}
    ]
  },
  "duration_ms": 412,
  "artifacts": []
}
```

`E_UNSUPPORTED_OPERATION` is a normal contract, not a crash. For example, AVD cloning reports why it is absent and which supported workflow to use instead.
