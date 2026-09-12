//! Independently confirmed resolution. The transport has NO dispatch method.
use super::*;
use crate::machine_control::{ConnectionToken, MachineControl};
use crate::server::{ProductRequestAuth, operator_approval::TelemetryResolutionAuthority};
use crate::telemetry_binding::{
    Ledger,
    resolution::{ResolutionAction, ResolutionIntent, ResolutionPermit},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const RESOLUTION_WRITE_ADMISSION: bool = false;

trait Journal: Sync {
    fn read(
        &self,
        service: &str,
    ) -> impl std::future::Future<Output = Result<Option<Ledger>>> + Send;
    fn resolve(
        &self,
        permit: &ResolutionPermit,
        current: &(dyn Fn() -> bool + Sync),
    ) -> impl std::future::Future<Output = Result<Operation>> + Send;
}

impl Journal for Store {
    async fn read(&self, service: &str) -> Result<Option<Ledger>> {
        self.telemetry_binding_ledger(service).await
    }
    async fn resolve(
        &self,
        permit: &ResolutionPermit,
        current: &(dyn Fn() -> bool + Sync),
    ) -> Result<Operation> {
        Ok(self
            .change_telemetry_binding(&Change::Resolve(permit), current)
            .await?
            .operation)
    }
}

trait Observation: Sync {
    fn current(&self) -> bool;
    fn observe(
        &self,
        step: &BindingStep,
    ) -> impl std::future::Future<Output = Result<BindingObservation>> + Send;
}

struct LiveObservation {
    control: Arc<MachineControl>,
    connection: ConnectionToken,
    ended: AtomicBool,
}

impl LiveObservation {
    fn capture(control: Arc<MachineControl>, machine: &str) -> Result<Self> {
        let connection = control
            .operation_connection(machine)
            .map_err(|_| anyhow::anyhow!("resolution Machine is not connected"))?;
        Ok(Self {
            control,
            connection,
            ended: AtomicBool::new(false),
        })
    }
}

impl Observation for LiveObservation {
    fn current(&self) -> bool {
        let current =
            !self.ended.load(Ordering::Acquire) && self.control.is_current(&self.connection);
        if !current {
            self.ended.store(true, Ordering::Release);
        }
        current
    }
    async fn observe(&self, step: &BindingStep) -> Result<BindingObservation> {
        ensure!(self.current(), "binding resolution connection ended");
        // Bookkeeping does not select/install a Plugin or adopt a private policy.
        let observation = self
            .control
            .telemetry_binding_observation(&self.connection, step)
            .await
            .map_err(|_| anyhow::anyhow!("binding resolution observation unavailable"))?;
        ensure!(self.current(), "binding resolution connection ended");
        Ok(observation)
    }
}

async fn resolve(
    store: &Store,
    intent: &ResolutionIntent,
    authority: TelemetryResolutionAuthority,
    auth: ProductRequestAuth<'_>,
    observation: Option<&impl Observation>,
) -> Result<Operation> {
    ensure!(
        RESOLUTION_WRITE_ADMISSION,
        "telemetry binding resolution admission is closed"
    );
    coordinate(store, intent, authority, auth, observation).await
}

fn recorded(ledger: &Ledger, intent: &ResolutionIntent) -> Result<Option<Operation>> {
    if let Some(record) = ledger.resolutions.iter().find(|record| {
        record.intent.resolution_id == intent.resolution_id
            || record.intent.operation_id == intent.operation_id
    }) {
        ensure!(
            &record.intent == intent,
            "binding resolution identity conflict"
        );
        return Ok(Some(Operation {
            intent: record.before.intent.clone(),
            progress: record.after.clone(),
        }));
    }
    Ok(None)
}

async fn coordinate(
    store: &impl Journal,
    intent: &ResolutionIntent,
    authority: TelemetryResolutionAuthority,
    auth: ProductRequestAuth<'_>,
    observer: Option<&impl Observation>,
) -> Result<Operation> {
    ensure!(
        authority.check(auth, intent).await,
        "binding resolution authorization ended"
    );
    let ledger = store
        .read(&intent.service_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing binding journal"))?;
    // Historical results are reads, never a second query or a restored grant.
    if let Some(operation) = recorded(&ledger, intent)? {
        return Ok(operation);
    }
    let before = ledger
        .operations
        .last()
        .ok_or_else(|| anyhow::anyhow!("missing binding operation"))?;
    intent.check_before(before)?;
    let observer = if matches!(intent.action, ResolutionAction::AbortBeforeDispatch) {
        None // Local Prepared CAS must work without a Machine, including offline.
    } else {
        Some(observer.ok_or_else(|| anyhow::anyhow!("missing fresh binding observation channel"))?)
    };
    let current = || observer.is_none_or(Observation::current);
    ensure!(
        current() && authority.check(auth, intent).await,
        "binding resolution admission ended"
    );
    let observation = match observer {
        Some(observer) => Some(
            tokio::time::timeout(
                authority.remaining(),
                observer.observe(&before.intent.machine_step()?),
            )
            .await
            .map_err(|_| anyhow::anyhow!("binding resolution query budget ended"))??,
        ),
        None => None,
    };
    ensure!(current(), "binding resolution connection ended");
    let permit = authority
        .into_permit(auth, intent.clone(), before.clone(), observation)
        .await?;
    match store.resolve(&permit, &current).await {
        Ok(operation) => Ok(operation),
        Err(error) => {
            // An ambiguous local commit permits one local evidence read only.
            // Never retry the write, refresh the channel, or query the Machine again.
            if let Ok(Some(ledger)) = store.read(&intent.service_id).await
                && let Some(operation) = recorded(&ledger, intent)?
            {
                return Ok(operation);
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "machine-host"))]
pub(super) async fn resolve_fixture(
    store: &Store,
    intent: &ResolutionIntent,
    auth: ProductRequestAuth<'_>,
    control: Option<Arc<MachineControl>>,
) -> Result<Operation> {
    let observer = control
        .map(|control| LiveObservation::capture(control, &intent.machine_id))
        .transpose()?;
    coordinate(
        store,
        intent,
        tests::authority(auth, intent),
        auth,
        observer.as_ref(),
    )
    .await
}
