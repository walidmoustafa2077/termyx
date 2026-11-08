@echo off
setlocal
cd /d "%~dp0"

set "EXE=target\release\termyx.exe"

where cargo >nul 2>nul
if errorlevel 1 goto cargo_home
set "CARGO=cargo"
goto build

:cargo_home
set "CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"

:build
echo Building release binary...
"%CARGO%" build --release
if errorlevel 1 (
    echo Build failed.
    pause >nul
    exit /b 1
)

:run
start "" "%EXE%" %*
