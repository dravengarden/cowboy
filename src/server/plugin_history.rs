//! Bounded, durable, read-only core lifecycle diagnostics. This module has no
//! Machine port, installer, authorization permit or recovery executor. Separate
//! journals are independent observations, not a cross-site atomic snapshot.

use super::*;
use crate::plugin_operation::installation::InstallOperation;
use crate::plugin_operation::resolution::{ResolutionAction, ResolutionReceipt};
use crate::plugin_operation::{Operation, Phase, Problem};
use anyhow::ensure;
use axum::http::HeaderValue;
use std::time::Duration;

#[derive(Serialize)]
struct UninstallEvidence<'a> {
    evidence_schema: u32,
    operation_id: &'a str,
    phase: Phase,
    problem: Option<Problem>,
    cause: Option<Problem>,
    attention_from: Option<Phase>,
    plugin_version: &'a str,
    generation_digest: &'a str,
    affected_session_count: usize,
    purge_after_ms: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl<'a> From<&'a Operation> for UninstallEvidence<'a> {
    fn from(op: &'a Operation) -> Self {
        Self {
            evidence_schema: op.intent.schema,
            operation_id: &op.intent.operation_id,
            phase: op.phase,
            problem: op.problem,
            cause: op.cause,
            attention_from: op.attention_from,
            plugin_version: &op.intent.plugin_version,
            generation_digest: &op.intent.generation_digest,
            affected_session_count: op.intent.session_ids.len(),
            purge_after_ms: op.intent.purge_after_ms,
            created_at_ms: op.created_at_ms,
            updated_at_ms: op.updated_at_ms,
        }
    }
}

#[derive(Serialize)]
struct ResolutionEvidence<'a> {
    resolution_id: &'a str,
    action: ResolutionAction,
    resolved_at_ms: i64,
    plugin_mutation_performed: bool,
    session_mutation_performed: bool,
    worker_restoration_performed: bool,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Entry<'a> {
    Install {
        operation: plugin_install::journal::Evidence<'a>,
    },
    Uninstall {
        operation: UninstallEvidence<'a>,
        resolution: Option<ResolutionEvidence<'a>>,
    },
}

fn uninstall_entry<'a>(
    op: &'a Operation,
    receipt: Option<&'a ResolutionReceipt>,
) -> anyhow::Result<Entry<'a>> {
    let resolution = receipt
        .map(|receipt| {
            // A resolution that arrived after the operation read is a changed
            // observation, not permission to glue incompatible records together.
            ensure!(
                receipt.matches_completed(op)?,
                "Plugin resolution observation changed"
            );
            Ok::<_, anyhow::Error>(ResolutionEvidence {
                resolution_id: &receipt.intent.resolution_id,
                action: receipt.intent.action,
                resolved_at_ms: receipt.resolved_at_ms,
                plugin_mutation_performed: false,
                session_mutation_performed: false,
                worker_restoration_performed: false,
            })
        })
        .transpose()?;
    Ok(Entry::Uninstall {
        operation: op.into(),
        resolution,
    })
}

#[derive(Serialize)]
struct Admission {
    install: bool,
    uninstall: bool,
}

#[derive(Serialize)]
struct History<'a> {
    schema: &'static str,
    machine_id: &'a str,
    plugin_id: &'a str,
    execution_authorized: bool,
    observation: &'static str,
    window: &'static str,
    limit_per_kind: u16,
    admission: Admission,
    requires_reconciliation: bool,
    entries: Vec<Entry<'a>>,
}

fn project<'a>(
    machine: &'a str,
    plugin: &'a str,
    installs: &'a [InstallOperation],
    uninstalls: &'a [(Operation, Option<ResolutionReceipt>)],
    requires_reconciliation: bool,
) -> anyhow::Result<History<'a>> {
    ensure!(
        installs.len() <= 32 && uninstalls.len() <= 32,
        "history exceeds window"
    );
    // Domain + ID is the identity: the two durable journals may reuse an ID.
    // Ordering is presentation only, never causal DAG order or current state.
    let mut rows = Vec::with_capacity(installs.len() + uninstalls.len());
    for op in installs {
        rows.push((
            op.updated_at_ms,
            "install",
            &op.intent.operation_id,
            Entry::Install {
                operation: op.into(),
            },
        ));
    }
    for (op, receipt) in uninstalls {
        rows.push((
            op.updated_at_ms,
            "uninstall",
            &op.intent.operation_id,
            uninstall_entry(op, receipt.as_ref())?,
        ));
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)).then(a.2.cmp(b.2)));
    Ok(History {
        schema: "dravengarden.cowboy.plugin-lifecycle-history/v1",
        machine_id: machine,
        plugin_id: plugin,
        execution_authorized: false,
        observation: "independent_durable_reads",
        window: "latest_per_kind",
        limit_per_kind: 32,
        admission: Admission {
            install: plugin_install::DURABLE_INSTALL_ENABLED,
            uninstall: plugin_uninstall::DURABLE_UNINSTALL_ENABLED,
        },
        requires_reconciliation,
        entries: rows.into_iter().map(|(_, _, _, entry)| entry).collect(),
    })
}

async fn read(
    store: &Store,
    service: &str,
    machine: &str,
    plugin: &str,
) -> anyhow::Result<(
    Vec<InstallOperation>,
    Vec<(Operation, Option<ResolutionReceipt>)>,
)> {
    let (installs, uninstalls) = tokio::try_join!(
        store.plugin_install_history(service, machine, plugin),
        store.plugin_uninstall_history(service, machine, plugin),
    )?;
    let mut with_resolutions = Vec::with_capacity(uninstalls.len());
    for op in uninstalls {
        let resolution = store
            .plugin_uninstall_resolution(&op.intent.operation_id)
            .await?;
        with_resolutions.push((op, resolution));
    }
    Ok((installs, with_resolutions))
}

pub(super) async fn get_history(
    State(state): State<Arc<AppState>>,
    Path((machine, plugin)): Path<(String, String)>,
) -> Response {
    let result = async {
        let store = state
            .store
            .as_ref()
            .context("history storage unavailable")?;
        let (installs, uninstalls) = read(store, &state.service_id, &machine, &plugin).await?;
        let fenced = state
            .plugin_lifecycle_fences
            .read()
            .get(&(machine.clone(), plugin.clone()))
            == Some(&PluginFenceState::NeedsReconcile);
        let history = project(&machine, &plugin, &installs, &uninstalls, fenced)?;
        let bytes = serde_json::to_vec(&history)?;
        ensure!(bytes.len() <= 128 * 1024, "history exceeds response budget");
        Ok::<_, anyhow::Error>(
            ([(header::CONTENT_TYPE, "application/json")], bytes).into_response(),
        )
    };
    let mut response = match tokio::time::timeout(Duration::from_secs(5), result).await {
        Ok(Ok(response)) => response,
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Plugin lifecycle evidence is unavailable or changed; no operation was repeated",
        )
            .into_response(),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_operation::resolution::{ResolutionIntent, ResolutionPermit};

    #[test]
    fn projection_matches_the_exact_typescript_browser_fixture() {
        use crate::machine_protocol::plugin_install::{InstallOutcome, InstallReceipt};
        use crate::plugin_operation::installation::{InstallPhase, machine_fixture};
        let mut intent = machine_fixture("shared");
        intent.operation_id = "operation-shared-fixture".into();
        intent.request_id = format!("plugin-install-{}", intent.operation_id);
        intent.machine_id = "hawk".into();
        intent.expires_at_ms = 3;
        let step = intent.machine_step().unwrap();
        let install = InstallOperation {
            machine_receipt: Some(InstallReceipt {
                request_digest: step.request_digest().unwrap(),
                step,
                outcome: InstallOutcome::Applied {
                    revision: format!("installation-{}", "b".repeat(64))
                        .try_into()
                        .unwrap(),
                },
            }),
            intent,
            phase: InstallPhase::Completed,
            problem: None,
            attention_from: None,
            created_at_ms: 1,
            updated_at_ms: 3,
        };
        install.validate().unwrap();
        let mut before = crate::plugin_operation::resolution::fixture();
        before.intent.schema = 2;
        before.intent.installation_revision = Some(
            format!("installation-{}", "b".repeat(64))
                .try_into()
                .unwrap(),
        );
        before.intent.operation_id = "operation-shared-fixture".into();
        before.intent.session_ids = vec!["session-one".into(), "session-two".into()];
        before.intent.expires_at_ms = 3;
        before.intent.purge_after_ms = 4;
        let receipt = ResolutionReceipt {
            intent: ResolutionIntent::new(
                "resolution-separate-fixture".into(),
                before.intent.actor.clone(),
                &before,
                3,
            )
            .unwrap(),
            resolved_at_ms: 3,
        };
        let after = Operation {
            phase: Phase::Aborted,
            updated_at_ms: 3,
            ..before
        };
        let rows = [(after, Some(receipt))];
        let installs = [install];
        let history = project("hawk", "victoria", &installs, &rows, false).unwrap();
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/plugin-lifecycle-history.json"
        ))
        .unwrap();
        assert_eq!(serde_json::to_value(history).unwrap(), expected);
    }

    #[tokio::test]
    async fn history_is_target_scoped_read_only_and_retains_independent_resolution() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let intent = crate::plugin_operation::fixture("history");
        store.begin_plugin_uninstall(&intent).await.unwrap();
        store
            .advance_plugin_uninstall(
                &intent.operation_id,
                Phase::Prepared,
                Phase::NeedsAttention,
                Some(Problem::Interrupted),
            )
            .await
            .unwrap();
        let before = store
            .plugin_uninstall_operation(&intent.operation_id)
            .await
            .unwrap()
            .unwrap();
        let (installs, uninstalls) = read(&store, "service-test", "hawk", "victoria")
            .await
            .unwrap();
        let bytes = serde_json::to_value(
            project("hawk", "victoria", &installs, &uninstalls, true).unwrap(),
        )
        .unwrap();
        assert_eq!(bytes["entries"][0]["operation"]["phase"], "needs_attention");
        assert!(bytes["entries"][0]["resolution"].is_null());
        assert_eq!(
            store
                .plugin_uninstall_operation(&intent.operation_id)
                .await
                .unwrap()
                .unwrap(),
            before
        );
        for (service, machine, plugin) in [
            ("other", "hawk", "victoria"),
            ("service-test", "other", "victoria"),
            ("service-test", "hawk", "other"),
        ] {
            let (installs, uninstalls) = read(&store, service, machine, plugin).await.unwrap();
            assert!(installs.is_empty() && uninstalls.is_empty());
        }
        let receipt = store
            .resolve_plugin_uninstall(&ResolutionPermit::for_test(
                ResolutionIntent::new(
                    "resolution-history-fixture".into(),
                    intent.actor.clone(),
                    &before,
                    intent.expires_at_ms,
                )
                .unwrap(),
            ))
            .await
            .unwrap();
        assert!(
            uninstall_entry(&before, Some(&receipt)).is_err(),
            "no mixed read may claim resolution"
        );
        let (installs, uninstalls) = read(&store, "service-test", "hawk", "victoria")
            .await
            .unwrap();
        let history = serde_json::to_value(
            project("hawk", "victoria", &installs, &uninstalls, false).unwrap(),
        )
        .unwrap();
        assert_eq!(
            history["entries"][0]["resolution"]["action"],
            "abort_before_effects"
        );
        assert_eq!(history["entries"][0]["operation"]["phase"], "aborted");
        assert_eq!(history["execution_authorized"], false);
        let document = serde_json::to_string(&history).unwrap();
        for private in [
            "actor",
            "user-test",
            "session_ids",
            "operation_digest",
            "expires_at_ms",
            "service_id",
            "contract_fingerprint",
        ] {
            assert!(
                !document.contains(private),
                "private evidence escaped: {private}"
            );
        }
        let (again, operations) = read(&store, "service-test", "hawk", "victoria")
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(project("hawk", "victoria", &again, &operations, false).unwrap())
                .unwrap(),
            history
        );
        assert_eq!(
            super::super::classify_route(
                &Method::GET,
                "/api/machines/hawk/plugins/victoria/lifecycle-history"
            ),
            super::super::RouteAuth::ProductOrAdminOperator
        );
    }
}
