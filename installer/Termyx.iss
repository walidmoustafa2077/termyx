; Termyx installer, compiled by .github/workflows/release.yml.
;
; Everything is per-user: files land in %LOCALAPPDATA%\Programs\Termyx and every
; registry write stays under HKCU, so installing and uninstalling need no
; administrator rights.

#define AppName "Termyx"
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#define AppPublisher "Walid Mostafa"
#define AppUrl "https://github.com/walidmoustafa2077/termyx"
#define AppExe "termyx.exe"

[Setup]
AppId={{4C1F9E27-6A3B-4D58-9E07-B2C5A8D41F63}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
DefaultDirName={localappdata}\Programs\Termyx
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename={#AppName}-{#AppVersion}-setup
SetupIconFile=..\assets\icons\termyx.ico
UninstallDisplayIcon={app}\{#AppExe}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
MinVersion=10.0
ArchitecturesAllowed=x64compatible
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "startmenu"; Description: "Create a &Start Menu shortcut"; GroupDescription: "Shortcuts:"
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked
Name: "addtopath"; Description: "Add Termyx to my &PATH (run ""termyx"" anywhere)"; GroupDescription: "Integration:"
Name: "contextmenu"; Description: "Add ""Open in &Termyx"" to the Explorer context menu"; GroupDescription: "Integration:"
Name: "adminmenu"; Description: "Add ""Open in Termyx (&Administrator)"" to the Explorer context menu"; GroupDescription: "Integration:"
Name: "psmodule"; Description: "Install the Termyx &PowerShell module (prompt and spellcheck)"; GroupDescription: "Integration:"

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\shell\powershell\Termyx\*"; DestDir: "{app}\powershell\Termyx"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\shell\powershell\tools\Install.ps1"; DestDir: "{app}\powershell\tools"; Flags: ignoreversion
Source: "path.ps1"; DestDir: "{app}\setup"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{%USERPROFILE}"; IconFilename: "{app}\{#AppExe}"; Comment: "Termyx terminal"; Tasks: startmenu
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{%USERPROFILE}"; IconFilename: "{app}\{#AppExe}"; Comment: "Termyx terminal"; Tasks: desktopicon

[Registry]
; "Open in Termyx" for a folder, a folder background, and a drive. %1 is the
; selected item, %V is the folder behind the background menu.
Root: HKA; Subkey: "Software\Classes\Directory\shell\Termyx"; ValueType: string; ValueName: ""; ValueData: "Open in Termyx"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\Termyx"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe},0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\Termyx\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu

Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\Termyx"; ValueType: string; ValueName: ""; ValueData: "Open in Termyx"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\Termyx"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe},0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\Termyx\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%V"""; Tasks: contextmenu

Root: HKA; Subkey: "Software\Classes\Drive\shell\Termyx"; ValueType: string; ValueName: ""; ValueData: "Open in Termyx"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Drive\shell\Termyx"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe},0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\Drive\shell\Termyx\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: contextmenu

; Explorer has no per-verb elevation flag, so the administrator entries start a
; second, elevated Termyx through Start-Process. HasLUAShield draws the icon.
Root: HKA; Subkey: "Software\Classes\Directory\shell\TermyxAdmin"; ValueType: string; ValueName: ""; ValueData: "Open in Termyx (Administrator)"; Flags: uninsdeletekey; Tasks: adminmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\TermyxAdmin"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe},0"; Tasks: adminmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\TermyxAdmin"; ValueType: string; ValueName: "HasLUAShield"; ValueData: ""; Tasks: adminmenu
Root: HKA; Subkey: "Software\Classes\Directory\shell\TermyxAdmin\command"; ValueType: string; ValueName: ""; ValueData: """{sys}\WindowsPowerShell\v1.0\powershell.exe"" -NoProfile -WindowStyle Hidden -Command ""Start-Process -FilePath '{app}\{#AppExe}' -ArgumentList @('%1') -Verb RunAs"""; Tasks: adminmenu

Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\TermyxAdmin"; ValueType: string; ValueName: ""; ValueData: "Open in Termyx (Administrator)"; Flags: uninsdeletekey; Tasks: adminmenu
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\TermyxAdmin"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe},0"; Tasks: adminmenu
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\TermyxAdmin"; ValueType: string; ValueName: "HasLUAShield"; ValueData: ""; Tasks: adminmenu
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\TermyxAdmin\command"; ValueType: string; ValueName: ""; ValueData: """{sys}\WindowsPowerShell\v1.0\powershell.exe"" -NoProfile -WindowStyle Hidden -Command ""Start-Process -FilePath '{app}\{#AppExe}' -ArgumentList @('%V') -Verb RunAs"""; Tasks: adminmenu

Root: HKA; Subkey: "Software\Classes\Drive\shell\TermyxAdmin"; ValueType: string; ValueName: ""; ValueData: "Open in Termyx (Administrator)"; Flags: uninsdeletekey; Tasks: adminmenu
Root: HKA; Subkey: "Software\Classes\Drive\shell\TermyxAdmin"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\{#AppExe},0"; Tasks: adminmenu
Root: HKA; Subkey: "Software\Classes\Drive\shell\TermyxAdmin"; ValueType: string; ValueName: "HasLUAShield"; ValueData: ""; Tasks: adminmenu
Root: HKA; Subkey: "Software\Classes\Drive\shell\TermyxAdmin\command"; ValueType: string; ValueName: ""; ValueData: """{sys}\WindowsPowerShell\v1.0\powershell.exe"" -NoProfile -WindowStyle Hidden -Command ""Start-Process -FilePath '{app}\{#AppExe}' -ArgumentList @('%1') -Verb RunAs"""; Tasks: adminmenu

; App Paths makes "termyx" resolve from Win+R and from shells that do not read
; the updated PATH yet.
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\termyx.exe"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\termyx.exe"; ValueType: string; ValueName: "Path"; ValueData: "{app}"

[Run]
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\setup\path.ps1"" -Install -Directory ""{app}"""; StatusMsg: "Adding Termyx to your PATH..."; Flags: runhidden waituntilterminated; Tasks: addtopath
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\powershell\tools\Install.ps1"" -ModuleSource ""{app}\powershell\Termyx"""; StatusMsg: "Installing the Termyx PowerShell module..."; Flags: runhidden waituntilterminated; Tasks: psmodule
Filename: "{app}\{#AppExe}"; Description: "Launch Termyx"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\setup\path.ps1"" -Remove -Directory ""{app}"""; StatusMsg: "Removing Termyx from your PATH..."; RunOnceId: "TermyxPath"; Flags: runhidden waituntilterminated
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\powershell\tools\Install.ps1"" -Uninstall"; StatusMsg: "Removing the Termyx PowerShell module..."; RunOnceId: "TermyxModule"; Flags: runhidden waituntilterminated
