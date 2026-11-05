//! `cowboy.code.diagnostics`: the language engine's last observation of
//! buffers a Code pane already holds. It acquires nothing, so a file no one has
//! open answers `not_open`; it never claims a clean file. Diagnostics may lag
//! the newest edit: freshness is `last_observed`, not verified analysis.

use serde_json::{Value, json};

use super::super::super::code_reads::language::observe_open_buffer;
use super::super::super::{AppState, ZedAdapterResponse};
use crate::machine_control::SessionReadScope;

pub(super) const MAX_PATHS: usize = 20;
const MAX_PER_PATH: usize = 50;
const MAX_MESSAGE_CHARS: usize = 500;

fn severity(value: i32) -> &'static str {
    match value {
        1 => "error",
        2 => "warning",
        3 => "information",
        4 => "hint",
        _ => "unknown",
    }
}

pub(super) async fn read(
    state: &AppState,
    observed: &SessionReadScope,
    paths: Vec<String>,
) -> Value {
    let allowed = state
        .hub
        .language_servers_allowed(observed.session().session_id());
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        if !allowed {
            files.push(json!({"path":path,"state":"restricted"}));
            continue;
        }
        files.push(match observe_open_buffer(state, observed, &path).await {
            Ok(ZedAdapterResponse::BufferLanguage { diagnostics, .. }) => {
                let mut counts = json!({"error":0,"warning":0,"information":0,"hint":0});
                for diagnostic in &diagnostics {
                    if let Some(count) = counts.get_mut(severity(diagnostic.severity)) {
                        *count = json!(count.as_u64().unwrap_or(0) + 1);
                    }
                }
                let truncated = diagnostics.len() > MAX_PER_PATH;
                let mut listed: Vec<_> = diagnostics.iter().collect();
                listed.sort_by_key(|diagnostic| (diagnostic.severity, diagnostic.start.row));
                let listed: Vec<Value> = listed
                    .into_iter()
                    .take(MAX_PER_PATH)
                    .map(|diagnostic| {
                        json!({
                            "severity": severity(diagnostic.severity),
                            "line": diagnostic.start.row + 1,
                            "column": diagnostic.start.column + 1,
                            "endLine": diagnostic.end.row + 1,
                            "source": diagnostic.source,
                            "message": diagnostic.message.chars().take(MAX_MESSAGE_CHARS).collect::<String>(),
                        })
                    })
                    .collect();
                json!({"path":path,"state":"last_observed","counts":counts,
                    "diagnostics":listed,"truncated":truncated})
            }
            Ok(_) => json!({"path":path,"state":"unavailable"}),
            Err(error) if error.to_string().contains("not open") => {
                json!({"path":path,"state":"not_open"})
            }
            Err(error) if error.to_string().contains("outside this session") => {
                json!({"path":path,"state":"refused"})
            }
            Err(_) => json!({"path":path,"state":"unavailable"}),
        });
    }
    json!({
        "freshness": "last_observed",
        "coverage": "open_buffers",
        "files": files,
    })
}
