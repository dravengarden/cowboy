use super::*;
use crate::telemetry_binding::Progress;
use reqwest::Method;

const PLAN: &str = "/api/telemetry/binding/plan";
const CONFIRM: &str = "/api/telemetry/binding/confirm";

fn confirmation(plan: &Value) -> Value {
    json!({"plan_id":plan["plan_id"],"action":plan["action"]})
}
fn id(plan: &Value) -> Result<&str, Failure> {
    plan["plan_id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .ok_or(Failure::WrongObservation)
}
fn route(operation: &str, suffix: &str) -> String {
    format!("/api/telemetry/binding/operations/{operation}/{suffix}")
}

async fn plan(pair: &Pair<'_>, body: Value) -> Result<Value, Failure> {
    let before = pair.evidence()?;
    let value = pair.http.post(PLAN, body).await?;
    check(value["confirmation_available"] == true && value["operation"]["machine_id"] == MACHINE)?;
    id(&value)?;
    before.matches(pair.root)?;
    Ok(value)
}

async fn applied(pair: &Pair<'_>, plan: &Value) -> Result<Value, Failure> {
    let before = pair.evidence()?;
    let receipt = pair.http.post(CONFIRM, confirmation(plan)).await?;
    let after = pair.evidence()?;
    let ledger = after.ledger().map_err(|_| Failure::EvidenceChanged)?;
    let operation = ledger.operations.last().ok_or(Failure::EvidenceChanged)?;
    let old_count = before
        .service
        .as_ref()
        .map(|_| before.ledger().map(|l| l.operations.len()))
        .transpose()
        .map_err(|_| Failure::EvidenceChanged)?
        .unwrap_or(0);
    check(
        receipt["schema"] == 1
            && receipt["request_digest"] == plan["request_digest"]
            && receipt["operation"]["operation_id"] == id(plan)?
            && receipt["operation"]["phase"] == "completed"
            && ledger.operations.len() == old_count + 1
            && operation.intent.operation_id == id(plan)?
            && operation.intent.actor
                == (crate::plugin_operation::Actor::Product {
                    user_id: "c".repeat(32),
                })
            && matches!(operation.progress, Progress::Completed { .. })
            && serde_json::to_value(&ledger.current).map_err(|_| Failure::Setup)?
                == plan["result_head"]
            && serde_json::to_value(
                operation
                    .intent
                    .machine_step()
                    .map_err(|_| Failure::Setup)?
                    .request_digest()
                    .map_err(|_| Failure::Setup)?,
            )
            .map_err(|_| Failure::Setup)?
                == receipt["request_digest"]
            && after.machine.is_some()
            && after.machine != before.machine,
    )?;
    Ok(receipt)
}

async fn saved(pair: &Pair<'_>, operation: &str, expected: &Value) -> Result<(), Failure> {
    let before = pair.evidence()?;
    let counts = pair.proxy.counts()?;
    let value = pair.http.get(&route(operation, "receipt")).await?;
    check(&value == expected && pair.proxy.counts()? == counts)?;
    before.matches(pair.root)
}

pub(super) async fn exercise(
    pair: &mut Pair<'_>,
    flow: Flow,
    stage: &mut Stage,
) -> Result<(), Failure> {
    match flow {
        Flow::BindingRoundTrip => round_trip(pair, stage).await,
        Flow::BindingLostAck => lost_ack(pair, stage).await,
        Flow::BindingDisconnected => disconnected(pair, stage).await,
        Flow::PreparedRecovery => recovery(pair, stage).await,
    }
}

async fn round_trip(pair: &mut Pair<'_>, stage: &mut Stage) -> Result<(), Failure> {
    *stage = Stage::Preview;
    let untouched = pair.evidence()?;
    let abandoned = plan(pair, pair.selection()).await?;
    *stage = Stage::Authentication;
    let logout = pair.http.post("/api/auth/logout", json!({})).await?;
    check(logout["ok"] == true)?;
    pair.http
        .denied(Method::POST, CONFIRM, Some(confirmation(&abandoned)))
        .await?;
    pair.http
        .denied(Method::GET, "/api/telemetry/binding", None)
        .await?;
    untouched.matches(pair.root)?;
    pair.http.login(&pair.fixture.password).await?;
    *stage = Stage::Reopen;
    pair.reopen(false).await?;
    pair.http
        .conflict(
            CONFIRM,
            confirmation(&abandoned),
            "preview_or_evidence_changed",
        )
        .await?;
    untouched.matches(pair.root)?;
    *stage = Stage::ConnectionReplacement;
    let detached = plan(pair, pair.selection()).await?;
    let connections = pair.proxy.counts()?.connections;
    pair.proxy.disconnect();
    pair.connected(connections + 1).await?;
    pair.http
        .conflict(CONFIRM, confirmation(&detached), "outcome_unverified")
        .await?;
    untouched.matches(pair.root)?;
    check(pair.proxy.counts()?.binding_commands == 0)?;
    *stage = Stage::Confirmation;
    let selection = plan(pair, pair.selection()).await?;
    let selected = applied(pair, &selection).await?;
    let revoke = plan(pair, json!({"action":"revoke"})).await?;
    let revoked = applied(pair, &revoke).await?;
    let restore = plan(
        pair,
        json!({"action":"restore","operation_id":id(&revoke)?}),
    )
    .await?;
    let restored = applied(pair, &restore).await?;
    check(
        selection["result_head"]["revision"] == "1"
            && revoke["result_head"]["revision"] == "2"
            && restore["result_head"]["revision"] == "3"
            && restore["result_head"]["policy_epoch"] == "3"
            && restore["result_head"]["selection"] == selection["result_head"]["selection"]
            && revoke["result_head"]["selection"].is_null(),
    )?;
    *stage = Stage::Reopen;
    pair.reopen(true).await?;
    for (preview, receipt) in [
        (&selection, &selected),
        (&revoke, &revoked),
        (&restore, &restored),
    ] {
        saved(pair, id(preview)?, receipt).await?;
    }
    pair.http
        .conflict(
            CONFIRM,
            confirmation(&restore),
            "preview_or_evidence_changed",
        )
        .await?;
    plan(pair, json!({"action":"revoke"})).await?; // Actual reopened Machine head query only.
    let counts = pair.proxy.counts()?;
    check(
        counts.binding_commands == 3
            && counts.recovery_commands == 0
            && counts.forced_disconnects == 1,
    )
}

async fn lost_ack(pair: &mut Pair<'_>, stage: &mut Stage) -> Result<(), Failure> {
    *stage = Stage::Preview;
    let preview = plan(pair, pair.selection()).await?;
    let queries = pair.proxy.counts()?.binding_queries;
    *stage = Stage::Confirmation;
    let receipt = applied(pair, &preview).await?;
    let counts = pair.proxy.counts()?;
    check(
        counts.binding_commands == 1
            && counts.dropped_binding_acks == 1
            && counts.binding_queries == queries + 2
            && counts.recovery_commands == 0,
    )?;
    // The two reads are the coordinator's preflight and one post-timeout
    // observation. The exact 45s runtime deadline was not shortened for tests.
    *stage = Stage::Reopen;
    pair.reopen(true).await?;
    saved(pair, id(&preview)?, &receipt).await?;
    pair.http
        .conflict(
            CONFIRM,
            confirmation(&preview),
            "preview_or_evidence_changed",
        )
        .await?;
    plan(pair, json!({"action":"revoke"})).await?;
    check(pair.proxy.counts()?.binding_commands == 1)
}

async fn disconnected(pair: &mut Pair<'_>, stage: &mut Stage) -> Result<(), Failure> {
    *stage = Stage::Preview;
    let preview = plan(pair, pair.selection()).await?;
    *stage = Stage::Confirmation;
    let uncertain = pair.http.post(CONFIRM, confirmation(&preview)).await?;
    check(uncertain["operation"]["phase"] == "needs_attention")?;
    let pending = pair.evidence()?;
    let ledger = pending.ledger().map_err(|_| Failure::EvidenceChanged)?;
    check(ledger.current.is_none() && ledger.resolutions.is_empty() && pending.machine.is_some())?;
    *stage = Stage::ConnectionReplacement;
    pair.connected(2).await?;
    pending.matches(pair.root)?; // New connection cannot auto-adopt Applied.
    saved(pair, id(&preview)?, &uncertain).await?;
    *stage = Stage::Resolution;
    let resolved = resolve(pair, id(&preview)?, "accept_applied", "completed").await?;
    check(pair.evidence()?.machine == pending.machine)?;
    let completed = pair.http.get(&route(id(&preview)?, "receipt")).await?;
    check(
        completed["operation"]["phase"] == "completed"
            && completed["request_digest"] == preview["request_digest"],
    )?;
    *stage = Stage::Reopen;
    pair.reopen(true).await?;
    saved(pair, id(&preview)?, &completed).await?;
    let before = pair.evidence()?;
    check(pair.http.get(&route(id(&preview)?, "resolution")).await? == resolved)?;
    before.matches(pair.root)?;
    plan(pair, json!({"action":"revoke"})).await?;
    let counts = pair.proxy.counts()?;
    check(
        counts.binding_commands == 1
            && counts.recovery_commands == 0
            && counts.dropped_binding_acks == 1
            && counts.forced_disconnects == 1,
    )
}

async fn resolve(
    pair: &Pair<'_>,
    operation: &str,
    action: &str,
    phase: &str,
) -> Result<Value, Failure> {
    let before = pair.evidence()?;
    let preview = pair
        .http
        .post(&route(operation, "resolution-plan"), json!({}))
        .await?;
    check(preview["action"] == action && preview["confirmation_available"] == true)?;
    before.matches(pair.root)?;
    let receipt = pair
        .http
        .post(&route(operation, "resolve"), confirmation(&preview))
        .await?;
    let after = pair.evidence()?;
    let ledger = after.ledger().map_err(|_| Failure::EvidenceChanged)?;
    check(
        receipt["resolution_id"] == id(&preview)?
            && receipt["operation_id"] == operation
            && receipt["action"] == action
            && receipt["phase"] == phase
            && ledger.resolutions.len() == 1
            && ledger.resolutions[0].intent.resolution_id == id(&preview)?
            && after.machine == before.machine
            && after.service != before.service,
    )?;
    Ok(receipt)
}

async fn recovery(pair: &mut Pair<'_>, stage: &mut Stage) -> Result<(), Failure> {
    let operation = pair.fixture.reader.step.operation_id.clone();
    let before = pair.evidence()?;
    let queries = pair.proxy.counts()?.recovery_queries;
    *stage = Stage::Preview;
    let preview = pair
        .http
        .post(&route(&operation, "machine-recovery-plan"), json!({}))
        .await?;
    check(
        preview["confirmation_available"] == true
            && preview["action"] == "reject_interrupted_prepared",
    )?;
    before.matches(pair.root)?;
    *stage = Stage::Recovery;
    let receipt = pair
        .http
        .post(
            &route(&operation, "recover-machine"),
            confirmation(&preview),
        )
        .await?;
    let recovered = pair.evidence()?;
    check(
        receipt["resolution_id"] == id(&preview)?
            && receipt["operation_id"] == operation
            && recovered.service == before.service
            && recovered.machine != before.machine,
    )?;
    let counts = pair.proxy.counts()?;
    check(
        counts.recovery_commands == 1
            && counts.recovery_queries == queries + 3
            && counts.dropped_recovery_acks == 1
            && counts.binding_commands == 0,
    )?;
    *stage = Stage::Resolution;
    let resolution = resolve(pair, &operation, "record_rejected", "rejected").await?;
    let evidence = pair.evidence()?;
    let ledger = evidence.ledger().map_err(|_| Failure::EvidenceChanged)?;
    check(
        ledger.current.as_ref() == Some(&pair.fixture.reader.step.expected)
            && evidence.machine == recovered.machine,
    )?;
    *stage = Stage::Reopen;
    pair.reopen(true).await?;
    let audit = pair
        .http
        .get(&route(&operation, "machine-recovery-audit"))
        .await?;
    check(
        audit["recovery"]["receipt"] == receipt
            && audit["operation"]["phase"] == "rejected"
            && audit["recovery"]["before"]["phase"] == "needs_attention",
    )?;
    let history = pair
        .http
        .get(&route(
            &operation,
            &format!("machine-recoveries/{}", id(&preview)?),
        ))
        .await?;
    check(
        history == receipt && pair.http.get(&route(&operation, "resolution")).await? == resolution,
    )?;
    pair.http
        .conflict(
            &route(&operation, "recover-machine"),
            confirmation(&preview),
            "preview_or_evidence_changed",
        )
        .await?;
    evidence.matches(pair.root)?;
    let counts = pair.proxy.counts()?;
    check(counts.recovery_commands == 1 && counts.binding_commands == 0)
}
