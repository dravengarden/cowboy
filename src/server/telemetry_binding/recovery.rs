//! Machine-only recovery leaves Service NeedsAttention and its fence intact.
//! A separate Service resolution confirmation must adopt a definite result.
use super::*;
use crate::machine_control::{
    CommandFailure, CommandRequestError, ConnectionToken, MachineControl,
};
use crate::machine_protocol::telemetry_recovery::{RecoveryObservation, RecoveryRequest};
use crate::server::{ProductRequestAuth, operator_approval::TelemetryRecoveryAuthority};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const RECOVERY_WRITE_ADMISSION: bool = false;

pub(in crate::server) mod surface;

trait Effects: Sync {
    fn current(&self) -> bool;
    fn observe(
        &self,
        request: &RecoveryRequest,
    ) -> impl std::future::Future<Output = Result<RecoveryObservation, CommandRequestError>> + Send;
    fn recover(
        &self,
        request: &RecoveryRequest,
    ) -> impl std::future::Future<Output = Result<RecoveryObservation, CommandRequestError>> + Send;
}

struct Live {
    control: Arc<MachineControl>,
    connection: ConnectionToken,
    request: RecoveryRequest,
    ended: AtomicBool,
}

impl Live {
    fn bind(control: Arc<MachineControl>, request: &RecoveryRequest) -> Result<Self> {
        let connection = control
            .operation_connection(&request.step.machine_id)
            .map_err(|_| anyhow::anyhow!("binding recovery Machine is not connected"))?;
        let live = Self {
            control,
            connection,
            request: request.clone(),
            ended: AtomicBool::new(false),
        };
        ensure!(live.current(), "binding recovery connection is unavailable");
        Ok(live)
    }
    fn check(&self, request: &RecoveryRequest) -> Result<(), CommandRequestError> {
        if request == &self.request && self.current() {
            return Ok(());
        }
        self.ended.store(true, Ordering::Release);
        Err(CommandRequestError {
            certainty: CommandFailure::NotSent,
            detail: "binding recovery target changed".into(),
        })
    }
}

impl Effects for Live {
    fn current(&self) -> bool {
        let current = !self.ended.load(Ordering::Acquire)
            && self
                .control
                .telemetry_recovery_target_current(&self.connection, &self.request);
        if !current {
            self.ended.store(true, Ordering::Release);
        }
        current
    }
    async fn observe(
        &self,
        request: &RecoveryRequest,
    ) -> Result<RecoveryObservation, CommandRequestError> {
        self.check(request)?;
        self.control
            .telemetry_recovery_observation(&self.connection, request)
            .await
    }
    async fn recover(
        &self,
        request: &RecoveryRequest,
    ) -> Result<RecoveryObservation, CommandRequestError> {
        self.check(request)?;
        self.control
            .recover_telemetry_binding(&self.connection, request)
            .await
    }
}

async fn recover_machine(
    store: &Store,
    request: &RecoveryRequest,
    authority: TelemetryRecoveryAuthority,
    auth: ProductRequestAuth<'_>,
    control: Arc<MachineControl>,
) -> Result<RecoveryObservation> {
    ensure!(
        RECOVERY_WRITE_ADMISSION,
        "Machine binding recovery admission is closed"
    );
    let live = Live::bind(control, request)?;
    coordinate(store, request, authority, auth, &live).await
}

async fn coordinate(
    store: &Store,
    request: &RecoveryRequest,
    authority: TelemetryRecoveryAuthority,
    auth: ProductRequestAuth<'_>,
    effects: &impl Effects,
) -> Result<RecoveryObservation> {
    ensure!(
        effects.current() && authority.check(auth, store, request).await,
        "binding recovery authorization ended"
    );
    let observed = tokio::time::timeout(authority.remaining(), effects.observe(request))
        .await
        .map_err(|_| anyhow::anyhow!("binding recovery query budget ended"))?
        .map_err(|_| anyhow::anyhow!("binding recovery observation unavailable"))?;
    ensure!(
        observed.matches(request),
        "binding recovery evidence mismatch"
    );
    let RecoveryObservation::Observed { snapshot } = &observed else {
        anyhow::bail!("binding recovery evidence unavailable");
    };
    if snapshot.receipt.is_some() {
        ensure!(
            effects.current() && authority.check(auth, store, request).await,
            "binding recovery history authorization ended"
        );
        return Ok(observed); // Read-only history.
    }
    ensure!(
        request.expects(&snapshot.binding),
        "binding recovery is not an exact Prepared operation"
    );
    ensure!(
        effects.current() && authority.check(auth, store, request).await,
        "binding recovery authorization ended"
    );
    // Atomic Machine head/receipt/audit replacement is the only mutation. No
    // Service head, private policy, installation or export permission changes.
    let result = tokio::time::timeout(authority.remaining(), effects.recover(request)).await;
    let observed = match result {
        Ok(Ok(observed)) => observed,
        Ok(Err(CommandRequestError {
            certainty: CommandFailure::NotSent | CommandFailure::Rejected,
            ..
        })) => {
            anyhow::bail!("binding recovery was not admitted");
        }
        _ => {
            ensure!(
                effects.current() && authority.check(auth, store, request).await,
                "binding recovery needs a fresh observation"
            );
            // One query of this exact recovery, never resending either command.
            tokio::time::timeout(authority.remaining(), effects.observe(request))
                .await
                .map_err(|_| anyhow::anyhow!("binding recovery query budget ended"))?
                .map_err(|_| anyhow::anyhow!("binding recovery remains uncertain"))?
        }
    };
    ensure!(
        observed.matches(request),
        "binding recovery evidence mismatch"
    );
    ensure!(
        effects.current() && authority.check(auth, store, request).await,
        "binding recovery requires fresh confirmation before continuing"
    );
    // A missing audit never proves this resolution committed. Leave the
    // original Service operation fenced even when the exact audit is present.
    ensure!(
        matches!(&observed, RecoveryObservation::Observed { snapshot } if snapshot.receipt.is_some()),
        "binding recovery remains uncertain"
    );
    Ok(observed)
}

#[cfg(test)]
pub(in crate::server) fn request_fixture(
    before: &Operation,
    actor: &crate::plugin_operation::Actor,
) -> RecoveryRequest {
    use crate::machine_protocol::{
        telemetry_binding::binding_digest,
        telemetry_recovery::{fixture, prepared},
    };
    let step = before.intent.machine_step().unwrap();
    RecoveryRequest {
        actor: actor.into(),
        service_operation_digest: binding_digest(&serde_json::to_vec(before).unwrap()),
        expected_observation_digest: binding_digest(
            &serde_json::to_vec(&prepared(&step).unwrap()).unwrap(),
        ),
        step,
        ..fixture()
    }
}

#[cfg(all(test, feature = "machine-host"))]
pub(super) async fn recover_fixture(
    store: &Store,
    request: &RecoveryRequest,
    auth: ProductRequestAuth<'_>,
    control: Arc<MachineControl>,
) -> Result<RecoveryObservation> {
    let before = store
        .telemetry_binding_ledger(&request.step.service_id)
        .await?
        .unwrap()
        .operations
        .last()
        .unwrap()
        .clone();
    let verified = crate::server::AuthenticatedProductRequest {
        principal: crate::product_auth::local_product_principal(),
        cookie_session: None,
        device_identity: None,
    };
    let approval = crate::server::operator_approval::OperatorApproval::capture(
        auth,
        &request.step.service_id,
        Some(&verified),
        &axum::http::HeaderMap::new(),
    )
    .map_err(|_| anyhow::anyhow!("fixture Operator confirmation unavailable"))?;
    let authority = approval.bind_telemetry_recovery(request, &before)?;
    let live = Live::bind(control, request)?;
    coordinate(store, request, authority, auth, &live).await
}

#[cfg(test)]
mod tests;
