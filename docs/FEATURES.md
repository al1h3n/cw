# Feature status against the brief

Every capability named in [`../ClassWatcher.md`](../ClassWatcher.md), with an honest state. Updated
2026-09-14. Legend: **done** = built and tested · **partial** = some of it works · **planned** = designed
but no code.

**Summary: 12 of 27 done, 6 partial, 9 planned.** The product can now *watch* screens, *listen* to a PC, *drive* its mouse and keyboard, *lock* and
*power off* one PC or a whole room, *block* apps and games, *start and close programs*, *record* a
screen to a file, and *broadcast* the teacher's screen. What it still cannot do is run an exam: there
is no inescapable lock screen, no file collection, and no signed offline policy engine yet (blocking
and wallpaper persist on the Agent, but lock and power do not).

There is also a list of things that **cannot** be built at all, and things we **will not** build:
see [Impossible, and deliberately refused](#impossible-and-deliberately-refused) at the end.

## Free tier

| # | From the brief | State | Notes |
|---|----------------|-------|-------|
| 1 | Screen share, pin other monitors, virtual desktops | **done** | Every monitor is enumerated and selectable per PC. The grid uses change-only JPEG thumbnails at a teacher-chosen width; opening one screen now gives a real **H.264 video stream** on its own QUIC uni-stream. Measured on the dev PC: 720p30 and 1080p30 both at a true 30.2 fps, every packet decoded, and about 57–78 kbit/s on a near-idle screen — a full 1080p stream for roughly what one 320×180 JPEG thumbnail used to cost. Inactive virtual desktops cannot be captured by anyone — see *Impossible*. |
| 2 | Audio share, on/off switch | **done** | WASAPI loopback of what the student hears, downmixed to mono and decimated to ~16 kHz (~256 kbit/s). Off until asked for, and exclusive: one PC at a time. Verified end to end — 16 kHz mono, 64 000 samples for 4.0 s, loud while a tone played and exactly 0 in silence. Raw PCM for now; Opus would cut it to ~32 kbit/s when several PCs need listening at once. |
| 3 | Remote mouse and keyboard, Win key captured, exit chord | **done** | Move, click, scroll, keys and direct Unicode. Positions travel as screen *fractions*, so a teacher on 1080p driving a student on 1440p (or a scaled display, or a second monitor) lands where they meant. Input is dropped unless the teacher has explicitly taken control, and taking or releasing it is audit-logged. Win goes to the remote PC; plain Esc still reaches the remote program; **Ctrl+Alt+Esc** hands the keyboard back; every held modifier is released when control ends. Verified live: refused before consent, then a click focused Notepad and the typed text read back exactly. Still missing: the low-level hook that stops the *local* PC reacting too (the decision function is written and unit-tested; the hook needs a physical-keyboard check). |
| 4 | Lock screen like parental controls | **partial** (built, unverified) | Two locks now: the standard Windows lock (Win+L), and **exam lockdown** — a fullscreen message window on a **separate Win32 desktop** (`CreateDesktopW` + `SwitchDesktop`, spike 0.8) that the student cannot Alt+Tab or Win-key away from; the teacher starts/ends it. Built but not yet verified live (it seizes the desktop, so it needs a VM/second machine). Honest limits: **Ctrl+Alt+Del** always reaches Winlogon, and Task Manager is not yet disabled — locking those down needs the policy engine (`DisableTaskMgr`). A per-PC custom lock UI (#10) builds on this. |
| 5 | Shut down / power off all or one PC | **done** | Shut down, restart, sign out and cancel, for one PC or the whole room, with a Now / 1 min / 5 min warning. Windows shows its own localised countdown; an immediate shutdown force-closes programs. Offline PCs are never queued. Every action is audit-logged (D3). **Wake-on-LAN** is built too: the Console (or an awake Agent in the same room) broadcasts a magic packet for a PC's stored MAC; the PC wakes if its BIOS/UEFI has WoL enabled (a one-time IT setting). Verified live: countdown started+cancelled; a wake packet broadcast on the LAN. |
| 6 | Constant wallpaper nobody can change | **built, needs the service (which now exists)** | The `NoChangingWallPaper` policy is now enforced by the Agent **service** (`cowatcher-agent install` / `run`), which runs as LocalSystem and so can write the admin-only Policies hive. Installing the service needs one elevated command; verifying the whole install→boot→enforce path needs an admin machine or VM (the elevation gate is unchanged). A *specific* school image still needs file transfer. |
| 7 | Keep student files temporarily, wipe with one button, host can browse | planned | Baseline + diff design; wipe must be scope-proven by tests. |
| 8 | Add computers by ID or LAN; changes apply when back online | **done** (ID/LAN) / planned (offline queue) | Pairing by key with a 6-digit code, mDNS on LAN, reconnect by key after an IP change. Device IDs are six characters (`K7M2Q9`) **derived from the public key**, so no central table allocates them and two devices can never race for one. A paired PC **joins a room** and cannot leave without the room password (Argon2id-hashed on the PC). The offline mailbox (D9) is not built. |
| 9 | Enable/disable programs, editable games list, **app launcher** | **done** (apps) / planned (websites) | A room-wide, editable list of program names (`steam.exe`, `roblox.exe`, …) closes those programs on every connected PC within a second and closes them again if a student reopens them. Matching is by exact file name so a rule never kills an unrelated app, system-critical processes are protected, and the list is saved on the Agent so it keeps enforcing after a reboot with no network (D9). Verified live: Notepad closed within 1 s and stayed closed until the rule was cleared. Website blocking via browser policy files is still planned. Also a launcher: each PC publishes its own Start Menu and a teacher starts an entry by id or closes a running program by pid. The Console never sends a path, so "launch an app" can never become "run anything", and system-critical processes are never even offered as closable. |
| 10 | Custom lock screen: background, per-PC shortcuts, terminal with `unlock` and power commands | planned | Depends on #4. |
| 11 | Black background while the host watches | **done** | The Agent swaps the wallpaper for black when a teacher opens the PC's screen and restores it on close/disconnect (unelevated SPI). Restore is guarded four ways (on close, on disconnect, on start-up, on Drop) so a student is never left with a black desktop. Verified live via `wallpaper-selftest`. |
| 12 | Screen recordings, all or one, scheduled | **done** (manual) / planned (scheduled, two-pass) | Records on the student PC at a chosen size and frame rate, e.g. a 1440p screen saved as 1080p, area-averaged so text stays readable. When an `ffmpeg.exe` is present next to the Agent (or on PATH) it encodes real video — **H.264/H.265/AV1** with a chosen preset, CRF quality, B-frames and the lanczos scaler — to an `.mp4`; without it, the built-in **MJPEG-in-AVI** writer is used (every frame independent, index rewritten every 30 frames, so a power cut still leaves a playable file, and the **measured** fps is written so it plays at normal speed). The teacher can **download** any recording to their own PC (streamed over a QUIC uni-stream, path-traversal guarded). Still planned: **two-pass** encode (needs a post-record re-encode, not a live pipe) and scheduling from a Policy. |
| 13 | Broadcast the host screen to all/some, input blocked | **partial** | The teacher's screen appears full-screen and on top on the student PC, scaled to whatever resolution that PC has, and disappears on command. Input is **not** blocked yet: Alt+Tab and the Windows key still work, because trapping a session needs the separate Win32 desktop from spike 0.8 — that is #4/#10. So today this is "everyone look at my screen", not "nobody can do anything else". |
| 14 | Send audio/video in real time, notification, play once | planned | Preload + synchronised start beats live streaming for exams. |
| 15 | Update centre from the deploy branch | planned | Signed manifests, channels, rollback. |
| 16 | Tutorial on first start, skippable | **done** | A five-step tour on first run — add PCs, watch, open one screen, take control, room password — skippable from every step and re-openable from the Help button. |
| 17 | One binary, choose client or server, ID added later | **partial** | Two binaries today. The agent can install itself as an auto-start Windows service (`install`/`uninstall`/`status`/`run`): starts at boot, absent from Task Manager's Startup tab (as every service is), removable only by an admin — not hidden (still in services.msc, Details, tray, login notice). A single installer with a role picker is still the plan (D8). |

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
  `proto` — no arbitrary command execution (D-rules). Its own crate and milestone.

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
