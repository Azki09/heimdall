; The Windows installer (DESIGN §17.6), built with Inno Setup 6.7 by
; release.yml:
;
;   ISCC.exe /DAppVersion=1.4.0 /DBinDir=<folder with heimdall.exe and
;            heimdall.com> /DOutDir=<output folder> packaging\windows\heimdall.iss
;
; Per user, no administrator rights: %LOCALAPPDATA%\Programs\Heimdall.
; Never touches vaults or settings, never closes Heimdall by itself, and
; never goes online (the only link opens only when clicked).

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef BinDir
  #define BinDir "..\..\target\release"
#endif
#ifndef OutDir
  #define OutDir "..\..\dist"
#endif
; Never change it: Windows knows an installed Heimdall by it.
#define AppGuid "{12062550-2C5B-4564-BD3D-CDBCD10F20F3}"
#define RepoUrl "https://github.com/Azki09/heimdall"

[Setup]
AppId={{#AppGuid}
AppName=Heimdall
AppVersion={#AppVersion}
AppVerName=Heimdall {#AppVersion}
AppPublisher=Azki09
AppPublisherURL={#RepoUrl}
AppSupportURL={#RepoUrl}/issues
AppUpdatesURL={#RepoUrl}/releases
VersionInfoVersion={#AppVersion}
VersionInfoDescription=Heimdall Setup
DefaultDirName={autopf}\Heimdall
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
; An update keeps the folder and the options chosen last time.
DisableDirPage=auto
UsePreviousAppDir=yes
UsePreviousTasks=yes
; Closing Heimdall could lose unsaved changes: the person closes it.
CloseApplications=no
RestartApplications=no
; The PATH changes: tell open programs (new terminals see it).
ChangesEnvironment=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
WizardStyle=modern dynamic
WizardImageFile=wizard-164.png,wizard-328.png
WizardSmallImageFile=small-55.png,small-110.png
WizardImageBackColor=#000000
WizardImageStretch=no
SetupIconFile=..\..\crates\heimdall-app\assets\icon\heimdall.ico
UninstallDisplayIcon={app}\heimdall.exe
UninstallDisplayName=Heimdall
OutputDir={#OutDir}
OutputBaseFilename=heimdall-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"

[Messages]
WelcomeLabel1=Welcome to the Heimdall Setup Wizard
WelcomeLabel2=This will install Heimdall {#AppVersion}, an encrypted notepad, on your computer.%n%nIt installs only for you and doesn't need administrator rights.%n%nHeimdall never connects to the internet, so it can't update itself. To get a new version, download its installer from the Heimdall releases page and run it.%n%nClick Next to continue, or Cancel to exit Setup.
FinishedHeadingLabel=Heimdall is installed
ConfirmUninstall=Are you sure you want to completely remove %1?%n%nYour vaults (.heim files) and settings are kept. Heimdall leaves them where they are.

[Tasks]
Name: "path"; Description: "Use Heimdall from the terminal: type heimdall in PowerShell or cmd (adds it to your PATH)"
Name: "startmenu"; Description: "Create a Start menu shortcut"
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#BinDir}\heimdall.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\heimdall.com"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[InstallDelete]
; A shortcut unticked on an update goes away.
Type: files; Name: "{autoprograms}\Heimdall.lnk"; Tasks: not startmenu
Type: files; Name: "{autodesktop}\Heimdall.lnk"; Tasks: not desktopicon

[Icons]
Name: "{autoprograms}\Heimdall"; Filename: "{app}\heimdall.exe"; Tasks: startmenu
Name: "{autodesktop}\Heimdall"; Filename: "{app}\heimdall.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\heimdall.exe"; Description: "Open Heimdall now"; Flags: nowait postinstall skipifsilent

[Code]
const
  EnvKey = 'Environment';
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#AppGuid}_is1';

var
  { The version already installed, '' if none. }
  Previous: String;
  RepoLink: TNewStaticText;

{ ---- PATH (the user's, never the system's) ---- }

function SamePath(A, B: String): Boolean;
begin
  Result := CompareText(RemoveBackslashUnlessRoot(Trim(A)), RemoveBackslashUnlessRoot(Trim(B))) = 0;
end;

{ The user's PATH without `Dir`. }
function PathWithout(Paths, Dir: String): String;
var
  Item: String;
  P: Integer;
begin
  Result := '';
  while Paths <> '' do
  begin
    P := Pos(';', Paths);
    if P = 0 then
    begin
      Item := Paths;
      Paths := '';
    end else
    begin
      Item := Copy(Paths, 1, P - 1);
      Delete(Paths, 1, P);
    end;
    if (Trim(Item) <> '') and not SamePath(Item, Dir) then
    begin
      if Result <> '' then
        Result := Result + ';';
      Result := Result + Item;
    end;
  end;
end;

procedure SetPath(Add: Boolean);
var
  Paths, Dir, Updated: String;
begin
  Dir := ExpandConstant('{app}');
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Paths) then
    Paths := '';
  Updated := PathWithout(Paths, Dir);
  if Add then
  begin
    if Updated <> '' then
      Updated := Updated + ';';
    Updated := Updated + Dir;
  end;
  if Updated <> Paths then
  begin
    if Updated = '' then
      RegDeleteValue(HKCU, EnvKey, 'Path')
    else
      RegWriteExpandStringValue(HKCU, EnvKey, 'Path', Updated);
  end;
end;

{ ---- Heimdall still open ---- }

{ A heimdall.exe or heimdall.com running from `Dir`. A portable copy
  elsewhere doesn't matter: it holds no file here. }
function RunningIn(Dir: String): Boolean;
var
  Locator, Service, Found, Item: Variant;
  I: Integer;
begin
  Result := False;
  try
    Locator := CreateOleObject('WbemScripting.SWbemLocator');
    Service := Locator.ConnectServer('.', 'root\CIMV2');
    Found := Service.ExecQuery('SELECT ExecutablePath FROM Win32_Process WHERE Name = ''heimdall.exe'' OR Name = ''heimdall.com''');
    for I := 0 to Found.Count - 1 do
    begin
      Item := Found.ItemIndex(I);
      if not VarIsNull(Item.ExecutablePath) then
        if SamePath(ExtractFileDir(Item.ExecutablePath), Dir) then
          Result := True;
    end;
  except
    { No WMI: Windows itself will say a file is in use. }
  end;
end;

{ Asks to close Heimdall until it is closed (True) or the person cancels. }
function WaitUntilClosed(): Boolean;
begin
  Result := True;
  while RunningIn(ExpandConstant('{app}')) do
    if TaskDialogMsgBox('Heimdall is open.',
      'Save your notes and close Heimdall (the window and any terminal), then click Retry.',
      mbError, MB_RETRYCANCEL, [], 0) <> IDRETRY then
    begin
      Result := False;
      Exit;
    end;
end;

{ ---- Versions ---- }

{ <0, 0, >0 like CompareStr, for "1.4.0"-style versions. }
function CompareVersions(A, B: String): Integer;
var
  VA, VB: Int64;
begin
  if StrToVersion(A, VA) and StrToVersion(B, VB) then
    Result := ComparePackedVersion(VA, VB)
  else
    Result := CompareStr(A, B);
end;

function InitializeSetup(): Boolean;
var
  Order: Integer;
begin
  Result := True;
  if not RegQueryStringValue(HKCU, UninstallKey, 'DisplayVersion', Previous) then
    Previous := '';
  if (Previous = '') or WizardSilent() then
    Exit;
  Order := CompareVersions(Previous, '{#AppVersion}');
  if Order = 0 then
    Result := TaskDialogMsgBox('Heimdall {#AppVersion} is already installed.',
      'Install it again? This repairs missing or damaged program files. Your vaults and settings are not changed.',
      mbConfirmation, MB_OKCANCEL, ['Reinstall', 'Cancel'], 0) = IDOK
  else if Order > 0 then
    Result := TaskDialogMsgBox('A newer Heimdall (' + Previous + ') is installed.',
      'Installing {#AppVersion} replaces it with an older version. Vaults saved with ' + Previous +
      ' may not open in {#AppVersion} ("unsupported content version").' + #13#10#13#10 +
      'Install the older version anyway?',
      mbError, MB_OKCANCEL or MB_DEFBUTTON2, ['Install anyway', 'Cancel'], 0) = IDOK;
end;

{ ---- Pages ---- }

procedure OpenRepo(Sender: TObject);
var
  Code: Integer;
begin
  ShellExecAsOriginalUser('open', '{#RepoUrl}', '', '', SW_SHOWNORMAL, ewNoWait, Code);
end;

procedure InitializeWizard();
begin
  RepoLink := TNewStaticText.Create(WizardForm);
  RepoLink.Parent := WizardForm.FinishedPage;
  RepoLink.Caption := 'github.com/Azki09/heimdall';
  RepoLink.Cursor := crHand;
  RepoLink.Font.Style := [fsUnderline];
  RepoLink.Font.Color := clHotLight;
  RepoLink.OnClick := @OpenRepo;
end;

{ An update goes straight to Ready: same folder, same options. }
function ShouldSkipPage(PageID: Integer): Boolean;
begin
  Result := (Previous <> '') and
    ((PageID = wpWelcome) or (PageID = wpSelectDir) or (PageID = wpSelectTasks));
end;

procedure CurPageChanged(CurPageID: Integer);
var
  Text: String;
begin
  if (CurPageID = wpReady) and (Previous <> '') then
  begin
    WizardForm.PageDescriptionLabel.Caption := 'Setup is now ready to update Heimdall on your computer.';
    WizardForm.ReadyLabel.Caption := 'Heimdall ' + Previous + ' is installed. Setup will install {#AppVersion}, in the same folder and with the same options.' + #13#10#13#10 +
      'Your vaults and settings are not changed.';
  end;
  if CurPageID = wpFinished then
  begin
    Text := '';
    if WizardIsTaskSelected('path') then
      Text := 'To use it from the terminal, open a new PowerShell or cmd window and type:' + #13#10#13#10 +
        '        heimdall' + #13#10#13#10 +
        'Windows that were already open don''t see the change yet.' + #13#10#13#10;
    Text := Text + 'Updates are manual: Heimdall never goes online. When you want a newer version, run its installer over this one.' + #13#10#13#10 +
      'Heimdall keeps improving, and feedback is always welcome for future versions:';
    WizardForm.FinishedLabel.Caption := Text;
    WizardForm.AdjustLabelHeight(WizardForm.FinishedLabel);
    RepoLink.Left := WizardForm.FinishedLabel.Left;
    RepoLink.Top := WizardForm.FinishedLabel.Top + WizardForm.FinishedLabel.Height + ScaleY(4);
    WizardForm.RunList.Top := RepoLink.Top + RepoLink.Height + ScaleY(16);
  end;
end;

{ ---- Installing and uninstalling ---- }

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if not WaitUntilClosed() then
    Result := 'Heimdall is still open. Nothing was changed.';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    SetPath(WizardIsTaskSelected('path'));
end;

function InitializeUninstall(): Boolean;
begin
  Result := WaitUntilClosed();
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    SetPath(False);
end;
