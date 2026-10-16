CrabyTube for macOS
===================

A lightweight YouTube Music player. No account, no sign-in and no telemetry.
It needs macOS 11 or later (Apple silicon or Intel) and an internet connection. There is no
installer and no administrator rights are needed to run it.


What is in this folder
----------------------

  CrabyTube.app             The application. It contains two small helper programs that
                            CrabyTube starts by itself when it plays a song.
  LICENSE.txt               The license (GPL-3.0-or-later).
  THIRD-PARTY-LICENSES.md   The licenses of the components CrabyTube uses.
  THIRD-PARTY-LICENSES.txt  The full texts of those licenses.
  README-first-launch.txt   This file.

Drag CrabyTube.app to your Applications folder (or anywhere you like). Do not open or change
what is inside the app: it is signed, and a change would stop macOS from starting it.


The first start: "Apple could not verify CrabyTube"
---------------------------------------------------

CrabyTube is free and open source. It is not notarized by Apple, which needs a paid developer
account, and macOS blocks every app it has not verified the first time it starts. The warning
is not a sign that something is wrong with the file. You only need to allow it once.

On macOS 15 (Sequoia) and later:

  1. Double-click CrabyTube. A message says "CrabyTube" Not Opened. Click "Done"
     (not "Move to Trash").
  2. Open System Settings and choose "Privacy & Security" in the sidebar.
  3. Scroll down to the "Security" section. It says "CrabyTube" was blocked to protect your
     Mac. Click "Open Anyway".
  4. Enter your password or use Touch ID, then confirm.

CrabyTube starts, and from then on it opens normally. The "Open Anyway" button stays there for
about an hour after the attempt in step 1; if you do not see it, double-click CrabyTube again
and look once more. On these versions, right-clicking the app and choosing "Open" no longer
skips the warning.

On macOS 14 and earlier:

  Hold Control and click CrabyTube (or right-click it), choose "Open", then click "Open" in
  the message that appears. If you only see "Cancel" and "Move to Trash", use the steps above.

If you prefer Terminal, this does the same thing. Run it once, then open the app normally:

  xattr -dr com.apple.quarantine /Applications/CrabyTube.app

(Change the path if the app is somewhere else.)


Updates
-------

CrabyTube looks for a new version once a day; Settings > Updates shows what it found, has
"Check now", and can turn the daily check off. "Update now" downloads the new version, checks
that it is the one the release signed, puts it in place of the app and opens it again. That needs
the app in a folder you can write to, such as Applications on an administrator account; otherwise
"Update now" opens the release page. Your settings, favorites and queue stay as they are.


Media keys and "Now Playing"
----------------------------

While CrabyTube plays, the play/pause, next and previous keys, headphone buttons and the Now
Playing widget in Control Center work with it, including the cover and the progress bar. Until
something is playing, macOS sends the media keys to the Music app. On many keyboards the keys
are F7, F8 and F9; if they change the brightness or volume instead, hold the "fn" key or change
"Use F1, F2, etc. keys as standard function keys" in System Settings > Keyboard.


Where CrabyTube keeps its data
------------------------------

  ~/Library/Application Support/CrabyTube
        Settings, favorites, queue and search history, and, in its "logs" folder, a small log
        file. Web addresses are not written to the log.
  ~/Library/Caches/crabytube
        Caches: covers, lyrics, the home page snapshot and data for the player.

"Local data > Export" in Settings saves a backup in your Documents folder.

To open a folder in Finder, choose Go > Go to Folder (Shift-Command-G) and paste its path.

Everything stays on your Mac. To remove CrabyTube, quit it and move CrabyTube.app to the Trash.
To remove all its data as well, quit the app and run this in Terminal:

  rm -rf ~/Library/Application\ Support/CrabyTube ~/Library/Application\ Support/crabytube ~/Library/Caches/crabytube

(The first two names are the same folder on a normal disk; on a case-sensitive disk they are
two.) Backups you exported to Documents are not removed.


More information
----------------

  Source code and documentation: https://github.com/holypandadev/crabytube

CrabyTube is not affiliated with Google or YouTube.
