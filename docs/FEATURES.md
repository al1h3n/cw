# Feature status against the brief

Every capability named in [`../ClassWatcher.md`](../ClassWatcher.md), with an honest state. Updated
2026-09-14. Legend: **done** = built and tested · **partial** = some of it works · **planned** = designed
but no code.

**Summary: 4 of 27 done, 4 partial, 19 planned.** The product can currently *watch* screens. It cannot
yet *act* on a student PC (no lock, no blocking, no power, no remote control) — that is the next slab
of work and is what makes it worth a teacher's time.

## Free tier

| # | From the brief | State | Notes |
|---|----------------|-------|-------|
| 1 | Screen share, pin other monitors, virtual desktops | **partial** | Thumbnails of monitor 0 at ~1 fps over the real transport. Missing: full-resolution live stream (spike 0.5 proved the codec), picking a monitor, virtual desktops. |
| 2 | Audio share, on/off switch | planned | Design set (WASAPI loopback + Opus, off by default). |
| 3 | Remote mouse and keyboard, Win key captured, exit chord | planned | Spike 0.9 proved the keyboard hook; injection and forwarding not built. |
| 4 | Lock screen like parental controls | planned | Spike 0.8 proved a separate Win32 desktop + WebView2 lock UI. |
| 5 | Shut down / power off all or one PC | planned | Straight OS calls; needs the command channel (now exists). |
| 6 | Constant wallpaper nobody can change | planned | Via Windows policy keys, not by fighting the OS. |
| 7 | Keep student files temporarily, wipe with one button, host can browse | planned | Baseline + diff design; wipe must be scope-proven by tests. |
| 8 | Add computers by ID or LAN; changes apply when back online | **done** (ID/LAN) / planned (offline queue) | Pairing by key with a 6-digit code, mDNS on LAN, reconnect by key after an IP change. The offline mailbox (D9) is not built. |
| 9 | Enable/disable programs, editable games list | planned | Process watcher + browser policy files. |
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

## What to build next, in value order

1. **Act on a PC**: power off/reboot, lock screen, block apps — the actual classroom control (#5, #4, #9).
2. **Policy engine** so those survive a reboot and work offline (#24, #6).
3. **Full-resolution view and remote control** (#1, #3).
4. **Exam mode**: lock everyone, broadcast, collect and wipe files (#13, #7, #14).
5. Then updates (#15), installer and role picker (#17), other platforms (#23), AI (#18–20).
