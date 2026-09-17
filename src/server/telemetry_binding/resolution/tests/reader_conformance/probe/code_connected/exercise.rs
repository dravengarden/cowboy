use super::*;
use reqwest::{Method, StatusCode};

fn target() -> Value {
    json!({"sessionId":SESSION,"path":"fixture.txt"})
}
pub(super) fn endpoint(id: &str) -> String {
    format!("/api/code/buffers/{id}")
}
fn content(text: &str, kind: &str) -> Value {
    json!({"kind":"content","content":{"sha256":sha256(text.as_bytes()),"utf8Bytes":text.len()},
        "query": if kind == "hover" { json!({"kind":kind,"position":{"row":0,"column":3}}) } else { json!({"kind":kind}) }})
}
fn snapshot(value: &Value, id: &str, state: &str, pending: bool) -> Result<(), Failure> {
    check(value == &json!({"apiVersion":1,"resourceId":id,"state":state,"pending":pending}))
}
async fn prepare(pair: &Pair<'_>) -> Result<String, Failure> {
    let value = pair.http.post("/api/code/buffers", target()).await?;
    let id = value["resourceId"]
        .as_str()
        .ok_or(Failure::WrongObservation)?
        .to_owned();
    check(id.len() == 49)?;
    snapshot(&value, &id, "prepared", false)?;
    Ok(id)
}
async fn operation(pair: &Pair<'_>, method: Method, id: &str, state: &str) -> Result<(), Failure> {
    let value = pair
        .http
        .call(method, &endpoint(id), Some(json!({})))
        .await?
        .ok()?;
    snapshot(&value, id, state, false)
}
async fn settled(pair: &Pair<'_>, id: &str, state: &str) -> Result<(), Failure> {
    tokio::time::timeout(DEADLINE, async {
        loop {
            let reply = pair.http.call(Method::GET, &endpoint(id), None).await?;
            if reply.status == StatusCode::OK {
                return snapshot(&reply.value, id, state, false);
            }
            check(reply.status == StatusCode::ACCEPTED && reply.value["pending"] == true)?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| Failure::Timeout)?
}
pub(super) async fn reads(
    pair: &Pair<'_>,
    id: &str,
    text: &str,
    mismatch: bool,
) -> Result<(), Failure> {
    for kind in ["language", "symbols", "hover"] {
        let body = content(text, kind);
        let value = pair
            .http
            .post(&format!("{}/read", endpoint(id)), body.clone())
            .await?;
        check(
            value["apiVersion"] == 1
                && value["resourceId"] == id
                && value["openedVersion"].is_array(),
        )?;
        check(value.as_object().is_some_and(|o| o.len() == 4))?;
        let result = &value["result"];
        check(result["kind"] == "content" && result["content"] == body["content"])?;
        if mismatch {
            check(result["result"] == json!({"kind":"mismatch"}))?;
        } else if kind == "hover" {
            check(result["result"] == json!({"kind":"hover","contents":[]}))?;
        } else {
            check(
                result["result"]["kind"] == "observed"
                    && result["result"]["observation"]["kind"] == kind,
            )?;
        }
    }
    Ok(())
}

/// Dispose only the HTTP observer after the REAL native reply reaches the relay.
pub(super) async fn cancel(
    pair: &Pair<'_>,
    gate: &proxy::Gate,
    method: Method,
    path: &str,
    body: Value,
) -> Result<(), Failure> {
    let request = pair.http.call(method, path, Some(body));
    tokio::pin!(request);
    tokio::select! {
        result = gate.held() => result,
        _ = &mut request => Err(Failure::WrongObservation),
    }
}

pub(super) async fn run(
    pair: &mut Pair<'_>,
    stage: &mut &'static str,
    checks: &mut Vec<&'static str>,
) -> Result<(), Failure> {
    *stage = "effect_free_preparation";
    let before = pair
        .http
        .call(Method::POST, "/api/code/buffers", Some(target()))
        .await?;
    check(before.status == StatusCode::SERVICE_UNAVAILABLE)?;
    check(
        !pair
            .proxy
            .counts()?
            .commands
            .contains_key("openBufferLease"),
    )?;
    // Only the ordinary manifest owns readiness. Preparation cannot invent it.
    let manifest = pair
        .http
        .get(&format!("/api/code/sessions/{SESSION}/manifest"))
        .await?;
    check(manifest["language"]["state"] == "ready")?;
    // Actual Controller selection, not a browser-invented feature flag. This
    // remains advisory; each following owner request rechecks its own authority.
    check(manifest["bufferMode"] == "owned")?;
    checks.push("real_login_enrollment_and_effect_free_unready_refusal");

    *stage = "cancelled_open";
    let first = prepare(pair).await?;
    let gate = pair.proxy.hold("openBufferLease")?;
    cancel(pair, &gate, Method::PUT, &endpoint(&first), json!({})).await?;
    let pending = pair
        .http
        .call(Method::PUT, &endpoint(&first), Some(json!({})))
        .await?;
    check(pending.status == StatusCode::ACCEPTED)?;
    snapshot(&pending.value, &first, "unknown", true)?;
    gate.release();
    settled(pair, &first, "open").await?;
    operation(pair, Method::PUT, &first, "open").await?;
    check(pair.proxy.counts()?.commands.get("openBufferLease") == Some(&1))?;
    checks.push("cancelled_http_open_drains_without_replay");

    *stage = "content_and_independent_owners";
    let second = prepare(pair).await?;
    let retained = prepare(pair).await?;
    check(first != second && first != retained && second != retained)?;
    operation(pair, Method::PUT, &second, "open").await?;
    operation(pair, Method::PUT, &retained, "open").await?;
    reads(pair, &first, TEXT, false).await?;
    std::fs::write(
        pair.root.join("workspace/fixture.txt"),
        "different disk text\n",
    )
    .map_err(|_| Failure::Setup)?;
    reads(pair, &second, "different disk text\n", true).await?;
    reads(pair, &second, TEXT, false).await?;
    checks.push("independent_owners_unicode_content_equality_and_disk_mismatch");

    *stage = "cancelled_read";
    let gate = pair.proxy.hold("readBufferLease")?;
    cancel(
        pair,
        &gate,
        Method::POST,
        &format!("{}/read", endpoint(&first)),
        content(TEXT, "hover"),
    )
    .await?;
    let pending = pair
        .http
        .call(Method::DELETE, &endpoint(&first), Some(json!({})))
        .await?;
    check(pending.status == StatusCode::ACCEPTED)?;
    snapshot(&pending.value, &first, "open", true)?;
    check(
        !pair
            .proxy
            .counts()?
            .commands
            .contains_key("releaseBufferLease"),
    )?;
    gate.release();
    settled(pair, &first, "open").await?;
    operation(pair, Method::DELETE, &first, "released").await?;
    reads(pair, &second, TEXT, false).await?;
    checks.push("cancelled_read_drains_before_explicit_release_without_closing_peer");

    let navigation = navigation::prepare(pair, &second, stage, checks).await?;

    *stage = "synchronization_preparation";
    let sync = synchronization::prepare(pair).await?;
    checks.push("explicit_synchronization_authentication_shared_owner_refusal_and_local_fence");

    *stage = "uninstall_preview";
    let plugin = format!("/api/machines/{MACHINE}/plugins/zed");
    let plan = pair
        .http
        .call(Method::POST, &format!("{plugin}/uninstall-plan"), None)
        .await?
        .ok()?;
    check(plan["affected_sessions"] == json!([]) && plan["plan_id"].is_string())?;
    *stage = "uninstall_commit";
    let uninstalled = pair
        .http
        .post(
            &format!("{plugin}/uninstall"),
            json!({"plan_id":plan["plan_id"]}),
        )
        .await?;
    check(uninstalled["phase"] == "completed" && uninstalled["deleted_session_ids"] == json!([]))?;
    *stage = "synchronization_after_uninstall";
    let sync = synchronization::finish(pair, sync, stage).await?;
    checks.push("lost_real_sync_reply_original_id_query_and_no_apply_replay_after_uninstall");
    checks.push("synchronization_retirement_drains_after_cancelled_http_without_replay");
    let navigation = navigation::handoff(pair, navigation, stage, checks).await?;
    *stage = "synchronization_next_inert_operation";
    let sync = synchronization::next_inert(pair, sync).await?;
    *stage = "removed_paths";
    std::fs::remove_file(pair.root.join("workspace/fixture.txt")).map_err(|_| Failure::Setup)?;
    std::fs::rename(
        pair.root.join("workspace"),
        pair.root.join("moved-workspace"),
    )
    .map_err(|_| Failure::Setup)?;
    reads(pair, &second, TEXT, false).await?;
    checks.push("http_uninstall_and_missing_paths_preserve_original_native_reads");
    navigation::after_path_removal(pair, &navigation, stage, checks).await?;

    *stage = "cancelled_release";
    let prior_releases = pair
        .proxy
        .counts()?
        .commands
        .get("releaseBufferLease")
        .copied()
        .unwrap_or(0);
    let gate = pair.proxy.hold("releaseBufferLease")?;
    cancel(pair, &gate, Method::DELETE, &endpoint(&second), json!({})).await?;
    let pending = pair
        .http
        .call(Method::DELETE, &endpoint(&second), Some(json!({})))
        .await?;
    check(pending.status == StatusCode::ACCEPTED)?;
    gate.release();
    settled(pair, &second, "released").await?;
    operation(pair, Method::DELETE, &second, "released").await?;
    check(pair.proxy.counts()?.commands.get("releaseBufferLease") == Some(&(prior_releases + 1)))?;
    reads(pair, &retained, TEXT, false).await?;
    checks.push("cancelled_release_observed_once_and_independent_owner_retained");

    *stage = "connection_replacement";
    let commands = pair.proxy.counts()?.commands;
    pair.proxy.cut()?;
    pair.connected(2).await?;
    synchronization::replacement_refused(pair, &sync).await?;
    navigation::unavailable(pair, &navigation, StatusCode::CONFLICT).await?;
    let observed = pair
        .http
        .call(Method::GET, &endpoint(&retained), None)
        .await?;
    check(observed.status == StatusCode::BAD_GATEWAY)?;
    let release = pair
        .http
        .call(Method::DELETE, &endpoint(&retained), Some(json!({})))
        .await?;
    check(release.status == StatusCode::BAD_GATEWAY)?;
    let unknown = pair
        .http
        .call(Method::DELETE, &endpoint(&retained), Some(json!({})))
        .await?
        .ok()?;
    snapshot(&unknown, &retained, "unknown", false)?;
    check(pair.proxy.counts()?.commands == commands)?;
    *stage = "controller_restart";
    pair.controller
        .take()
        .ok_or(Failure::Setup)?
        .finish_with_reaper(true)
        .await?;
    pair.start_controller().await?;
    pair.connected(3).await?;
    let missing = pair
        .http
        .call(Method::GET, &endpoint(&retained), None)
        .await?;
    check(missing.status == StatusCode::NOT_FOUND)?;
    let missing = pair
        .http
        .call(Method::GET, &synchronization::endpoint(&sync), None)
        .await?;
    check(missing.status == StatusCode::NOT_FOUND)?;
    check(pair.proxy.counts()?.commands == commands)?;
    navigation::unavailable(pair, &navigation, StatusCode::NOT_FOUND).await?;
    checks.push("navigation_replacement_connection_and_restart_refuse_adoption");
    checks.push("replacement_connection_fenced_and_restart_does_not_adopt_or_release_old_id");
    Ok(())
}
