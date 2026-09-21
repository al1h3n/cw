# Platform support — what works where

Co-watcher targets **Windows 10 1809+ / 11 first** (decision **D4** in `AGENTS.md`), then Linux and
macOS, with a Windows 7/8.1 **Agent-only** build much later. This file is the honest, current state of
each per-OS backend so nobody assumes cross-platform parity that does not exist yet.

**Short version: everything that touches the screen, input, power or the desktop is Windows-only
today. Linux and macOS compile, but those backends are stubs that return "not supported".** The
portable parts — the wire protocol (`proto`), pairing and transport (`net`, built on iroh), the
JPEG/H.264 codecs and resize math (`media`), the console UI (`console/ui`) — are already
cross-platform in principle; only the OS glue is missing.

## Per-feature backend status

| Feature | Crate / module | Windows | Linux | macOS |
|---|---|---|---|---|
| Screen capture (monitors) | `media::ThumbnailCapturer` (DXGI + GDI) | ✅ | ❌ stub | ❌ stub |
| Window capture (share one app) | `media::window_capture` (EnumWindows + PrintWindow) | ✅ | ❌ stub | ❌ stub |
| H.264 encode/decode | `media::h264` (OpenH264) | ✅ | ✅ portable¹ | ✅ portable¹ |
| Audio capture | `media::audio` (cpal) | ✅ | ✅ portable¹ | ✅ portable¹ |
| Remote input (mouse/keyboard) | `platform::input` (SendInput) | ✅ | ❌ | ❌ |
| Power (shutdown/reboot/logoff/lock) | `platform::power` | ✅ | ❌ | ❌ |
| App catalogue + launch | `platform::apps` (Start Menu, ShellExecute) | ✅ | ❌ | ❌ |
| App icons | `platform::apps::icon_bgra` (SHGetFileInfo) | ✅ | ❌ returns none | ❌ returns none |
| Full-screen broadcast | `platform::present` (StretchDIBits) | ✅ | ❌ stub | ❌ stub |
| Broadcast/exam **lockdown** desktop | `platform::present` / `platform::examlock` (`CreateDesktopW` + `SwitchDesktop`) | ✅ | ❌ n/a² | ❌ n/a² |
| Wallpaper black-out | `platform::wallpaper` | ✅ | ❌ | ❌ |
| Wake-on-LAN | `platform::wol` | ✅ (portable UDP) | likely ✅¹ | likely ✅¹ |
| Service + per-session helper | `platform::service`, `platform::session` | ✅ (unverified, see §8 checklist) | ❌ | ❌ |
| Console GUI (Tauri) | `crates/console` | ✅ | build not yet exercised | build not yet exercised |
| Native viewer | `crates/viewer` (winit + softbuffer) | ✅ | likely ✅¹ | likely ✅¹ |

¹ *Portable in principle* — the crate has no Windows-only code, but it has not been **built or run** on
that OS yet, so treat it as unverified until someone does.
² *n/a* — the separate-desktop lock is a Win32 concept. The equivalent on Linux/macOS is a different
mechanism entirely (a full-screen override-redirect/kiosk surface, or the platform's Assessment/Guided
Access mode); it will be designed when those platforms are built, not ported.

## What each non-Windows backend does today

Every OS-specific module has a `#[cfg(not(windows))]` implementation that compiles and returns a typed
"not supported" error (or an empty list / `None`), so the whole workspace builds on Linux and macOS and
the failure is explicit, never a panic or a silent wrong result. Examples:

- `platform::examlock` / `platform::present` → `NotSupported`.
- `platform::apps::list_apps` → empty catalogue; `icon_bgra` → `None`.
- `media::window_capture::list_windows` → empty; `capture_window_bgra` → `CaptureError`.
- `media::ThumbnailCapturer` (non-Windows `stub`) → `CaptureError`.

## What Linux support will need (rough order, Phase 5)

1. **Capture:** PipeWire screencast (Wayland, the modern path) with an X11 `XShmGetImage`/`XComposite`
   fallback. This is the big one and gates broadcast + thumbnails + streaming.
2. **Input injection:** `uinput` (works under both X11 and Wayland) or XTEST (X11 only).
3. **Power:** `logind` (`systemctl`, `loginctl lock-session`).
4. **App catalogue:** parse `.desktop` files; launch via `gio launch` / `xdg-open`.
5. **App icons:** resolve the `.desktop` `Icon=` key against the icon theme (freedesktop spec).
6. **Lockdown:** a full-screen override-redirect window plus a compositor-level input grab, or a kiosk
   session — there is no `CreateDesktop` equivalent, so this is a redesign, not a port.
7. **Service:** a `systemd` unit for the daemon and a per-seat user-session helper.

## What macOS support will need (Phase 5+)

1. **Capture:** ScreenCaptureKit (macOS 12.3+), which also enumerates windows for the "share one app"
   picker. Requires the Screen Recording TCC permission.
2. **Input injection:** `CGEventPost`. Requires the Accessibility TCC permission.
3. **Power:** `caffeinate`, `pmset`, and the login-window APIs.
4. **App catalogue/icons:** `NSWorkspace`, `.app` bundles and their `Icon` resources.
5. **Lockdown:** there is no separate-desktop trick; the realistic route is a full-screen
   `NSWindow` at a high window level plus **Guided Access / Automatic Assessment Configuration**, or a
   kiosk configuration profile. Another redesign, not a port.
6. **Packaging:** notarisation and a hardened-runtime signed build; the TCC prompts above must be
   granted by whoever deploys the Agent.

## Practical note

Because capture, input, power, broadcast and lockdown are all Windows-only right now, a Linux or macOS
build of the Agent would pair and connect but be unable to actually watch or control a screen. That is
why the first pilot (D2) is Windows-only, and why these two platforms are a **later phase**, not a flag
to flip.
