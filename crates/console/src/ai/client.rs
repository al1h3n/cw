//! Talking to the AI provider over HTTP, from Rust (never the web layer — D22).
//!
//! One internal message/tool representation is serialised into either the OpenAI chat-completions
//! shape or the native Anthropic Messages shape, so the same tool-calling loop drives OpenAI, an
//! OpenAI-compatible proxy, a local Ollama/LM Studio server, or Anthropic directly.

use serde_json::{Value, json};

use super::provider::ProviderConfig;

/// One message in the running conversation the model sees.
#[derive(Debug, Clone)]
pub enum Msg {
    /// The standing system brief (pre-prompt).
    System(String),
    /// Something the teacher said.
    User(String),
    /// The model's reply: prose and/or a batch of tool calls.
    Assistant {
        /// The assistant's text, if any.
        text: Option<String>,
        /// Tools the assistant asked to run.
        tool_calls: Vec<ToolCall>,
    },
    /// The result of running one tool, fed back to the model. Keyed by the call `id` (OpenAI's
    /// `tool_call_id` / Anthropic's `tool_use_id`); the tool name is not needed on the wire.
    ToolResult {
        /// The id of the tool call this answers.
        id: String,
        /// The tool's result as text (usually JSON).
        content: String,
    },
}

/// A tool the model asked to run.
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// Provider-assigned call id, echoed back with the result.
    pub id: String,
    /// The tool name.
    pub name: String,
    /// The parsed arguments object.
    pub arguments: Value,
}

/// A tool offered to the model (an OpenAI function / Anthropic tool schema).
#[derive(Debug, Clone)]
pub struct ToolSpec {
    /// Tool name.
    pub name: String,
    /// What it does (the model reads this).
    pub description: String,
    /// JSON Schema for its arguments.
    pub parameters: Value,
}

/// What the model produced this step: prose and/or tool calls.
#[derive(Debug, Clone, Default)]
pub struct Step {
    /// Any assistant text.
    pub text: Option<String>,
    /// Any tool calls to run.
    pub tool_calls: Vec<ToolCall>,
}

/// Runs one round-trip to the provider.
///
/// # Errors
/// Network failure, a non-success HTTP status, or a reply that does not match the expected shape.
pub async fn complete(
    client: &reqwest::Client,
    config: &ProviderConfig,
    key: Option<&str>,
    messages: &[Msg],
    tools: &[ToolSpec],
) -> Result<Step, String> {
    if config.kind.is_anthropic() {
        anthropic(client, config, key, messages, tools).await
    } else {
        openai(client, config, key, messages, tools).await
    }
}

/// OpenAI chat-completions (and every OpenAI-compatible endpoint).
async fn openai(
    client: &reqwest::Client,
    config: &ProviderConfig,
    key: Option<&str>,
    messages: &[Msg],
    tools: &[ToolSpec],
) -> Result<Step, String> {
    let url = format!("{}/chat/completions", config.trimmed_base());
    let mut body = json!({
        "model": config.model,
        "messages": openai_messages(messages),
    });
    if !tools.is_empty() {
        body["tools"] = openai_tools(tools);
        body["tool_choice"] = json!("auto");
    }
    let mut request = client.post(&url).json(&body);
    if let Some(key) = key {
        request = request.bearer_auth(key);
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
    let value = read_json(response).await?;
    let message = value
        .pointer("/choices/0/message")
        .ok_or("provider reply had no message")?;

    let text = message
        .get("content")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let raw_args = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let arguments = serde_json::from_str(raw_args).unwrap_or(json!({}));
            tool_calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
    }
    Ok(Step { text, tool_calls })
}

/// Native Anthropic Messages API.
async fn anthropic(
    client: &reqwest::Client,
    config: &ProviderConfig,
    key: Option<&str>,
    messages: &[Msg],
    tools: &[ToolSpec],
) -> Result<Step, String> {
    let url = format!("{}/messages", config.trimmed_base());
    let system: String = messages
        .iter()
        .filter_map(|m| match m {
            Msg::System(s) => Some(s.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut body = json!({
        "model": config.model,
        "max_tokens": 1024,
        "system": system,
        "messages": anthropic_messages(messages),
    });
    if !tools.is_empty() {
        body["tools"] = anthropic_tools(tools);
    }
    let mut request = client
        .post(&url)
        .header("anthropic-version", "2023-06-01")
        .json(&body);
    if let Some(key) = key {
        request = request.header("x-api-key", key);
    }
    let response = request.send().await.map_err(|e| e.to_string())?;
    let value = read_json(response).await?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    if let Some(blocks) = value.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(Value::as_str) {
                        text.push_str(t);
                    }
                }
                Some("tool_use") => tool_calls.push(ToolCall {
                    id: block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    arguments: block.get("input").cloned().unwrap_or(json!({})),
                }),
                _ => {}
            }
        }
    }
    Ok(Step {
        text: (!text.is_empty()).then_some(text),
        tool_calls,
    })
}

/// Reads a JSON body, turning an HTTP error into a readable message (including the provider's own
/// error text, which is where "bad model" / "bad key" actually shows up).
async fn read_json(response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .or_else(|| v.get("error"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or(text);
        return Err(format!("provider returned {status}: {detail}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("provider sent invalid JSON: {e}"))
}

fn openai_tools(tools: &[ToolSpec]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect(),
    )
}

fn openai_messages(messages: &[Msg]) -> Value {
    let mut out = Vec::new();
    for message in messages {
        match message {
            Msg::System(s) => out.push(json!({ "role": "system", "content": s })),
            Msg::User(s) => out.push(json!({ "role": "user", "content": s })),
            Msg::Assistant { text, tool_calls } => {
                let mut m = json!({ "role": "assistant", "content": text.clone() });
                if !tool_calls.is_empty() {
                    m["tool_calls"] = Value::Array(
                        tool_calls
                            .iter()
                            .map(|c| {
                                json!({
                                    "id": c.id,
                                    "type": "function",
                                    "function": {
                                        "name": c.name,
                                        "arguments": c.arguments.to_string(),
                                    }
                                })
                            })
                            .collect(),
                    );
                }
                out.push(m);
            }
            Msg::ToolResult { id, content } => out.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "content": content,
            })),
        }
    }
    Value::Array(out)
}

fn anthropic_tools(tools: &[ToolSpec]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect(),
    )
}

/// Anthropic messages: system is a top-level field (handled by the caller), and consecutive tool
/// results must be merged into one `user` message with several `tool_result` blocks.
fn anthropic_messages(messages: &[Msg]) -> Value {
    let mut out: Vec<Value> = Vec::new();
    let mut pending_results: Vec<Value> = Vec::new();

    let flush = |out: &mut Vec<Value>, results: &mut Vec<Value>| {
        if !results.is_empty() {
            out.push(json!({ "role": "user", "content": std::mem::take(results) }));
        }
    };

    for message in messages {
        match message {
            Msg::System(_) => {}
            Msg::User(s) => {
                flush(&mut out, &mut pending_results);
                out.push(json!({ "role": "user", "content": [{ "type": "text", "text": s }] }));
            }
            Msg::Assistant { text, tool_calls } => {
                flush(&mut out, &mut pending_results);
                let mut content = Vec::new();
                if let Some(t) = text {
                    content.push(json!({ "type": "text", "text": t }));
                }
                for c in tool_calls {
                    content.push(json!({
                        "type": "tool_use",
                        "id": c.id,
                        "name": c.name,
                        "input": c.arguments,
                    }));
                }
                out.push(json!({ "role": "assistant", "content": content }));
            }
            Msg::ToolResult { id, content } => pending_results.push(json!({
                "type": "tool_result",
                "tool_use_id": id,
                "content": content,
            })),
        }
    }
    flush(&mut out, &mut pending_results);
    Value::Array(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ToolSpec {
        ToolSpec {
            name: "lock".into(),
            description: "lock a pc".into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    #[test]
    fn openai_messages_carry_tool_calls_and_results() {
        let msgs = vec![
            Msg::System("brief".into()),
            Msg::User("lock pc 4".into()),
            Msg::Assistant {
                text: None,
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "lock".into(),
                    arguments: json!({ "device_id": "K7M2Q9" }),
                }],
            },
            Msg::ToolResult {
                id: "call_1".into(),
                content: "{\"ok\":true}".into(),
            },
        ];
        let v = openai_messages(&msgs);
        let arr = v.as_array().expect("array");
        assert_eq!(arr[0]["role"], "system");
        assert_eq!(arr[2]["tool_calls"][0]["function"]["name"], "lock");
        // Arguments are a JSON *string* in the OpenAI shape.
        assert!(arr[2]["tool_calls"][0]["function"]["arguments"].is_string());
        assert_eq!(arr[3]["role"], "tool");
        assert_eq!(arr[3]["tool_call_id"], "call_1");
    }

    #[test]
    fn anthropic_merges_consecutive_tool_results_into_one_user_turn() {
        let msgs = vec![
            Msg::User("lock 4 and 5".into()),
            Msg::Assistant {
                text: None,
                tool_calls: vec![
                    ToolCall {
                        id: "a".into(),
                        name: "lock".into(),
                        arguments: json!({}),
                    },
                    ToolCall {
                        id: "b".into(),
                        name: "lock".into(),
                        arguments: json!({}),
                    },
                ],
            },
            Msg::ToolResult {
                id: "a".into(),
                content: "ok".into(),
            },
            Msg::ToolResult {
                id: "b".into(),
                content: "ok".into(),
            },
        ];
        let v = anthropic_messages(&msgs);
        let arr = v.as_array().expect("array");
        // user, assistant(tool_use x2), then ONE user turn with two tool_result blocks.
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[2]["role"], "user");
        assert_eq!(arr[2]["content"].as_array().map(Vec::len), Some(2));
        assert_eq!(arr[2]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn tools_serialise_for_both_shapes() {
        let tools = [spec()];
        assert_eq!(openai_tools(&tools)[0]["type"], "function");
        assert_eq!(openai_tools(&tools)[0]["function"]["name"], "lock");
        assert_eq!(anthropic_tools(&tools)[0]["name"], "lock");
        assert!(anthropic_tools(&tools)[0]["input_schema"].is_object());
    }
}
