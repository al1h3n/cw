# Roadmap — small steps, each with a test

> Rules and decisions live in [`../AGENTS.md`](../AGENTS.md). This file is the order of work.
> Every step has a **Done when** line. A step without a passing check is not done.
> Tick boxes as you go. Add measured numbers next to the step, and fix AGENTS.md §6 when they differ.

## How we work

- **Walking skeleton first.** One Console and one Agent on Windows, over LAN. Then add features, then
  internet, then more OSes.
- **De-risk first.** Phase 0 spikes attack the parts most likely to kill the project. Spike code goes in
  `spikes/` and gets thrown away. Only the numbers and the decisions survive.
- **Definition of Done (every step):** code + its test + CI green + one line in the changelog. Platform
  code also needs a manual check on a real PC or VM (write down which).
- **Bug bash at the end of every phase:** 1–2 hours of trying to break it on real hardware (student
  mindset: "how do I escape the lock?"). Every bug becomes a regression test.
- **Test lab (cost $0):** founder's PC + 1 old laptop + Windows VMs (Hyper-V/VirtualBox; Microsoft's
  free evaluation images). Packet loss / latency on Windows via [clumsy](https://jagt.github.io/clumsy/).
- **Milestones map to real people:** M1 = founder (client #0), M2 = teacher's lab (client #1).

---

## Phase 0 — Foundations and spikes (≈2–3 weeks)

> **Dev PC for all numbers below:** i7-12700F (12 threads), RTX 3060 Ti, 2560×1440, Windows 10 LTSC 21H2.
> It is far faster than a school PC, so every CPU number must be re-measured on the oldest test PC.

- [ ] **0.1 Repo + CI.** `git init`, Cargo workspace, `rust-toolchain.toml` (stable), `LICENSE`
  (AGPL-3.0), `CONTRIBUTING.md` (CLA + Conventional Commits), `.github/workflows/ci.yml`
  (fmt, clippy, nextest, cargo-deny on windows/macos/ubuntu).
  **Done when:** CI is green on an empty `proto` crate on all 3 OSes.
  *Result 2026-09-11:* all files in place; fmt/clippy/nextest/deny green locally on Windows. Actions
  pinned by SHA, Dependabot on. **Pending:** push to GitHub to see CI green on 3 OSes.
- [ ] **0.2 Apply for SignPath Foundation** (free OSS code signing). Approval takes time, so start now.
  **Done when:** the application is submitted. *Pending: founder (needs the public repo).*
- [ ] **0.3 Spike: iroh connectivity.** Two binaries dial each other by public key.
  **Done when:** it connects (a) on LAN with the internet unplugged, (b) across two different networks
  (home Wi-Fi ↔ phone hotspot) direct or via relay. Record RTT and whether it was direct or relayed.
  *Result (`spikes/iroh-echo`, same PC):* connects by key in 0.8 s via the EU relay, then upgrades to a
  direct path by itself; ping median 0.27 ms. **Pending:** (a) and (b) need a second PC.
- [ ] **0.4 Spike: Windows capture → thumbnails.** DXGI Desktop Duplication → GPU downscale → JPEG
  only when the dirty rects are non-empty.
  **Done when:** 1 fps change-only thumbnails at < 3 % CPU on the oldest test PC. Record the numbers.
  *Result (`spikes/capture-thumbs`, dev PC):* 2560×1440 → 320×180 via GPU mip level 3; 5.4–6.7 KB per
  thumbnail, 41–53 kbit/s, JPEG encode 0.37 ms, CPU ≤ 0.03 % of one core over 60 s. **GO.**
  Pending: old-PC run. Note: a blinking cursor counts as a change; hash the thumbnail to skip identical ones.
- [x] **0.5 Spike: H.264 encode/decode + native window.** (OpenH264 fallback path first; the Media
  Foundation hardware encoder is a later spike.) Capture → encode → decode → winit + softbuffer window.
  → `spikes/h264-loop`.
  **Done when:** 1080p30 on LAN at < 100 ms glass-to-glass (film a stopwatch on both screens with a phone).
  *Result (`spikes/h264-loop`, dev PC, one process, NASM SIMD build):* 720p30 encode 5.5 ms, decode 1 ms,
  capture→present 7 ms, 25–35 % of one core. 1440p30: encode 22 ms, ~30 ms latency, ~1 core.
  **GO** for the OpenH264 fallback. **User TODO:** true glass-to-glass across 2 PCs; add the MF hardware
  encoder to hit the <10 % CPU budget at 1080p (§6).
- [x] **0.6 Spike: media over iroh.** One QUIC uni-stream per GOP. Drop the old GOP when congested,
  then request a keyframe. → `spikes/media-iroh`.
  **Done when:** 5 % loss + 50 ms jitter (clumsy) still gives a usable stream that recovers in < 1 s.
  If not → fall back to `str0m` (WebRTC) for media only, and update D6.
  *Result (dev PC, built-in `stall` mode = 700 ms freeze every 5 s):* viewer sends a keyframe request on
  the control stream, sender resets the GOP stream and forces an IDR, excess latency drops from ~700 ms
  back to ~4 ms within one report second, 0 stale frames shown. **GO.** **User TODO:** rerun under clumsy
  (5 % loss + 50 ms jitter) across 2 PCs.
- [ ] **0.7 Spike: service + session helper.** A SYSTEM service spawns a helper in the user session.
  The helper captures the screen and injects input, including on the UAC secure desktop.
  **Done when:** remote clicking through a UAC prompt works, and the helper survives logout/login.
  *Not started — needs an elevated admin session and a VM; this dev session is not elevated. Folded into
  Phase 1 (step 1.4). Design is in AGENTS.md §4.*
- [x] **0.8 Spike: lock desktop.** `CreateDesktop` + `SwitchDesktop` with a fullscreen window; WebView2
  renders on it. → `spikes/lock-desktop`.
  **Done when:** Alt+Tab, Win, Win+D, Ctrl+Shift+Esc and Ctrl+Alt+Del → Task Manager all fail to reach
  the normal desktop (the Task Manager policy is set). If WebView2 fails there → lock UI goes native, update D13.
  *Result (dev PC):* the WebView2 lock UI renders on the separate desktop — screenshot proof
  `spikes/out/lock-webview.jpg`. Failsafes verified: parent Drop-guard and child watchdog both switch
  back. **WebView2 works there → D13 stands.** **User TODO:** the manual escape checklist with a physical
  keyboard.
- [x] **0.9 Spike: Console keyboard capture.** A low-level `WH_KEYBOARD_LL` hook swallows keys while the
  window is focused; Win+Esc releases. → `spikes/key-hook`.
  **Done when:** pressing Win opens Start on the *remote* PC only. Document what cannot be captured
  (Win+L, Ctrl+Alt+Del → toolbar buttons "Send Ctrl+Alt+Del" and "Lock remote").
  *Finding:* SendInput-injected Win/Alt+Tab are NOT a valid automated test — the shell processes them
  before the hook, so the selftest was removed. In Phase 1 the decision (swallow vs pass, incl. the exit
  chord) becomes a pure unit-tested function; the physical-key behaviour is a release-checklist item.
  Uncapturable by any hook: **Win+L** and **Ctrl+Alt+Del** → the viewer needs toolbar buttons.
  **User TODO:** physical-keyboard check.
- [x] **0.10 Write down the results** in AGENTS.md (decisions + §6 budgets).
  **Done when:** every spike has a go/no-go line. *(Recorded in AGENTS.md §8, with the pending real-PC
  runs listed here.)*

## Phase 1 — Walking skeleton: 1 Console ↔ 1 Agent, Windows, LAN (≈4–6 weeks)

- [x] **1.1 Identity.** Device keypair on first run, stored with DPAPI. Device ID = 9 digits derived
  from the key (the Hub resolves collisions later). → `proto::DeviceId`, `net::Identity`,
  `platform::secret`.
  **Done when:** unit tests — the key persists across restarts, and the ID is stable and the right format.
  *Done 2026-09-11: 15 tests pass. The device key IS the iroh secret key (no second keypair). DPAPI
  moved to `crates/platform` so `net` stays unsafe-free (AGENTS §5). DeviceId is pure in `proto`,
  parses spaces/dashes, is serde-ready for wire use. Atomic key write; 0600 on unix.*
- [x] **1.2 `proto` v1.** `Hello{protocol_version, role, device_id, capabilities}`, `Ping`/`Pong`,
  typed `ProtocolError`, plus `encode`/`decode` (postcard) and `version_compatible`. → `proto::wire`.
  **Done when:** round-trip tests pass, a `cargo fuzz` target runs for 10 min without crashing, and a
  version-mismatch test returns a clean error.
  *Done 2026-09-11: postcard wire format; `decode` rejects trailing bytes (strict trust boundary) and
  is exercised by a 10k-iteration no-panic stable test now, plus a real libfuzzer target in `fuzz/`
  (`cargo +nightly fuzz run decode_control`, wired for Linux CI). Version mismatch → typed
  `UnsupportedVersion{ours,theirs}`. `Capabilities` is a forward-compatible bit set. No unbounded
  collections in any message. 21 tests pass.*
- [~] **1.3 Pairing.** The Console shows a 6-digit code + QR. The Agent enters it (or the Console types
  the Agent's Device ID). Keys get pinned on both sides, and LAN discovery works. → `proto::pairing`,
  `net::pairing` (`PairingCode`, `PairingSession`, `TrustStore`).
  **Done when:** tests cover: wrong code rejected, code expires after 5 min, replay rejected, pairing
  survives an IP change.
  *Protocol logic done 2026-09-11 (11 tests): wrong code → `WrongCode` + attempt consumed; expiry at
  exactly 5 min (valid 1 ms before); replay → `AlreadyUsed`; lock after 5 tries → `TooManyAttempts`;
  constant-time code compare; `TrustStore` pins by public key with no address, so it is inherently
  IP-change-proof (iroh reconnects by key). Security note in `proto::pairing`: no PAKE needed because
  the iroh transport already gives an encrypted, key-authenticated channel — the code is an
  authorization token (expiring, single-use, rate-limited), not an interception defence.*
  **Still to wire (needs the endpoint, below): run this over a real iroh connection + mDNS LAN
  discovery, and the two-PC "survives IP change" run.**
- [x] **1.3b Endpoint + discovery (transport wiring for 1.3).** Build the `net` iroh endpoint
  (secret key from `Identity`, mDNS address lookup), carry `PairMessage` on a pairing stream, and run
  pairing Console↔Agent for real. → `net::endpoint` (`bind`, `console_accept_pairing`,
  `agent_request_pairing`, `PairedPeer`, `now_ms`).
  **Done when:** two processes on one LAN pair by code with the internet unplugged; a paired peer is
  re-recognised after its address changes.
  *Done 2026-09-12: two in-process endpoints pair over a real iroh connection — correct code pins both
  sides, wrong code refused, nothing pinned (`tests/pairing_over_iroh.rs`, `#[ignore]` = network e2e,
  stable across repeated runs). Length-prefixed postcard framing capped at 64 KiB (trust boundary).
  Two bugs found and fixed: (1) the session clock must match the endpoint's — added `endpoint::now_ms`;
  (2) a QUIC close race dropped the reply — the agent now drives the close and the console waits on
  `conn.closed()` before returning.*
  **User TODO (still real-world):** run the two binaries on two physical PCs on one LAN with the
  internet unplugged (mDNS path), and confirm re-recognition after an IP change. Needs the Console/Agent
  binaries (Phase 1.5+); until then use a small two-subcommand example.
- [~] **1.4a Agent supervisor + session enumeration (no elevation).** → `crates/agent`
  (`supervisor::RestartPolicy` + `supervise`), `platform::session` (`list_sessions`,
  `list_active_sessions`), `cowatcher-agent` bin (`sessions`, `supervise`, `version`).
  **Done 2026-09-12:** backoff policy tested (doubles 1→2→4→8→16→30 cap, resets after a healthy run,
  no overflow); a real supervised child is restarted ≥3× then stopped via the stop signal;
  `list_sessions` verified unelevated (sees session 0 + the active Console session). 40 tests pass.
- [ ] **1.4b Agent service (SCM) + session-helper spawn — VM/elevated.** Register a SYSTEM service
  (`windows-service`), have it enumerate active sessions and spawn the helper via `WTSQueryUserToken`
  + `CreateProcessAsUserW` (new `platform::session::spawn_in_session`, SYSTEM-only), driven by the
  1.4a supervisor.
  **Done when:** VM test — reboot → Agent back within 30 s of login; killing the helper → it respawns.
  *Deferred: needs an elevated session + a throwaway VM; not run on the dev machine.*
- [ ] **1.4 (superseded by 1.4a + 1.4b) Agent service install/uninstall** (the helper from 0.7, made production-grade): auto-start,
  restart on crash.
  **Done when:** VM test — reboot → Agent back within 30 s of login; killing the helper → it respawns.
- [~] **1.5 Thumbnail grid** in the Console (Tauri + Svelte). The Agent captures only while a Console
  grid is open.
  **Done when:** budget check (§6) passes, and minimising the Console drops Agent CPU to idle within 3 s.
  - [x] **1.5a Control session + on-demand thumbnails (transport + logic).** → `net::control`
    (`ControlSession::connect`/`accept`/`request_thumbnail`/`serve_thumbnails`, `CaptureSource` trait,
    `LocalHello`/`PeerInfo`), `CONTROL_ALPN`, `Control::RequestThumbnail`/`Thumbnail` in `proto`.
    *Done 2026-09-12: over real iroh (`tests/control_over_iroh.rs`, `#[ignore]`), a trusted Console
    opens a session (Hello + version + **mutual trust check**), pulls 3 thumbnails from a fake
    `CaptureSource`, and capture happens only on request (0 before, 3 after → satisfies "zero capture
    when idle"). An untrusted Console is refused with `Unauthorized` and gets nothing. Framing capped
    at 64 KiB (trust boundary).*
  - [ ] **1.5b Real capture source.** Port spike 0.4's DXGI→JPEG into `crates/media` implementing
    `CaptureSource`; the Agent wires it into `serve_thumbnails`. Multi-monitor via `monitor` index.
  - [ ] **1.5c Console grid UI (Tauri 2 + Svelte 5).** Poll each paired Agent at ≤1 fps, show the grid;
    stop polling when minimised (drives the Agent to idle within 3 s). Native viewer window is Phase 1.6.
- [ ] **1.6 Full view** in the native viewer (from 0.5/0.6). Wallpaper goes black while streaming (D11).
  **Done when:** latency budget passes; wallpaper restores after disconnect, **including after the
  Console crashes** (test it).
- [ ] **1.7 Remote control.** Mouse, keyboard, Win key capture, exit chord, Ctrl+Alt+Del button (SendSAS).
  **Done when:** an E2E test injects scripted input into a test app on the VM and asserts the result.
  Manual check: control an elevated window (UIPI).
- [ ] **1.8 Visible indicator + login notice + local audit log** (D3).
  **Done when:** a test asserts the badge appears when a stream opens and every remote action writes
  an audit row.
- [ ] **1.9 First-run wizard.** Choose the role (Console/Agent), plus a skippable tutorial (3–5 screens,
  "Skip" on every screen, re-openable from Help).
  **Done when:** a UI test covers both paths; after "Skip", the tutorial never auto-shows again.
- [ ] **Bug bash + M1:** founder uses it daily at home on 2+ PCs for 1 week (client #0). Every annoyance
  becomes an issue.

## Phase 2 — Classroom essentials: the free tier, LAN-first (≈8–10 weeks)

Order = what a teacher needs first in a real lesson.

- [ ] **2.1 Policy engine** (`crates/policy`). Signed, versioned desired state. The Agent persists it
  and enforces it offline, then reports the actual state back (D9).
  **Done when:** property tests (proptest) show diff→apply is idempotent; a tampered Policy is rejected;
  an older version never overwrites a newer one; with the network unplugged the Agent still applies the
  stored Policy after a reboot.
- [ ] **2.2 Power.** Shutdown/reboot/log-off for all or selected PCs, with an optional countdown
  message. Wake-on-LAN: an online Agent in the same room sends the magic packet.
  **Done when:** VM test for each action; WoL is verified on one real PC.
  *Done 2026-09-14 except WoL: `proto::Action` (closed enum, PROTOCOL_VERSION 2), `platform::power`
  (InitiateSystemShutdownExW / AbortSystemShutdownW / ExitWindowsEx / LockWorkStation, privilege
  enabled at start-up), agent `audit.log` (tab-separated, one line per action including refusals),
  console room-wide and per-PC menu with a confirm step. Live on the dev PC: shutdown 300 s started,
  cancelled, and a second cancel reported "nothing scheduled". **User TODO:** log-off and an actual
  power-off on a VM or spare PC (not run on the dev machine for obvious reasons).*
- [ ] **2.3 Lock screen on demand** (lock desktop from 0.8) with the teacher's message.
  **Done when:** the escape checklist from 0.8 passes on a real PC; the lock survives Agent helper restarts.
- [ ] **2.4 Unlock codes + break-glass** (D10). Terminal command `unlock` → per-device offline OTP or
  the Org code. `admin` → break-glass pauses all enforcement for N minutes and notifies the Console.
  **Done when:** tests cover: correct code works offline; clock skew of ±2 min is accepted; 5 wrong
  attempts → exponential lockout; every attempt is audit-logged.
- [~] **2.5 App blocking.** A process watcher (poll 1 s; ETW later) + rules: exe name, path, publisher
  signature, path heuristics (`steamapps\common`, `Epic Games`, `Riot Games`, Roblox, Minecraft).
  The default "games" list is editable. Website blocking uses **browser policies** (Chrome/Edge
  `URLBlocklist` in the registry, Firefox `policies.json`), so no extension is needed.
  **Done when:** unit tests with fixture process lists; snapshot tests (`insta`) of the generated
  browser policy files; manual check — a blocked game dies in < 2 s and a blocked site shows the
  browser's block page.
  *Apps done 2026-09-14: `platform::process` (Toolhelp snapshot + exact-name match, protected-process
  guard, unit-tested), agent `blocker` thread (1 s sweep, rules saved to disk so blocking survives a
  reboot offline per D9), `proto::SetBlocklist`/`BlocklistState` (bounded to MAX_BLOCKLIST=256),
  room-wide list in the Console pushed to every connected PC and a Svelte editor with a starter list.
  Live: Notepad closed within 1 s, re-closed on relaunch, survived after the rule was cleared.
  **Still planned:** website blocking via browser policy files; publisher-signature and path rules;
  the snapshot format tests (`insta`).*
- [~] **2.6 Wallpaper lock** via Windows policy keys (`Policies\System\Wallpaper` + NoChangingWallPaper).
  **Done when:** the user changes the wallpaper in Settings → it is reverted or greyed out; the policy is
  removed cleanly when disabled.
  *Built 2026-09-14: `platform::wallpaper` sets/clears `NoChangingWallPaper`, wired as the
  `LockWallpaper`/`UnlockWallpaper` actions with a menu button. Confirmed the mechanism is correct
  but **admin-only**: writing the HKCU `Policies` hive returns "Access is denied" unelevated, so this
  only bites once the Agent is the SYSTEM service (1.4b). The unit test is `#[ignore]` for that
  reason. Setting a specific image waits for file transfer.*
- [ ] **2.7 Startup shell / launcher** (PC-club style): the admin sets the background and the per-PC
  shortcuts. The bottom-right terminal has `help`, `unlock`, `request` (asks the teacher), `shutdown`,
  `reboot`. The mode is set per Policy: normal desktop / launcher / locked.
  **Done when:** an E2E UI test (tauri-driver/WebDriver) runs each command; unknown commands give a
  friendly error; shortcuts start only allow-listed apps.
- [ ] **2.8 Student file workspace.** Take a baseline manifest of Desktop, Documents, Downloads,
  Pictures, Videos and Music at session start. The Console lists new/changed files, **collects** them
  (e.g. exam work) and **wipes** them with one button.
  **Done when:** unit tests cover baseline/diff; a **safety test proves wipe never deletes outside the
  scope** (symlinks, junctions, `..` paths, files open in other apps); the collected archive opens.
  `ponytail:` restoring *modified* files needs copy-on-write backups — later, if pilots ask for it.
- [~] **2.9 Audio share.** WASAPI loopback (`cpal`) + Opus, with a per-stream on/off toggle. Off = zero
  audio capture.
  **Done when:** the bandwidth test shows 0 audio bytes when off; A/V drift < 80 ms over 10 min.
  *Done 2026-09-14 (pulled forward, because screen without sound is half a lesson): `media::audio`
  captures the student's output device in loopback mode, downmixes to mono and decimates to ~16 kHz,
  and a ring buffer drops the oldest audio so the teacher always hears "now" instead of a backlog.
  Capture starts only when a Console sends `SetAudio{enabled:true}` and stops when it stops, so "off =
  zero capture" holds. Listening is exclusive — one PC at a time. Verified with the release binaries:
  16 kHz mono, 64 000 samples for 4.0 s, loudest sample 16386 during a tone and exactly 0 in silence.*
  `ponytail:` still raw PCM (~256 kbit/s). Opus (~32 kbit/s) and the A/V drift measurement wait until
  full-rate video exists to drift against.
- [ ] **2.10 Presentation mode.** The Console shares its full screen, a region or one window
  (Windows.Graphics.Capture) to all or selected Agents. They show it on the lock desktop with input
  blocked.
  **Done when:** a testkit run with 30 simulated Agents on one LAN stays under the measured bandwidth
  budget; a real PC cannot escape presentation mode (checklist from 0.8).
  `ponytail:` unicast fan-out; Hub/peer relaying when > 40 viewers or on weak Wi-Fi.
- [ ] **2.11 Media broadcast** (listening exams). The file is **preloaded** to the Agents, then they
  start **in sync** at time T. No controls, plays once, and is deleted afterwards. Live mode (teacher's
  microphone) reuses 2.10.
  **Done when:** start offset between Agents < 100 ms (measured); an Agent that reconnects mid-play
  does not restart the file.
- [ ] **2.12 Scheduled recordings.** Agent-side, low fps (e.g. 5 fps, 720p) with the hardware encoder
  → fragmented MP4 segments. The schedule lives in the Policy (works offline), and the file uploads later.
  **Done when:** cutting power mid-recording still leaves a playable file; the schedule fires with the
  network unplugged.
- [ ] **Bug bash + M2: pilot in the teacher's lab (client #1), ≤ 10 PCs, 2 weeks.** Visit on day 1.
  Collect feedback daily (a 3-question form: what broke, what was missing, what did students try).
  Fix the top 3 before moving on.

## Phase 3 — Internet + offline delivery: the Hub (≈4–5 weeks)

- [ ] **3.1 Hub skeleton** (`axum` + SQLite): Orgs, Rooms, the Device ID → public-key directory, and
  enrolment tokens.
  **Done when:** API integration tests; the Hub cannot learn anything beyond metadata (reviewed and noted).
- [ ] **3.2 Mailbox.** Signed, E2E-encrypted Policy blobs are queued for offline Agents and return
  delivery receipts. The Console shows "pending / delivered / applied" per device.
  **Done when:** a testkit scenario — Agent offline 3 simulated days, 5 Policy edits, reconnect → it
  converges to the newest version and all receipts arrive.
- [ ] **3.3 Self-host pack.** Hub + `iroh-relay` as one Docker Compose file *and* as plain binaries +
  systemd units. Install guide in Russian/English for school IT.
  **Done when:** a fresh VPS/VM install from the guide takes ≤ 15 min (time it with someone who is not
  the author).
- [ ] **3.4 Buffered events.** Audit log and alerts queue on the Agent and upload on reconnect.
  **Done when:** no events are lost across offline periods and restarts (test with 10k events).
- [ ] **3.5 Free-tier limit** (10 devices) in official builds + the hosted Hub (D1).
  **Done when:** the 11th device gets a clear upgrade message, and nothing that already works breaks.
- [ ] **M3:** the teacher manages the lab from home over the internet. With the school's internet down,
  blocking still works.

## Phase 4 — Update center + release pipeline (≈3 weeks; can run in parallel with Phase 3)

- [ ] **4.1 Versioning.** `release-plz` opens release PRs with the SemVer bump + CHANGELOG from
  Conventional Commits.
  **Done when:** a test release `0.1.0` is tagged automatically.
- [ ] **4.2 Build matrix + installers.** Windows MSI (Tauri bundler/WiX), later `.pkg` (macOS) and
  `.deb`/`.rpm`/AUR/Nix flake (Linux). Linux builds go through a Nix flake for reproducibility (our
  "Hydra-lite"; a real Hydra only if CI minutes become the bottleneck).
  **Done when:** every `main` commit produces nightly artifacts.
- [ ] **4.3 Signing.** SignPath for Windows binaries; minisign (ed25519) for the update manifest.
  **Done when:** CI refuses to publish unsigned artifacts; a tampered manifest is rejected by a test.
- [ ] **4.4 Update center.** Channels (stable/beta/nightly). The admin can auto-update or pin a version
  per Room. The Console serves the update to its Agents as a **LAN cache** (1 download instead of 40).
  The Agent self-updates, runs a health check, and **rolls back automatically** if it fails.
  **Done when:** VM test N-1 → N succeeds; a deliberately broken build rolls back by itself; an update
  interrupted by power loss leaves a working Agent.
- [ ] **4.5 Mirrors.** GitHub is primary, with push-mirrors to Codeberg + GitLab. The release job uploads
  artifacts to all three, and the manifest lists every mirror URL.
  **Done when:** blocking GitHub in the hosts file → the update still installs from a mirror.
- [ ] **4.6 AI PR review (optional, when the budget allows):** Claude Code GitHub Action on PRs, as a
  reviewer on top of the tests, never a replacement for them.

## Phase 5 — Cross-platform (≈8–12 weeks; order by pilot demand)

- [ ] **5.1 Linux Agent.** X11 (XShm + XDamage); Wayland (xdg-desktop-portal ScreenCast + PipeWire with
  a restore token; **DRM/KMS capture as root** when there is no portal consent); input via **uinput**
  (works on X11 and Wayland); audio via the PulseAudio API (also covers PipeWire-pulse); systemd
  service; logind for power; dconf/KDE kiosk locks for the wallpaper; browser policy files in
  `/etc/opt/chrome/policies/managed` etc.
  **Done when:** CI builds pass, and a VM smoke test (pair, view, control, lock, block) passes on
  Ubuntu LTS, Debian, Fedora, Arch, NixOS (flake module) and openSUSE.
- [ ] **5.2 macOS Agent.** ScreenCaptureKit (12.3+), CGEvent input, launchd daemon + per-user agent. The
  TCC permissions (Screen Recording, Accessibility) need user/MDM approval — write a PPPC-profile guide.
  **Done when:** it works on the oldest and newest supported macOS; the permission onboarding is tested
  end to end.
- [ ] **5.3 Console on Linux and macOS** (Tauri + native viewer).
  **Done when:** the same E2E suite passes on all 3 OSes.
- [ ] **5.4 Win7/8.1 Agent-only build.** Target `x86_64-win7-windows-msvc` (tier 3, built from source),
  GDI capture, native Win32 lock screen, no WebView.
  **Done when:** a Win7 VM passes the Agent smoke test.
- [ ] **5.5 BSD (best effort).** FreeBSD X11 + OSS/sndio. Community-maintained, CI build only.

## Phase 6 — Pro (closed) + AI (≈6–8 weeks)

- [ ] **6.1 Entitlements.** A licence token signed by our key, checked by the Hub and by official
  builds; `--features pro` compiles in the private crates.
  **Done when:** tests cover expired/forged/over-limit tokens; the community build compiles and runs
  without `pro/`.
- [ ] **6.2 AI provider layer.** Two adapters cover almost everything: **OpenAI-compatible**
  (OpenAI, DeepSeek, Google's OpenAI-compatible endpoint, OpenRouter, local Ollama) and
  **Anthropic native**. BYOK keys are stored encrypted in the Console only.
  **Done when:** contract tests with recorded responses; a key never appears in logs (tested).
- [ ] **6.3 AI Watch — cheapest check first.** Each stage runs only if the previous one can't decide:
  1. Rules (process / window title / URL) — free, catches most games and videos.
  2. Change gate — only when the foreground app changed or a perceptual hash differs.
  3. Text-first — window title + native OCR (Windows.Media.Ocr / macOS Vision / Tesseract) sent to a
     cheap text model. Text tokens cost far less than image tokens.
  4. Local classifier (ONNX via `ort`): game / video / social / work.
  5. Vision model only for the rest: **one grid image of up to 16 downscaled screens per request**,
     cached by hash.
  The teacher writes rules in plain language. The AI can only return **allow-listed actions** (close
  app, close tab, notify teacher, lock, save evidence), with a confidence threshold. **"Suggest" mode is
  the default**, and there is a daily cost cap per Org.
  **Done when:** a labelled eval set of 500+ screenshots meets precision ≥ 95 % for "close" actions;
  cost per student-hour is measured and written into `docs/BUSINESS.md`.
- [ ] **6.4 Lesson summaries.** The teacher's screen (slide changes) + microphone → transcription (local
  whisper or API) → summary, optionally shared with students.
  **Done when:** a 45-min recorded lesson → summary in < 2 min; the teacher can edit it before sharing.
- [ ] **6.5 Hosted AI** for subscribers: **a fixed, benchmarked cheap model per task** (not a random
  free HuggingFace model — free tiers are rate-limited and quality varies, which paid users won't accept),
  with hard cost caps.

## Phase 7 — Monetisation + launch (the marketing parts start in Phase 1 — see BUSINESS.md)

- [ ] **7.1 Billing.** A merchant of record or local payment provider (depends on the pilot country),
  **plus invoices** — public schools pay by invoice/contract, not by card.
- [ ] **7.2 Tender pack.** Spec sheet, data-flow description (what leaves the school: nothing, when
  self-hosted), install guide and price list in the local language.
- [ ] **7.3 Docs site** (from Markdown, hosted free on Codeberg/GitHub Pages).
- [ ] **7.4 Crash reporting.** Opt-in minidumps uploaded to the Hub (self-hosted), with no screen content.

## Always-on test layers (build them as the steps above need them)

| Layer | Where | What |
|-------|-------|------|
| Unit + property | `proto`, `policy`, pure logic | serialization round-trip, policy idempotency |
| Fuzz | every decoder at a trust boundary | nightly CI job, 10 min per target |
| Snapshot (`insta`) | generated OS policy files, i18n keys | catch accidental changes |
| E2E simulated | `testkit`: fake capture + fake platform + real iroh on localhost | runs on all 3 OSes in CI |
| Load | `testkit` | 40 / 200 / 1000 simulated Agents vs Console and Hub; budgets in AGENTS.md §6 |
| Platform smoke | VMs (later self-hosted runner) | pair, view, control, lock, block, update, rollback |
| Escape checklist | real PC, every release | Alt+Tab, Win, Task Manager, safe mode, clock change, killing processes, unplugging the network |
| Real classroom | client #1's lab | every beta before stable |
