@echo off
rem Process-only policy override for this unsigned release; no persistent policy changes.
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup.ps1" %*
set "setup_exit=%errorlevel%"
echo.
pause
exit /b %setup_exit%
