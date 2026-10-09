//! Codex app-server transport. Provider policy and JSON-RPC stay out of the shared agent loop.
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, watch};

use crate::agent::{AgentEvent, Outcome, SYSTEM_PROMPT};
use crate::error::{Error, Result};
use crate::{env, mcp};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    #[default]
    Ask,
    AutoReview,
    FullAccess,
    Custom,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    Accept,
    AcceptForSession,
    Decline,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: String,
    pub title: String,
    pub reason: Option<String>,
    pub details: String,
    pub accept_label: String,
    pub decisions: Vec<Decision>,
}

struct Pending {
    deck_id: String,
    run_id: String,
    wire_id: Value,
    result: HashMap<DecisionKey, Value>,
    sender: mpsc::UnboundedSender<(String, Value)>,
}
// Wire decision names are also the keys for the responses offered by the server.
type DecisionKey = &'static str;
impl Decision {
    fn key(self) -> DecisionKey {
        match self {
            Self::Accept => "accept",
            Self::AcceptForSession => "acceptForSession",
            Self::Decline => "decline",
        }
    }
}

#[derive(Default)]
pub struct Approvals(Mutex<HashMap<String, Pending>>);
impl Approvals {
    pub fn respond(&self, deck_id: &str, id: &str, decision: Decision) -> Result<()> {
        let mut pending = self.0.lock().unwrap();
        let request = pending
            .get(id)
            .ok_or_else(|| Error::msg("This approval is no longer pending."))?;
        if request.deck_id != deck_id {
            return Err(Error::msg("This approval belongs to a different deck."));
        }
        let result = request
            .result
            .get(decision.key())
            .ok_or_else(|| Error::msg("Codex did not offer that decision."))?
            .clone();
        let request = pending.remove(id).expect("checked pending request");
        request
            .sender
            .send((id.into(), json!({"id":request.wire_id,"result":result})))
            .map_err(|_| Error::msg("This Codex turn has ended."))
    }

    pub fn cancel_deck(&self, deck_id: &str) {
        self.0.lock().unwrap().retain(|_, p| p.deck_id != deck_id);
    }

    fn resolved(&self, run_id: &str, wire_id: &Value) -> Option<String> {
        let mut pending = self.0.lock().unwrap();
        let id = pending
            .iter()
            .find(|(_, p)| p.run_id == run_id && p.wire_id == *wire_id)
            .map(|(id, _)| id.clone())?;
        pending.remove(&id);
        Some(id)
    }
}

struct PendingGuard {
    approvals: Arc<Approvals>,
    run_id: String,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.approvals
            .0
            .lock()
            .unwrap()
            .retain(|_, p| p.run_id != self.run_id);
    }
}

struct Server {
    _child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    buffered: VecDeque<Value>,
    active_turn: Option<(String, String)>,
}
impl Server {
    fn spawn(bin: &Path, cwd: Option<&Path>, lint_server: Option<&Path>) -> Result<Self> {
        let mut cmd = Command::new(bin);
        cmd.arg("app-server");
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        if let (Some(exe), Some(dir)) = (lint_server, cwd) {
            cmd.arg("-c")
                .arg(format!(
                    "mcp_servers.{}.command={}",
                    mcp::SERVER,
                    serde_json::to_string(&exe.to_string_lossy()).expect("serializable path")
                ))
                .arg("-c")
                .arg(format!(
                    "mcp_servers.{}.args={}",
                    mcp::SERVER,
                    json!([mcp::FLAG, dir.to_string_lossy()])
                ));
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::msg(format!("Could not start Codex app-server: {e}")))?;
        Ok(Self {
            stdin: child.stdin.take().expect("piped stdin"),
            lines: BufReader::new(child.stdout.take().expect("piped stdout")).lines(),
            _child: child,
            next_id: 0,
            buffered: VecDeque::new(),
            active_turn: None,
        })
    }
    async fn send(&mut self, value: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(value).expect("serializable JSON");
        bytes.push(b'\n');
        self.stdin.write_all(&bytes).await?;
        self.stdin.flush().await?;
        Ok(())
    }
    async fn next(&mut self) -> Result<Value> {
        if let Some(v) = self.buffered.pop_front() {
            return Ok(v);
        }
        self.read().await
    }
    async fn read(&mut self) -> Result<Value> {
        loop {
            let line =
                self.lines.next_line().await?.ok_or_else(|| {
                    Error::msg("Codex app-server exited before finishing the turn.")
                })?;
            if let Ok(v) = serde_json::from_str(&line) {
                return Ok(v);
            }
        }
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({"id":id,"method":method,"params":params}))
            .await?;
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let v = self.read().await?;
                if v.get("method").is_some() {
                    self.buffered.push_back(v);
                    continue;
                }
                if v["id"] != id {
                    continue;
                }
                if v.get("error").is_some() {
                    return Err(Error::msg(format!(
                        "Codex {method}: {}",
                        v["error"]["message"].as_str().unwrap_or("request failed")
                    )));
                }
                return Ok(v["result"].clone());
            }
        })
        .await
        .map_err(|_| Error::msg(format!("Codex {method} timed out.")))?
    }
    async fn reject_unknown(&mut self, v: &Value) -> Result<()> {
        self.send(&json!({"id":v["id"],"error":{"code":-32601,"message":"SlopSlide does not support this interactive request; no permission was granted."}})).await
    }
    async fn initialize(&mut self) -> Result<()> {
        self.request("initialize",json!({"clientInfo":{"name":"slopslide","title":"SlopSlide","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
        self.send(&json!({"method":"initialized"})).await
    }
    async fn configuration(&mut self, dir: &Path) -> Result<(Value, Value)> {
        let config = self.request("config/read", json!({"cwd":dir})).await?;
        let requirements = self.request("configRequirements/read", Value::Null).await?;
        Ok((
            config["config"].clone(),
            requirements["requirements"].clone(),
        ))
    }
}

fn allowed(requirements: &Value, key: &str, value: &str) -> bool {
    requirements[key]
        .as_array()
        .map_or(true, |a| a.iter().any(|v| v == value))
}
fn supported_modes(config: &Value, requirements: &Value) -> Vec<PermissionMode> {
    let reviewer = config.get("approvals_reviewer").is_some();
    let mut modes = Vec::new();
    if allowed(requirements, "allowedSandboxModes", "workspace-write")
        && allowed(requirements, "allowedApprovalPolicies", "on-request")
    {
        if !reviewer || allowed(requirements, "allowedApprovalsReviewers", "user") {
            modes.push(PermissionMode::Ask);
        }
        if reviewer && allowed(requirements, "allowedApprovalsReviewers", "auto_review") {
            modes.push(PermissionMode::AutoReview);
        }
    }
    if allowed(requirements, "allowedSandboxModes", "danger-full-access")
        && allowed(requirements, "allowedApprovalPolicies", "never")
        && (!reviewer || allowed(requirements, "allowedApprovalsReviewers", "user"))
    {
        modes.push(PermissionMode::FullAccess);
    }
    modes.push(PermissionMode::Custom);
    modes
}

pub async fn permission_modes(dir: &Path) -> Result<Vec<PermissionMode>> {
    let bin = env::resolve_codex().ok_or_else(|| Error::msg("Codex is not installed."))?;
    let mut server = Server::spawn(&bin, Some(dir), None)?;
    server.initialize().await?;
    let (config, requirements) = server.configuration(dir).await?;
    Ok(supported_modes(&config, &requirements))
}

fn thread_params(args: &TurnArgs<'_>, config: &Value) -> Value {
    let mut p = json!({"cwd":args.dir,"developerInstructions":SYSTEM_PROMPT});
    if let Some(model) = args.model {
        p["model"] = json!(model);
    }
    // Custom must explicitly restore current config when resuming a previously unrestricted thread.
    let (sandbox, policy, reviewer) = match args.mode {
        PermissionMode::Ask => (json!("workspace-write"), json!("on-request"), json!("user")),
        PermissionMode::AutoReview => (
            json!("workspace-write"),
            json!("on-request"),
            json!("auto_review"),
        ),
        PermissionMode::FullAccess => (json!("danger-full-access"), json!("never"), json!("user")),
        PermissionMode::Custom => (
            config["sandbox_mode"]
                .as_str()
                .map_or(json!("read-only"), |v| json!(v)),
            config
                .get("approval_policy")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or(json!("on-request")),
            config
                .get("approvals_reviewer")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or(json!("user")),
        ),
    };
    p["sandbox"] = sandbox;
    p["approvalPolicy"] = policy;
    if config.get("approvals_reviewer").is_some() {
        p["approvalsReviewer"] = reviewer;
    }
    if args.mode != PermissionMode::Custom && args.mode != PermissionMode::FullAccess {
        p["config"] = json!({"sandbox_workspace_write.network_access":false});
    }
    p
}

pub struct TurnArgs<'a> {
    pub bin: &'a Path,
    pub dir: &'a Path,
    pub lint_server: &'a Path,
    pub deck_id: &'a str,
    pub prompt: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub session: Option<&'a str>,
    pub mode: PermissionMode,
    pub approvals: Arc<Approvals>,
}

pub async fn run_turn(
    args: TurnArgs<'_>,
    cancel: &mut watch::Receiver<bool>,
    emit: &(dyn Fn(&AgentEvent) + Sync),
    on_session: &(dyn Fn(&str) -> Result<()> + Sync),
) -> Result<Outcome> {
    let mut server = Server::spawn(args.bin, Some(args.dir), Some(args.lint_server))?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let _guard = PendingGuard {
        approvals: args.approvals.clone(),
        run_id: run_id.clone(),
    };
    // Cancellation covers initialization/resume as well as generation and approval waits.
    if *cancel.borrow() {
        return Ok(Outcome::Interrupted);
    }
    tokio::select! {
        biased;
        _ = cancel.changed() => {
            // Give Codex a chance to stop its tools before killing the server process.
            if let Some((thread, turn)) = server.active_turn.clone() {
                let _ = tokio::time::timeout(Duration::from_secs(2), async {
                    server.request("turn/interrupt", json!({"threadId":thread,"turnId":turn})).await?;
                    loop {
                        let v = server.next().await?;
                        if v["method"] == "turn/completed" && v["params"]["turn"]["id"] == turn {
                            return Ok::<(), Error>(());
                        }
                    }
                }).await;
            }
            Ok(Outcome::Interrupted)
        },
        result = connected_turn(&mut server, &args, &run_id, emit, on_session) => result,
    }
    // Server's kill_on_drop closes the transport, including any pending approvals.
}

async fn connected_turn(
    server: &mut Server,
    args: &TurnArgs<'_>,
    run_id: &str,
    emit: &(dyn Fn(&AgentEvent) + Sync),
    on_session: &(dyn Fn(&str) -> Result<()> + Sync),
) -> Result<Outcome> {
    server.initialize().await?;
    let (config, requirements) = server.configuration(args.dir).await?;
    if !supported_modes(&config, &requirements).contains(&args.mode) {
        return Err(Error::msg(
            "This permission mode is unavailable in your Codex configuration. Select another mode.",
        ));
    }
    let mut params = thread_params(args, &config);
    let method = if let Some(id) = args.session {
        params["threadId"] = json!(id);
        "thread/resume"
    } else {
        "thread/start"
    };
    let response = match server.request(method, params).await {
        Ok(v) => v,
        Err(e) if args.session.is_some() && missing_thread(&e.to_string()) => {
            return Ok(Outcome::ResumeFailed)
        }
        Err(e) => return Err(e),
    };
    let thread = response["thread"]["id"]
        .as_str()
        .ok_or_else(|| Error::msg("Codex returned no thread ID."))?
        .to_string();
    on_session(&thread)?;
    emit(&AgentEvent::Started {
        session_id: Some(thread.clone()),
    });
    let started = Instant::now();
    let response = server
        .request(
            "turn/start",
            json!({
                "threadId": thread,
                "input": [{"type": "text", "text": args.prompt}],
                "effort": args.effort,
            }),
        )
        .await?;
    let turn = response["turn"]["id"]
        .as_str()
        .ok_or_else(|| Error::msg("Codex returned no turn ID."))?
        .to_string();
    server.active_turn = Some((thread.clone(), turn.clone()));
    let mut mapper = EventMapper::default();
    let (tx, mut rx) = mpsc::unbounded_channel();
    loop {
        let message = tokio::select! {
            biased;
            Some((id, answer)) = rx.recv() => {
                server.send(&answer).await?;
                emit(&AgentEvent::ApprovalResolved { id });
                continue;
            }
            message = server.next() => message?,
        };
        let method = message["method"].as_str().unwrap_or_default();
        let p = &message["params"];
        let request = message.get("id").is_some() && message.get("method").is_some();
        let other_thread = p["threadId"].as_str().is_some_and(|id| id != thread);
        let other_turn = p["turnId"].as_str().is_some_and(|id| id != turn);
        if other_thread || other_turn {
            if request {
                server.reject_unknown(&message).await?;
            }
            continue;
        }
        if request {
            // MCP elicitation is scoped to a thread; its turn correlation is optional.
            let scoped_mcp = method == "mcpServer/elicitation/request" && p["turnId"].is_null();
            if p["threadId"] != thread || (p["turnId"] != turn && !scoped_mcp) {
                server.reject_unknown(&message).await?;
                continue;
            }
            if let Some((approval, result)) =
                approval_request(method, &mapper.approval_details(method, p))
            {
                args.approvals.0.lock().unwrap().insert(
                    approval.id.clone(),
                    Pending {
                        deck_id: args.deck_id.into(),
                        run_id: run_id.into(),
                        wire_id: message["id"].clone(),
                        result,
                        sender: tx.clone(),
                    },
                );
                emit(&AgentEvent::ApprovalRequested { approval });
            } else {
                server.reject_unknown(&message).await?;
                emit(&AgentEvent::Error { message: format!("Codex requested an unsupported interaction ({method}); no permission was granted.") });
            }
            continue;
        }
        if method == "serverRequest/resolved" {
            if let Some(id) = args.approvals.resolved(run_id, &p["requestId"]) {
                emit(&AgentEvent::ApprovalResolved { id });
            }
        }
        for event in mapper.map(method, p) {
            emit(&event);
        }
        if method == "turn/completed" && p["turn"]["id"] == turn {
            let status = p["turn"]["status"].as_str().unwrap_or("failed");
            if status == "interrupted" {
                return Ok(Outcome::Interrupted);
            }
            let text = (status != "completed").then(|| {
                p["turn"]["error"]["message"]
                    .as_str()
                    .unwrap_or("Codex turn failed.")
                    .into()
            });
            emit(&AgentEvent::Result {
                is_error: status != "completed",
                text,
                cost_usd: None,
                duration_ms: Some(started.elapsed().as_millis() as u64),
            });
            return Ok(Outcome::Done);
        }
    }
}

fn missing_thread(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("no rollout found")
        || m.contains("thread not found")
        || m.contains("no conversation found")
}

fn approval_request(method: &str, p: &Value) -> Option<(Approval, HashMap<DecisionKey, Value>)> {
    if method == "mcpServer/elicitation/request" {
        return mcp_approval_request(p);
    }
    let permissions = method == "item/permissions/requestApproval";
    let (title, mut details) = match method {
        "item/commandExecution/requestApproval" => {
            if !p["networkApprovalContext"].is_null() {
                (
                    "Network access",
                    serde_json::to_string_pretty(&p["networkApprovalContext"]).ok()?,
                )
            } else {
                (
                    "Run a command",
                    p["command"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| serde_json::to_string_pretty(p).unwrap_or_default()),
                )
            }
        }
        "item/fileChange/requestApproval" => {
            ("Change files", serde_json::to_string_pretty(p).ok()?)
        }
        "item/permissions/requestApproval" => (
            "Additional permissions",
            serde_json::to_string_pretty(&p["permissions"]).ok()?,
        ),
        _ => return None,
    };
    if method == "item/commandExecution/requestApproval" {
        // A command approval may also grant access or send stdin to an existing process.
        // Keep that scope visible rather than displaying only the command text.
        let mut context = serde_json::Map::new();
        for key in ["cwd", "kind", "additionalPermissions"] {
            if let Some(value) = p.get(key).filter(|v| !v.is_null()) {
                context.insert(key.into(), value.clone());
            }
        }
        if !p["networkApprovalContext"].is_null() && !p["command"].is_null() {
            context.insert("command".into(), p["command"].clone());
        }
        if !context.is_empty() {
            details.push_str("\n\n");
            details.push_str(&serde_json::to_string_pretty(&context).ok()?);
        }
    }
    let choices = [
        Decision::Accept,
        Decision::AcceptForSession,
        Decision::Decline,
    ];
    let mut result = HashMap::new();
    let mut decisions = Vec::new();
    for d in choices {
        let offered = if permissions {
            d != Decision::AcceptForSession
        } else {
            p["availableDecisions"]
                .as_array()
                .map_or(true, |a| a.iter().any(|v| v == d.key()))
        };
        if !offered {
            continue;
        }
        let value = if permissions {
            json!({"permissions":if d==Decision::Accept {p["permissions"].clone()}else{json!({})},"scope":"turn"})
        } else {
            json!({"decision":d.key()})
        };
        result.insert(d.key(), value);
        decisions.push(d);
    }
    Some((
        Approval {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.into(),
            reason: p["reason"].as_str().map(str::to_string),
            details,
            accept_label: if permissions {
                "Allow for this turn"
            } else {
                "Allow once"
            }
            .into(),
            decisions,
        },
        result,
    ))
}

/// Codex wraps MCP tool approvals in empty forms. These are confirmations, not input forms.
fn mcp_approval_request(p: &Value) -> Option<(Approval, HashMap<DecisionKey, Value>)> {
    let meta = &p["_meta"];
    let schema = &p["requestedSchema"];
    if p["mode"] != "form"
        || meta["codex_approval_kind"] != "mcp_tool_call"
        || meta["codex_requires_user_input"] == true
        || schema["type"] != "object"
        || !schema["properties"].as_object()?.is_empty()
        || schema["required"].as_array().is_some_and(|a| !a.is_empty())
    {
        return None;
    }
    let server = p["serverName"].as_str()?;
    let message = p["message"].as_str()?;
    let mut decisions = vec![Decision::Accept];
    let mut result = HashMap::from([
        ("accept", json!({"action":"accept","content":{}})),
        ("decline", json!({"action":"decline"})),
    ]);
    let persist = &meta["persist"];
    if persist == "session"
        || persist
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "session"))
    {
        decisions.push(Decision::AcceptForSession);
        result.insert(
            "acceptForSession",
            json!({"action":"accept","content":{},"_meta":{"persist":"session"}}),
        );
    }
    decisions.push(Decision::Decline);
    Some((
        Approval {
            id: uuid::Uuid::new_v4().to_string(),
            title: "Use an MCP tool".into(),
            reason: Some(message.into()),
            details: serde_json::to_string_pretty(&json!({"server":server,"tool":meta})).ok()?,
            accept_label: "Allow once".into(),
            decisions,
        },
        result,
    ))
}

#[derive(Default)]
struct EventMapper {
    text_started: HashSet<String>,
    streamed: HashSet<String>,
    tools: HashSet<String>,
    items: HashMap<String, Value>,
}
impl EventMapper {
    fn approval_details(&self, method: &str, p: &Value) -> Value {
        let mut details = p.clone();
        if method == "item/fileChange/requestApproval" {
            if let Some(item) = p["itemId"].as_str().and_then(|id| self.items.get(id)) {
                details["changes"] = item["changes"].clone();
            }
        }
        details
    }
    fn map(&mut self, method: &str, p: &Value) -> Vec<AgentEvent> {
        match method {
            "turn/started" => vec![AgentEvent::Thinking],
            "item/agentMessage/delta" => {
                let id = p["itemId"].as_str().unwrap_or_default().to_string();
                self.streamed.insert(id.clone());
                let mut e = Vec::new();
                if self.text_started.insert(id) {
                    e.push(AgentEvent::TextStart);
                }
                if let Some(text) = p["delta"].as_str() {
                    e.push(AgentEvent::TextDelta { text: text.into() });
                }
                e
            }
            "item/started" | "item/completed" => {
                let item = &p["item"];
                let id = item["id"].as_str().unwrap_or_default().to_string();
                let complete = method == "item/completed";
                if !complete {
                    self.items.insert(id.clone(), item.clone());
                }
                if item["type"] == "agentMessage" {
                    if complete && !self.streamed.contains(&id) {
                        return vec![
                            AgentEvent::TextStart,
                            AgentEvent::TextDelta {
                                text: item["text"].as_str().unwrap_or_default().into(),
                            },
                        ];
                    }
                    return Vec::new();
                }
                let Some((name, input)) = tool(item) else {
                    return Vec::new();
                };
                let mut events = Vec::new();
                if self.tools.insert(id.clone()) {
                    events.push(AgentEvent::ToolUse {
                        id: id.clone(),
                        name,
                        input,
                    });
                }
                if complete {
                    events.push(AgentEvent::ToolResult {
                        id,
                        is_error: item["status"] == "failed"
                            || item["status"] == "declined"
                            || item["exitCode"].as_i64().is_some_and(|n| n != 0),
                    });
                }
                events
            }
            "thread/tokenUsage/updated" => vec![AgentEvent::Usage {
                context_tokens: p["tokenUsage"]["last"]["totalTokens"].as_u64(),
                context_window: p["tokenUsage"]["modelContextWindow"].as_u64(),
            }],
            "item/autoApprovalReview/started" | "item/autoApprovalReview/completed" => {
                vec![AgentEvent::ApprovalReview {
                    id: p["reviewId"].as_str().unwrap_or_default().into(),
                    status: p["review"]["status"].as_str().unwrap_or("reviewing").into(),
                    detail: p["review"]["rationale"].as_str().map(str::to_string),
                }]
            }
            "error" if !p["willRetry"].as_bool().unwrap_or(false) => vec![AgentEvent::Error {
                message: p["error"]["message"]
                    .as_str()
                    .unwrap_or("Codex reported an error.")
                    .into(),
            }],
            _ => Vec::new(),
        }
    }
}
fn tool(item: &Value) -> Option<(String, Value)> {
    match item["type"].as_str()? {
        "commandExecution" => Some(("Bash".into(), json!({"command":item["command"]}))),
        "fileChange" => Some((
            "Edit".into(),
            json!({"file_path":item["changes"].as_array().and_then(|a|a.first()).map(|v|v["path"].clone())}),
        )),
        "mcpToolCall" => Some((
            item["tool"].as_str().unwrap_or("Tool").into(),
            item["arguments"].clone(),
        )),
        "webSearch" => Some(("WebSearch".into(), json!({"query":item["query"]}))),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
