@echo off
setlocal
set "termvault_selfremove="
if exist "%~dp0.termvault-install.json" set "termvault_selfremove=1"
if defined termvault_selfremove (
    "%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall.ps1" -KeepLauncher %*
) else (
    "%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall.ps1" %*
)
if errorlevel 1 (
    pause
    exit /b 1
)
pause
if not defined termvault_selfremove exit /b 0
if exist "%~dp0.termvault-install.json" exit /b 0
rem End the batch context before deleting this launcher. rd has no /s: user files are never removed.
(goto) 2>nul & del "%~f0" & rd "%~dp0" 2>nul & exit /b 0
