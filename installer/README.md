# Building the Co-watcher installer

One installer (`cowatcher-setup.exe`) bundles **all three** binaries — Console, Agent, Viewer — and
picks the role at first run (ADR **D8**):

- **Teacher console** — installs the GUI and a Start Menu shortcut.
- **Student agent** — installs the Agent as an **auto-start Windows service** (`CowatcherAgent`). A
  service does **not** appear in Task Manager's *Startup* tab, which satisfies the "auto-launch without
  showing in autostart" requirement without any hidden Run key. Uninstalling removes the service.

## Build it locally

1. Build the release binaries (NASM must be on `PATH` for OpenH264):

   ```powershell
   cd crates\console\ui; npm ci; npm run build; cd ..\..\..
   cargo build --release -p console-app -p agent -p viewer
   ```

2. Install [Inno Setup 6](https://jrsoftware.org/isdl.php) (`choco install innosetup`).

3. Compile the installer (reads the binaries from `..\target\release` by default):

   ```powershell
   & "C:\Program Files (x86)\Inno Setup 6\ISCC.exe" installer\cowatcher.iss
   ```

   The result is `installer\Output\cowatcher-setup.exe`.

Optional: drop `ffmpeg.exe` into `dist\` before compiling and it is bundled next to the Agent for
real-codec recording (it is skipped if absent).

## CI

- **GitHub** (`.github/workflows/`): `ci.yml` runs fmt/clippy/tests on every push; `release.yml` builds
  the installer on a `v*` tag (or manually) and attaches it to a GitHub Release. `windows-latest` is
  free for public repos (D5).
- **GitLab** (`.gitlab-ci.yml`): needs a **Windows** runner (`saas-windows-medium-amd64` on SaaS, or a
  self-hosted one tagged `windows`).
- **Codeberg** (`.forgejo/workflows/`): Forgejo Actions, but Codeberg's shared runners are Linux and
  **cannot build Windows `.exe`s** — register a self-hosted Windows runner labelled `windows`, or treat
  Codeberg as a mirror and build elsewhere (D18).

## Lockdown without a signed driver

Blocking **Win+L** and stripping the **Ctrl+Alt+Del** screen does **not** need a kernel driver or code
signing. It is done with Windows **policies** applied by the Agent at runtime while an exam/broadcast
lock is active, and reverted when it ends — e.g. `DisableLockWorkstation`, `DisableTaskMgr`,
`NoLogoff`, `DisableChangePassword`, `HideFastUserSwitching` under
`HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System`. The Secure Attention Sequence itself
still fires, but with those entries removed there is nothing to do from it but cancel. This policy work
is a separate change from this installer.
