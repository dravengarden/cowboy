use super::super::{Observer, authority, coordinate};
use super::*;
use crate::machine_plugins::{MachinePluginStore, PluginExecutionScope};
use crate::machine_protocol::{
    Platform,
    telemetry_binding::{
        BindingChange, BindingCommitResult, BindingObservation, BindingOutcome, BindingStep,
        binding_digest,
    },
    telemetry_recovery::{
        RecoveryAction, RecoveryActor, RecoveryObservation, RecoveryRequest, RecoveryResult,
        prepared,
    },
};
use crate::server::telemetry_binding::{advance, tests::FixtureEffects};
use crate::store::Store;
use crate::telemetry_binding::{
    Attention, Intent, Ledger, Progress,
    resolution::{ResolutionAction, ResolutionIntent},
    writer::Change,
};

pub(super) const SERVICE: &str = "svc-00000000000000000000000000000001";
pub(super) const MACHINE: &str = "telemetry-reader-fixture";
pub(super) const JOURNAL: &str = "plugin-operations/telemetry-bindings-v1.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Case {
    Absent,
    Completed,
    Prepared,
    Unknown,
    Recovered,
    Advanced,
    ChecksumCorrupt,
    AuditCorrupt,
}

impl Case {
    pub const ALL: [Self; 8] = [
        Self::Absent,
        Self::Completed,
        Self::Prepared,
        Self::Unknown,
        Self::Recovered,
        Self::Advanced,
        Self::ChecksumCorrupt,
        Self::AuditCorrupt,
    ];
    pub fn corrupt(self) -> bool {
        matches!(self, Self::ChecksumCorrupt | Self::AuditCorrupt)
    }
}

pub(super) struct Fixture {
    pub case: Case,
    pub document: Option<String>,
    pub machine: Option<Vec<u8>>,
    pub step: BindingStep,
    pub recovery: RecoveryRequest,
    pub binding: BindingObservation,
    pub audit: RecoveryObservation,
}

impl Fixture {
    pub async fn all() -> Result<Vec<Self>> {
        let mut fixtures = Vec::new();
        for case in Case::ALL {
            fixtures.push(Self::build(case).await?);
        }
        Ok(fixtures)
    }

    async fn build(case: Case) -> Result<Self> {
        let root = tempfile::tempdir()?;
        let state = root.path().join("machine");
        let open = || MachinePluginStore::new(&state, Platform::Linux, "x86_64".into());
        let store = Store::connect("sqlite::memory:", root.path().join("artifacts")).await?;
        store.migrate().await?;
        let intent = Intent {
            schema: 2,
            operation_id: "telemetry-reader-operation-1".into(),
            service_id: SERVICE.into(),
            machine_id: MACHINE.into(),
            actor: crate::plugin_operation::Actor::Product {
                user_id: crate::product_auth::local_product_principal().user_id,
            },
            expected: None,
            // Data-reader acceptance needs no endpoint, credential or installed
            // Plugin. A real Revoke writer advances both head and policy epoch.
            change: BindingChange::Revoke {
                policy_epoch: "1".to_owned().try_into().unwrap(),
            },
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 60_000,
        };
        let step = intent.machine_step()?;
        let owner = PluginExecutionScope::new(Some(SERVICE), MACHINE);
        let mut machine = open()?;
        if case != Case::Absent {
            let before = store
                .change_telemetry_binding(&Change::Begin(&intent), &|| true)
                .await?
                .operation;
            let before = advance(&store, &before, Progress::Dispatching).await?;
            machine.enable_binding_writer_for_test();
            if case == Case::Completed {
                let result = machine
                    .commit_telemetry_binding_command(
                        &step,
                        owner.telemetry_binding(&step).unwrap(),
                    )
                    .await;
                let BindingCommitResult::Observed { observation } = result else {
                    anyhow::bail!("fixture completion failed")
                };
                advance(&store, &before, Progress::Completed { observation }).await?;
            } else {
                machine.interrupt_binding_for_test(&step).await;
                drop(machine);
                if case == Case::Unknown {
                    let path = state.join(JOURNAL);
                    let bytes = MachinePluginStore::rewrite_binding_fixture(
                        &std::fs::read(&path)?,
                        |ledger| {
                            ledger["receipts"][0]["outcome"] =
                                serde_json::json!({"state":"unknown"});
                        },
                    );
                    std::fs::write(path, bytes)?;
                }
                machine = open()?;
                let observed = machine
                    .telemetry_binding_observation(&step, Some(SERVICE), MACHINE)
                    .await;
                advance(
                    &store,
                    &before,
                    Progress::NeedsAttention {
                        reason: Attention::Uncertain,
                        observation: Some(observed),
                    },
                )
                .await?;
            }
        }
        let before = store
            .telemetry_binding_ledger(SERVICE)
            .await?
            .and_then(|l| l.operations.last().cloned());
        let recovery = RecoveryRequest {
            schema: 1,
            resolution_id: "telemetry-reader-recovery-1".into(),
            actor: RecoveryActor::Product {
                user_id: crate::product_auth::local_product_principal().user_id,
            },
            service_operation_digest: binding_digest(&serde_json::to_vec(&before)?),
            action: RecoveryAction::RejectInterruptedPrepared,
            step: step.clone(),
            expected_observation_digest: binding_digest(&serde_json::to_vec(&prepared(&step)?)?),
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 60_000,
        };
        if matches!(
            case,
            Case::Recovered | Case::Advanced | Case::ChecksumCorrupt | Case::AuditCorrupt
        ) {
            machine.enable_binding_recovery_for_test();
            let result = machine
                .recover_telemetry_binding(&recovery, owner.telemetry_recovery(&recovery).unwrap())
                .await;
            ensure!(
                matches!(result, RecoveryResult::Observed { .. }),
                "fixture recovery failed"
            );
            let observation = machine
                .telemetry_binding_observation(&step, Some(SERVICE), MACHINE)
                .await;
            let request = ResolutionIntent::new(
                "telemetry-reader-resolution-1".into(),
                intent.actor.clone(),
                before.as_ref().unwrap(),
                ResolutionAction::RecordRejected {
                    observation_digest: binding_digest(&serde_json::to_vec(&observation)?),
                },
                chrono::Utc::now().timestamp_millis() + 60_000,
            )?;
            let local = FixtureEffects::new(&intent);
            let auth = local.confirmation().auth;
            coordinate(
                &store,
                &request,
                authority(auth, &request),
                auth,
                Some(&Observer::new(observation)),
            )
            .await?;
            if case == Case::Advanced {
                let mut next = intent.clone();
                next.operation_id = "telemetry-reader-operation-2".into();
                next.expected = Some(step.expected.clone());
                let next_step = next.machine_step()?;
                let pending = store
                    .change_telemetry_binding(&Change::Begin(&next), &|| true)
                    .await?
                    .operation;
                let pending = advance(&store, &pending, Progress::Dispatching).await?;
                machine.enable_binding_writer_for_test();
                let result = machine
                    .commit_telemetry_binding_command(
                        &next_step,
                        owner.telemetry_binding(&next_step).unwrap(),
                    )
                    .await;
                let BindingCommitResult::Observed { observation } = result else {
                    anyhow::bail!("fixture subsequent completion failed")
                };
                advance(&store, &pending, Progress::Completed { observation }).await?;
            }
        }
        let binding = machine
            .telemetry_binding_observation(&step, Some(SERVICE), MACHINE)
            .await;
        let audit = machine
            .telemetry_recovery_observation(&recovery, Some(SERVICE), MACHINE)
            .await;
        ensure!(
            binding.matches(&step) && audit.matches(&recovery),
            "invalid fixture observations"
        );
        let mut document = store
            .telemetry_binding_ledger(SERVICE)
            .await?
            .map(|l| l.encode(SERVICE))
            .transpose()?;
        let mut machine_bytes = if case == Case::Absent {
            None
        } else {
            Some(std::fs::read(state.join(JOURNAL))?)
        };
        if case == Case::AuditCorrupt {
            let mut json: serde_json::Value = serde_json::from_str(document.as_ref().unwrap())?;
            json["resolutions"][0]["resolved_at_ms"] = 0.into();
            let corrupted = serde_json::to_string(&json)?;
            ensure!(
                Ledger::decode(&corrupted, SERVICE).is_err(),
                "bad Service audit must fail structurally"
            );
            document = Some(corrupted);
            machine_bytes = Some(MachinePluginStore::rewrite_binding_fixture(
                machine_bytes.as_ref().unwrap(),
                |ledger| {
                    ledger["resolutions"][0]["resolved_at_ms"] = 0.into();
                },
            ));
        }
        if case == Case::ChecksumCorrupt {
            let mut json: serde_json::Value =
                serde_json::from_slice(machine_bytes.as_ref().unwrap())?;
            json["evidence_digest"] = serde_json::to_value(binding_digest(b"wrong checksum"))?;
            machine_bytes = Some(serde_json::to_vec(&json)?);
        }
        drop(machine);
        if case.corrupt() {
            std::fs::write(state.join(JOURNAL), machine_bytes.as_ref().unwrap())?;
            ensure!(open().is_err(), "bad Machine fixture must fail to open");
        }
        Ok(Self {
            case,
            document,
            machine: machine_bytes,
            step,
            recovery,
            binding,
            audit,
        })
    }
}

#[tokio::test]
async fn fixtures_cover_real_durable_writers_and_distinct_historical_current_heads() -> Result<()> {
    let fixtures = Fixture::all().await?;
    assert_eq!(fixtures.len(), Case::ALL.len());
    for fixture in fixtures {
        if fixture.case.corrupt() {
            continue;
        }
        let ledger = fixture
            .document
            .as_ref()
            .map(|doc| Ledger::decode(doc, SERVICE).unwrap());
        assert_eq!(ledger.is_none(), fixture.case == Case::Absent);
        if let Some(ledger) = ledger {
            assert_eq!(
                ledger.resolutions.len(),
                usize::from(matches!(fixture.case, Case::Recovered | Case::Advanced))
            );
        }
        let BindingObservation::Observed { snapshot } = fixture.binding else {
            panic!("fixture unavailable")
        };
        assert_eq!(
            snapshot.unresolved,
            matches!(fixture.case, Case::Prepared | Case::Unknown)
        );
        if fixture.case == Case::Advanced {
            assert_eq!(snapshot.current, Some(fixture.step.after()?));
            assert!(matches!(
                snapshot.receipt.unwrap().outcome,
                BindingOutcome::Rejected { .. }
            ));
        }
    }
    Ok(())
}
