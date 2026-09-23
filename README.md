# Co-watcher (working name)

Open-source classroom and computer-lab control. See every student screen, take control, lock PCs,
block games and run exams, over LAN or the internet. Policies keep working offline.

Status: active development — Phase 1 largely built, Phase 2 well underway; not yet released. Two
Windows binaries work today: `cowatcher-console` (teacher) and `cowatcher-agent` (student). You can
pair over LAN or the internet, watch a live screen grid, open one screen as an H.264 stream, take
mouse/keyboard control, listen to a PC, lock/power/wake, block apps, push and lock wallpaper,
record, broadcast, and run the in-Console AI assistant (Surey). Exam lockdown, the SYSTEM service
and the secure-desktop paths are built but still need two-machine/VM verification; the Hub, updates,
signed installers and non-Windows Agents are not built yet. See [`docs/FEATURES.md`](docs/FEATURES.md)
for the honest per-feature state and [`docs/PLATFORMS.md`](docs/PLATFORMS.md) for per-OS support.

- Project rules, decisions and architecture: [`AGENTS.md`](AGENTS.md)
- Roadmap: [`docs/PLAN.md`](docs/PLAN.md)
- Contributing: [`CONTRIBUTING.md`](CONTRIBUTING.md)

Licence: [AGPL-3.0-only](LICENSE).
