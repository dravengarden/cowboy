//! Scoped reconciliation of one immutable owner-authorized revocation plan.
#![allow(dead_code)]
use crate::{
    remote_runtime::{RemoteRuntime, account_stop::Outcome},
    runtime_router::RuntimeRouter,
    store::{Store, provider_accounts::revocation::stops::Evidence},
};
use anyhow::{Context as _, Result};
use std::{sync::Arc, time::Duration};

mod batch;
mod grants;
mod native;
mod pass;
mod recovery;
mod remote;
mod temporary;
mod worker_trees;
pub(in crate::server) use recovery::run as run_recovery;

/// Explicit bounded retry of an existing owner-scoped revocation intent.
pub(in crate::server) async fn reconcile(
    state: &Arc<super::super::AppState>,
    owner: &str,
    operation: &str,
) -> Result<serde_json::Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let receipt = store
        .provider_account_receipt(owner, operation)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("revocation_operation_missing")?;
    if receipt.revocation.is_none() {
        return Err("revocation_plan_missing");
    }
    if receipt.revocation_completed {
        return Ok(completed_result());
    }
    let run = pass::run(
        store,
        &state.runtime_router,
        &state.provider_auth,
        owner,
        operation,
        |login| async move {
            super::login::cancel_operation(state, owner, &login)
                .await
                .map(|_| ())
                .map_err(anyhow::Error::msg)
        },
    );
    let report = run.await.map_err(|_| "revocation_reconciliation_failed")?;
    let ready = report.sessions_stopped();
    let mut result = serde_json::to_value(report).map_err(|_| "unavailable")?;
    result["worker_trees"] = worker_trees::run(state, owner, operation)
        .await
        .map_err(|_| "revocation_reconciliation_failed")?;
    let native = if ready {
        native::run(store, &state.machine_control, owner, operation)
            .await
            .map_err(|_| "revocation_reconciliation_failed")?
    } else {
        Vec::new()
    };
    let cleanup_ready = ready && native::permits_cleanup(&native);
    result["native_profiles"] = serde_json::to_value(native).map_err(|_| "unavailable")?;
    result["temporary_logins"] = serde_json::to_value(
        temporary::run(state, owner, operation)
            .await
            .map_err(|_| "revocation_reconciliation_failed")?,
    )
    .map_err(|_| "unavailable")?;
    result["remote_grants"] = if cleanup_ready {
        serde_json::to_value(
            remote::run(state, owner, operation)
                .await
                .map_err(|_| "revocation_reconciliation_failed")?,
        )
        .map_err(|_| "unavailable")?
    } else {
        serde_json::json!({"state": if ready { "waiting_for_native_logout" } else { "waiting_for_session_stop" }})
    };
    if store
        .complete_provider_revocation(owner, operation)
        .await
        .map_err(|_| "revocation_reconciliation_failed")?
    {
        result["status"] = serde_json::json!("completed");
    }
    result["completion"] = serde_json::to_value(
        store
            .provider_revocation_readiness(owner, operation)
            .await
            .map_err(|_| "revocation_reconciliation_failed")?,
    )
    .map_err(|_| "unavailable")?;
    Ok(result)
}

fn completed_result() -> serde_json::Value {
    serde_json::json!({
        "status": "completed",
        "completion": {
            "account_completed": true,
            "missing": [],
            "completion_admission": "internal_only"
        }
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// Historical process-stop evidence only; not account-wide completion.
    Stopped(Evidence),
    Pending,
    Refused(String),
}

fn current(
    router: &RuntimeRouter,
    machine: &str,
    runtime: &Arc<RemoteRuntime>,
    epoch: u64,
) -> bool {
    runtime.account_stop_connection() == Some(epoch)
        && router
            .runtime(machine)
            .is_some_and(|active| Arc::ptr_eq(&active, runtime))
}

/// Caller authenticates the owner and authorizes the revocation operation.
/// No absent/offline Machine or lost acknowledgement is counted as completed.
pub(super) async fn stop_session(
    store: &Store,
    router: &RuntimeRouter,
    owner: &str,
    operation: &str,
    session: &str,
) -> Result<Step> {
    if let Some(evidence) = store
        .provider_account_stop(owner, operation, session)
        .await?
    {
        return Ok(Step::Stopped(evidence));
    }
    let receipt = store
        .provider_account_receipt(owner, operation)
        .await?
        .context("revocation_operation_missing")?;
    let target = receipt
        .revocation
        .as_ref()
        .context("revocation_plan_missing")?
        .sessions
        .iter()
        .find(|target| target.session_id == session)
        .context("stop_scope_mismatch")?;
    stop_planned_session(store, router, owner, operation, &receipt.account_id, target).await
}

async fn stop_planned_session(
    store: &Store,
    router: &RuntimeRouter,
    owner: &str,
    operation: &str,
    account: &crate::provider_account::ProviderAccountId,
    target: &crate::store::provider_accounts::revocation::SessionTarget,
) -> Result<Step> {
    let session = &target.session_id;
    if let Some(evidence) = store
        .provider_account_stop(owner, operation, session)
        .await?
    {
        anyhow::ensure!(
            &evidence.account_id == account
                && target.machine_id.as_ref() == Some(&evidence.machine_id),
            "stop_scope_mismatch"
        );
        return Ok(Step::Stopped(evidence));
    }
    let Some(machine) = &target.machine_id else {
        return Ok(Step::Pending);
    };
    let Some(runtime) = router.runtime(machine) else {
        return Ok(Step::Pending);
    };
    let Some(epoch) = runtime.account_stop_connection() else {
        return Ok(Step::Pending);
    };
    if !current(router, machine, &runtime, epoch) {
        return Ok(Step::Pending);
    }
    let evidence = Evidence {
        tree_exit: None,
        account_id: account.clone(),
        session_id: session.clone(),
        machine_id: machine.clone(),
        recipient_fingerprint: runtime
            .account_stop_recipient()
            .context("machine_enrollment_missing")?,
    };
    let pending = store
        .dispatch_provider_account_stop(owner, operation, &evidence, || {
            anyhow::ensure!(
                current(router, machine, &runtime, epoch),
                "machine_connection_changed"
            );
            runtime.quiesce_account_session(operation, session, &evidence.account_id)
        })
        .await?;
    match pending.wait(Duration::from_secs(60)).await {
        Outcome::Stopped => {
            let evidence = store
                .record_provider_account_stop(owner, operation, &evidence, || {
                    current(router, machine, &runtime, epoch)
                })
                .await?;
            Ok(Step::Stopped(evidence))
        }
        Outcome::Refused(reason) => Ok(Step::Refused(reason)),
        Outcome::Unconfirmed => Ok(Step::Pending),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_retry_retains_completion_object_without_remote_stages() {
        let result = completed_result();
        assert_eq!(result["status"], "completed");
        assert_eq!(result["completion"]["account_completed"], true);
        assert_eq!(result["completion"]["missing"], serde_json::json!([]));
        assert!(result.get("remote_grants").is_none());
    }
    use crate::core::Hub;
    const OWNER: &str = "0123456789abcdef0123456789abcdef";

    async fn fixture(store: &Store) -> String {
        use crate::store::provider_accounts::{Change, Mutation, MutationResult};
        use base64::Engine as _;
        let key = base64::engine::general_purpose::STANDARD.encode([12; 32]);
        let enrollment = store
            .create_machine_enrollment("step-machine", "Fixture", 60)
            .await
            .unwrap();
        store
            .consume_machine_enrollment(&enrollment, &format!("ssh-ed25519 {key}"), &key)
            .await
            .unwrap();
        store
            .insert_user(&crate::store::ProductUser {
                id: OWNER.into(),
                username: "owner".into(),
                password_algo: "fixture".into(),
                password_hash: "fixture".into(),
                created_at_ms: 1,
                updated_at_ms: 1,
                disabled_at_ms: None,
            })
            .await
            .unwrap();
        let MutationResult::Applied(created) = store
            .mutate_provider_accounts(
                OWNER,
                "create",
                &Mutation {
                    expected_revision: 0,
                    change: Change::Create {
                        provider_id: "claude-code".into(),
                        label: "Personal".into(),
                    },
                },
            )
            .await
            .unwrap()
        else {
            panic!("create account")
        };
        fixture_sessions(store, &created.account_id).await;
        store
            .claim_provider_login(
                OWNER,
                "login",
                &crate::store::provider_accounts::logins::Request {
                    account_id: created.account_id.clone(),
                    machine_id: "step-machine".into(),
                    provider_version: "fixture".into(),
                    generation_digest: "fixture".into(),
                    method: "native".into(),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            store
                .mutate_provider_accounts(
                    OWNER,
                    "revoke",
                    &Mutation {
                        expected_revision: 1,
                        change: Change::BeginRevocation {
                            id: created.account_id
                        },
                    }
                )
                .await
                .unwrap(),
            MutationResult::Applied(_)
        ));
        key
    }

    async fn fixture_sessions(store: &Store, account: &crate::provider_account::ProviderAccountId) {
        use base64::Engine as _;
        let key = base64::engine::general_purpose::STANDARD.encode([13; 32]);
        let enrollment = store
            .create_machine_enrollment("offline-machine", "Offline", 60)
            .await
            .unwrap();
        store
            .consume_machine_enrollment(&enrollment, &format!("ssh-ed25519 {key}"), &key)
            .await
            .unwrap();
        let hub = Hub::new();
        for (id, machine) in [
            ("session", "step-machine"),
            ("offline-session", "offline-machine"),
        ] {
            hub.create_local_session(
                id.into(),
                "claude-code".into(),
                "/fixture".into(),
                "Fixture".into(),
                crate::core::SessionOrigin::Api,
                false,
            );
            let mut meta = hub.session_info(id).unwrap().meta;
            meta.machine_id = machine.into();
            meta.owner_user_id = Some(OWNER.into());
            meta.provider_account_id = account.clone();
            store.insert_session(&meta).await.unwrap();
        }
    }

    #[cfg(feature = "machine-host")]
    #[tokio::test]
    async fn account_pass_keeps_offline_targets_and_resumes_without_repeating_stops() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("batch.sqlite").display()
        );
        let store = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let key = fixture(&store).await;
        let router = RuntimeRouter::new();
        let runtime = RemoteRuntime::for_account_stop_test(key);
        router.install("step-machine".into(), runtime.clone());
        let (pass, ()) = tokio::join!(
            batch::stop_sessions(&store, &router, OWNER, "revoke"),
            ack_next(&runtime, true)
        );
        assert_batch_progress(pass.unwrap());
        drop(store);
        let store = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        assert_batch_progress(
            batch::stop_sessions(&store, &router, OWNER, "revoke")
                .await
                .unwrap(),
        );
        assert!(runtime.pending_for_test().is_empty());
    }

    #[cfg(feature = "machine-host")]
    #[tokio::test]
    async fn account_pass_keeps_partial_failures_and_never_claims_logout() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::connect(
            &format!("sqlite://{}", dir.path().join("store.db").display()),
            dir.path().join("artifacts"),
        )
        .await
        .unwrap();
        store.migrate().await.unwrap();
        let key = fixture(&store).await;
        let router = RuntimeRouter::new();
        let runtime = RemoteRuntime::for_account_stop_test(key);
        router.install("step-machine".into(), runtime.clone());
        let vault =
            crate::provider_service::ProviderAuthService::open(&dir.path().join("vault")).unwrap();
        let (report, ()) = tokio::join!(
            pass::run(
                &store,
                &router,
                &vault,
                OWNER,
                "revoke",
                |operation| async move {
                    assert_eq!(operation, "login");
                    anyhow::bail!("fixture error must not leak into public report")
                }
            ),
            ack_next(&runtime, true)
        );
        let report = serde_json::to_value(report.unwrap()).unwrap();
        assert_eq!(report["status"], "revoking");
        assert_eq!(report["native_logout"], "pending");
        assert_eq!(report["logins"][0]["state"], "failed");
        assert_eq!(report["sessions"][0]["state"], "pending");
        assert_eq!(report["sessions"][1]["state"], "stopped");
        assert!(!report.to_string().contains("fixture error"));
        let retry = pass::run(&store, &router, &vault, OWNER, "revoke", |_| {
            std::future::pending::<Result<()>>()
        })
        .await
        .unwrap();
        let retry = serde_json::to_value(retry).unwrap();
        assert_eq!(retry["logins"][0]["state"], "pending");
        assert_eq!(retry["sessions"][1]["state"], "stopped");
        assert!(runtime.pending_for_test().is_empty());
        let receipt = store
            .provider_account_receipt(OWNER, "revoke")
            .await
            .unwrap()
            .unwrap();
        let value = super::super::receipt_value(&receipt);
        assert_eq!(value["status"], "revoking");
        assert_eq!(value["completion"], "unconfirmed");
        assert!(
            pass::run(
                &store,
                &router,
                &vault,
                "other-owner",
                "revoke",
                |_| async { panic!("wrong owner cannot cancel") }
            )
            .await
            .is_err()
        );
    }

    fn assert_batch_progress(pass: Vec<batch::SessionStop>) {
        assert_eq!(pass.len(), 2);
        assert_eq!(pass[0].session_id, "offline-session");
        assert!(matches!(&pass[0].outcome, Ok(Step::Pending)));
        assert_eq!(pass[1].session_id, "session");
        assert!(matches!(&pass[1].outcome, Ok(Step::Stopped(_))));
    }

    #[cfg(feature = "machine-host")]
    async fn ack_next(runtime: &RemoteRuntime, accepted: bool) {
        use crate::runtime_wire::{CoreCommand, Frame};
        let (session_id, command_id) = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                for command in runtime.pending_for_test() {
                    if let CoreCommand::QuiesceAccountSession {
                        session_id,
                        command_id,
                        ..
                    } = command
                    {
                        return (session_id, command_id);
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        runtime
            .project_trace_fixture(Frame::CommandAck {
                session_id,
                command_id,
                accepted,
                reason: (!accepted).then(|| "still running".into()),
            })
            .await;
    }

    #[cfg(feature = "machine-host")]
    #[tokio::test]
    async fn stop_driver_records_only_positive_ack_and_replays_without_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("step.sqlite").display()
        );
        let store = Store::connect(&url, dir.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let key = fixture(&store).await;
        let router = RuntimeRouter::new();
        assert_eq!(
            stop_session(&store, &router, OWNER, "revoke", "session")
                .await
                .unwrap(),
            Step::Pending
        );
        let runtime = RemoteRuntime::for_account_stop_test(key);
        router.install("step-machine".into(), runtime.clone());
        let (refused, ()) = tokio::join!(
            stop_session(&store, &router, OWNER, "revoke", "session"),
            ack_next(&runtime, false)
        );
        assert_eq!(refused.unwrap(), Step::Refused("still running".into()));
        assert!(
            store
                .provider_account_stop(OWNER, "revoke", "session")
                .await
                .unwrap()
                .is_none()
        );
        let (stopped, ()) = tokio::join!(
            stop_session(&store, &router, OWNER, "revoke", "session"),
            ack_next(&runtime, true)
        );
        let Step::Stopped(evidence) = stopped.unwrap() else {
            panic!("positive stop")
        };
        assert_eq!(
            store
                .provider_account_stop(OWNER, "revoke", "session")
                .await
                .unwrap(),
            Some(evidence.clone())
        );
        runtime.disconnect();
        assert_eq!(
            stop_session(&store, &router, OWNER, "revoke", "session")
                .await
                .unwrap(),
            Step::Stopped(evidence)
        );
        assert!(runtime.pending_for_test().is_empty());
    }

    #[tokio::test]
    async fn replacement_runtime_cannot_complete_the_old_connections_step() {
        let router = RuntimeRouter::new();
        let old = RemoteRuntime::for_test(Hub::new(), vec![]);
        old.connect_for_test();
        router.install("machine".into(), old.clone());
        let old_epoch = old.account_stop_connection().unwrap();
        assert!(current(&router, "machine", &old, old_epoch));
        old.connect_for_test();
        assert!(!current(&router, "machine", &old, old_epoch));
        let reconnected_epoch = old.account_stop_connection().unwrap();
        let replacement = RemoteRuntime::for_test(Hub::new(), vec![]);
        replacement.connect_for_test();
        router.install("machine".into(), replacement.clone());
        assert!(!current(&router, "machine", &old, reconnected_epoch));
        let epoch = replacement.account_stop_connection().unwrap();
        assert!(current(&router, "machine", &replacement, epoch));
        replacement.disconnect();
        assert!(!current(&router, "machine", &replacement, epoch));
    }
}
