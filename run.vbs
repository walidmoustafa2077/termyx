Option Explicit

' Hidden launcher: builds the release binary in a console that never appears,
' then starts the GUI. Use run.cmd instead when you want to watch the build.
Dim shell, fso, root, cargo, quote, rc, exe, args, i
Set shell = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")

root = fso.GetParentFolderName(WScript.ScriptFullName)
shell.CurrentDirectory = root

cargo = shell.ExpandEnvironmentStrings("%USERPROFILE%") & "\.cargo\bin\cargo.exe"
If Not fso.FileExists(cargo) Then cargo = "cargo"
quote = Chr(34)

rc = shell.Run(quote & cargo & quote & " build --release", 0, True)
If rc <> 0 Then
    MsgBox "Build failed. Run run.cmd in a terminal to see the error.", 16, "Termyx"
    WScript.Quit rc
End If

exe = root & "\target\release\termyx.exe"
args = ""
For i = 0 To WScript.Arguments.Count - 1
    args = args & " " & quote & Replace(WScript.Arguments(i), quote, quote & quote) & quote
Next
shell.Run quote & exe & quote & args, 1, False
