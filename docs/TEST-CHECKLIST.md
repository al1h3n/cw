# Manual test checklist (two real PCs)

Everything in here is a check that **cannot** be automated from a single developer machine. The
Automated tests cover the pure logic, the file formats, the protocol round-trips and
the trust boundaries; `cargo test -- --ignored` additionally runs the two network end-to-end tests.
What is left needs a second machine, administrator rights, real hardware, or a human's eyes.

Run this list before any release, and after any change to `net`, `platform` or `media`.

**Machines used in this checklist**

| Name | Role | Notes |
|------|------|-------|
| **TEACHER** | runs `cowatcher-console.exe` | your main PC |
| **STUDENT** | runs `cowatcher-agent.exe` | the laptop — ideally the *oldest/slowest* machine you have, because several checks are about whether it keeps up |

Record the date, the build (`git rev-parse --short HEAD`), and both machines' specs at the bottom.

> **Several checks below deliberately disrupt the STUDENT machine** — they black its wallpaper, seize
> its mouse, close its programs, lock it, or power it off. Do them on the test laptop, not on a PC
> someone is working on. Each one says how to undo it.

---

## 0. Setup (once per test run)

- [ ] Unzip the same build on both machines.
- [ ] TEACHER: `cowatcher-console.exe id` → note the **endpoint key** (long hex).
- [ ] STUDENT: `cowatcher-agent.exe id` → note the **device id** (6 characters) and endpoint key.
- [ ] Put both machines on the **same LAN** (same switch/Wi-Fi, no VPN).

Record: TEACHER key `____________`  STUDENT id `______`  STUDENT key `____________`

---

## 1. Pairing over a real network *(the core unproven path)*

Automated tests pair two endpoints **inside one process**. This proves it across two machines.

- [ ] TEACHER: open `cowatcher-console.exe` (no arguments) → **Add a PC** → note the command + 6-digit code.
- [ ] STUDENT: run that command within 5 minutes.
- [ ] STUDENT prints `joined room "..."` and `paired with console ...`.
- [ ] TEACHER shows the PC in the grid.
- [ ] STUDENT: `cowatcher-agent.exe room` → shows the room name.

**Wrong code is refused**
- [ ] Repeat with a deliberately wrong code → STUDENT reports `wrong pairing code`, TEACHER does **not** add it.

**Expiry**
- [ ] Show a code, wait >5 minutes, then use it → refused as expired.

### 1b. With the internet unplugged (mDNS/LAN-only path) — **important**

- [ ] Unplug the router's WAN / disable internet on both machines (keep them on the same LAN).
- [ ] Repeat the pairing above. **Pass = it still works.** This is the claim that the product works
      in a school with no internet; nothing else proves it.

### 1c. Survives an IP change

- [ ] With the PC paired and watching, change STUDENT's IP (reconnect Wi-Fi, or release/renew).
- [ ] TEACHER reconnects to it without re-pairing. Pass = it comes back on its own.

---

## 2. Watching, sound and video

- [ ] TEACHER: **Start watching** → STUDENT's screen appears in the grid.
- [ ] Click the tile → the opened view refreshes noticeably faster.
- [ ] Multi-monitor: if STUDENT has 2+ screens, the monitor chips appear and switching shows the
      *other* screen (not the primary twice).
- [ ] Preview quality picker changes sharpness.

**Sound**
- [ ] Play music on STUDENT → **Listen** on TEACHER → you hear it. Stop → silence.

**H.264 video stream** (`kbps` is the number that decides whether a room fits on the network)
- [ ] `cowatcher-console.exe stream <STUDENT-key> 10 1280 720 30 2000`
- [ ] Record: achieved fps `_____`, kbit/s `_____` (idle screen)
- [ ] Repeat while a **video plays** on STUDENT — this is the realistic bitrate.
      Record: fps `_____`, kbit/s `_____`
- [ ] `... stream <key> 10 1920 1080 30 4000` → record fps `_____`, kbit/s `_____`

> Dev-PC reference (both machines in one box): 30.2 fps at 720p and 1080p, 57–78 kbit/s idle.
> A slower STUDENT will be lower — that number is the point of measuring it here.

**Glass-to-glass latency** (PLAN 0.5's outstanding item)
- [ ] Open a running stopwatch/clock with milliseconds on STUDENT, stream it, and photograph both
      screens at once with a phone. Difference = end-to-end latency.
      Record: `_____ ms` (budget in AGENTS.md §6 is < 100 ms on LAN)

**Under a bad network** (PLAN 0.6's outstanding item)
- [ ] With [clumsy](https://jagt.github.io/clumsy/) on TEACHER, apply 5 % loss + 50 ms jitter.
- [ ] Stream stays usable and recovers within ~1 s of a stall; no frozen/stale picture.

---

## 3. Acting on the PC

> These interrupt STUDENT. Each row says how to undo it.

- [ ] **Lock** one PC → STUDENT locks. *Undo: log back in.*
- [ ] **Lock all** from the room menu.
- [ ] **Blocked apps**: add `notepad.exe`, Save → open Notepad on STUDENT → it closes within ~1 s,
      and closes again if reopened. *Undo: remove it from the list and Save.*
- [ ] **Blocking survives a reboot with no network**: with `notepad.exe` blocked, unplug STUDENT's
      network, reboot it, log in, start the agent, open Notepad → still blocked. **This is the D9
      offline-enforcement claim.**
- [ ] **Programs** dialog: the list is STUDENT's real Start Menu; **Start** launches a classic app
      (e.g. Character Map); **Close** ends a running one.
- [ ] **Shut down with a 1-minute warning** → STUDENT shows Windows' own countdown → **Cancel a
      pending shutdown** stops it. *Pass = the countdown disappears.*
- [ ] **Sign out** *(not yet verified anywhere — do this one)*: STUDENT logs off.
- [ ] **Restart** *(not yet verified anywhere)*: STUDENT reboots.
- [ ] **Shut down, now** *(not yet verified anywhere)*: STUDENT powers off.

---

## 4. Remote mouse and keyboard

- [ ] Open STUDENT's screen → **Take control**.
- [ ] Your mouse moves STUDENT's pointer and lands where you expect **even though the two screens
      are different sizes/scales**. (This is the fractions-not-pixels claim.)
- [ ] Clicking focuses a window on STUDENT; typing appears there.
- [ ] Take control with **Right Ctrl** (VirtualBox-style host key). The viewer border/hint shows
      control is on.
- [ ] Press **Win** → Start opens **on STUDENT only**, not on TEACHER. *(Now expected on the student
      side — the local keyboard grab `platform::keygrab` is built. Note if it still opens on TEACHER.)*
- [ ] Also try **Alt+Tab**, **Ctrl+Esc**, **Alt+F4** while controlling → they go to STUDENT, not
      TEACHER.
- [ ] Press **Right Ctrl** again → control is released immediately.
- [ ] After releasing, no modifier is stuck on STUDENT (type a letter — it should be lower case,
      not a shortcut).
- [ ] Right Ctrl only grabs while the viewer window is **focused** — click another app on TEACHER and
      confirm Right Ctrl works normally there.
- [ ] Ctrl+Alt+Del / Win+L are **not** forwarded (documented as impossible — Secure Attention Sequence).

---

## 5. Broadcast and black wallpaper

- [ ] **Black wallpaper**: open STUDENT's screen from TEACHER → STUDENT's wallpaper turns black.
      Close the view → the student's own wallpaper comes back.
- [ ] Kill the agent while the view is open (Task Manager on STUDENT) → restart it → wallpaper is
      restored. *Pass = a crash never leaves a black desktop.*
- [ ] Also: `cowatcher-agent.exe wallpaper-selftest` → prints **PASS**.
- [ ] **Broadcast** (unlocked): from the Console's **Content → Broadcast** picker choose a monitor or
      window and target PCs → TEACHER's screen fills STUDENT's screen, then Stop makes it disappear.
- [ ] Note honestly whether **Alt+Tab still escapes** the *unlocked* broadcast — it should, today.
- [ ] **Locked broadcast**: tick **Lock students onto it** → on STUDENT try Alt+Tab / Win / Ctrl+Esc /
      Alt+F4. **Pass = none of them escape** (it shows on a separate desktop with the key guard). This is
      built but unverified live — record exactly what happens.
- [ ] The client cursor over a broadcast is a **normal arrow**, not a busy spinner.
- [ ] **Wallpaper reverts on unlock**: lock the wallpaper (or push one), then unlock → STUDENT's *own*
      original wallpaper returns (not a black or blank one). The opened-PC menu shows locked/unlocked.
- [ ] **Exam lock**: from the opened view start exam lock → STUDENT sees the fullscreen message on a
      separate desktop; Alt+Tab/Win do nothing; End exam → the student's desktop returns intact.
- [ ] Start an untimed exam, use Ctrl+Alt+Del to sign out, then sign back into the same account.
      The lock should return without another command. TEACHER should see an interruption notice,
      and **Reapply exam** should work. Then use **Unlock exam** and confirm the desktop returns.
- [ ] During exam, use Win+L and unlock Windows. Confirm the exam message reappears; check the
      teacher's grid and opened view for the exam badge. The Winlogon screen itself is OS-controlled.
- [ ] Turn STUDENT off. Its grid preview should be blank by default. Enable the last-preview
      setting and confirm the last image is shown instead.

---

## 6. Recording

- [ ] `cowatcher-console.exe record <STUDENT-key> 20 1280 720 10`
- [ ] Record: frames `_____`, reported fps `_____`.
- [ ] Copy the `.avi` from STUDENT's `%LOCALAPPDATA%\co-watcher\agent\recordings\` and **play it**
      in VLC or Windows Media Player. Pass = it opens and plays at *normal speed*.
- [ ] **Crash safety**: start a 60 s recording, and after ~20 s kill the agent (or pull the power on
      a machine you don't mind). The part-written file must still play. **This is PLAN 2.12's claim.**

---

## 7. Rooms and the leave-password

- [ ] TEACHER: bottom of the window → **Room** → note the password.
- [ ] STUDENT: `cowatcher-agent.exe leave WRONGPASSWORD` → refused.
- [ ] Repeat 5 times → locked out for 60 s; the *correct* password is also refused during lockout.
- [ ] After the lockout, `cowatcher-agent.exe leave <correct-password>` → leaves the room, and
      TEACHER can no longer control it.
- [ ] Re-pair to continue.

---

## 8. Administrator-only (needs an elevated prompt on STUDENT)

None of this could be verified on the dev machine — it is the elevation gate from PLAN 1.4b.

- [ ] STUDENT, **elevated** prompt: `cowatcher-agent.exe install` → reports the service installed.
- [ ] Run `install` twice from Downloads. Both invocations succeed; after the command exits, the
      Downloads executable can be deleted. Check the service points to
      `%ProgramData%\co-watcher\bin\cowatcher-agent.exe`.
- [ ] Install with a signed HTTPS update manifest and public key. Confirm the `CowatcherAgentUpdate`
      scheduled task exists. Publish a higher version and run the task manually; confirm the service
      restarts on the new version. Try an altered signature and a hash mismatch; neither may stop the
      working service. Change to another HTTPS mirror with the same key and repeat.
- [ ] `services.msc` → **Co-watcher Agent** is listed, Automatic, with a description.
- [ ] Task Manager → **Startup** tab → it is **not** listed (expected: services never are).
- [ ] Task Manager → **Details/Services** → the process **is** visible (it must not be hidden).
- [ ] **Reboot STUDENT** → the service is running before/without anyone logging in.
- [ ] A **standard (non-admin) student account** cannot stop it: try `sc stop CowatcherAgent` →
      access denied.
- [ ] An **administrator** can: elevated `sc stop CowatcherAgent`, then `cowatcher-agent.exe uninstall`.
- [ ] After `uninstall`, the service and scheduled update task are gone. Reinstall immediately; if
      Windows reports pending deletion, close `services.msc`, wait briefly, and retry.

**Capture through the service (the per-session helper — the whole point of 1.4b)**

The service runs in session 0 and cannot capture a desktop itself; it launches a helper into the
logged-in student's session. This is the path that must work for "watch/control with autostart", and
it is entirely unverified.

- [ ] With the service installed, **log in as the student**, then from TEACHER open that PC's screen.
      Pass = you see the live screen and can control it, **without** anyone having run
      `cowatcher-agent serve` by hand. (Behind the scenes the service started `cowatcher-agent helper`
      in the student's session — visible in Task Manager → Details as a second `cowatcher-agent.exe`
      with **no console window**.)
- [ ] **Sign out and back in** on STUDENT (or switch user) → after a few seconds TEACHER can watch
      again (the service relaunched the helper in the new session; the tile reconnects on its own).
- [ ] At the **login screen** (no one signed in) TEACHER shows the PC offline — expected, there is no
      desktop to capture until someone logs in.
- [ ] First run after upgrading: the student's device id is **unchanged** (the identity was migrated
      from `%LOCALAPPDATA%` to `%ProgramData%\co-watcher\agent`). If it changed, TEACHER must re-pair.
      Note: **pair before installing the service** — an admin-created ProgramData folder can stop a
      standard student account from writing a new pairing there.

**Constant wallpaper (needs the service, runs as SYSTEM)**
- [ ] With the service running, on STUDENT open Settings → Personalisation → Background.
      Pass = changing the wallpaper is blocked/greyed out.
- [ ] Uninstall the service → the student can change it again.

---

## 9. Wake-on-LAN (needs a BIOS setting)

- [ ] On STUDENT: enable **Wake on LAN** in BIOS/UEFI (often "Power On by PCI-E"), and in Windows:
      Device Manager → network adapter → Power Management → *Allow this device to wake the computer*.
- [ ] Pair and watch STUDENT once (so TEACHER learns its MAC), then **shut STUDENT down**.
- [ ] TEACHER: the offline tile shows a **Wake** button → click it → STUDENT powers on.
- [ ] Also works from the CLI: `cowatcher-console.exe wake <STUDENT-MAC>`.

> If it does not wake: confirm the BIOS setting first, and note that WoL usually needs a **wired**
> connection and does not survive a full power cut on some machines.

---

## 10. Performance on the slow machine *(re-measure — the dev numbers are from a fast PC)*

With STUDENT being the oldest laptop, use Task Manager → Details on STUDENT:

- [ ] Agent running, **nobody watching** → CPU `_____ %`, RAM `_____ MB`
      (budget: < 0.5 % CPU, < 30 MB)
- [ ] TEACHER watching the grid (thumbnails only) → CPU `_____ %` (budget: < 3 %)
- [ ] TEACHER streaming 1080p30 → CPU `_____ %` (budget: < 10 %, and **expect to miss it** — the
      software H.264 encoder is the documented fallback; the hardware encoder is the fix)
- [ ] Minimise/stop watching → agent CPU returns to idle **within 3 seconds**.
- [ ] TEACHER with all PCs in the grid → CPU `_____ %`, RAM `_____ MB` (budget: < 15 %, < 400 MB)

---

## 11. Escape checklist — try to break out as a student would

On STUDENT, while a broadcast or lock is active, try each and record what happens:

- [ ] Alt+Tab   - [ ] Win   - [ ] Win+D   - [ ] Win+Tab   - [ ] Ctrl+Shift+Esc (Task Manager)
- [ ] Ctrl+Alt+Del   - [ ] Win+L   - [ ] Unplug the network   - [ ] End the agent from Task Manager
- [ ] Safe mode   - [ ] Change the system clock

Expected today: the separate-desktop exam lock and locked broadcast **are** built (with the
`platform::keyguard` hook), so Alt+Tab / Win / Win+D / Win+Tab / Ctrl+Shift+Esc should be trapped;
**Ctrl+Alt+Del and Win+L still escape** (Secure Attention Sequence, unblockable from user space), and
Task Manager is not yet stripped (the `DisableTaskMgr` policy of D23 is not wired). Killing the agent,
safe mode and a clock change are organisational/OS limits (see *Impossible* in FEATURES). The point is
to record exactly which escape, so the claims in `docs/FEATURES.md` stay honest.

---

## 12. Newer features (2026-09-22 → 09-23 batches)

- [ ] **Screenshot**: opened view → **Screenshot** → a full-resolution `.jpg` is saved on TEACHER
      (in the console data dir's `screenshots/`) and a toast confirms it. Locked/prompted screens error
      cleanly instead of saving a black image.
- [ ] **Recording resolution guard**: try to record a PC at a size **larger** than its monitor → a
      clear error, no file. With `record all`, if even one watched PC is smaller than the requested
      size the whole batch is refused and the too-small PCs are named.
- [ ] **Freeze input (screen lock)**: opened view → freeze a student's own mouse+keyboard *without*
      taking control → STUDENT cannot type/click; release, or disconnect the Console, frees it.
- [ ] **Per-PC wallpaper + fit**: push a wallpaper to one PC from its Restrictions menu with a fit
      (fill/fit/stretch/centre/tile) → it applies; reboot STUDENT → the pushed wallpaper returns.
- [ ] **Broadcast survives a client reboot**: start a broadcast, reboot one STUDENT → the broadcast
      does **not** stay "stuck on" for that PC (dead-audience timeout); the banner clears.
- [ ] **Voice input**: in Surey, pick a transcription mode (chat / custom Whisper endpoint / local
      program), record → the recognised text lands in the box. Chat mode against an endpoint with no
      Whisper route should give a clear message, not a bare 404.
- [ ] **Surey in the background**: send a message, close the panel, reopen → the conversation is still
      there and any in-flight reply completed; the launcher pulses while it was still working.
- [ ] **Multiple classrooms**: header switcher → **New classroom** opens a second Console window; each
      has its own devices/room/blocklist.
- [ ] **Browser dashboard**: run `cowatcher-console web`, open the printed loopback URL (with its
      token) → the same grid works; a request without the token is refused (401).

## Already verified — do not repeat unless something changed

These were measured end to end on the dev machine and are covered by automated tests:
pairing inside one process, thumbnail capture, audio (16 kHz mono, loud vs silent), power
shutdown+cancel, app blocking (Notepad closed within 1 s), launcher (Character Map start/stop,
unknown id refused), recording (valid AVI, real-time playback), broadcast (frames delivered, window
closed cleanly), remote input (refused before consent; typed text read back), black wallpaper
round-trip, H.264 stream at 30.2 fps.

---

## Sign-off

| Field | Value |
|---|---|
| Date | |
| Build (`git rev-parse --short HEAD`) | |
| TEACHER machine (CPU / RAM / screen) | |
| STUDENT machine (CPU / RAM / screen) | |
| Windows versions | |
| Blocking problems found | |
| Tester | |
