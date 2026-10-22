//! Real bounded-diff rejection, including a lost actual Apply reply. No mock
//! budget, timeout override, alternate writer or production state is involved.
use super::*;
use reqwest::{Method, StatusCode};
use std::fmt::Write as _;
use std::io::Write as _;

fn text(prefix: &str) -> String {
    let mut result = String::new();
    for n in 0..=1024 {
        writeln!(result, "{prefix}-{n}\nanchor-{n}").unwrap();
    }
    result
}

pub(super) fn seed(root: &Path) -> Result<()> {
    private_write(&root.join("workspace/budget.txt"), text("old").as_bytes())
}

pub(super) async fn exercise(
    pair: &Pair<'_>,
    stage: &mut &'static str,
    checks: &mut Vec<&'static str>,
) -> Result<(), Failure> {
    *stage = "budget_preparation";
    let original = text("old");
    let desired = text("new");
    let value = pair
        .http
        .post(
            "/api/code/buffers",
            json!({"sessionId":SESSION,"path":"budget.txt"}),
        )
        .await?;
    let resource = value["resourceId"]
        .as_str()
        .ok_or(Failure::WrongObservation)?;
    let path = exercise::endpoint(resource);
    check(
        pair.http
            .call(Method::PUT, &path, Some(json!({})))
            .await?
            .ok()?["state"]
            == "open",
    )?;
    exercise::reads(pair, resource, &original, false).await?;

    // Preserve metadata to isolate explicit sync from a watcher reload, as in
    // the existing success case. Actual changed bytes still cross native I/O.
    let file_path = pair.root.join("workspace/budget.txt");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&file_path)
        .map_err(|_| Failure::Setup)?;
    let modified = file
        .metadata()
        .and_then(|metadata| metadata.modified())
        .map_err(|_| Failure::Setup)?;
    check(original.len() == desired.len())?;
    file.write_all(desired.as_bytes())
        .and_then(|()| file.set_modified(modified))
        .map_err(|_| Failure::Setup)?;
    check(file.metadata().is_ok_and(|metadata| {
        metadata.len() == desired.len() as u64 && metadata.modified().ok() == Some(modified)
    }))?;
    let content = json!({"sha256":sha256(desired.as_bytes()),"utf8Bytes":desired.len()});
    let prepared = pair
        .http
        .post(
            &format!("{path}/synchronizations"),
            json!({"purpose":"refresh_from_disk","content":content}),
        )
        .await?;
    let operation = prepared["operationId"]
        .as_str()
        .ok_or(Failure::WrongObservation)?;
    check(prepared["resourceId"] == resource && prepared["state"] == json!({"kind":"prepared"}))?;
    let endpoint = synchronization::endpoint(operation);
    let before = pair.proxy.counts()?.commands;
    let count = |commands: &std::collections::BTreeMap<String, u32>, key: &str| {
        commands.get(key).copied().unwrap_or(0)
    };
    let gate = pair.proxy.hold("codeSyncApply")?;
    exercise::cancel(pair, &gate, Method::PUT, &endpoint, json!({})).await?;
    synchronization::fenced(pair, resource).await?;
    gate.discard();
    *stage = "budget_lost_apply_reply";
    let unknown = synchronization::settled(pair, operation, Method::PUT).await?;
    check(unknown["state"] == json!({"kind":"unknown"}))?;
    synchronization::fenced(pair, resource).await?;
    check(
        pair.http
            .call(Method::DELETE, &endpoint, Some(json!({})))
            .await?
            .status
            == StatusCode::CONFLICT,
    )?;
    *stage = "budget_original_id_query";
    let observed = pair.http.get(&endpoint).await?;
    check(
        observed
            == json!({
                "apiVersion":1,"operationId":operation,"resourceId":resource,
                "purpose":"refresh_from_disk","content":content,
                "state":{"kind":"refused","reason":"budget"},"pending":false
            }),
    )?;
    for method in [Method::PUT, Method::GET] {
        let value = pair
            .http
            .call(
                method.clone(),
                &endpoint,
                (method == Method::PUT).then(|| json!({})),
            )
            .await?
            .ok()?;
        check(value == observed)?;
    }
    let after = pair.proxy.counts()?.commands;
    check(count(&after, "codeSyncApply") == count(&before, "codeSyncApply") + 1)?;
    check(count(&after, "codeSyncQuery") == count(&before, "codeSyncQuery") + 1)?;
    check(count(&after, "codeSyncRetire") == count(&before, "codeSyncRetire"))?;
    exercise::reads(pair, resource, &original, false).await?;
    exercise::reads(pair, resource, &desired, true).await?;
    check(std::fs::read_to_string(&file_path).map_err(|_| Failure::Setup)? == desired)?;
    *stage = "budget_explicit_retirement";
    let retired = pair
        .http
        .call(Method::DELETE, &endpoint, Some(json!({})))
        .await?
        .ok()?;
    check(retired["state"] == json!({"kind":"retired"}))?;
    check(
        pair.http
            .call(Method::DELETE, &endpoint, Some(json!({})))
            .await?
            .ok()?
            == retired,
    )?;
    check(
        count(&pair.proxy.counts()?.commands, "codeSyncRetire")
            == count(&before, "codeSyncRetire") + 1,
    )?;
    check(
        pair.http
            .call(Method::DELETE, &path, Some(json!({})))
            .await?
            .ok()?["state"]
            == "released",
    )?;
    checks.push(
        "native_budget_refusal_lost_reply_original_query_unchanged_text_and_explicit_retirement",
    );
    Ok(())
}
