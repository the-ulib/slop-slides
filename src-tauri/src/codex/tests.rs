use super::*;
use std::collections::BTreeSet;

mod fake_server;

fn args(dir: &Path) -> TurnArgs<'_> {
    TurnArgs {
        bin: dir,
        dir,
        lint_server: dir,
        deck_id: "deck-1",
        prompt: "Make slides",
        model: Some("test-model"),
        effort: Some("high"),
        session: None,
        mode: PermissionMode::Ask,
        approvals: Arc::default(),
    }
}

fn mcp_approval_params() -> Value {
    json!({"serverName":"slopslide","mode":"form","message":"Allow lint_deck?",
        "requestedSchema":{"type":"object","properties":{}},
        "_meta":{"codex_approval_kind":"mcp_tool_call","tool_params":{}}})
}

/// Codex app-server schemas for the pinned release, from `scripts/update-codex-schemas.sh`.
const SCHEMA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/codex-schemas");

fn pinned_schema(name: &str) -> Option<Value> {
    let text = std::fs::read_to_string(format!("{SCHEMA_DIR}/{name}.json")).ok()?;
    Some(serde_json::from_str(&text).expect("schema is JSON"))
}

fn property_names(schema: &Value, names: &mut BTreeSet<String>) {
    if let Some(props) = schema["properties"].as_object() {
        names.extend(props.keys().cloned());
    }
    for keyword in ["oneOf", "anyOf", "allOf"] {
        for branch in schema[keyword].as_array().into_iter().flatten() {
            property_names(branch, names);
        }
    }
}

/// Validate wire payloads against Codex's schemas, returning one message per problem.
fn validate_cases(
    schema: impl Fn(&str) -> Option<Value>,
    cases: &[(String, Value)],
) -> Vec<String> {
    if cases.is_empty() {
        return vec!["No contract cases were collected.".into()];
    }
    let mut failures = Vec::new();
    for (index, (name, payload)) in cases.iter().enumerate() {
        let label = format!("case {} ({name})", index + 1);
        let Some(schema) = schema(name) else {
            failures.push(format!(
                "{label}: no pinned schema; add it to scripts/update-codex-schemas.sh"
            ));
            continue;
        };
        let validator = match jsonschema::validator_for(&schema) {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!("{label}: invalid schema: {e}"));
                continue;
            }
        };
        // Codex accepts unknown fields; reject typos and removed field names in our payloads.
        let mut known = BTreeSet::new();
        property_names(&schema, &mut known);
        let unknown: Vec<_> = payload
            .as_object()
            .into_iter()
            .flat_map(|o| o.keys())
            .filter(|k| !known.contains(*k))
            .collect();
        if !unknown.is_empty() {
            failures.push(format!("{label}: unknown fields: {unknown:?}"));
        }
        for error in validator.iter_errors(payload) {
            let at = error.instance_path.to_string();
            let at = if at.is_empty() { "<root>" } else { &at };
            failures.push(format!("{label} at {at}: {error}"));
        }
    }
    failures
}

mod contract_check {
    use super::*;

    fn check(payload: Value) -> Vec<String> {
        let schema = |name: &str| {
            (name == "Approval").then(|| {
                json!({"type":"object","properties":{"action":{"enum":["accept","decline"]}},
                    "required":["action"]})
            })
        };
        validate_cases(schema, &[("Approval".into(), payload)])
    }

    #[test]
    fn valid_response_passes() {
        assert_eq!(check(json!({"action":"accept"})), Vec::<String>::new());
    }

    #[test]
    fn wrong_response_shape_fails() {
        assert!(!check(json!({"decision":"accept"})).is_empty());
    }

    #[test]
    fn invalid_decision_fails() {
        assert!(!check(json!({"action":"acceptForSession"})).is_empty());
    }

    #[test]
    fn unknown_fields_fail_even_when_upstream_ignores_them() {
        let failures = check(json!({"action":"accept","persist":"always"}));
        assert!(
            failures.iter().any(|f| f.contains("unknown fields")),
            "{failures:?}"
        );
    }

    #[test]
    fn missing_schema_and_empty_case_list_fail() {
        assert!(!validate_cases(pinned_schema, &[]).is_empty());
        let failures = validate_cases(pinned_schema, &[("Removed".into(), json!({}))]);
        assert!(failures[0].contains("no pinned schema"), "{failures:?}");
    }

    #[test]
    fn unknown_field_check_sees_properties_inside_one_of_branches() {
        let schema = |_: &str| {
            Some(json!({"oneOf":[
                {"type":"object","properties":{"a":{"type":"string"}},"required":["a"]},
                {"type":"object","properties":{"b":{"type":"string"}},"required":["b"]}]}))
        };
        assert!(validate_cases(schema, &[("U".into(), json!({"b":"x"}))]).is_empty());
        assert!(!validate_cases(schema, &[("U".into(), json!({"b":"x","c":1}))]).is_empty());
    }

    #[test]
    fn pinned_schemas_reject_a_wrong_permission_value() {
        let a = args(Path::new("/deck"));
        let mut p = thread_params(&a, &json!({}));
        let case = |p: &Value| vec![("ThreadStartParams".to_string(), p.clone())];
        assert_eq!(
            validate_cases(pinned_schema, &case(&p)),
            Vec::<String>::new()
        );
        p["sandbox"] = json!("workspace");
        assert!(!validate_cases(pinned_schema, &case(&p)).is_empty());
    }
}

/// Every payload we send, or answer with, must match the pinned Codex protocol schemas.
#[test]
fn codex_payloads_match_pinned_protocol_schemas() {
    let mut cases: Vec<(String, Value)> = Vec::new();
    let mut add = |schema: &str, payload: Value| cases.push((schema.into(), payload));
    let dir = std::env::temp_dir();
    let mut a = args(&dir);
    let config = json!({"sandbox_mode":"read-only","approval_policy":"on-request","approvals_reviewer":"user"});
    for mode in [
        PermissionMode::Ask,
        PermissionMode::AutoReview,
        PermissionMode::FullAccess,
        PermissionMode::Custom,
    ] {
        a.mode = mode;
        let mut p = thread_params(&a, &config);
        add("ThreadStartParams", p.clone());
        add(
            "ClientRequest",
            json!({"id":1,"method":"thread/start","params":p}),
        );
        p["threadId"] = json!("thread-1");
        add("ThreadResumeParams", p.clone());
        add(
            "ClientRequest",
            json!({"id":2,"method":"thread/resume","params":p}),
        );
    }
    for (method, input_schema, output_schema) in [
        (
            "item/commandExecution/requestApproval",
            "CommandExecutionRequestApprovalParams",
            "CommandExecutionRequestApprovalResponse",
        ),
        (
            "item/fileChange/requestApproval",
            "FileChangeRequestApprovalParams",
            "FileChangeRequestApprovalResponse",
        ),
        (
            "item/permissions/requestApproval",
            "PermissionsRequestApprovalParams",
            "PermissionsRequestApprovalResponse",
        ),
    ] {
        let p = json!({"threadId":"thread-1","turnId":"turn-1","itemId":"tool-1","startedAtMs":1,
            "cwd":dir,"command":"echo hello","permissions":{"network":{"enabled":true}}});
        // Include only fields belonging to each request type.
        let mut input =
            json!({"threadId":"thread-1","turnId":"turn-1","itemId":"tool-1","startedAtMs":1});
        if method == "item/commandExecution/requestApproval" {
            input["command"] = p["command"].clone();
        }
        if method == "item/permissions/requestApproval" {
            input["cwd"] = p["cwd"].clone();
            input["permissions"] = p["permissions"].clone();
        }
        let (_, responses) = approval_request(method, &input).unwrap();
        add(
            "ServerRequest",
            json!({"id":90,"method":method,"params":input}),
        );
        add(input_schema, input);
        for response in responses.into_values() {
            add(output_schema, response);
        }
    }
    for turn in [Value::Null, json!("turn-1")] {
        let mut p = mcp_approval_params();
        p["threadId"] = json!("thread-1");
        p["turnId"] = turn;
        p["_meta"]["persist"] = json!(["session", "always"]);
        let (_, responses) = approval_request("mcpServer/elicitation/request", &p).unwrap();
        add(
            "ServerRequest",
            json!({"id":"mcp-approval","method":"mcpServer/elicitation/request","params":p}),
        );
        add("McpServerElicitationRequestParams", p);
        for response in responses.into_values() {
            add("McpServerElicitationRequestResponse", response);
        }
    }
    // Capture the real transport's startup and interrupt messages too, rather than
    // maintaining hand-written copies of initialize/turn-start/config payloads.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let f = process::Fixture::new(
                json!({"approvals":[{"params":{"command":"ls"}}],"hold":true}),
            );
            let (tx, mut rx) = watch::channel(false);
            tokio::time::timeout(
                Duration::from_secs(10),
                run_turn(
                    f.args(),
                    &mut rx,
                    &|e| {
                        if matches!(e, AgentEvent::ApprovalRequested { .. }) {
                            tx.send(true).unwrap();
                        }
                    },
                    &|_| Ok(()),
                ),
            )
            .await
            .expect("contract transport capture timed out")
            .unwrap();
            for request in f.requests() {
                let schema = match request["method"].as_str() {
                    Some("initialize") => "InitializeParams",
                    Some("config/read") => "ConfigReadParams",
                    Some("configRequirements/read") => {
                        // This parameterless RPC has no separate Params schema.
                        add("ClientRequest", request.clone());
                        continue;
                    }
                    Some("turn/start") => "TurnStartParams",
                    Some("turn/interrupt") => "TurnInterruptParams",
                    _ => continue,
                };
                add("ClientRequest", request.clone());
                add(schema, request["params"].clone());
            }
        });
    let failures = validate_cases(pinned_schema, &cases);
    let version = std::fs::read_to_string(format!("{SCHEMA_DIR}/VERSION")).unwrap();
    assert!(
        failures.is_empty(),
        "Codex {} protocol mismatches:\n{}",
        version.trim(),
        failures.join("\n")
    );
}

#[test]
fn mcp_tool_approvals_use_elicitation_actions_without_persistent_grants() {
    let mut p = mcp_approval_params();
    for persist in [
        Value::Null,
        json!("session"),
        json!(["session", "always"]),
        json!("always"),
    ] {
        p["_meta"]["persist"] = persist.clone();
        let (a, result) = approval_request("mcpServer/elicitation/request", &p).unwrap();
        assert_eq!(result["accept"], json!({"action":"accept","content":{}}));
        assert_eq!(result["decline"], json!({"action":"decline"}));
        assert!(a.details.contains("slopslide"));
        assert_eq!(a.reason.as_deref(), Some("Allow lint_deck?"));
        let session = persist == "session" || persist.is_array();
        assert_eq!(a.decisions.contains(&Decision::AcceptForSession), session);
        if session {
            assert_eq!(
                result["acceptForSession"],
                json!({"action":"accept","content":{},"_meta":{"persist":"session"}})
            );
        }
        assert!(result.values().all(|v| v["_meta"]["persist"] != "always"));
    }
}

#[test]
fn mcp_input_forms_and_authentication_are_not_treated_as_tool_approvals() {
    for mode in [
        "url",
        "openai/userVerification",
        "openai/form",
        "openaiForm",
    ] {
        let mut p = mcp_approval_params();
        p["mode"] = json!(mode);
        assert!(approval_request("mcpServer/elicitation/request", &p).is_none());
    }
    for (key, value) in [
        ("properties", json!({"confirmed":{"type":"boolean"}})),
        ("required", json!(["confirmed"])),
        ("type", json!("string")),
    ] {
        let mut p = mcp_approval_params();
        p["requestedSchema"][key] = value;
        assert!(approval_request("mcpServer/elicitation/request", &p).is_none());
    }
    for meta in [
        Value::Null,
        json!({"codex_approval_kind":"browser_auth"}),
        json!({"codex_approval_kind":"mcp_tool_call","codex_requires_user_input":true}),
    ] {
        let mut p = mcp_approval_params();
        p["_meta"] = meta;
        assert!(approval_request("mcpServer/elicitation/request", &p).is_none());
    }
}

#[test]
fn policies_are_explicit_and_custom_resets_resumed_threads() {
    let dir = Path::new("/deck");
    let mut a = args(dir);
    let config = json!({"sandbox_mode":"read-only","approval_policy":"never","approvals_reviewer":"auto_review"});
    for (mode, sandbox, policy, reviewer) in [
        (PermissionMode::Ask, "workspace-write", "on-request", "user"),
        (
            PermissionMode::AutoReview,
            "workspace-write",
            "on-request",
            "auto_review",
        ),
        (
            PermissionMode::FullAccess,
            "danger-full-access",
            "never",
            "user",
        ),
        (PermissionMode::Custom, "read-only", "never", "auto_review"),
    ] {
        a.mode = mode;
        let p = thread_params(&a, &config);
        assert_eq!(p["sandbox"], sandbox);
        assert_eq!(p["approvalPolicy"], policy);
        assert_eq!(p["approvalsReviewer"], reviewer);
        assert_eq!(p["cwd"], "/deck");
        assert_eq!(p["model"], "test-model");
        assert_eq!(p["developerInstructions"], SYSTEM_PROMPT);
    }
    a.mode = PermissionMode::Custom;
    let p = thread_params(&a, &json!({}));
    assert_eq!(p["sandbox"], "read-only");
    assert_eq!(p["approvalPolicy"], "on-request");
    assert!(p.get("approvalsReviewer").is_none());
}

#[test]
fn managed_restrictions_and_older_clients_limit_the_menu() {
    let c = json!({"approvals_reviewer":null});
    assert_eq!(
        supported_modes(&c, &Value::Null),
        vec![
            PermissionMode::Ask,
            PermissionMode::AutoReview,
            PermissionMode::FullAccess,
            PermissionMode::Custom
        ]
    );
    assert_eq!(
        supported_modes(&json!({}), &Value::Null),
        vec![
            PermissionMode::Ask,
            PermissionMode::FullAccess,
            PermissionMode::Custom
        ]
    );
    let requirements = json!({"allowedSandboxModes":["workspace-write"],"allowedApprovalPolicies":["on-request"],"allowedApprovalsReviewers":["auto_review"]});
    assert_eq!(
        supported_modes(&c, &requirements),
        vec![PermissionMode::AutoReview, PermissionMode::Custom]
    );
    assert_eq!(
        supported_modes(&c, &json!({"allowedSandboxModes":[]})),
        vec![PermissionMode::Custom]
    );
}

#[test]
fn approval_choices_never_invent_a_grant_or_persistent_rule() {
    let (a,results)=approval_request("item/commandExecution/requestApproval",&json!({"command":"ls","availableDecisions":["decline",{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["ls"]}}]})).unwrap();
    assert_eq!(a.decisions, vec![Decision::Decline]);
    assert!(!results.contains_key("accept"));
    let (a, results) = approval_request(
        "item/permissions/requestApproval",
        &json!({"permissions":{"network":{"enabled":true}}}),
    )
    .unwrap();
    assert_eq!(a.accept_label, "Allow for this turn");
    assert!(!a.decisions.contains(&Decision::AcceptForSession));
    assert_eq!(
        results["accept"],
        json!({"permissions":{"network":{"enabled":true}},"scope":"turn"})
    );
    assert_eq!(results["decline"], json!({"permissions":{},"scope":"turn"}));
    assert!(approval_request("unknown", &json!({})).is_none());
    let (a,_)=approval_request("item/commandExecution/requestApproval",&json!({"command":"irrelevant","networkApprovalContext":{"host":"example.com","protocol":"https"}})).unwrap();
    assert_eq!(a.title, "Network access");
    assert!(a.details.contains("example.com"));
    assert!(a.details.contains("irrelevant"));
}

#[test]
fn command_approval_displays_the_requested_access_and_execution_context() {
    let (a, _) = approval_request(
        "item/commandExecution/requestApproval",
        &json!({"command":"python script.py", "cwd":"/reference", "kind":"stdin",
            "additionalPermissions":{"fileSystem":{"write":["/outside"]}}}),
    )
    .unwrap();
    for detail in [
        "python script.py",
        "/reference",
        "stdin",
        "/outside",
        "additionalPermissions",
    ] {
        assert!(a.details.contains(detail));
    }
}

#[test]
fn mapper_streams_text_once_and_preserves_tools_usage_and_reviews() {
    let mut m = EventMapper::default();
    assert_eq!(
        m.map(
            "item/agentMessage/delta",
            &json!({"itemId":"a","delta":"Hello"})
        ),
        vec![
            AgentEvent::TextStart,
            AgentEvent::TextDelta {
                text: "Hello".into()
            }
        ]
    );
    assert_eq!(
        m.map(
            "item/agentMessage/delta",
            &json!({"itemId":"a","delta":"!"})
        ),
        vec![AgentEvent::TextDelta { text: "!".into() }]
    );
    assert!(m
        .map(
            "item/completed",
            &json!({"item":{"id":"a","type":"agentMessage","text":"Hello!"}})
        )
        .is_empty());
    let edit = json!({"item":{"id":"e","type":"fileChange","changes":[{"path":"/deck/deck.html","diff":"+Slide"}],"status":"inProgress"}});
    assert!(
        matches!(&m.map("item/started",&edit)[..],[AgentEvent::ToolUse{name,..}] if name=="Edit")
    );
    let details = m.approval_details("item/fileChange/requestApproval", &json!({"itemId":"e"}));
    assert_eq!(details["changes"][0]["diff"], "+Slide");
    assert!(matches!(
        &m.map(
            "item/completed",
            &json!({"item":{"id":"e","type":"fileChange","status":"declined"}})
        )[..],
        [AgentEvent::ToolResult { is_error: true, .. }]
    ));
    assert_eq!(
        m.map(
            "thread/tokenUsage/updated",
            &json!({"tokenUsage":{"last":{"totalTokens":42},"modelContextWindow":100}})
        ),
        vec![AgentEvent::Usage {
            context_tokens: Some(42),
            context_window: Some(100)
        }]
    );
    assert!(
        matches!(&m.map("item/autoApprovalReview/completed",&json!({"reviewId":"r","review":{"status":"denied","rationale":"Outside scope"}}))[..],[AgentEvent::ApprovalReview{status,detail:Some(detail),..}] if status=="denied" && detail=="Outside scope")
    );
    assert!(m
        .map(
            "error",
            &json!({"willRetry":true,"error":{"message":"retry"}})
        )
        .is_empty());
}

mod process {
    use super::*;

    pub(super) struct Fixture {
        dir: std::path::PathBuf,
        bin: std::path::PathBuf,
    }
    impl Fixture {
        pub(super) fn new(scenario: Value) -> Self {
            let dir =
                std::env::temp_dir().join(format!("slopslide codex test {}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            // This test binary doubles as `codex app-server`; see fake_server.rs.
            let bin = std::env::current_exe().unwrap();
            std::fs::write(dir.join("scenario.json"), scenario.to_string()).unwrap();
            Self { dir, bin }
        }
        pub(super) fn args(&self) -> TurnArgs<'_> {
            let mut a = args(&self.dir);
            a.bin = &self.bin;
            a
        }
        pub(super) fn requests(&self) -> Vec<Value> {
            std::fs::read_to_string(self.dir.join("requests.jsonl"))
                .unwrap()
                .lines()
                .map(|s| serde_json::from_str(s).unwrap())
                .collect()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
    fn run(f: impl std::future::Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(10), f)
                    .await
                    .expect("mock turn timed out");
            });
    }

    #[test]
    fn mcp_approvals_route_actions_and_accept_nullable_turn_correlation() {
        run(async {
            for (tool, turn, decision) in [
                (mcp::READ_NARRATION, json!("turn-1"), Decision::Accept),
                (mcp::WRITE_NARRATION, Value::Null, Decision::Decline),
                (
                    mcp::WRITE_NARRATION,
                    json!("turn-1"),
                    Decision::AcceptForSession,
                ),
            ] {
                let mut params = mcp_approval_params();
                params["message"] = json!(format!("Allow {tool}?"));
                params["turnId"] = turn;
                params["_meta"]["persist"] = json!("session");
                let f = Fixture::new(
                    json!({"approvals":[{"method":"mcpServer/elicitation/request","params":params}]}),
                );
                let a = f.args();
                let broker = a.approvals.clone();
                let (_tx, mut rx) = watch::channel(false);
                let seen = Mutex::new(false);
                let outcome = run_turn(
                    a,
                    &mut rx,
                    &|e| {
                        if let AgentEvent::ApprovalRequested { approval } = e {
                            *seen.lock().unwrap() = true;
                            broker.respond("deck-1", &approval.id, decision).unwrap();
                        }
                    },
                    &|_| Ok(()),
                )
                .await
                .unwrap();
                assert!(matches!(outcome, Outcome::Done));
                assert!(*seen.lock().unwrap());
                let requests = f.requests();
                let start = requests
                    .iter()
                    .find(|r| r["method"] == "thread/start")
                    .unwrap();
                assert_eq!(start["params"]["approvalPolicy"], "on-request");
                assert_eq!(start["params"]["sandbox"], "workspace-write");
                let response = requests.into_iter().find(|r| r["id"] == 90).unwrap();
                assert_eq!(
                    response["result"]["action"],
                    if decision == Decision::Decline {
                        "decline"
                    } else {
                        "accept"
                    }
                );
                assert_eq!(
                    response["result"]["_meta"]["persist"] == "session",
                    decision == Decision::AcceptForSession
                );
                assert!(response["result"].get("decision").is_none());
                assert!(broker.0.lock().unwrap().is_empty());
            }
        });
    }

    #[test]
    fn mcp_requests_from_other_threads_or_turns_are_rejected() {
        run(async {
            for (thread, turn) in [("other", Value::Null), ("thread-1", json!("other"))] {
                let mut params = mcp_approval_params();
                params["threadId"] = json!(thread);
                params["turnId"] = turn;
                let f = Fixture::new(
                    json!({"approvals":[{"method":"mcpServer/elicitation/request","params":params}]}),
                );
                let (_tx, mut rx) = watch::channel(false);
                run_turn(
                    f.args(),
                    &mut rx,
                    &|e| assert!(!matches!(e, AgentEvent::ApprovalRequested { .. })),
                    &|_| Ok(()),
                )
                .await
                .unwrap();
                assert!(f
                    .requests()
                    .iter()
                    .any(|r| r["id"] == 90 && r.get("error").is_some()));
            }
        });
    }

    #[test]
    fn complete_turn_buffers_early_events_and_filters_other_threads() {
        run(async {
            let f = Fixture::new(json!({"malformed":true,"autoReview":true}));
            let a = f.args();
            let events = Mutex::new(Vec::new());
            let session = Mutex::new(None);
            let (_tx, mut rx) = watch::channel(false);
            let outcome = run_turn(
                a,
                &mut rx,
                &|e| events.lock().unwrap().push(e.clone()),
                &|s| {
                    *session.lock().unwrap() = Some(s.to_string());
                    Ok(())
                },
            )
            .await
            .unwrap();
            assert!(matches!(outcome, Outcome::Done));
            assert_eq!(*session.lock().unwrap(), Some("thread-1".into()));
            let events = events.into_inner().unwrap();
            let text: String = events
                .iter()
                .filter_map(|e| {
                    if let AgentEvent::TextDelta { text } = e {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(text, "Hello world");
            assert!(events.iter().any(|e| matches!(
                e,
                AgentEvent::Usage {
                    context_tokens: Some(250),
                    ..
                }
            )));
            assert!(events
                .iter()
                .any(|e| matches!(e,AgentEvent::ApprovalReview{status,..} if status=="approved")));
            assert!(matches!(
                events.last(),
                Some(AgentEvent::Result {
                    is_error: false,
                    ..
                })
            ));
            let requests = f.requests();
            let p = &requests
                .iter()
                .find(|r| r["method"] == "thread/start")
                .unwrap()["params"];
            assert_eq!(p["sandbox"], "workspace-write");
            assert_eq!(p["approvalsReviewer"], "user");
            assert_eq!(
                requests
                    .iter()
                    .find(|r| r["method"] == "turn/start")
                    .unwrap()["params"]["effort"],
                "high"
            );
        });
    }

    #[test]
    fn concurrent_approvals_route_numeric_and_string_ids_and_reject_stale_decisions() {
        run(async {
            let f = Fixture::new(
                json!({"approvals":[{"id":90,"params":{"command":"ls","availableDecisions":["accept","decline"]}},{"id":"ninety-one","method":"item/fileChange/requestApproval","params":{"grantRoot":"/outside"}}]}),
            );
            let a = f.args();
            let broker = a.approvals.clone();
            let events = Mutex::new(Vec::new());
            let (_tx, mut rx) = watch::channel(false);
            let outcome = run_turn(
                a,
                &mut rx,
                &|e| {
                    events.lock().unwrap().push(e.clone());
                    if let AgentEvent::ApprovalRequested { approval } = e {
                        assert!(broker
                            .respond("other-deck", &approval.id, Decision::Accept)
                            .is_err());
                        if approval.title == "Run a command" {
                            assert!(broker
                                .respond("deck-1", &approval.id, Decision::AcceptForSession)
                                .is_err());
                        }
                        broker
                            .respond(
                                "deck-1",
                                &approval.id,
                                if approval.title == "Run a command" {
                                    Decision::Accept
                                } else {
                                    Decision::Decline
                                },
                            )
                            .unwrap();
                        assert!(broker
                            .respond("deck-1", &approval.id, Decision::Accept)
                            .is_err());
                    }
                },
                &|_| Ok(()),
            )
            .await
            .unwrap();
            assert!(matches!(outcome, Outcome::Done));
            let requests = f.requests();
            assert!(requests
                .iter()
                .any(|r| r["id"] == 90 && r["result"]["decision"] == "accept"));
            assert!(requests
                .iter()
                .any(|r| r["id"] == "ninety-one" && r["result"]["decision"] == "decline"));
            assert_eq!(
                events
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|e| matches!(e, AgentEvent::ApprovalResolved { .. }))
                    .count(),
                2
            );
            assert!(broker.0.lock().unwrap().is_empty());
        });
    }

    #[test]
    fn permission_grants_are_scoped_to_the_turn() {
        run(async {
            let f = Fixture::new(
                json!({"approvals":[{"method":"item/permissions/requestApproval","params":{"permissions":{"fileSystem":{"read":["/reference"]}}}}]}),
            );
            let a = f.args();
            let broker = a.approvals.clone();
            let (_tx, mut rx) = watch::channel(false);
            run_turn(
                a,
                &mut rx,
                &|e| {
                    if let AgentEvent::ApprovalRequested { approval } = e {
                        broker
                            .respond("deck-1", &approval.id, Decision::Accept)
                            .unwrap();
                    }
                },
                &|_| Ok(()),
            )
            .await
            .unwrap();
            let r = f
                .requests()
                .into_iter()
                .find(|r| r["id"] == 90 && r.get("result").is_some())
                .unwrap();
            assert_eq!(r["result"]["scope"], "turn");
            assert_eq!(
                r["result"]["permissions"]["fileSystem"]["read"][0],
                "/reference"
            );
        });
    }

    #[test]
    fn stopping_a_pending_approval_clears_it_without_granting() {
        run(async {
            let f = Fixture::new(json!({"approvals":[{"params":{"command":"ls"}}],"hold":true}));
            let a = f.args();
            let broker = a.approvals.clone();
            let (tx, mut rx) = watch::channel(false);
            let id = Mutex::new(String::new());
            let outcome = run_turn(
                a,
                &mut rx,
                &|e| {
                    if let AgentEvent::ApprovalRequested { approval } = e {
                        *id.lock().unwrap() = approval.id.clone();
                        tx.send(true).unwrap();
                    }
                },
                &|_| Ok(()),
            )
            .await
            .unwrap();
            assert!(matches!(outcome, Outcome::Interrupted));
            assert!(broker.0.lock().unwrap().is_empty());
            assert!(broker
                .respond("deck-1", &id.lock().unwrap(), Decision::Accept)
                .is_err());
            assert!(!f
                .requests()
                .iter()
                .any(|r| r["id"] == 90 && r.get("result").is_some()));
            assert!(f.requests().iter().any(|r| r["method"] == "turn/interrupt"
                && r["params"] == json!({"threadId":"thread-1","turnId":"turn-1"})));
        });
    }

    #[test]
    fn stopping_still_finishes_when_the_server_ignores_interrupt() {
        run(async {
            let f = Fixture::new(
                json!({"approvals":[{"params":{"command":"ls"}}],"hold":true,"ignoreInterrupt":true}),
            );
            let a = f.args();
            let broker = a.approvals.clone();
            let (tx, mut rx) = watch::channel(false);
            let outcome = tokio::time::timeout(
                Duration::from_secs(4),
                run_turn(
                    a,
                    &mut rx,
                    &|e| {
                        if matches!(e, AgentEvent::ApprovalRequested { .. }) {
                            tx.send(true).unwrap();
                        }
                    },
                    &|_| Ok(()),
                ),
            )
            .await
            .expect("Stop must not wait indefinitely")
            .unwrap();
            assert!(matches!(outcome, Outcome::Interrupted));
            assert!(broker.0.lock().unwrap().is_empty());
            assert!(!f
                .requests()
                .iter()
                .any(|r| r["id"] == 90 && r.get("result").is_some()));
        });
    }

    #[test]
    fn server_resolution_expires_the_request() {
        run(async {
            let f = Fixture::new(json!({"approvals":[{"params":{"command":"ls"}}],"resolve":true}));
            let a = f.args();
            let broker = a.approvals.clone();
            let id = Mutex::new(String::new());
            let (_tx, mut rx) = watch::channel(false);
            run_turn(
                a,
                &mut rx,
                &|e| match e {
                    AgentEvent::ApprovalRequested { approval } => {
                        *id.lock().unwrap() = approval.id.clone()
                    }
                    AgentEvent::ApprovalResolved { id } => {
                        assert!(broker.respond("deck-1", id, Decision::Accept).is_err())
                    }
                    _ => {}
                },
                &|_| Ok(()),
            )
            .await
            .unwrap();
            assert!(!id.lock().unwrap().is_empty());
            assert!(broker.0.lock().unwrap().is_empty());
        });
    }

    #[test]
    fn resume_reapplies_policy_and_only_missing_threads_trigger_fallback() {
        run(async {
            for mode in [
                PermissionMode::Ask,
                PermissionMode::AutoReview,
                PermissionMode::FullAccess,
                PermissionMode::Custom,
            ] {
                let f = Fixture::new(json!({}));
                let mut a = f.args();
                a.session = Some("saved");
                a.mode = mode;
                let (_tx, mut rx) = watch::channel(false);
                run_turn(a, &mut rx, &|_| {}, &|_| Ok(())).await.unwrap();
                let p = f
                    .requests()
                    .into_iter()
                    .find(|r| r["method"] == "thread/resume")
                    .unwrap()["params"]
                    .clone();
                assert_eq!(p["threadId"], "saved");
                assert_eq!(
                    p["sandbox"],
                    if mode == PermissionMode::FullAccess {
                        "danger-full-access"
                    } else if mode == PermissionMode::Custom {
                        "read-only"
                    } else {
                        "workspace-write"
                    }
                );
            }
            for (message, missing) in [
                ("no rollout found for thread", true),
                ("Permission denied by administrator", false),
            ] {
                let f = Fixture::new(json!({"resumeError":message}));
                let mut a = f.args();
                a.session = Some("saved");
                let (_tx, mut rx) = watch::channel(false);
                let r = run_turn(a, &mut rx, &|_| {}, &|_| Ok(())).await;
                if missing {
                    assert!(matches!(r, Ok(Outcome::ResumeFailed)));
                } else {
                    assert!(r.is_err());
                }
                assert!(!f.requests().iter().any(|r| r["method"] == "turn/start"));
            }
        });
    }

    #[test]
    fn unsupported_requests_are_rejected_and_failed_turns_are_reported() {
        run(async {
            let f = Fixture::new(
                json!({"approvals":[{"method":"unsupported/request"}],"status":"failed"}),
            );
            let a = f.args();
            let events = Mutex::new(Vec::new());
            let (_tx, mut rx) = watch::channel(false);
            run_turn(
                a,
                &mut rx,
                &|e| events.lock().unwrap().push(e.clone()),
                &|_| Ok(()),
            )
            .await
            .unwrap();
            assert!(f
                .requests()
                .iter()
                .any(|r| r["id"] == 90 && r["error"]["code"] == -32601));
            assert!(events.lock().unwrap().iter().any(|e|matches!(e,AgentEvent::Result{is_error:true,text:Some(t),..} if t=="Generation failed")));
        });
    }

    #[test]
    fn restrictions_and_process_exit_fail_without_starting_unintended_work() {
        run(async {
            let f = Fixture::new(json!({"requirements":{"allowedSandboxModes":["read-only"]}}));
            let a = f.args();
            let (_tx, mut rx) = watch::channel(false);
            assert!(run_turn(a, &mut rx, &|_| {}, &|_| Ok(())).await.is_err());
            assert!(!f.requests().iter().any(|r| r["method"] == "thread/start"));
            let f = Fixture::new(json!({"exitEarly":true}));
            let a = f.args();
            let broker = a.approvals.clone();
            let (_tx, mut rx) = watch::channel(false);
            assert!(run_turn(a, &mut rx, &|_| {}, &|_| Ok(())).await.is_err());
            assert!(broker.0.lock().unwrap().is_empty());
        });
    }

    #[test]
    fn cancellation_interrupts_a_hung_initialization() {
        run(async {
            let f = Fixture::new(json!({"hangInit":true}));
            let a = f.args();
            let broker = a.approvals.clone();
            let (tx, mut rx) = watch::channel(false);
            let cancel = async {
                // Wait until the child actually received initialize, rather than relying on timing.
                loop {
                    let log =
                        std::fs::read_to_string(f.dir.join("requests.jsonl")).unwrap_or_default();
                    if log.contains("initialize") {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                tx.send(true).unwrap();
            };
            let (outcome, ()) = tokio::join!(run_turn(a, &mut rx, &|_| {}, &|_| Ok(())), cancel);
            assert!(matches!(outcome, Ok(Outcome::Interrupted)));
            assert!(broker.0.lock().unwrap().is_empty());
            assert!(!f.requests().iter().any(|r| r["method"] == "thread/start"));
        });
    }

    #[test]
    fn cancellation_already_set_does_not_begin_a_turn() {
        run(async {
            let f = Fixture::new(json!({}));
            let a = f.args();
            let (tx, mut rx) = watch::channel(false);
            tx.send(true).unwrap();
            assert!(matches!(
                run_turn(a, &mut rx, &|_| {}, &|_| Ok(())).await,
                Ok(Outcome::Interrupted)
            ));
        });
    }
}

/// Explicit opt-in: exercise the real MCP approval path using only the app's read-only linter.
#[test]
#[ignore = "requires a signed-in Codex CLI, network access, and subscription usage"]
fn real_codex_lint_approval_test() {
    real_codex_read_tool_approval_test("lint_deck");
}

/// Explicit opt-in: reproduce narration's MCP approval path without changing a deck.
#[test]
#[ignore = "requires a signed-in Codex CLI, network access, and subscription usage"]
fn real_codex_narration_approval_test() {
    real_codex_read_tool_approval_test("read_narration");
}

fn real_codex_read_tool_approval_test(tool: &str) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let bin = env::resolve_codex().expect("Codex installed");
        let dir = std::env::temp_dir().join(format!("slopslide-lint-smoke-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("deck.html"), "<html><body><main class=\"deck\"><section class=\"slide\" id=\"smoke\"><div></section></main></body></html>").unwrap();
        let lint = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/slopslide");
        assert!(lint.is_file(), "Build the app before running this test");
        let events = Mutex::new(Vec::new());
        let mut a = args(&dir);
        a.bin = &bin;
        a.lint_server = &lint;
        a.model = None;
        a.effort = Some("low");
        let prompt = format!("Call the slopslide MCP {tool} tool exactly once, then briefly report its findings. Use tool search to discover it if needed. Do not run shell commands, call other MCP tools, or change any files.");
        a.prompt = &prompt;
        let broker = a.approvals.clone();
        let (_tx, mut rx) = watch::channel(false);
        let outcome = tokio::time::timeout(Duration::from_secs(90), run_turn(
            a, &mut rx,
            &|e| {
                events.lock().unwrap().push(e.clone());
                if let AgentEvent::ApprovalRequested { approval } = e {
                    // Never approve unrelated tools or shell/file access in the live test.
                    assert_eq!(approval.title, "Use an MCP tool");
                    assert!(approval.details.contains("slopslide"));
                    assert!(approval.reason.as_deref().unwrap_or_default().contains(tool));
                    broker.respond("deck-1", &approval.id, Decision::Accept).unwrap();
                }
            }, &|_| Ok(()),
        )).await.expect("live read-only MCP turn timed out").unwrap();
        let events = events.into_inner().unwrap();
        assert!(matches!(outcome, Outcome::Done));
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ApprovalRequested { .. })), "{events:?}");
        let tool_id = events.iter().find_map(|e| match e {
            AgentEvent::ToolUse { id, name, .. } if name == tool => Some(id),
            _ => None,
        }).expect("real read-only MCP tool was called");
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolResult { id, is_error: false } if id == tool_id)), "{events:?}");
        assert!(matches!(events.last(), Some(AgentEvent::Result { is_error: false, .. })), "{events:?}");
        let _ = std::fs::remove_dir_all(dir);
    });
}

/// Explicit opt-in: inspect effective presets without starting inference or running tools.
#[test]
#[ignore = "requires an installed Codex CLI and its local configuration"]
fn real_codex_permission_modes_test() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let bin = env::resolve_codex().expect("Codex installed");
        let dir =
            std::env::temp_dir().join(format!("slopslide-policy-smoke-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut server = Server::spawn(&bin, Some(&dir), None).unwrap();
        server.initialize().await.unwrap();
        let (config, requirements) = server.configuration(&dir).await.unwrap();
        for (mode, sandbox, policy, reviewer) in [
            (PermissionMode::Ask, "workspaceWrite", "on-request", "user"),
            (
                PermissionMode::AutoReview,
                "workspaceWrite",
                "on-request",
                "auto_review",
            ),
            (
                PermissionMode::FullAccess,
                "dangerFullAccess",
                "never",
                "user",
            ),
        ] {
            if !supported_modes(&config, &requirements).contains(&mode) {
                continue;
            }
            let mut a = args(&dir);
            a.mode = mode;
            a.model = None;
            let mut params = thread_params(&a, &config);
            params["ephemeral"] = json!(true);
            // No turn is started: inspect effective permissions without executing tools or inference.
            let response = server.request("thread/start", params).await.unwrap();
            assert_eq!(response["sandbox"]["type"], sandbox, "{mode:?}");
            assert_eq!(response["approvalPolicy"], policy, "{mode:?}");
            assert_eq!(response["approvalsReviewer"], reviewer, "{mode:?}");
            if mode != PermissionMode::FullAccess {
                assert_eq!(response["sandbox"]["networkAccess"], false);
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    });
}

/// Explicit opt-in: two tiny inference turns using the local ChatGPT login. No deck edits.
#[test]
#[ignore = "requires a signed-in Codex CLI, network access, and subscription usage"]
fn real_codex_smoke_test() {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let bin=env::resolve_codex().expect("Codex installed");
        let dir=std::env::temp_dir().join(format!("slopslide-real-smoke-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let lint=Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/slopslide");
        let mut probe=Server::spawn(&bin,Some(&dir),None).unwrap();
        probe.initialize().await.unwrap();
        let models=probe.request("model/list",json!({})).await.unwrap();
        let model=models["data"].as_array().unwrap().iter().find(|m| m["isDefault"]==true).or_else(||models["data"].as_array().unwrap().first()).unwrap()["model"].as_str().unwrap();
        let mut session=None;
        for (mode,prompt) in [(PermissionMode::Ask,"Reply exactly SLOPSLIDE_SMOKE_OK. Do not use tools or change files."),(PermissionMode::Custom,"What exact marker did you reply with in your previous turn? Reply only with that marker. Do not use tools or change files.")] {
            let events=Mutex::new(Vec::new());
            let saved=Mutex::new(None);
            let mut a=args(&dir);a.bin=&bin;a.lint_server=&lint;a.model=Some(model);a.effort=Some("low");a.prompt=prompt;a.mode=mode;a.session=session.as_deref();
            let (_tx,mut rx)=watch::channel(false);
            let outcome=tokio::time::timeout(Duration::from_secs(90),run_turn(a,&mut rx,&|e|events.lock().unwrap().push(e.clone()),&|s|{*saved.lock().unwrap()=Some(s.to_string());Ok(())})).await.expect("real turn timed out").unwrap();
            let events=events.into_inner().unwrap();
            assert!(matches!(outcome,Outcome::Done));
            assert!(matches!(events.last(),Some(AgentEvent::Result{is_error:false,..})),"{events:?}");
            let text:String=events.iter().filter_map(|e|if let AgentEvent::TextDelta{text}=e{Some(text.as_str())}else{None}).collect();
            assert!(text.contains("SLOPSLIDE_SMOKE_OK"),"{events:?}");
            if let Some(previous)=&session {assert_eq!(saved.lock().unwrap().as_ref(),Some(previous));}
            session=saved.into_inner().unwrap();
        }
        let _=std::fs::remove_dir_all(dir);
    });
}
