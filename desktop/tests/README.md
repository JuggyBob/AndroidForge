# Desktop smoke harness

`smoke.ps1` launches the native desktop on every page, captures a screenshot,
verifies the page rendered, records the Windows UI Automation tree, and invokes
the visible Refresh button on the overview page.

It is a local release gate, not a headless CI unit test. Windows GUI sessions are
required for the screenshots and UI Automation. The harness uses an isolated
`ANDROIDFORGE_HOME` and only starts/stops the desktop processes it created.

```powershell
cargo build --workspace --release
.\desktop\tests\smoke.ps1 -EvidenceDir .\target\desktop-smoke
```

To render the list-heavy pages with the current local state, use
`-UseExistingHome`. That mode is read-only; the harness still invokes only
Refresh on overview. The default mode uses an isolated home so a clean machine
can run the gate.

The result is `desktop-smoke.json` in the evidence directory plus one PNG per
page. A non-zero exit means a page failed to launch or render.

## What it checks

- CLI and desktop report the same version.
- All 15 pages launch: setup, tools, overview, context, devices, sessions, matrix,
  ownership, runs, triage, compare, logcat, inspector, maintenance, settings.
- Each page has a non-blank screenshot and a real window title.
- The UI Automation tree is available and contains controls.
- The Refresh button on overview can be invoked without exiting the process.

## What it does not check

- Every interactive control or destructive confirmation.
- Pixel-perfect visual regressions.
- Mobile/DPI variants beyond the current desktop window.

Those remain manual review items for the final UI pass. The harness is the
repeatable baseline that prevents a page from silently failing to render.
