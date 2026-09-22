//! `cowatcher-console web` — the console as a browser dashboard.
//!
//! The same Svelte UI that the Tauri window shows is served over HTTP here, so a teacher (or, on the
//! paid tier, a remote dashboard) can watch and drive a classroom from a browser. The front end talks
//! through a tiny bridge (`ui/src/lib/bridge.ts`) that turns every `invoke(cmd, args)` into
//! `POST /invoke/{cmd}` and every `listen(event)` into a message on the `/events` SSE stream — so the
//! components are byte-for-byte the same in both transports.
//!
//! **What is different in a browser:** the native H.264 viewer window (`open_viewer`) opens on the
//! *server* machine, so it is refused here with a clear message; file downloads land on the server
//! too. Everything else — the grid, control, restrictions, power, apps, recordings, broadcast,
//! settings, classrooms, Surey — works.
//!
//! **Security:** the server binds to loopback by default (only this machine's browser reaches it) and
//! every data call (`/invoke`, `/events`) requires a random per-run token. The startup line prints the
//! full URL with the token; opening it drops a cookie so the rest of the session is authenticated.
//! Binding to a non-loopback address is allowed but prints a plain warning first (D3/D10: a control
//! surface is never exposed unauthenticated).
//!
//! This dispatch mirrors the Tauri command surface in [`crate::gui`]; the heavy logic it calls lives in
//! [`crate::manager`], [`crate::broadcast`] and [`crate::ai`], so only the thin argument mapping is
//! written twice. Keep the two command lists in step when adding a command.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio_stream::StreamExt;

use crate::broadcast::Broadcasts;
use crate::events::{Emitter, WebEvent};
use crate::manager::{DeviceManager, DeviceStatus};

/// The built Svelte UI, baked into the binary so `cowatcher-console web` needs no extra files.
#[derive(rust_embed::Embed)]
#[folder = "ui/dist"]
struct Assets;

/// Everything a request needs. One instance is shared (behind `Arc`) across every connection.
struct WebState {
    manager: Arc<DeviceManager>,
    broadcasts: Broadcasts,
    ai: crate::ai::AiState,
    subscription: crate::subscription::Store,
    settings: crate::settings::Store,
    data_dir: PathBuf,
    base_dir: PathBuf,
    classroom: String,
    pairing_stop: Mutex<Option<Arc<tokio::sync::Notify>>>,
    /// Fan-out of out-of-band events to every connected browser's SSE stream.
    events: tokio::sync::broadcast::Sender<WebEvent>,
    /// The per-run access token every data call must present.
    token: String,
}

impl WebState {
    fn emitter(&self) -> Emitter {
        Emitter::Web(self.events.clone())
    }
}

/// Serves the dashboard until the process is stopped. `args` are the words after `web`.
pub async fn run(
    data_dir: PathBuf,
    base_dir: PathBuf,
    classroom: String,
    args: Vec<String>,
) -> Result<(), String> {
    let host: IpAddr = flag(&args, "--host")
        .and_then(|h| h.parse().ok())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));
    let port: u16 = flag(&args, "--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(8787);

    let manager = Arc::new(DeviceManager::load(&data_dir)?);
    let (events, _) = tokio::sync::broadcast::channel(256);
    let token = gen_token();
    let state = Arc::new(WebState {
        manager,
        broadcasts: Broadcasts::new(),
        ai: crate::ai::AiState::load(&data_dir),
        subscription: crate::subscription::Store::new(&data_dir),
        settings: crate::settings::Store::new(&base_dir),
        data_dir,
        base_dir,
        classroom,
        pairing_stop: Mutex::new(None),
        events,
        token,
    });

    let app = Router::new()
        .route("/invoke/{cmd}", post(invoke))
        .route("/events", get(sse))
        .route("/", get(index))
        .route("/{*path}", get(asset))
        .with_state(Arc::clone(&state));

    let addr = SocketAddr::new(host, port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("could not bind {addr}: {e}"))?;

    if !host.is_loopback() {
        eprintln!(
            "WARNING: serving the console on {addr} exposes classroom control to your network. \
             Only the token-carrying URL below can act; treat it like a password."
        );
    }
    println!(
        "Co-watcher web dashboard for classroom '{}' is running.\nOpen:  http://{}/?token={}",
        state.classroom,
        display_host(host, port),
        state.token
    );

    axum::serve(listener, app)
        .await
        .map_err(|e| format!("web server error: {e}"))
}

/// The value after a `--flag`, if present.
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// A URL-friendly host (loopback shows as `localhost`).
fn display_host(host: IpAddr, port: u16) -> String {
    if host.is_loopback() {
        format!("localhost:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// A random URL-safe token for this run.
fn gen_token() -> String {
    // Reuse the identity RNG shape: 24 bytes of randomness as hex is plenty and needs no new crate.
    let mut bytes = [0u8; 24];
    getrandom_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fills `buf` with OS randomness. Falls back to time-seeded bytes only if the OS RNG is unavailable,
/// which never happens on a supported desktop — the token still must not be predictable, so the OS RNG
/// is the real path.
fn getrandom_bytes(buf: &mut [u8]) {
    if getrandom::fill(buf).is_err() {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (seed >> (i % 16 * 8)) as u8 ^ (i as u8).wrapping_mul(31);
        }
    }
}

// ---- Auth --------------------------------------------------------------------------------------

/// True if the request carries this run's token, in the `cw_token` cookie, a `token` query parameter,
/// or a bearer header.
fn authed(state: &WebState, headers: &HeaderMap, query_token: Option<&str>) -> bool {
    if query_token == Some(state.token.as_str()) {
        return true;
    }
    if let Some(auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && auth.strip_prefix("Bearer ") == Some(state.token.as_str())
    {
        return true;
    }
    cookie_token(headers).as_deref() == Some(state.token.as_str())
}

/// Reads the `cw_token` value out of the Cookie header.
fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookies
        .split(';')
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == "cw_token")
        .map(|(_, v)| v.to_string())
}

// ---- Static UI ---------------------------------------------------------------------------------

/// Serves `index.html`, and, when the URL carries `?token=`, drops it into a cookie so every later
/// data call is authenticated.
async fn index(State(state): State<Arc<WebState>>, uri: axum::http::Uri) -> Response {
    let query_token = uri
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")));
    let body = Assets::get("index.html").map_or_else(
        || {
            "<!doctype html><meta charset=utf-8><p>The web UI has not been built. Run \
             <code>npm --prefix crates/console/ui run build</code> and start the server again."
                .to_string()
                .into_bytes()
        },
        |f| f.data.into_owned(),
    );
    let mut response = ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response();
    if let Some(token) = query_token
        && token == state.token
    {
        // HttpOnly keeps the token out of page JavaScript; SameSite=Strict blocks cross-site use.
        if let Ok(cookie) = header::HeaderValue::from_str(&format!(
            "cw_token={token}; Path=/; HttpOnly; SameSite=Strict"
        )) {
            response.headers_mut().append(header::SET_COOKIE, cookie);
        }
    }
    response
}

/// Serves one embedded asset by path, guessing its content type from the extension.
async fn asset(Path(path): Path<String>) -> Response {
    match Assets::get(&path) {
        Some(file) => {
            let mime = mime_for(&path);
            ([(header::CONTENT_TYPE, mime)], file.data.into_owned()).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        _ => "application/octet-stream",
    }
}

// ---- Events (SSE) ------------------------------------------------------------------------------

async fn sse(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Result<
    Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>>,
    StatusCode,
> {
    // In a browser the same-origin cookie authenticates the EventSource; a `?token=` is also accepted
    // so the stream works before the cookie is set and for non-browser clients.
    let query_token = uri
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")));
    if !authed(&state, &headers, query_token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let rx = state.events.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|item| {
        // A lagged receiver just skips the missed events; the next full poll re-syncs the UI.
        item.ok().and_then(|event| {
            serde_json::to_string(&event)
                .ok()
                .map(|data| Ok(Event::default().data(data)))
        })
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// ---- Command bridge ----------------------------------------------------------------------------

/// `POST /invoke/{cmd}` with a JSON object body of named arguments. Returns the command's JSON result,
/// or `{ "error": "…" }` with a 4xx/5xx so the bridge can reject the promise with a readable message.
async fn invoke(
    State(state): State<Arc<WebState>>,
    Path(cmd): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if !authed(&state, &headers, None) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let args: Value = if body.trim().is_empty() {
        json!({})
    } else {
        match serde_json::from_str(&body) {
            Ok(value) => value,
            Err(e) => return err_response(StatusCode::BAD_REQUEST, &format!("bad JSON body: {e}")),
        }
    };
    match dispatch(&state, &cmd, &args).await {
        Ok(value) => (
            [(header::CONTENT_TYPE, "application/json")],
            value.to_string(),
        )
            .into_response(),
        Err(message) => err_response(StatusCode::BAD_REQUEST, &message),
    }
}

fn err_response(code: StatusCode, message: &str) -> Response {
    (
        code,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "error": message }).to_string(),
    )
        .into_response()
}

/// One argument out of the JSON body (missing/null → the type's default handling; `Option<T>` is fine).
fn arg<T: DeserializeOwned>(args: &Value, key: &str) -> Result<T, String> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("argument '{key}': {e}"))
}

/// Serializes any command result into the JSON the bridge returns.
fn ok<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// Maps a command name and its arguments to the same work the Tauri command of that name does.
#[allow(clippy::too_many_lines)]
async fn dispatch(state: &WebState, cmd: &str, a: &Value) -> Result<Value, String> {
    let m = &state.manager;
    match cmd {
        // ---- identity, grid, previews ----
        "console_info" => ok(json!({ "device_id": m.device_id(), "public_key": m.public_key() })),
        "devices" => ok(m.devices()),
        "start_watching" => {
            m.start_watching().await?;
            ok(())
        }
        "stop_watching" => {
            m.stop_watching();
            ok(())
        }
        "preview_widths" => {
            let (grid, focused, quality) = m.preview_widths();
            ok(json!({ "grid": grid, "focused": focused, "quality": quality }))
        }
        "set_preview_widths" => {
            m.set_preview_widths(arg(a, "grid")?, arg(a, "focused")?, arg(a, "quality")?);
            ok(())
        }
        "set_focused" => {
            m.set_focused(arg::<Option<String>>(a, "deviceId")?.as_deref());
            ok(())
        }
        "set_monitor" => ok(m.set_monitor(&arg::<String>(a, "deviceId")?, arg(a, "monitor")?)?),
        "set_listening" => ok(m.set_listening(arg::<Option<String>>(a, "deviceId")?.as_deref())?),
        "listening" => ok(m.listening()),
        "wake" => ok(m.wake(&arg::<String>(a, "deviceId")?)?),
        "rename_device" => ok(m.rename(&arg::<String>(a, "deviceId")?, &arg::<String>(a, "name")?)?),

        // ---- control + input ----
        "set_controlling" => {
            ok(m.set_controlling(arg::<Option<String>>(a, "deviceId")?.as_deref())?)
        }
        "controlling" => ok(m.controlling()),
        "send_input" => {
            let events = crate::gui::to_wire(arg(a, "events")?);
            ok(m.queue_input(events)?)
        }
        "open_viewer" => Err(
            "the native full-resolution viewer opens on the machine running the console, so it is not \
             available from the web dashboard — use the in-page grid/opened preview instead"
                .into(),
        ),

        // ---- actions / power / restrictions ----
        "perform" => {
            let name: String = arg(a, "action")?;
            let delay: u16 = arg::<Option<u16>>(a, "delaySeconds")?.unwrap_or(0);
            let action = crate::manager::parse_action(&name, delay)
                .ok_or_else(|| format!("unknown action '{name}'"))?;
            ok(m.perform(arg::<Option<String>>(a, "deviceId")?.as_deref(), action)?)
        }
        "set_exam" => {
            let (locked, problem) = m
                .set_exam(&arg::<String>(a, "deviceId")?, arg(a, "on")?, &arg::<String>(a, "message")?)
                .await?;
            ok(json!([locked, problem]))
        }
        "set_screen_lock" => {
            let (locked, problem) = m
                .set_screen_lock(&arg::<String>(a, "deviceId")?, arg(a, "on")?)
                .await?;
            ok(json!([locked, problem]))
        }
        "set_wallpaper" => ok(set_wallpaper(state, a).await?),

        // ---- blocklist + room ----
        "blocklist" => ok(m.blocklist()),
        "set_blocklist" => ok(m.set_blocklist(arg(a, "programs")?)?),
        "room_info" => {
            let (name, password) = m.room();
            ok(json!({ "name": name, "password": password }))
        }
        "rename_room" => ok(m.rename_room(&arg::<String>(a, "name")?)?),
        "new_room_password" => ok(m.new_room_password()?),

        // ---- apps ----
        "list_apps" => ok(m.list_apps(&arg::<String>(a, "deviceId")?).await?),
        "list_running" => ok(m.list_running(&arg::<String>(a, "deviceId")?).await?),
        "launch_app" => ok(m
            .launch_app(&arg::<String>(a, "deviceId")?, arg(a, "id")?)
            .await
            .map(|(_, started)| started)?),
        "close_app" => ok(m
            .close_app(&arg::<String>(a, "deviceId")?, arg(a, "pid")?)
            .await?),
        "app_icon" => ok(icon(m.app_icon(&arg::<String>(a, "deviceId")?, arg(a, "id")?).await?)),
        "running_icon" => ok(icon(
            m.running_icon(&arg::<String>(a, "deviceId")?, arg(a, "pid")?)
                .await?,
        )),

        // ---- recordings ----
        "start_recording" => ok(m
            .start_recording(&arg::<String>(a, "deviceId")?, record_options(a)?)
            .await?),
        "stop_recording" => ok(m.stop_recording(&arg::<String>(a, "deviceId")?).await?),
        "recording_status" => ok(m.recording_status(&arg::<String>(a, "deviceId")?).await?),
        "list_recordings" => ok(m.list_recordings(&arg::<String>(a, "deviceId")?).await?),
        "download_recording" => ok(m
            .download_recording(&arg::<String>(a, "deviceId")?, &arg::<String>(a, "file")?)
            .await?),
        "recording_overview" => ok(recording_overview(state).await),
        "record_all" => ok(record_all(state, record_options(a)?).await),
        "stop_all_recording" => ok(stop_all_recording(state).await),

        // ---- broadcast ----
        "list_broadcast_sources" => ok(broadcast_sources().await?),
        "start_broadcast" => {
            state
                .broadcasts
                .start(
                    Arc::clone(&state.manager),
                    state.emitter(),
                    crate::broadcast::StartParams {
                        source_kind: arg(a, "sourceKind")?,
                        source_id: arg(a, "sourceId")?,
                        source_title: arg::<Option<String>>(a, "sourceTitle")?.unwrap_or_default(),
                        width: arg(a, "width")?,
                        locked: arg(a, "locked")?,
                        on_close: arg::<Option<String>>(a, "onClose")?.unwrap_or_default(),
                        targets: arg(a, "targets")?,
                    },
                )
                .await?;
            ok(())
        }
        "broadcast_status" => ok(state.broadcasts.status()),
        "stop_broadcast" => {
            state
                .broadcasts
                .stop(&state.manager, arg::<Option<u64>>(a, "id")?)
                .await;
            ok(())
        }

        // ---- pairing ----
        "begin_pairing" => ok(begin_pairing(state)),
        "stop_pairing" => {
            if let Some(stop) = state
                .pairing_stop
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
            {
                stop.notify_waiters();
            }
            ok(())
        }

        // ---- i18n ----
        "translation" => ok(translation(state, a)),
        "set_language" => {
            std::fs::create_dir_all(&state.data_dir).map_err(|e| e.to_string())?;
            std::fs::write(state.data_dir.join("language.txt"), arg::<String>(a, "code")?)
                .map_err(|e| e.to_string())?;
            ok(())
        }
        "export_language_template" => {
            let dir = state.data_dir.join("languages");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let path = dir.join("template.ini");
            std::fs::write(&path, crate::i18n::template()).map_err(|e| e.to_string())?;
            ok(path.display().to_string())
        }

        // ---- AI (Surey) ----
        "ai_config" => ok(state.ai.config_view()),
        "ai_set_config" => ok(ai_set_config(state, a)?),
        "ai_list_models" => ok(state.ai.list_models().await?),
        "ai_send" => ok(state
            .ai
            .run_turn(&state.emitter(), &state.manager, arg(a, "messages")?)
            .await?),
        "ai_choice_reply" => ok(state
            .ai
            .resolve_choice(&arg::<String>(a, "id")?, arg(a, "value")?)),
        "ai_transcribe" => ok(state
            .ai
            .transcribe(arg(a, "audio")?, arg(a, "filename")?)
            .await?),

        // ---- subscription + cloud ----
        "subscription_config" => ok(state.subscription.view()),
        "subscription_set" => ok(state
            .subscription
            .set(&arg::<String>(a, "dashboardUrl")?, arg(a, "license")?)?),
        "open_dashboard" => ok(state.subscription.open_dashboard()?),
        "cloud_status" => ok(sync_client(state).status()),
        "cloud_push" => {
            let (room_name, _) = m.room();
            let classroom_name = crate::classroom::label_of(&state.data_dir, &state.classroom);
            let snapshot = crate::cloud::ClassroomSnapshot::build(
                &state.classroom,
                &classroom_name,
                &room_name,
                &m.devices(),
                m.blocklist(),
            );
            ok(sync_client(state).push(&snapshot).await?)
        }
        "cloud_pull" => ok(sync_client(state).pull(&state.classroom).await?),

        // ---- classrooms + settings ----
        "classrooms" => ok(crate::classroom::list(&state.base_dir, &state.classroom)),
        "create_classroom" => ok(crate::classroom::create(&state.base_dir, &arg::<String>(a, "name")?)?),
        "switch_classroom" => Err(
            "open another classroom from the desktop app, or start a second `cowatcher-console web` on \
             a different port with `--classroom <slug>`"
                .into(),
        ),
        "settings_get" => ok(state.settings.get()),
        "settings_set" => ok(state.settings.set(&arg(a, "settings")?)?),

        other => Err(format!("unknown command '{other}'")),
    }
}

/// Turns the manager's `(w, h, bgra)` icon into the same JSON the Tauri `IconReply` produces.
fn icon(reply: Option<(u16, u16, Vec<u8>)>) -> Value {
    match reply {
        Some((width, height, bgra)) => json!({ "width": width, "height": height, "bgra": bgra }),
        None => Value::Null,
    }
}

/// Reads the flat recording options off the argument body.
fn record_options(a: &Value) -> Result<proto::RecordOptions, String> {
    Ok(proto::RecordOptions {
        max_width: arg(a, "maxWidth")?,
        max_height: arg(a, "maxHeight")?,
        fps: arg(a, "fps")?,
        codec: crate::gui::parse_codec(&arg::<String>(a, "codec")?),
        preset: crate::gui::parse_preset(&arg::<String>(a, "preset")?),
        quality: arg(a, "quality")?,
        bframes: arg(a, "bframes")?,
        scaler: crate::gui::parse_scaler(&arg::<String>(a, "scaler")?),
        two_pass: arg(a, "twoPass")?,
    })
}

/// Sets the wallpaper on the chosen (or all connected) PCs — mirrors [`crate::gui`]'s command.
async fn set_wallpaper(state: &WebState, a: &Value) -> Result<Value, String> {
    let image: Vec<u8> = arg(a, "image")?;
    if image.is_empty() {
        return Err("no image was chosen".into());
    }
    let fit = match arg::<String>(a, "fit")?.as_str() {
        "fit" => proto::WallpaperFit::Fit,
        "stretch" => proto::WallpaperFit::Stretch,
        "center" => proto::WallpaperFit::Center,
        "tile" => proto::WallpaperFit::Tile,
        _ => proto::WallpaperFit::Fill,
    };
    let targets: Vec<String> = arg(a, "targets")?;
    let connected: Vec<String> = state
        .manager
        .devices()
        .into_iter()
        .filter(|d| d.status == DeviceStatus::Live)
        .map(|d| d.device_id)
        .collect();
    let chosen: Vec<String> = if targets.is_empty() {
        connected
    } else {
        targets
            .into_iter()
            .filter(|id| connected.contains(id))
            .collect()
    };
    if chosen.is_empty() {
        return Err("no connected PCs to set the wallpaper on".into());
    }
    let (mut okc, mut failed) = (0u32, 0u32);
    for id in chosen {
        match state.manager.set_wallpaper(&id, image.clone(), fit).await {
            Ok((true, _)) => okc += 1,
            _ => failed += 1,
        }
    }
    Ok(json!({ "ok": okc, "failed": failed }))
}

/// Starts recording on every connected PC — mirrors [`crate::gui`]'s command.
async fn record_all(state: &WebState, options: proto::RecordOptions) -> Value {
    let (mut okc, mut failed) = (0u32, 0u32);
    for device in state.manager.devices() {
        if device.status != DeviceStatus::Live {
            continue;
        }
        match state
            .manager
            .start_recording(&device.device_id, options)
            .await
        {
            Ok(_) => okc += 1,
            Err(_) => failed += 1,
        }
    }
    json!({ "ok": okc, "failed": failed })
}

/// Stops recording on every connected PC — mirrors [`crate::gui`]'s command.
async fn stop_all_recording(state: &WebState) -> Value {
    let (mut okc, mut failed) = (0u32, 0u32);
    for device in state.manager.devices() {
        if device.status != DeviceStatus::Live {
            continue;
        }
        match state.manager.stop_recording(&device.device_id).await {
            Ok(_) => okc += 1,
            Err(_) => failed += 1,
        }
    }
    json!({ "ok": okc, "failed": failed })
}

/// Every PC's recording state at once — mirrors [`crate::gui`]'s command.
async fn recording_overview(state: &WebState) -> Vec<Value> {
    let mut out = Vec::new();
    for device in state.manager.devices() {
        let connected = device.status == DeviceStatus::Live;
        let (active, frames) = if connected {
            state
                .manager
                .recording_status(&device.device_id)
                .await
                .map(|info| (info.active, info.frames))
                .unwrap_or((false, 0))
        } else {
            (false, 0)
        };
        let recordings = if connected {
            state
                .manager
                .list_recordings(&device.device_id)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        out.push(json!({
            "device_id": device.device_id,
            "name": device.name,
            "active": active,
            "frames": frames,
            "connected": connected,
            "recordings": recordings,
        }));
    }
    out
}

/// Lists monitors and windows the teacher could present — mirrors [`crate::gui`]'s command.
async fn broadcast_sources() -> Result<Value, String> {
    tokio::task::spawn_blocking(|| {
        let mut out: Vec<Value> = Vec::new();
        if let Ok(mut capturer) = media::ThumbnailCapturer::new() {
            for monitor in capturer.monitors() {
                let thumb = capturer
                    .capture_jpeg(monitor.index, 320, proto::DEFAULT_THUMBNAIL_QUALITY)
                    .ok()
                    .map(|jpeg| format!("data:image/jpeg;base64,{}", crate::manager::base64(&jpeg)))
                    .unwrap_or_default();
                out.push(json!({
                    "kind": "monitor",
                    "id": u64::from(monitor.index),
                    "title": "",
                    "primary": monitor.primary,
                    "thumb": thumb,
                }));
            }
        }
        for window in media::window_capture::list_windows() {
            let thumb = media::window_capture::capture_window_jpeg(window.id, 320)
                .ok()
                .map(|jpeg| format!("data:image/jpeg;base64,{}", crate::manager::base64(&jpeg)))
                .unwrap_or_default();
            out.push(json!({
                "kind": "window",
                "id": window.id,
                "title": window.title,
                "primary": false,
                "thumb": thumb,
            }));
        }
        Value::Array(out)
    })
    .await
    .map_err(|e| e.to_string())
}

/// Starts a fresh continuous-pairing loop and returns the invite — mirrors [`crate::gui`]'s command,
/// forwarding each joined/failed PC as a `cowatcher://paired` / `cowatcher://pair-error` SSE event.
fn begin_pairing(state: &WebState) -> Value {
    let code = net::PairingCode::generate();
    if let Some(previous) = state
        .pairing_stop
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
    {
        previous.notify_waiters();
    }
    let stop = Arc::new(tokio::sync::Notify::new());
    *state.pairing_stop.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&stop));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, String>>();
    let manager = Arc::clone(&state.manager);
    tokio::spawn(async move { manager.pair_loop(code, stop, tx).await });
    let emitter = state.emitter();
    tokio::spawn(async move {
        while let Some(result) = rx.recv().await {
            match result {
                Ok(id) => emitter.emit("cowatcher://paired", id),
                Err(err) => emitter.emit("cowatcher://pair-error", err),
            }
        }
    });

    json!({
        "code": code.to_string(),
        "command": format!("cowatcher-agent pair {} {}", state.manager.public_key(), code),
    })
}

/// Resolves the language to show — mirrors [`crate::gui`]'s `translation` command.
fn translation(state: &WebState, a: &Value) -> Value {
    let dir = state.data_dir.join("languages");
    let (catalogs, problems) = crate::i18n::available(&dir);
    let code: Option<String> = a.get("code").and_then(Value::as_str).map(str::to_string);
    let system = a
        .get("system")
        .and_then(Value::as_str)
        .unwrap_or("")
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let saved = std::fs::read_to_string(state.data_dir.join("language.txt"))
        .ok()
        .map(|s| s.trim().to_owned());
    let wanted = code
        .or(saved)
        .filter(|c| catalogs.iter().any(|k| &k.code == c))
        .or_else(|| {
            catalogs
                .iter()
                .find(|k| k.code == system)
                .map(|k| k.code.clone())
        })
        .unwrap_or_else(|| "en".to_owned());
    let resolved = crate::i18n::resolve(&catalogs, &wanted);
    json!({
        "code": resolved.code,
        "strings": resolved.strings,
        "available": catalogs.into_iter().map(|c| json!({ "code": c.code, "name": c.name })).collect::<Vec<_>>(),
        "problems": problems,
        "languages_dir": dir.display().to_string(),
    })
}

/// Saves the AI provider choice — mirrors [`crate::gui`]'s command.
fn ai_set_config(state: &WebState, a: &Value) -> Result<(), String> {
    use crate::ai::provider::{ProviderConfig, ProviderKind};
    let kind = match arg::<String>(a, "kind")?.as_str() {
        "openai" => ProviderKind::OpenAi,
        "anthropic" => ProviderKind::Anthropic,
        "local" => ProviderKind::Local,
        _ => ProviderKind::Custom,
    };
    state.ai.set_config(
        ProviderConfig {
            kind,
            base_url: arg(a, "baseUrl")?,
            model: arg(a, "model")?,
        },
        arg(a, "key")?,
    )
}

fn sync_client(state: &WebState) -> crate::cloud::SyncClient {
    crate::cloud::SyncClient::new(
        state.subscription.view().dashboard_url,
        state.subscription.license(),
    )
}
