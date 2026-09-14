use super::super::installation::{InstallCase, database_path, intent};
use super::*;
use crate::plugin_operation::installation::{InstallPhase, InstallProblem};

pub(in super::super) async fn run(
    artifact: &Artifact,
    empty: &Fixture,
    root: &Path,
    case: InstallCase,
) -> Result<serde_json::Value, Failure> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| Failure::Setup)?;
    let address = listener.local_addr().map_err(|_| Failure::Setup)?;
    drop(listener);
    let mut running = Running::spawn(&mut configured_command(artifact, root, address))?;
    let result = tokio::time::timeout(DEADLINE, async {
        if let Some(marker) = case.marker() {
            rejected_startup(&mut running, marker).await?;
        } else {
            controller(&mut running, address, empty).await?;
            observe(address, case).await?;
        }
        snapshot(root)
    })
    .await
    .unwrap_or(Err(Failure::Timeout));
    running.finish().await.and(result)
}

fn snapshot(root: &Path) -> Result<serde_json::Value, Failure> {
    let db = rusqlite::Connection::open_with_flags(
        database_path(root),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| Failure::EvidenceChanged)?;
    let mut statement = db.prepare("SELECT intent, intent_sha256, phase, problem, attention_from, created_at_ms, updated_at_ms, machine_receipt, machine_receipt_sha256 FROM plugin_install_operations")
        .map_err(|_| Failure::EvidenceChanged)?;
    let rows = statement.query_map([], |row| Ok(serde_json::json!({
        "intent": row.get::<_, String>(0)?, "checksum": row.get::<_, String>(1)?,
        "phase": row.get::<_, String>(2)?, "problem": row.get::<_, Option<String>>(3)?,
        "attention_from": row.get::<_, Option<String>>(4)?, "created": row.get::<_, i64>(5)?, "updated": row.get::<_, i64>(6)?,
        "machine_receipt": row.get::<_, Option<String>>(7)?, "machine_receipt_sha256": row.get::<_, Option<String>>(8)?,
    }))).map_err(|_| Failure::EvidenceChanged)?;
    Ok(serde_json::Value::Array(
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|_| Failure::EvidenceChanged)?,
    ))
}

async fn observe(address: std::net::SocketAddr, case: InstallCase) -> Result<(), Failure> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|_| Failure::Setup)?;
    let target = format!("http://{address}/api/machines/{MACHINE}/plugins/victoria");
    let response = client
        .get(format!("{target}/installation-operations"))
        .send()
        .await
        .map_err(|_| Failure::WrongObservation)?;
    if !response.status().is_success() {
        return Err(Failure::WrongObservation);
    }
    let report: serde_json::Value = response
        .json()
        .await
        .map_err(|_| Failure::WrongObservation)?;
    let phase = case.phase();
    let fenced = phase.is_some_and(|phase| !phase.terminal());
    if report["schema"] != "dravengarden.cowboy.plugin-install-history/v2"
        || report["requires_reconciliation"] != fenced
        || report["execution_authorized"] != false
        || !report["admission_enabled"].is_boolean()
        || report["operations"]
            .as_array()
            .is_none_or(|rows| rows.len() != usize::from(phase.is_some()))
    {
        return Err(Failure::WrongObservation);
    }
    let Some(phase) = phase else {
        return Ok(());
    };
    let expected_phase = if fenced {
        InstallPhase::NeedsAttention
    } else {
        phase
    };
    let expected_from = if phase == InstallPhase::NeedsAttention {
        Some(InstallPhase::Installing)
    } else {
        fenced.then_some(phase)
    };
    let problem = match phase {
        InstallPhase::AuthenticationPending => Some(InstallProblem::AuthenticationSyncFailed),
        InstallPhase::Aborted => Some(if matches!(case, InstallCase::MachineRejected) {
            InstallProblem::MachineRejected
        } else {
            InstallProblem::PreconditionsChanged
        }),
        InstallPhase::NeedsAttention => Some(InstallProblem::UnknownMachineOutcome),
        _ if fenced => Some(InstallProblem::Interrupted),
        _ => None,
    };
    let row = &report["operations"][0];
    if row["phase"] != serde_json::json!(expected_phase)
        || row["problem"] != serde_json::json!(problem)
        || row["attention_from"] != serde_json::json!(expected_from)
        || row["operation_id"] != intent(case).operation_id
        || row["evidence_schema"] != if case.durable() { 2 } else { 1 }
        || row["machine_receipt"] != serde_json::json!(case.outcome())
    {
        return Err(Failure::WrongObservation);
    }
    // This is a synthetic auth-off fixture, not production Operator authority.
    // Even an enabled writer may only observe this retained identity.
    let intent = intent(case);
    let response = client.post(target).header(reqwest::header::ORIGIN, format!("http://{address}")).json(&serde_json::json!({
        "operation_id": intent.operation_id, "version": intent.plugin_version, "digest": intent.generation_digest,
    })).send().await.map_err(|_| Failure::WrongObservation)?;
    if report["admission_enabled"] == false {
        if response.status() != reqwest::StatusCode::SERVICE_UNAVAILABLE {
            return Err(Failure::WrongObservation);
        }
    } else {
        if response.status() != reqwest::StatusCode::CONFLICT {
            return Err(Failure::WrongObservation);
        }
        let duplicate: serde_json::Value = response
            .json()
            .await
            .map_err(|_| Failure::WrongObservation)?;
        if duplicate["execution_authorized"] != false || &duplicate["operation"] != row {
            return Err(Failure::WrongObservation);
        }
    }
    Ok(())
}
