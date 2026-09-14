use super::*;
use crate::plugin_operation::installation::InstallOperation;
use reqwest::{Method, StatusCode};

pub(super) async fn run(
    prepared: &exercise::Prepared,
    controller: &Artifact,
    machine: &Artifact,
    report: &mut Check,
) -> Result<(), Failure> {
    report.stage = Stage::Copy;
    let root = tempfile::tempdir().map_err(|_| Failure::Setup)?;
    copy::stopped_fixture(prepared.root.path(), root.path())?;
    check(Evidence::read(root.path())? == prepared.evidence)?;
    let mut pair = Pair::prepare(
        root.path(),
        &prepared.fixture,
        controller,
        machine,
        report.flow,
        Some(&prepared.http),
    )
    .await?;
    let result = async {
        report.stage = Stage::ReaderStart;
        pair.start(false).await?;
        let normalized = Evidence::read(root.path())?;
        normalized.verify(&prepared.fixture, report.flow, true)?;
        normalized.recovered_from(&prepared.evidence, report.flow)?;
        report.normalized_service_sha256 = Some(normalized.hashes()?.0);
        for cold_read in 1..=2 {
            if cold_read == 2 {
                report.stage = Stage::Reopen;
                pair.reopen().await?;
            }
            report.stage = Stage::History;
            let admitted = history(&pair.http, &normalized, report.flow).await?;
            report.stage = Stage::Duplicate;
            duplicates(&pair, &normalized, admitted).await?;
            check(Evidence::read(root.path())? == normalized)?;
            read_only(&pair.proxy.counts()?, cold_read)?;
            report.cold_reads = cold_read;
        }
        Ok(())
    }
    .await;
    if result.is_ok() {
        report.stage = Stage::Cleanup;
    }
    let cleaned = pair.finish().await;
    report.reader_wire = pair.proxy.snapshot();
    result.and(cleaned)?;
    read_only(&pair.proxy.counts()?, 2)?;
    report.stage = Stage::Complete;
    Ok(())
}

fn read_only(counts: &WireCounts, cold_read: u8) -> Result<(), Failure> {
    check(
        *counts
            == WireCounts {
                connections: u32::from(cold_read),
                runtime_configurations: u32::from(cold_read),
                ..WireCounts::default()
            },
    )
}

fn projection(op: &InstallOperation) -> Value {
    json!({
        "evidence_schema": op.intent.schema,
        "operation_id": op.intent.operation_id,
        "plugin_kind": op.intent.plugin_kind,
        "plugin_version": op.intent.plugin_version,
        "generation_digest": op.intent.generation_digest,
        "phase": op.phase,
        "problem": op.problem,
        "attention_from": op.attention_from,
        "created_at_ms": op.created_at_ms,
        "updated_at_ms": op.updated_at_ms,
        "machine_receipt": op.machine_receipt.as_ref().map(|r| &r.outcome),
    })
}

async fn history(http: &Http, evidence: &Evidence, flow: Flow) -> Result<bool, Failure> {
    let report = http
        .get(&format!("{}/installation-operations", endpoint()))
        .await?;
    let admission = report["admission_enabled"]
        .as_bool()
        .ok_or(Failure::WrongObservation)?;
    let rows = report["operations"]
        .as_array()
        .ok_or(Failure::WrongObservation)?;
    check(
        report
            == json!({
                "schema": "dravengarden.cowboy.plugin-install-history/v2",
                "admission_enabled": admission,
                "execution_authorized": false,
                "requires_reconciliation": flow != Flow::InstallAndReinstall,
                "operations": rows,
            }),
    )?;
    let operations = evidence.operations()?;
    check(rows.len() == operations.len())?;
    for op in operations {
        check(rows.iter().filter(|r| **r == projection(&op)).count() == 1)?;
    }
    Ok(admission)
}

async fn duplicates(pair: &Pair<'_>, evidence: &Evidence, admitted: bool) -> Result<(), Failure> {
    for op in evidence.operations()? {
        let body = pair.request(&op.intent.operation_id);
        let same = pair
            .http
            .call(Method::POST, &endpoint(), Some(body.clone()))
            .await?;
        if admitted {
            check(
                same.status == StatusCode::CONFLICT
                    && same.value["execution_authorized"] == false
                    && same.value["operation"] == projection(&op),
            )?;
        } else {
            check(same.status == StatusCode::SERVICE_UNAVAILABLE)?;
        }
        // Reusing an identity with another release must not refresh its target,
        // actor, deadline or authority, even on a writer-capable Controller.
        let mut changed = body;
        changed["version"] = json!("2.0.0");
        changed["digest"] = json!(format!("sha256:{}", "f".repeat(64)));
        let different = pair
            .http
            .call(Method::POST, &endpoint(), Some(changed))
            .await?;
        check(
            different.status
                == if admitted {
                    StatusCode::CONFLICT
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                },
        )?;
    }
    Ok(())
}
