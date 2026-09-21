//! The MCP protocol layer: JSON-RPC 2.0 over stdio.
//!
//! One JSON object per line in, one per line out; logs go to stderr so they never corrupt the
//! protocol stream. The server is deliberately small — it handshakes, lists and calls tools, and
//! serves the operator skill as a prompt and a resource — and holds no per-request state, so the
//! order of calls never matters.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::fleet::Fleet;
use crate::tools;

/// The protocol version this server speaks. Clients that ask for a different one still work — MCP
/// negotiates by the server echoing its supported version.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// The operator skill, embedded at build time and served once as the `initialize` `instructions`
/// (and also as a prompt/resource), so a client sends it ahead of prompts without re-fetching it.
const SKILL: &str = include_str!("../SKILL.md");

/// Handles decoded JSON-RPC messages against the fleet.
pub struct Server {
    fleet: Arc<Fleet>,
}

impl Server {
    /// Wraps a loaded fleet.
    #[must_use]
    pub fn new(fleet: Arc<Fleet>) -> Self {
        Self { fleet }
    }

    /// Handles one line of input, returning the line of output to write, or `None` for a
    /// notification (which gets no reply). Never panics: malformed input becomes a JSON-RPC error.
    pub async fn handle_line(&self, line: &str) -> Option<String> {
        let request: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(e) => {
                return Some(render(error_object(
                    Value::Null,
                    -32700,
                    &format!("parse error: {e}"),
                )));
            }
        };

        // A request has an `id`; a notification does not, and gets no response.
        let id = request.get("id").cloned()?;
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = request.get("params").cloned().unwrap_or(Value::Null);

        let outcome = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::catalogue() })),
            "tools/call" => self.tools_call(&params).await,
            "prompts/list" => Ok(self.prompts_list()),
            "prompts/get" => self.prompts_get(&params),
            "resources/list" => Ok(self.resources_list()),
            "resources/read" => self.resources_read(&params),
            other => Err(RpcError {
                code: -32601,
                message: format!("method not found: {other}"),
            }),
        };

        Some(match outcome {
            Ok(result) => render(json!({ "jsonrpc": "2.0", "id": id, "result": result })),
            Err(e) => render(error_object(id, e.code, &e.message)),
        })
    }

    /// The `initialize` result: capabilities, server info, and the operator skill as `instructions`.
    fn initialize(&self, params: &Value) -> Value {
        // Echo the client's protocol version when it sent one we recognise; otherwise offer ours.
        let version = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(PROTOCOL_VERSION);
        json!({
            "protocolVersion": version,
            "capabilities": {
                "tools": {},
                "prompts": {},
                "resources": {},
            },
            "serverInfo": {
                "name": "cowatcher-mcp",
                "title": format!("{} MCP", proto::PRODUCT_NAME),
                "version": env!("CARGO_PKG_VERSION"),
            },
            "instructions": SKILL,
        })
    }

    /// Runs a tool. Tool failures come back as an `isError` result (data for the model), not a
    /// protocol error, so the agent can read the reason and adjust.
    async fn tools_call(&self, params: &Value) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid("tools/call needs a tool name"))?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        match tools::call(&self.fleet, name, &args).await {
            Ok(content) => Ok(json!({ "content": content, "isError": false })),
            Err(message) => Ok(json!({
                "content": [{ "type": "text", "text": message }],
                "isError": true,
            })),
        }
    }

    /// The one prompt: the operator skill, so a client can inject it ahead of a conversation.
    fn prompts_list(&self) -> Value {
        json!({
            "prompts": [{
                "name": "cowatcher_operator",
                "title": "Co-watcher operator brief",
                "description": "Standing instructions for driving the classroom safely and precisely.",
            }]
        })
    }

    fn prompts_get(&self, params: &Value) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if name != "cowatcher_operator" {
            return Err(RpcError {
                code: -32602,
                message: format!("no such prompt: {name}"),
            });
        }
        Ok(json!({
            "description": "Co-watcher operator brief",
            "messages": [{
                "role": "user",
                "content": { "type": "text", "text": SKILL },
            }],
        }))
    }

    /// Resources: the operator skill as a readable document.
    fn resources_list(&self) -> Value {
        json!({
            "resources": [{
                "uri": "cowatcher:///skill",
                "name": "Co-watcher operator skill",
                "description": "How to drive the classroom safely and precisely.",
                "mimeType": "text/markdown",
            }]
        })
    }

    fn resources_read(&self, params: &Value) -> Result<Value, RpcError> {
        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if uri != "cowatcher:///skill" {
            return Err(RpcError {
                code: -32602,
                message: format!("no such resource: {uri}"),
            });
        }
        Ok(json!({
            "contents": [{
                "uri": uri,
                "mimeType": "text/markdown",
                "text": SKILL,
            }]
        }))
    }
}

/// A JSON-RPC error to return to the client.
struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn invalid(message: &str) -> Self {
        Self {
            code: -32602,
            message: message.to_string(),
        }
    }
}

/// Builds a JSON-RPC error response object.
fn error_object(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

/// Serialises a response to a single line, falling back to a static error line if serialisation
/// somehow fails (it cannot for our own `json!` values, but we never unwrap in the hot path).
fn render(value: Value) -> String {
    serde_json::to_string(&value).unwrap_or_else(|_| {
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"internal serialisation error"}}"#
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        // A fleet is only touched by tool calls, which these tests avoid; load from a temp dir so no
        // network or real console state is needed.
        let dir = std::env::temp_dir().join(format!("cw-mcp-srv-{}", std::process::id()));
        let fleet = Fleet::load(&dir).expect("load fleet from empty dir");
        let _ = std::fs::remove_dir_all(&dir);
        Server::new(Arc::new(fleet))
    }

    #[tokio::test]
    async fn initialize_returns_capabilities_and_the_skill() {
        let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#;
        let out = server().handle_line(req).await.expect("a response");
        let value: Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(value["id"], json!(1));
        assert!(value["result"]["capabilities"]["tools"].is_object());
        let instructions = value["result"]["instructions"]
            .as_str()
            .expect("instructions string");
        assert!(instructions.contains("Co-watcher operator skill"));
    }

    #[tokio::test]
    async fn a_notification_gets_no_reply() {
        let note = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(server().handle_line(note).await.is_none());
    }

    #[tokio::test]
    async fn tools_list_includes_the_catalogue() {
        let req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let out = server().handle_line(req).await.expect("a response");
        let value: Value = serde_json::from_str(&out).expect("valid json");
        let tools = value["result"]["tools"].as_array().expect("tools array");
        assert!(!tools.is_empty());
    }

    #[tokio::test]
    async fn an_unknown_method_is_a_json_rpc_error() {
        let req = r#"{"jsonrpc":"2.0","id":3,"method":"does/not/exist"}"#;
        let out = server().handle_line(req).await.expect("a response");
        let value: Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(value["error"]["code"], json!(-32601));
    }

    #[tokio::test]
    async fn malformed_json_is_a_parse_error() {
        let out = server().handle_line("{not json").await.expect("a response");
        let value: Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(value["error"]["code"], json!(-32700));
    }

    #[tokio::test]
    async fn calling_an_unknown_tool_is_a_tool_error_not_a_crash() {
        let req = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#;
        let out = server().handle_line(req).await.expect("a response");
        let value: Value = serde_json::from_str(&out).expect("valid json");
        assert_eq!(value["result"]["isError"], json!(true));
    }
}
