; Unsigned, per-user desktop client. Never installs, starts or removes a service.
#ifndef AppVersion
  #error AppVersion must be supplied by build.ps1
#endif
#ifndef FileVersion
  #error FileVersion must be supplied by build.ps1
#endif
#ifndef BinaryDir
  #error BinaryDir must be supplied by build.ps1
#endif
#ifndef RepoDir
  #error RepoDir must be supplied by build.ps1
#endif

[Setup]
AppId={{9743971B-8E07-4828-A04B-C50B0B3D976C}
AppName=HP Charge Limit Desktop
AppVersion={#AppVersion}
VersionInfoVersion={#FileVersion}
AppPublisher=kralonur
AppPublisherURL=https://github.com/kralonur/hp-charge-limit-rs
AppSupportURL=https://github.com/kralonur/hp-charge-limit-rs/issues
DefaultDirName={localappdata}\Programs\hp-charge-limit-desktop
DefaultGroupName=HP Charge Limit
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
DisableProgramGroupPage=yes
LicenseFile={#RepoDir}\LICENSE-MIT
OutputBaseFilename=hp-charge-limit-desktop-v{#AppVersion}-x86_64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
CloseApplicationsFilter=*.exe
RestartApplications=no
UninstallDisplayName=HP Charge Limit Desktop
UninstallDisplayIcon={app}\hp-charge-limit-gui.exe

[Files]
Source: "{#BinaryDir}\hp-charge-limit-rs.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinaryDir}\hp-charge-limit-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinaryDir}\hp-charge-limit-tray.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoDir}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoDir}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\HP Charge Limit"; Filename: "{app}\hp-charge-limit-gui.exe"; WorkingDir: "{app}"
Name: "{group}\HP Charge Limit Tray"; Filename: "{app}\hp-charge-limit-tray.exe"; WorkingDir: "{app}"

[Run]
Filename: "{app}\hp-charge-limit-gui.exe"; Description: "Launch HP Charge Limit"; Flags: postinstall nowait skipifsilent

[Code]
function LegacyMarkerIsOurs(): Boolean;
var
  Marker: AnsiString;
begin
  Result := LoadStringFromFile(ExpandConstant('{app}\.hp-charge-limit-desktop'), Marker) and
    (Trim(String(Marker)) = 'hp-charge-limit-desktop');
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  RegisteredDir: String;
begin
  Result := '';
  if DirExists(ExpandConstant('{app}')) and not LegacyMarkerIsOurs() then
    if not RegQueryStringValue(HKCU,
      'Software\Microsoft\Windows\CurrentVersion\Uninstall\{9743971B-8E07-4828-A04B-C50B0B3D976C}_is1',
      'InstallLocation', RegisteredDir) or
      (CompareText(RemoveBackslashUnlessRoot(RegisteredDir),
        RemoveBackslashUnlessRoot(ExpandConstant('{app}'))) <> 0) then
      Result := 'Refusing to replace an unrecognized installation directory. Choose a new directory or remove the old installation first.';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  { Transfer ownership only after a successful installation. }
  if (CurStep = ssPostInstall) and LegacyMarkerIsOurs() then
    DeleteFile(ExpandConstant('{app}\.hp-charge-limit-desktop'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Command: String;
begin
  if CurUninstallStep = usUninstall then
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run',
      'HpChargeLimitTray', Command) then
      if CompareText(Command, '"' + ExpandConstant('{app}\hp-charge-limit-tray.exe') + '"') = 0 then
        RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'HpChargeLimitTray');
end;
