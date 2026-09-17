//! Real authenticated finite synchronization through all four supplied layers.
use super::*;
use reqwest::{Method, StatusCode};

const ORIGINAL: &str = "old🙂buffer\n";
const DESIRED: &str = "new🙂synchronized buffer\n";

pub(super) struct Prepared {
    resource: String,
    operation: String,
}

pub(super) fn endpoint(operation: &str) -> String {
    format!("/api/code/buffer-synchronizations/{operation}")
}

fn request() -> Value {
    json!({"purpose":"refresh_from_disk","content":{
        "sha256":sha256(DESIRED.as_bytes()),"utf8Bytes":DESIRED.len()
    }})
}

fn snapshot(value: &Value, resource: &str, state: &str) -> Result<String, Failure> {
    eprintln!(
        "Code synchronization observation: expected={state}, actual={:?}, reason={:?}, pending={:?}",
        value["state"]["kind"].as_str(),
        value["state"]["reason"].as_str(),
        value["pending"].as_bool(),
    );
    let id = value["operationId"]
        .as_str()
        .ok_or(Failure::WrongObservation)?;
    check(id.starts_with("sync-") && id.len() == 54)?;
    check(
        value.as_object().is_some_and(|value| value.len() == 7)
            && value["apiVersion"] == 1
            && value["resourceId"] == resource
            && value["purpose"] == "refresh_from_disk"
            && value["content"] == request()["content"]
            && value["state"]["kind"] == state
            && value["pending"] == false,
    )?;
    Ok(id.to_owned())
}

async fn buffer(pair: &Pair<'_>) -> Result<String, Failure> {
    let prepared = pair
        .http
        .post(
            "/api/code/buffers",
            json!({"sessionId":SESSION,"path":"sync.txt"}),
        )
        .await?;
    let id = prepared["resourceId"]
        .as_str()
        .ok_or(Failure::WrongObservation)?
        .to_owned();
    let opened = pair
        .http
        .call(Method::PUT, &exercise::endpoint(&id), Some(json!({})))
        .await?
        .ok()?;
    check(opened["resourceId"] == id && opened["state"] == "open")?;
    Ok(id)
}

async fn fenced(pair: &Pair<'_>, resource: &str) -> Result<(), Failure> {
    let commands = pair.proxy.counts()?.commands;
    let read = pair
        .http
        .call(
            Method::POST,
            &format!("{}/read", exercise::endpoint(resource)),
            Some(json!({"kind":"language"})),
        )
        .await?;
    let release = pair
        .http
        .call(
            Method::DELETE,
            &exercise::endpoint(resource),
            Some(json!({})),
        )
        .await?;
    check(read.status == StatusCode::CONFLICT && release.status == StatusCode::CONFLICT)?;
    check(pair.proxy.counts()?.commands == commands)
}

pub(super) async fn prepare(pair: &Pair<'_>) -> Result<Prepared, Failure> {
    std::fs::write(pair.root.join("workspace/sync.txt"), ORIGINAL).map_err(|_| Failure::Setup)?;
    let resource = buffer(pair).await?;
    let peer = buffer(pair).await?;
    exercise::reads(pair, &resource, ORIGINAL, false).await?;
    std::fs::write(pair.root.join("workspace/sync.txt"), DESIRED).map_err(|_| Failure::Setup)?;
    let path = format!("{}/synchronizations", exercise::endpoint(&resource));
    Http::new(pair.address)?
        .denied(Method::POST, &path, Some(request()))
        .await?;
    let shared = pair.http.call(Method::POST, &path, Some(request())).await?;
    check(shared.status == StatusCode::BAD_GATEWAY)?;
    check(!pair.proxy.counts()?.commands.contains_key("codeSyncApply"))?;
    exercise::reads(pair, &resource, ORIGINAL, false).await?;
    let released = pair
        .http
        .call(Method::DELETE, &exercise::endpoint(&peer), Some(json!({})))
        .await?
        .ok()?;
    check(released["state"] == "released")?;
    let value = pair.http.post(&path, request()).await?;
    let operation = snapshot(&value, &resource, "prepared")?;
    Http::new(pair.address)?
        .denied(Method::PUT, &endpoint(&operation), Some(json!({})))
        .await?;
    fenced(pair, &resource).await?;
    Ok(Prepared {
        resource,
        operation,
    })
}

async fn settled(pair: &Pair<'_>, operation: &str, method: Method) -> Result<Value, Failure> {
    // Deliberately covers the product's actual 40-second transport timeout.
    // The relay drops one real reply; neither a synthetic ACK nor test timeout
    // override can turn the uncertain operation into acceptance.
    tokio::time::timeout(Duration::from_secs(50), async {
        loop {
            let value = pair
                .http
                .call(
                    method.clone(),
                    &endpoint(operation),
                    (method != Method::GET).then(|| json!({})),
                )
                .await?;
            if value.status == StatusCode::OK {
                return Ok(value.value);
            }
            check(value.status == StatusCode::ACCEPTED && value.value["pending"] == true)?;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| Failure::Timeout)?
}

pub(super) async fn finish(
    pair: &Pair<'_>,
    prepared: Prepared,
    stage: &mut &'static str,
) -> Result<String, Failure> {
    let Prepared {
        resource,
        operation,
    } = prepared;
    let gate = pair.proxy.hold("codeSyncApply")?;
    exercise::cancel(pair, &gate, Method::PUT, &endpoint(&operation), json!({})).await?;
    fenced(pair, &resource).await?;
    gate.discard();
    *stage = "synchronization_lost_apply_reply";
    let unknown = settled(pair, &operation, Method::PUT).await?;
    snapshot(&unknown, &resource, "unknown")?;
    fenced(pair, &resource).await?;
    check(pair.proxy.counts()?.commands.get("codeSyncApply") == Some(&1))?;
    *stage = "synchronization_original_id_query";
    let observed = pair.http.get(&endpoint(&operation)).await?;
    snapshot(&observed, &resource, "applied")?;
    check(
        observed["state"]["content"] == request()["content"]
            && observed["state"]["version"].is_array(),
    )?;
    check(pair.proxy.counts()?.commands.get("codeSyncQuery") == Some(&1))?;
    let duplicate = pair
        .http
        .call(Method::PUT, &endpoint(&operation), Some(json!({})))
        .await?
        .ok()?;
    check(duplicate == observed)?;
    check(pair.proxy.counts()?.commands.get("codeSyncApply") == Some(&1))?;
    *stage = "synchronization_exact_content";
    exercise::reads(pair, &resource, DESIRED, false).await?;
    exercise::reads(pair, &resource, ORIGINAL, true).await?;

    *stage = "synchronization_retirement";
    let gate = pair.proxy.hold("codeSyncRetire")?;
    exercise::cancel(
        pair,
        &gate,
        Method::DELETE,
        &endpoint(&operation),
        json!({}),
    )
    .await?;
    let pending = pair
        .http
        .call(Method::DELETE, &endpoint(&operation), Some(json!({})))
        .await?;
    check(pending.status == StatusCode::ACCEPTED)?;
    gate.release();
    snapshot(
        &settled(pair, &operation, Method::GET).await?,
        &resource,
        "retired",
    )?;
    let duplicate = pair
        .http
        .call(Method::DELETE, &endpoint(&operation), Some(json!({})))
        .await?
        .ok()?;
    snapshot(&duplicate, &resource, "retired")?;
    check(pair.proxy.counts()?.commands.get("codeSyncRetire") == Some(&1))?;

    // Keep a distinct inert continuation for real connection/restart refusal.
    // Its original resource/process remains owned after uninstall too.
    *stage = "synchronization_next_inert_operation";
    let value = pair
        .http
        .post(
            &format!("{}/synchronizations", exercise::endpoint(&resource)),
            request(),
        )
        .await?;
    let next = snapshot(&value, &resource, "prepared")?;
    check(next != operation)?;
    Ok(next)
}

pub(super) async fn replacement_refused(pair: &Pair<'_>, operation: &str) -> Result<(), Failure> {
    let commands = pair.proxy.counts()?.commands;
    for method in [Method::PUT, Method::GET, Method::DELETE] {
        let value = pair
            .http
            .call(
                method.clone(),
                &endpoint(operation),
                (method != Method::GET).then(|| json!({})),
            )
            .await?;
        check(value.status == StatusCode::CONFLICT)?;
    }
    check(pair.proxy.counts()?.commands == commands)
}
