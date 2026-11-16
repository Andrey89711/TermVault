@echo off
powershell.exe -NoProfile -File "%~dp0setup.ps1" %*
set "setup_exit=%errorlevel%"
echo.
pause
exit /b %setup_exit%
