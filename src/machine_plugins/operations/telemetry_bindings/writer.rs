//! Finite local CAS, not a general executor or a Service authorization API.
//! Admission is closed in production. Fixtures enable only their own store.

use super::*;
use crate::machine_plugins::operations::lease::BindingExecutionLease;
use crate::machine_protocol::telemetry_binding::{
    BindingCommitFailure as WriteError, BindingCommitResult, BindingRejection,
};

impl From<BindingRejection> for WriteError {
    fn from(reason: BindingRejection) -> Self {
        Self::Rejected(reason)
    }
}

type WriteResult<T> = std::result::Result<T, WriteError>;

impl Ledger {
    fn precondition(&self, step: &BindingStep) -> WriteResult<()> {
        if self.current != step.expected {
            return Err(BindingRejection::TargetChanged.into());
        }
        if self.receipts.last().is_some_and(BindingReceipt::unresolved) {
            return Err(WriteError::Fenced);
        }
        let next_epoch = self
            .current
            .policy_epoch
            .next()
            .map_err(|_| WriteError::Rejected(BindingRejection::PolicyChanged))?;
        let after = step
            .after()
            .map_err(|_| WriteError::Unavailable(BindingUnavailable::InvalidRequest))?;
        // Epochs are issued here, not imported from a request. A caller can
        // name the exact next epoch but cannot jump, reuse or restore one.
        if after.policy_epoch != next_epoch {
            return Err(BindingRejection::PolicyChanged.into());
        }
        if let BindingChange::Restore {
            forward_request_digest,
            selection,
            ..
        } = &step.change
        {
            let valid = self.receipts.iter().any(|forward| {
                forward.request_digest == *forward_request_digest
                    && matches!(&forward.outcome, BindingOutcome::Applied { after } if *after == self.current)
                    && forward.step.expected.selection == *selection
            });
            if !valid {
                return Err(BindingRejection::TargetChanged.into());
            }
        }
        Ok(())
    }

    pub(super) fn encode(&self) -> WriteResult<Vec<u8>> {
        #[derive(serde::Serialize)]
        struct Evidence<'a> {
            ledger: &'a Ledger,
            evidence_digest: BindingDigest,
        }
        self.validate()
            .map_err(|_| WriteError::Unavailable(BindingUnavailable::InvalidRequest))?;
        let bytes = serde_json::to_vec(&Evidence {
            ledger: self,
            evidence_digest: binding_digest(&serde_json::to_vec(self).expect("bounded ledger")),
        })
        .expect("bounded binding evidence");
        if bytes.len() as u64 > MAX_BINDING_BYTES {
            return Err(WriteError::Capacity);
        }
        Ok(bytes)
    }

    fn finish(&mut self, outcome: BindingOutcome) {
        if let BindingOutcome::Applied { after } = &outcome {
            self.current = after.clone();
        }
        self.receipts.last_mut().expect("prepared intent").outcome = outcome;
    }
}

impl Bindings {
    fn commit(
        &self,
        step: &BindingStep,
        lease: &BindingExecutionLease,
        mut check_local: impl FnMut() -> std::result::Result<(), BindingRejection>,
    ) -> WriteResult<BindingObservation> {
        self.commit_with_io(step, lease, &mut check_local, |bytes| {
            atomic_write(&self.path, bytes, 0o600)?;
            fs::File::open(self.path.parent().context("binding journal parent")?)?.sync_all()?;
            Ok(())
        })
    }

    fn commit_with_io(
        &self,
        step: &BindingStep,
        lease: &BindingExecutionLease,
        check_local: &mut impl FnMut() -> std::result::Result<(), BindingRejection>,
        mut persist: impl FnMut(&[u8]) -> Result<()>,
    ) -> WriteResult<BindingObservation> {
        if !lease.matches(step) {
            return Err(WriteError::Unavailable(BindingUnavailable::InvalidRequest));
        }
        let mut state = self.state.lock();
        if !self.retained_current(&mut state) {
            return Err(WriteError::Unavailable(BindingUnavailable::Storage));
        }
        let observation = Self::lookup(&state, step);
        match &observation {
            BindingObservation::Unavailable { reason } => {
                return Err(WriteError::Unavailable(*reason));
            }
            BindingObservation::Observed { snapshot } if snapshot.receipt.is_some() => {
                // Prepared/Unknown also stop here. Repeating an ID never
                // resumes an interrupted transaction, even with a fresh lease.
                return Ok(observation);
            }
            BindingObservation::Observed { .. } => {}
        }
        if !state.writer {
            return Err(WriteError::ReaderOnly);
        }
        if step.expected_namespace.is_some_and(|expected| {
            (expected == BindingNamespace::Managed) != state.ledger.is_some()
        }) {
            return Err(BindingRejection::TargetChanged.into());
        }
        let mut prepared = state.ledger.clone().unwrap_or_else(|| Ledger {
            schema: 1,
            service_id: step.service_id.clone(),
            machine_id: step.machine_id.clone(),
            current: BindingSnapshot::initial(),
            receipts: Vec::new(),
            resolutions: Vec::new(),
        });
        prepared.precondition(step)?;
        if prepared.receipts.len() >= MAX_BINDING_RECORDS {
            return Err(WriteError::Capacity);
        }
        let mut check = || {
            lease.check()?;
            check_local()?;
            lease.check()
        };
        check()?;
        prepared.receipts.push(BindingReceipt {
            step: step.clone(),
            request_digest: step.request_digest().expect("validated lease"),
            outcome: BindingOutcome::Prepared {},
        });
        let intent = prepared.encode()?;
        // Reserve space for EVERY possible completion before writing intent.
        // Capacity may not leave an otherwise avoidable unresolved fence.
        let completions = completions(&prepared, step)?;
        check()?;
        if persist(&intent).is_err() {
            state.poisoned = true;
            return Err(WriteError::Unavailable(BindingUnavailable::Storage));
        }
        state.ledger = Some(prepared.clone());
        let outcome = match check() {
            Ok(()) => BindingOutcome::Applied {
                after: step.after().expect("validated lease"),
            },
            Err(reason) => BindingOutcome::Rejected { reason },
        };
        let (_, bytes) = completions
            .iter()
            .find(|(candidate, _)| *candidate == outcome)
            .expect("all finite completions reserved");
        if persist(bytes).is_err() {
            // A failed directory flush can mean that the applied replacement
            // is already visible. Never report the cached Prepared as fact.
            state.poisoned = true;
            return Err(WriteError::Unavailable(BindingUnavailable::Storage));
        }
        prepared.finish(outcome);
        state.ledger = Some(prepared);
        Ok(Self::lookup(&state, step))
    }
}

fn completions(
    prepared: &Ledger,
    step: &BindingStep,
) -> WriteResult<Vec<(BindingOutcome, Vec<u8>)>> {
    let outcomes = [
        BindingOutcome::Applied {
            after: step.after().expect("validated lease"),
        },
        BindingOutcome::Rejected {
            reason: BindingRejection::Expired,
        },
        BindingOutcome::Rejected {
            reason: BindingRejection::TargetChanged,
        },
        BindingOutcome::Rejected {
            reason: BindingRejection::PolicyChanged,
        },
        BindingOutcome::Rejected {
            reason: BindingRejection::AuthorizationEnded,
        },
    ];
    outcomes
        .into_iter()
        .map(|outcome| {
            let mut completed = prepared.clone();
            completed.finish(outcome.clone());
            let bytes = completed.encode()?;
            if (bytes.len() + crate::machine_protocol::telemetry_recovery::MAX_RECOVERY_BYTES + 128)
                as u64
                > MAX_BINDING_BYTES
            {
                return Err(WriteError::Capacity);
            }
            Ok((outcome, bytes))
        })
        .collect()
}

impl MachinePluginStore {
    /// Execute real target/policy checks but interrupt the second replacement.
    /// The resulting live owner is poisoned; a new owner must validate reopen.
    #[cfg(test)]
    pub(crate) async fn interrupt_binding_for_test(&self, step: &BindingStep) {
        let _lifecycle = self.lifecycle.lock().await;
        let scope = crate::machine_plugins::PluginExecutionScope::new(
            Some(&step.service_id),
            &step.machine_id,
        );
        let lease = scope.telemetry_binding(step).unwrap();
        let after = step.after().unwrap();
        let mut policy = None;
        let bindings = &self.operations.telemetry_bindings;
        let mut writes = 0;
        let result = bindings.commit_with_io(
            step,
            &lease,
            &mut || self.check_binding_target(&after, &mut policy),
            |bytes| {
                writes += 1;
                ensure!(
                    writes == 1,
                    "hermetic interruption before completion replacement"
                );
                atomic_write(&bindings.path, bytes, 0o600)?;
                fs::File::open(bindings.path.parent().unwrap())?.sync_all()?;
                Ok(())
            },
        );
        assert_eq!(writes, 2);
        assert_eq!(
            result,
            Err(WriteError::Unavailable(BindingUnavailable::Storage))
        );
        assert!(bindings.state.lock().poisoned);
    }

    pub(crate) async fn commit_telemetry_binding_command(
        &self,
        step: &BindingStep,
        lease: BindingExecutionLease,
    ) -> BindingCommitResult {
        if step.validate_commit().is_err() {
            return BindingCommitResult::Unavailable {
                failure: WriteError::Unavailable(BindingUnavailable::InvalidRequest),
            };
        }
        match self.commit_telemetry_binding(step, lease).await {
            Ok(observation) => BindingCommitResult::Observed { observation },
            Err(failure) => BindingCommitResult::Unavailable { failure },
        }
    }

    async fn commit_telemetry_binding(
        &self,
        step: &BindingStep,
        lease: BindingExecutionLease,
    ) -> WriteResult<BindingObservation> {
        let _lifecycle = tokio::time::timeout(lease.remaining()?, self.lifecycle.lock())
            .await
            .map_err(|_| WriteError::Rejected(BindingRejection::Expired))?;
        lease.check()?;
        if self.operations.state.lock().poisoned {
            return Err(WriteError::Unavailable(BindingUnavailable::Storage));
        }
        let after = step
            .after()
            .map_err(|_| WriteError::Unavailable(BindingUnavailable::InvalidRequest))?;
        let mut policy = None;
        self.operations.telemetry_bindings.commit(step, &lease, || {
            self.check_binding_target(&after, &mut policy)
        })
    }

    fn check_binding_target(
        &self,
        after: &BindingSnapshot,
        policy: &mut Option<crate::telemetry_plugin::PreparedPolicy>,
    ) -> std::result::Result<(), BindingRejection> {
        let Some(installation) = &after.selection else {
            // Revocation/restoration to absence must work even after a
            // Plugin or its private policy has been removed.
            return Ok(());
        };
        let policy_path = self
            .root
            .parent()
            .expect("Machine state directory")
            .join("telemetry.json");
        let selection = crate::telemetry_plugin::PluginSelection {
            plugin_id: installation.plugin_id.clone(),
            plugin_version: installation.plugin_version.clone(),
            generation_digest: installation.generation_digest.clone().into(),
        };
        let check_target = || -> Result<()> {
            self.operations.ensure_unfenced(&selection.plugin_id)?;
            let active = self.telemetry_inventory(&selection)?;
            ensure!(
                active.installation_revision.as_ref() == Some(&installation.installation_revision)
                    && active.contract_fingerprint
                        == String::from(installation.contract_fingerprint.clone()),
                "telemetry binding target changed"
            );
            self.telemetry_contract(&selection, &active)?;
            Ok(())
        };
        check_target().map_err(|_| BindingRejection::TargetChanged)?;
        match policy {
            Some(original) => {
                if !original.unchanged(&policy_path) {
                    return Err(BindingRejection::PolicyChanged);
                }
            }
            None => {
                *policy = Some(
                    crate::telemetry_plugin::prepare_policy(&policy_path, &selection)
                        .map_err(|_| BindingRejection::PolicyChanged)?,
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
