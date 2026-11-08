//! Revocation of an account's Machine browsers: every placement in the plan
//! is told to delete its profile and hibernate; the Machine's hibernated
//! answer is the evidence recorded under the owner lock. Never completion of
//! the whole account.

use super::super::super::*;
use crate::machine_protocol::MACHINE_BROWSER_PROTOCOL_VERSION;
use crate::machine_protocol::machine_browser::{Command, State as BrowserState};
use crate::store::provider_accounts::revocation::browser_revocations::Evidence;
use anyhow::Result;
use serde_json::{Value, json};

const REVOKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Machine-side operation id for this revocation: bounded, derived from the
/// owner's operation so the Machine's receipt replays on retry.
pub(in crate::server) fn machine_operation_id(operation: &str) -> String {
    format!(
        "revoke-{}",
        &crate::admin::hex_sha256(operation.as_bytes())[..32]
    )
}

pub(in crate::server) async fn run(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
) -> Result<Vec<Value>> {
    let store = state.store.as_ref().context("store unavailable")?;
    let receipt = store
        .provider_account_receipt(owner, operation)
        .await?
        .context("revocation operation missing")?;
    let plan = receipt.revocation.context("revocation plan missing")?;
    let mut results = Vec::new();
    for target in &plan.browsers {
        if let Some(saved) = receipt.browser_revocations.get(&target.machine_id) {
            results
                .push(json!({"machine": target.machine_id, "state": "revoked", "evidence": saved}));
            continue;
        }
        let outcome = revoke_one(
            state,
            owner,
            operation,
            &receipt.account_id,
            &target.machine_id,
            &target.instance_id,
        )
        .await;
        results.push(match outcome {
            Ok(evidence) => {
                json!({"machine": target.machine_id, "state": "revoked", "evidence": evidence})
            }
            Err(error) => {
                tracing::info!(%error, machine = %target.machine_id, "browser revocation pending");
                json!({"machine": target.machine_id, "state": "pending", "evidence": null})
            }
        });
    }
    Ok(results)
}

async fn revoke_one(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
    account: &crate::provider_account::ProviderAccountId,
    machine_id: &str,
    instance_id: &str,
) -> Result<Evidence> {
    let store = state.store.as_ref().context("store unavailable")?;
    let (connection, protocol) = state
        .machine_control
        .connection_protocol(machine_id)
        .context("machine offline")?;
    anyhow::ensure!(
        protocol >= MACHINE_BROWSER_PROTOCOL_VERSION,
        "machine protocol predates browsers"
    );
    let machine_operation = machine_operation_id(operation);
    let event = state
        .machine_control
        .browser_command(
            &connection,
            super::request_id("browser-revoke"),
            Command::Revoke {
                account_id: account.clone(),
                operation_id: machine_operation.clone(),
            },
            REVOKE_TIMEOUT,
        )
        .await
        .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        event.state == BrowserState::Hibernated
            && !matches!(
                event.reason,
                Some(
                    crate::machine_protocol::machine_browser::Reason::Busy
                        | crate::machine_protocol::machine_browser::Reason::Failed
                )
            ),
        "browser revocation unconfirmed"
    );
    let evidence = Evidence {
        machine_id: machine_id.to_owned(),
        instance_id: instance_id.to_owned(),
        operation_id: machine_operation,
        profile_revision: event.profile_revision.unwrap_or(0),
        observed_at_ms: event.observed_at_ms,
    };
    store
        .record_provider_browser_revocation(owner, operation, &evidence, || {
            state.machine_control.is_current(&connection)
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_operation_ids_fit_the_wire_bound_and_replay() {
        let id = machine_operation_id("http-revoke-abc");
        assert_eq!(id, machine_operation_id("http-revoke-abc"));
        assert_ne!(id, machine_operation_id("http-revoke-abd"));
        assert!(id.len() <= 128);
        assert!(
            id.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        );
    }
}
