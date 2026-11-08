Audio Interface (portable)
==========================

No installation and no administrator rights needed.

1. Extract this zip anywhere you can write to, for example:
     %LOCALAPPDATA%\Programs\AudioInterface
   (paste that into the File Explorer address bar)
2. Double-click audio-interface.exe.
   The first time, Windows SmartScreen may say "Windows protected your PC"
   because the app is not code-signed. Click "More info" -> "Run anyway".
3. Optional: double-click "Install shortcuts.cmd" to add a Start-menu entry
   and (if you want) start the app when you log in. Current user only.

Level meters need microphone access:
  Settings -> Privacy & security -> Microphone ->
  "Let desktop apps access your microphone" = On

Settings are stored in %APPDATA%\ikjbi\AudioInterface\config\settings.json.
Recordings go to Documents\Audio Interface Recordings.
To uninstall, delete the folder and the shortcuts.

Command line (from this folder):  audio-interface.exe --help
More: https://github.com/ikjbi/audioInterface
