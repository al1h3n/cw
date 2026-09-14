# Feature status against the brief

Every capability named in [`../ClassWatcher.md`](../ClassWatcher.md), with an honest state. Updated
2026-09-14. Legend: **done** = built and tested · **partial** = some of it works · **planned** = designed
but no code.

**Summary: 8 of 27 done, 4 partial, 15 planned.** The product can now *watch* screens, *listen* to a PC, *lock* and *power off* one PC or a whole room,
and *block* apps and games. What it cannot yet do is take live *remote control* of a mouse and keyboard,
and it has no signed offline policy engine yet (blocking persists on the Agent, but power/lock do not).

There is also a list of things that **cannot** be built at all, and things we **will not** build:
see [Impossible, and deliberately refused](#impossible-and-deliberately-refused) at the end.

## Free tier

| # | From the brief | State | Notes |
|---|----------------|-------|-------|
| 1 | Screen share, pin other monitors, virtual desktops | **done** (see limits) | Every monitor is enumerated and selectable per PC; preview width is chosen by the teacher (240–1920 px) separately for the grid and the opened screen, which also refreshes 4× faster. Still JPEG frames rather than an H.264 stream — fine to ~4 fps, and spike 0.5 has the codec ready when smoother is needed. Inactive virtual desktops cannot be captured by anyone — see *Impossible*. |
| 2 | Audio share, on/off switch | **done** | WASAPI loopback of what the student hears, downmixed to mono and decimated to ~16 kHz (~256 kbit/s). Off until asked for, and exclusive: one PC at a time. Verified end to end — 16 kHz mono, 64 000 samples for 4.0 s, loud while a tone played and exactly 0 in silence. Raw PCM for now; Opus would cut it to ~32 kbit/s when several PCs need listening at once. |
| 3 | Remote mouse and keyboard, Win key captured, exit chord | planned | Spike 0.9 proved the keyboard hook; injection and forwarding not built. |
| 4 | Lock screen like parental controls | **partial** | One PC or the whole room can be locked now — the standard Windows lock, as Win+L. The full-screen *custom* lock (separate Win32 desktop, spike 0.8) that the student cannot unlock without the teacher is #10. |
| 5 | Shut down / power off all or one PC | **done** | Shut down, restart, sign out and cancel, for one PC or every connected PC, with a Now / 1 min / 5 min warning. Windows shows the student its own localised countdown; an immediate shutdown force-closes programs so one unsaved file cannot veto the room. Offline PCs are never queued (a shutdown must not fire next morning). Every action, including refusals, goes to the PC's `audit.log` (D3). Verified live: a real 300 s countdown started and was cancelled. Wake-on-LAN is still planned. |
| 6 | Constant wallpaper nobody can change | **built, needs the service** | Implemented as the `NoChangingWallPaper` user policy (PLAN 2.6), toggled by a Lock/Unlock wallpaper action and a menu button. It persists across reboots because it is a registry value. **But** the `Policies` hive is admin/GPO-writable only, so it takes effect only once the Agent runs as the SYSTEM service (step 1.4b); an unelevated Agent gets "Access is denied", which is why this is not yet ticked done. Setting a *specific* school image needs file transfer (not built). |
| 7 | Keep student files temporarily, wipe with one button, host can browse | planned | Baseline + diff design; wipe must be scope-proven by tests. |
| 8 | Add computers by ID or LAN; changes apply when back online | **done** (ID/LAN) / planned (offline queue) | Pairing by key with a 6-digit code, mDNS on LAN, reconnect by key after an IP change. The offline mailbox (D9) is not built. |
| 9 | Enable/disable programs, editable games list | **done** (apps) / planned (websites) | A room-wide, editable list of program names (`steam.exe`, `roblox.exe`, …) closes those programs on every connected PC within a second and closes them again if a student reopens them. Matching is by exact file name so a rule never kills an unrelated app, system-critical processes are protected, and the list is saved on the Agent so it keeps enforcing after a reboot with no network (D9). Verified live: Notepad closed within 1 s and stayed closed until the rule was cleared. Website blocking via browser policy files is still planned. |
| 10 | Custom lock screen: background, per-PC shortcuts, terminal with `unlock` and power commands | planned | Depends on #4. |
| 11 | Black background while the host watches | planned | Cheap win once streaming lands. |
| 12 | Screen recordings, all or one, scheduled | planned | Agent-side, low fps, crash-safe segments. |
| 13 | Broadcast the host screen to all/some, input blocked | planned | Reuses #4's lock desktop. |
| 14 | Send audio/video in real time, notification, play once | planned | Preload + synchronised start beats live streaming for exams. |
| 15 | Update centre from the deploy branch | planned | Signed manifests, channels, rollback. |
| 16 | Tutorial on first start, skippable | planned | — |
| 17 | One binary, choose client or server, ID added later | **partial** | Two binaries today (`cowatcher-agent`, `cowatcher-console`). Double-clicking the console opens the window; the agent is headless. A single installer with a role picker is the plan (D8). |

## Paid tier (AI)

| # | From the brief | State | Notes |
|---|----------------|-------|-------|
| 18 | AI lesson summaries | planned | — |
| 19 | Bring your own API key (Claude/ChatGPT/Gemini/DeepSeek), or a hosted model | planned | Two adapters cover nearly everything: OpenAI-compatible and Anthropic. |
| 20 | AI watches the screen and acts (close games etc.) | planned | Cost ladder designed: rules → change gate → OCR text → local classifier → batched vision. |
| 21 | More than 10 devices needs a subscription | planned | Enforceable only in official builds and our hosted service — see AGENTS.md D1. |
| 22 | Pricing (schools per room, business per device) | **done** (as a plan) | `docs/BUSINESS.md`. |

## Platforms, infrastructure, policy

| # | From the brief | State | Notes |
|---|----------------|-------|-------|
| 23 | Windows first, then macOS, Linux, BSD | **partial** | Windows 10/11 works. The other platforms compile as stubs only. |
| 24 | Works with no internet; changes apply on reconnect | **partial** | LAN discovery works offline; enforcing a stored policy offline (D9) is not built because there is no policy engine yet. |
| 25 | Admin secret code to disable watchers | planned | Deliberately **not** a hardcoded password (D10): per-Org code + offline one-time codes. |
| 26 | Optimised, low resource use | **done** so far | Idle agent captures nothing; thumbnails measured at ≤0.03 % of one core on the dev PC. Needs a rerun on an old lab PC. |
| 27 | CI/CD, auto-built releases, GitHub + GitLab + Codeberg mirrors | **partial** | CI runs fmt/clippy/tests/licence checks on three OSes. No releases, signing or mirrors yet. |

## Impossible, and deliberately refused

Two different things, kept apart on purpose. The first list is where the operating system says no and
no amount of engineering changes that — if a competitor claims one of these, they are either wrong or
doing something that will break. The second list is where it *is* possible and we choose not to.

### Cannot be done at all

| Wanted | Why not | What you get instead |
|--------|---------|----------------------|
| See a virtual desktop the student is **not** currently on | Windows does not render an inactive virtual desktop; there are no pixels anywhere to read | Capture follows whatever desktop they switch to, instantly |
| Block **Ctrl+Alt+Del** | It is the Secure Attention Sequence, handled by the kernel and winlogon before any application or hook sees it. This is a deliberate anti-malware guarantee | Policy can remove Task Manager, Change Password and Sign Out from that screen, so the menu is useless |
| Block **Win+L** (lock) | Also reserved by winlogon, ahead of low-level hooks | A locked PC still shows as locked in the grid; the lock screen returns when they log back in |
| Capture **DRM-protected video** (Netflix, some players) | The GPU never hands protected frames to any capture path, by design of the content-protection chain | The window shows black in the thumbnail; the window title still identifies what it is |
| Capture a window that opted out of capture | `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` hides a window from every capture API, ours included | Rare outside password managers and banking apps |
| Stop a student **switching off or unplugging** the PC | No software runs on a powered-off machine | The PC shows as offline immediately, which is itself the signal |
| Control a PC that is **off** | — | Wake-on-LAN can power it on where the network supports it (planned) |
| Stop a student with **local administrator rights** from killing the agent | An administrator can stop any service; that is what administrator means | Do not give students admin. This is an organisational fix, not a software one |
| Stop booting **another OS from USB** | Nothing installed inside Windows can prevent a different OS from booting | BIOS password + Secure Boot + disk encryption, set by school IT |
| Record **macOS** screens without the user agreeing once | Apple's TCC requires explicit consent for Screen Recording; MDM can pre-stage but not silently grant it | A one-time approval per Mac, documented for IT |
| Capture on **Linux/Wayland** without consent | The compositor only shares the screen through a portal the user approves | A restore token makes it a one-time prompt per PC |
| Enforce the 10-device limit in a **self-compiled** build | The code is open source; anyone can delete the check | The limit lives in official builds and the hosted service — see AGENTS.md D1 |
| Capture the **UAC prompt / lock screen** from a normal user process | Those live on a separate secure desktop | The agent service (SYSTEM) with a session helper can, which is exactly why that design exists |
| Per-application audio on **older Windows** | Process loopback needs Windows 10 2004+ | Whole-PC audio, which is what a teacher actually wants |

### Possible, but we will not build it

| Wanted | Why we refuse |
|--------|---------------|
| Hidden or disguised agent, no tray icon, no notice | It is spyware then. It also gets the product flagged by antivirus, and in most jurisdictions monitoring without notice is unlawful. The visible indicator is decision D3 |
| Keylogging: recording everything a student types | Captures passwords and private messages far beyond a classroom's purpose. A screen thumbnail answers "are they working?" without building a credential harvester |
| Reading browser history or saved passwords | Same reason. Website *blocking* is done with the browser's own policy system, which needs neither |
| A remote shell / run-any-command feature | One bug or one stolen console key turns the whole lab into a botnet. Remote actions stay a fixed, typed list (AGENTS.md §5) |
| Watching a teacher's or employee's PC without them knowing | Same rule as students, and the same law |
| A single hardcoded master password | In an open-source project it is public the day it ships. Replaced by per-organisation codes and offline one-time codes (D10) |
| Silent install with no administrator involved | Anything that installs a SYSTEM service without admin is, by definition, a privilege-escalation exploit |

## What to build next, in value order

1. **Act on a PC**: power off/reboot, lock screen, block apps — the actual classroom control (#5, #4, #9).
2. **Policy engine** so those survive a reboot and work offline (#24, #6).
3. **Full-resolution view and remote control** (#1, #3).
4. **Exam mode**: lock everyone, broadcast, collect and wipe files (#13, #7, #14).
5. Then updates (#15), installer and role picker (#17), other platforms (#23), AI (#18–20).
