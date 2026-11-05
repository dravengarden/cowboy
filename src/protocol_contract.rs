//! Cross-language wire discriminant contract.

use std::collections::BTreeSet;

fn snake_case(name: &str) -> String {
    let mut output = String::new();
    for (index, character) in name.chars().enumerate() {
        if character.is_uppercase() {
            if index > 0 {
                output.push('_');
            }
            output.extend(character.to_lowercase());
        } else {
            output.push(character);
        }
    }
    output
}

fn rust_tags(enum_name: &str) -> BTreeSet<String> {
    let file = syn::parse_file(include_str!("core.rs")).expect("core.rs parses");
    file.items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(item) if item.ident == enum_name => Some(
                item.variants
                    .iter()
                    .map(|variant| snake_case(&variant.ident.to_string()))
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing Rust enum {enum_name}"))
}

fn typescript_tags(type_name: &str, end_marker: &str) -> BTreeSet<String> {
    let source = include_str!("../web/src/protocol.ts");
    let start = format!("export type {type_name} =");
    let section = source
        .split_once(&start)
        .unwrap_or_else(|| panic!("missing TypeScript union {type_name}"))
        .1
        .split_once(end_marker)
        .unwrap_or_else(|| panic!("missing end marker for TypeScript union {type_name}"))
        .0;
    section
        .match_indices("type: \"")
        .filter_map(|(index, marker)| {
            let rest = &section[index + marker.len()..];
            rest.split_once('"').map(|(tag, _)| tag.to_owned())
        })
        .collect()
}

#[test]
fn inbound_tags_match_typescript() {
    assert_eq!(
        rust_tags("Inbound"),
        typescript_tags("Inbound", "// End of Inbound wire union.")
    );
}

#[test]
fn outbound_tags_match_typescript() {
    assert_eq!(
        rust_tags("Outbound"),
        typescript_tags("Outbound", "export interface SessionBootstrapResponse")
    );
}

#[test]
fn event_field_serialization_matches_typescript_fixtures() {
    use crate::core::{Event, Status};

    let events = vec![
        Event::Update {
            update: serde_json::json!({
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "hello"}
            }),
        },
        Event::PermissionRequest {
            request_id: "p1".to_owned(),
            tool_call: serde_json::json!({"title": "Shell"}),
            options: serde_json::json!([]),
        },
        Event::PermissionResolved {
            request_id: "p1".to_owned(),
            option_id: None,
        },
        Event::Lifecycle {
            status: Status::Running,
            detail: None,
        },
        Event::TurnEnd {
            usage: None,
            stop_reason: "end_turn".to_owned(),
        },
    ];
    let values = events
        .into_iter()
        .map(|event| serde_json::to_value(event).expect("serialize event"))
        .collect::<Vec<_>>();
    assert_eq!(values[1]["request_id"], "p1");
    assert!(values[2]["option_id"].is_null());
    assert_eq!(values[3]["status"], "running");
    assert_eq!(values[4]["stop_reason"], "end_turn");
}

#[test]
fn session_meta_owner_fields_are_optional_on_the_wire() {
    let mut meta = sample_session_meta();
    let absent = serde_json::to_value(&meta).expect("serialize unowned session");
    assert!(absent.get("owner_user_id").is_none());
    assert!(absent.get("owner_username").is_none());

    meta.owner_user_id = Some("0123456789abcdef0123456789abcdef".to_owned());
    meta.owner_username = Some("draven".to_owned());
    let present = serde_json::to_value(&meta).expect("serialize owned session");
    assert_eq!(present["owner_user_id"], "0123456789abcdef0123456789abcdef");
    assert_eq!(present["owner_username"], "draven");

    let source = include_str!("../web/src/protocol.ts");
    assert!(source.contains("owner_user_id?: string;"));
    assert!(source.contains("owner_username?: string;"));
}

/// An unowned session as the Controller lists it.
fn sample_session_meta() -> crate::core::SessionMeta {
    use crate::core::{SessionMeta, SessionOrigin, Status};

    SessionMeta {
        continued_from: None,
        forked_from: None,
        id: "sess-1".to_owned(),
        provider: "codex".to_owned(),
        provider_version: String::new(),
        provider_generation_digest: String::new(),
        provider_account_id: crate::provider_account::ProviderAccountId::default(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: "local".to_owned(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        execution_binding: None,
        cwd: "/tmp".to_owned(),
        title: "owner stamp".to_owned(),
        status: Status::Starting,
        origin: SessionOrigin::Web,
        agent_session_id: None,
        paused: false,
        closing: false,
        system: false,
        context_used: 0,
        context_size: 0,
        usage: None,
        background_tasks: 0,
        supports_current_turn: false,
        provider_update: None,
        provider_update_available: None,
        resource_wait: None,
        next_schedule_ms: None,
        owner_user_id: None,
        owner_username: None,
        trashed_at_ms: None,
    }
}

/// Rust-serialized wire fixtures for the web's hand-written protocol types.
/// `tools/generate-protocol-fixtures.ts` turns the checked-in JSON into a
/// module whose literals must satisfy `web/src/protocol.ts`, so the web
/// typecheck rejects a field, tag or nullability the TypeScript mirror lacks.
/// Every variant appears with each optional field present, absent and null.
/// Regenerate with `COWBOY_UPDATE_CONTRACTS=1 cargo test protocol_fixtures`.
fn event_fixtures() -> Vec<(&'static str, crate::core::Event)> {
    use crate::core::{Event, Status};
    let mut fixtures = vec![
        (
            "update",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "hello"}
                }),
            },
        ),
        (
            "permission_request",
            Event::PermissionRequest {
                request_id: "p1".to_owned(),
                tool_call: serde_json::json!({"title": "Shell"}),
                options: serde_json::json!([
                    {"optionId": "allow", "name": "Allow", "kind": "allow_once"}
                ]),
            },
        ),
        (
            "permission_resolved_selected",
            Event::PermissionResolved {
                request_id: "p1".to_owned(),
                option_id: Some("allow".to_owned()),
            },
        ),
        (
            "permission_resolved_cancelled",
            Event::PermissionResolved {
                request_id: "p1".to_owned(),
                option_id: None,
            },
        ),
        (
            "turn_end_without_usage",
            Event::TurnEnd {
                stop_reason: "end_turn".to_owned(),
                usage: None,
            },
        ),
        (
            "turn_end_with_usage",
            Event::TurnEnd {
                stop_reason: "end_turn".to_owned(),
                usage: Some(crate::agent_model::TurnUsage {
                    total_tokens: 1200,
                    input_tokens: 1000,
                    output_tokens: 200,
                    cached_input_tokens: Some(900),
                    uncached_input_tokens: Some(100),
                    cache_write_tokens: None,
                    reasoning_tokens: Some(50),
                }),
            },
        ),
    ];
    for (name, status) in [
        ("lifecycle_starting", Status::Starting),
        ("lifecycle_running", Status::Running),
        ("lifecycle_busy", Status::Busy),
        ("lifecycle_exited", Status::Exited),
        ("lifecycle_interrupted", Status::Interrupted),
    ] {
        fixtures.push((
            name,
            Event::Lifecycle {
                status,
                detail: None,
            },
        ));
    }
    fixtures.push((
        "lifecycle_crashed_with_detail",
        Event::Lifecycle {
            status: Status::Crashed,
            detail: Some("worker exited".to_owned()),
        },
    ));
    fixtures
}

/// Every Outbound variant as the Controller serializes it, with optional
/// fields present and absent.
fn outbound_fixtures() -> Vec<(&'static str, crate::core::Outbound)> {
    let mut fixtures = outbound_session_fixtures();
    fixtures.extend(outbound_control_fixtures());
    fixtures
}

/// Session list, bootstrap and transcript frames.
fn outbound_session_fixtures() -> Vec<(&'static str, crate::core::Outbound)> {
    use crate::core::{Envelope, Event, Outbound, Status};
    let envelope = |seq, event, cmid: Option<&str>| Envelope {
        session_id: "sess-1".to_owned(),
        seq,
        event,
        cmid: cmid.map(str::to_owned),
    };
    let mut owned = sample_session_meta();
    owned.owner_user_id = Some("0123456789abcdef0123456789abcdef".to_owned());
    owned.owner_username = Some("draven".to_owned());
    vec![
        (
            "auth_session",
            Outbound::AuthSession {
                session: serde_json::json!({"authenticated": true}),
            },
        ),
        (
            "client_capacity",
            Outbound::ClientCapacity {
                capacity: serde_json::json!({
                    "status": "waiting",
                    "position": 2,
                    "retry_after_ms": 5000,
                    "channel_limit": 4
                }),
            },
        ),
        (
            "sessions",
            Outbound::Sessions {
                sessions: vec![sample_session_meta(), owned],
            },
        ),
        (
            "machines",
            Outbound::Machines {
                revision: 7,
                machines: Vec::new(),
                resync: false,
            },
        ),
        ("bootstrap_complete", Outbound::BootstrapComplete),
        (
            "snapshot",
            Outbound::Snapshot {
                session_id: "sess-1".to_owned(),
                events: vec![envelope(
                    1,
                    Event::Lifecycle {
                        status: Status::Running,
                        detail: None,
                    },
                    None,
                )],
                reached_start: true,
            },
        ),
        (
            "event_with_cmid",
            Outbound::Event {
                envelope: envelope(
                    2,
                    Event::Update {
                        update: serde_json::json!({
                            "sessionUpdate": "user_message_chunk",
                            "content": {"type": "text", "text": "hi"}
                        }),
                    },
                    Some("cmid-1"),
                ),
            },
        ),
        (
            "event_without_cmid",
            Outbound::Event {
                envelope: envelope(
                    3,
                    Event::TurnEnd {
                        stop_reason: "end_turn".to_owned(),
                        usage: None,
                    },
                    None,
                ),
            },
        ),
    ]
}

/// Connection, sync, settings, usage and error frames.
fn outbound_control_fixtures() -> Vec<(&'static str, crate::core::Outbound)> {
    use crate::core::{CommandOutcome, Outbound};
    vec![
        ("ping", Outbound::Ping),
        ("connection_probe", Outbound::ConnectionProbe { nonce: 42 }),
        (
            "config_options",
            Outbound::ConfigOptions {
                session_id: "sess-1".to_owned(),
                options: serde_json::json!([]),
            },
        ),
        (
            "sync_patch",
            Outbound::SyncPatch {
                state: "queue:sess-1".to_owned(),
                version: 3,
                value: serde_json::json!({"queue": [], "drafts": []}),
                confirmed: vec!["cmid-1".to_owned()],
                resync: false,
            },
        ),
        (
            "settings",
            Outbound::Settings {
                settings: std::collections::HashMap::from([(
                    "theme".to_owned(),
                    serde_json::json!("dark"),
                )]),
            },
        ),
        (
            "usage",
            Outbound::Usage {
                snapshot: serde_json::json!({
                    "refreshed_at_ms": 1,
                    "next_refresh_at_ms": 2,
                    "refresh_interval_ms": 60000,
                    "providers": []
                }),
            },
        ),
        (
            "account_usage_changed",
            Outbound::AccountUsageChanged {
                owner_user_id: "0123456789abcdef0123456789abcdef".to_owned(),
            },
        ),
        (
            "call_approval_pending",
            Outbound::CallApproval {
                session_id: "sess-1".to_owned(),
                approval: Some(serde_json::json!({
                    "schema": 1,
                    "caller": "sess-1",
                    "reason": "policy_denied",
                    "requests": 1,
                    "agents": ["codex"],
                    "items": [{"agent": "codex", "purpose": "review", "summary": "Review the diff"}],
                    "ttl_ms": 60000
                })),
            },
        ),
        (
            "call_approval_cleared",
            Outbound::CallApproval {
                session_id: "sess-1".to_owned(),
                approval: None,
            },
        ),
        (
            "error_for_session",
            Outbound::Error {
                session_id: Some("sess-1".to_owned()),
                message: "command failed".to_owned(),
            },
        ),
        (
            "error_global",
            Outbound::Error {
                session_id: None,
                message: "bad inbound command".to_owned(),
            },
        ),
        (
            "command_result",
            Outbound::CommandResult {
                session_id: "sess-1".to_owned(),
                cmid: "cmid-1".to_owned(),
                outcome: CommandOutcome::Stale,
                message: "row already left the queue".to_owned(),
            },
        ),
    ]
}

fn check_contract(file: &str, fixtures: Vec<(&'static str, serde_json::Value)>) {
    let fixtures: Vec<serde_json::Value> = fixtures
        .into_iter()
        .map(|(name, value)| serde_json::json!({"name": name, "value": value}))
        .collect();
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(&fixtures).expect("render fixtures")
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("contracts/protocol")
        .join(file);
    if std::env::var_os("COWBOY_UPDATE_CONTRACTS").is_some() {
        std::fs::create_dir_all(path.parent().expect("contract directory"))
            .expect("create contract directory");
        std::fs::write(&path, &rendered).expect("write contract");
        return;
    }
    let checked_in = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        checked_in == rendered,
        "{} is out of date: run COWBOY_UPDATE_CONTRACTS=1 cargo test protocol_fixtures, then bun tools/generate-protocol-fixtures.ts",
        path.display()
    );
}

fn serialized<T: serde::Serialize>(
    fixtures: Vec<(&'static str, T)>,
) -> Vec<(&'static str, serde_json::Value)> {
    fixtures
        .into_iter()
        .map(|(name, value)| {
            (
                name,
                serde_json::to_value(value).expect("serialize fixture"),
            )
        })
        .collect()
}

#[test]
fn protocol_fixtures_match_the_checked_in_contract() {
    check_contract("event.json", serialized(event_fixtures()));
    check_contract("outbound.json", serialized(outbound_fixtures()));
}
