//! Deterministic stand-in for `codex app-server`, linked into the unit-test binary.
//!
//! Process tests spawn the test executable itself as their Codex binary. Codex is always
//! started as `codex app-server ...`, so a constructor that runs before the test harness
//! recognizes that argument and serves the scenario instead. It reads `scenario.json` from
//! its working directory, appends every received message to `requests.jsonl`, and never
//! calls a model or runs requested commands.
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, Write};
use std::process::exit;

#[ctor::ctor(unsafe)]
fn serve_when_spawned_as_codex() {
    if std::env::args().nth(1).as_deref() == Some("app-server") {
        FakeServer::new().run();
    }
}

struct FakeServer {
    scenario: Value,
    log: File,
}

/// Python-style truthiness, so scenario flags can be `true`, a message, or a list.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

impl FakeServer {
    fn new() -> Self {
        let scenario = std::fs::read_to_string("scenario.json").expect("scenario.json");
        Self {
            scenario: serde_json::from_str(&scenario).expect("scenario is JSON"),
            log: OpenOptions::new()
                .create(true)
                .append(true)
                .open("requests.jsonl")
                .expect("requests.jsonl"),
        }
    }

    fn flag(&self, name: &str) -> bool {
        self.scenario.get(name).is_some_and(truthy)
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
            exit(0);
        }
        let message: Value = serde_json::from_str(&line).expect("client sent JSON");
        writeln!(self.log, "{message}").expect("log request");
        if message["method"] == "turn/interrupt" && !self.flag("ignoreInterrupt") {
            reply(&message, json!({}));
            event(
                "turn/completed",
                json!({"turn":{"id":"turn-1","status":"interrupted"}}),
            );
            exit(0);
        }
        message
    }

    /// Wait without responding, but exit once the client closes stdin.
    fn drain(&mut self) -> ! {
        loop {
            self.read();
        }
    }

    fn run(mut self) -> ! {
        loop {
            let message = self.read();
            match message["method"].as_str().unwrap_or_default() {
                "initialize" => {
                    if self.flag("hangInit") {
                        self.drain();
                    }
                    reply(&message, json!({"userAgent":"mock"}));
                }
                "config/read" => {
                    let config = self.scenario.get("config").cloned().unwrap_or_else(|| {
                        json!({"sandbox_mode":"read-only","approval_policy":"on-request","approvals_reviewer":"user"})
                    });
                    reply(&message, json!({ "config": config }));
                }
                "configRequirements/read" => {
                    let requirements = self.scenario["requirements"].clone();
                    reply(&message, json!({ "requirements": requirements }));
                }
                method @ ("thread/start" | "thread/resume") => {
                    if method == "thread/resume" && self.flag("resumeError") {
                        send(&json!({"id":message["id"],
                            "error":{"code":-32000,"message":self.scenario["resumeError"]}}));
                    } else {
                        reply(&message, json!({"thread":{"id":"thread-1"}}));
                    }
                }
                "turn/start" => self.turn(&message),
                _ => {}
            }
        }
    }

    fn turn(&mut self, message: &Value) {
        if self.flag("exitEarly") {
            exit(1);
        }
        if self.flag("malformed") {
            println!("not-json");
        }
        let delta = |text: &str| json!({"itemId":"text-1","delta":text});
        event("item/agentMessage/delta", delta("Hello "));
        // Deliberately send notifications before the turn/start response.
        event("item/agentMessage/delta", delta("world"));
        reply(message, json!({"turn":{"id":"turn-1"}}));
        send(&json!({"method":"item/agentMessage/delta","params":
            {"threadId":"other","turnId":"other","itemId":"x","delta":"IGNORE"}}));
        event(
            "item/completed",
            json!({"item":{"id":"text-1","type":"agentMessage","text":"Hello world"}}),
        );
        event(
            "thread/tokenUsage/updated",
            json!({"tokenUsage":{"last":{"totalTokens":250},"modelContextWindow":128000}}),
        );
        if self.flag("autoReview") {
            event(
                "item/autoApprovalReview/started",
                json!({"reviewId":"review-1","review":{"status":"inProgress"}}),
            );
            event(
                "item/autoApprovalReview/completed",
                json!({"reviewId":"review-1","review":{"status":"approved","rationale":"Read reference"}}),
            );
        }
        let requests = self.scenario["approvals"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let id =
            |index: usize, request: &Value| request.get("id").cloned().unwrap_or(json!(90 + index));
        for (index, request) in requests.iter().enumerate() {
            let mut params = json!({"threadId":"thread-1","turnId":"turn-1",
                "itemId":format!("tool-{index}"),"reason":"Need access"});
            merge(&mut params, request.get("params"));
            let method = request
                .get("method")
                .cloned()
                .unwrap_or(json!("item/commandExecution/requestApproval"));
            send(&json!({"id":id(index, request),"method":method,"params":params}));
        }
        if self.flag("resolve") {
            for (index, request) in requests.iter().enumerate() {
                event(
                    "serverRequest/resolved",
                    json!({"requestId":id(index, request)}),
                );
            }
        } else {
            for _ in &requests {
                self.read();
            }
        }
        if self.flag("hold") {
            self.drain();
        }
        let status = self
            .scenario
            .get("status")
            .cloned()
            .unwrap_or(json!("completed"));
        event(
            "turn/completed",
            json!({"turn":{"id":"turn-1","status":status,"error":{"message":"Generation failed"}}}),
        );
    }
}

fn merge(target: &mut Value, extra: Option<&Value>) {
    if let (Some(target), Some(Value::Object(extra))) = (target.as_object_mut(), extra) {
        target.extend(extra.clone());
    }
}

fn send(message: &Value) {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{message}").expect("write to client");
    out.flush().expect("flush to client");
}

fn reply(message: &Value, result: Value) {
    send(&json!({"id":message["id"],"result":result}));
}

fn event(method: &str, params: Value) {
    let mut p = json!({"threadId":"thread-1","turnId":"turn-1"});
    merge(&mut p, Some(&params));
    send(&json!({"method":method,"params":p}));
}
