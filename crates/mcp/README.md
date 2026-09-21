# cowatcher-mcp

A [Model Context Protocol](https://modelcontextprotocol.io) server that exposes Co-watcher's
classroom controls as MCP **tools**, so an AI agent can operate a lab precisely and safely.

It speaks JSON-RPC 2.0 over **stdio** and acts as the teacher's Console: it reads the same state
directory (`%LOCALAPPDATA%\co-watcher\console` by default, or `COWATCHER_DIR` / `--data-dir`) and can
reach only the PCs already **paired** in that Console. Every tool maps onto the same typed, signed
[`net::ControlSession`] the Console uses, so there is **no arbitrary-command tool** — an AI can only do
the named, reviewable things a teacher can (AGENTS.md §5), and every action is audit-logged on the
student PC (D3).

## Design notes

- **The operator skill is sent once, not per prompt.** On `initialize` the server returns
  [`SKILL.md`](SKILL.md) in the `instructions` field (MCP's built-in "system prompt" channel), which a
  client injects ahead of the conversation and caches. It is also offered as a prompt
  (`cowatcher_operator`) and a resource (`cowatcher:///skill`) for clients that prefer those. This is
  how the AI stays "certain and precise" about what the tools do without spending tokens re-reading the
  brief on every turn.
- **OS-agnostic.** This crate depends only on the shared `net` and `proto` crates; no Windows code
  lives here (the pinned architecture rule). All OS effects happen inside the Agent, over the wire.

## Tools

`list_devices`, `device_status`, `screen_thumbnail` (returns an image), `list_apps`, `list_running`,
`launch_app`, `close_app`, `set_blocklist`, `perform_action` (shutdown/reboot/log-off/lock-screen/
cancel-shutdown/lock-wallpaper/unlock-wallpaper), `set_exam`, `set_wallpaper`, `recording_status`,
`start_recording`, `stop_recording`, `list_recordings`.

## Running

```sh
cowatcher-mcp                 # uses the Console's default state dir
cowatcher-mcp --data-dir C:\path\to\console-state
```

Add it to an MCP client (e.g. Claude Desktop / Claude Code) as a stdio server whose command is the
`cowatcher-mcp` binary. Example client entry:

```json
{
  "mcpServers": {
    "cowatcher": { "command": "cowatcher-mcp", "args": [] }
  }
}
```

The Console must have paired at least one PC first; the AI reaches PCs only when they are on and online.

## Quick manual check

```sh
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_devices","arguments":{}}}' \
  | cowatcher-mcp
```
