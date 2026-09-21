# AGENTS.md — Co-watcher (working name)

> **Read this file first, every session.** It is the single source of truth for rules, decisions and
> architecture. If code and this file disagree, stop and ask — then fix whichever is wrong.
> Original brief: [`ClassWatcher.md`](ClassWatcher.md) · Roadmap: [`docs/PLAN.md`](docs/PLAN.md) ·
> Go-to-market: [`docs/BUSINESS.md`](docs/BUSINESS.md) ·
> Pre-release manual checks: [`docs/TEST-CHECKLIST.md`](docs/TEST-CHECKLIST.md)

**Status (2026-09-21):** **broadcast with a source picker + student lockdown** landed. A Zoom/Teams-style
picker (`gui::list_broadcast_sources`) lists every monitor and app window with a thumbnail; the teacher
picks one, chooses target PCs, and can tick **Lock students onto it** — a locked broadcast shows on a
separate Win32 desktop (`platform::present::open_locked`, sharing the exam lock's by-name restore +
watchdog) so Alt+Tab/Win/Ctrl+Esc do nothing. Window capture is `media::window_capture` (`EnumWindows`
+ `PrintWindow`). Also this session: the **exam + Win+L** bare-desktop bug is fixed (restore Default by
name + watchdog, verified live), **lazy program icons** in the app picker (`Control::FetchAppIcon`), and
the **binaries carry real icons** (Host for Console/viewer, Client for Agent). `PROTOCOL_VERSION` is 15.
Per-OS support is now written down in `docs/PLATFORMS.md` (capture/input/power/broadcast/lockdown are
Windows-only; Linux/macOS are stubs). AI-endpoint choice recorded as **D22**. **Next: the AI chat panel.**

**Status (2026-09-16, later):** **exam lockdown** exists — `platform::examlock` puts a fullscreen
message window on a **separate Win32 desktop** (`CreateDesktopW` + `SwitchDesktop`) the student can't
Alt+Tab or Win-key away from, toggled from the focused view (`Control::SetExam`, `PROTOCOL_VERSION`
13). It is **built but unverified** — running it seizes the desktop, so it needs a VM/second machine;
Ctrl+Alt+Del and Task Manager are not yet blocked (policy engine, next). Recording also gained
**two-pass** (a background re-encode after capture, since a live pipe can't 2-pass). 183 tests pass.

**Status (2026-09-16):** recording is now real video, not just MJPEG. When an `ffmpeg.exe` sits next
to the Agent (or on PATH) the recorder pipes raw BGRA to it and encodes H.264/H.265/AV1 with a chosen
preset, CRF quality, B-frames and the lanczos scaler at a custom size/rate (verified end-to-end by a
test that produces a real mp4); without ffmpeg it falls back to the built-in MJPEG writer. Recordings
can now be **downloaded to the teacher's PC** (`FetchRecording` over a QUIC uni-stream, path-traversal
guarded). Also fixed this session: the grid+stream two-duplication contention (thumbnails go GDI while
streaming), viewer control (was never forwarding input; green border now shows control), the wallpaper
black-out is tied to the live-preview stream and restores on close (the service no longer auto-locks
the wallpaper), PC renaming, the footer floating mid-screen, a resize-safe layout, a draggable/resizable
Programs window with process search, and a DVD-style "disabled" easter egg. `PROTOCOL_VERSION` is 12.
182 tests pass. **Next: two-pass recording, then the exam/lockdown mode.**

**Earlier status (2026-09-15):** first real two-machine test done, and it drove two fixes. The capture
storm — a student locking their PC made DXGI report `ACCESS_LOST` (shown as the misleading "keyed
mutex was abandoned"), and *both* sides tore the session down and reconnected forever — is fixed:
`ProtocolError::ScreenUnavailable` (transient) keeps the session alive on both sides, and the
capturer now recovers from a lost D3D device, not just a lost duplication (`PROTOCOL_VERSION` 9→10).
The **native viewer** (D12) now exists: `crates/viewer` (`cowatcher-viewer`) is a resizable
winit+softbuffer window that decodes the live H.264 stream and forwards mouse/keyboard; the Console's
focused view launches it (Live view / Control, with a resolution + fps picker, so 3840×2160 @ 15 is
selectable). The Agent now serves **concurrent** sessions so the grid and a viewer can watch one PC
at once. **Phase 1.4b landed (unverified):** the SYSTEM service no longer tries to serve from session
0 — it launches a per-session **helper** (`platform::session::launch_in_session`, held in a
kill-on-close job) into the logged-in student's session, which does the capture; agent identity moved
machine-wide to `%ProgramData%\co-watcher\agent` (with a one-time migration from `%LOCALAPPDATA%`) so
service, helper and `pair` share one key. The viewer is now a GUI app (no console window), and the
launcher overlay is draggable/resizable. 181 tests pass. Everything about the service (install, the
token launch, capture-through-helper) needs the VM/two-machine checks in `docs/TEST-CHECKLIST.md §8`.
Open next: pairing-over-real-network reliability (a first-dial timeout was seen), and the D3 indicator
trio (tray icon / login notice / being-viewed badge), which is still entirely missing.

**Earlier status (2026-09-14, later):** the Console can now *act*, not just watch. Remote mouse and
keyboard (Ctrl+Alt+Esc to release), lock, shutdown/reboot/log-off, app blocking, an app launcher,
screen recording at a chosen size and rate, broadcasting the teacher's screen, rooms with a
leave-password, six-character device IDs and a first-run tutorial are all built and verified live.
See `docs/FEATURES.md` for the honest per-feature state.

**Earlier status (2026-09-14):** shipping as two working binaries. `cowatcher-console.exe` opens a teacher
window (screen grid, pairing, per-PC monitor choice, preview quality, listen, `.ini` translations with
a language switcher); `cowatcher-agent.exe` serves screens and audio. Screen share (#1) and audio share
(#2) from the brief are **done**; the product can watch and listen but cannot yet *act* on a PC.
See `docs/FEATURES.md` for the full state, including what is **impossible** to build.

**Earlier status (2026-09-12):** Phase 0 done (6/8 spikes; 0.7 folded into Phase 1). Phase 1 underway:
1.1 identity, 1.2 wire proto, 1.3 pairing logic, and 1.3b pairing over a real iroh endpoint with mDNS
are built and tested (44 offline tests + 2 `#[ignore]` network e2e tests, all green). Next: 1.4 Agent
service + session helper (needs an elevated VM). Outstanding real-world reruns for the user are listed
per-step in `docs/PLAN.md` (two-PC pairing, old-PC CPU, physical-keyboard escape).

---

## 1. What we are building

An open-core classroom and computer-lab control system. It is an "AnyDesk for teachers": watch all
student screens, take control, lock, power off, block games, run exams. It works over LAN and over
the internet by device ID. When offline, each PC keeps enforcing the last policy it received.

- **"Co-watcher" is a placeholder.** Do not invent a brand. The product name lives in **one place
  only** (`crates/proto/src/lib.rs` → `pub const PRODUCT_NAME`, plus Cargo/Tauri metadata), so a
  rename is a one-line change.
- This codebase is meant for a large future team. Write boring, explicit, well-named code.
  No clever tricks and no obfuscation.

## 2. Glossary (use these words in code, docs, UI strings)

| Term | Meaning |
|------|---------|
| **Console** | Teacher/admin app (Tauri). The brief calls it "server / main host". |
| **Agent** | Student-PC side: a headless background service plus a per-session helper. The brief calls it "client". |
| **Hub** | Optional always-on service (our cloud, or self-hosted by a school). It holds the short-ID directory, the offline mailbox, licences and update manifests. |
| **Relay** | `iroh-relay`. Carries traffic when a direct/hole-punched connection fails. Can be self-hosted. |
| **Org** | A school or company. It owns the signing key that every Agent trusts. |
| **Room** | A group of devices, e.g. one computer lab (the brief says "cabinet"). |
| **Policy** | Signed, versioned desired state for a device or room: blocklists, wallpaper, lock mode, schedules. |
| **Device ID** | A 6-character human-typable handle (Crockford base32, e.g. `K7M2Q9`) **derived by hashing** the device's public key. The public key is the real identity. Derived, not allocated, so no central table and no lock is ever needed — see `crates/proto/src/device_id.rs`. |

## 3. Decisions (ADR-lite — change only with the user's approval, then log it here)

| # | Decision | Why |
|---|----------|-----|
| D1 | **Open-core.** Public repo: **AGPL-3.0** (Agent, Console, Hub, free tier). Private repo: **closed Pro crates** (AI, licensing/entitlements, large-fleet management), compiled in only with cargo feature `pro`. **CLA required** for outside contributions. | User choice. AGPL stops closed forks. The CLA lets us ship AGPL-core+Pro builds. The 10-device limit is enforced in official builds and the hosted Hub. Self-compiled community builds are unlimited, and we accept that. |
| D2 | First market: **public K-12 schools in a CIS country**. Client #1 is a teacher/school the founder knows. | User choice. See `docs/BUSINESS.md`. |
| D3 | **Visible by default:** tray icon, plus a "being viewed" badge while a stream is open. The Org admin may hide the badge. The **login notice cannot be disabled**, and **every remote action goes to the audit log**. | User choice. Legal notice rules; lower antivirus/RAT flagging risk. |
| D4 | Platforms: **Windows 10 1809+ / 11 first.** Then Linux and macOS. **Win7/8.1 = Agent-only build, later.** BSD = best effort. | User choice. Win7 is a Rust tier-3 target and has no WebView2. |
| D5 | **Budget ~$0 for the first 3 months:** SignPath Foundation (free OSS code signing, apply early), GitHub Actions (free for public repos), n0 public relays for dev, founder PC as the dev Hub. | User choice. Pro builds need a paid certificate later. |
| D6 | **Transport = [iroh 1.0](https://www.iroh.computer/blog/v1)** (QUIC, dial-by-public-key, NAT traversal, relay fallback, LAN without internet) for control, media and files. | One stack instead of WebRTC + signalling + TURN. Stable since June 2026. |
| D7 | **SemVer `MAJOR.MINOR.PATCH`**, plus a separate integer `PROTOCOL_VERSION` in the handshake. Release automation via `release-plz`. | Cargo requires SemVer. MSI caps major/minor at 255, so `2026.09.11` cannot be an MSI version. Protocol breaks need an explicit number. |
| D8 | **One installer, role chosen at first run** (Console or Agent). The Agent is a separate small headless binary; the UI is a separate process started only when needed. | Keeps student PCs light and the attack surface small. |
| D9 | **Offline = desired-state model.** Console signs the Policy, which goes to the Agent directly (LAN) or via the Hub mailbox (E2E encrypted; the Hub cannot read it). Agent stores it and **enforces the last valid Policy fully offline**. Console shows "pending" until it gets a delivery receipt. Agent logs are buffered and uploaded later. | Answers "what if there's no internet". Game blocking must work with the cable unplugged. |
| D10 | **No hardcoded master password — ever.** Break-glass = a per-Org admin code (Argon2id hash on the Agent), plus **per-device offline one-time codes** (HMAC/TOTP-style, shown in the Console). Attempts are rate-limited, and every use is logged and reported. | A hardcoded code in a public repo is a public backdoor on day 1. |
| D11 | **Video:** thumbnails are small JPEGs sent **only when the screen changed**. Full view is **H.264** (OS hardware encoder → OpenH264 fallback), with AV1 later where hardware exists. **Zero capture when nobody is watching.** Wallpaper is replaced with black while streaming. | Cheapest path that scales to 40+ screens. |
| D12 | **Remote view = native window** (winit + wgpu), decoded in Rust. The Tauri UI shows JPEG thumbnails only. | WebKitGTK (Linux Tauri) has no reliable WebCodecs. One path works on every OS. |
| D13 | **UI = Tauri 2 + TypeScript + Svelte 5** for the Console, the lock screen and the launcher. **Win7 lock screen = native Win32** (later). | User preference for Tauri. Svelte keeps bundles small for weak school PCs. |
| D14 | Storage = **SQLite** everywhere (Agent, Console, Hub). Move to Postgres only when a measurement says so. | Zero ops, easy to self-host. |
| D15 | Rust stack: `tokio`, `serde` + `postcard` (wire), `thiserror` (libs) / `anyhow` (bins), `tracing`, `windows` crate (official Microsoft bindings), `axum` (Hub). | Mainstream crates, easy hiring. |
| D16 | **Hub must be self-hostable** as one binary or Docker image, with install docs for school IT. Our own cloud Hub will be hosted in-country later. | CIS data-localisation laws; public-school IT policy. |
| D17 | i18n from day 1: **Russian + English**, then the pilot country's language. Rust sends message codes, not user-facing text. | Market (D2). |
| D19 | **Device IDs are derived, never allocated.** Six Crockford base32 characters (`K7M2Q9`) hashed from the device's public key. **Record** ids (recordings, log rows) are **UUIDv7**. | A derived id needs no central table, no lock and no "is this taken?" round trip, and works offline — the failure mode that makes a 4-byte auto-increment painful cannot occur. The key (2^256) is the real identity, so a short-id collision is harmless. UUIDv7 keeps v4's freedom from coordination but sorts by creation time, so recordings and audit rows order themselves. |
| D20 | **A paired device joins a Room and needs the room password to leave.** The password is 12 Crockford characters generated at first run, kept on the Console under DPAPI, and stored on the Agent only as an **Argon2id hash**. Wrong attempts are rate-limited and audit-logged. | Answers "students must not be able to unenrol their own PC" without ever shipping a default or hardcoded password (D10). It stops a student, not a local administrator — that limit is recorded in FEATURES. |
| D21 | **Storage stays files → SQLite.** ScyllaDB is recorded as a *planned, measurement-triggered* option for the hosted Hub only. | A room is tens of devices; a cluster database is the wrong shape and would make self-hosting (D16) impossible for a school. See the storage section in `docs/FEATURES.md` for the threshold. |
| D22 | **AI features use a user-supplied, OpenAI/Anthropic-compatible endpoint.** The teacher enters a base URL, API key and model in Settings (e.g. an OpenAI-style `…/v1` proxy such as Omniroute, key `sk-…`, model `kr/claude-sonnet-4.5`). The key is stored locally under DPAPI and every AI request is **proxied through the Console's Rust side**, never called from the web layer (keeps the key off the front end and sidesteps CORS/CSP). No AI key or endpoint is shipped or hardcoded. | User choice. Schools/founders bring their own inference (self-hosted or a proxy), so we never resell tokens or embed a secret, and one code path (OpenAI chat-completions shape) covers OpenAI, Anthropic-compatible proxies and most gateways. |
| D18 | Git: trunk-based `main` + **Conventional Commits**. Channels: nightly (each `main` commit), beta, stable (tags). GitHub is primary, with push-mirrors to **Codeberg + GitLab**. Every update is **signed (minisign/ed25519)**, so mirrors are untrusted transport. | User request: automatic releases and multiple mirrors. |

## 4. Architecture

```
        Console (teacher PC, Tauri)                      Agent (student PC)
   ┌───────────────────────────────┐              ┌────────────────────────────────────┐
   │ UI (Svelte) ── thumbnails grid│              │ agent service (SYSTEM/root)        │
   │ native viewer window (wgpu)   │  iroh QUIC   │  net · policy engine · blocker     │
   │ policy editor + org key       │◄────────────►│  power · updates · file baseline   │
   └──────────────┬────────────────┘ LAN direct / │ session helper (per logged-in user)│
                  │                  hole-punch / │  capture · input · lock desktop    │
                  │                  relay        │  wallpaper · audio                 │
                  │                               └───────────────┬────────────────────┘
                  │        ┌──────────────────────────────┐       │
                  └───────►│ Hub (optional; cloud / self) │◄──────┘
                           │ directory · mailbox · licence│
                           │ update manifest · iroh-relay │
                           └──────────────────────────────┘
```

- On Windows, the service runs in session 0, which **cannot capture the user's desktop**. It spawns
  a **session helper** in each interactive session. The helper does capture, input and the lock desktop.
- Lock screen / launcher / presentation mode = a fullscreen window on a **separate Win32 desktop**
  (`CreateDesktop` + `SwitchDesktop`, the approach Safe Exam Browser uses). One mechanism covers
  three features.
- The Agent trusts **only Org keys enrolled at pairing**. Every command and Policy is signed.

### Target workspace layout — create a crate only when its first real code lands

```
crates/proto     wire types, PROTOCOL_VERSION, PRODUCT_NAME. No I/O. Fuzzed.
crates/policy    desired-state model, signing, diff → apply plan. Pure logic, heavily tested.
crates/net       iroh endpoint, pairing, stream multiplexing, file transfer
crates/media     capture/encode/decode/audio; per-OS backends behind #[cfg(target_os)]
crates/platform  input, power, lock desktop, process control, wallpaper, file baseline (per-OS)
crates/agent     bin: service + session helper
crates/console   bin: Tauri app (src-tauri/ + ui/)
crates/viewer    native remote-view window
crates/hub       bin: axum + SQLite
crates/testkit   fake capture source, fake platform, simulated agents (E2E + load tests)
spikes/          throwaway Phase-0 experiments. Never imported by product code.
pro/             private repo (git submodule), only built with --features pro
```

## 5. Rules for every agent/contributor

**Security (never simplify away):**
- No remote shell or arbitrary command execution in any tier. Remote actions are a fixed, typed
  enum in `proto`.
- Validate every inbound message at the trust boundary. Parsers get fuzz targets.
- `unsafe` only in `media`/`platform`, and each block gets a `// SAFETY:` comment. Keep FFI thin.
- Secrets: never logged, never committed. Store device keys with OS protection (DPAPI / 0600 root).
- Updates: verify the signature **before** writing anything to disk. Keep a rollback copy.
- File wipe code may only touch paths inside the configured workspace scope, and tests prove it.

**Code:**
- Follow the ponytail ladder: reuse → stdlib → OS feature → existing dependency → new code. Any new
  dependency needs a one-line reason in the PR.
  Deliberate shortcuts get a `// ponytail:` comment naming the limit and the upgrade path.
- Prefer OS policy over fighting the OS (wallpaper lock, browser URL policies, power APIs).
- Every non-trivial step ships with its test (see `docs/PLAN.md` → "Done when"). No test, not done.
- Performance budgets (§6) are requirements, not wishes.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo nextest run` and `cargo deny check`
  must pass locally and in CI.
- Commits: Conventional Commits (`feat(agent): …`, `fix(media): …`). A breaking wire change bumps
  `PROTOCOL_VERSION`.
- User-facing strings go in i18n files only.

**Skills to load:**
- Planning/theory: `/marketing-mindset`, `/ponytail`, `/grill-me` (grill-me was not installed on
  2026-09-11; the grilling was done by hand).
- Coding: `/caveman`, `/ponytail`, `/rust-best-practices`, `/anti-ui-slop`, `/best-practices`,
  `/tauri`, `/i-have-adhd` (i-have-adhd was not installed on 2026-09-11).

## 6. Performance budgets (initial targets; recalibrate with Phase-0 measurements on a real low-end school PC)

| Scenario | Budget |
|----------|--------|
| Agent idle (nobody watching) | < 0.5 % CPU, < 30 MB RAM |
| Agent sending thumbnails (1 fps max, change-only) | < 3 % CPU |
| Agent full stream 1080p30, hardware encode | < 10 % CPU |
| LAN glass-to-glass latency (full view) | < 100 ms |
| Console with 40 thumbnails | < 15 % CPU, < 400 MB RAM |
| Thumbnail bandwidth per Agent | < 150 kbit/s average |

## 7. Open questions (grill these before the phase that needs them)

1. Exact pilot country → UI language #3, procurement thresholds, payment provider (merchant of record vs local). *(Before Phase 7.)*
2. Which competitor the pilot school uses today (Veyon? NetOp? nothing?) and the lab's OS/hardware. *(Before Phase 1 ends.)*
3. Legal entity for CLA, invoices, and later a paid code-signing certificate. *(Before Phase 6.)*
4. Exact contents of Pro (closed) vs free beyond "AI + >10 devices". *(Before Phase 6.)*
5. Default student-session start mode: normal desktop, launcher shell, or locked. *(Before step 2.7; defaulting to normal desktop.)*

## 8. Phase-0 spike results (dev PC: i7-12700F, RTX 3060 Ti, 32 GB, Win10 LTSC, 2560×1440)

Each spike lives in `spikes/<name>` (throwaway; never imported by product code). Numbers are from the
developer's machine — the "user TODO" reruns validate the same on a real classroom setup.

| Spike | Verdict | Key numbers (dev PC) | User must still verify |
|-------|---------|----------------------|------------------------|
| 0.3 iroh connectivity (`iroh-echo`) | **GO** | Dial-by-key in ~0.8 s; starts on the EU relay, self-upgrades to a direct path; ping median 0.27 ms | LAN with internet unplugged; two-network (Wi-Fi ↔ hotspot) — needs a 2nd PC |
| 0.4 capture → thumbnails (`capture-thumbs`) | **GO** | 2560×1440 → 320×180 (GPU mip); 5–7 KB/frame; 41–53 kbit/s; JPEG 0.37 ms; **CPU ≤ 0.03 % of one core**. Well under the §6 idle/thumbnail budgets. | Rerun on the oldest lab PC |
| 0.5 H.264 loop (`h264-loop`, OpenH264+NASM) | **GO** (fallback path) | 720p30: encode 5.5 ms, decode 1 ms, capture→present 7 ms, 25–35 % of one core. 1440p30: encode 22 ms, ~1 core. | Glass-to-glass on 2 PCs; add MF **hardware** encoder to meet <10 % CPU @1080p |
| 0.6 media over iroh (`media-iroh`) | **GO** | GOP-per-uni-stream; on a 700 ms stall the viewer requests a keyframe and latency recovers to ~4 ms within 1 s; 0 stale frames shown | Rerun under clumsy (5 % loss + 50 ms jitter) across 2 PCs |
| 0.7 service + session helper | **not started** | — | Needs an elevated VM; folded into step 1.4 |
| 0.8 lock desktop (`lock-desktop`) | **GO** | WebView2 lock UI renders on a separate `CreateDesktop`; proof `spikes/out/lock-webview.jpg`; Drop-guard + watchdog both restore the desktop. **Confirms D13.** | Manual escape checklist with a physical keyboard |
| 0.9 keyboard capture (`key-hook`) | **GO, with caveat** | LL hook swallows keys while focused; Win+Esc releases. **Caveat:** injected keys can't test this — must be a physical-key check + a pure unit-tested decision fn. Uncapturable: Win+L, Ctrl+Alt+Del. | Physical-keyboard check |

**Toolchain notes for reproducing the spikes:** OpenH264's SIMD needs **NASM** on `PATH` (portable copy
under `spikes/target/tools/`); without it the build silently falls back to ~4× slower C (720p encode
22 ms instead of 5.5 ms). Crate versions in use: `iroh` 1.2, `iroh-mdns-address-lookup` 0.5,
`windows` 0.62, `openh264` 0.9, `winit` 0.30, `softbuffer` 0.4, `wry`/`tao` 0.57/0.37.

**Net Phase-0 conclusion:** the whole media + transport + lock + input stack is viable in pure Rust on
Windows within the §6 budgets. No decision needs reversing. iroh (D6) is confirmed the right transport
(see D6 rationale + "Why not SSH/RDP/VNC" below).

### Why not SSH / RDP / VNC as the transport (D6 rationale, expanded)

Asked directly: is dial-by-public-key over iroh better than SSH auth? For *this* product, yes — they
solve different problems.

- **SSH is a shell/tunnel, not a fleet.** It authenticates one client to one always-reachable host with
  a known IP/port. Our problem is 40 student PCs behind a school NAT, IPs that change, no port-forwarding
  allowed, some offline. SSH gives you none of NAT traversal, a relay fallback, a short-ID directory, or
  an offline mailbox — you'd rebuild all of that (a bastion/jump host, autossh, a directory service)
  around it. iroh gives it in one library.
- **Identity model matches, and is actually stronger.** SSH pins a host key and authorises a client key;
  iroh's TLS *is* the endpoint's public key — you dial the key, so a wrong or spoofed endpoint can't
  complete the handshake. We still layer our own **Org-signed pairing** on top (an Agent trusts only keys
  enrolled at pairing, D-list), which is the equivalent of SSH `authorized_keys` but per-Org and
  revocable, plus every Policy/command is separately signed (D9/D10). So we keep SSH's key-auth guarantee
  and add authorisation SSH doesn't have.
- **We need media + datagrams, not a byte stream.** Screen video wants QUIC's independent streams and
  unreliable datagrams (drop-and-keyframe, spike 0.6). Tunnelling H.264 through one ordered SSH TCP
  stream reintroduces head-of-line blocking — the exact thing 0.6 avoids.
- **RDP/VNC are the closest "watch/control a screen" tools, but** RDP kicks the local user off (no good
  for "watch what the student is doing"), and both still need the reachability layer above. Veyon uses
  VNC internally and is LAN-bound for exactly this reason — matching our over-the-internet differentiator
  (BUSINESS.md §1).
- **Where SSH still wins:** an admin shelling into one server with a fixed address. If we ever want that
  for our own Hub ops, use real SSH — don't reinvent it. It's the wrong tool only for the agent fleet.
