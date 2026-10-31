use super::*;

#[test]
fn advertised_actions_follow_the_authorized_grant_for_remote_sessions() {
    use crate::core::{Hub, SessionOrigin, SessionRegistration};
    let hub = Hub::new();
    hub.create_session(SessionRegistration {
        id: "parent".into(),
        provider: "codex".into(),
        provider_version: String::new(),
        provider_generation_digest: String::new(),
        provider_account_id: crate::provider_account::ProviderAccountId::default(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: "runtime".into(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        execution_binding: Some(crate::execution_environment::fixture()),
        cwd: "/runtime/session".into(),
        title: String::new(),
        origin: SessionOrigin::Web,
        system: false,
        owner_user_id: Some("owner".into()),
        owner_username: None,
    });
    let info = hub.session_info("parent").unwrap();
    let mut grant: Grant = serde_json::from_value(json!({
        "grant_id":"grant-test", "parent_session_id":"parent", "agent_api":true,
    }))
    .unwrap();
    // An old execution Machine cannot gain writes through a newer runtime.
    assert_eq!(status_view(&info, true, &grant)["actions"], false);
    // A new execution Machine can expose writes even with an older runtime.
    grant.agent_actions = true;
    let status = status_view(&info, false, &grant);
    assert_eq!(status["actions"], true);
    assert_eq!(status["calls"]["enabled"], false);
    assert_eq!(status["session"]["id"], "parent");
}

#[test]
fn large_text_pages_preserve_unicode_and_signal_the_exact_continuation() {
    let original = "a中🙂b".repeat(100_000);
    let mut value = json!({"body":original});
    text_window(&mut value, "body", &json!({"offset":1,"chars":2})).unwrap();
    assert_eq!(value["body"], "中🙂");
    assert_eq!(value["range"], json!({"offset":1,"next":3,"total":400_000}));
    let mut result = json!({"text":"a中🙂b","truncated":true});
    text_window(&mut result, "text", &json!({"offset":3,"chars":2})).unwrap();
    assert_eq!(result["text"], "b");
    assert_eq!(result["range"]["next"], Value::Null);
    assert_eq!(result["truncated"], true);
    assert!(text_window(&mut result, "text", &json!({"chars":0})).is_err());
    assert!(text_window(&mut result, "text", &json!({"offset":-1})).is_err());
    let mut default_page = json!({"body": original});
    assert_eq!(
        text_window(&mut default_page, "body", &json!({})),
        Err("text_window_required".into())
    );
    assert_eq!(default_page["body"], original); // a failed read never presents a prefix as complete
    text_window(&mut default_page, "body", &json!({"offset":0})).unwrap();
    assert_eq!(default_page["body"].as_str().unwrap().chars().count(), 1000);
    assert_eq!(default_page["range"]["next"], 1000);
    let mut retained_result = json!({"text":"finding ".repeat(200),"truncated":false});
    assert_eq!(
        text_window(&mut retained_result, "text", &json!({})),
        Err("text_window_required".into())
    );
    let mut small = json!({"body":"whole body"});
    text_window(&mut small, "body", &json!({})).unwrap();
    assert_eq!(small["body"], "whole body");
    assert_eq!(small["range"]["next"], Value::Null);
}

#[test]
fn queued_messages_are_bounded_and_cursor_loss_is_explicit() {
    let messages: Vec<QueuedMessage> = (0..30)
        .map(|n| {
            serde_json::from_value(json!({
                "id":format!("message-{n}"),"text":"中🙂".repeat(4000)
            }))
            .unwrap()
        })
        .collect();
    assert_eq!(
        message_page(&messages, &json!({})),
        Err("page_required".into())
    );
    assert_eq!(
        message_page(&messages, &json!({"limit":null})),
        Err("page_required".into())
    );
    assert_eq!(
        message_page(&messages, &json!({"limit":20})),
        Err("text_window_required".into())
    );
    let page = message_page(&messages, &json!({"limit":20,"chars":400})).unwrap();
    assert_eq!(page.as_array().unwrap().len(), 20);
    assert_eq!(page[0]["text"].as_str().unwrap().chars().count(), 400);
    assert_eq!(
        page[0]["range"],
        json!({"offset":0,"next":400,"total":8000})
    );
    let page = message_page(
        &messages,
        &json!({"after":"message-19","limit":20,"chars":400}),
    )
    .unwrap();
    assert_eq!(page.as_array().unwrap().len(), 10);
    assert_eq!(page[0]["id"], "message-20");
    assert_eq!(
        message_page(&messages, &json!({"after":"removed"})),
        Err("invalid_cursor".into())
    );
    for args in [json!({"limit":0}), json!({"limit":101}), json!({"chars":0})] {
        assert!(message_page(&messages, &args).is_err());
    }
    let small: QueuedMessage =
        serde_json::from_value(json!({"id":"small","text":"entire prompt"})).unwrap();
    let page = message_page(&[small], &json!({})).unwrap();
    assert_eq!(page[0]["text"], "entire prompt");
    assert_eq!(page[0]["range"]["next"], Value::Null);
}

#[tokio::test]
async fn unpaged_document_lists_never_silently_drop_the_last_document() {
    use crate::store::draft_documents::{DraftChange, DraftKind, DraftMutation, DraftResult};
    let root = tempfile::tempdir().unwrap();
    let store = crate::store::Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    for n in 0..51 {
        let id = format!("doc-{n:03}");
        let result = store
            .mutate_draft_document(
                "owner",
                &DraftMutation {
                    operation_id: format!("create-{id}"),
                    document_id: id.clone(),
                    expected_revision: 0,
                    change: DraftChange::Create {
                        kind: DraftKind::Document,
                        title: id,
                        parent_id: None,
                        body: "full body".into(),
                        attachments: vec![],
                    },
                },
            )
            .await
            .unwrap();
        assert!(matches!(result, DraftResult::Applied(_)));
    }
    for query in [json!({}), json!({"limit":null})] {
        assert_eq!(
            document_page(&store, "owner", &query).await,
            Err("page_required".into())
        );
    }
    let first = document_page(&store, "owner", &json!({"limit":50}))
        .await
        .unwrap();
    assert_eq!(first.as_array().unwrap().len(), 50);
    assert!(first[0].get("body").is_none());
    let last = document_page(
        &store,
        "owner",
        &json!({"after":first[49]["id"],"limit":50}),
    )
    .await
    .unwrap();
    assert_eq!(last.as_array().unwrap().len(), 1);
    assert_eq!(last[0]["id"], "doc-050");
    assert_eq!(
        document_page(&store, "other-owner", &json!({}))
            .await
            .unwrap(),
        json!([])
    );
}

fn envelope(seq: u64, event: Event) -> Envelope {
    Envelope {
        session_id: "s".into(),
        seq,
        event,
        cmid: None,
    }
}

fn update(seq: u64, update: Value) -> Envelope {
    envelope(seq, Event::Update { update })
}

#[test]
fn transcript_chunks_coalesce_and_tool_updates_fold_into_their_call() {
    let window = vec![
        update(
            1,
            json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"fix "}}),
        ),
        update(
            2,
            json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"it"}}),
        ),
        update(
            3,
            json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"hidden"}}),
        ),
        update(
            4,
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"On it."}}),
        ),
        update(
            5,
            json!({"sessionUpdate":"tool_call","toolCallId":"t1","title":"Bash","status":"pending","kind":"execute"}),
        ),
        update(
            6,
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Running."}}),
        ),
        update(
            7,
            json!({"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed"}),
        ),
        envelope(
            8,
            Event::TurnEnd {
                usage: None,
                stop_reason: "end_turn".into(),
            },
        ),
    ];
    let query = EventQuery::parse(&json!({})).unwrap();
    let page = query.page(&window, true);
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 5);
    assert_eq!(
        items[0],
        json!({"seq":1,"to":2,"kind":"user","text":"fix it"})
    );
    assert_eq!(items[1]["text"], "On it.");
    assert_eq!(
        items[2],
        json!({"seq":5,"to":7,"kind":"tool","tool":{"id":"t1","title":"Bash","status":"completed","kind":"execute"}})
    );
    assert_eq!(items[4]["kind"], "turn_end");
    assert_eq!(page["more"], false);
    assert_eq!(page["reached_start"], true);

    // The newest items, truncated text, filtered kinds.
    let query = EventQuery::parse(&json!({"limit":1,"chars":3,"kinds":["agent"]})).unwrap();
    let page = query.page(&window, true);
    assert_eq!(
        page["items"],
        json!([{"seq":6,"to":6,"kind":"agent","text":"Run","truncated":true}])
    );
    assert_eq!(page["more"], true);
    assert!(EventQuery::parse(&json!({"kinds":["thought"]})).is_err());
    assert!(EventQuery::parse(&json!({"limit":101})).is_err());
    assert!(EventQuery::parse(&json!({"limit":0})).is_err());
}

#[test]
fn transcript_continuations_do_not_skip_messages_inside_tool_spans() {
    let window = vec![
        update(
            1,
            json!({"sessionUpdate":"tool_call","toolCallId":"t","title":"test"}),
        ),
        update(
            2,
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"between"}}),
        ),
        update(
            3,
            json!({"sessionUpdate":"tool_call_update","toolCallId":"t","status":"completed"}),
        ),
    ];
    let first = EventQuery::parse(&json!({"after":0,"limit":1}))
        .unwrap()
        .page(&window, true);
    assert_eq!(first["items"][0]["to"], 1);
    assert_eq!(first["next"], json!({"after":1}));
    let second = EventQuery::parse(&json!({"after":first["next"]["after"],"limit":1}))
        .unwrap()
        .page(&window[1..], false);
    assert_eq!(second["items"][0]["text"], "between");
    assert_eq!(second["next"], json!({"after":2}));
    let reverse = EventQuery::parse(&json!({"limit":1}))
        .unwrap()
        .page(&window, true);
    assert_eq!(reverse["next"], json!({"before":3}));
    assert_eq!(reverse["items"][0]["tool"]["status"], "completed");
    let earlier = EventQuery::parse(&json!({"before":3,"limit":1}))
        .unwrap()
        .page(&window[..2], true);
    assert_eq!(earlier["items"][0]["text"], "between");
    assert_eq!(earlier["next"], json!({"before":2}));
}

#[test]
fn a_streamed_message_stays_one_group_when_selecting_page_boundaries() {
    let window: Vec<_> = (1..=1000)
        .map(|seq| {
            update(
                seq,
                json!({
                    "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"x"}
                }),
            )
        })
        .collect();
    for query in [json!({"limit":1}), json!({"limit":1,"after":0})] {
        let page = EventQuery::parse(&query).unwrap().page(&window, true);
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(page["items"][0]["to"], 1000);
        assert_eq!(page["more"], false);
    }
}

#[test]
fn filtered_empty_transcript_pages_still_advance() {
    let window: Vec<_> = (1..=EVENT_WINDOW as u64).map(|seq| update(seq,
        json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"hidden"}})
    )).collect();
    let forward = EventQuery::parse(&json!({"after":0,"kinds":["user"]}))
        .unwrap()
        .page(&window, true);
    assert_eq!(forward["items"], json!([]));
    assert_eq!(forward["next"], json!({"after":EVENT_WINDOW}));
    let reverse = EventQuery::parse(&json!({"kinds":["user"]}))
        .unwrap()
        .page(&window, false);
    assert_eq!(reverse["next"], json!({"before":1}));
    let end = EventQuery::parse(&json!({"before":1}))
        .unwrap()
        .page(&[], false);
    assert_eq!(end["next"], Value::Null);
    assert_eq!(end["more"], false);
    assert_eq!(end["reached_start"], false); // retained history is incomplete
}

#[test]
fn console_budget_counts_utf8_bytes() {
    let reply = finish(true, json!({"value":null,"logs":"🙂".repeat(4096)}));
    let logs = reply["logs"].as_str().unwrap();
    assert_eq!(logs.len(), MAX_LOG_BYTES);
    assert_eq!(logs.chars().count(), 1024);
}

struct FakeHost;

impl Host for FakeHost {
    async fn call(&self, op: &str, args: Value, _remaining: Duration) -> Result<Value, String> {
        match op {
            "session.info" => Ok(json!({"id":"parent-1","status":"busy","queued":2})),
            "calls.inspect" => Ok(json!({"call_id": args["call_id"], "state":"running"})),
            "sessions.info" => Err("not_found".into()),
            "machines.list" => Ok(json!({"machines":[{"id":"a"},{"id":"b"}],"next":null})),
            "machines.read" => {
                arg_str(&args, "id")?;
                Ok(json!({"op":op,"args":args}))
            }
            "api.help" => {
                only_keys(&args, &["topic"])?;
                crate::agent_api::help(arg_str(&args, "topic")?).ok_or("invalid_help".into())
            }
            "documents.read" => Ok(json!({"body": "x".repeat(MAX_REPLY_BYTES)})),
            "accounts.transition_status" => {
                only_keys(&args, &["operation"])?;
                Ok(json!({"operation":args["operation"],"stage":"completed",
                    "evidence":"historical","worker_state":"not_observed"}))
            }
            "accounts.logins" | "accounts.login_status" => Ok(json!({"op":op,"args":args})),
            "accounts.login_options" => {
                only_keys(&args, &["account_id", "machine_id"])?;
                Ok(
                    json!({"account_id":args["account_id"],"machine_id":args["machine_id"],"available":false,"reason":"machine_account_protocol_required"}),
                )
            }
            "session.usage" | "session.history" | "sessions.history" | "sessions.usage"
            | "sessions.events" | "sessions.message" | "sessions.queue" | "sessions.drafts"
            | "usage.list" | "usage.read" => Ok(json!({"op":op,"args":args})),
            _ => Err("unknown_operation".into()),
        }
    }
}

#[tokio::test]
async fn object_form_ids_mean_the_same_call_as_positional_ids() {
    let Some(value) = run(
        r#"
        const args = async (promise) => (await promise).args;
        return {
          history: [await args(cowboy.sessions.history("s", {limit:2})),
                    await args(cowboy.sessions.history({session_id:"s", limit:2})),
                    await args(cowboy.sessions.history({id:"s"}, {limit:2}))],
          usage: [await args(cowboy.sessions.usage({session_id:"s"})),
                  await args(cowboy.usage.read({account_id:"a"}))],
          machine: await args(cowboy.machines.read({machine:"m"})),
          call: (await cowboy.calls.inspect({id:"c"})).call_id,
          help: typeof (await cowboy.api.help({topic:"calls"})),
        };
        "#,
        false,
        10_000,
    )
    .await
    else {
        return;
    };
    assert_eq!(value["state"], "ok", "{value}");
    let out = &value["value"];
    let history = json!({"session_id":"s","limit":2});
    assert_eq!(out["history"], json!([history, history, history]));
    assert_eq!(out["usage"], json!([{"id":"s"},{"account":"a"}]));
    assert_eq!(out["machine"], json!({"id":"m"}));
    assert_eq!(out["call"], "c");
    assert_eq!(out["help"], "object");
}

#[tokio::test]
async fn login_discovery_keeps_account_and_machine_independent_in_one_program() {
    let Some(value) = run(
        r#"
        return await Promise.all(["machine-a", "machine-b"].map(async machine => {
            const option = await cowboy.accounts.login_options("account-1", machine);
            return {account: option.account_id, machine: option.machine_id, reason: option.reason};
        }));
        "#,
        true,
        10_000,
    )
    .await
    else {
        return;
    };
    assert_eq!(value["state"], "ok");
    assert_eq!(
        value["value"],
        json!([
            {"account":"account-1","machine":"machine-a","reason":"machine_account_protocol_required"},
            {"account":"account-1","machine":"machine-b","reason":"machine_account_protocol_required"},
        ])
    );
}

#[tokio::test]
async fn account_login_discovery_and_reference_status_share_the_read_program() {
    let Some(value) = run(
        "return [await cowboy.accounts.logins('account-1', {after_machine:'a',limit:2}), await cowboy.accounts.login_status('account-1','request-2')];",
        true, 10_000,
    ).await else { return; };
    assert_eq!(value["state"], "ok");
    assert_eq!(
        value["value"],
        json!([
            {"op":"accounts.logins","args":{"account_id":"account-1","after_machine":"a","limit":2}},
            {"op":"accounts.login_status","args":{"account_id":"account-1","request_id":"request-2"}}
        ])
    );
}

#[tokio::test]
async fn transition_status_program_preserves_historical_evidence_boundary() {
    let Some(value) = run(
        "return await cowboy.accounts.transition_status('move-1');",
        false,
        10_000,
    )
    .await
    else {
        return;
    };
    assert_eq!(value["state"], "ok");
    assert_eq!(
        value["value"],
        json!({"operation":"move-1","stage":"completed",
        "evidence":"historical","worker_state":"not_observed"})
    );
}

async fn run(code: &str, typescript: bool, timeout_ms: u64) -> Option<Value> {
    let Ok(sandbox) = Sandbox::resolve() else {
        eprintln!("skipping: prlimit, bwrap or bun unavailable");
        return None;
    };
    let program = Program {
        code: code.into(),
        typescript,
        module: false,
        timeout_ms,
    };
    Some(sandbox.execute(&program, &FakeHost).await)
}

#[tokio::test]
async fn a_typed_program_composes_calls_and_returns_only_its_result() {
    let Some(value) = run(
        r#"
        const info: { status: string; queued: number } = await cowboy.session.info();
        const call = await cowboy.calls.inspect("call-1");
        let missing: string = "";
        try { await cowboy.sessions.info("other"); } catch (e: any) { missing = e.code; }
        console.log("checked", info.queued);
        return { status: info.status, call: call.state, missing };
        "#,
        true,
        10_000,
    )
    .await
    else {
        return;
    };
    assert_eq!(
        value,
        json!({"schema":1,"state":"ok","value":{"status":"busy","call":"running","missing":"not_found"},
            "logs":"checked 2\n"})
    );
}

#[tokio::test]
async fn scoped_runner_methods_preserve_explicit_ids_over_query_fields() {
    let Some(value) = run(r#"
        return [
          await cowboy.session.usage(),
          await cowboy.sessions.usage('mine'),
          await cowboy.sessions.events('mine', {session_id:'foreign', after:7}),
          await cowboy.sessions.message('mine', 'message', {session_id:'foreign', id:'other', chars:8}),
          await cowboy.sessions.queue('mine', {limit:1}),
          await cowboy.sessions.drafts('mine', {chars:5}),
          await cowboy.usage.list({limit:1}),
          await cowboy.usage.read('account', {account:'other', section:'quota'}),
          await cowboy.sessions.history('mine', {session_id:'foreign',before:50}),
          await cowboy.session.history({before:30})
        ];
    "#, false, 10_000).await else { return; };
    assert_eq!(value["state"], "ok");
    let rows = value["value"].as_array().unwrap();
    assert_eq!(rows[0]["args"], json!({}));
    assert_eq!(rows[1]["args"], json!({"id":"mine"}));
    assert_eq!(rows[2]["args"], json!({"session_id":"mine","after":7}));
    assert_eq!(
        rows[3]["args"],
        json!({"session_id":"mine","id":"message","chars":8})
    );
    assert_eq!(rows[4]["args"], json!({"session_id":"mine","limit":1}));
    assert_eq!(rows[5]["args"], json!({"session_id":"mine","chars":5}));
    assert_eq!(rows[6]["args"], json!({"limit":1}));
    assert_eq!(
        rows[7]["args"],
        json!({"account":"account","section":"quota"})
    );
    assert_eq!(rows[8]["args"], json!({"session_id":"mine","before":50}));
    assert_eq!(rows[9]["args"], json!({"before":30}));
}

#[tokio::test]
async fn the_runtime_has_no_network_files_or_unbounded_time() {
    let Some(value) = run(
        r#"
        const out = {};
        try { await fetch("http://1.1.1.1", { signal: AbortSignal.timeout(500) }); out.net = "open"; }
        catch { out.net = "closed"; }
        try { require("fs").readFileSync("/etc/passwd"); out.files = "open"; }
        catch { out.files = "closed"; }
        out.env = Object.keys(process.env).filter((k) => !["HOME", "PWD"].includes(k) && !k.startsWith("BUN_")).length;
        return out;
        "#,
        false,
        10_000,
    )
    .await
    else {
        return;
    };
    assert_eq!(
        value["value"],
        json!({"net":"closed","files":"closed","env":0})
    );
    let value = run("while (true) {}", false, 1_000).await.unwrap();
    assert_eq!(value["error"]["code"], "timeout");
}

#[tokio::test]
async fn budgets_and_failures_are_explicit() {
    let Some(value) = run("return 'x'.repeat(20000);", false, 10_000).await else {
        return;
    };
    assert_eq!(value["error"]["code"], "result_too_large");
    // A result just under the limit arrives whole: the final frame is flushed.
    let value = run("return 'x'.repeat(16000);", false, 10_000)
        .await
        .unwrap();
    assert_eq!(value["value"].as_str().map(str::len), Some(16000));
    let value = run(
        "console.log('a' + '🙂'.repeat(4096)); console.log('x'); return 1;",
        false,
        10_000,
    )
    .await
    .unwrap();
    assert_eq!(value["value"], 1);
    let logs = value["logs"].as_str().unwrap();
    assert!(logs.len() <= MAX_LOG_BYTES);
    assert!(!logs.contains('\u{fffd}'));
    assert!(logs.starts_with("a🙂"));
    let value = run("return (", false, 10_000).await.unwrap();
    assert_eq!(value["error"]["code"], "compile_error");
    let value = run(
        "for (let i = 0; i < 17; i++) { try { await cowboy.session.info(); } catch (e) { return [i, e.code]; } } return 'unbounded';",
        false,
        10_000,
    )
    .await
    .unwrap();
    assert_eq!(value["value"], json!([16, "call_limit"]));
    let value = run(
        "try { await cowboy.documents.read('d'); } catch (e) { return e.code; }",
        false,
        10_000,
    )
    .await
    .unwrap();
    assert_eq!(value["value"], "reply_too_large");
    let value = run(
        "throw Object.assign(new Error('boom'), { code: 'mine' });",
        false,
        10_000,
    )
    .await
    .unwrap();
    assert_eq!(value["error"], json!({"code":"mine","message":"boom"}));
    let value = run(
        "throw Object.assign(new Error('x'.repeat(100000)), { code: 'c'.repeat(100000), extra: 'y'.repeat(100000) });",
        false,
        10_000,
    )
    .await
    .unwrap();
    assert_eq!(value["error"]["code"], "program_error");
    assert!(value.to_string().len() < 4096);
    // Scratch space is bounded host memory, not an unlimited tmpfs.
    let value = run(
        r#"const fs = require("fs"); const out = {};
        for (const dir of ["/tmp", "/dev"]) {
          try { for (let i = 0; i < 4; i++) fs.writeFileSync(`${dir}/f${i}`, Buffer.alloc(600000)); out[dir] = "unbounded"; }
          catch (e) { out[dir] = e.code; }
        }
        return out;"#,
        false,
        10_000,
    )
    .await
    .unwrap();
    assert_eq!(value["value"], json!({"/tmp":"ENOSPC","/dev":"EROFS"}));
}

#[tokio::test]
async fn failed_calls_name_the_operation_and_what_was_received() {
    let Some(value) = run(
        r#"
        const out = {};
        try { await cowboy.machines.read(42); }
        catch (e) { out.argument = [e.code, e.message]; }
        try { await cowboy.api.help({topic:"calls", cursor:"x"}); }
        catch (e) { out.option = e.message; }
        try { await cowboy.sessions.get("s"); } catch (e) { out.method = [e.code, e.message]; }
        try { await cowboy.help("calls"); } catch (e) { out.root = [e.code, e.message]; }
        out.passthrough = [typeof cowboy.then, typeof cowboy.sessions.toJSON];
        return out;
        "#,
        false,
        10_000,
    )
    .await
    else {
        return;
    };
    let out = &value["value"];
    assert_eq!(out["argument"][0], "invalid_argument", "{value}");
    let message = out["argument"][1].as_str().unwrap();
    assert!(
        message.starts_with("machines.read: invalid_argument: id must be an id string; received {")
            && message.contains(r#"{"id":42}"#)
            && message.contains(r#"cowboy_read({help:"machines"})"#),
        "{message}"
    );
    let option = out["option"].as_str().unwrap();
    assert!(
        option.contains("cursor is not accepted; options are topic"),
        "{option}"
    );
    assert_eq!(out["method"][0], "unknown_operation");
    let message = out["method"][1].as_str().unwrap();
    assert!(message.starts_with("cowboy.sessions.get does not exist;") && message.contains("info"));
    assert_eq!(out["root"][0], "unknown_operation");
    assert!(
        out["root"][1]
            .as_str()
            .unwrap()
            .contains("cowboy.api.help(topic)")
    );
    assert_eq!(out["passthrough"], json!(["undefined", "undefined"]));
}

#[test]
fn operation_errors_keep_codes_and_split_details() {
    let value = call_error(
        "control.read",
        "{}",
        "http_400: unknown field `window`".into(),
    );
    assert_eq!(value["code"], "http_400");
    assert_eq!(
        value["message"],
        "control.read: http_400: unknown field `window`"
    );
    let value = call_error("api.help", r#"{"topic":"plugins"}"#, "invalid_help".into());
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .contains("topics: session, sessions")
    );
    let value = call_error("control.read", "{}", "selection_too_large".into());
    assert!(value["message"].as_str().unwrap().contains("describe:true"));
    let long = format!("{{\"x\":\"{}\"}}", "é".repeat(1000));
    let value = call_error("sessions.info", &long, "invalid_argument".into());
    assert!(value["message"].as_str().unwrap().chars().count() < 420);
}

#[tokio::test]
async fn paged_lists_iterate_like_arrays_without_changing_their_json() {
    let Some(value) = run(
        r#"
        const list = await cowboy.machines.list();
        return { ids: list.map((m) => m.id), spread: [...list].length, length: list.length,
          found: list.find((m) => m.id === "b")?.id, json: JSON.parse(JSON.stringify(list)) };
        "#,
        false,
        10_000,
    )
    .await
    else {
        return;
    };
    assert_eq!(
        value["value"],
        json!({"ids":["a","b"],"spread":2,"length":2,"found":"b",
            "json":{"machines":[{"id":"a"},{"id":"b"}],"next":null}}),
        "{value}"
    );
}

#[test]
fn oversized_results_name_their_largest_fields() {
    let result = json!({"small":1,"providers":"x".repeat(900),"quota":"y".repeat(300),"x":[1]});
    let largest = largest_fields(&result);
    assert_eq!(largest[0]["field"], "providers");
    assert_eq!(largest[1]["field"], "quota");
    assert_eq!(largest.as_array().unwrap().len(), 3);
    assert_eq!(largest_fields(&json!([1, "zz"]))[0]["field"], "[1]");
    assert_eq!(largest_fields(&json!("text")), json!([]));
}
