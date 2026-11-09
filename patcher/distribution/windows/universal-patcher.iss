; Universal Patcher setup for Windows: a per-user web installer.
; It produces the same installation as  irm <origin>install.ps1 | iex  by running the same
; engine command. The engine verifies the signed application release and installs it, and it
; registers the Apps & features entry, so this setup registers no uninstaller of its own.
;
; Compile:  iscc /DAppVersion=1.2.3 /DOrigin=https://downloads.example.org/ /DEngineExe=path\to\patcher-engine.exe universal-patcher.iss

#ifndef AppVersion
  #error Pass /DAppVersion=MAJOR.MINOR.PATCH
#endif
#ifndef Origin
  #error Pass /DOrigin=https://host/ (the published origin, ending in /)
#endif
#ifndef EngineExe
  #error Pass /DEngineExe=path to the Windows patcher-engine.exe of this version
#endif

[Setup]
AppId={{D7129157-40D0-421D-8F03-72B4512FA63A}
AppName=Universal Patcher
AppVersion={#AppVersion}
AppVerName=Universal Patcher {#AppVersion}
PrivilegesRequired=lowest
DefaultDirName={localappdata}\UniversalPatcher
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
CreateUninstallRegKey=no
Uninstallable=no
CreateAppDir=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=output
OutputBaseFilename=UniversalPatcherSetup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Files]
Source: "{#EngineExe}"; DestDir: "{tmp}"; DestName: "patcher-engine.exe"; Flags: deleteafterinstall

[Run]
Filename: "{localappdata}\UniversalPatcher\bin\universal-patcher.exe"; Parameters: "launch"; Description: "Start Universal Patcher"; Flags: postinstall nowait skipifsilent; Check: Installed

[Code]
var
  InstallOk: Boolean;

function Installed: Boolean;
begin
  Result := InstallOk;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if CurStep = ssPostInstall then
  begin
    WizardForm.StatusLabel.Caption := 'Downloading and verifying Universal Patcher...';
    InstallOk := Exec(ExpandConstant('{tmp}\patcher-engine.exe'), 'install --origin "{#Origin}"', '',
      SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
    if not InstallOk then
      MsgBox('Universal Patcher could not be installed (code ' + IntToStr(ResultCode) +
        '). Nothing was changed. Check your internet connection and run setup again.',
        mbError, MB_OK);
  end;
end;
