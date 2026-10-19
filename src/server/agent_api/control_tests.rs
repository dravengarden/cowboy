use super::*;
use std::collections::BTreeSet;

#[tokio::test]
async fn slow_admitted_writer_survives_the_short_submission_reply() {
    let (release, blocked) = tokio::sync::oneshot::channel::<()>();
    let (written, completed) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        blocked.await.unwrap();
        written.send(true).unwrap();
        Ok(json!({"status":"accepted"}))
    });
    let pending = json!({"status":"pending","outcome":"unknown","domain_operation":"original"});
    assert_eq!(
        submission_reply(task, pending.clone(), Duration::from_millis(1))
            .await
            .unwrap(),
        pending
    );
    release.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), completed)
            .await
            .unwrap()
            .unwrap()
    );
}

#[test]
fn product_api_and_commands_must_be_covered_or_have_an_owned_exception() {
    let coverage: Value =
        serde_json::from_str(include_str!("../../../contracts/agent-api-coverage.json")).unwrap();
    let server = include_str!("../../server.rs")
        .split("    let app = Router::new()")
        .nth(1)
        .unwrap()
        .split("    let listener =")
        .next()
        .unwrap();
    let pattern =
        regex::Regex::new(r"\b(?:get|post|put|patch|delete|any)\(\s*([a-z_][a-zA-Z0-9_:]*)\s*\)")
            .unwrap();
    let handlers: BTreeSet<_> = pattern
        .captures_iter(server)
        .map(|m| m[1].to_owned())
        .collect();
    let core = include_str!("../../core.rs")
        .split("pub enum Inbound {")
        .nth(1)
        .unwrap()
        .split("pub enum Outbound")
        .next()
        .unwrap();
    let pattern = regex::Regex::new(r"(?m)^    ([A-Z][a-zA-Z0-9_]+)(?:\s*[\{,])").unwrap();
    let commands: BTreeSet<_> = pattern
        .captures_iter(core)
        .map(|m| m[1].to_owned())
        .collect();
    let pattern = regex::Regex::new(r"\.merge\(([a-z_:]+)\(").unwrap();
    let merges: BTreeSet<_> = pattern
        .captures_iter(server)
        .map(|m| m[1].to_owned())
        .collect();
    for (surface, actual) in [
        ("http", handlers),
        ("websocket", commands),
        ("merged_routers", merges),
    ] {
        let entries = coverage[surface].as_object().unwrap();
        assert_eq!(
            actual,
            entries.keys().cloned().collect(),
            "unclassified or stale {surface} capability"
        );
        for (name, target) in entries {
            let target = target.as_str().unwrap();
            assert!(
                catalog::find(target).is_some()
                    || crate::agent_api::READ_OPERATIONS.contains(&target)
                    || crate::agent_api::INVOKE_OPERATIONS.contains(&target)
                    || (target.starts_with("exception:") && target.len() > 50),
                "{surface}/{name}: missing MCP operation or concrete owner exception"
            );
        }
    }
}

#[test]
fn adapter_paths_retain_the_product_route_and_permission_policy() {
    let server = include_str!("../../server.rs");
    let parameters = regex::Regex::new(r"\{[a-z_]+\}").unwrap();
    let root = parameters.replace_all(server, "{id}");
    let principal = |role| crate::product_auth::ProductPrincipal {
        user_id: "owner".into(),
        username: "owner".into(),
        role,
    };
    for op in catalog::OPERATIONS {
        let Some((method, path, _)) = route(op.name) else {
            continue;
        };
        let normalized = parameters.replace_all(path, "{id}");
        assert!(
            root.contains(&format!("\"{normalized}\"")),
            "unregistered path {path}"
        );
        let path = parameters.replace_all(path, "target");
        assert!(
            route_permission(
                op.name,
                &principal(AdminRole::Owner),
                &method.parse().unwrap(),
                &path
            )
            .is_ok(),
            "owner can't call {}",
            op.name
        );
    }
    for (name, path) in [
        ("plugins.operations", "/api/machines/m/plugins/p/operations"),
        (
            "plugins.history",
            "/api/machines/m/plugins/p/lifecycle-history",
        ),
        ("machines.durable_state", "/api/machines/m/durable-state"),
        ("diagnostics.log", "/api/logs/log"),
    ] {
        assert!(route_permission(name, &principal(AdminRole::Viewer), &Method::GET, path).is_err());
        assert!(
            route_permission(name, &principal(AdminRole::Operator), &Method::GET, path).is_ok()
        );
    }
    assert!(
        route_permission(
            "tools.defaults_set",
            &principal(AdminRole::Operator),
            &Method::PUT,
            "/api/agent-tools/codex"
        )
        .is_err()
    );
    assert!(
        route_permission(
            "arbitrary",
            &principal(AdminRole::Owner),
            &Method::POST,
            "/api/admin/accounts"
        )
        .is_err()
    );
}

fn fixture(hub: &crate::core::Hub, id: &str, owner: Option<&str>, system: bool) -> SessionMeta {
    hub.create_session(crate::core::SessionRegistration {
        id: id.into(),
        provider: "codex".into(),
        provider_account_id: Default::default(),
        provider_version: "1".into(),
        provider_generation_digest: "generation".into(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: "machine".into(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        execution_binding: None,
        cwd: "/worktree".into(),
        title: id.into(),
        origin: crate::core::SessionOrigin::Web,
        system,
        owner_user_id: owner.map(str::to_owned),
        owner_username: None,
    });
    hub.session_info(id).unwrap().meta
}

#[test]
fn sync_and_direct_commands_cannot_target_foreign_system_or_missing_sessions() {
    let hub = crate::core::Hub::new();
    let parent = fixture(&hub, "parent", Some("owner"), false);
    fixture(&hub, "own", Some("owner"), false);
    fixture(&hub, "foreign", Some("else"), false);
    fixture(&hub, "unowned", None, false);
    fixture(&hub, "system", Some("owner"), true);
    for id in ["own", "foreign", "unowned", "system", "missing"] {
        for (name, input) in [
            ("sessions.rename", json!({"session_id":id,"title":"x"})),
            (
                "drafts.move",
                json!({"session_id":"parent","id":"draft","to_session":id}),
            ),
            (
                "sync.apply",
                json!({"state":"title","name":"rename","args":{"session_id":id,"title":"x"}}),
            ),
            (
                "sync.apply",
                json!({"state":"order","name":"reorder","args":{"order":[id]}}),
            ),
            (
                "sync.apply",
                json!({"state":"folders","name":"place","args":{"session_ids":[id],"folder":null}}),
            ),
            (
                "sync.apply",
                json!({"state":format!("mobile-review:{id}"),"name":"setDirectory","args":{"directory":null}}),
            ),
        ] {
            assert_eq!(
                scoped_targets(&hub, &parent, name, &input).is_ok(),
                id == "own",
                "{name}/{id}"
            );
        }
    }
    assert!(parent_current(&hub, &parent));
    hub.set_agent_session_id("parent", "replacement-native".into());
    assert!(!parent_current(&hub, &parent));
}

#[test]
fn commands_preserve_real_payloads_and_reject_unsupported_fields() {
    let inputs = [
        (
            "sessions.configure",
            json!({"session_id":"s","config_id":"model","value":"model-v2"}),
        ),
        (
            "queue.submit",
            json!({"session_id":"s","text":"queued","front":true,"force":false}),
        ),
        (
            "queue.reorder",
            json!({"session_id":"s","order":["two","one"]}),
        ),
        (
            "drafts.move",
            json!({"session_id":"s","id":"draft","to_session":"other"}),
        ),
        (
            "drafts.schedule",
            json!({"session_id":"s","text":"scheduled","fire_at_ms":1792310000000u64,"delivery":"front"}),
        ),
        (
            "drafts.edit",
            json!({"session_id":"s","id":"draft","text":"edited","content":[{"type":"image","data":"AA==","mimeType":"image/png"}]}),
        ),
        (
            "sync.apply",
            json!({"state":"folders","name":"rename","args":{"id":"folder","name":"new"}}),
        ),
    ];
    for (name, input) in inputs {
        let value = command_value(name, &input, "stable").unwrap();
        let _typed: crate::core::Inbound = serde_json::from_value(value.clone()).unwrap();
        for (key, data) in input.as_object().unwrap() {
            assert_eq!(&value[key], data);
        }
        assert_eq!(
            value[if name == "sync.apply" { "id" } else { "cmid" }],
            "stable"
        );
        let mut injected = input;
        injected["owner_user_id"] = json!("foreign");
        assert!(command_value(name, &injected, "stable").is_err());
    }
    for id in [".", "..", "a/b", "a?b", "a%2fb", "a#b", "a\\b"] {
        assert!(uri("/api/logs/{id}", &json!({"id":id})).is_err());
    }
    assert_eq!(
        uri("/api/logs/{id}", &json!({"id":"machine:log-1"})).unwrap(),
        "/api/logs/machine:log-1"
    );
}

#[test]
fn explicit_send_intent_preserves_queue_default_and_rejects_contradictions() {
    for (delivery, force) in [("next_turn", false), ("interrupt", true)] {
        let value = command_value(
            "queue.submit",
            &json!({"session_id":"s","text":"message","delivery":delivery}),
            "stable",
        )
        .unwrap();
        assert_eq!(value["force"], force);
        assert_eq!(value["cmid"], "stable");
    }
    for input in [
        json!({"session_id":"s","text":"message","delivery":"next_turn","force":true}),
        json!({"session_id":"s","text":"message","delivery":"current_turn"}),
        json!({"session_id":"s","text":"message","delivery":"next_tool_call"}),
    ] {
        assert!(command_value("queue.submit", &input, "stable").is_err());
    }
}

#[test]
fn discovery_and_receipts_do_not_dump_the_whole_catalog_or_result() {
    let index = catalog::list(None);
    assert!(index.get("operations").is_none());
    assert!(serde_json::to_vec(&index).unwrap().len() < 1600);
    let domain = catalog::list(Some("usage"));
    assert!(domain.get("exceptions").is_none());
    assert!(
        domain["operations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|op| op["name"].as_str().unwrap().starts_with("usage."))
    );
    let large = json!({"status":"accepted","result":{"text":"中".repeat(20000)}});
    let summary = receipt_view(large.clone());
    assert_eq!(summary["partial"], true);
    assert!(serde_json::to_vec(&summary).unwrap().len() < 512);
    let page = json_reads::select(
        &large,
        &json!({"path":"/result/text","offset":10,"chars":20}),
    )
    .unwrap();
    assert_eq!(page["data"], "中".repeat(20));
    assert_eq!(page["range"]["next"], 30);
}
