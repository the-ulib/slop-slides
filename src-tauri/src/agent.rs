//! Drives a coding agent CLI headless inside a deck folder, one process per turn, resuming
//! the deck's session between turns: Claude Code (`claude -p --output-format stream-json`)
//! OpenAI Codex (`codex app-server`), or GitHub Copilot (see [`crate::copilot`]). Stream events are normalized into [`AgentEvent`]s
//! and emitted to the frontend as `agent-event`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;

use crate::deck::{self, INTERNAL_DIR};
use crate::env;
use crate::error::{Error, Result};
use crate::mcp;
use crate::{codex, copilot};

pub(crate) const SYSTEM_PROMPT: &str = include_str!("../prompts/system.md");
/// File edits, URL fetches and this app's MCP tools; no shell.
const TOOLS: &str = "Read,Write,Edit,Glob,Grep,WebSearch,WebFetch";
const STDERR_LIMIT: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    #[default]
    Claude,
    Codex,
    Copilot,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Claude, Provider::Codex, Provider::Copilot];

    /// File under `.slopslide/` holding this provider's resumable session id.
    pub fn session_file(self) -> &'static str {
        match self {
            Provider::Claude => "session",
            Provider::Codex => "codex-session",
            Provider::Copilot => "copilot-session",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Provider::Claude => "Claude Code",
            Provider::Codex => "Codex",
            Provider::Copilot => "GitHub Copilot",
        }
    }

    fn resolve(self) -> Result<PathBuf> {
        match self {
            Provider::Claude => env::resolve_claude().ok_or_else(|| {
                Error::msg(
                    "Claude Code was not found. Install it from https://claude.com/claude-code, \
                     or set SLOPSLIDE_CLAUDE_PATH to the `claude` executable.",
                )
            }),
            Provider::Codex => env::resolve_codex().ok_or_else(|| {
                Error::msg(
                    "Codex was not found. Install it with `npm i -g @openai/codex` and sign in, \
                     or set SLOPSLIDE_CODEX_PATH to the `codex` executable.",
                )
            }),
            Provider::Copilot => env::resolve_copilot().ok_or_else(|| {
                Error::msg(
                    "GitHub Copilot was not found. Install it with `npm i -g @github/copilot` and \
                     sign in, or set SLOPSLIDE_COPILOT_PATH to the `copilot` executable.",
                )
            }),
        }
    }
}

#[derive(Default)]
pub struct AgentManager {
    running: Mutex<HashMap<String, watch::Sender<bool>>>,
    pub approvals: Arc<codex::Approvals>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AgentEvent {
    ApprovalRequested {
        approval: codex::Approval,
    },
    ApprovalResolved {
        id: String,
    },
    ApprovalReview {
        id: String,
        status: String,
        detail: Option<String>,
    },
    Started {
        session_id: Option<String>,
    },
    Thinking,
    TextStart,
    TextDelta {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        is_error: bool,
    },
    Result {
        is_error: bool,
        text: Option<String>,
        cost_usd: Option<f64>,
        duration_ms: Option<u64>,
    },
    Error {
        message: String,
    },
    /// How full the session's context window is. A `None` field is not known from this
    /// event; the frontend keeps what it knew before.
    Usage {
        context_tokens: Option<u64>,
        context_window: Option<u64>,
    },
    /// The agent started summarizing the conversation to free up context.
    Compacting,
    /// The conversation was summarized; its new size is reported by the next `Usage`.
    Compacted,
    Finished {
        interrupted: bool,
    },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a> {
    deck_id: &'a str,
    event: &'a AgentEvent,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendArgs {
    pub deck_id: String,
    pub prompt: String,
    #[serde(default)]
    pub provider: Provider,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Claude only: `200k` or `1m`.
    #[serde(default)]
    pub context_window: Option<String>,
    /// Summarize the conversation so far instead of sending `prompt`.
    #[serde(default)]
    pub compact: bool,
    #[serde(default)]
    pub permission_mode: codex::PermissionMode,
}

impl AgentManager {
    pub fn is_running(&self, deck_id: &str) -> bool {
        self.running.lock().unwrap().contains_key(deck_id)
    }

    pub fn send(&self, app: AppHandle, args: SendArgs) -> Result<()> {
        let dir = deck::deck_dir(&app, &args.deck_id)?;
        let bin = args.provider.resolve()?;
        let (cancel_tx, cancel_rx) = watch::channel(false);
        {
            let mut running = self.running.lock().unwrap();
            if running.contains_key(&args.deck_id) {
                return Err(Error::msg("The agent is still working on this deck."));
            }
            running.insert(args.deck_id.clone(), cancel_tx);
        }
        let approvals = self.approvals.clone();
        tauri::async_runtime::spawn(async move {
            let (emitter, deck_id) = (app.clone(), args.deck_id.clone());
            let turn = Turn {
                emit: Box::new(move |event| {
                    let _ = emitter.emit(
                        "agent-event",
                        Envelope {
                            deck_id: &deck_id,
                            event,
                        },
                    );
                }),
                dir,
                provider: args.provider,
                bin,
                model: args
                    .model
                    .filter(|m| !m.is_empty())
                    .map(|m| match args.provider {
                        Provider::Claude => claude_model(m, args.context_window.as_deref()),
                        _ => m,
                    }),
                effort: args.effort.filter(|e| !e.is_empty()),
                compact: args.compact,
                deck_id: args.deck_id.clone(),
                permission_mode: args.permission_mode,
                approvals,
            };
            // The whole deck is one file: keep a copy to fall back on before every turn.
            if let Err(e) = deck::snapshot(&turn.dir) {
                log::warn!("snapshot failed: {e}");
            }
            // Remember the locked slides, to put back any the agent changes.
            if let Err(e) = deck::guard_locked(&turn.dir) {
                log::warn!("could not record locked slides: {e}");
            }
            let interrupted = turn.run(&args.prompt, cancel_rx).await;
            match deck::release_guard(&turn.dir) {
                Ok(restored) if !restored.is_empty() => turn.emit(&AgentEvent::Error {
                    message: locked_restored_message(&restored),
                }),
                Ok(_) => {}
                Err(e) => turn.emit(&AgentEvent::Error {
                    message: format!("Could not check the locked slides after this turn: {e}"),
                }),
            }
            // Give new slides ids and restore the player runtime if the agent touched it.
            if let Err(e) = deck::normalize(&turn.dir) {
                turn.emit(&AgentEvent::Error {
                    message: format!("Could not tidy deck.html after this turn: {e}"),
                });
            }
            app.state::<AgentManager>()
                .running
                .lock()
                .unwrap()
                .remove(&args.deck_id);
            turn.emit(&AgentEvent::Finished { interrupted });
        });
        Ok(())
    }

    pub fn interrupt(&self, deck_id: &str) {
        self.approvals.cancel_deck(deck_id);
        if let Some(tx) = self.running.lock().unwrap().get(deck_id) {
            let _ = tx.send(true);
        }
    }
}

/// Tells the user which locked slides the agent changed and the app put back.
fn locked_restored_message(ids: &[String]) -> String {
    let list = ids
        .iter()
        .map(|id| format!("`{id}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("The agent changed locked slides; they were put back as they were: {list}.")
}

struct Turn {
    emit: Box<dyn Fn(&AgentEvent) + Send + Sync>,
    dir: PathBuf,
    provider: Provider,
    bin: PathBuf,
    model: Option<String>,
    effort: Option<String>,
    compact: bool,
    deck_id: String,
    permission_mode: codex::PermissionMode,
    approvals: Arc<codex::Approvals>,
}

/// The prompt Claude Code runs as its built-in compaction command.
const CLAUDE_COMPACT: &str = "/compact";

pub(crate) enum Outcome {
    Done,
    Interrupted,
    ResumeFailed,
}

impl Turn {
    fn emit(&self, event: &AgentEvent) {
        (self.emit)(event);
    }

    /// Returns whether the turn was interrupted.
    async fn run(&self, prompt: &str, mut cancel: watch::Receiver<bool>) -> bool {
        let session_file = self.provider.session_file();
        let session = deck::read_session(&self.dir, session_file);
        if self.compact {
            if let Some(message) = compact_refusal(self.provider, session.as_deref()) {
                self.emit(&AgentEvent::Error {
                    message: message.into(),
                });
                return false;
            }
        }
        let mut outcome = self.run_once(prompt, session.as_deref(), &mut cancel).await;
        if self.compact && matches!(outcome, Ok(Outcome::ResumeFailed)) {
            // Starting fresh would leave nothing to compact.
            let _ = deck::write_session(&self.dir, session_file, None);
            self.emit(&AgentEvent::Error {
                message: "The conversation could not be resumed, so there is nothing to compact."
                    .into(),
            });
            return false;
        }
        if matches!(outcome, Ok(Outcome::ResumeFailed)) {
            // The stored session is gone (other machine, cleared history): start fresh.
            let _ = deck::write_session(&self.dir, session_file, None);
            outcome = self.run_once(prompt, None, &mut cancel).await;
        }
        match outcome {
            Ok(Outcome::Interrupted) => true,
            Ok(_) => false,
            Err(e) => {
                self.emit(&AgentEvent::Error {
                    message: e.to_string(),
                });
                false
            }
        }
    }

    async fn run_once(
        &self,
        prompt: &str,
        session: Option<&str>,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<Outcome> {
        let model = self.model.as_deref();
        let effort = self.effort.as_deref();
        if self.provider == Provider::Copilot {
            let args = copilot::TurnArgs {
                bin: &self.bin,
                dir: &self.dir,
                lint_server: &std::env::current_exe()?,
                prompt,
                model,
                effort,
                session,
                compact: self.compact,
            };
            let on_session =
                |id: &str| deck::write_session(&self.dir, self.provider.session_file(), Some(id));
            return copilot::run_turn(args, cancel, &|event| self.emit(event), &on_session).await;
        }
        if self.provider == Provider::Codex {
            return codex::run_turn(
                codex::TurnArgs {
                    bin: &self.bin,
                    dir: &self.dir,
                    lint_server: &std::env::current_exe()?,
                    deck_id: &self.deck_id,
                    prompt,
                    model,
                    effort,
                    session,
                    mode: self.permission_mode,
                    approvals: self.approvals.clone(),
                },
                cancel,
                &|event| self.emit(event),
                &|id| deck::write_session(&self.dir, self.provider.session_file(), Some(id)),
            )
            .await;
        }
        let system_prompt = self.dir.join(INTERNAL_DIR).join("system-prompt.md");
        std::fs::write(&system_prompt, SYSTEM_PROMPT)?;
        let mcp_config = self.dir.join(INTERNAL_DIR).join("mcp.json");
        std::fs::write(
            &mcp_config,
            lint_server_config(&std::env::current_exe()?, &self.dir),
        )?;
        let args = build_claude_args(&system_prompt, &mcp_config, model, effort, session);
        let started = Instant::now();

        let mut cmd = Command::new(&self.bin);
        cmd.args(args)
            .current_dir(&self.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        let mut child = cmd.spawn().map_err(|e| {
            Error::msg(format!(
                "Could not start {} ({}): {e}",
                self.provider.name(),
                self.bin.display()
            ))
        })?;
        self.emit(&AgentEvent::Started {
            session_id: session.map(str::to_string),
        });

        let mut stdin = child.stdin.take().expect("piped stdin");
        let prompt = if self.compact { CLAUDE_COMPACT } else { prompt };
        stdin.write_all(prompt.as_bytes()).await?;
        drop(stdin);

        let mut stderr = child.stderr.take().expect("piped stderr");
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = (&mut stderr)
                .take(STDERR_LIMIT as u64)
                .read_to_end(&mut buf)
                .await;
            // Keep draining so the child never blocks on a full pipe.
            let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
            String::from_utf8_lossy(&buf).into_owned()
        });

        let mut lines = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
        let mut saw_init = false;
        let mut result_text = None;
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let Some(line) = line? else { break };
                    let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
                    if let Some(id) = session_id_of_init(&value) {
                        saw_init = true;
                        deck::write_session(&self.dir, self.provider.session_file(), Some(&id))?;
                        self.emit(&AgentEvent::Started { session_id: Some(id) });
                    }
                    if session.is_some() && !saw_init && is_missing_session(&value) {
                        let _ = child.kill().await;
                        return Ok(Outcome::ResumeFailed);
                    }
                    for mut event in parse_line(&value) {
                        if let AgentEvent::Result { text, duration_ms, .. } = &mut event {
                            result_text = text.clone();
                            duration_ms.get_or_insert(started.elapsed().as_millis() as u64);
                        }
                        self.emit(&event);
                    }
                }
                _ = cancel.changed() => {
                    let _ = child.kill().await;
                    return Ok(Outcome::Interrupted);
                }
            }
        }

        let status = child.wait().await?;
        let stderr = stderr_task.await.unwrap_or_default();
        if session.is_some() && !saw_init && stderr.contains(MISSING_SESSION) {
            return Ok(Outcome::ResumeFailed);
        }
        if !status.success() && result_text.is_none() {
            let detail = stderr.trim();
            let detail = if detail.is_empty() {
                format!("exit status {status}")
            } else {
                detail.to_string()
            };
            return Err(Error::msg(format!(
                "{} stopped unexpectedly: {detail}",
                self.provider.name()
            )));
        }
        Ok(Outcome::Done)
    }
}

/// Why a compaction cannot run, if it cannot: there must be a conversation, and Codex's
/// headless mode has no way to compact one.
fn compact_refusal(provider: Provider, session: Option<&str>) -> Option<&'static str> {
    if provider == Provider::Codex {
        Some("Codex cannot compact the conversation from SlopSlide. Start a new chat instead.")
    } else if session.is_none() {
        Some("There is no conversation to compact yet.")
    } else {
        None
    }
}

/// MCP config running this app binary as the agent's lint tool server (`mcp.rs`).
fn lint_server_config(exe: &Path, dir: &Path) -> String {
    serde_json::json!({
        "mcpServers": {
            mcp::SERVER: {
                "type": "stdio",
                "command": exe,
                "args": [mcp::FLAG, dir],
            }
        }
    })
    .to_string()
}

/// Claude Code selects the 1M-token context window with a `[1m]` model suffix.
fn claude_model(model: String, context_window: Option<&str>) -> String {
    match context_window {
        Some("1m") if !model.ends_with("[1m]") => format!("{model}[1m]"),
        _ => model,
    }
}

fn build_claude_args(
    system_prompt: &Path,
    mcp_config: &Path,
    model: Option<&str>,
    effort: Option<&str>,
    session: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-mode",
        "acceptEdits",
        "--strict-mcp-config",
        "--tools",
        TOOLS,
    ]
    .into_iter()
    .map(String::from)
    .collect();
    args.extend([
        "--allowedTools".into(),
        format!("{TOOLS},{},{}", mcp::QUALIFIED_TOOL, mcp::NARRATION_TOOLS),
        "--mcp-config".into(),
        mcp_config.to_string_lossy().into_owned(),
        "--append-system-prompt-file".into(),
        system_prompt.to_string_lossy().into_owned(),
    ]);
    if let Some(model) = model {
        args.extend(["--model".into(), model.into()]);
    }
    if let Some(effort) = effort {
        args.extend(["--effort".into(), effort.into()]);
    }
    if let Some(session) = session {
        args.extend(["--resume".into(), session.into()]);
    }
    args
}

const MISSING_SESSION: &str = "No conversation found";

fn is_missing_session(value: &Value) -> bool {
    value["type"] == "result"
        && value["errors"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|e| e.as_str().is_some_and(|e| e.contains(MISSING_SESSION)))
}

fn session_id_of_init(value: &Value) -> Option<String> {
    (value["type"] == "system" && value["subtype"] == "init")
        .then(|| value["session_id"].as_str().map(str::to_string))
        .flatten()
}

/// Maps one Claude Code stream-json line to UI events. Unknown lines map to nothing.
fn parse_line(value: &Value) -> Vec<AgentEvent> {
    // Events from nested agents are not shown.
    if !value["parent_tool_use_id"].is_null() {
        return Vec::new();
    }
    match value["type"].as_str() {
        Some("stream_event") => {
            let event = &value["event"];
            match (
                event["type"].as_str(),
                event["content_block"]["type"].as_str(),
                event["delta"]["type"].as_str(),
            ) {
                (Some("content_block_start"), Some("text"), _) => vec![AgentEvent::TextStart],
                (Some("content_block_start"), Some("thinking"), _) => vec![AgentEvent::Thinking],
                (Some("content_block_delta"), _, Some("text_delta")) => event["delta"]["text"]
                    .as_str()
                    .map(|text| {
                        vec![AgentEvent::TextDelta {
                            text: text.to_string(),
                        }]
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            }
        }
        Some("assistant") => content_blocks(value)
            .filter(|block| block["type"] == "tool_use")
            .map(|block| AgentEvent::ToolUse {
                id: block["id"].as_str().unwrap_or_default().to_string(),
                name: block["name"].as_str().unwrap_or_default().to_string(),
                input: block["input"].clone(),
            })
            .chain(
                claude_context_tokens(&value["message"]["usage"]).map(|tokens| AgentEvent::Usage {
                    context_tokens: Some(tokens),
                    context_window: None,
                }),
            )
            .collect(),
        Some("system") => match value["subtype"].as_str() {
            Some("status") if value["status"] == "compacting" => vec![AgentEvent::Compacting],
            Some("compact_boundary") => vec![AgentEvent::Compacted],
            _ => Vec::new(),
        },
        Some("user") => content_blocks(value)
            .filter(|block| block["type"] == "tool_result")
            .map(|block| AgentEvent::ToolResult {
                id: block["tool_use_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                is_error: block["is_error"].as_bool().unwrap_or(false),
            })
            .collect(),
        Some("result") => {
            let mut events = vec![AgentEvent::Result {
                is_error: value["is_error"].as_bool().unwrap_or(false)
                    || value["subtype"] != "success",
                text: value["result"].as_str().map(str::to_string),
                cost_usd: value["total_cost_usd"].as_f64(),
                duration_ms: value["duration_ms"].as_u64(),
            }];
            // Only the result says how large the window is. Background calls to smaller
            // models are listed too, so take the largest.
            let window = value["modelUsage"]
                .as_object()
                .into_iter()
                .flat_map(|models| models.values())
                .filter_map(|model| model["contextWindow"].as_u64())
                .max();
            if window.is_some() {
                events.push(AgentEvent::Usage {
                    context_tokens: None,
                    context_window: window,
                });
            }
            events
        }
        _ => Vec::new(),
    }
}

/// Tokens in the context after one model call: everything it read plus what it wrote,
/// which the next call reads back.
fn claude_context_tokens(usage: &Value) -> Option<u64> {
    let input = usage["input_tokens"].as_u64()?;
    Some(
        input
            + usage["cache_creation_input_tokens"].as_u64().unwrap_or(0)
            + usage["cache_read_input_tokens"].as_u64().unwrap_or(0)
            + usage["output_tokens"].as_u64().unwrap_or(0),
    )
}

fn content_blocks(value: &Value) -> impl Iterator<Item = &Value> {
    value["message"]["content"].as_array().into_iter().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_model_adds_1m_suffix() {
        assert_eq!(
            claude_model("claude-opus-5-5".into(), Some("1m")),
            "claude-opus-5-5[1m]"
        );
        assert_eq!(
            claude_model("claude-opus-5-5[1m]".into(), Some("1m")),
            "claude-opus-5-5[1m]"
        );
        assert_eq!(
            claude_model("claude-sonnet-5".into(), Some("200k")),
            "claude-sonnet-5"
        );
        assert_eq!(
            claude_model("claude-haiku-4-5".into(), None),
            "claude-haiku-4-5"
        );
    }

    #[test]
    fn parses_text_stream() {
        let start = json!({"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}});
        let delta = json!({"type":"stream_event","parent_tool_use_id":null,"event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hi"}}});
        assert_eq!(parse_line(&start), vec![AgentEvent::TextStart]);
        assert_eq!(
            parse_line(&delta),
            vec![AgentEvent::TextDelta { text: "Hi".into() }]
        );
    }

    #[test]
    fn parses_tools_and_results() {
        let tool = json!({"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t1","name":"Write","input":{"file_path":"/d/slides/01.html"}}]}});
        let result = json!({"type":"user","parent_tool_use_id":null,"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}});
        let done = json!({"type":"result","subtype":"success","is_error":false,"result":"done","total_cost_usd":0.02,"duration_ms":1200});
        assert!(
            matches!(&parse_line(&tool)[0], AgentEvent::ToolUse { name, .. } if name == "Write")
        );
        assert_eq!(
            parse_line(&result),
            vec![AgentEvent::ToolResult {
                id: "t1".into(),
                is_error: false
            }]
        );
        assert!(matches!(
            &parse_line(&done)[0],
            AgentEvent::Result {
                is_error: false,
                ..
            }
        ));
    }

    #[test]
    fn parses_context_usage() {
        let assistant = json!({"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":9,"cache_creation_input_tokens":7662,"cache_read_input_tokens":12313,"output_tokens":4}}});
        assert_eq!(
            parse_line(&assistant),
            vec![AgentEvent::Usage {
                context_tokens: Some(19988),
                context_window: None
            }]
        );
        let tool = json!({"type":"assistant","parent_tool_use_id":null,"message":{"content":[{"type":"tool_use","id":"t1","name":"Read","input":{}}],"usage":{"input_tokens":100}}});
        assert!(matches!(
            &parse_line(&tool)[..],
            [
                AgentEvent::ToolUse { .. },
                AgentEvent::Usage {
                    context_tokens: Some(100),
                    ..
                }
            ]
        ));
        let nested = json!({"type":"assistant","parent_tool_use_id":"t1","message":{"content":[],"usage":{"input_tokens":5}}});
        assert!(
            parse_line(&nested).is_empty(),
            "sub-agents have their own context"
        );
        let no_usage =
            json!({"type":"assistant","parent_tool_use_id":null,"message":{"content":[]}});
        assert!(parse_line(&no_usage).is_empty());

        let result = json!({"type":"result","subtype":"success","is_error":false,"result":"ok","modelUsage":{
            "claude-haiku-4-5-20251001":{"contextWindow":200000},
            "claude-opus-5-5":{"contextWindow":1000000}
        }});
        assert_eq!(
            parse_line(&result)[1..],
            [AgentEvent::Usage {
                context_tokens: None,
                context_window: Some(1_000_000)
            }]
        );
        let bare = json!({"type":"result","subtype":"success","is_error":false,"result":"ok"});
        assert_eq!(parse_line(&bare).len(), 1, "no window, no usage event");
    }

    #[test]
    fn parses_compaction() {
        let status = |status: Value| json!({"type":"system","subtype":"status","status":status});
        assert_eq!(
            parse_line(&status(json!("compacting"))),
            vec![AgentEvent::Compacting]
        );
        assert!(parse_line(&status(Value::Null)).is_empty());
        let boundary = json!({"type":"system","subtype":"compact_boundary","compact_metadata":{"trigger":"auto","pre_tokens":20018,"post_tokens":1512}});
        assert_eq!(parse_line(&boundary), vec![AgentEvent::Compacted]);
        let init = json!({"type":"system","subtype":"init","session_id":"s"});
        assert!(parse_line(&init).is_empty());
    }

    #[test]
    fn refuses_compaction_it_cannot_run() {
        assert_eq!(compact_refusal(Provider::Claude, Some("s")), None);
        assert_eq!(compact_refusal(Provider::Copilot, Some("s")), None);
        assert!(compact_refusal(Provider::Claude, None).is_some());
        assert!(compact_refusal(Provider::Copilot, None).is_some());
        assert!(compact_refusal(Provider::Codex, Some("s"))
            .unwrap()
            .starts_with("Codex cannot compact"));
    }

    #[test]
    fn names_the_locked_slides_it_put_back() {
        assert_eq!(
            locked_restored_message(&["intro".into(), "plan".into()]),
            "The agent changed locked slides; they were put back as they were: `intro`, `plan`."
        );
    }

    #[test]
    fn send_args_compact_defaults_to_off() {
        let args: SendArgs =
            serde_json::from_value(json!({"deckId":"d","prompt":"hi","provider":"claude"}))
                .unwrap();
        assert!(!args.compact);
        let args: SendArgs = serde_json::from_value(
            json!({"deckId":"d","prompt":"/compact","provider":"copilot","compact":true}),
        )
        .unwrap();
        assert!(args.compact);
    }

    #[test]
    fn serializes_usage_events_in_camel_case() {
        let usage = AgentEvent::Usage {
            context_tokens: Some(12),
            context_window: None,
        };
        assert_eq!(
            serde_json::to_value(&usage).unwrap(),
            json!({"type":"usage","contextTokens":12,"contextWindow":null})
        );
        assert_eq!(
            serde_json::to_value(&AgentEvent::Compacted).unwrap(),
            json!({"type":"compacted"})
        );
    }

    #[test]
    fn detects_missing_session() {
        let missing = json!({"type":"result","subtype":"error_during_execution","is_error":true,"errors":["No conversation found with session ID: x"]});
        assert!(is_missing_session(&missing));
        assert!(!is_missing_session(
            &json!({"type":"result","subtype":"success"})
        ));
    }

    #[test]
    fn reads_session_from_init() {
        let init = json!({"type":"system","subtype":"init","session_id":"abc"});
        assert_eq!(session_id_of_init(&init).as_deref(), Some("abc"));
        assert_eq!(
            session_id_of_init(&json!({"type":"system","subtype":"status"})),
            None
        );
    }

    #[test]
    fn parses_thinking_and_ignores_other_stream_events() {
        let thinking = json!({"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"thinking"}}});
        assert_eq!(parse_line(&thinking), vec![AgentEvent::Thinking]);
        for ignored in [
            json!({"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"input_json_delta","partial_json":"{"}}}),
            json!({"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta"}}}),
            json!({"type":"stream_event","event":{"type":"message_stop"}}),
            json!({"type":"system","subtype":"init","session_id":"abc"}),
            json!({"type":"something_new"}),
            json!({}),
            json!("just a string"),
        ] {
            assert_eq!(parse_line(&ignored), vec![], "{ignored}");
        }
    }

    #[test]
    fn hides_events_from_nested_agents() {
        let nested = json!({"type":"assistant","parent_tool_use_id":"t0","message":{"content":[{"type":"tool_use","id":"t9","name":"Read","input":{}}]}});
        assert_eq!(parse_line(&nested), vec![]);
        let nested_text = json!({"type":"stream_event","parent_tool_use_id":"t0","event":{"type":"content_block_start","content_block":{"type":"text"}}});
        assert_eq!(parse_line(&nested_text), vec![]);
    }

    #[test]
    fn emits_every_tool_use_in_a_message_and_skips_text_blocks() {
        let message = json!({"type":"assistant","message":{"content":[
            {"type":"text","text":"Let me look."},
            {"type":"tool_use","id":"a","name":"Read","input":{"file_path":"deck.html"}},
            {"type":"tool_use","id":"b","name":"Edit","input":{"old_string":"x","new_string":"y"}}
        ]}});
        assert_eq!(
            parse_line(&message),
            vec![
                AgentEvent::ToolUse {
                    id: "a".into(),
                    name: "Read".into(),
                    input: json!({"file_path":"deck.html"})
                },
                AgentEvent::ToolUse {
                    id: "b".into(),
                    name: "Edit".into(),
                    input: json!({"old_string":"x","new_string":"y"})
                },
            ]
        );
        let no_content = json!({"type":"assistant","message":{}});
        assert_eq!(parse_line(&no_content), vec![]);
    }

    #[test]
    fn reports_failed_tool_results() {
        let result = json!({"type":"user","message":{"content":[
            {"type":"tool_result","tool_use_id":"a","is_error":true,"content":"denied"},
            {"type":"text","text":"ignored"}
        ]}});
        assert_eq!(
            parse_line(&result),
            vec![AgentEvent::ToolResult {
                id: "a".into(),
                is_error: true
            }]
        );
    }

    #[test]
    fn non_success_results_are_errors() {
        let max_turns =
            json!({"type":"result","subtype":"error_max_turns","is_error":false,"duration_ms":10});
        assert_eq!(
            parse_line(&max_turns),
            vec![AgentEvent::Result {
                is_error: true,
                text: None,
                cost_usd: None,
                duration_ms: Some(10)
            }]
        );
        let flagged = json!({"type":"result","subtype":"success","is_error":true,"result":"API Error","total_cost_usd":0.5});
        assert_eq!(
            parse_line(&flagged),
            vec![AgentEvent::Result {
                is_error: true,
                text: Some("API Error".into()),
                cost_usd: Some(0.5),
                duration_ms: None
            }]
        );
    }

    #[test]
    fn missing_session_needs_a_result_with_that_error() {
        assert!(!is_missing_session(
            &json!({"type":"assistant","errors":["No conversation found"]})
        ));
        assert!(!is_missing_session(
            &json!({"type":"result","errors":["Rate limited"]})
        ));
        assert!(!is_missing_session(&json!({"type":"result","errors":[42]})));
        assert!(!is_missing_session(&json!({"type":"result"})));
    }

    #[test]
    fn init_without_session_id_is_ignored() {
        assert_eq!(
            session_id_of_init(&json!({"type":"system","subtype":"init"})),
            None
        );
        assert_eq!(
            session_id_of_init(&json!({"type":"result","subtype":"init","session_id":"x"})),
            None
        );
    }

    #[test]
    fn builds_cli_arguments() {
        let prompt = Path::new("/deck/.slopslide/system-prompt.md");
        let mcp = Path::new("/deck/.slopslide/mcp.json");
        let base = build_claude_args(prompt, mcp, None, None, None);
        assert_eq!(base[..3], ["-p", "--output-format", "stream-json"]);
        let after = |args: &[String], flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .map(|i| args[i + 1].clone())
        };
        assert_eq!(
            after(&base, "--permission-mode").as_deref(),
            Some("acceptEdits")
        );
        assert_eq!(after(&base, "--tools").as_deref(), Some(TOOLS));
        assert_eq!(
            after(&base, "--allowedTools").as_deref(),
            Some(format!("{TOOLS},{},{}", mcp::QUALIFIED_TOOL, mcp::NARRATION_TOOLS).as_str())
        );
        assert_eq!(
            after(&base, "--mcp-config").as_deref(),
            Some("/deck/.slopslide/mcp.json")
        );
        assert_eq!(
            after(&base, "--append-system-prompt-file").as_deref(),
            Some("/deck/.slopslide/system-prompt.md")
        );
        assert!(base.contains(&"--strict-mcp-config".to_string()));
        assert!(!base.contains(&"--model".to_string()));
        assert!(!base.contains(&"--resume".to_string()));
        assert!(!base.contains(&"--effort".to_string()));
        assert!(!TOOLS.contains("Bash"), "the agent must not get a shell");

        let full = build_claude_args(prompt, mcp, Some("opus"), Some("high"), Some("s-1"));
        assert_eq!(after(&full, "--model").as_deref(), Some("opus"));
        assert_eq!(after(&full, "--effort").as_deref(), Some("high"));
        assert_eq!(after(&full, "--resume").as_deref(), Some("s-1"));
    }

    #[test]
    fn lint_server_config_runs_this_binary_for_the_deck() {
        let config: Value = serde_json::from_str(&lint_server_config(
            Path::new("/Apps/SlopSlide"),
            Path::new("/decks/my deck"),
        ))
        .unwrap();
        let server = &config["mcpServers"]["slopslide"];
        assert_eq!(server["type"], "stdio");
        assert_eq!(server["command"], "/Apps/SlopSlide");
        assert_eq!(server["args"], json!(["--lint-mcp", "/decks/my deck"]));
    }

    #[test]
    fn events_serialize_to_the_frontend_shape() {
        let cases = [
            (
                AgentEvent::Started { session_id: None },
                json!({"type":"started","sessionId":null}),
            ),
            (AgentEvent::Thinking, json!({"type":"thinking"})),
            (AgentEvent::TextStart, json!({"type":"textStart"})),
            (
                AgentEvent::TextDelta { text: "hi".into() },
                json!({"type":"textDelta","text":"hi"}),
            ),
            (
                AgentEvent::ToolUse {
                    id: "t".into(),
                    name: "Write".into(),
                    input: json!({"a":1}),
                },
                json!({"type":"toolUse","id":"t","name":"Write","input":{"a":1}}),
            ),
            (
                AgentEvent::ToolResult {
                    id: "t".into(),
                    is_error: true,
                },
                json!({"type":"toolResult","id":"t","isError":true}),
            ),
            (
                AgentEvent::Result {
                    is_error: false,
                    text: Some("ok".into()),
                    cost_usd: Some(0.1),
                    duration_ms: Some(9),
                },
                json!({"type":"result","isError":false,"text":"ok","costUsd":0.1,"durationMs":9}),
            ),
            (
                AgentEvent::Error {
                    message: "bad".into(),
                },
                json!({"type":"error","message":"bad"}),
            ),
            (
                AgentEvent::Finished { interrupted: true },
                json!({"type":"finished","interrupted":true}),
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(serde_json::to_value(&event).unwrap(), expected);
        }
        let envelope = Envelope {
            deck_id: "talk",
            event: &AgentEvent::Thinking,
        };
        assert_eq!(
            serde_json::to_value(envelope).unwrap(),
            json!({"deckId":"talk","event":{"type":"thinking"}})
        );
    }

    #[test]
    fn send_args_deserialize_from_the_frontend() {
        let args: SendArgs =
            serde_json::from_value(json!({"deckId":"talk","prompt":"Hi","model":null})).unwrap();
        assert_eq!(
            (args.deck_id.as_str(), args.prompt.as_str()),
            ("talk", "Hi")
        );
        assert_eq!(args.model, None);
        assert_eq!(
            args.provider,
            Provider::Claude,
            "older frontends send no provider"
        );
        let args: SendArgs = serde_json::from_value(
            json!({"deckId":"t","prompt":"","provider":"codex","model":"gpt-6-astra","effort":"high"}),
        )
        .unwrap();
        assert_eq!(args.provider, Provider::Codex);
        assert_eq!(args.model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(args.effort.as_deref(), Some("high"));
    }

    #[test]
    fn manager_tracks_nothing_by_default() {
        let manager = AgentManager::default();
        assert!(!manager.is_running("talk"));
        manager.interrupt("talk"); // no-op, must not panic
    }

    /// Runs whole turns against a shell script standing in for the `claude` CLI.
    #[cfg(unix)]
    mod process {
        use super::*;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::sync::Arc;
        use std::time::Duration;

        struct Fixture {
            dir: PathBuf,
            events: Arc<Mutex<Vec<AgentEvent>>>,
        }

        impl Fixture {
            /// A deck folder plus a fake `claude` that logs its arguments and stdin, then
            /// runs `body` (with `$RESUME` set when `--resume` was passed).
            fn new(body: &str) -> Self {
                let dir =
                    std::env::temp_dir().join(format!("slopslide-agent-{}", uuid::Uuid::new_v4()));
                fs::create_dir_all(dir.join(INTERNAL_DIR)).unwrap();
                let script = format!(
                    "#!/bin/sh\n\
                     LOG=\"$PWD/{INTERNAL_DIR}\"\n\
                     RESUME=\n\
                     prev=\n\
                     for a in \"$@\"; do [ \"$prev\" = --resume ] && RESUME=\"$a\"; prev=\"$a\"; done\n\
                     printf '%s\\n' \"$@\" > \"$LOG/args.$$\"\n\
                     printf '%s\\n' \"$$\" >> \"$LOG/runs\"\n\
                     cat > \"$LOG/stdin.log\"\n\
                     {body}\n"
                );
                let claude = dir.join("claude");
                fs::write(&claude, script).unwrap();
                fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
                Fixture {
                    dir,
                    events: Arc::default(),
                }
            }

            fn turn(&self) -> Turn {
                self.turn_with(self.dir.join("claude"), None)
            }

            fn turn_with(&self, claude: PathBuf, model: Option<&str>) -> Turn {
                let events = self.events.clone();
                Turn {
                    emit: Box::new(move |e| events.lock().unwrap().push(e.clone())),
                    dir: self.dir.clone(),
                    provider: Provider::Claude,
                    bin: claude,
                    model: model.map(str::to_string),
                    effort: None,
                    compact: false,
                    deck_id: "test".into(),
                    permission_mode: codex::PermissionMode::Ask,
                    approvals: Arc::default(),
                }
            }

            async fn run_compact(&self) -> bool {
                let (_tx, rx) = watch::channel(false);
                let mut turn = self.turn();
                turn.compact = true;
                tokio::time::timeout(
                    Duration::from_secs(20),
                    turn.run("whatever the composer held", rx),
                )
                .await
                .expect("turn timed out")
            }

            async fn run(&self, prompt: &str, model: Option<&str>) -> bool {
                let (_tx, rx) = watch::channel(false);
                let turn = self.turn_with(self.dir.join("claude"), model);
                tokio::time::timeout(Duration::from_secs(20), turn.run(prompt, rx))
                    .await
                    .expect("turn timed out")
            }

            fn events(&self) -> Vec<AgentEvent> {
                self.events.lock().unwrap().clone()
            }

            fn errors(&self) -> Vec<String> {
                self.events()
                    .into_iter()
                    .filter_map(|e| match e {
                        AgentEvent::Error { message } => Some(message),
                        _ => None,
                    })
                    .collect()
            }

            /// Arguments of each invocation, in order.
            fn invocations(&self) -> Vec<Vec<String>> {
                let log = self.dir.join(INTERNAL_DIR);
                let runs = fs::read_to_string(log.join("runs")).unwrap_or_default();
                runs.lines()
                    .map(|pid| {
                        fs::read_to_string(log.join(format!("args.{pid}")))
                            .unwrap()
                            .lines()
                            .map(String::from)
                            .collect()
                    })
                    .collect()
            }

            fn session(&self) -> Option<String> {
                deck::read_session(&self.dir, Provider::Claude.session_file())
            }
        }

        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.dir);
            }
        }

        const INIT: &str =
            r#"echo '{"type":"system","subtype":"init","session_id":"new-session"}'"#;
        const OK: &str = r#"echo '{"type":"result","subtype":"success","is_error":false,"result":"Done.","total_cost_usd":0.01,"duration_ms":5}'"#;

        fn has_flag(args: &[String], flag: &str, value: &str) -> bool {
            args.windows(2).any(|w| w[0] == flag && w[1] == value)
        }

        #[tokio::test]
        async fn streams_a_turn_and_stores_the_session() {
            let fx = Fixture::new(&format!(
                r#"{INIT}
echo 'not json, ignored'
echo '{{"type":"stream_event","event":{{"type":"content_block_start","content_block":{{"type":"text"}}}}}}'
echo '{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"Hello"}}}}}}'
echo '{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"t1","name":"Edit","input":{{}}}}]}}}}'
echo '{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1"}}]}}}}'
{OK}"#
            ));
            let interrupted = fx.run("Make it pop", Some("sonnet")).await;
            assert!(!interrupted);
            assert_eq!(
                fx.events(),
                vec![
                    AgentEvent::Started { session_id: None },
                    AgentEvent::Started {
                        session_id: Some("new-session".into())
                    },
                    AgentEvent::TextStart,
                    AgentEvent::TextDelta {
                        text: "Hello".into()
                    },
                    AgentEvent::ToolUse {
                        id: "t1".into(),
                        name: "Edit".into(),
                        input: json!({})
                    },
                    AgentEvent::ToolResult {
                        id: "t1".into(),
                        is_error: false
                    },
                    AgentEvent::Result {
                        is_error: false,
                        text: Some("Done.".into()),
                        cost_usd: Some(0.01),
                        duration_ms: Some(5)
                    },
                ]
            );
            assert_eq!(fx.session().as_deref(), Some("new-session"));
            let log = fx.dir.join(INTERNAL_DIR);
            assert_eq!(
                fs::read_to_string(log.join("stdin.log")).unwrap(),
                "Make it pop"
            );
            let prompt_file = log.join("system-prompt.md");
            assert_eq!(fs::read_to_string(&prompt_file).unwrap(), SYSTEM_PROMPT);
            let runs = fx.invocations();
            assert_eq!(runs.len(), 1);
            assert!(has_flag(&runs[0], "--model", "sonnet"));
            assert!(has_flag(
                &runs[0],
                "--append-system-prompt-file",
                &prompt_file.to_string_lossy()
            ));
            assert!(!runs[0].contains(&"--resume".to_string()));
        }

        #[tokio::test]
        async fn resumes_the_stored_session() {
            let fx = Fixture::new(&format!(
                r#"echo "{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"$RESUME\"}}"
{OK}"#
            ));
            deck::write_session(&fx.dir, "session", Some("old-session")).unwrap();
            assert!(!fx.run("Again", None).await);
            let runs = fx.invocations();
            assert_eq!(runs.len(), 1);
            assert!(has_flag(&runs[0], "--resume", "old-session"));
            assert!(!runs[0].contains(&"--model".to_string()));
            assert_eq!(
                fx.events()[0],
                AgentEvent::Started {
                    session_id: Some("old-session".into())
                }
            );
            assert_eq!(fx.session().as_deref(), Some("old-session"));
            assert!(fx.errors().is_empty());
        }

        #[tokio::test]
        async fn starts_fresh_when_the_session_is_gone() {
            let fx = Fixture::new(&format!(
                r#"if [ -n "$RESUME" ]; then
  echo '{{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["No conversation found with session ID: gone"]}}'
  exit 1
fi
{INIT}
{OK}"#
            ));
            deck::write_session(&fx.dir, "session", Some("gone")).unwrap();
            assert!(!fx.run("Hi", None).await);
            let runs = fx.invocations();
            assert_eq!(runs.len(), 2, "retried once");
            assert!(has_flag(&runs[0], "--resume", "gone"));
            assert!(!runs[1].contains(&"--resume".to_string()));
            assert_eq!(fx.session().as_deref(), Some("new-session"));
            assert!(fx.errors().is_empty(), "{:?}", fx.errors());
            let results: Vec<_> = fx
                .events()
                .into_iter()
                .filter(|e| matches!(e, AgentEvent::Result { .. }))
                .collect();
            assert_eq!(results.len(), 1, "the failed resume's result is not shown");
            assert!(matches!(
                results[0],
                AgentEvent::Result {
                    is_error: false,
                    ..
                }
            ));
        }

        #[tokio::test]
        async fn starts_fresh_when_stderr_reports_a_missing_session() {
            let fx = Fixture::new(&format!(
                r#"if [ -n "$RESUME" ]; then
  echo "Error: No conversation found with session ID: $RESUME" >&2
  exit 1
fi
{INIT}
{OK}"#
            ));
            deck::write_session(&fx.dir, "session", Some("gone")).unwrap();
            assert!(!fx.run("Hi", None).await);
            assert_eq!(fx.invocations().len(), 2);
            assert_eq!(fx.session().as_deref(), Some("new-session"));
            assert!(fx.errors().is_empty(), "{:?}", fx.errors());
        }

        #[tokio::test]
        async fn compacts_the_stored_session() {
            let fx = Fixture::new(&format!(
                r#"echo "{{\"type\":\"system\",\"subtype\":\"status\",\"status\":\"compacting\"}}"
echo "{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"$RESUME\"}}"
echo '{{"type":"system","subtype":"compact_boundary","compact_metadata":{{"trigger":"manual"}}}}'
{OK}"#
            ));
            deck::write_session(&fx.dir, "session", Some("old-session")).unwrap();
            assert!(!fx.run_compact().await);
            let runs = fx.invocations();
            assert_eq!(runs.len(), 1);
            assert!(has_flag(&runs[0], "--resume", "old-session"));
            assert_eq!(
                fs::read_to_string(fx.dir.join(INTERNAL_DIR).join("stdin.log")).unwrap(),
                "/compact"
            );
            let events = fx.events();
            assert!(events.contains(&AgentEvent::Compacting));
            assert!(events.contains(&AgentEvent::Compacted));
            assert!(fx.errors().is_empty(), "{:?}", fx.errors());
            assert_eq!(fx.session().as_deref(), Some("old-session"));
        }

        #[tokio::test]
        async fn compacting_needs_a_conversation() {
            let fx = Fixture::new(&format!("{INIT}\n{OK}"));
            assert!(!fx.run_compact().await);
            assert!(fx.invocations().is_empty(), "the CLI is not started");
            assert_eq!(fx.errors(), ["There is no conversation to compact yet."]);
        }

        #[tokio::test]
        async fn compacting_a_lost_session_does_not_start_fresh() {
            let fx = Fixture::new(&format!(
                r#"if [ -n "$RESUME" ]; then
  echo '{{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["No conversation found with session ID: gone"]}}'
  exit 1
fi
{INIT}
{OK}"#
            ));
            deck::write_session(&fx.dir, "session", Some("gone")).unwrap();
            assert!(!fx.run_compact().await);
            assert_eq!(fx.invocations().len(), 1, "not retried without the session");
            assert_eq!(fx.session(), None, "the stale session is forgotten");
            assert_eq!(fx.errors().len(), 1);
        }

        #[tokio::test]
        async fn a_fresh_turn_is_not_retried() {
            let fx = Fixture::new(
                r#"echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["No conversation found"]}'"#,
            );
            assert!(!fx.run("Hi", None).await);
            assert_eq!(fx.invocations().len(), 1);
        }

        #[tokio::test]
        async fn reports_a_crash_with_its_stderr() {
            let fx = Fixture::new("echo '  Invalid API key  ' >&2\nexit 3");
            assert!(!fx.run("Hi", None).await);
            assert_eq!(
                fx.errors(),
                ["Claude Code stopped unexpectedly: Invalid API key"]
            );
        }

        #[tokio::test]
        async fn reports_the_exit_status_when_stderr_is_empty() {
            let fx = Fixture::new("exit 7");
            assert!(!fx.run("Hi", None).await);
            let errors = fx.errors();
            assert_eq!(errors.len(), 1);
            assert!(
                errors[0].starts_with("Claude Code stopped unexpectedly: exit status"),
                "{}",
                errors[0]
            );
            assert!(errors[0].contains('7'), "{}", errors[0]);
        }

        #[tokio::test]
        async fn a_failing_exit_after_a_result_is_not_an_extra_error() {
            let fx = Fixture::new(&format!("{OK}\necho noise >&2\nexit 1"));
            assert!(!fx.run("Hi", None).await);
            assert!(fx.errors().is_empty(), "{:?}", fx.errors());
        }

        #[tokio::test]
        async fn large_stderr_does_not_block_the_child() {
            // Far more than a pipe buffer; the reader must keep draining past STDERR_LIMIT.
            let fx = Fixture::new(&format!(
                "i=0; while [ $i -lt 2000 ]; do echo 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx' >&2; i=$((i+1)); done\n{OK}"
            ));
            assert!(!fx.run("Hi", None).await);
            assert!(fx.events().iter().any(|e| matches!(
                e,
                AgentEvent::Result {
                    is_error: false,
                    ..
                }
            )));
        }

        #[tokio::test]
        async fn interrupting_kills_the_process() {
            let fx = Fixture::new(&format!("{INIT}\nexec sleep 30"));
            let (tx, rx) = watch::channel(false);
            let turn = fx.turn();
            let run = tokio::spawn(async move { turn.run("Hi", rx).await });
            // Wait until the turn is under way.
            for _ in 0..200 {
                if fx.session().is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            assert_eq!(fx.session().as_deref(), Some("new-session"));
            tx.send(true).unwrap();
            let interrupted = tokio::time::timeout(Duration::from_secs(5), run)
                .await
                .expect("interrupt did not stop the turn")
                .unwrap();
            assert!(interrupted);
            assert!(fx.errors().is_empty());
        }

        #[tokio::test]
        async fn reports_a_missing_executable() {
            let fx = Fixture::new("");
            let (_tx, rx) = watch::channel(false);
            let missing = fx.dir.join("no-such-claude");
            assert!(!fx.turn_with(missing, None).run("Hi", rx).await);
            let errors = fx.errors();
            assert_eq!(errors.len(), 1);
            assert!(
                errors[0].starts_with("Could not start Claude Code"),
                "{}",
                errors[0]
            );
            assert!(errors[0].contains("no-such-claude"), "{}", errors[0]);
        }
    }
}
