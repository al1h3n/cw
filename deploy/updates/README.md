# Agent update mirror

Serve `files/` through any **HTTPS** host. The included Caddy container listens on loopback
port 8080; expose it with a TLS reverse proxy, Tailscale Serve, or a zrok public share.
The mirror holds only `manifest.json` and `cowatcher-agent.exe`. The signing private key must
stay on the build machine, outside this directory and outside the repository.

On the build machine:

```text
python -m pip install cryptography
mkdir C:\secure
python deploy/updates/sign_update.py keygen C:\secure\cowatcher-update-private.hex
cargo build --release -p agent
python deploy/updates/sign_update.py sign C:\secure\cowatcher-update-private.hex 0.2.0 target/release/cowatcher-agent.exe deploy/updates/files
```

Keep the printed **public** key. Set `0.2.0` to the binary's actual Cargo package version;
change the workspace `version` in the root `Cargo.toml` before the release build. Store
`C:\secure\cowatcher-update-private.hex` somewhere only the publisher can read, with a backup.
the scheduled updater only installs versions newer than its own. Publish the `files/` contents
to your HTTPS mirror. Start the included local server with `docker compose up -d` from this
directory; connect your TLS tunnel to `http://127.0.0.1:8080`.

On each student PC, in an elevated prompt:

```text
cowatcher-agent install https://your-host.example/manifest.json PUBLIC_KEY_HEX
```

Installation checks the signed manifest and executable **before** stopping the service, copies
the selected executable to `%ProgramData%\co-watcher\bin`, registers the service and a daily
03:00 SYSTEM update task. The pinned key lives in an Administrators/SYSTEM-only directory at
`%ProgramData%\co-watcher\updates`. The binary folder permits student read/execute only; only
Administrators and SYSTEM can replace it. Later `cowatcher-agent install` uses the stored configuration.
To change mirrors, rerun install with the new URL and the same public key. After installing,
the copy in Downloads can be deleted. `cowatcher-agent uninstall` stops/removes the service
and update task; it leaves pairing state and the installed executable for reuse.

If the mirror is unreachable or verification fails, the existing service continues. New
binary installation keeps the prior executable as `cowatcher-agent.old.exe` and attempts a
rollback if service startup fails. A Windows VM test is still required before deployment.
