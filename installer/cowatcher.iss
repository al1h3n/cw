; Co-watcher installer (Inno Setup 6). One installer, role chosen at first run (ADR D8):
;   - Teacher console  -> installs the GUI + a Start Menu shortcut.
;   - Student agent     -> installs the background Windows *service* (auto-start). A service does not
;                          appear in Task Manager's Startup tab, which is exactly what the brief asks.
;
; Build locally:  iscc installer\cowatcher.iss   (after building the release binaries to target\release)
; The CI workflows do this automatically on a tag.

#ifndef Bin
  ; Where the built .exe files are, relative to THIS .iss file. CI leaves them in target\release.
  #define Bin "..\target\release"
#endif
#define AppName "Co-watcher"
#define AppVersion "0.1.0"
#define AgentService "CowatcherAgent"

[Setup]
AppId={{7B2C9E14-3F5A-4D6B-9C21-A1B2C3D4E5F6}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Co-watcher
DefaultDirName={autopf}\Co-watcher
DefaultGroupName=Co-watcher
DisableProgramGroupPage=yes
; The student role installs a service, which needs elevation.
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
Compression=lzma2
SolidCompression=yes
OutputDir=Output
OutputBaseFilename=cowatcher-setup
WizardStyle=modern
UninstallDisplayName={#AppName}

[Files]
Source: "{#Bin}\cowatcher-console.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\cowatcher-agent.exe";   DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\cowatcher-viewer.exe";  DestDir: "{app}"; Flags: ignoreversion
; Optional: drop ffmpeg.exe next to the agent for real-codec recording (skipped if not present).
Source: "..\dist\ffmpeg.exe";           DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "..\dist\README.txt";           DestDir: "{app}"; Flags: ignoreversion isreadme skipifsourcedoesntexist
Source: "..\dist\DEPENDENCIES.txt";     DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
; Teacher gets a Console shortcut. The student agent is a service, so it gets no shortcut on purpose.
Name: "{group}\Co-watcher Console"; Filename: "{app}\cowatcher-console.exe"; Check: IsTeacher
Name: "{group}\Uninstall Co-watcher"; Filename: "{uninstallexe}"

[Run]
; Student: register the auto-start service (hidden from the Startup tab) and start it now.
Filename: "{app}\cowatcher-agent.exe"; Parameters: "install"; StatusMsg: "Installing the Co-watcher background service..."; Flags: runhidden waituntilterminated; Check: IsStudent
Filename: "{sys}\sc.exe"; Parameters: "start {#AgentService}"; Flags: runhidden; Check: IsStudent
; Teacher: offer to launch the console at the end.
Filename: "{app}\cowatcher-console.exe"; Description: "Launch Co-watcher Console"; Flags: postinstall nowait skipifsilent; Check: IsTeacher

[UninstallRun]
; Always try to remove the service on uninstall; harmless (and hidden) if it was never installed.
Filename: "{app}\cowatcher-agent.exe"; Parameters: "uninstall"; Flags: runhidden; RunOnceId: "RemoveCowatcherService"

[Code]
var
  RolePage: TInputOptionWizardPage;

procedure InitializeWizard;
begin
  RolePage := CreateInputOptionPage(wpSelectDir,
    'Install role', 'What is this computer?',
    'Choose how Co-watcher runs here. Install one role per computer.',
    True, False);
  RolePage.Add('Teacher console — watch and control the class (a normal app).');
  RolePage.Add('Student agent — runs in the background as a service (no Startup entry).');
  RolePage.SelectedValueIndex := 0;
end;

function IsTeacher: Boolean;
begin
  Result := RolePage.SelectedValueIndex = 0;
end;

function IsStudent: Boolean;
begin
  Result := RolePage.SelectedValueIndex = 1;
end;
