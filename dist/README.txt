Co-watcher — preview build (working name)
=========================================

Two Windows programs. Nothing to install: unzip and double-click.

  cowatcher-console.exe   The TEACHER app (the "host"). Opens a window with a
                          live grid of student screens, and the controls to
                          listen, lock, power off and block apps.

  cowatcher-agent.exe     The STUDENT program (the "client"). Runs on each
                          student PC and serves its screen, sound and actions
                          to a paired teacher. It has no window; run it from a
                          terminal for now (a background Windows service that
                          starts it automatically is the next step).


WHAT THIS BUILD CAN DO
----------------------
  * See every student screen in a grid, pick which monitor, set preview quality
  * Open one screen larger and listen to what that PC is playing
  * Lock a PC (like Win+L), one or the whole room
  * Shut down / restart / sign out, now or after a 1- or 5-minute warning
  * Block apps and games by name (e.g. steam.exe) — closed within a second and
    kept closed; the list survives a reboot on the student PC
  * Switch the teacher UI language (English / Russian, and any .ini you add)

Not in this build yet: live remote mouse/keyboard control, the custom exam
lock screen, file collection, recordings, over-the-internet directory, and the
AI features. Wallpaper lock is built but only bites once the student program
runs as the SYSTEM service (a Windows limit — the policy key is admin-only).


HOW TO TRY IT (two PCs, or two folders on one PC)
-------------------------------------------------
1. On the teacher PC, double-click  cowatcher-console.exe.
   Click "Add a PC". It shows a command and a 6-digit code.

2. On the student PC, open a terminal in this folder and run the command it
   showed, for example:

       cowatcher-agent.exe pair <long-key-from-the-teacher> 123456

   Then leave the agent serving:

       cowatcher-agent.exe serve

3. Back on the teacher app, the student PC appears. Press "Start watching".

Both PCs must be on the same local network for this preview (internet-wide
pairing by short ID is a later step). No configuration files to edit.


WHAT YOU NEED ON EACH PC (dependencies)
---------------------------------------
See DEPENDENCIES.txt. Short version: on an up-to-date Windows 10 or Windows 11
you need nothing extra. Only an older Windows 10 may need the free Microsoft
"WebView2 Runtime" for the teacher app — that is the one component we do not
bundle, because Windows ships it itself.


This is a preview, not a finished product. It is open source (AGPL-3.0).
