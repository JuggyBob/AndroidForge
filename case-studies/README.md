# Case Studies

These are summaries of real runs from the delivery emulator (Android 16 / API 36), not generated
samples. The repository intentionally does **not** publish raw run directories: reports, logcat,
dumpsys and screenshots can contain machine paths, device identifiers and app data. Each case-study
README records the run ids, build identities, checks and limitations.

| Case | What it proves |
| --- | --- |
| [CalcSuite-smoke](CalcSuite-smoke/README.md) | Current 3.0.3 release identity, Settings updater assertion (`You have the newest version (3.0.3).`), fixture, external conformance and checksummed report. |
| [DiscountHunter-smoke](DiscountHunter-smoke/README.md) | Debug package, first-run UI, notification permission, app-private watch-state seed, `run-as` background-check hook with a verified app-state postcondition, notification/channel evidence and diagnostics. |
| [IronPulse-smoke](IronPulse-smoke/README.md) | Release first-run onboarding, build identity in the report, process/task liveness, widget action intents, Settings, screenshot, screen recording, debug file round-trip and non-destructive signature mismatch. |

The complete verification record, including the final binary hash and limitations, is in
[VERIFICATION.md](../VERIFICATION.md).
