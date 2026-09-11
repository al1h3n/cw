//! Spike 0.8 — can a lock screen live on its own Win32 desktop, and can WebView2 render there?
//!
//! ```text
//! lock-desktop [SECONDS=15] [webview|native]
//! ```
//!
//! The parent creates desktop `cowatcher-lock-spike`, starts itself as a child *on that desktop*,
//! switches the screen to it, and switches back after SECONDS (max 60) or when you type `unlock` + Enter
//! in the WebView terminal. While locked, try to escape: Alt+Tab, Win, Win+D, Win+Tab, Ctrl+Shift+Esc,
//! Ctrl+Alt+Del -> Task Manager. Write down what got you back to the normal desktop.
//! (Production also sets the DisableTaskMgr policy while locked; this spike changes no settings.)
//!
//! Failsafes against getting stuck: the parent switches back in a Drop guard, and the child has its own
//! watchdog that switches to "Default" and exits at SECONDS + 5. Ctrl+Alt+Del -> Task Manager also returns.
//!
//! Proof: after the window is up, the child takes a GDI screenshot of the lock desktop and saves
//! `spikes/out/lock-<mode>.jpg`.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("child") => child::run(&args[1..]),
        _ => parent::run(&args),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("lock-desktop is Windows-only");
}

#[cfg(windows)]
const DESKTOP_NAME: &str = "cowatcher-lock-spike";

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
mod parent {
    use std::{thread, time::Duration};

    use anyhow::{Context, Result, bail};
    use windows::{
        Win32::{
            Foundation::{CloseHandle, GENERIC_ALL, WAIT_OBJECT_0},
            System::{
                StationsAndDesktops::*,
                Threading::{
                    CreateProcessW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW,
                    TerminateProcess, WaitForSingleObject,
                },
            },
        },
        core::{PCWSTR, PWSTR},
    };

    use super::{DESKTOP_NAME, wide};

    /// Switches the screen back to the original desktop no matter how we leave this scope.
    struct SwitchBack(HDESK);

    impl Drop for SwitchBack {
        fn drop(&mut self) {
            // SAFETY: handle came from OpenInputDesktop and stays open for this guard's lifetime.
            let result = unsafe { SwitchDesktop(self.0) };
            println!("switched back to original desktop: {result:?}");
        }
    }

    pub fn run(args: &[String]) -> Result<()> {
        let seconds: u32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(15).min(60);
        let mode = args.get(1).map_or("webview", String::as_str);
        if !matches!(mode, "webview" | "native") {
            bail!("mode must be webview or native");
        }

        // SAFETY: plain Win32 desktop calls; every handle is closed below.
        unsafe {
            let original = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_SWITCHDESKTOP)
                .context("OpenInputDesktop")?;
            let name = wide(DESKTOP_NAME);
            let lock = CreateDesktopW(
                PCWSTR(name.as_ptr()),
                PCWSTR::null(),
                None,
                DESKTOP_CONTROL_FLAGS(0),
                GENERIC_ALL.0,
                None,
            )
            .context("CreateDesktopW")?;

            let exe = std::env::current_exe()?;
            let mut command_line = wide(&format!("\"{}\" child {seconds} {mode}", exe.display()));
            let mut desktop = wide(DESKTOP_NAME);
            let startup = STARTUPINFOW {
                cb: size_of::<STARTUPINFOW>() as u32,
                lpDesktop: PWSTR(desktop.as_mut_ptr()),
                ..Default::default()
            };
            let mut process = PROCESS_INFORMATION::default();
            CreateProcessW(
                PCWSTR::null(),
                Some(PWSTR(command_line.as_mut_ptr())),
                None,
                None,
                false,
                PROCESS_CREATION_FLAGS(0),
                None,
                PCWSTR::null(),
                &startup,
                &mut process,
            )
            .context("CreateProcessW on lock desktop")?;

            // Give the child a moment to create its window so the switch doesn't flash an empty desktop.
            thread::sleep(Duration::from_millis(1500));
            println!("switching to lock desktop for up to {seconds}s ({mode})...");
            {
                let _guard = SwitchBack(original);
                SwitchDesktop(lock).context("SwitchDesktop to lock")?;
                let waited = WaitForSingleObject(process.hProcess, seconds * 1000);
                println!(
                    "{}",
                    if waited == WAIT_OBJECT_0 { "child exited (unlock typed)" } else { "timeout" }
                );
            }
            let _ = TerminateProcess(process.hProcess, 0);
            let _ = CloseHandle(process.hProcess);
            let _ = CloseHandle(process.hThread);
            let _ = CloseDesktop(lock);
            let _ = CloseDesktop(original);
        }

        let proof = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../out/lock-{mode}.jpg"));
        println!(
            "screenshot proof {}: {}",
            proof.display(),
            if proof.exists() { "saved" } else { "MISSING" }
        );
        Ok(())
    }
}

#[cfg(windows)]
mod child {
    use std::{path::PathBuf, process, thread, time::Duration};

    use anyhow::{Context, Result};
    use tao::{
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoop},
        window::{Fullscreen, WindowBuilder},
    };
    use windows::{
        Win32::{
            Graphics::Gdi::*,
            System::StationsAndDesktops::{
                DESKTOP_CONTROL_FLAGS, DESKTOP_SWITCHDESKTOP, OpenDesktopW, SwitchDesktop,
            },
            UI::{
                HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
                WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN},
            },
        },
        core::PCWSTR,
    };
    use wry::WebViewBuilder;

    use super::wide;

    const LOCK_HTML: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><style>
  html,body{margin:0;height:100%;background:#101418;color:#e8edf2;font:16px system-ui,sans-serif;overflow:hidden}
  main{height:100%;display:grid;place-content:center;text-align:center;gap:8px}
  #clock{font-size:96px;font-weight:300;letter-spacing:2px}
  .terminal{position:fixed;right:24px;bottom:24px;width:380px;background:#0a0d10;border:1px solid #2a3440;
    border-radius:6px;padding:10px 12px;font:14px ui-monospace,Consolas,monospace}
  .terminal input{width:100%;background:transparent;border:0;color:#9fe0a8;font:inherit;outline:none}
</style></head><body>
<main><div id="clock">--:--</div><div>This computer is locked by the teacher.</div>
<div style="opacity:.6">spike 0.8 &middot; returns automatically</div></main>
<div class="terminal"><div style="opacity:.6">type <b>unlock</b> and press Enter</div>
<input id="cmd" autofocus aria-label="command" placeholder="&gt; "></div>
<script>
  const clock = document.getElementById('clock');
  const tick = () => clock.textContent = new Date().toLocaleTimeString([], {hour:'2-digit', minute:'2-digit'});
  tick(); setInterval(tick, 1000);
  document.getElementById('cmd').addEventListener('keydown', e => {
    if (e.key === 'Enter') window.ipc.postMessage('cmd:' + e.target.value.trim());
  });
  window.addEventListener('load', () => window.ipc.postMessage('loaded'));
</script></body></html>"#;

    pub fn run(args: &[String]) -> Result<()> {
        let seconds: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(15);
        let webview = args.get(1).is_some_and(|m| m == "webview");
        let proof = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../out/lock-{}.jpg", if webview { "webview" } else { "native" }));
        let _ = std::fs::remove_file(&proof);

        // SAFETY: called once at startup before any window exists.
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        thread::spawn(move || watchdog(seconds + 5));

        let event_loop = EventLoop::new();
        let window = WindowBuilder::new()
            .with_title("lock-desktop spike")
            .with_decorations(false)
            .with_always_on_top(true)
            .with_fullscreen(Some(Fullscreen::Borderless(None)))
            .build(&event_loop)
            .context("create window")?;

        let _webview = if webview {
            let proof = proof.clone();
            Some(
                WebViewBuilder::new()
                    .with_html(LOCK_HTML)
                    .with_ipc_handler(move |request| match request.body().as_str() {
                        "loaded" => screenshot_later(proof.clone()),
                        "cmd:unlock" => process::exit(0),
                        _ => {}
                    })
                    .build(&window)
                    .context("create WebView2")?,
            )
        } else {
            screenshot_later(proof);
            None
        };

        event_loop.run(move |event, _, control_flow| {
            *control_flow = ControlFlow::Wait;
            if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
                *control_flow = ControlFlow::Exit;
            }
        });
    }

    /// Last-resort failsafe: return the screen to the normal desktop even if the parent died.
    fn watchdog(seconds: u64) {
        thread::sleep(Duration::from_secs(seconds));
        let name = wide("Default");
        // SAFETY: plain Win32 calls; the process exits right after.
        unsafe {
            if let Ok(default) =
                OpenDesktopW(PCWSTR(name.as_ptr()), DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_SWITCHDESKTOP.0)
            {
                let _ = SwitchDesktop(default);
            }
        }
        process::exit(0);
    }

    fn screenshot_later(path: PathBuf) {
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(1500));
            if let Err(err) = screenshot(&path) {
                eprintln!("screenshot failed: {err:#}");
            }
        });
    }

    /// GDI copy of the current input desktop (ours, while locked), saved as a quarter-size JPEG.
    fn screenshot(path: &std::path::Path) -> Result<()> {
        // SAFETY: GDI objects are created, used and released in this block; buffer sized w*h*4.
        let (pixels, w, h) = unsafe {
            let (w, h) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
            let screen = GetDC(None);
            let memory = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(memory, bitmap.into());
            BitBlt(memory, 0, 0, w, h, Some(screen), 0, 0, SRCCOPY).context("BitBlt")?;
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down rows
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            GetDIBits(memory, bitmap, 0, h as u32, Some(pixels.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
            SelectObject(memory, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            (pixels, w as usize, h as usize)
        };
        let (qw, qh) = (w / 4, h / 4);
        let mut small = Vec::with_capacity(qw * qh * 4);
        for y in 0..qh {
            for x in 0..qw {
                let i = ((y * 4) * w + x * 4) * 4;
                small.extend_from_slice(&[pixels[i], pixels[i + 1], pixels[i + 2], 255]);
            }
        }
        jpeg_encoder::Encoder::new_file(path, 70)?.encode(
            &small,
            qw as u16,
            qh as u16,
            jpeg_encoder::ColorType::Bgra,
        )?;
        Ok(())
    }
}
