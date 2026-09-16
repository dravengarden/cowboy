//! Installation admission is part of Code acceptance, not a pre-seeded slot.
use super::*;
use reqwest::{Method, StatusCode};

pub(super) async fn run(pair: &Pair<'_>, request: &Value) -> Result<(), Failure> {
    let endpoint = format!("/api/machines/{MACHINE}/plugins/zed");
    let history = format!("{endpoint}/installation-operations");
    let before = pair.http.get(&history).await?;
    check(before["admission_enabled"] == true && before["operations"] == json!([]))?;
    let anonymous = Http::new(pair.address)?;
    anonymous
        .denied(Method::POST, &endpoint, Some(request.clone()))
        .await?;
    check(
        !pair
            .proxy
            .counts()?
            .commands
            .contains_key("installationStep"),
    )?;

    // Lose only the HTTP observer after the actual Machine has applied and
    // durably acknowledged the installation. The owned operation must settle
    // without that observer, and a duplicate ID must not install a second time.
    let gate = pair.proxy.hold("installationStep")?;
    {
        let call = pair
            .http
            .call(Method::POST, &endpoint, Some(request.clone()));
        tokio::pin!(call);
        tokio::select! {
            held = gate.held() => held?,
            _ = &mut call => return Err(Failure::WrongObservation),
        }
    }
    gate.release();
    tokio::time::timeout(DEADLINE, async {
        loop {
            let value = pair.http.get(&history).await?;
            let operations = value["operations"]
                .as_array()
                .ok_or(Failure::WrongObservation)?;
            check(operations.len() == 1)?;
            let operation = &operations[0];
            if operation["phase"] == "completed" {
                return check(operation["operation_id"] == request["operation_id"]);
            }
            check(operation["phase"] != "needs_attention")?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| Failure::Timeout)??;
    let duplicate = pair
        .http
        .call(Method::POST, &endpoint, Some(request.clone()))
        .await?;
    check(
        duplicate.status == StatusCode::CONFLICT
            && duplicate.value["operation"]["phase"] == "completed"
            && duplicate.value["execution_authorized"] == false,
    )?;
    let counts = pair.proxy.counts()?;
    check(
        counts.commands.get("installationStep") == Some(&1)
            && counts.commands.get("installationObservation") == Some(&1),
    )
}
