//! `cowboy.code.diagnostics`: the language engine's last observation of
//! buffers a Code pane already holds. It acquires nothing, so a file no one has
//! open answers `not_open`; it never claims a clean file. A file is
//! `document_current` only when every language server running for it reports
//! analysing exactly the text shown, `stale` when one analysed an older
//! version, and otherwise `last_observed`. None of these covers the project's
//! other files or configuration.

use serde_json::{Value, json};

use super::super::super::code_reads::language::observe_open_buffer;
use super::super::super::{AnalysisState, AppState, ServerAnalysis, ZedAdapterResponse};
use crate::machine_control::SessionReadScope;

const MAX_SERVERS: usize = 32;
const MAX_NAME_CHARS: usize = 128;

/// One file's freshness from its per-server evidence.
fn file_state(analysis: Option<&[ServerAnalysis]>) -> &'static str {
    let Some(analysis) = analysis.filter(|servers| !servers.is_empty()) else {
        return "last_observed";
    };
    if analysis
        .iter()
        .any(|server| server.state == AnalysisState::Stale)
    {
        "stale"
    } else if analysis
        .iter()
        .all(|server| server.state == AnalysisState::DocumentCurrent)
    {
        "document_current"
    } else {
        "last_observed"
    }
}

fn servers(analysis: &[ServerAnalysis]) -> Vec<Value> {
    analysis
        .iter()
        .map(|server| {
            let name = if server.name.is_empty() {
                server.server_id.to_string()
            } else {
                server.name.chars().take(MAX_NAME_CHARS).collect()
            };
            json!({"server": name, "state": server.state, "reason": server.reason})
        })
        .collect()
}

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
            Ok(ZedAdapterResponse::BufferLanguage {
                diagnostics,
                analysis,
                ..
            }) => {
                let analysis = analysis.filter(|servers| servers.len() <= MAX_SERVERS);
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
                let mut file = json!({"path":path,"state":file_state(analysis.as_deref()),
                    "counts":counts,"diagnostics":listed,"truncated":truncated});
                if let Some(analysis) = &analysis {
                    file["analysis"] = json!(servers(analysis));
                }
                file
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
        "coverage": "open_buffers",
        "files": files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(state: AnalysisState) -> ServerAnalysis {
        ServerAnalysis {
            server_id: 4,
            name: String::new(),
            state,
            reason: None,
        }
    }

    #[test]
    fn a_file_is_current_only_when_every_server_analysed_its_exact_text() {
        use AnalysisState::{DocumentCurrent, Stale, Unknown};
        assert_eq!(file_state(None), "last_observed");
        assert_eq!(file_state(Some(&[])), "last_observed");
        assert_eq!(
            file_state(Some(&[server(DocumentCurrent)])),
            "document_current"
        );
        assert_eq!(
            file_state(Some(&[server(DocumentCurrent), server(Unknown)])),
            "last_observed"
        );
        assert_eq!(file_state(Some(&[server(Unknown), server(Stale)])), "stale");
    }

    #[test]
    fn server_evidence_parses_unknown_states_conservatively_and_names_servers() {
        let parsed: Vec<ServerAnalysis> = serde_json::from_value(json!([
            {"serverId": 2, "name": "rust-analyzer", "state": "document_current"},
            {"serverId": 3, "name": "", "state": "invented", "reason": "in_flight"},
        ]))
        .unwrap();
        assert_eq!(parsed[1].state, AnalysisState::Unknown);
        assert_eq!(
            servers(&parsed),
            vec![
                json!({"server":"rust-analyzer","state":"document_current","reason":null}),
                json!({"server":"3","state":"unknown","reason":"in_flight"}),
            ]
        );
    }
}
