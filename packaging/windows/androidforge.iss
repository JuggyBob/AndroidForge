; AndroidForge per-user Windows installer.
; Build with: iscc packaging\windows\androidforge.iss
; The binaries are intentionally unsigned. Code signing is a documented future step.

#define MyAppName "AndroidForge"
#define MyAppVersion "0.11.0"
#define MyAppPublisher "AndroidForge"
#define MyAppURL "https://github.com/JuggyBob/AndroidForge"
#define MyAppExeName "androidforge-desktop.exe"
#define MyAppCliName "androidforge.exe"

[Setup]
AppId={{6F2B8F94-6F6D-4F1B-9F7C-6A0B6E4C2D31}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={localappdata}\Programs\AndroidForge
DefaultGroupName=AndroidForge
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\..\dist
OutputBaseFilename=androidforge-windows-x86_64-setup
SetupIconFile=..\..\desktop\assets\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no
VersionInfoVersion=0.11.0.0
VersionInfoProductVersion=0.11.0
VersionInfoTextVersion=0.11.0
VersionInfoCompany=AndroidForge
VersionInfoDescription=AndroidForge developer cockpit
VersionInfoCopyright=AndroidForge contributors

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "..\..\target\release\androidforge-desktop.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\androidforge.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\USER_GUIDE.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\CHANGELOG.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\AndroidForge"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\AndroidForge CLI"; Filename: "{app}\{#MyAppCliName}"
Name: "{group}\Uninstall AndroidForge"; Filename: "{uninstallexe}"
Name: "{autodesktop}\AndroidForge"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch AndroidForge"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Only installer-owned files are removed. User configuration, AndroidForge state and
; run artifacts live outside {app} and are deliberately preserved on uninstall/upgrade.
Type: files; Name: "{app}\*.bak-*"

