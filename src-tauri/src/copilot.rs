//! Drives the GitHub Copilot CLI in server mode (`copilot --server --stdio`), speaking the
//! same JSON-RPC protocol as the official Copilot SDKs: LSP-style `Content-Length` framing,
//! `session.create`/`session.resume`/`session.send`, and `session.event` notifications.
//! One server process per turn, like the other providers.

use std::collections::{HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::watch;

use crate::agent::{AgentEvent, Outcome, SYSTEM_PROMPT};
use crate::error::{Error, Result};
use crate::mcp;
use crate::providers::ModelInfo;

const METHOD_NOT_FOUND: i64 = -32601;
const DETACH_TIMEOUT: Duration = Duration::from_secs(5);

fn spawn(bin: &Path, dir: Option<&Path>) -> Result<(Child, Conn)> {
    let mut cmd = Command::new(bin);
    cmd.args(["--server", "--stdio", "--no-auto-update"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::msg(format!("Could not start GitHub Copilot: {e}")))?;
    let conn = Conn {
        stdin: child.stdin.take().expect("piped stdin"),
        reader: BufReader::new(child.stdout.take().expect("piped stdout")),
        next_id: 0,
        queued: VecDeque::new(),
    };
    Ok((child, conn))
}

/// JSON-RPC 2.0 over `Content-Length` framed stdio.
struct Conn {
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: u64,
    /// Messages that arrived while waiting for a response, in order.
    queued: VecDeque<Value>,
}

impl Conn {
    async fn write(&mut self, message: &Value) -> Result<()> {
        let body = serde_json::to_vec(message).expect("json");
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes()).await?;
        self.stdin.write_all(&body).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// Reads one framed message; `None` when the server closed its output.
    async fn read(&mut self) -> Result<Option<Value>> {
        loop {
            let mut length = None;
            loop {
                let mut line = String::new();
                if self.reader.read_line(&mut line).await? == 0 {
                    return Ok(None);
                }
                let line = line.trim();
                if line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    if name.trim().eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse::<usize>().ok();
                    }
                }
            }
            let Some(length) = length else { continue };
            let mut body = vec![0; length];
            self.reader.read_exact(&mut body).await?;
            if let Ok(message) = serde_json::from_slice(&body) {
                return Ok(Some(message));
            }
        }
    }

    async fn next(&mut self) -> Result<Option<Value>> {
        match self.queued.pop_front() {
            Some(message) => Ok(Some(message)),
            None => self.read().await,
        }
    }

    async fn notify_request(&mut self, method: &str, params: Value) -> Result<u64> {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await?;
        Ok(id)
    }

    /// Sends a request and waits for its response, queueing everything else.
    async fn request(
        &mut self,
        method: &str,
        params: Value,
    ) -> std::result::Result<Value, RpcError> {
        let id = self.notify_request(method, params).await?;
        loop {
            let Some(message) = self.read().await? else {
                return Err(Error::msg("GitHub Copilot exited before answering.").into());
            };
            if message.get("method").is_some() || message["id"].as_u64() != Some(id) {
                self.queued.push_back(message);
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(RpcError::Remote {
                    code: error["code"].as_i64().unwrap_or_default(),
                    message: error["message"]
                        .as_str()
                        .unwrap_or("unknown error")
                        .to_string(),
                });
            }
            return Ok(message["result"].clone());
        }
    }

    async fn reply_unsupported(&mut self, id: &Value) -> Result<()> {
        self.write(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": METHOD_NOT_FOUND, "message": "Not supported by SlopSlide" },
        }))
        .await
    }

    /// The SDK handshake; servers predating `connect` answer `ping` instead.
    async fn handshake(&mut self) -> Result<()> {
        match self.request("connect", json!({})).await {
            Ok(_) => Ok(()),
            Err(RpcError::Remote { code, .. }) if code == METHOD_NOT_FOUND => self
                .request("ping", json!({}))
                .await
                .map(drop)
                .map_err(Into::into),
            Err(e) => Err(e.into()),
        }
    }
}

enum RpcError {
    Remote { code: i64, message: String },
    Local(Error),
}

impl From<Error> for RpcError {
    fn from(e: Error) -> Self {
        RpcError::Local(e)
    }
}

impl From<std::io::Error> for RpcError {
    fn from(e: std::io::Error) -> Self {
        RpcError::Local(e.into())
    }
}

impl From<RpcError> for Error {
    fn from(e: RpcError) -> Self {
        match e {
            RpcError::Remote { message, .. } => Error::msg(format!("GitHub Copilot: {message}")),
            RpcError::Local(e) => e,
        }
    }
}

/// Models the signed-in account may use, from `models.list`.
pub async fn list_models(bin: &Path) -> Result<Vec<ModelInfo>> {
    let (mut child, mut conn) = spawn(bin, None)?;
    conn.handshake().await?;
    let result = conn.request("models.list", json!({})).await?;
    let _ = child.kill().await;
    Ok(parse_models(&result["models"]))
}

/// Enabled models, first occurrence of each id only (the CLI may list one twice).
fn parse_models(models: &Value) -> Vec<ModelInfo> {
    let mut seen = HashSet::new();
    models
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(parse_model)
        .filter(|model| seen.insert(model.id.clone()))
        .collect()
}

fn parse_model(model: &Value) -> Option<ModelInfo> {
    if model["policy"]["state"] == "disabled" {
        return None;
    }
    let id = model["id"].as_str()?.to_string();
    Some(ModelInfo {
        label: model["name"].as_str().unwrap_or(&id).to_string(),
        is_default: false,
        efforts: model["supportedReasoningEfforts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| e.as_str().map(str::to_string))
            .collect(),
        default_effort: model["defaultReasoningEffort"].as_str().map(str::to_string),
        context_windows: Vec::new(),
        default_context_window: None,
        id,
    })
}

pub struct TurnArgs<'a> {
    pub bin: &'a Path,
    pub dir: &'a Path,
    /// This app's binary, which serves the `lint_deck` MCP tool (`mcp.rs`).
    pub lint_server: &'a Path,
    pub prompt: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub session: Option<&'a str>,
    /// Summarize the session's history instead of sending `prompt`.
    pub compact: bool,
}

/// Runs one turn. `on_session` persists the session id as soon as it is known.
pub async fn run_turn(
    args: TurnArgs<'_>,
    cancel: &mut watch::Receiver<bool>,
    emit: &(dyn Fn(&AgentEvent) + Sync),
    on_session: &(dyn Fn(&str) -> Result<()> + Sync),
) -> Result<Outcome> {
    let started = Instant::now();
    let (mut child, mut conn) = spawn(args.bin, Some(args.dir))?;
    conn.handshake().await?;

    let mut config = json!({
        "workingDirectory": args.dir,
        "systemMessage": { "mode": "append", "content": SYSTEM_PROMPT },
        "requestPermission": true,
        "streaming": true,
        "clientName": "SlopSlide",
        "mcpServers": {
            mcp::SERVER: {
                "type": "stdio",
                "command": args.lint_server,
                "args": [mcp::FLAG, args.dir],
                "tools": [mcp::TOOL, mcp::READ_NARRATION, mcp::WRITE_NARRATION],
            }
        },
    });
    if let Some(model) = args.model {
        config["model"] = json!(model);
    }
    if let Some(effort) = args.effort {
        config["reasoningEffort"] = json!(effort);
    }
    let session_id = match args.session {
        Some(session) => {
            config["sessionId"] = json!(session);
            match conn.request("session.resume", config).await {
                Ok(_) => session.to_string(),
                // The stored session is gone or unreadable: the caller starts fresh.
                Err(RpcError::Remote { .. }) => return Ok(Outcome::ResumeFailed),
                Err(e) => return Err(e.into()),
            }
        }
        None => {
            let result = conn.request("session.create", config).await?;
            result["sessionId"]
                .as_str()
                .ok_or_else(|| Error::msg("GitHub Copilot did not return a session id."))?
                .to_string()
        }
    };
    on_session(&session_id)?;
    emit(&AgentEvent::Started {
        session_id: Some(session_id.clone()),
    });

    let mut mapper = EventMapper::default();
    if args.compact {
        emit(&AgentEvent::Compacting);
        let result = tokio::select! {
            result = conn.request("session.history.compact", json!({ "sessionId": session_id })) => result,
            _ = cancel.changed() => {
                let _ = child.kill().await;
                return Ok(Outcome::Interrupted);
            }
        };
        // Compaction and usage events that arrived while it ran.
        let mut compacted = false;
        while let Some(message) = conn.queued.pop_front() {
            if message["method"] == "session.event" {
                for event in mapper.map(&message["params"]["event"]) {
                    compacted |= event == AgentEvent::Compacted;
                    emit(&event);
                }
            }
        }
        match result {
            Ok(result) if result["success"].as_bool() == Some(true) => {
                // Only when the server did not say so itself: it reports the new size after.
                if !compacted {
                    emit(&AgentEvent::Compacted);
                }
                emit(&AgentEvent::Result {
                    is_error: false,
                    text: None,
                    cost_usd: None,
                    duration_ms: Some(started.elapsed().as_millis() as u64),
                });
            }
            Ok(_) => emit(&AgentEvent::Error {
                message: "GitHub Copilot could not compact the conversation.".into(),
            }),
            Err(e) => emit(&AgentEvent::Error {
                message: Error::from(e).to_string(),
            }),
        }
        detach(&mut conn, &session_id).await;
        let _ = child.kill().await;
        return Ok(Outcome::Done);
    }

    emit(&AgentEvent::Thinking);
    conn.request(
        "session.send",
        json!({ "sessionId": session_id, "prompt": args.prompt }),
    )
    .await?;

    loop {
        let message = tokio::select! {
            message = conn.next() => message?,
            _ = cancel.changed() => {
                let _ = conn
                    .notify_request("session.abort", json!({ "sessionId": session_id }))
                    .await;
                let _ = child.kill().await;
                return Ok(Outcome::Interrupted);
            }
        };
        let Some(message) = message else {
            return Err(Error::msg("GitHub Copilot stopped unexpectedly."));
        };
        let method = message["method"].as_str();
        if method != Some("session.event") {
            // Server-initiated requests this client never opted into.
            if method.is_some() && message.get("id").is_some() {
                conn.reply_unsupported(&message["id"]).await?;
            }
            continue;
        }
        let event = &message["params"]["event"];
        if event["type"] == "permission.requested" {
            if let Some((request_id, result)) = permission_decision(&event["data"], args.dir) {
                conn.notify_request(
                    "session.permissions.handlePendingPermissionRequest",
                    json!({ "sessionId": session_id, "requestId": request_id, "result": result }),
                )
                .await?;
            }
            continue;
        }
        let idle = event["type"] == "session.idle";
        for mut event in mapper.map(event) {
            if let AgentEvent::Result { duration_ms, .. } = &mut event {
                *duration_ms = Some(started.elapsed().as_millis() as u64);
            }
            emit(&event);
        }
        if idle {
            break;
        }
    }

    detach(&mut conn, &session_id).await;
    let _ = child.kill().await;
    Ok(Outcome::Done)
}

/// Detaches so the CLI flushes the session to disk before exiting.
async fn detach(conn: &mut Conn, session_id: &str) {
    let _ = tokio::time::timeout(
        DETACH_TIMEOUT,
        conn.request("session.detach", json!({ "sessionId": session_id })),
    )
    .await;
}

/// Like the Claude provider's tool allowlist: read anything, write only inside the deck
/// folder, fetch URLs, lint the deck; no shell, other MCP servers, or extensions.
fn permission_decision(data: &Value, dir: &Path) -> Option<(String, Value)> {
    if data["resolvedByHook"].as_bool().unwrap_or(false) {
        return None;
    }
    let request_id = data["requestId"].as_str()?.to_string();
    let request = &data["permissionRequest"];
    let allowed = match request["kind"].as_str() {
        Some("read" | "url") => true,
        Some("mcp") => request["serverName"] == mcp::SERVER,
        Some("write") => request["fileName"]
            .as_str()
            .is_some_and(|file| is_inside(dir, Path::new(file))),
        _ => false,
    };
    let result = if allowed {
        json!({ "kind": "approve-once" })
    } else {
        json!({
            "kind": "reject",
            "feedback": "SlopSlide only allows reading files, editing files inside the deck folder, fetching URLs, and SlopSlide's lint/narration tools.",
        })
    };
    Some((request_id, result))
}

fn is_inside(dir: &Path, file: &Path) -> bool {
    let joined = if file.is_absolute() {
        file.to_path_buf()
    } else {
        dir.join(file)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                if !normalized.pop() {
                    return false;
                }
            }
            Component::CurDir => {}
            other => normalized.push(other),
        }
    }
    normalized.starts_with(dir)
}

/// Maps `session.event` payloads to UI events, using the Claude tool names the chat knows.
#[derive(Default)]
struct EventMapper {
    message_id: Option<String>,
    streamed: HashSet<String>,
    tools: HashSet<String>,
    last_message: Option<String>,
}

impl EventMapper {
    fn map(&mut self, event: &Value) -> Vec<AgentEvent> {
        let data = &event["data"];
        // Events from sub-agents are not shown.
        if data["parentToolCallId"].is_string() {
            return Vec::new();
        }
        match event["type"].as_str().unwrap_or_default() {
            "assistant.turn_start" | "assistant.reasoning_delta" | "assistant.reasoning" => {
                vec![AgentEvent::Thinking]
            }
            "assistant.message_delta" => {
                let text = data["deltaContent"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let id = data["messageId"].as_str().unwrap_or_default().to_string();
                let mut events = Vec::new();
                if self.message_id.as_deref() != Some(id.as_str()) {
                    self.message_id = Some(id.clone());
                    events.push(AgentEvent::TextStart);
                }
                self.streamed.insert(id);
                events.push(AgentEvent::TextDelta { text });
                events
            }
            "assistant.message" => {
                let content = data["content"].as_str().unwrap_or_default().to_string();
                let id = data["messageId"].as_str().unwrap_or_default().to_string();
                if !content.trim().is_empty() {
                    self.last_message = Some(content.clone());
                }
                if self.streamed.contains(&id) || content.trim().is_empty() {
                    return Vec::new();
                }
                self.message_id = Some(id);
                vec![
                    AgentEvent::TextStart,
                    AgentEvent::TextDelta { text: content },
                ]
            }
            "tool.execution_start" => {
                let name = data["toolName"].as_str().unwrap_or_default();
                if name == "report_intent" {
                    return Vec::new();
                }
                let id = data["toolCallId"].as_str().unwrap_or_default().to_string();
                self.tools.insert(id.clone());
                // A new message follows a tool call.
                self.message_id = None;
                let (name, input) = tool_presentation(name, &data["arguments"]);
                vec![AgentEvent::ToolUse { id, name, input }]
            }
            "tool.execution_complete" => {
                let id = data["toolCallId"].as_str().unwrap_or_default().to_string();
                if !self.tools.remove(&id) {
                    return Vec::new();
                }
                vec![AgentEvent::ToolResult {
                    id,
                    is_error: !data["success"].as_bool().unwrap_or(true),
                }]
            }
            "session.error" => vec![AgentEvent::Error {
                message: data["message"]
                    .as_str()
                    .unwrap_or("GitHub Copilot reported an error.")
                    .to_string(),
            }],
            // Sub-agents report their own, separate context.
            "session.usage_info" if !event["agentId"].is_string() => vec![AgentEvent::Usage {
                context_tokens: data["currentTokens"].as_u64(),
                context_window: data["tokenLimit"].as_u64(),
            }],
            "session.compaction_start" if !event["agentId"].is_string() => {
                vec![AgentEvent::Compacting]
            }
            "session.compaction_complete" if !event["agentId"].is_string() => {
                if data["success"].as_bool().unwrap_or(false) {
                    vec![AgentEvent::Compacted]
                } else {
                    vec![AgentEvent::Error {
                        message: data["error"]
                            .as_str()
                            .unwrap_or("GitHub Copilot could not compact the conversation.")
                            .to_string(),
                    }]
                }
            }
            "session.idle" => vec![AgentEvent::Result {
                is_error: false,
                text: self.last_message.take(),
                cost_usd: None,
                duration_ms: None,
            }],
            _ => Vec::new(),
        }
    }
}

fn tool_presentation(name: &str, args: &Value) -> (String, Value) {
    let path = ["path", "file_path", "filePath", "fileName"]
        .iter()
        .find_map(|key| args[key].as_str())
        .map(Value::from)
        .unwrap_or(Value::Null);
    match name {
        "view" | "read_file" => ("Read".into(), json!({ "file_path": path })),
        "create" | "write_file" => ("Write".into(), json!({ "file_path": path })),
        "edit" | "str_replace" | "str_replace_editor" | "apply_patch" => (
            "Edit".into(),
            json!({
                "file_path": path,
                "old_string": args["old_str"],
                "new_string": args["new_str"],
            }),
        ),
        "bash" | "shell" | "powershell" => ("Bash".into(), json!({ "command": args["command"] })),
        "grep" | "glob" => ("Grep".into(), json!({ "pattern": args["pattern"] })),
        "web_fetch" => ("WebFetch".into(), json!({ "url": args["url"] })),
        "web_search" => ("WebSearch".into(), json!({ "query": args["query"] })),
        _ => (name.to_string(), args.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_streamed_turn() {
        let mut mapper = EventMapper::default();
        let delta =
            json!({"type":"assistant.message_delta","data":{"messageId":"m1","deltaContent":"Hi"}});
        let full = json!({"type":"assistant.message","data":{"messageId":"m1","content":"Hi"}});
        let tool = json!({"type":"tool.execution_start","data":{"toolCallId":"t1","toolName":"edit","arguments":{"path":"/d/deck.html","old_str":"a","new_str":"b"}}});
        let done =
            json!({"type":"tool.execution_complete","data":{"toolCallId":"t1","success":true}});
        let idle = json!({"type":"session.idle","data":{}});
        assert_eq!(
            mapper.map(&delta),
            vec![
                AgentEvent::TextStart,
                AgentEvent::TextDelta { text: "Hi".into() }
            ]
        );
        assert!(mapper.map(&full).is_empty());
        assert!(matches!(
            &mapper.map(&tool)[..],
            [AgentEvent::ToolUse { name, input, .. }] if name == "Edit" && input["new_string"] == "b"
        ));
        assert_eq!(
            mapper.map(&done),
            vec![AgentEvent::ToolResult {
                id: "t1".into(),
                is_error: false
            }]
        );
        assert!(matches!(
            &mapper.map(&idle)[..],
            [AgentEvent::Result { text: Some(t), .. }] if t == "Hi"
        ));
    }

    #[test]
    fn maps_context_usage_and_compaction() {
        let mut mapper = EventMapper::default();
        let usage = json!({"type":"session.usage_info","data":{"tokenLimit":128000,"currentTokens":11591,"messagesLength":2}});
        assert_eq!(
            mapper.map(&usage),
            vec![AgentEvent::Usage {
                context_tokens: Some(11591),
                context_window: Some(128000)
            }]
        );
        let sub_agent = json!({"type":"session.usage_info","agentId":"a1","data":{"tokenLimit":1,"currentTokens":1}});
        assert!(mapper.map(&sub_agent).is_empty());
        assert_eq!(
            mapper.map(&json!({"type":"session.compaction_start","data":{"currentTokens":11805}})),
            vec![AgentEvent::Compacting]
        );
        assert_eq!(
            mapper.map(&json!({"type":"session.compaction_complete","data":{"success":true,"postCompactionTokens":539}})),
            vec![AgentEvent::Compacted]
        );
        assert_eq!(
            mapper.map(&json!({"type":"session.compaction_complete","data":{"success":false,"error":"rate limited"}})),
            vec![AgentEvent::Error {
                message: "rate limited".into()
            }]
        );
    }

    #[test]
    fn decides_permissions() {
        let dir = Path::new("/decks/pitch");
        let decide = |request: Value| {
            permission_decision(&json!({"requestId":"r","permissionRequest":request}), dir)
                .unwrap()
                .1["kind"]
                .clone()
        };
        assert_eq!(decide(json!({"kind":"read"})), "approve-once");
        assert_eq!(
            decide(json!({"kind":"write","fileName":"/decks/pitch/deck.html"})),
            "approve-once"
        );
        assert_eq!(
            decide(json!({"kind":"write","fileName":"deck.html"})),
            "approve-once"
        );
        assert_eq!(
            decide(json!({"kind":"write","fileName":"../other/deck.html"})),
            "reject"
        );
        assert_eq!(
            decide(json!({"kind":"shell","fullCommandText":"rm -rf /"})),
            "reject"
        );
        assert_eq!(
            decide(json!({"kind":"mcp","serverName":"slopslide","toolName":"lint_deck"})),
            "approve-once"
        );
        assert_eq!(
            decide(json!({"kind":"mcp","serverName":"github","toolName":"push"})),
            "reject"
        );
    }

    #[test]
    fn parses_models() {
        let enabled = json!({"id":"gpt-x","name":"GPT X","supportedReasoningEfforts":["low","high"],"defaultReasoningEffort":"high","policy":{"state":"enabled"}});
        let disabled = json!({"id":"gpt-y","name":"GPT Y","policy":{"state":"disabled"}});
        let model = parse_model(&enabled).unwrap();
        assert_eq!((model.label.as_str(), model.efforts.len()), ("GPT X", 2));
        assert!(parse_model(&disabled).is_none());
    }

    #[test]
    fn drops_duplicate_models() {
        let models = parse_models(&json!([
            {"id":"auto","name":"Auto","capabilities":{}},
            {"id":"auto","name":"Auto","capabilities":{}},
            {"id":"gpt-x","name":"GPT X"},
        ]));
        let ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["auto", "gpt-x"]);
        assert!(models[0].efforts.is_empty());
    }

    /// A stand-in `copilot --server --stdio` that answers by message order.
    #[cfg(unix)]
    #[test]
    fn lists_models_over_framed_stdio() {
        use std::os::unix::fs::PermissionsExt;

        let script = r#"#!/usr/bin/env python3
import json, sys
def read():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        line = line.strip()
        if not line: break
        name, _, value = line.partition(b":")
        if name.lower() == b"content-length": length = int(value)
    return json.loads(sys.stdin.buffer.read(length))
def send(message):
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()
request = read()
send({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32601, "message": "no connect"}})
request = read()
send({"jsonrpc": "2.0", "id": request["id"], "result": {"protocolVersion": 3}})
request = read()
send({"jsonrpc": "2.0", "method": "session.lifecycle", "params": {}})
send({"jsonrpc": "2.0", "id": request["id"], "result": {"models": [{"id": "m1", "name": "Model One"}]}})
read()
"#;
        let path =
            std::env::temp_dir().join(format!("slopslide-mock-copilot-{}", std::process::id()));
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let models = runtime.block_on(list_models(&path));
        let _ = std::fs::remove_file(&path);
        let models = models.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].label, "Model One");
    }

    /// A stand-in server that resumes a session and compacts it, logging each method.
    #[cfg(unix)]
    #[test]
    fn compacts_over_framed_stdio() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Mutex;

        let dir = std::env::temp_dir().join(format!("slopslide-compact-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = r#"#!/usr/bin/env python3
import json, sys
log = open("methods.log", "w")
def read():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line: sys.exit(0)
        line = line.strip()
        if not line: break
        name, _, value = line.partition(b":")
        if name.lower() == b"content-length": length = int(value)
    message = json.loads(sys.stdin.buffer.read(length))
    log.write(message.get("method", "") + "\n"); log.flush()
    return message
def send(message):
    body = json.dumps(message).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()
def event(kind, data):
    send({"jsonrpc": "2.0", "method": "session.event", "params": {"event": {"type": kind, "data": data}}})
while True:
    request = read()
    method = request["method"]
    if method == "session.history.compact":
        event("session.compaction_start", {"currentTokens": 90000})
        event("session.compaction_complete", {"success": True})
        event("session.usage_info", {"currentTokens": 12000, "tokenLimit": 128000, "messagesLength": 1})
        send({"jsonrpc": "2.0", "id": request["id"], "result": {"success": True, "tokensRemoved": 78000, "messagesRemoved": 9}})
    else:
        send({"jsonrpc": "2.0", "id": request["id"], "result": {}})
"#;
        let bin = dir.join("copilot");
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let events = Mutex::new(Vec::new());
        let (_tx, mut rx) = watch::channel(false);
        let args = TurnArgs {
            bin: &bin,
            dir: &dir,
            lint_server: Path::new("/app/slopslide"),
            prompt: "/compact",
            model: None,
            effort: None,
            session: Some("s1"),
            compact: true,
        };
        let outcome = runtime.block_on(run_turn(
            args,
            &mut rx,
            &|e| events.lock().unwrap().push(e.clone()),
            &|_| Ok(()),
        ));
        let methods = std::fs::read_to_string(dir.join("methods.log")).unwrap_or_default();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(matches!(outcome, Ok(Outcome::Done)));
        let methods: Vec<_> = methods.lines().collect();
        assert_eq!(
            methods,
            [
                "connect",
                "session.resume",
                "session.history.compact",
                "session.detach"
            ],
            "nothing is sent to the model"
        );
        let events = events.into_inner().unwrap();
        assert!(
            matches!(
                &events[..],
                [
                    AgentEvent::Started { .. },
                    AgentEvent::Compacting,
                    AgentEvent::Compacting,
                    AgentEvent::Compacted,
                    AgentEvent::Usage {
                        context_tokens: Some(12000),
                        context_window: Some(128000)
                    },
                    AgentEvent::Result {
                        is_error: false,
                        ..
                    },
                ]
            ),
            "{events:?}"
        );
    }
}
