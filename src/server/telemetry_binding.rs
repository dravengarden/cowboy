//! Finite Service coordinator and connection-bound protocol-15 transport.
//! Admission and HTTP mutations remain closed pending reader floors/leases.
#![cfg_attr(not(test), allow(dead_code))]

use crate::machine_protocol::telemetry_binding::{BindingObservation, BindingOutcome, BindingStep};
use crate::store::Store;
use crate::telemetry_binding::{
    Attention, Intent, LegacyFence, Operation, Progress, writer::Change,
};
use anyhow::{Result, ensure};

mod export;
mod http;
#[cfg_attr(not(feature = "machine-host"), allow(dead_code))]
mod live;
pub(super) mod recovery;
pub(super) mod resolution;

struct Confirmation<'a> {
    authority: &'a super::operator_approval::TelemetryBindingAuthority,
    auth: super::ProductRequestAuth<'a>,
}

impl Confirmation<'_> {
    fn within_budget(&self, effects: &impl Effects) -> bool {
        let valid = self.authority.within_budget() && effects.within_budget();
        if !valid {
            self.authority.revoke();
        }
        valid
    }

    async fn authorized(&self, intent: &Intent, effects: &impl Effects) -> bool {
        let valid = self.within_budget(effects)
            && self
                .authority
                .check(self.auth, &intent.service_id, intent)
                .await
            && effects.authorized(intent).await
            && self.within_budget(effects);
        if !valid {
            self.authority.revoke();
        }
        valid
    }
}
trait Effects: Sync {
    /// Recheck the exact trusted release and original connection. The core
    /// Confirmation independently checks actual Operator authority above.
    fn authorized(&self, intent: &Intent) -> impl std::future::Future<Output = bool> + Send;
    fn within_budget(&self) -> bool;
    fn dispatch(
        &self,
        step: &BindingStep,
    ) -> impl std::future::Future<Output = Result<BindingObservation>> + Send;
    /// Observation is not authorization. This always addresses the original
    /// complete request; a reconnect cannot turn it into another dispatch.
    fn observe(
        &self,
        step: &BindingStep,
    ) -> impl std::future::Future<Output = Result<BindingObservation>> + Send;
}

async fn advance(store: &Store, expected: &Operation, progress: Progress) -> Result<Operation> {
    Ok(store
        .change_telemetry_binding(&Change::Advance { expected, progress }, &|| true)
        .await?
        .operation)
}

fn conclusion(intent: &Intent, observation: BindingObservation) -> Progress {
    let Ok(step) = intent.machine_step() else {
        return Progress::NeedsAttention {
            reason: Attention::InvalidEvidence,
            observation: None,
        };
    };
    if !observation.matches(&step) {
        return Progress::NeedsAttention {
            reason: Attention::InvalidEvidence,
            observation: None,
        };
    }
    if let BindingObservation::Observed { snapshot } = &observation
        && !snapshot.unresolved
    {
        match snapshot.receipt.as_ref().map(|receipt| &receipt.outcome) {
            Some(BindingOutcome::Applied { after }) if snapshot.current.as_ref() == Some(after) => {
                return Progress::Completed { observation };
            }
            Some(BindingOutcome::Rejected { .. })
                if snapshot.current.as_ref() == Some(&step.expected) =>
            {
                return Progress::Rejected { observation };
            }
            Some(BindingOutcome::Applied { .. } | BindingOutcome::Rejected { .. }) => {
                return Progress::NeedsAttention {
                    reason: Attention::HeadChanged,
                    observation: Some(observation),
                };
            }
            _ => {}
        }
    }
    Progress::NeedsAttention {
        reason: Attention::Uncertain,
        observation: Some(observation),
    }
}

async fn coordinate(
    store: &Store,
    fence: &LegacyFence,
    intent: &Intent,
    confirmation: Confirmation<'_>,
    effects: &impl Effects,
) -> Result<Operation> {
    let step = intent.machine_step()?;
    // Validate owner, capacity, restoration and slot state in memory before
    // observing another Machine or closing legacy admission. The transaction
    // repeats this validation under its writer lock; this is not the CAS itself.
    let mut preview = store.telemetry_binding_ledger(&intent.service_id).await?;
    let proposed = crate::telemetry_binding::writer::apply(&mut preview, &Change::Begin(intent))?;
    if !proposed.admitted {
        return Ok(proposed.operation);
    }
    ensure!(
        confirmation.authorized(intent, effects).await,
        "binding confirmation ended"
    );
    let before = effects.observe(&step).await?;
    ensure!(before.matches(&step), "invalid binding preflight");
    ensure!(
        matches!(&before, BindingObservation::Observed { snapshot } if snapshot.current == intent.expected && !snapshot.unresolved && snapshot.receipt.is_none()),
        "Machine binding changed before admission"
    );
    ensure!(
        confirmation.authorized(intent, effects).await,
        "binding confirmation ended during preflight"
    );
    // Even an uncertain INSERT closes admission in this process. Reopen may
    // observe absence, but it can never dispatch an old operation automatically.
    fence.close();
    let prepared = store
        .change_telemetry_binding(&Change::Begin(intent), &|| {
            confirmation.within_budget(effects)
        })
        .await?;
    if !prepared.admitted {
        return Ok(prepared.operation);
    }
    let prepared = prepared.operation;
    if !confirmation.authorized(intent, effects).await {
        return advance(store, &prepared, Progress::Aborted).await;
    }
    let dispatching = store
        .change_telemetry_binding(
            &Change::Advance {
                expected: &prepared,
                progress: Progress::Dispatching,
            },
            &|| confirmation.within_budget(effects),
        )
        .await?
        .operation;
    if !confirmation.authorized(intent, effects).await {
        return advance(
            store,
            &dispatching,
            Progress::NeedsAttention {
                reason: Attention::AuthorizationEnded,
                observation: None,
            },
        )
        .await;
    }
    // Exactly one dispatch attempt. An error (including lost ACK) is followed
    // by one read-only query, not another operation ID or inverse command.
    let observed = match effects.dispatch(&step).await {
        Ok(observation) => Ok(observation),
        Err(_) => effects.observe(&step).await,
    };
    let progress = match observed {
        Ok(observation) => conclusion(intent, observation),
        Err(_) => Progress::NeedsAttention {
            reason: Attention::Uncertain,
            observation: None,
        },
    };
    let observation = match &progress {
        Progress::Completed { observation } | Progress::Rejected { observation } => {
            observation.clone()
        }
        _ => return advance(store, &dispatching, progress).await,
    };
    if !confirmation.authorized(intent, effects).await {
        return advance(
            store,
            &dispatching,
            Progress::NeedsAttention {
                reason: Attention::AuthorizationEnded,
                observation: Some(observation),
            },
        )
        .await;
    }
    let change = Change::Advance {
        expected: &dispatching,
        progress: progress.clone(),
    };
    match store
        .change_telemetry_binding(&change, &|| confirmation.within_budget(effects))
        .await
    {
        Ok(updated) => Ok(updated.operation),
        Err(error) => {
            // A failed COMMIT response is not proof of rollback. Re-read the
            // local atomic head/receipt before retaining uncertainty. No RPC.
            let ledger = store.telemetry_binding_ledger(&intent.service_id).await?;
            let current = ledger
                .as_ref()
                .and_then(|l| l.operations.last())
                .ok_or(error)?;
            if current.intent == *intent && current.progress == progress {
                return Ok(current.clone());
            }
            advance(
                store,
                &dispatching,
                Progress::NeedsAttention {
                    reason: Attention::Uncertain,
                    observation: Some(observation),
                },
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests;
