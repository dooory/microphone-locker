; Inno Setup script for Microphone Locker (per-user Windows installer)
; Compile via: scripts\build-installer.ps1
; Or directly: iscc /DAppVersion=0.1.0 installer\microphone-locker.iss

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

#ifndef SourceExe
  #define SourceExe "..\target\release\microphone-locker.exe"
#endif

#ifndef OutputDir
  #define OutputDir "..\target\installer"
#endif

#ifndef OutputBaseFilename
  #define OutputBaseFilename "MicrophoneLocker-Setup"
#endif

#define AppName "Microphone Locker"
#define AppExeName "microphone-locker.exe"
#define AppPublisher "dooory"
#define AppURL "https://github.com/dooory/microphone-locker"

[Setup]
; Stable GUID so upgrades replace the existing installation in place.
AppId={{8E4F3B7A-9C2D-4F1E-B6A5-3D7C8E2F1A90}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases
; Per-user install to %LOCALAPPDATA%\Programs\Microphone Locker (no UAC prompt).
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseFilename}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#AppExeName}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "Start {#AppName} with Windows"; GroupDescription: "Startup:"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Registry]
; Matches WindowsStartupRegistry format in src/windows_startup.rs: quoted exe path in HKCU\...\Run.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "MicrophoneLocker"; ValueData: """{app}\{#AppExeName}"""; Flags: uninsdeletevalue; Tasks: autostart
; Always remove the startup entry on uninstall, even if enabled later from inside the app.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "MicrophoneLocker"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(AppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  AppMutexName = 'Local\MicrophoneLocker.SingleInstance.Mutex';

procedure StopRunningApp();
var
  ResultCode: Integer;
begin
  if CheckForMutexes(AppMutexName) then
  begin
    Exec(
      ExpandConstant('{sys}\taskkill.exe'),
      '/F /IM {#AppExeName}',
      '',
      SW_HIDE,
      ewWaitUntilTerminated,
      ResultCode
    );
    Sleep(300);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopRunningApp();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    StopRunningApp();
  end;
end;
