//! The MCP tool catalogue and its dispatch onto the fleet.
//!
//! Every tool maps directly onto a method of [`net::ControlSession`], which speaks the same closed,
//! typed protocol the Console uses. There is deliberately **no** "run a command" tool: an AI driving
//! this server can only do the named, reviewable things below, exactly like a teacher (AGENTS.md §5).

use base64::Engine;
use serde_json::{Value, json};

use crate::fleet::Fleet;

/// Builds one tool-descriptor for `tools/list`.
fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
        }
    })
}

/// A JSON schema fragment for a required `device_id` string.
fn device_id_prop() -> Value {
    json!({ "type": "string", "description": "Six-character device id from list_devices, e.g. K7M2Q9." })
}

/// The full tool catalogue returned by `tools/list`.
#[must_use]
pub fn catalogue() -> Value {
    json!([
        tool(
            "list_devices",
            "List every paired student PC (id and teacher-given name). Does not connect, so it does \
             not tell you which are online — use device_status for that.",
            json!({}),
            &[],
        ),
        tool(
            "device_status",
            "Connect to one PC and report whether it is online and which monitors it has. Use this to \
             confirm a PC is reachable before acting on it.",
            json!({ "device_id": device_id_prop() }),
            &["device_id"],
        ),
        tool(
            "screen_thumbnail",
            "Capture a JPEG thumbnail of a student's screen and return it as an image, so you can see \
             what the student is doing before deciding what to do.",
            json!({
                "device_id": device_id_prop(),
                "monitor": { "type": "integer", "minimum": 0, "description": "Monitor index (default 0)." },
                "max_width": { "type": "integer", "minimum": 64, "maximum": 3840, "description": "Max width in pixels (default 640)." }
            }),
            &["device_id"],
        ),
        tool(
            "list_apps",
            "List the programs a PC offers to launch (each has an id for launch_app).",
            json!({ "device_id": device_id_prop() }),
            &["device_id"],
        ),
        tool(
            "list_running",
            "List the programs currently running on a PC that a teacher may close (with their pids).",
            json!({ "device_id": device_id_prop() }),
            &["device_id"],
        ),
        tool(
            "launch_app",
            "Start one of the programs the PC published (by the id from list_apps). A PC can only run \
             what it itself published — there is no way to run an arbitrary path.",
            json!({
                "device_id": device_id_prop(),
                "app_id": { "type": "integer", "description": "An id from list_apps." }
            }),
            &["device_id", "app_id"],
        ),
        tool(
            "close_app",
            "Close a running program by its pid (from list_running). System-critical processes are \
             refused by the Agent.",
            json!({
                "device_id": device_id_prop(),
                "pid": { "type": "integer", "description": "A pid from list_running." }
            }),
            &["device_id", "pid"],
        ),
        tool(
            "set_blocklist",
            "Replace the set of program names blocked on a PC (an empty list clears it). Blocked \
             programs are closed if a student launches them.",
            json!({
                "device_id": device_id_prop(),
                "programs": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Executable names to block, e.g. [\"game.exe\", \"steam.exe\"]."
                }
            }),
            &["device_id", "programs"],
        ),
        tool(
            "perform_action",
            "Perform one power/lock action on a PC. DESTRUCTIVE actions (shutdown, reboot, log-off) \
             interrupt the student — confirm with the teacher first. Allowed actions: shutdown, \
             reboot, log-off, lock-screen, cancel-shutdown, lock-wallpaper, unlock-wallpaper.",
            json!({
                "device_id": device_id_prop(),
                "action": {
                    "type": "string",
                    "enum": ["shutdown", "reboot", "log-off", "lock-screen", "cancel-shutdown", "lock-wallpaper", "unlock-wallpaper"]
                },
                "delay_seconds": { "type": "integer", "minimum": 0, "maximum": 3600, "description": "Countdown for shutdown/reboot (default 0)." }
            }),
            &["device_id", "action"],
        ),
        tool(
            "set_exam",
            "Start or end exam lockdown on a PC: a fullscreen lock on a separate desktop the student \
             cannot Alt+Tab or Win-key away from. This seizes the student's screen — confirm intent.",
            json!({
                "device_id": device_id_prop(),
                "on": { "type": "boolean", "description": "True to lock, false to release." },
                "message": { "type": "string", "description": "Message shown on the lock (when on)." }
            }),
            &["device_id", "on"],
        ),
        tool(
            "set_wallpaper",
            "Set a PC's desktop wallpaper to an image you supply as base64 (PNG, JPEG or BMP bytes).",
            json!({
                "device_id": device_id_prop(),
                "image_base64": { "type": "string", "description": "The image file's bytes, base64-encoded." }
            }),
            &["device_id", "image_base64"],
        ),
        tool(
            "recording_status",
            "Report whether a PC is currently recording its screen, and the running frame count.",
            json!({ "device_id": device_id_prop() }),
            &["device_id"],
        ),
        tool(
            "start_recording",
            "Start recording a PC's screen to a file on that PC (default codec/size/rate). The teacher \
             collects the file later from the Console.",
            json!({
                "device_id": device_id_prop(),
                "monitor": { "type": "integer", "minimum": 0, "description": "Monitor index (default 0)." }
            }),
            &["device_id"],
        ),
        tool(
            "stop_recording",
            "Stop the recording on a PC and report the final state.",
            json!({ "device_id": device_id_prop() }),
            &["device_id"],
        ),
        tool(
            "list_recordings",
            "List the recordings already stored on a PC (file name and size).",
            json!({ "device_id": device_id_prop() }),
            &["device_id"],
        ),
    ])
}

/// One text content item for a tool result.
fn text(body: impl Into<String>) -> Value {
    json!({ "type": "text", "text": body.into() })
}

/// A pretty-printed JSON value as a text content item.
fn json_text(value: &Value) -> Value {
    text(serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()))
}

/// Reads a required string argument.
fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required string argument '{key}'"))
}

/// Reads a required integer argument as `u32`.
fn arg_u32(args: &Value, key: &str) -> Result<u32, String> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| format!("missing or out-of-range integer argument '{key}'"))
}

/// Dispatches one `tools/call`. `Ok(content)` is the content array; `Err(message)` becomes a tool
/// error the client shows the AI (it is data for the model, never a crash).
pub async fn call(fleet: &Fleet, name: &str, args: &Value) -> Result<Vec<Value>, String> {
    match name {
        "list_devices" => {
            let devices = fleet.devices();
            Ok(vec![json_text(&json!({ "devices": devices }))])
        }
        "device_status" => {
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let monitors = session
                .request_monitors()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![json_text(
                &json!({ "online": true, "monitors": monitors }),
            )])
        }
        "screen_thumbnail" => {
            let monitor = args
                .get("monitor")
                .and_then(Value::as_u64)
                .and_then(|n| u8::try_from(n).ok())
                .unwrap_or(0);
            let max_width = args
                .get("max_width")
                .and_then(Value::as_u64)
                .and_then(|n| u16::try_from(n).ok())
                .unwrap_or(640);
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let jpeg = session
                .request_thumbnail(monitor, max_width, proto::DEFAULT_THUMBNAIL_QUALITY)
                .await
                .map_err(|e| e.to_string())?;
            let data = base64::engine::general_purpose::STANDARD.encode(&jpeg);
            Ok(vec![
                json!({ "type": "image", "data": data, "mimeType": "image/jpeg" }),
            ])
        }
        "list_apps" => {
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let apps = session.request_apps().await.map_err(|e| e.to_string())?;
            Ok(vec![json_text(&json!({ "apps": apps }))])
        }
        "list_running" => {
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let running = session.request_running().await.map_err(|e| e.to_string())?;
            Ok(vec![json_text(&json!({ "running": running }))])
        }
        "launch_app" => {
            let id = arg_u32(args, "app_id")?;
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let (name, started) = session.launch_app(id).await.map_err(|e| e.to_string())?;
            Ok(vec![json_text(
                &json!({ "name": name, "started": started }),
            )])
        }
        "close_app" => {
            let pid = arg_u32(args, "pid")?;
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let closed = session.close_app(pid).await.map_err(|e| e.to_string())?;
            Ok(vec![json_text(&json!({ "closed": closed }))])
        }
        "set_blocklist" => {
            let programs = args
                .get("programs")
                .and_then(Value::as_array)
                .ok_or("missing required array argument 'programs'")?
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>();
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let (rules, closed) = session
                .set_blocklist(programs)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![json_text(
                &json!({ "rules": rules, "closed": closed }),
            )])
        }
        "perform_action" => {
            let action_name = arg_str(args, "action")?;
            let delay = args
                .get("delay_seconds")
                .and_then(Value::as_u64)
                .and_then(|n| u16::try_from(n).ok())
                .unwrap_or(0);
            let action = build_action(action_name, delay)
                .ok_or_else(|| format!("unknown action '{action_name}'"))?;
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let outcome = session.perform(action).await.map_err(|e| e.to_string())?;
            Ok(vec![json_text(
                &serde_json::to_value(outcome).unwrap_or(Value::Null),
            )])
        }
        "set_exam" => {
            let on = args
                .get("on")
                .and_then(Value::as_bool)
                .ok_or("missing required boolean argument 'on'")?;
            let message = args
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let (locked, problem) = session
                .set_exam(on, message)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![json_text(
                &json!({ "locked": locked, "problem": problem }),
            )])
        }
        "set_wallpaper" => {
            let image = base64::engine::general_purpose::STANDARD
                .decode(arg_str(args, "image_base64")?)
                .map_err(|e| format!("image_base64 is not valid base64: {e}"))?;
            if image.is_empty() {
                return Err("image_base64 decoded to no bytes".into());
            }
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let (ok, problem) = session
                .set_wallpaper(image)
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![json_text(&json!({ "ok": ok, "problem": problem }))])
        }
        "recording_status" => {
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let info = session
                .recording_status()
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![json_text(
                &serde_json::to_value(info).unwrap_or(Value::Null),
            )])
        }
        "start_recording" => {
            let monitor = args
                .get("monitor")
                .and_then(Value::as_u64)
                .and_then(|n| u8::try_from(n).ok())
                .unwrap_or(0);
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let info = session
                .start_recording(monitor, proto::RecordOptions::default())
                .await
                .map_err(|e| e.to_string())?;
            Ok(vec![json_text(
                &serde_json::to_value(info).unwrap_or(Value::Null),
            )])
        }
        "stop_recording" => {
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let info = session.stop_recording().await.map_err(|e| e.to_string())?;
            Ok(vec![json_text(
                &serde_json::to_value(info).unwrap_or(Value::Null),
            )])
        }
        "list_recordings" => {
            let mut session = fleet.connect(arg_str(args, "device_id")?).await?;
            let list = session.list_recordings().await.map_err(|e| e.to_string())?;
            Ok(vec![json_text(&json!({ "recordings": list }))])
        }
        other => Err(format!("unknown tool '{other}'")),
    }
}

/// Maps a tool action name to the typed, closed [`proto::Action`].
fn build_action(name: &str, delay_seconds: u16) -> Option<proto::Action> {
    use proto::Action;
    match name {
        "shutdown" => Some(Action::Shutdown { delay_seconds }),
        "reboot" => Some(Action::Reboot { delay_seconds }),
        "log-off" => Some(Action::LogOff),
        "lock-screen" => Some(Action::LockScreen),
        "cancel-shutdown" => Some(Action::CancelShutdown),
        "lock-wallpaper" => Some(Action::LockWallpaper),
        "unlock-wallpaper" => Some(Action::UnlockWallpaper),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_lists_the_expected_tools_with_schemas() {
        let cat = catalogue();
        let tools = cat.as_array().expect("array");
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        for expected in [
            "list_devices",
            "device_status",
            "screen_thumbnail",
            "launch_app",
            "perform_action",
            "set_wallpaper",
            "set_exam",
        ] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }
        // Every tool declares an object input schema.
        for t in tools {
            assert_eq!(
                t.pointer("/inputSchema/type").and_then(Value::as_str),
                Some("object"),
                "tool {:?} lacks an object inputSchema",
                t.get("name")
            );
        }
    }

    #[test]
    fn only_the_named_actions_resolve() {
        assert!(build_action("shutdown", 30).is_some());
        assert!(build_action("lock-screen", 0).is_some());
        assert!(build_action("unlock-wallpaper", 0).is_some());
        // Anything not on the closed list is refused, never guessed.
        assert!(build_action("format-c", 0).is_none());
        assert!(build_action("run", 0).is_none());
        assert!(build_action("", 0).is_none());
    }

    #[test]
    fn missing_arguments_are_reported_not_panicked() {
        let empty = json!({});
        assert!(arg_str(&empty, "device_id").is_err());
        assert!(arg_u32(&empty, "app_id").is_err());
        let good = json!({ "device_id": "K7M2Q9", "app_id": 4 });
        assert_eq!(arg_str(&good, "device_id"), Ok("K7M2Q9"));
        assert_eq!(arg_u32(&good, "app_id"), Ok(4));
    }
}
