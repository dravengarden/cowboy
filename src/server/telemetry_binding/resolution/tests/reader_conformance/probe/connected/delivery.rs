//! Real process intake/queue/managed RPC/HTTP, with a private protocol receiver.
use super::*;
use crate::otlp::Signal;

mod receiver;
mod round;
mod wire;
pub(super) use receiver::Destination;
pub(super) use wire::Wire;

pub(super) async fn exercise(
    pair: &mut Pair<'_>,
    stage: &mut Stage,
    report: &mut DeliveryReport,
) -> Result<(), Failure> {
    let mut policies = Vec::new();
    *stage = Stage::ExportDelivery;
    round::run(pair, DeliveryStep::Unconfigured, report).await?;
    *stage = Stage::Confirmation;
    let selection = flows::plan(pair, pair.selection()).await?;
    flows::applied(pair, &selection).await?;
    round::run(pair, DeliveryStep::BindingOnly, report).await?;

    *stage = Stage::ExportActivation;
    policies.push(activate(pair, report, false).await?);
    *stage = Stage::ExportDelivery;
    for step in [
        DeliveryStep::Delivered,
        DeliveryStep::Partial,
        DeliveryStep::Unavailable,
        DeliveryStep::RateLimited,
        DeliveryStep::Redirect,
        DeliveryStep::LostAck,
    ] {
        round::run(pair, step, report).await?;
    }
    let next_connection = pair.proxy.counts()?.connections + 1;
    round::run(pair, DeliveryStep::Disconnected, report).await?;
    *stage = Stage::ConnectionReplacement;
    pair.connected(next_connection).await?;
    *stage = Stage::ExportDelivery;
    round::run(pair, DeliveryStep::Reconnected, report).await?;

    *stage = Stage::ExportRevocation;
    let revoke = flows::plan(pair, json!({"action":"revoke"})).await?;
    flows::applied(pair, &revoke).await?;
    round::run(pair, DeliveryStep::Revoked, report).await?;
    let restore = flows::plan(
        pair,
        json!({"action":"restore","operation_id":flows::id(&revoke)?}),
    )
    .await?;
    flows::applied(pair, &restore).await?;
    check(
        restore["result_head"]["revision"] == "3"
            && restore["result_head"]["policy_epoch"] == "3"
            && restore["result_head"]["selection"] == selection["result_head"]["selection"],
    )?;
    round::run(pair, DeliveryStep::RestoredStalePolicy, report).await?;

    *stage = Stage::Reopen;
    pair.background.as_mut().ok_or(Failure::Setup)?.active = false;
    reopen_without_replay(pair, true).await?;
    round::run(pair, DeliveryStep::ReopenedStalePolicy, report).await?;

    *stage = Stage::ExportActivation;
    policies.push(activate(pair, report, false).await?);
    round::run(pair, DeliveryStep::Reactivated, report).await?;
    *stage = Stage::Reopen;
    reopen_without_replay(pair, true).await?;
    round::run(pair, DeliveryStep::ReopenedActive, report).await?;
    *stage = Stage::ExportActivation;
    policies.push(activate(pair, report, true).await?);
    round::run(pair, DeliveryStep::MetricsOnly, report).await?;
    for (path, expected) in policies {
        check(
            super::super::admission::policy_snapshot(&path)
                .map_err(|_| Failure::EvidenceChanged)?
                == Some(expected),
        )?;
    }
    let counts = pair.proxy.counts()?;
    let expected_exports: usize = report.rounds.iter().map(|r| r.export_commands_added).sum();
    check(
        report.rounds.len() == 16
            && report.rounds.iter().all(|r| r.accepted)
            && counts.binding_commands == 3
            && counts.recovery_commands == 0
            && counts.export_commands as usize == expected_exports
            && counts.export_receipts == counts.export_commands
            && counts.dropped_export_acks == 2
            && counts.forced_disconnects == 1
            && counts.connections == 7
            && counts.runtime_configurations == 7,
    )
}

async fn activate(
    pair: &mut Pair<'_>,
    report: &mut DeliveryReport,
    metrics_only: bool,
) -> Result<(PathBuf, super::super::admission::PolicySnapshot), Failure> {
    let before = pair.evidence()?;
    let ledger = before.ledger().map_err(|_| Failure::EvidenceChanged)?;
    let path = pair.root.join(format!(
        "background-policy-{}.json",
        report.background_policy_sha256.len()
    ));
    let bytes = serde_json::to_vec(&json!({
        "schema":1, "service_id":SERVICE, "machine_id":MACHINE, "binding":ledger.current,
        "signals":{"logs":!metrics_only,"metrics":true,"traces":!metrics_only}, "startup":"activate_exact_binding"
    })).map_err(|_| Failure::Setup)?;
    private_write(&path, &bytes).map_err(|_| Failure::Setup)?;
    let snapshot = super::super::admission::policy_snapshot(&path)
        .map_err(|_| Failure::Setup)?
        .ok_or(Failure::Setup)?;
    report.background_policy_sha256.push(sha256(&bytes));
    pair.background = Some(Background {
        path: path.clone(),
        active: true,
    });
    before.matches(pair.root)?;
    reopen_without_replay(pair, false).await?;
    Ok((path, snapshot))
}

async fn reopen_without_replay(pair: &mut Pair<'_>, machine: bool) -> Result<(), Failure> {
    let destination = pair.destination.as_ref().ok_or(Failure::Setup)?;
    let records = destination.records()?;
    let exports = pair.proxy.export_snapshot();
    pair.reopen(machine).await?;
    // New runtime generation initialization is not old queue/file replay.
    tokio::time::sleep(Duration::from_millis(200)).await;
    check(
        pair.destination.as_ref().ok_or(Failure::Setup)?.records()? == records
            && pair.proxy.export_snapshot() == exports
            && pair.http.get("/api/metrics").await?["observability_accepted_batches"] == 0,
    )
}
