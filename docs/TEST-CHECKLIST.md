# Manual test checklist (two real PCs)

Everything in here is a check that **cannot** be automated from a single developer machine. The
179 automated tests already cover the pure logic, the file formats, the protocol round-trips and
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
- [ ] Press **Win** → Start opens **on STUDENT**, not on TEACHER. *(Currently expected to open on
      both — the local-side keyboard hook is not built yet. Note which happens.)*
- [ ] Press **Ctrl+Alt+Esc** → control is released immediately.
- [ ] After releasing, no modifier is stuck on STUDENT (type a letter — it should be lower case,
      not a shortcut).
- [ ] Ctrl+Alt+Del is **not** forwarded (documented as impossible).

---

## 5. Broadcast and black wallpaper

- [ ] **Black wallpaper**: open STUDENT's screen from TEACHER → STUDENT's wallpaper turns black.
      Close the view → the student's own wallpaper comes back.
- [ ] Kill the agent while the view is open (Task Manager on STUDENT) → restart it → wallpaper is
      restored. *Pass = a crash never leaves a black desktop.*
- [ ] Also: `cowatcher-agent.exe wallpaper-selftest` → prints **PASS**.
- [ ] **Broadcast**: `cowatcher-console.exe broadcast <STUDENT-key> 10` → TEACHER's screen fills
      STUDENT's screen, then disappears.
- [ ] Note honestly whether **Alt+Tab still escapes it** — it should, today. Input-blocking needs
      the exam lock screen, which is not built.

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
- [ ] `services.msc` → **Co-watcher Agent** is listed, Automatic, with a description.
- [ ] Task Manager → **Startup** tab → it is **not** listed (expected: services never are).
- [ ] Task Manager → **Details/Services** → the process **is** visible (it must not be hidden).
- [ ] **Reboot STUDENT** → the service is running before/without anyone logging in.
- [ ] A **standard (non-admin) student account** cannot stop it: try `sc stop CowatcherAgent` →
      access denied.
- [ ] An **administrator** can: elevated `sc stop CowatcherAgent`, then `cowatcher-agent.exe uninstall`.

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

Expected today: several of these **do** escape, because the separate-desktop exam lock is not built.
The point is to record exactly which, so the claims in `docs/FEATURES.md` stay honest.

---

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
