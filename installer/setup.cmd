@echo off
rem Starts the Termyx setup screen in this console window.
rem Pass options through, for example: setup.cmd -Uninstall
cd /d "%USERPROFILE%"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup.ps1" %*
if errorlevel 1 pause
