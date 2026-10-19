//! Journal admission surrounds the synchronous spawn boundary.
use anyhow::{Context as _, Result};
use tokio::process::{Child, Command};

use crate::{machine_plugins::MachinePluginStore, provider_account::ProviderAccountId};

pub(super) fn binding_matches(
    previous: &crate::runtime_wire::StartSession,
    incoming: &crate::runtime_wire::StartSession,
) -> bool {
    previous.provider_account_id == incoming.provider_account_id
        && (incoming.provider_account_id.is_default()
            || (previous.provider == incoming.provider
                && previous.provider_version == incoming.provider_version
                && previous.provider_generation_digest == incoming.provider_generation_digest
                && previous.provider_auth_generation == incoming.provider_auth_generation))
}

pub(super) fn spawn_bound(
    command: &mut Command,
    store: &MachinePluginStore,
    session: &crate::runtime_wire::StartSession,
) -> Result<(Child, Option<uuid::Uuid>)> {
    let launch = (!session.provider_account_id.is_default())
        .then(|| store.begin_bound_account_worker(session))
        .transpose()?;
    spawn_reserved(
        command,
        store,
        &session.session_id,
        &session.provider_account_id,
        launch,
    )
}

#[cfg(test)]
pub(super) fn spawn(
    command: &mut Command,
    store: &MachinePluginStore,
    session: &str,
    account: &ProviderAccountId,
) -> Result<(Child, Option<uuid::Uuid>)> {
    let launch = (!account.is_default())
        .then(|| store.begin_account_worker(session, account))
        .transpose()?;
    spawn_reserved(command, store, session, account, launch)
}

fn spawn_reserved(
    command: &mut Command,
    store: &MachinePluginStore,
    session: &str,
    account: &ProviderAccountId,
    launch: Option<uuid::Uuid>,
) -> Result<(Child, Option<uuid::Uuid>)> {
    if let Some(launch) = launch {
        command.env(super::systemd_exit::LAUNCH_ENV, launch.to_string());
    }
    match command.spawn() {
        Ok(child) => Ok((child, launch)),
        Err(error) => {
            // A synchronous spawn failure did not execute the worker (or
            // systemd-run). Unlike an unsuccessful child exit, it cannot have
            // dispatched a unit or run provider code. Keep unknown state if
            // recording this fact fails; never remove the reservation first.
            if let Some(launch) = launch {
                store
                    .record_account_worker_exit(session, account, launch)
                    .with_context(|| {
                        format!("spawn failed ({error}); recording non-start failed")
                    })?;
            }
            Err(error).context("spawning account worker")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::Platform;

    #[test]
    fn named_adoption_cannot_replace_its_authenticated_generation() {
        let original: crate::runtime_wire::StartSession = serde_json::from_value(serde_json::json!({
            "session_id":"s", "provider":"claude-code", "provider_account_id":uuid::Uuid::new_v4().to_string(),
            "provider_version":"1", "provider_generation_digest":"digest", "provider_auth_generation":7,
            "cwd":"/fixture", "generation":"worker"
        })).unwrap();
        let mut adoption = original.clone();
        adoption.adopt_only = true;
        assert!(binding_matches(&original, &adoption));
        for field in 0..5 {
            let mut changed = adoption.clone();
            match field {
                0 => changed.provider = "other".into(),
                1 => changed.provider_version = "2".into(),
                2 => changed.provider_generation_digest = "other".into(),
                3 => changed.provider_auth_generation = Some(8),
                _ => changed.provider_auth_generation = None,
            }
            assert!(!binding_matches(&original, &changed));
        }
        let mut legacy = original.clone();
        legacy.provider_account_id = ProviderAccountId::default();
        let mut upgraded = legacy.clone();
        upgraded.provider_auth_generation = Some(8);
        assert!(binding_matches(&legacy, &upgraded));
        assert!(!binding_matches(&original, &legacy));
    }

    #[tokio::test]
    async fn failed_spawn_allows_retry_but_started_child_requires_exit_proof() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(temp.path(), Platform::Linux, "x86_64".into()).unwrap();
        let account = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let mut missing = Command::new(temp.path().join("missing-worker"));
        assert!(spawn(&mut missing, &store, "s", &account).is_err());
        let failed = store.account_worker_exit("s").unwrap().unwrap();
        assert!(failed.tree_exited);
        drop(store);
        let store = MachinePluginStore::new(temp.path(), Platform::Linux, "x86_64".into()).unwrap();
        let (mut child, launch) = spawn(&mut Command::new("true"), &store, "s", &account).unwrap();
        assert!(child.wait().await.unwrap().success());
        assert_ne!(launch.unwrap(), failed.launch);
        assert!(!store.account_worker_exit("s").unwrap().unwrap().tree_exited);
        // Even a successful child exit is not a tree observation.
        assert!(spawn(&mut missing, &store, "s", &account).is_err());
        assert!(!store.account_worker_exit("s").unwrap().unwrap().tree_exited);
        assert!(
            store
                .record_account_worker_exit("s", &account, failed.launch)
                .is_err()
        );
    }
}
