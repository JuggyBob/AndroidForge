# AndroidForge v0.11.0 screenshots

These are real captures of the compiled v0.11.0 native desktop cockpit. They were taken from the
Windows release build against an isolated AndroidForge home with the IronPulse project selected and
16 real screenshot files in total. No mockups or browser-rendered UI are included.

> The v0.12.0 release does not change the desktop UI, so this gallery remains representative. The
> v0.12.0 work is CLI/verification-focused; see `docs/release-notes/v0.12.0.md`.

The gallery covers the full page set, the new Setup final page, recursive Inspector cards and light
mode.

| File | Page |
| --- | --- |
| `01-setup-welcome.png` | Setup wizard, Welcome step |
| `02-setup-agent-handoff.png` | Setup final step with the copyable agent prompt |
| `03-overview.png` | Overview, health and recent runs |
| `04-context.png` | Context snapshot |
| `05-devices.png` | Connected devices |
| `06-sessions.png` | Sessions and ownership |
| `07-ownership.png` | Ownership, leases and safe alternatives |
| `08-runs.png` | Run report table |
| `09-triage.png` | Failure triage and explanation |
| `10-compare.png` | Compatible run comparison |
| `11-matrix.png` | Matrix form |
| `12-logcat.png` | Logcat controls and streaming state |
| `13-inspector.png` | Live device preview and recursive semantic hierarchy cards |
| `14-maintenance.png` | Managed paths, cleanup preview and sessions/runs |
| `15-settings.png` | Connection, appearance, fonts, updates and about |
| `16-overview-light.png` | Light theme overview |

## Notes

- The desktop is Windows-focused; all captures are from Windows at a high-DPI scale.
- Screenshots contain local project paths and emulator labels. Review or redact them before reusing
  in a different environment.
- The binaries are intentionally unsigned and Windows SmartScreen may warn on first launch.
- The full release assets and SHA-256 checksums are listed in the
  [v0.11.0 release notes](../../release-notes/v0.11.0.md).
