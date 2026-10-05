//! Service-owned placement for account usage operations, independent of sessions.

use super::{PluginCommandRoute, UsageService};
use anyhow::{Context as _, Result, ensure};
use serde::Serialize;
use std::collections::BTreeMap;

pub(super) fn select_candidate<'a>(
    preferred: Option<&str>,
    mut candidates: Vec<(&'a str, String, crate::machine_protocol::PluginInventory)>,
    restrictions: &crate::project_placement::RuntimeRestrictions,
) -> Option<(&'a str, String, crate::machine_protocol::PluginInventory)> {
    candidates.retain(|candidate| restrictions.allows(&candidate.2.plugin_id, &candidate.1));
    if let Some(machine) = preferred {
        candidates.retain(|candidate| candidate.1 == machine);
    }
    candidates.sort_by(|left, right| {
        semver::Version::parse(right.0)
            .ok()
            .cmp(&semver::Version::parse(left.0).ok())
            .then(left.1.cmp(&right.1))
            .then(left.2.generation_digest.cmp(&right.2.generation_digest))
    });
    candidates.into_iter().next()
}

#[derive(Serialize)]
pub struct UsageExecutionSettings {
    pub providers: BTreeMap<String, UsageExecutor>,
    pub machines: Vec<UsageMachine>,
}

#[derive(Serialize)]
pub struct UsageExecutor {
    pub machine_id: Option<String>,
    pub selected_machine_id: Option<String>,
    pub status: &'static str,
    pub detail: Option<String>,
}

#[derive(Serialize)]
pub struct UsageMachine {
    pub id: String,
    pub name: String,
    pub status: String,
}

impl UsageService {
    pub async fn restore_execution_settings(&self) -> Result<()> {
        let Some(store) = &self.store else {
            return Ok(());
        };
        let configured = store.usage_execution_machines().await?;
        *self.execution_machines.write() = configured;
        Ok(())
    }

    pub async fn execution_settings(&self) -> Result<UsageExecutionSettings> {
        let machines = match &self.store {
            Some(store) => store.list_machines().await?,
            None => Vec::new(),
        };
        let configured = self.execution_machines.read().clone();
        let providers = self
            .plugin_bindings()
            .into_iter()
            .filter(|binding| !binding.collector_argv.is_empty())
            .map(|binding| {
                let (selected_machine_id, status, detail) = match self.command_route(
                    &binding.account,
                    crate::machine_protocol::PluginHostOperation::CollectUsage,
                ) {
                    PluginCommandRoute::Machine { machine_id, .. } => {
                        (Some(machine_id), "ready", None)
                    }
                    PluginCommandRoute::Bootstrap => (None, "service", None),
                    PluginCommandRoute::Unavailable(detail) => (None, "unavailable", Some(detail)),
                };
                let executor = UsageExecutor {
                    machine_id: configured.get(&binding.account).cloned(),
                    selected_machine_id,
                    status,
                    detail,
                };
                (binding.account, executor)
            })
            .collect();
        Ok(UsageExecutionSettings {
            providers,
            machines: machines
                .into_iter()
                .filter(|machine| !machine.revoked)
                .map(|machine| UsageMachine {
                    id: machine.id,
                    name: machine.display_name,
                    status: machine.status,
                })
                .collect(),
        })
    }

    pub async fn set_execution_machine(
        &self,
        account: &str,
        machine_id: Option<String>,
    ) -> Result<()> {
        ensure!(
            self.plugin_bindings()
                .iter()
                .any(|binding| binding.account == account && !binding.collector_argv.is_empty()),
            "account has no usage collector"
        );
        let store = self
            .store
            .as_ref()
            .context("usage execution settings require persistent storage")?;
        if let Some(machine_id) = &machine_id {
            ensure!(
                store
                    .list_machines()
                    .await?
                    .iter()
                    .any(|machine| &machine.id == machine_id && !machine.revoked),
                "Machine is unknown or revoked"
            );
        }
        // Serialize with collectors and other edits: a completed refresh cannot
        // publish an old executor's result after this mutation returns. Wait
        // for this account's collection, then hold its slot so none starts
        // until the new route is recorded.
        loop {
            self.wait_settled(&[account.to_owned()]).await;
            if self.in_flight.lock().insert(account.to_owned()) {
                break;
            }
        }
        let result = self
            .record_execution_machine(store, account, machine_id)
            .await;
        self.in_flight.lock().remove(account);
        result?;
        self.notify();
        tracing::info!(account, "usage execution Machine updated");
        Ok(())
    }

    async fn record_execution_machine(
        &self,
        store: &crate::store::Store,
        account: &str,
        machine_id: Option<String>,
    ) -> Result<()> {
        let mut configured = self.execution_machines.read().clone();
        if configured.get(account) == machine_id.as_ref() {
            return Ok(());
        }
        store
            .set_usage_execution_machine(account, machine_id.as_deref())
            .await?;
        if let Some(machine_id) = machine_id {
            configured.insert(account.to_owned(), machine_id);
        } else {
            configured.remove(account);
        }
        *self.execution_machines.write() = configured;
        // The next explicit refresh must not reuse the previous route's cooldown.
        if let Some(provider) = self
            .snapshot
            .lock()
            .await
            .providers
            .iter_mut()
            .find(|provider| provider.provider == account)
        {
            provider.refresh = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::PluginInventory;

    fn inventory(version: &str) -> PluginInventory {
        serde_json::from_value(serde_json::json!({
            "plugin_id":"agent", "plugin_version":version,
            "generation_digest":"sha256:fixture", "contract_fingerprint":"fixture", "state":"active"
        }))
        .unwrap()
    }

    #[test]
    fn pinned_machine_beats_newer_elsewhere_and_never_falls_back() {
        let restrictions = crate::project_placement::RuntimeRestrictions::default();
        let candidates = || {
            vec![
                ("2.0.0", "hawk".into(), inventory("2.0.0")),
                ("1.0.0", "ovh".into(), inventory("1.0.0")),
            ]
        };
        assert_eq!(
            select_candidate(None, candidates(), &restrictions)
                .unwrap()
                .1,
            "hawk"
        );
        assert_eq!(
            select_candidate(Some("ovh"), candidates(), &restrictions)
                .unwrap()
                .1,
            "ovh"
        );
        assert!(select_candidate(Some("offline"), candidates(), &restrictions).is_none());
        assert!(select_candidate(Some("ovh"), vec![], &restrictions).is_none());
        let restricted =
            crate::project_placement::RuntimeRestrictions::parse(&["agent=ovh".into()]).unwrap();
        assert_eq!(
            select_candidate(None, candidates(), &restricted).unwrap().1,
            "ovh"
        );
        assert!(select_candidate(Some("hawk"), candidates(), &restricted).is_none());
        assert!(
            select_candidate(
                None,
                vec![("2.0.0", "hawk".into(), inventory("2.0.0"))],
                &restricted
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn placement_survives_restart_and_rejects_unknown_accounts_and_machines() {
        placement_contract("sqlite::memory:").await;
    }

    #[tokio::test]
    #[ignore = "requires the owned isolated PostgreSQL fixture"]
    async fn postgres_usage_execution_placement_contract() {
        let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL fixture");
        placement_contract(&url).await;
    }

    async fn placement_contract(url: &str) {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::connect(url, temp.path().to_path_buf())
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let mut binding = super::super::tests::openai_usage_binding();
        binding.collector_argv = vec!["fixture-collector".into()];
        let service = UsageService::with_bindings(Some(store.clone()), None, vec![binding.clone()]);
        // The registry's built-in local Machine is an enrolled identity too.
        let machine = store
            .list_machines()
            .await
            .unwrap()
            .into_iter()
            .find(|machine| !machine.revoked)
            .unwrap()
            .id;
        service
            .set_execution_machine("openai", Some(machine.clone()))
            .await
            .unwrap();
        assert!(
            service
                .set_execution_machine("unknown", Some(machine.clone()))
                .await
                .is_err()
        );
        assert!(
            service
                .set_execution_machine("openai", Some("missing".into()))
                .await
                .is_err()
        );
        let restored = UsageService::with_bindings(Some(store.clone()), None, vec![binding]);
        restored.restore_execution_settings().await.unwrap();
        assert_eq!(
            restored.execution_machines.read().get("openai"),
            Some(&machine)
        );
        restored
            .set_execution_machine("openai", None)
            .await
            .unwrap();
        service.restore_execution_settings().await.unwrap();
        assert!(service.execution_machines.read().is_empty());
    }
}
