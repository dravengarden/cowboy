use super::*;
use crate::telemetry_binding::{Ledger, Progress};
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};

async fn request(
    client: &Client,
    base: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> Result<(StatusCode, Value), Failure> {
    let mut request = client
        .request(method, format!("{base}{path}"))
        .header(reqwest::header::ORIGIN, base);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| Failure::WrongObservation)?;
    if response
        .headers()
        .get(reqwest::header::CACHE_CONTROL)
        .and_then(|h| h.to_str().ok())
        != Some("no-store")
    {
        return Err(Failure::WrongObservation);
    }
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Failure::WrongObservation)?
    {
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err(Failure::WrongObservation);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((
        status,
        serde_json::from_slice(&bytes).map_err(|_| Failure::WrongObservation)?,
    ))
}

fn document(root: &Path) -> Result<String, Failure> {
    let db = rusqlite::Connection::open_with_flags(
        root.join("controller/store.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| Failure::EvidenceChanged)?;
    let (document, checksum): (String, String) = db.query_row("SELECT document, document_sha256 FROM telemetry_binding_journal WHERE slot='telemetry'", [], |row| Ok((row.get(0)?, row.get(1)?))).map_err(|_| Failure::EvidenceChanged)?;
    if sha256(document.as_bytes()) != checksum {
        return Err(Failure::EvidenceChanged);
    }
    Ok(document)
}

pub(super) async fn exercise(
    address: std::net::SocketAddr,
    policy: PolicyCase,
    state: &mut State,
    root: &Path,
    cold_read: u8,
) -> Result<(), Failure> {
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(1))
        .build()
        .map_err(|_| Failure::Setup)?;
    let base = format!("http://{address}");
    let route = format!(
        "/api/telemetry/binding/operations/{}",
        state.evidence.step.operation_id
    );
    let allowed = policy.admits(4);
    let (status, view) =
        request(&client, &base, Method::GET, "/api/telemetry/binding", None).await?;
    if status != StatusCode::OK
        || view["resolution_admission"] != if allowed { "open" } else { "closed" }
    {
        return Err(Failure::WrongObservation);
    }
    unchanged(root, &state.evidence).map_err(|_| Failure::EvidenceChanged)?;
    if cold_read > 1 {
        // The second start rejects a never-submitted preview from the first;
        // the third rejects the separate confirmation consumed on the second.
        let (status, value) = request(
            &client,
            &base,
            Method::POST,
            &format!("{route}/resolve"),
            state.confirmation.clone(),
        )
        .await?;
        // A foreign Machine can pass Service-only admission but cannot revive a plan.
        let expected = if policy.admits(4) || policy == PolicyCase::ForeignMachine {
            "preview_or_evidence_changed"
        } else {
            "resolution_admission_closed"
        };
        if status != StatusCode::CONFLICT || value["error"] != expected {
            return Err(Failure::WrongObservation);
        }
        unchanged(root, &state.evidence).map_err(|_| Failure::EvidenceChanged)?;
    }
    if cold_read <= 2 {
        let (status, plan) = request(
            &client,
            &base,
            Method::POST,
            &format!("{route}/resolution-plan"),
            Some(json!({})),
        )
        .await?;
        if status != StatusCode::OK
            || plan["confirmation_available"] != allowed
            || plan["action"] != "abort_before_dispatch"
            || plan["result_phase"] != "aborted"
        {
            return Err(Failure::WrongObservation);
        }
        unchanged(root, &state.evidence).map_err(|_| Failure::EvidenceChanged)?;
        state.confirmation = Some(json!({"plan_id":plan["plan_id"],"action":plan["action"]}));
        if cold_read == 2 {
            let (status, receipt) = request(
                &client,
                &base,
                Method::POST,
                &format!("{route}/resolve"),
                state.confirmation.clone(),
            )
            .await?;
            if allowed {
                accept_resolution(state, root, &plan, status, receipt)?;
            } else if status != StatusCode::CONFLICT
                || receipt["error"] != "resolution_admission_closed"
            {
                return Err(Failure::WrongObservation);
            }
        }
    }
    let (status, receipt) = request(
        &client,
        &base,
        Method::GET,
        &format!("{route}/resolution"),
        None,
    )
    .await?;
    if let Some(expected) = &state.receipt {
        if status != StatusCode::OK || &receipt != expected {
            return Err(Failure::WrongObservation);
        }
    } else if status != StatusCode::NOT_FOUND || receipt["error"] != "not_found" {
        return Err(Failure::WrongObservation);
    }
    unchanged(root, &state.evidence).map_err(|_| Failure::EvidenceChanged)
}

fn accept_resolution(
    state: &mut State,
    root: &Path,
    plan: &Value,
    status: StatusCode,
    receipt: Value,
) -> Result<(), Failure> {
    if status != StatusCode::OK
        || receipt["resolution_id"] != plan["plan_id"]
        || receipt["operation_id"] != state.evidence.step.operation_id
        || receipt["machine_id"] != MACHINE
        || receipt["action"] != "abort_before_dispatch"
        || receipt["phase"] != "aborted"
        || receipt["operation_digest"] != plan["operation"]["operation_digest"]
    {
        return Err(Failure::WrongObservation);
    }
    let before = Ledger::decode(
        state.evidence.document.as_deref().ok_or(Failure::Setup)?,
        SERVICE,
    )
    .map_err(|_| Failure::Setup)?;
    let bytes = document(root)?;
    let after = Ledger::decode(&bytes, SERVICE).map_err(|_| Failure::EvidenceChanged)?;
    if after.operations.len() != 1
        || after.operations[0].intent != before.operations[0].intent
        || after.operations[0].progress != Progress::Aborted
        || after.current != before.current
        || after.resolutions.len() != 1
        || after.resolutions[0].intent.resolution_id != plan["plan_id"]
    {
        return Err(Failure::EvidenceChanged);
    }
    state.evidence.document = Some(bytes);
    state.receipt = Some(receipt);
    Ok(())
}
