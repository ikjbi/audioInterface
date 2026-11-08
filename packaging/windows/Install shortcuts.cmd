@echo off
rem Adds Start-menu (and optionally autostart) shortcuts for the CURRENT USER.
rem No administrator rights needed. Keep this file next to audio-interface.exe.
setlocal
set "EXE=%~dp0audio-interface.exe"
if not exist "%EXE%" (
  echo audio-interface.exe not found next to this script.
  pause
  exit /b 1
)
set "MENU=%APPDATA%\Microsoft\Windows\Start Menu\Programs"
set "STARTUP=%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup"

powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('%MENU%\Audio Interface.lnk');" ^
  "$s.TargetPath='%EXE%'; $s.WorkingDirectory='%~dp0'; $s.IconLocation='%EXE%,0';" ^
  "$s.Description='Audio device manager'; $s.Save()"
echo Start-menu shortcut created: "Audio Interface"

choice /M "Also start Audio Interface automatically when you log in"
if errorlevel 2 goto done
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('%STARTUP%\Audio Interface.lnk');" ^
  "$s.TargetPath='%EXE%'; $s.WorkingDirectory='%~dp0'; $s.IconLocation='%EXE%,0'; $s.Save()"
echo Autostart enabled (remove "Audio Interface" from shell:startup to undo).

:done
echo.
echo Done. You can close this window.
pause
