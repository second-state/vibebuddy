; The Windows installer, VibeBuddy-<label>-windows-x86_64-setup.exe, built by make-installer.sh with Inno Setup 6.
; It installs for the current user only, with no admin prompt: the app, daemon and hook, the voice packs and the
; licenses go to %LOCALAPPDATA%\Programs\VibeBuddy. Like install.sh on Linux it also connects the agents found on
; this computer (vibebuddy-hook install) and starts the app at login. Uninstalling takes the hooks out again.
;
; make-installer.sh passes /DAppVersion=<x.y.z>, /DLabel=<label> and /DStage=<folder with the files>.

#define AppName "VibeBuddy"
; The shortcut carries this id, and the app and daemon show their toasts under it once the shortcut exists.
#define AppId "SecondState.VibeBuddy"

[Setup]
AppId={{6F1C6C5E-6A3E-4E8B-9C35-2B7D9F0A41C2}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#Label}
AppPublisher=Second State
AppPublisherURL=https://github.com/second-state/vibebuddy
DefaultDirName={autopf}\{#AppName}
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputBaseFilename=VibeBuddy-{#Label}-windows-x86_64-setup
SetupIconFile=vibebuddy.ico
UninstallDisplayIcon={app}\vibebuddy.ico
WizardStyle=modern
Compression=lzma2
SolidCompression=yes

; English only: Inno Setup ships no Simplified Chinese messages file, only an unofficial one.
[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
LaunchAtLogin=Launch VibeBuddy at login
ConnectingAgents=Connecting the agents on this computer...

[Tasks]
Name: "login"; Description: "{cm:LaunchAtLogin}"

[Files]
Source: "{#Stage}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs
Source: "vibebuddy.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\vibebuddy-desktop.exe"; Parameters: "--settings"; IconFilename: "{app}\vibebuddy.ico"; AppUserModelID: "{#AppId}"

[Registry]
; The same value the app's Launch at login switch reads and writes.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "{#AppName}"; ValueData: """{app}\vibebuddy-desktop.exe"""; Tasks: login; Flags: uninsdeletevalue

[Run]
Filename: "{app}\vibebuddy-hook.exe"; Parameters: "install"; StatusMsg: "{cm:ConnectingAgents}"; Flags: runhidden
Filename: "{app}\vibebuddy-desktop.exe"; Parameters: "--settings"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\vibebuddy-hook.exe"; Parameters: "uninstall"; RunOnceId: "Hooks"; Flags: runhidden

[Code]
// The running app holds its files, and the daemon exits with it; stop both before files are replaced or removed.
procedure StopApp();
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM vibebuddy-desktop.exe /F', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM vibebuddyd.exe /F', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopApp();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  StopApp();
  Result := True;
end;
