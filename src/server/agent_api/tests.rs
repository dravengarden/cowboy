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
}

struct FakeHost;

impl Host for FakeHost {
    async fn call(&self, op: &str, args: Value, _remaining: Duration) -> Result<Value, String> {
        match op {
            "session.info" => Ok(json!({"id":"parent-1","status":"busy","queued":2})),
            "calls.inspect" => Ok(json!({"call_id": args["call_id"], "state":"running"})),
            "sessions.info" => Err("not_found".into()),
            "documents.read" => Ok(json!({"body": "x".repeat(MAX_REPLY_BYTES)})),
            _ => Err("unknown_operation".into()),
        }
    }
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
