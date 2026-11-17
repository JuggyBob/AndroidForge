# Windows packaging

Build and package AndroidForge from the repository root:

```powershell
cargo build --workspace --release
.\packaging\package.ps1 -SkipBuild
```

`package.ps1` produces:

- `dist/androidforge-cli-windows-x86_64.zip` — CLI only; also used by the desktop update/repair path.
- `dist/androidforge-desktop-windows-x86_64.zip` — portable desktop + CLI + LICENSE, README, USER_GUIDE,
  CHANGELOG and PORTABLE.txt.
- `dist/androidforge-windows-x86_64-setup.exe` — per-user Inno Setup installer.
- `dist/SHA256SUMS.txt` — SHA-256 for every published asset.

The installer requires Inno Setup 6 or 7 (`iscc.exe`). CI installs it with Chocolatey. `package.ps1`
searches `PATH`, `%LOCALAPPDATA%\Programs\Inno Setup 6|7`, and the usual Program Files locations.

## Installer properties

- Per-user install to `%LOCALAPPDATA%\Programs\AndroidForge`; no administrator rights required.
- Both executables are adjacent in `{app}`, preserving sidecar discovery.
- Start Menu shortcuts, optional desktop shortcut and uninstaller.
- User configuration (`%APPDATA%\AndroidForge`, `%LOCALAPPDATA%\AndroidForge`) and project artifacts are
  outside `{app}` and are preserved by upgrade and uninstall.
- Version metadata is `0.6.0` for this release.

## Signing

The release assets are **unsigned**. No code-signing certificate or trusted publisher identity is
available, so SmartScreen may warn on first launch. Do not claim a verified publisher. To sign later,
sign both executables and the installer after packaging, then publish a new checksum file; the packaging
layout does not need to change.
