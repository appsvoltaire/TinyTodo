; TinyTodo installer (Inno Setup 6). Build with installer\build.ps1.
; Per-user install, no admin rights. The Slack tokens entered here are
; checked with Slack and stored in Windows Credential Manager by
; `tinytodo.exe --save-tokens`; they are passed through the environment,
; never on a command line or in a file.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{8F1C5B2E-4C1D-4E7A-9C2B-6A1D3E5F7B90}
AppName=TinyTodo
AppVersion={#AppVersion}
AppVerName=TinyTodo {#AppVersion}
DefaultDirName={localappdata}\Programs\TinyTodo
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\installer
OutputBaseFilename=TinyTodo-Setup-{#AppVersion}
SetupIconFile=tinytodo.ico
UninstallDisplayIcon={app}\tinytodo.ico
UninstallDisplayName=TinyTodo
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes

[Files]
Source: "..\target\release\tinytodo.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "tinytodo.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\TinyTodo"; Filename: "{app}\tinytodo.exe"; IconFilename: "{app}\tinytodo.ico"

[Tasks]
Name: "startup"; Description: "Start TinyTodo when I sign in"

[Registry]
; The same value the app's Settings > "Launch TinyTodo when I sign in" toggles.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "TinyTodo"; ValueData: """{app}\tinytodo.exe"""; Tasks: startup

[Run]
Filename: "{app}\tinytodo.exe"; Description: "Start TinyTodo now"; Flags: postinstall nowait skipifsilent

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

var
  TokenPage: TInputQueryWizardPage;

function SetEnvironmentVariable(lpName: String; lpValue: String): BOOL;
  external 'SetEnvironmentVariableW@kernel32.dll stdcall';

procedure SetTokenEnv(const Bot, App: String);
begin
  SetEnvironmentVariable('SLACK_BOT_TOKEN', Bot);
  SetEnvironmentVariable('SLACK_APP_TOKEN', App);
end;

function BotToken: String;
begin
  Result := Trim(TokenPage.Values[0]);
end;

function AppToken: String;
begin
  Result := Trim(TokenPage.Values[1]);
end;

procedure StopRunningApp;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM tinytodo.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

procedure InitializeWizard;
begin
  TokenPage := CreateInputQueryPage(wpSelectTasks,
    'Connect to Slack', 'Enter the tokens of your TinyTodo Slack app.',
    'Find them at api.slack.com/apps > TinyTodo: the Bot User OAuth Token (OAuth && Permissions) ' +
    'and an App-Level Token with connections:write (Basic Information).' + #13#10#13#10 +
    'They are stored in Windows Credential Manager. Leave both empty to keep the tokens ' +
    'already saved on this PC.');
  TokenPage.Add('&Bot token (xoxb-...):', True);
  TokenPage.Add('&App-level token (xapp-...):', True);
end;

function JoinLines(const Lines: TArrayOfString): String;
var
  I: Integer;
begin
  Result := '';
  for I := 0 to GetArrayLength(Lines) - 1 do
    Result := Result + Lines[I] + #13#10;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
var
  ResultCode: Integer;
  Output: TExecOutput;
  Ran: Boolean;
begin
  Result := True;
  if CurPageID <> TokenPage.ID then
    Exit;
  if (BotToken = '') and (AppToken = '') then
    Exit;
  if (Pos('xoxb-', BotToken) <> 1) or (Pos('xapp-', AppToken) <> 1) then
  begin
    MsgBox('The bot token starts with "xoxb-" and the app-level token with "xapp-". ' +
      'Enter both, or leave both empty.', mbError, MB_OK);
    Result := False;
    Exit;
  end;

  // Ask Slack whether the tokens work before installing anything.
  ExtractTemporaryFile('tinytodo.exe');
  SetTokenEnv(BotToken, AppToken);
  try
    Ran := ExecAndCaptureOutput(ExpandConstant('{tmp}\tinytodo.exe'), '--check-tokens', '',
      SW_HIDE, ewWaitUntilTerminated, ResultCode, Output);
  finally
    SetTokenEnv('', '');
  end;
  if not Ran or (ResultCode <> 0) then
    Result := MsgBox('Slack did not accept these tokens:' + #13#10#13#10 +
      JoinLines(Output.StdErr) + #13#10 + 'Use them anyway?', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopRunningApp;
  Result := '';
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if (CurStep <> ssPostInstall) or (BotToken = '') then
    Exit;
  SetTokenEnv(BotToken, AppToken);
  try
    if not Exec(ExpandConstant('{app}\tinytodo.exe'), '--save-tokens', '', SW_HIDE,
      ewWaitUntilTerminated, ResultCode) or (ResultCode <> 0) then
      MsgBox('Could not save the Slack tokens to Windows Credential Manager. ' +
        'Run Setup again to retry.', mbError, MB_OK);
  finally
    SetTokenEnv('', '');
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ResultCode: Integer;
begin
  if CurUninstallStep <> usUninstall then
    Exit;
  StopRunningApp;
  if SuppressibleMsgBox('Also remove your Slack tokens from Windows Credential Manager?',
    mbConfirmation, MB_YESNO, IDYES) = IDYES then
    Exec(ExpandConstant('{app}\tinytodo.exe'), '--forget-tokens', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  RegDeleteValue(HKEY_CURRENT_USER, RunKey, 'TinyTodo');
end;
