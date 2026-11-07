//! `cowboy.code.diagnostics`: the language engine's observation of a file of
//! this session's worktree. A file no Code pane holds is acquired for the
//! session's agent when its "Project language servers" switch is on and the
//! agent share allows (`acquire`), and reported `"acquired": true`; otherwise
//! it answers `not_open`. It never claims a clean file. A file is
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

/// How long a newly acquired file may wait for its servers' first analysis.
const ANALYSIS_WAIT: std::time::Duration = std::time::Duration::from_secs(4);

/// Read a file a Code pane holds; otherwise acquire it for this session's
/// agent and wait, briefly, for evidence that its servers analysed it.
async fn observe(
    state: &AppState,
    observed: &SessionReadScope,
    path: &str,
) -> (anyhow::Result<ZedAdapterResponse>, bool) {
    let first = observe_open_buffer(state, observed, path).await;
    let session = observed.session().session_id();
    if !matches!(&first, Err(error) if error.to_string().contains("not open")) {
        if first.is_ok() {
            // Track an open file under the agent's own owner too, within its
            // share: a holding left by an earlier Controller is then released
            // when idle instead of pinning its code runtime indefinitely.
            let _ = super::acquire::acquire(state, session, path).await;
        }
        return (first, false);
    }
    if let Err(reason) = super::acquire::acquire(state, session, path).await {
        return (Err(anyhow::anyhow!("not open: {reason}")), false);
    }
    let deadline = tokio::time::Instant::now() + ANALYSIS_WAIT;
    loop {
        let reply = observe_open_buffer(state, observed, path).await;
        let analysed = matches!(&reply, Ok(ZedAdapterResponse::BufferLanguage { analysis: Some(servers), .. })
            if !servers.is_empty() && servers.iter().all(|server| server.state != AnalysisState::Unknown));
        if analysed || reply.is_err() || tokio::time::Instant::now() >= deadline {
            return (reply, true);
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
}

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

fn diagnostic(value: &super::super::super::CodeDiagnostic) -> Value {
    json!({
        "severity": severity(value.severity),
        "line": value.start.row + 1,
        "column": value.start.column + 1,
        "endLine": value.end.row + 1,
        "endColumn": value.end.column + 1,
        "source": value.source,
        // Never truncate identity strings into a different identity.
        "code": value.code.as_ref().filter(|value| value.len() <= 256),
        "provenance": value.provenance.as_ref().filter(|value| value.bounded()),
        "message": value.message.chars().take(MAX_MESSAGE_CHARS).collect::<String>(),
    })
}

pub(super) async fn read(
    state: &AppState,
    observed: &SessionReadScope,
    paths: Vec<String>,
) -> Value {
    read_with_acquisition(state, observed, paths, true).await
}

pub(super) async fn read_existing(
    state: &AppState,
    observed: &SessionReadScope,
    paths: Vec<String>,
) -> Value {
    read_with_acquisition(state, observed, paths, false).await
}

async fn read_with_acquisition(
    state: &AppState,
    observed: &SessionReadScope,
    paths: Vec<String>,
    acquire: bool,
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
        let (reply, acquired) = if acquire {
            observe(state, observed, &path).await
        } else {
            (observe_open_buffer(state, observed, &path).await, false)
        };
        files.push(match reply {
            Ok(ZedAdapterResponse::BufferLanguage {
                diagnostics,
                analysis,
                buffer_snapshot,
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
                    .map(diagnostic)
                    .collect();
                let mut file = json!({"path":path,"state":file_state(analysis.as_deref()),
                    "counts":counts,"diagnostics":listed,"truncated":truncated,
                    "bufferSnapshot":buffer_snapshot.filter(|value| value.bounded())});
                if acquired {
                    file["acquired"] = json!(true);
                }
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
    // Revocation during an asynchronous query discards its observation too.
    if !state
        .hub
        .language_servers_allowed(observed.session().session_id())
    {
        for file in &mut files {
            *file = json!({"path":file["path"],"state":"restricted"});
        }
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
    fn provenance_and_codes_survive_projection_without_inventing_missing_identity() {
        let old = json!({"start":{"row":0,"column":1},"end":{"row":2,"column":3},
            "severity":1,"source":"compiler","message":"wording"});
        let project = |value| diagnostic(&serde_json::from_value(value).unwrap());
        let historical = project(old.clone());
        assert!(historical["code"].is_null());
        assert!(historical["provenance"].is_null());
        let mut current = old;
        current["code"] = json!("E0123");
        current["provenance"] = json!({"instance":"a".repeat(32),"buffer":"7","bufferEpoch":"1",
            "server":"18446744073709551615","sourceKind":1,"registration":"r"});
        let projected = project(current.clone());
        assert_eq!(projected["code"], "E0123");
        assert_eq!(projected["provenance"], current["provenance"]);
        assert_eq!(projected["endColumn"], 4);
        current["message"] = json!("different wording");
        current["severity"] = json!(2);
        assert_eq!(
            project(current.clone())["provenance"],
            projected["provenance"]
        );
        current["code"] = json!("x".repeat(257));
        current["provenance"]["registration"] = json!("x".repeat(257));
        let oversized = project(current);
        assert!(oversized["code"].is_null());
        assert!(oversized["provenance"].is_null());
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
