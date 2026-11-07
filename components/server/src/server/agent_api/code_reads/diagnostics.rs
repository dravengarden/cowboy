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
use super::super::super::{
    AnalysisState, AppState, ServerAnalysis, SourceState, ZedAdapterResponse,
};
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
    trust: &crate::core::LanguageServerTrust,
) -> (anyhow::Result<ZedAdapterResponse>, bool) {
    let first = observe_open_buffer(state, observed, path).await;
    if !trust.current(&state.hub) {
        return (Err(anyhow::anyhow!("restricted")), false);
    }
    let session = observed.session().session_id();
    if !matches!(&first, Err(error) if error.to_string().contains("not open")) {
        if first.is_ok() {
            // Track an open file under the agent's own owner too, within its
            // share: a holding left by an earlier Controller is then released
            // when idle instead of pinning its code runtime indefinitely.
            let _ = super::acquire::acquire(state, session, path, trust).await;
        }
        return (first, false);
    }
    if let Err(reason) = super::acquire::acquire(state, session, path, trust).await {
        return (Err(anyhow::anyhow!("not open: {reason}")), false);
    }
    let deadline = tokio::time::Instant::now() + ANALYSIS_WAIT;
    loop {
        if !trust.current(&state.hub) {
            return (Err(anyhow::anyhow!("restricted")), true);
        }
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
fn file_state(analysis: Option<&[ServerAnalysis]>, source: SourceState) -> &'static str {
    if source == SourceState::Changed {
        return "stale";
    }
    let Some(analysis) = analysis.filter(|servers| !servers.is_empty()) else {
        return "last_observed";
    };
    if analysis
        .iter()
        .any(|server| server.state == AnalysisState::Stale)
    {
        "stale"
    } else if source == SourceState::Matching
        && analysis
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

fn identities_match(
    diagnostics: &[super::super::super::CodeDiagnostic],
    snapshot: Option<&super::super::super::BufferSnapshot>,
) -> bool {
    diagnostics.iter().all(|diagnostic| {
        diagnostic.provenance.as_ref().is_none_or(|provenance| {
            provenance.bounded() && snapshot.is_none_or(|snapshot| provenance.belongs_to(snapshot))
        })
    })
}

fn severity(value: i32) -> &'static str {
    match value {
        1 => "error",
        2 => "warning",
        3 => "information",
        4 => "hint",
        _ => "unknown",
    }
}

fn diagnostic(
    value: &super::super::super::CodeDiagnostic,
    snapshot: Option<&super::super::super::BufferSnapshot>,
) -> Value {
    json!({
        "severity": severity(value.severity),
        "line": value.start.row + 1,
        "column": value.start.column + 1,
        "endLine": value.end.row + 1,
        "endColumn": value.end.column + 1,
        "source": value.source,
        // Never truncate identity strings into a different identity.
        "code": value.code.as_ref().filter(|value| value.len() <= 256),
        "provenance": value.provenance.as_ref().filter(|value| snapshot.is_some_and(|snapshot| value.belongs_to(snapshot))),
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
    let trust = state
        .hub
        .observe_language_server_trust(observed.session().session_id())
        .filter(|trust| trust.scope() == observed.session());
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        if trust
            .as_ref()
            .is_none_or(|trust| !trust.current(&state.hub))
        {
            files.push(json!({"path":path,"state":"restricted"}));
            continue;
        }
        let (reply, acquired) = if acquire {
            observe(
                state,
                observed,
                &path,
                trust.as_ref().expect("admitted trust"),
            )
            .await
        } else {
            (observe_open_buffer(state, observed, &path).await, false)
        };
        files.push(match reply {
            Ok(ZedAdapterResponse::BufferLanguage {
                diagnostics,
                analysis,
                buffer_snapshot,
                source_state,
                ..
            }) => {
                let analysis = analysis.filter(|servers| servers.len() <= MAX_SERVERS);
                let buffer_snapshot = buffer_snapshot.filter(|value| value.bounded());
                // A conflicting identity invalidates the entire observation,
                // including counts. Dropping only provenance could still label
                // another buffer's diagnostics as document_current.
                if !identities_match(&diagnostics, buffer_snapshot.as_deref()) {
                    files.push(json!({"path":path,"state":"unavailable"}));
                    continue;
                }
                let source_state = if buffer_snapshot.is_some() { source_state } else { SourceState::Unavailable };
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
                    .map(|value| diagnostic(value, buffer_snapshot.as_deref()))
                    .collect();
                let mut file = json!({"path":path,"state":file_state(analysis.as_deref(), source_state),
                    "counts":counts,"diagnostics":listed,"truncated":truncated,
                    "bufferSnapshot":buffer_snapshot,"sourceState":source_state});
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
    // Revocation during an asynchronous query discards its observation even
    // when the owner re-enabled language servers before the reply arrived.
    fence_files(&state.hub, trust.as_ref(), &mut files);
    json!({
        "coverage": "open_buffers",
        "files": files,
    })
}

fn fence_files(
    hub: &crate::core::Hub,
    trust: Option<&crate::core::LanguageServerTrust>,
    files: &mut [Value],
) {
    if trust.is_none_or(|trust| !trust.current(hub)) {
        for file in files {
            *file = json!({"path":file["path"],"state":"restricted"});
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reenabled_policy_does_not_publish_a_reply_from_before_revocation() {
        let hub = crate::core::Hub::new();
        hub.create_local_session(
            "diagnostic-policy".into(),
            "codex".into(),
            "/tmp".into(),
            "fixture".into(),
            crate::core::SessionOrigin::Web,
            false,
        );
        hub.sync_apply(
            "folders",
            "create".into(),
            "create",
            &json!({"id":"folder","name":"F","project":null}),
        )
        .unwrap();
        hub.sync_apply(
            "folders",
            "place".into(),
            "place",
            &json!({"session_ids":["diagnostic-policy"],"folder":"folder"}),
        )
        .unwrap();
        let original = hub
            .observe_language_server_trust("diagnostic-policy")
            .unwrap();
        let pending_reply = async {
            for value in ["off", "on"] {
                hub.sync_apply(
                    "folders",
                    value.into(),
                    "defaults",
                    &json!({"id":"folder","values":{"language_servers":value}}),
                )
                .unwrap();
            }
            vec![json!({"path":"file.rs","state":"document_current",
                "counts":{"error":0},"diagnostics":[],"bufferSnapshot":{"private":"old"}})]
        };
        let mut files = pending_reply.await;
        assert!(hub.language_servers_allowed("diagnostic-policy"));
        fence_files(&hub, Some(&original), &mut files);
        assert_eq!(files, vec![json!({"path":"file.rs","state":"restricted"})]);
    }

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
        let snapshot = serde_json::from_value(json!({"instance":"a".repeat(32),"buffer":"7",
            "bufferEpoch":"1","version":[],"content":{"sha256":"b".repeat(64),"utf8Bytes":0}}))
        .unwrap();
        let project = |value| diagnostic(&serde_json::from_value(value).unwrap(), Some(&snapshot));
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
    fn diagnostic_identity_requires_the_original_bounded_buffer_snapshot() {
        let snapshot_value = json!({"instance":"a".repeat(32),"buffer":"7",
            "bufferEpoch":"1","version":[],"content":{"sha256":"b".repeat(64),"utf8Bytes":10}});
        let snapshot = serde_json::from_value(snapshot_value.clone()).unwrap();
        let value = json!({"start":{"row":0,"column":1},"end":{"row":0,"column":3},
            "severity":1,"source":"compiler","message":"diagnostic",
            "provenance":{"instance":"a".repeat(32),"buffer":"7","bufferEpoch":"1",
                "server":"0","sourceKind":1,"registration":null}});
        let original = serde_json::from_value(value.clone()).unwrap();
        assert!(!diagnostic(&original, Some(&snapshot))["provenance"].is_null());
        assert!(diagnostic(&original, None)["provenance"].is_null());
        assert!(identities_match(
            std::slice::from_ref(&original),
            Some(&snapshot)
        ));
        for (field, replacement) in [
            ("instance", json!("c".repeat(32))),
            ("instance", json!("A".repeat(32))),
            ("buffer", json!("8")),
            ("bufferEpoch", json!("2")),
            ("buffer", json!("07")),
            ("bufferEpoch", json!("+1")),
            ("server", json!("00")),
            ("server", json!("+0")),
        ] {
            let mut changed = value.clone();
            changed["provenance"][field] = replacement;
            let changed = serde_json::from_value(changed).unwrap();
            assert!(
                !identities_match(std::slice::from_ref(&changed), Some(&snapshot)),
                "{field}"
            );
            let projected = diagnostic(&changed, Some(&snapshot));
            assert!(projected["provenance"].is_null(), "{field}");
            assert_eq!(projected["message"], "diagnostic");
        }
        let mut invalid = snapshot_value;
        invalid["content"]["sha256"] = json!("invalid");
        assert!(
            diagnostic(&original, Some(&serde_json::from_value(invalid).unwrap()))["provenance"]
                .is_null()
        );
    }

    #[test]
    fn a_file_is_current_only_when_every_server_analysed_its_exact_text() {
        use AnalysisState::{DocumentCurrent, Stale, Unknown};
        assert_eq!(file_state(None, SourceState::Matching), "last_observed");
        assert_eq!(
            file_state(Some(&[]), SourceState::Matching),
            "last_observed"
        );
        assert_eq!(
            file_state(Some(&[server(DocumentCurrent)]), SourceState::Matching),
            "document_current"
        );
        assert_eq!(
            file_state(
                Some(&[server(DocumentCurrent), server(Unknown)]),
                SourceState::Matching
            ),
            "last_observed"
        );
        assert_eq!(
            file_state(
                Some(&[server(Unknown), server(Stale)]),
                SourceState::Matching
            ),
            "stale"
        );
        assert_eq!(
            file_state(Some(&[server(DocumentCurrent)]), SourceState::Changed),
            "stale"
        );
        assert_eq!(
            file_state(Some(&[server(DocumentCurrent)]), SourceState::Unavailable),
            "last_observed"
        );
        assert_eq!(
            file_state(Some(&[server(Stale)]), SourceState::Unavailable),
            "stale"
        );
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
