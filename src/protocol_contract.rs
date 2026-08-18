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
    use crate::core::{SessionMeta, SessionOrigin, Status};

    let mut meta = SessionMeta {
        id: "sess-1".to_owned(),
        provider: "codex".to_owned(),
        provider_version: String::new(),
        provider_generation_digest: String::new(),
        provider_auth_generation: None,
        provider_behavior: None,
        machine_id: "local".to_owned(),
        workspace_id: None,
        workspace_name: None,
        workspace_source_path: None,
        cwd: "/tmp".to_owned(),
        title: "owner stamp".to_owned(),
        status: Status::Starting,
        origin: SessionOrigin::Web,
        agent_session_id: None,
        auto_resume: None,
        awaiting_user: false,
        done: false,
        judging: false,
        paused: false,
        system: false,
        context_used: 0,
        context_size: 0,
        usage: None,
        next_schedule_ms: None,
        owner_user_id: None,
        owner_username: None,
    };
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
