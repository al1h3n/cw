# Feature status against the brief

Every capability named in [`../ClassWatcher.md`](../ClassWatcher.md), with an honest state. Updated
2026-09-23. Legend: **done** = built and tested · **partial** = some of it works · **planned** = designed
but no code. "Built, unverified live" means the code exists and its pure parts are tested, but the
disruptive on-a-real-student-PC behaviour still needs the two-machine/VM checklist
([`TEST-CHECKLIST.md`](TEST-CHECKLIST.md)).

**Summary: ~14 of 27 done, ~7 partial, ~6 planned.** The product can now *watch* screens, *listen* to
a PC, *drive* its mouse and keyboard (with a real local keyboard grab, VirtualBox-style Right-Ctrl
release), *lock* and *power off* one PC or a whole room, *freeze* a student's own input, *block* apps
and games, *start and close programs*, *push and lock the wallpaper* (per-PC or class-wide), *record* a
screen, *screenshot* one, and *broadcast* the teacher's screen (optionally locking students onto it on
a separate desktop). It also runs several **classrooms** side by side, serves the same UI in a
**browser** (`cowatcher-console web`), has **Settings** (AI toggle, theme) and an in-Console **AI
assistant (Surey)**. What still gates a real exam: the secure-desktop cluster (login/lock/UAC screens,
Ctrl+Alt+Del) needs a SYSTEM Winlogon helper and a driver-free policy lockdown (D23) verified on a VM;
file collection/wipe (#7) and the signed offline **policy engine** (D9) are not built, so lock, power
and exam do not yet survive a reboot the way blocking and pushed wallpaper do.

There is also a list of things that **cannot** be built at all, and things we **will not** build:
see [Impossible, and deliberately refused](#impossible-and-deliberately-refused) at the end.

## Free tier

| # | From the brief | State | Notes |
|---|----------------|-------|-------|
| 1 | Screen share, pin other monitors, virtual desktops | **done** | Every monitor is enumerated and selectable per PC. The grid uses change-only JPEG thumbnails at a teacher-chosen width; opening one screen now gives a real **H.264 video stream** on its own QUIC uni-stream. Measured on the dev PC: 720p30 and 1080p30 both at a true 30.2 fps, every packet decoded, and about 57–78 kbit/s on a near-idle screen — a full 1080p stream for roughly what one 320×180 JPEG thumbnail used to cost. Inactive virtual desktops cannot be captured by anyone — see *Impossible*. |
| 2 | Audio share, on/off switch | **done** | WASAPI loopback of what the student hears, downmixed to mono and decimated to ~16 kHz (~256 kbit/s). Off until asked for, and exclusive: one PC at a time. Verified end to end — 16 kHz mono, 64 000 samples for 4.0 s, loud while a tone played and exactly 0 in silence. Raw PCM for now; Opus would cut it to ~32 kbit/s when several PCs need listening at once. |
| 3 | Remote mouse and keyboard, Win key captured, exit chord | **done** | Move, click, scroll, keys and direct Unicode. Positions travel as screen *fractions*, so a teacher on 1080p driving a student on 1440p (or a scaled display, or a second monitor) lands where they meant. Input is dropped unless the teacher has explicitly taken control, and taking or releasing it is audit-logged. Win goes to the remote PC; plain Esc still reaches the remote program; every held modifier is released when control ends. The native viewer now installs a real low-level keyboard grab (`platform::keygrab`, `WH_KEYBOARD_LL`): while controlling it swallows every key locally and forwards it (Windows key, Alt+Tab, Ctrl+Esc, Alt+F4), and **Right Ctrl** takes/releases control (VirtualBox convention — the old `Ctrl+Alt+Esc` never worked because `Alt+Esc` is a shell shortcut the OS ate first). The grab is armed only while the viewer is focused, so it never steals keys from other apps. Verified live: refused before consent, then a click focused Notepad and the typed text read back exactly. **Still unverified live:** the Right-Ctrl grab and Win-key forwarding on a second machine. Ctrl+Alt+Del / Win+L stay OS-reserved (see *Impossible*). |
| 4 | Lock screen like parental controls | **partial** (built, unverified) | Two locks now: the standard Windows lock (Win+L), and **exam lockdown** — a fullscreen message window on a **separate Win32 desktop** (`CreateDesktopW` + `SwitchDesktop`, spike 0.8) that the student cannot Alt+Tab or Win-key away from; the teacher starts/ends it. Built but not yet verified live (it seizes the desktop, so it needs a VM/second machine). A `platform::keyguard` `WH_KEYBOARD_LL` hook is installed while the lock is up, dropping Alt+Tab, Alt+Esc, Ctrl+Esc, the Windows keys and Alt+F4 (plus a `SC_CLOSE` block), and a watchdog re-asserts the lock desktop after a Win+L cycle. Honest limits (D23): **Ctrl+Alt+Del** and **Win+L** always reach Winlogon (Secure Attention Sequence, unblockable from user space), and Task Manager is not yet stripped — the driver-free path is toggling documented registry policies (`DisableTaskMgr`, `DisableLockWorkstation`, `NoLogoff`, …) at lock start, which is written down but not yet wired. The **secure-desktop cluster** (seeing/typing the login-lock password screen, showing overlays on the lock screen, Ctrl+Alt+Del re-enabling host control) needs a SYSTEM helper bound to the Winlogon desktop. A per-PC custom lock UI (#10) builds on this. |
| 5 | Shut down / power off all or one PC | **done** | Shut down, restart, sign out and cancel, for one PC or the whole room, with a Now / 1 min / 5 min warning. Windows shows its own localised countdown; an immediate shutdown force-closes programs. Offline PCs are never queued. Every action is audit-logged (D3). **Wake-on-LAN** is built too: the Console (or an awake Agent in the same room) broadcasts a magic packet for a PC's stored MAC; the PC wakes if its BIOS/UEFI has WoL enabled (a one-time IT setting). Verified live: countdown started+cancelled; a wake packet broadcast on the LAN. |
| 6 | Constant wallpaper nobody can change | **built, needs the service (which now exists)** | The `NoChangingWallPaper` policy is now enforced by the Agent **service** (`cowatcher-agent install` / `run`), which runs as LocalSystem and so can write the admin-only Policies hive. Installing the service needs one elevated command; verifying the whole install→boot→enforce path needs an admin machine or VM (the elevation gate is unchanged). A *specific* school image still needs file transfer. |
| 7 | Keep student files temporarily, wipe with one button, host can browse | **done** | A shared per-student **workspace** folder holds files for the lesson. The Console's **Files** dialog browses it, **collects** work (download one, or **Collect all**), **deletes a chosen file**, and **wipes everything with one button**, and marks a **baseline** so new/changed files are flagged — the diff is **size-based, never time-based**, so a student changing the clock/timezone cannot fool it. Delete and wipe are **scope-proven by unit tests** (AGENTS §5): confined to the workspace, escaping paths (`..`, absolute, drive prefix) refused, and the wipe never follows a symlink out of the folder. Same tools on the MCP and Surey. Not built: restoring *modified* files (needs copy-on-write backups — deferred, see PLAN 2.8). |
| 8 | Add computers by ID or LAN; changes apply when back online | **done** (ID/LAN) / planned (offline queue) | Pairing by key with a 6-digit code, mDNS on LAN, reconnect by key after an IP change. Device IDs are six characters (`K7M2Q9`) **derived from the public key**, so no central table allocates them and two devices can never race for one. A paired PC **joins a room** and cannot leave without the room password (Argon2id-hashed on the PC). The offline mailbox (D9) is not built. |
| 9 | Enable/disable programs, editable games list, **app launcher** | **done** (apps) / planned (websites) | A room-wide, editable list of program names (`steam.exe`, `roblox.exe`, …) closes those programs on every connected PC within a second and closes them again if a student reopens them. Matching is by exact file name so a rule never kills an unrelated app, system-critical processes are protected, and the list is saved on the Agent so it keeps enforcing after a reboot with no network (D9). Verified live: Notepad closed within 1 s and stayed closed until the rule was cleared. Website blocking via browser policy files is still planned. Also a launcher: each PC publishes its own Start Menu and a teacher starts an entry by id or closes a running program by pid. The Console never sends a path, so "launch an app" can never become "run anything", and system-critical processes are never even offered as closable. |
| 10 | Custom lock screen: background, per-PC shortcuts, terminal with `unlock` and power commands | planned | Depends on #4. |
| 11 | Black background while the host watches | **done** | The Agent swaps the wallpaper for black when a teacher opens the PC's screen and restores it on close/disconnect (unelevated SPI). Restore is guarded four ways (on close, on disconnect, on start-up, on Drop) so a student is never left with a black desktop. Verified live via `wallpaper-selftest`. |
| 12 | Screen recordings, all or one, scheduled | **done** (manual) / planned (scheduled, two-pass) | Records on the student PC at a chosen size and frame rate, e.g. a 1440p screen saved as 1080p, area-averaged so text stays readable. When an `ffmpeg.exe` is present next to the Agent (or on PATH) it encodes real video — **H.264/H.265/AV1** with a chosen preset, CRF quality, B-frames and the lanczos scaler — to an `.mp4`; without it, the built-in **MJPEG-in-AVI** writer is used (every frame independent, index rewritten every 30 frames, so a power cut still leaves a playable file, and the **measured** fps is written so it plays at normal speed). The teacher can **download** any recording to their own PC (streamed over a QUIC uni-stream, path-traversal guarded). Still planned: **two-pass** encode (needs a post-record re-encode, not a live pipe) and scheduling from a Policy. |
| 13 | Broadcast the host screen to all/some, input blocked | **partial** (locked mode built, unverified live) | The teacher's screen appears full-screen and on top on the student PC, scaled to whatever resolution that PC has, and disappears on command. Several broadcasts can run at once to different PC groups, each with its own Stop, and they survive closing the picker. The teacher can tick **Lock students onto it**: a locked broadcast shows on a **separate Win32 desktop** (`platform::present::open_locked`, plus `platform::keyguard` dropping Alt+Tab/Alt+Esc/Ctrl+Esc/Win/Alt+F4) so an ordinary user cannot Alt+Tab or Win-key away — built, but the on-student lockdown itself is best confirmed on a second machine. Unlocked mode is still "everyone look at my screen". A busy-cursor spinner over the broadcast was fixed (the present window now sets a normal arrow cursor). |
| 14 | Send audio/video in real time, notification, play once | **done** (audio) / planned (video) | **Listening exam:** the teacher preloads an **audio** file to the chosen PCs (stored privately, not the shared workspace) and starts it on all of them **together**, with an optional on-screen notice + lock, **no controls**, and it is **deleted after it plays once**. Sync never trusts the student's clock — each PC starts a fixed delay after it *receives* the play command, so a changed timezone cannot desync it (the Feature-5 rule). Playback is Windows MCI (`platform::audio`, wav/mp3/wma), the lock overlay reuses the exam lock. On the MCP (`preload_media`/`play_media`/`stop_media`) and Surey. **Video** play-once is not built (needs a real fullscreen player); live teacher mic/screen reuses the broadcast path (#13). |
| 15 | Update centre from the deploy branch | planned | Signed manifests, channels, rollback. |
| 16 | Tutorial on first start, skippable | **done** | A five-step tour on first run — add PCs, watch, open one screen, take control, room password — skippable from every step and re-openable from the Help button. |
| 17 | One binary, choose client or server, ID added later | **partial** | Still three binaries (Console, Agent, Viewer), but a single **installer** (`installer/cowatcher.iss`, Inno Setup) now bundles all three and picks the role at first run (D8) — Console = GUI + shortcut, Agent = the auto-start Windows service. The agent can install itself as an auto-start service (`install`/`uninstall`/`status`/`run`): starts at boot, copies itself to `%ProgramData%\co-watcher\agent` first so deleting the source folder cannot break it, absent from Task Manager's Startup tab (as every service is), removable only by an admin — not hidden (still in services.msc, Details, tray, login notice). A true single fat binary with a role flag is still the longer-term plan. |

## Paid tier (AI)

| # | From the brief | State | Notes |
|---|----------------|-------|-------|
| 18 | AI lesson summaries | planned | — |
| 19 | Bring your own API key (Claude/ChatGPT/Gemini/DeepSeek), or a hosted model | **done** (BYOK) | **Surey**, the in-Console assistant (D22): OpenAI, Anthropic native, any OpenAI-compatible endpoint, and local Ollama/LM Studio, with a "list models" helper. The key is sealed with DPAPI and every request is proxied through Rust, never the web layer. Voice input has three transcription modes (chat / custom Whisper endpoint / local program). The hosted-model option waits on the Hub. |
| 20 | AI watches the screen and acts (close games etc.) | **partial** | Surey can already *act* on the class through the same typed fleet tools the MCP exposes (list/close apps, blocklist, exam, power, recording, `set_wallpaper`), with an `ask_user` selection tool and confirmation of destructive actions — driven by the teacher, not yet autonomous. Autonomous *watching* (the cost ladder: rules → change gate → OCR text → local classifier → batched vision) is still planned. |
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


## Storage: why SQLite, and where ScyllaDB would actually fit

A fair question came up: should the device list live in SQL, and is ScyllaDB worth using because it
handles very large numbers of devices well?

**Today there is no database at all**, and that is deliberate rather than lazy. The state each side
keeps is tiny and is read whole every time: an Agent has a device key, a trust store, a room file and
a blocklist; a Console has its key, its trust store, a room file and a blocklist. Plain files with
atomic writes are the right tool for a few kilobytes that are always read in full.

**SQLite (AGENTS.md D14) is the next step, not Postgres or Scylla.** It earns its place the moment we
need to *query* rather than *load*: searching an audit log, indexing recordings, or the Hub's
directory of devices. It is one file, needs no server, and a school's IT can copy it as a backup.

**ScyllaDB is a cluster database.** It is excellent at what it does — a wide-column store, Cassandra
compatible, built for millions of operations a second spread over many machines. Every one of those
properties is aimed at a problem we do not have:

* A Console manages one room, tens of devices. A Hub for an entire school district is still in the
  thousands of rows, with traffic measured in a few writes per device per lesson.
* **D16 says the Hub must be self-hostable by a school.** "Run this one binary" is a realistic ask of
  a school's IT; "operate a Scylla cluster" is not. Adopting it would make self-hosting the privilege
  of organisations with a database team, which contradicts the plan.
* Scylla trades away joins, transactions and ad-hoc queries for scale. We would pay that price
  immediately and collect the benefit at a size we may never reach.

So it goes on the planned list with an honest trigger rather than being dismissed or adopted on
faith. **We would move the hosted Hub to a clustered store when a measurement says so** — concretely,
when a single Postgres/SQLite node can no longer keep up with audit-event ingestion, which for the
shape of data here means somewhere north of a hundred thousand actively reporting devices. At that
point the sensible ladder is SQLite → Postgres → (Scylla or similar) for the *hosted* Hub only, while
self-hosted Hubs stay on the single-file option forever. Writing the storage layer behind a narrow
interface now is what keeps that door open, and costs nothing today.

| Where | Now | Next | Only if a measurement demands it |
|-------|-----|------|----------------------------------|
| Agent | files (key, trust, room, blocklist) | SQLite for the audit log and recording index | — |
| Console | files + recordings list | SQLite for device names, rooms, audit history | — |
| Hub (self-hosted) | not built | SQLite, one file | Postgres for a large district |
| Hub (our hosted service) | not built | Postgres | ScyllaDB/Cassandra at 100k+ reporting devices |

## What to build next, in value order

1. **Verify + harden exam lockdown** (#4/#10): the separate-desktop lock is built — verify it on a VM,
   then disable Task Manager (`DisableTaskMgr` policy) and add an allow-list of apps that may run on
   the lock desktop, turning "freeze the PC" into a real exam environment.
2. **Policy engine** (2.1) so lock, power and wallpaper survive a reboot the way blocking already
   does, signed and enforced offline (#24, #6).
3. **The D3 indicator trio**: tray icon, "being viewed" badge, un-disable-able login notice — a
   release blocker (legal notice + lower antivirus/RAT flagging), currently entirely missing.
4. **Exam mode** on top of the lock screen: collect and wipe student files (#7), play media once (#14).
5. Then updates (#15), installer and role picker (#17), other platforms (#23), AI (#18–20).

**Landed since this list was written:** the SYSTEM service + per-session helper (1.4b) — the Agent
installs as an auto-start service that launches a capture helper into the logged-in student's session;
and ffmpeg-based recording with download-to-teacher (#12). Elevation paths still need VM verification
(`docs/TEST-CHECKLIST.md §8`).

### Done 2026-09-23 (exam timer, wallpaper reset, classroom focus)

`PROTOCOL_VERSION` is now **20**.

- **Timed exam lock (bug: "lock for 10 s locked forever").** `SetExam` gained a `duration_seconds`; the
  **Agent** enforces the auto-release timer itself (a generation-guarded thread), so a lock started with
  a duration ends even if the Console — or the AI/MCP process that started it — has since exited. Exposed
  on the MCP `set_exam` tool, Surey's `set_exam`, and the console command (`0` = until released, the old
  behaviour, which the teacher UI still uses).
- **Reset wallpaper to default (bug: "AI couldn't set the wallpaper back to default").** A new
  `ResetWallpaper` action puts the wallpaper back to the student's **own** captured wallpaper, or the
  **Windows default** image when none was captured, and forgets every override — so it works even when
  nothing was locked (unlike unlock, which only reverts an override). In the Console Restrictions menus
  (main + opened PC), on the MCP `perform_action` and Surey's `power_action` (`reset-wallpaper`).
- **Switching to an open classroom focuses it (bug: opened a second window).** Each console records its
  window handle for its classroom; `switch_classroom` focuses that window instead of spawning a second
  process when one is already open (`platform::window::focus`, stale handles validated with `IsWindow`).

### Done 2026-09-24 (feature 14 — synchronised play-once media exam)

`PROTOCOL_VERSION` is now **23**.

- **Feature 14 (audio).** New wire: `PreloadMedia`/`MediaReady`/`MediaPreloaded` (bytes on a uni-stream,
  stored privately), `PlayMedia`/`StopMedia`/`MediaState`. `agent::exam_media` schedules playback a
  **relative** delay after the play command is *received* (never the wall clock), plays it once via
  Windows MCI (`platform::audio`), shows the exam-lock overlay as the notice + no-controls when `lock`,
  and **deletes the file after it plays**. Console **Media exam** dialog (Content menu): pick an audio
  file, targets, "start in N seconds", notice, lock; Play preloads to all then starts them together,
  Stop ends it. On the MCP (`preload_media`, `play_media`, `stop_media`) and Surey. Video play-once is
  deferred (needs a fullscreen player). Chose MCI over an audio crate to avoid a second `cpal`/`alsa`
  stack conflicting with the capture path's `cpal` on the `alsa` native `links`.

### Done 2026-09-23 (feature 7 — student file workspace: collect + wipe)

`PROTOCOL_VERSION` was **22**.

- **Feature 7 built** on the workspace from the file-transfer batch. New wire: `ListWorkspace`/
  `WorkspaceManifest` (recursive file list), `DeleteFile`/`FileDeleted` (remove a chosen file),
  `ClearWorkspace`/`WorkspaceCleared` (wipe everything). `agent::workspace` gains `manifest`, `delete`
  and `clear`, all confined to the workspace and **unit-tested for scope safety** — delete/clear refuse
  escaping paths and the wipe removes symlinks as links rather than following them out of the folder
  (AGENTS §5). Console **Files** dialog now has per-file **Delete**, **Delete all**, **Collect all**,
  and **Mark baseline** with New/Changed badges (size-based diff, clock-independent). Exposed on the MCP
  (`list_workspace`, `delete_file`, `clear_workspace`) and Surey, both flagging the destructive ones.

### Done 2026-09-23 (file transfer)

`PROTOCOL_VERSION` was **21**.

- **File transfer to/from student PCs — built.** A **Files** entry in the Console's Content menu browses
  a student's shared **workspace** folder, **sends** a file into it (any directory under it, created if
  needed) and **downloads** a file the student made (e.g. a Python script to assess). Transfer is
  confined to that one folder on the Agent: `crates/agent/src/workspace.rs` refuses any path that
  escapes it — absolute paths, a drive prefix, or a `..` component — with a canonicalised re-check for
  symlinks, and the guard is unit-tested (AGENTS §5). Wire: `ListFiles`/`Files`,
  `FetchFile`/`FileTransfer` (download on a uni-stream, like a recording), `SendFile`/`FileSendReady`/
  `FileSent` (upload on a uni-stream). Exposed to the AI on the **MCP** (`list_files`, `fetch_file`,
  `send_file`) and Surey. The workspace root defaults to `Co-watcher` in the student's profile, override
  with `COWATCHER_WORKSPACE`.
- **Feature 7** (collect + delete student files, choosing specific ones) and **feature 14** (preloaded,
  synchronised play-once media) now have the channel they needed; each is still its own batch — feature
  7 adds *delete* (scope-proven, choose specific files) on top of the browse/download already here.

### Done 2026-09-22 → 2026-09-23 (newest)

Fast follow-ups from live two-machine testing. `PROTOCOL_VERSION` is now **19** (added `FetchRunningIcon`
= 18, then a `fit` on `SetWallpaper` plus `SetScreenLock`/`ScreenLockState` = 19).

- **Multiple classrooms.** A classroom is a separate profile directory (own identity, paired devices,
  room password, blocklist). A header switcher lists them; "New classroom" opens it in its own Console
  window (`switch_classroom` spawns `cowatcher-console --classroom <slug>`). CLI: `classrooms`,
  `--classroom <slug>`.
- **`cowatcher-console web` — the same Svelte UI in a browser** at full parity (`crate::web`, axum +
  `rust-embed`). `lib/bridge.ts` makes every component transport-agnostic; a per-run random token
  (loopback by default) guards `/invoke` and `/events` (SSE). Native viewer + in-window classroom
  switching stay desktop-only and say so.
- **Settings: AI on/off + theme** (`crate::settings`, shared across classrooms). Turning AI off hides
  Surey and guarantees no AI request. Theme is dark / light / custom (all CSS variables). An **About**
  card credits the author and dependencies.
- **Cloud sync prepared (no server yet):** a secret-free `ClassroomSnapshot` and `SyncClient` against a
  subscription dashboard URL + licence key; returns a clear "not configured" until pointed at a hosted
  endpoint. Settings has a **subscription placeholder** (DPAPI-sealed licence, opens the dashboard).
- **Grouped toolbars + drag-to-reorder grid with named groups**, a right-click-anywhere action menu, a
  tile-size (zoom) slider, per-PC IP display once hole-punching promotes off the relay, and a resizable
  opened view.
- **Screen lock (freeze a student's own input without taking control)** — per-PC and "freeze all";
  released automatically if the Console disconnects (`SetScreenLock`).
- **Push wallpaper with a fit (fill/fit/stretch/centre/tile)**, per-PC from the opened view's
  Restrictions menu as well as class-wide; pushed wallpaper survives a reboot (sticky marker + reapply).
- **Wallpaper reverts to the student's own on unlock (bug 4) + a locked/unlocked indicator (feature 6):**
  `platform::wallpaper` snapshots the genuine original once and `revert()` restores it and forgets every
  override; the opened-PC menu shows the current lock state.
- **Screenshot a PC** to the teacher's PC at full resolution (`manager::screenshot`), and a **recording
  resolution guard** — recording refuses a size larger than the target screen; `record_all` is
  all-or-nothing and names every too-small PC.
- **Configurable voice transcription (`ai::transcribe`):** chat provider / custom Whisper endpoint /
  local program, so voice never forces a specific setup (the 404 came from assuming the chat endpoint
  also served Whisper). Endpoint key sealed with DPAPI.
- **Right-Ctrl control keybind** (see #3), **broadcast dead-audience timeout** (a rebooted client no
  longer leaves a broadcast "stuck on"), **Surey works in the background** (closing the panel hides it
  instead of unmounting, so a reply/tool loop keeps running and reopens where it was), Surey icon
  alignment, and **OS-agnostic blocklist suggestions** with real process names (Minecraft Java is
  `javaw.exe`, not "minecraft.exe") plus updated cross-platform reference catalogs.
- **Still open (each its own batch):** file send/receive to/from student PCs (#7 in spirit — new proto +
  path-traversal guards), a global-vs-local settings **override view** with reset and clock-independent
  recording retention, and the **secure-desktop cluster** (login/lock/UAC/Ctrl+Alt+Del — needs a SYSTEM
  Winlogon helper + a VM).

### Reported from live testing, queued (2026-09-16)

Fresh from two-machine use; not yet built unless noted.

- **AI chat panel** (not built): a button in the bottom-right of the Console opens a chat with
  **sessions** (multiple saved conversations), rendering text and an attachment tray like
  WhatsApp/Telegram — a small grid of files with icons, image previews, and a video's first frame,
  shown before sending. Talks to a **user-supplied OpenAI/Anthropic-compatible endpoint** (see D22):
  the request is proxied through Rust (keeps the key off the web layer and dodges CORS/CSP). Its own
  delivery — the biggest of the queued items.
- **Per-window live previews in the grid** (harder): show individual application windows as thumbnails.
  DWM live thumbnails (`DwmRegisterThumbnail`) cannot be copied to a bitmap for streaming; the workable
  path is `PrintWindow(PW_RENDERFULLCONTENT)` per top-level window on an interval, sent as small JPEGs.
  Its own capture path — scope as a separate round.
- **A Co-watcher MCP server** exposing the existing typed remote actions (screen check, launch/block
  apps, etc.) as MCP tools, for the future paid AI features. Must reuse the same signed-action enum in
  `proto` — no arbitrary command execution (D-rules). Its own crate and milestone. **Built** —
  `crates/mcp` (`cowatcher-mcp`), see *Done 2026-09-21 (later)* below.

### Bug fixes from live testing 2026-09-21

- **Grid drag was completely dead** — Tauri's OS drag-drop handler (`dragDropEnabled`, on by default)
  swallows HTML5 drag events inside WebView2. Set `dragDropEnabled: false` on the window so the
  drag-to-reorder grip works.
- **Locked broadcast/exam were escapable** — added `platform::keyguard` (a `WH_KEYBOARD_LL` hook that
  drops Alt+Tab, Alt+Esc, Ctrl+Esc, the Windows keys and Alt+F4), installed while a locked broadcast
  or exam is up, plus a `WM_SYSCOMMAND`/`SC_CLOSE` block so Alt+F4 cannot close the window.
  **Win+L and Ctrl+Alt+Del cannot be blocked from user space** (Secure Attention Sequence) — the
  watchdog re-asserts the lock desktop after a Win+L unlock, which is the documented best effort.
- **Broadcast could be buried by clicking another window** — the presenter now re-asserts `HWND_TOPMOST`
  on its watchdog tick (unlocked mode) without stealing focus.
- **Minimizing the shared window turned clients black** — `media::window_capture` now reports a
  minimized window (`IsIconic`) as uncapturable, so the broadcaster keeps showing the last good frame
  instead of a black one.
- **Some windows broadcast only their title bar** — when `PrintWindow` leaves the client area blank,
  capture falls back to a screen-region `BitBlt` (correct for a foreground, unobscured window). The
  complete fix for occluded/GPU windows is Windows.Graphics.Capture, noted as a later change.
- **No sign when a client dropped the broadcast** — the fan-out now emits `cowatcher://broadcast-ended`
  when a PC that was showing stops, and the Console toasts "<PC> closed the broadcast".

### Done 2026-09-21 (latest)

- **Surey — the in-Console AI assistant (D22).** A bottom-right **Ask Surey** button opens a floating,
  draggable, dockable panel (`crates/console/ui/src/lib/SureyPanel.svelte`): float / dock-left /
  dock-right, resizable, with multiple chat sessions and its placement persisted in `localStorage`.
  - **Providers.** Official OpenAI, official Anthropic (native `/v1/messages`), any custom
    OpenAI-compatible endpoint (the Omniroute example), and **local** servers (Ollama/LM Studio) via
    the OpenAI-compatible shape, with a "list models" helper (`GET /v1/models`). All traffic is proxied
    through Rust (`crates/console/src/ai/{provider,client,tools,mod}.rs`); the API key is sealed with
    DPAPI (`platform::secret`) and never reaches the web layer (D22).
  - **Acting on the class.** Surey uses the same typed fleet tools as the MCP (`ai::tools` →
    `DeviceManager` → closed `proto::Action`): `list_devices`, `power_action`, `set_exam`, `list_apps`,
    `launch_app`, `list_running`, `close_app`, `set_blocklist`, recording controls. No arbitrary-command
    tool. The pre-prompt carries a live device snapshot so it can map "PC 4,5" to ids; it confirms
    destructive actions.
  - **Selection tool (`ask_user`).** The interaction primitive the AI drives: it presents options the
    panel renders as a list chosen by number key, arrows+Enter, or mouse, with an optional free-form
    answer. The Rust side round-trips via a per-request channel; one choice can lead to the next.
  - **Voice.** Real-time speech-to-text via the WebView Speech API (interim transcript straight into the
    box, Jarvis-style), falling back to record-then-transcribe through the endpoint's Whisper-shape
    `/audio/transcriptions`.
  - Backend unit-tested (provider round-trip incl. DPAPI, OpenAI/Anthropic wire shapes, tool schemas,
    argument parsing). Live LLM calls need a real endpoint; not exercised in CI. **Follow-ups:**
    attachments (images/files) in the composer, and streaming token output.

### Done 2026-09-21 (later)

- **Co-watcher MCP server** (`crates/mcp`, binary `cowatcher-mcp`). A Model Context Protocol server
  (JSON-RPC 2.0 over stdio) that lets an AI client drive the classroom through the same typed, signed
  path the Console uses. Tools: `list_devices`, `device_status`, `screen_thumbnail` (returns an
  image), `list_apps`, `list_running`, `launch_app`, `close_app`, `set_blocklist`, `perform_action`
  (shutdown/reboot/log-off/lock-screen/cancel-shutdown/lock-/unlock-wallpaper), `set_exam`,
  `set_wallpaper`, `recording_status`, `start_recording`, `stop_recording`, `list_recordings`. Each
  maps onto a `net::ControlSession` method and thus the closed `proto::Action` enum — there is **no**
  arbitrary-command tool. The operator brief (`crates/mcp/SKILL.md`) is served once as the
  `initialize` `instructions` (and as a prompt `cowatcher_operator` + resource `cowatcher:///skill`),
  so the skill is sent ahead of prompts without being re-spent each turn. It reuses the Console's data
  dir (`COWATCHER_DIR` / `--data-dir`) and depends only on `net`+`proto` (no OS code — the pinned
  separation rule). Verified end-to-end over stdio (initialize → tools/list → tools/call) plus 11
  unit tests. Ships in the release build and, teacher-side, in the installer.

- **Push a desktop wallpaper to students.** A **Wallpaper** Console button opens `WallpaperDialog`:
  pick an image (PNG/JPEG/BMP), choose one/selected/all connected PCs, apply. The bytes travel as
  `Control::SetWallpaper` and the Agent writes them beside its data dir and points the desktop at the
  image (`platform::wallpaper::set_image`). If a teacher is watching (wallpaper blacked out), the new
  image is recorded as the wallpaper to restore so it appears when watching ends rather than fighting
  the black-out. Windows only for now (Linux/macOS stubs — see `docs/PLATFORMS.md`). `PROTOCOL_VERSION`
  is 16. Format detection + the black-bitmap are unit-tested; the live registry/SPI paths need a
  student PC (per `docs/TEST-CHECKLIST.md`).

### Done 2026-09-21 (this session)

- **Host-wide recordings panel.** A **Recordings** button opens a live view of every PC's recording
  state (a pulsing ● while a PC is recording, with its frame count), plus **Record all** / **Stop all**
  across the watched class, and **Download all (.zip)** — every stored recording is fetched to the
  teacher's PC and bundled into one Stored (uncompressed) archive laid out as `<device>/<file>`
  (`download_all_recordings_zip`, using the `zip` crate). Per-PC download already existed
  (`FetchRecording`); this adds the class-wide overview, bulk start/stop, and the single archive.
  Console-only, so `PROTOCOL_VERSION` stays 15. (Recording ops require a *watched* PC, since only a
  live control session answers — the panel shows unwatched PCs greyed out.)


- **Full-screen broadcast with a source picker + student lockdown.** A Zoom/Teams-style picker
  (`list_broadcast_sources`) lists every monitor and every ordinary app window with a thumbnail; the
  teacher picks one, chooses which PCs, and optionally ticks **Lock students onto it**. A locked
  broadcast shows on a **separate Win32 desktop** (`platform::present::open_locked`, same mechanism as
  the exam lock, with the by-name restore + watchdog) so Alt+Tab / Win / Ctrl+Esc do nothing.
  Window capture is `media::window_capture` (`EnumWindows` + `PrintWindow(PW_RENDERFULLCONTENT)`).
  `PROTOCOL_VERSION` = 15 (`ShowBroadcast` gained a `locked` flag). Source capture verified locally
  (`cargo run -p media --example broadcast_sources`) and the locked desktop restore
  (`cargo run -p platform --example present_locked_smoke`); the on-student lockdown itself is best
  confirmed on a second machine.
- **App icons on the binaries:** the Console and viewer use the "Host" icon, the Agent the "Client"
  icon (embedded via `winresource`; Tauri embeds the Console's from `tauri.conf.json`).

### Cross-platform status

`docs/PLATFORMS.md` now records, per feature, what works on Windows vs. the Linux/macOS stubs, and what
each of those platforms will need in Phase 5. Short version: capture, input, power, broadcast and
lockdown are Windows-only today; the portable crates (proto, net, codecs, UI) are unverified elsewhere.

### Done 2026-09-20 (this session)

- **Exam + Win+L no longer leaves a bare desktop.** `platform::examlock` now restores the student's
  desktop by resolving **Default** by name with a retry loop (not the possibly-stale captured handle),
  and a watchdog timer re-asserts the lock after a Win+L/unlock. Verified with a live start/stop smoke
  test (`crates/platform/examples/examlock_smoke.rs`); the watchdog path is by inspection.
- **Lazy program icons** ship: `Control::FetchAppIcon`/`AppIcon` carry raw BGRA (no image crate on the
  Agent), `platform::apps::icon_bgra` reads the shell icon via `SHGetFileInfoW` + GDI, and the Console
  paints them to a canvas, requested per visible row (IntersectionObserver). `PROTOCOL_VERSION` = 14.

### Fixed 2026-09-16 (this session)

- Pairing panel would not open — Tauri denied the frontend `event|listen` because the Console shipped
  with no capabilities file. Added `capabilities/default.json` (`core:default`).
- Focused-view toolbar overflowed with long localized labels, pushing buttons off-screen; it now wraps.
