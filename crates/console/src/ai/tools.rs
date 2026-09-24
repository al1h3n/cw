//! The tools Surey may use, and how each one drives the real fleet.
//!
//! Same principle as the MCP server: every tool maps onto a typed [`DeviceManager`] method (and thus
//! the closed `proto::Action` enum) — there is no "run a command" tool. `ask_user` is different: it is
//! an *interaction* tool handled by the orchestrator (`super::run_turn`), not here, because it has to
//! round-trip to the panel and wait for the teacher's choice.

use serde_json::{Value, json};

use super::client::ToolSpec;
use crate::manager::DeviceManager;

/// The tool schemas offered to the model (including `ask_user`, which the orchestrator fulfils).
#[must_use]
pub fn specs() -> Vec<ToolSpec> {
    let device_id = json!({ "type": "string", "description": "A device_id from list_devices." });
    vec![
        ToolSpec {
            name: "list_devices".into(),
            description: "List paired student PCs with their name and connection status (idle/connecting/live/offline). Only 'live' PCs can be acted on.".into(),
            parameters: json!({ "type": "object", "properties": {} }),
        },
        ToolSpec {
            name: "power_action".into(),
            description: "Run a power/lock action on PCs. targets is a list of device_ids; leave it empty to hit every connected PC. DESTRUCTIVE actions (shutdown/reboot/log-off) interrupt students — prefer ask_user to confirm first.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "targets": { "type": "array", "items": { "type": "string" }, "description": "device_ids to act on; empty = all connected." },
                    "action": { "type": "string", "enum": ["shutdown", "reboot", "log-off", "lock-screen", "cancel-shutdown", "lock-wallpaper", "unlock-wallpaper", "reset-wallpaper"] },
                    "delay_seconds": { "type": "integer", "minimum": 0, "maximum": 3600 }
                },
                "required": ["action"]
            }),
        },
        ToolSpec {
            name: "set_exam".into(),
            description: "Start or end exam lockdown on one PC (fullscreen lock on a separate desktop). Pass duration_seconds to auto-release after that long (0 = until you end it); the PC enforces the timer itself, so a timed lock ends even if this app closes.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "device_id": device_id,
                    "on": { "type": "boolean" },
                    "message": { "type": "string" },
                    "duration_seconds": { "type": "integer", "minimum": 0, "maximum": 86400, "description": "Auto-release after this many seconds; 0 = until ended." }
                },
                "required": ["device_id", "on"]
            }),
        },
        ToolSpec {
            name: "list_apps".into(),
            description: "List the programs one PC can launch (each has an id for launch_app).".into(),
            parameters: json!({ "type": "object", "properties": { "device_id": device_id }, "required": ["device_id"] }),
        },
        ToolSpec {
            name: "launch_app".into(),
            description: "Start a published program on one PC by its id from list_apps.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "device_id": device_id, "app_id": { "type": "integer" } },
                "required": ["device_id", "app_id"]
            }),
        },
        ToolSpec {
            name: "list_running".into(),
            description: "List closable running programs on one PC (with pids).".into(),
            parameters: json!({ "type": "object", "properties": { "device_id": device_id }, "required": ["device_id"] }),
        },
        ToolSpec {
            name: "close_app".into(),
            description: "Close a running program on one PC by pid.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "device_id": device_id, "pid": { "type": "integer" } },
                "required": ["device_id", "pid"]
            }),
        },
        ToolSpec {
            name: "set_blocklist".into(),
            description: "Replace the class-wide list of blocked program names (empty clears it).".into(),
            parameters: json!({
                "type": "object",
                "properties": { "programs": { "type": "array", "items": { "type": "string" } } },
                "required": ["programs"]
            }),
        },
        ToolSpec {
            name: "start_recording".into(),
            description: "Start recording one PC's screen (must be connected/watched).".into(),
            parameters: json!({ "type": "object", "properties": { "device_id": device_id }, "required": ["device_id"] }),
        },
        ToolSpec {
            name: "stop_recording".into(),
            description: "Stop recording on one PC.".into(),
            parameters: json!({ "type": "object", "properties": { "device_id": device_id }, "required": ["device_id"] }),
        },
        ToolSpec {
            name: "recording_status".into(),
            description: "Report whether one PC is recording, and its frame count.".into(),
            parameters: json!({ "type": "object", "properties": { "device_id": device_id }, "required": ["device_id"] }),
        },
        ToolSpec {
            name: "list_files".into(),
            description: "List a directory in one PC's shared workspace folder (where files are exchanged). dir is workspace-relative; empty = root. Transfer is confined to this folder.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "device_id": device_id, "dir": { "type": "string" } },
                "required": ["device_id"]
            }),
        },
        ToolSpec {
            name: "send_file".into(),
            description: "Send a file to one PC's workspace: write base64 bytes as name inside workspace directory dir (created if needed).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "device_id": device_id,
                    "dir": { "type": "string" },
                    "name": { "type": "string" },
                    "data_base64": { "type": "string" }
                },
                "required": ["device_id", "name", "data_base64"]
            }),
        },
        ToolSpec {
            name: "fetch_file".into(),
            description: "Download a file from one PC's workspace to the teacher's PC (e.g. a student's script); returns where it was saved. path is workspace-relative.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "device_id": device_id, "path": { "type": "string" } },
                "required": ["device_id", "path"]
            }),
        },
        ToolSpec {
            name: "list_workspace".into(),
            description: "Recursively list every file in one PC's workspace (paths and sizes), e.g. to collect all work or see what changed.".into(),
            parameters: json!({ "type": "object", "properties": { "device_id": device_id }, "required": ["device_id"] }),
        },
        ToolSpec {
            name: "delete_file".into(),
            description: "Delete one file from one PC's workspace (a specific file). Destructive — confirm with ask_user first. path is workspace-relative.".into(),
            parameters: json!({
                "type": "object",
                "properties": { "device_id": device_id, "path": { "type": "string" } },
                "required": ["device_id", "path"]
            }),
        },
        ToolSpec {
            name: "clear_workspace".into(),
            description: "Wipe EVERY file in one PC's workspace (one-button clear of student files). Destructive and irreversible — always confirm with ask_user first.".into(),
            parameters: json!({ "type": "object", "properties": { "device_id": device_id }, "required": ["device_id"] }),
        },
        ToolSpec {
            name: "preload_media".into(),
            description: "Preload an audio file (base64) onto one PC for a synchronised listening exam. Stored privately (not the shared workspace). Call play_media afterwards.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "device_id": device_id,
                    "name": { "type": "string" },
                    "data_base64": { "type": "string" }
                },
                "required": ["device_id", "name", "data_base64"]
            }),
        },
        ToolSpec {
            name: "play_media".into(),
            description: "Play the preloaded media once on one PC, start_in_ms after it receives this (relative delay — call on several PCs to start together). No controls; deleted after. lock also locks the screen with the message.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "device_id": device_id,
                    "start_in_ms": { "type": "integer", "minimum": 0, "maximum": 60000 },
                    "message": { "type": "string" },
                    "lock": { "type": "boolean" }
                },
                "required": ["device_id"]
            }),
        },
        ToolSpec {
            name: "stop_media".into(),
            description: "Stop media playback on one PC and release the lock.".into(),
            parameters: json!({ "type": "object", "properties": { "device_id": device_id }, "required": ["device_id"] }),
        },
        ToolSpec {
            name: "ask_user".into(),
            description: "Ask the teacher to pick one option (or type their own). Use this to confirm a destructive action or to choose between alternatives before acting. Returns the chosen text.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "The question to show." },
                    "options": { "type": "array", "items": { "type": "string" }, "description": "The choices, in order." },
                    "allow_custom": { "type": "boolean", "description": "Also let the teacher type a free-form answer (default false)." }
                },
                "required": ["prompt", "options"]
            }),
        },
    ]
}

/// The name of the interaction tool the orchestrator fulfils itself.
pub const ASK_USER: &str = "ask_user";

/// Runs one fleet tool and returns its result as JSON. `ask_user` is not handled here.
///
/// # Errors
/// A bad argument, an unknown tool, or a fleet error (unknown/offline PC) — all returned as text the
/// model reads and adapts to, never a panic.
pub async fn execute(manager: &DeviceManager, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "list_devices" => {
            let devices = manager.devices();
            serde_json::to_value(devices).map_err(|e| e.to_string())
        }
        "power_action" => {
            let action_name = arg_str(args, "action")?;
            let delay = args
                .get("delay_seconds")
                .and_then(Value::as_u64)
                .and_then(|n| u16::try_from(n).ok())
                .unwrap_or(0);
            let action = crate::manager::parse_action(action_name, delay)
                .ok_or_else(|| format!("unknown action '{action_name}'"))?;
            let targets: Vec<String> = args
                .get("targets")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if targets.is_empty() {
                let sent = manager.perform(None, action)?;
                return Ok(json!({ "sent_to_connected_pcs": sent }));
            }
            let mut results = Vec::new();
            for id in targets {
                match manager.perform(Some(&id), action) {
                    Ok(_) => results.push(json!({ "device_id": id, "ok": true })),
                    Err(e) => results.push(json!({ "device_id": id, "ok": false, "error": e })),
                }
            }
            Ok(json!({ "results": results }))
        }
        "set_exam" => {
            let id = arg_str(args, "device_id")?;
            let on = args
                .get("on")
                .and_then(Value::as_bool)
                .ok_or("missing boolean 'on'")?;
            let message = args.get("message").and_then(Value::as_str).unwrap_or("");
            let duration_seconds = args
                .get("duration_seconds")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0);
            let (locked, problem) = manager.set_exam(id, on, message, duration_seconds).await?;
            Ok(json!({ "locked": locked, "problem": problem }))
        }
        "list_apps" => {
            let apps = manager.list_apps(arg_str(args, "device_id")?).await?;
            serde_json::to_value(apps).map_err(|e| e.to_string())
        }
        "launch_app" => {
            let id = arg_str(args, "device_id")?;
            let app_id = arg_u32(args, "app_id")?;
            let (name, started) = manager.launch_app(id, app_id).await?;
            Ok(json!({ "name": name, "started": started }))
        }
        "list_running" => {
            let running = manager.list_running(arg_str(args, "device_id")?).await?;
            serde_json::to_value(running).map_err(|e| e.to_string())
        }
        "close_app" => {
            let id = arg_str(args, "device_id")?;
            let pid = arg_u32(args, "pid")?;
            let closed = manager.close_app(id, pid).await?;
            Ok(json!({ "closed": closed }))
        }
        "set_blocklist" => {
            let programs = args
                .get("programs")
                .and_then(Value::as_array)
                .ok_or("missing array 'programs'")?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>();
            let count = programs.len();
            manager.set_blocklist(programs)?;
            Ok(json!({ "ok": true, "count": count }))
        }
        "start_recording" => {
            let info = manager
                .start_recording(arg_str(args, "device_id")?, proto::RecordOptions::default())
                .await?;
            serde_json::to_value(info).map_err(|e| e.to_string())
        }
        "stop_recording" => {
            let info = manager.stop_recording(arg_str(args, "device_id")?).await?;
            serde_json::to_value(info).map_err(|e| e.to_string())
        }
        "recording_status" => {
            let info = manager
                .recording_status(arg_str(args, "device_id")?)
                .await?;
            serde_json::to_value(info).map_err(|e| e.to_string())
        }
        "list_files" => {
            let dir = args.get("dir").and_then(Value::as_str).unwrap_or("");
            let entries = manager.list_files(arg_str(args, "device_id")?, dir).await?;
            serde_json::to_value(json!({ "entries": entries })).map_err(|e| e.to_string())
        }
        "send_file" => {
            use base64::Engine;
            let dir = args.get("dir").and_then(Value::as_str).unwrap_or("");
            let name = arg_str(args, "name")?;
            let data = base64::engine::general_purpose::STANDARD
                .decode(arg_str(args, "data_base64")?)
                .map_err(|e| format!("data_base64 is not valid base64: {e}"))?;
            let bytes = data.len();
            manager
                .send_file(arg_str(args, "device_id")?, dir, name, data)
                .await?;
            Ok(json!({ "ok": true, "bytes": bytes }))
        }
        "fetch_file" => {
            let saved = manager
                .download_file(arg_str(args, "device_id")?, arg_str(args, "path")?)
                .await?;
            Ok(json!({ "ok": true, "saved_to": saved }))
        }
        "list_workspace" => {
            let files = manager
                .workspace_manifest(arg_str(args, "device_id")?)
                .await?;
            serde_json::to_value(json!({ "files": files })).map_err(|e| e.to_string())
        }
        "delete_file" => {
            manager
                .delete_file(arg_str(args, "device_id")?, arg_str(args, "path")?)
                .await?;
            Ok(json!({ "ok": true }))
        }
        "clear_workspace" => {
            let removed = manager.clear_workspace(arg_str(args, "device_id")?).await?;
            Ok(json!({ "ok": true, "removed": removed }))
        }
        "preload_media" => {
            use base64::Engine;
            let name = arg_str(args, "name")?;
            let data = base64::engine::general_purpose::STANDARD
                .decode(arg_str(args, "data_base64")?)
                .map_err(|e| format!("data_base64 is not valid base64: {e}"))?;
            let bytes = data.len();
            manager
                .preload_media(arg_str(args, "device_id")?, name, data)
                .await?;
            Ok(json!({ "ok": true, "bytes": bytes }))
        }
        "play_media" => {
            let start_in_ms = args
                .get("start_in_ms")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0);
            let message = args.get("message").and_then(Value::as_str).unwrap_or("");
            let lock = args.get("lock").and_then(Value::as_bool).unwrap_or(false);
            let playing = manager
                .play_media(arg_str(args, "device_id")?, start_in_ms, message, lock)
                .await?;
            Ok(json!({ "playing": playing }))
        }
        "stop_media" => {
            manager.stop_media(arg_str(args, "device_id")?).await?;
            Ok(json!({ "playing": false }))
        }
        ASK_USER => Err("ask_user is handled by the orchestrator".into()),
        other => Err(format!("unknown tool '{other}'")),
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required string argument '{key}'"))
}

fn arg_u32(args: &Value, key: &str) -> Result<u32, String> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| format!("missing or out-of-range integer argument '{key}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_has_an_object_schema_and_ask_user_is_present() {
        let specs = specs();
        assert!(specs.iter().any(|s| s.name == ASK_USER));
        assert!(specs.iter().any(|s| s.name == "power_action"));
        for s in &specs {
            assert_eq!(
                s.parameters["type"], "object",
                "{} lacks object schema",
                s.name
            );
        }
    }

    #[test]
    fn arg_helpers_report_missing_rather_than_panic() {
        let empty = json!({});
        assert!(arg_str(&empty, "device_id").is_err());
        assert!(arg_u32(&empty, "app_id").is_err());
        let good = json!({ "device_id": "K7M2Q9", "app_id": 3 });
        assert_eq!(arg_str(&good, "device_id"), Ok("K7M2Q9"));
        assert_eq!(arg_u32(&good, "app_id"), Ok(3));
    }
}
