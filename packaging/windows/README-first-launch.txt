CrabyTube for Windows
=====================

A lightweight YouTube Music player. No account, no sign-in and no telemetry.
It needs Windows 10 or 11 (64-bit) and an internet connection. There is no installer and no
administrator rights are needed.


What is in this folder
----------------------

  CrabyTube.exe            The application. Start this one.
  botguard-helper.exe      Two small programs that CrabyTube starts by itself when it plays a
  js-challenge-helper.exe  song. Do not move, rename or delete them.
  LICENSE.txt              The license of CrabyTube (GPL-3.0-or-later).
  THIRD-PARTY-LICENSES.md  The components CrabyTube uses and their licenses.
  README-first-launch.txt  This file.

Keep all the files together in one folder: CrabyTube looks for its helper programs next to
CrabyTube.exe. You can put the folder anywhere you like.


The first start: "Windows protected your PC"
--------------------------------------------

CrabyTube is free and open source. It is not signed with a paid code-signing certificate, and
Windows SmartScreen warns about any program it does not know. The warning is not a sign that
something is wrong with the file. You only need to confirm it once:

  1. Double-click CrabyTube.exe.
  2. If a blue window says "Windows protected your PC", click "More info".
  3. Click "Run anyway".

After that, CrabyTube starts normally.

If you prefer to avoid the warning, do this before extracting the zip: right-click the .zip
file, choose Properties, tick "Unblock" at the bottom and click OK.

Some antivirus programs also check unknown programs the first time they run. If yours does, let
the check finish.


Where CrabyTube keeps its data
------------------------------

  %APPDATA%\crabytube              Settings, favorites, queue and search history.
  %LOCALAPPDATA%\CrabyTube         Caches (covers, lyrics, home page snapshot) and, in its
                                   "logs" folder, a small log file. Web addresses are not
                                   written to the log.

Paste either of these into the address bar of File Explorer to open it.

Everything stays on your PC. To remove CrabyTube, delete its folder; delete the two folders above
as well if you also want to remove your data.


More information
----------------

  Source code and documentation: https://github.com/holypandadev/crabytube

CrabyTube is not affiliated with Google or YouTube.
