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
        typescript_tags("Outbound", "export interface SkillView")
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
