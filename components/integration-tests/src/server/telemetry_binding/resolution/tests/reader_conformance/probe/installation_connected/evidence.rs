use super::*;
use crate::machine_protocol::plugin_install::{InstallOutcome, InstallReceipt, InstallTarget};
use crate::plugin_operation::installation::{
    InstallIntent, InstallOperation, InstallPhase, InstallProblem,
};
use std::collections::BTreeMap;
use std::io::Read as _;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct MachineEvidence {
    attempts: Option<BTreeMap<String, Vec<u8>>>,
    slots: Option<BTreeMap<String, Vec<u8>>>,
    active: Option<PathBuf>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Evidence {
    pub service: Value,
    pub machine: MachineEvidence,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    schema: u16,
    receipt: InstallReceipt,
    evidence_digest: String,
}

fn documents(path: &Path) -> Result<Option<BTreeMap<String, Vec<u8>>>> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(metadata.is_dir(), "fixture evidence directory required");
    let mut records = BTreeMap::new();
    for entry in std::fs::read_dir(path)? {
        ensure!(records.len() < 4, "unexpected fixture evidence capacity");
        let entry = entry?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(entry.path())?;
        ensure!(file.metadata()?.is_file(), "fixture evidence file required");
        let mut bytes = Vec::new();
        file.take(8193).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 8192, "oversized fixture evidence");
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("fixture evidence name"))?;
        ensure!(
            records.insert(name, bytes).is_none(),
            "duplicate evidence name"
        );
    }
    Ok(Some(records))
}

impl Evidence {
    pub fn read(root: &Path) -> Result<Self, Failure> {
        let machine = (|| -> Result<_> {
            let active = root.join("machine/plugins/victoria/active");
            let active = match active.symlink_metadata() {
                Ok(metadata) => {
                    ensure!(
                        metadata.file_type().is_symlink(),
                        "fixture active link required"
                    );
                    Some(std::fs::read_link(active)?)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            Ok(MachineEvidence {
                attempts: documents(&root.join("machine/plugin-operations/install-attempts-v1"))?,
                slots: documents(&root.join("machine/plugin-operations/installations-v1"))?,
                active,
            })
        })()
        .map_err(|_| Failure::EvidenceChanged)?;
        Ok(Self {
            service: super::super::installation::snapshot(root)?,
            machine,
        })
    }

    pub fn hashes(&self) -> Result<(String, String), Failure> {
        Ok((
            sha256(&serde_json::to_vec(&self.service).map_err(|_| Failure::EvidenceChanged)?),
            sha256(&serde_json::to_vec(&self.machine).map_err(|_| Failure::EvidenceChanged)?),
        ))
    }

    pub fn operations(&self) -> Result<Vec<InstallOperation>, Failure> {
        self.service
            .as_array()
            .ok_or(Failure::EvidenceChanged)?
            .iter()
            .map(operation)
            .collect::<Result<_, _>>()
            .map_err(|_| Failure::EvidenceChanged)
    }

    pub fn verify(
        &self,
        fixture: &InstallFixture,
        flow: Flow,
        reopened: bool,
    ) -> Result<(), Failure> {
        let operations = self.operations()?;
        check(operations.len() == flow.attempts())?;
        let mut revisions: Vec<
            crate::machine_protocol::installation_revision::InstallationRevision,
        > = Vec::new();
        for (index, op) in operations.iter().enumerate() {
            let step = op
                .intent
                .machine_step()
                .map_err(|_| Failure::EvidenceChanged)?;
            check(
                step.matches_envelope(&fixture.desired)
                    && step.service_id == SERVICE
                    && step.machine_id == MACHINE
                    && step.operation_id == if index == 0 { FIRST } else { SECOND }
                    && op.intent.actor
                        == crate::plugin_operation::Actor::Product {
                            user_id: "c".repeat(32),
                        },
            )?;
            if index == 0 {
                check(step.expected == InstallTarget::Vacant {})?;
            } else {
                check(
                    step.expected
                        == InstallTarget::Installed {
                            revision: revisions[0].clone(),
                            generation_digest: fixture.desired.release.artifact_digest.clone(),
                        },
                )?;
            }
            let expected_phase = if flow == Flow::InstallAndReinstall {
                InstallPhase::Completed
            } else if flow == Flow::ControllerCrashAfterApplied && !reopened {
                InstallPhase::Installing
            } else {
                InstallPhase::NeedsAttention
            };
            check(op.phase == expected_phase)?;
            if expected_phase == InstallPhase::NeedsAttention {
                check(
                    op.attention_from == Some(InstallPhase::Installing)
                        && op.problem
                            == Some(if flow == Flow::ControllerCrashAfterApplied {
                                InstallProblem::Interrupted
                            } else {
                                InstallProblem::UnknownMachineOutcome
                            }),
                )?;
            }
            if flow == Flow::DisconnectBeforeDelivery {
                check(op.machine_receipt.is_none() && self.machine.attempts.is_none())?;
                continue;
            }
            let bytes = self
                .machine
                .attempts
                .as_ref()
                .and_then(|r| r.get(&format!("{}.json", step.key().ok()?)))
                .ok_or(Failure::EvidenceChanged)?;
            let saved: Attempt =
                serde_json::from_slice(bytes).map_err(|_| Failure::EvidenceChanged)?;
            check(
                saved.schema == 1
                    && saved.receipt.matches(&step)
                    && saved.evidence_digest
                        == crate::machine_protocol::plugin_step::digest(
                            &serde_json::to_vec(&saved.receipt)
                                .map_err(|_| Failure::EvidenceChanged)?,
                        ),
            )?;
            let InstallOutcome::Applied { revision } = &saved.receipt.outcome else {
                return Err(Failure::EvidenceChanged);
            };
            check(!revisions.contains(revision))?;
            revisions.push(revision.clone());
            check(if flow == Flow::InstallAndReinstall {
                op.machine_receipt.as_ref() == Some(&saved.receipt)
            } else {
                op.machine_receipt.is_none()
            })?;
        }
        if flow == Flow::DisconnectBeforeDelivery {
            check(
                self.machine.active.is_none()
                    && self.machine.slots.as_ref().is_some_and(BTreeMap::is_empty),
            )
        } else {
            check(
                self.machine
                    .attempts
                    .as_ref()
                    .is_some_and(|r| r.len() == flow.attempts()),
            )?;
            let slots = self
                .machine
                .slots
                .as_ref()
                .ok_or(Failure::EvidenceChanged)?;
            check(slots.len() == 1)?;
            let slot: Value =
                serde_json::from_slice(slots.get("victoria.json").ok_or(Failure::EvidenceChanged)?)
                    .map_err(|_| Failure::EvidenceChanged)?;
            check(
                slot["transition"]["revision"] == json!(revisions.last())
                    && slot["transition"]["generation_digest"]
                        == fixture.desired.release.artifact_digest
                    && slot["transition"]["outcome"]["state"] == "stable"
                    && self.machine.active
                        == Some(PathBuf::from(format!(
                            "generations/{}",
                            fixture
                                .desired
                                .release
                                .artifact_digest
                                .strip_prefix("sha256:")
                                .ok_or(Failure::EvidenceChanged)?
                        ))),
            )
        }
    }

    /// Startup may fence the crashed Service attempt, but it cannot invent a
    /// receipt, replace an intent, or normalize any Machine evidence.
    pub fn recovered_from(&self, original: &Self, flow: Flow) -> Result<(), Failure> {
        check(self.machine == original.machine)?;
        if flow != Flow::ControllerCrashAfterApplied {
            return check(self == original);
        }
        let mut expected = original.service.clone();
        let rows = expected.as_array_mut().ok_or(Failure::EvidenceChanged)?;
        check(rows.len() == 1)?;
        rows[0]["phase"] = json!(InstallPhase::NeedsAttention);
        // Unlike phase/attention_from, the SQL problem column retains a JSON
        // document (including its string quotes), not an unquoted enum value.
        rows[0]["problem"] = json!(
            serde_json::to_string(&InstallProblem::Interrupted)
                .map_err(|_| Failure::EvidenceChanged)?
        );
        rows[0]["attention_from"] = json!(InstallPhase::Installing);
        check(
            self.service[0]["updated"]
                .as_i64()
                .zip(rows[0]["updated"].as_i64())
                .is_some_and(|(new, old)| new >= old),
        )?;
        rows[0]["updated"] = self.service[0]["updated"].clone();
        check(self.service == expected)
    }
}

fn operation(row: &Value) -> Result<InstallOperation> {
    let document = row["intent"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing fixture intent"))?;
    ensure!(
        sha256(document.as_bytes()) == row["checksum"],
        "fixture intent checksum"
    );
    let intent: InstallIntent = serde_json::from_str(document)?;
    let machine_receipt = if let Some(document) = row["machine_receipt"].as_str() {
        ensure!(
            sha256(document.as_bytes()) == row["machine_receipt_sha256"],
            "fixture receipt checksum"
        );
        Some(serde_json::from_str(document)?)
    } else {
        ensure!(
            row["machine_receipt"].is_null() && row["machine_receipt_sha256"].is_null(),
            "fixture receipt pair"
        );
        None
    };
    let op = InstallOperation {
        intent,
        phase: serde_json::from_value(row["phase"].clone())?,
        problem: match &row["problem"] {
            Value::Null => None,
            Value::String(document) => Some(serde_json::from_str(document)?),
            _ => anyhow::bail!("invalid fixture problem column"),
        },
        attention_from: serde_json::from_value(row["attention_from"].clone())?,
        created_at_ms: row["created"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("fixture created time"))?,
        updated_at_ms: row["updated"]
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("fixture updated time"))?,
        machine_receipt,
    };
    op.validate()?;
    Ok(op)
}

#[test]
fn installation_recovery_allows_only_the_crashed_service_fence() {
    let original = Evidence {
        service: json!([{"intent":"same immutable intent", "phase":"installing",
            "problem":null,"attention_from":null,"updated":1,"machine_receipt":null}]),
        machine: MachineEvidence {
            attempts: None,
            slots: None,
            active: None,
        },
    };
    let mut recovered = original.clone();
    recovered.service[0]["phase"] = json!("needs_attention");
    recovered.service[0]["problem"] = json!("\"interrupted\"");
    recovered.service[0]["attention_from"] = json!("installing");
    recovered.service[0]["updated"] = json!(2);
    assert!(
        recovered
            .recovered_from(&original, Flow::ControllerCrashAfterApplied)
            .is_ok()
    );
    assert!(
        recovered
            .recovered_from(&original, Flow::LostReceipt)
            .is_err()
    );
    for field in ["intent", "machine_receipt", "attention_from", "problem"] {
        let mut altered = recovered.clone();
        altered.service[0][field] = json!("changed");
        assert!(
            altered
                .recovered_from(&original, Flow::ControllerCrashAfterApplied)
                .is_err()
        );
    }
    recovered.machine.attempts = Some(BTreeMap::new());
    assert!(
        recovered
            .recovered_from(&original, Flow::ControllerCrashAfterApplied)
            .is_err()
    );
}

#[tokio::test]
async fn installation_evidence_preserves_the_actual_sql_problem_document() {
    let root = tempfile::tempdir().unwrap();
    let helper = super::super::super::manifest::ssh_keygen().unwrap();
    InstallFixture::seed(root.path(), &helper.path)
        .await
        .unwrap();
    let store = crate::store::Store::connect(
        &database(root.path()),
        root.path().join("controller/artifacts"),
    )
    .await
    .unwrap();
    let intent = crate::plugin_operation::installation::machine_fixture("connected-codec-fixture");
    store.begin_plugin_install(&intent).await.unwrap();
    store
        .advance_plugin_install(
            &intent,
            InstallPhase::Prepared,
            InstallPhase::Installing,
            None,
        )
        .await
        .unwrap();
    store
        .advance_plugin_install(
            &intent,
            InstallPhase::Installing,
            InstallPhase::NeedsAttention,
            Some(InstallProblem::UnknownMachineOutcome),
        )
        .await
        .unwrap();
    let evidence = Evidence::read(root.path()).unwrap();
    assert_eq!(
        evidence.service[0]["problem"],
        json!("\"unknown_machine_outcome\"")
    );
    let operations = evidence.operations().unwrap();
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].intent, intent);
    assert_eq!(
        operations[0].problem,
        Some(InstallProblem::UnknownMachineOutcome)
    );
    assert_eq!(operations[0].attention_from, Some(InstallPhase::Installing));
}
