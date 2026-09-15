Co-watcher — preview build (working name)
=========================================

Two Windows programs. Nothing to install: unzip and double-click.

  cowatcher-console.exe   The TEACHER app (the "host"). Opens a window with a
                          live grid of student screens and the controls.

  cowatcher-agent.exe     The STUDENT program (the "client"). Runs on each
                          student PC. It has no window; run it from a terminal
                          for now (a background Windows service that starts it
                          automatically is the next step).

The first time you open the teacher app it shows a five-step tour. You can skip
it, and reopen it any time from the Help button.


WHAT THIS BUILD CAN DO
----------------------
  * See every student screen at once; pick the monitor and the preview quality
  * Open one screen larger, and listen to what that PC is playing
  * TAKE CONTROL of a PC's mouse and keyboard. Ctrl+Alt+Esc gives it back
  * Start a program on a student PC, or close one that is running
  * Block apps and games by name — closed within a second and kept closed,
    and the list survives a reboot on the student PC
  * Lock a PC, or the whole room (like Win+L)
  * Shut down / restart / sign out, now or after a 1- or 5-minute warning
  * Record a student's screen to a video file at a size and frame rate you pick
  * Broadcast YOUR screen full-screen onto a student PC
  * Switch the teacher UI language (English / Russian, or add your own .ini)

Not in this build yet: an exam lock screen a student cannot escape, collecting
student files, scheduled recordings, connecting over the internet by ID (LAN
only for now), and the AI features.


HOW TO TRY IT (two PCs, or two folders on one PC)
-------------------------------------------------
1. On the teacher PC, double-click  cowatcher-console.exe.
   Click "Add a PC". It shows a command and a 6-digit code.

2. On the student PC, open a terminal in this folder and run the command it
   showed, for example:

       cowatcher-agent.exe pair K7M2Q9...longkey... 123456

   Then leave the agent serving:

       cowatcher-agent.exe serve

3. Back on the teacher app, the student PC appears. Press "Start watching".

Both PCs must be on the same local network for this preview.


ROOMS AND THE ROOM PASSWORD  (important)
----------------------------------------
Every PC you add joins your room, and a PC CANNOT be removed from it without
the room password. Your teacher app generated one for you on first run — find
it at the bottom of the window next to "Room:", and write it down.

It is stored only on your computer, encrypted. Student PCs never receive the
password itself, only a hash of it, so reading a student PC teaches nobody
anything. To take a PC out of the room, on that PC run:

       cowatcher-agent.exe leave <room-password>

Honest limit: this stops a student. It does not stop someone with local
administrator rights on that PC, who can stop any program and delete any file.
Students should not have administrator rights.


EVERY DEVICE HAS A SHORT ID
---------------------------
Each PC shows a 6-character ID such as K7M2Q9. It is derived from that PC's
security key by hashing, not handed out by a server, so no two PCs can ever be
given the same one by accident and a PC knows its own ID even with no network.
The letters I, L, O and U are never used, so it can be read aloud without
confusion — and typing "O" for "0" or "l" for "1" still works.


COMMAND LINE (the teacher app, for testing and support)
-------------------------------------------------------
  cowatcher-console.exe                      open the window
  cowatcher-console.exe devices              list paired PCs
  cowatcher-console.exe watch <key> [n]      save screenshots of a PC
  cowatcher-console.exe listen <key> [secs]  listen to a PC
  cowatcher-console.exe control <key> [text] drive a PC's mouse/keyboard
  cowatcher-console.exe apps <key>           list what a PC can start/close
  cowatcher-console.exe record <key> [s] [w] [h] [fps]
  cowatcher-console.exe broadcast <key> [s]  put this screen on a PC
  cowatcher-console.exe act <key> <action>   lock-screen, shutdown, reboot,
                                             log-off, cancel-shutdown,
                                             lock-wallpaper, unlock-wallpaper
  cowatcher-console.exe block <key> [prog..] set the blocked-programs list

  cowatcher-agent.exe id | room | leave <pw> | serve | sessions


WHAT YOU NEED ON EACH PC (dependencies)
---------------------------------------
See DEPENDENCIES.txt. Short version: on an up-to-date Windows 10 or Windows 11
you need nothing extra. Only an older Windows 10 may need the free Microsoft
"WebView2 Runtime" for the teacher app — the one component we do not bundle,
because Windows ships it itself.


This is a preview, not a finished product. It is open source (AGPL-3.0).
