//! Real isolated database evidence is separate from protocol-receiver acceptance.
use super::*;
use crate::otlp::Signal;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DatabaseInputs {
    schema: u16,
    logs: PathBuf,
    metrics: PathBuf,
    traces: PathBuf,
}

#[derive(Serialize)]
pub(super) struct DatabaseArtifact {
    pub signal: Signal,
    pub executable: PathBuf,
    sha256: String,
}

impl DatabaseInputs {
    fn resolve(self) -> Result<Vec<DatabaseArtifact>> {
        ensure!(self.schema == 1, "unsupported database input schema");
        [
            (Signal::Logs, self.logs, "victoria-logs"),
            (Signal::Metrics, self.metrics, "victoria-metrics"),
            (Signal::Traces, self.traces, "victoria-traces"),
        ]
        .into_iter()
        .map(|(signal, executable, name)| {
            ensure!(
                executable.is_absolute()
                    && executable.starts_with("/nix/store")
                    && executable.components().count() == 6
                    && executable
                        .parent()
                        .and_then(Path::file_name)
                        .is_some_and(|file| file == "bin")
                    && executable.file_name().is_some_and(|file| file == name)
                    && executable.canonicalize()? == executable,
                "exact immutable database executable required"
            );
            ensure!(
                std::fs::metadata(&executable)?.len() <= 256 * 1024 * 1024,
                "bounded database ELF required"
            );
            let bytes = std::fs::read(&executable)?;
            ensure!(bytes.starts_with(b"\x7fELF"), "database ELF required");
            Ok(DatabaseArtifact {
                signal,
                executable,
                sha256: sha256(&bytes),
            })
        })
        .collect()
    }
}

#[derive(Default, Serialize)]
pub(super) struct DatabaseReport {
    pub empty_before_export: bool,
    pub queries: Vec<QueryRound>,
    pub database_restarts: u8,
    pub no_replay_after_host_restart: bool,
}

#[derive(Serialize)]
pub(super) struct QueryRound {
    pub after_database_restart: bool,
    pub log_records: usize,
    pub trace_spans: usize,
    pub metric_series: usize,
    pub normalized_result_sha256: String,
}

#[derive(Serialize)]
struct Check {
    controller_role: Role,
    machine_role: Role,
    accepted: bool,
    outcome: connected::Outcome,
}

#[derive(Serialize)]
struct Receipt {
    schema: u16,
    purpose: &'static str,
    source_revision: String,
    artifacts: Vec<Artifact>,
    databases: Vec<DatabaseArtifact>,
    ssh_keygen: manifest::Executable,
    checks: Vec<Check>,
    accepted: bool,
    not_checked: [&'static str; 5],
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "just telemetry-victoria-conformance; immutable inputs and isolated databases required"]
async fn immutable_victoria_telemetry() -> Result<()> {
    manifest::require_isolation()?;
    let matrix = std::env::var("COWBOY_TEST_TELEMETRY_VICTORIA_MATRIX")?;
    let inputs = std::env::var("COWBOY_TEST_TELEMETRY_VICTORIA_DATABASES")?;
    let path = PathBuf::from(std::env::var("COWBOY_TEST_TELEMETRY_VICTORIA_RECEIPT")?);
    ensure!(
        path.is_absolute() && path.symlink_metadata().is_err(),
        "new absolute receipt required"
    );
    let mut receipt = Receipt {
        schema: 1,
        purpose: "supplied_immutable_connected_victoria_databases",
        source_revision: manifest::clean_revision()?,
        artifacts: serde_json::from_slice::<Matrix>(&std::fs::read(matrix)?)?.resolve()?,
        databases: serde_json::from_slice::<DatabaseInputs>(&std::fs::read(inputs)?)?.resolve()?,
        ssh_keygen: manifest::ssh_keygen()?,
        checks: Vec::new(),
        accepted: false,
        not_checked: [
            "actual_host_roles_and_complete_production_configuration",
            "production_operator_credentials_and_postgres_startup",
            "production_destination_authentication_and_managed_cutover",
            "crash_power_loss_and_production_failure_restart_acceptance",
            "native_clients_provider_sessions_and_host_activation",
        ],
    };
    // Sequential database triples bound fixture resource consumption.
    for controller in receipt
        .artifacts
        .iter()
        .filter(|a| a.lane == Lane::Controller)
    {
        for machine in receipt.artifacts.iter().filter(|a| a.lane == Lane::Machine) {
            let outcome = probe::victoria_pair(
                controller,
                machine,
                &receipt.ssh_keygen.path,
                &receipt.databases,
            )
            .await;
            let accepted = outcome.failure.is_none()
                && outcome.victoria.as_ref().is_some_and(|report| {
                    report.empty_before_export
                        && report.database_restarts == 1
                        && report.no_replay_after_host_restart
                        && report.queries.len() == 2
                });
            eprintln!(
                "{:?}/{:?}: {:?}/{:?}",
                controller.role, machine.role, outcome.stage, outcome.failure
            );
            receipt.checks.push(Check {
                controller_role: controller.role,
                machine_role: machine.role,
                accepted,
                outcome,
            });
        }
    }
    receipt.accepted =
        receipt.checks.len() == 9 && receipt.checks.iter().all(|check| check.accepted);
    write_receipt(&path, &receipt)?;
    ensure!(
        receipt.accepted,
        "immutable Victoria database conformance failed"
    );
    Ok(())
}

#[test]
fn database_inputs_are_closed_and_cannot_select_live_endpoints_or_mutable_programs() {
    let valid = serde_json::json!({"schema":1,"logs":"/tmp/victoria-logs","metrics":"/tmp/victoria-metrics","traces":"/tmp/victoria-traces"});
    for field in ["logs", "metrics", "traces"] {
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<DatabaseInputs>(missing).is_err());
    }
    for field in ["endpoint", "token", "environment", "arguments", "state_dir"] {
        let mut extra = valid.clone();
        extra[field] = "forbidden".into();
        assert!(serde_json::from_value::<DatabaseInputs>(extra).is_err());
    }
    assert!(
        serde_json::from_value::<DatabaseInputs>(valid)
            .unwrap()
            .resolve()
            .is_err()
    );
}
