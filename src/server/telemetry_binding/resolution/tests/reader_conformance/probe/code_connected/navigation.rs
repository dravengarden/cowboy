//! Actual enrolled protocol-21, product-authenticated Service continuation.
//! Only the language answers are synthetic; no control/native reply is faked.
use super::*;
use reqwest::{Method, StatusCode};

const SOURCE: &str = "fn source() { target(); }\n";
const A: &str = "// a🙂z\npub fn target() {}\n";
const B: &str = "// b🙂q\npub struct Target;\n";

pub(super) struct Retained {
    group: String,
    other: String,
    destination: u64,
    target: Option<String>,
}

pub(super) fn executable() -> Result<Binary> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("plugins/zed/adapter/target/debug/examples/navigation_lsp")
        .canonicalize()?;
    let metadata = path.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= 128 * 1024 * 1024,
        "bounded test-only LSP required"
    );
    let bytes = std::fs::read(&path)?;
    ensure!(
        bytes.starts_with(b"\x7fELF"),
        "explicit test-only LSP ELF required"
    );
    Ok(Binary {
        path,
        sha256: sha256(&bytes),
    })
}

pub(super) fn seed(root: &Path) -> Result<()> {
    let workspace = root.join("workspace/lsp-worktree");
    std::fs::create_dir(&workspace)?;
    for (name, text) in [
        ("navigation.rs", SOURCE),
        ("destination-a.rs", A),
        ("destination-b.rs", B),
    ] {
        private_write(&workspace.join(name), text.as_bytes())?;
    }
    Ok(())
}

pub(super) fn configure(
    pair: &Pair<'_>,
    installation: &Value,
    executable: &Binary,
) -> Result<(), Failure> {
    let digest = installation["digest"]
        .as_str()
        .and_then(|value| value.strip_prefix("sha256:"))
        .ok_or(Failure::Setup)?;
    check(digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))?;
    let bytes = std::fs::read(&executable.path).map_err(|_| Failure::Setup)?;
    check(sha256(&bytes) == executable.sha256)?;
    let config = pair.root.join(format!(
        "machine/plugins/zed/runtime/{digest}/home/.config/zed"
    ));
    std::fs::create_dir_all(&config).map_err(|_| Failure::Setup)?;
    private_write(&config.join("settings.json"), &serde_json::to_vec(&json!({
        "languages":{"Rust":{"language_servers":["rust-analyzer"]}},
        "lsp":{"rust-analyzer":{"binary":{"path":executable.path,"arguments":[pair.root.join("workspace")],"ignore_system_version":true}}}
    })).map_err(|_| Failure::Setup)?).map_err(|_| Failure::Setup)
}

fn content(text: &str) -> Value {
    json!({"sha256":sha256(text.as_bytes()),"utf8Bytes":text.len()})
}
fn endpoint(id: &str) -> String {
    format!("/api/code/navigations/{id}")
}
fn count(pair: &Pair<'_>, kind: &str) -> Result<u32, Failure> {
    Ok(pair
        .proxy
        .counts()?
        .commands
        .get(kind)
        .copied()
        .unwrap_or(0))
}

async fn settled(pair: &Pair<'_>, id: &str, state: &str) -> Result<Value, Failure> {
    tokio::time::timeout(DEADLINE, async {
        loop {
            let reply = pair.http.call(Method::GET, &endpoint(id), None).await?;
            if reply.status == StatusCode::OK {
                check(reply.value["state"] == state && reply.value["pending"] == false)?;
                return Ok(reply.value);
            }
            check(reply.status == StatusCode::ACCEPTED && reply.value["pending"] == true)?;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| Failure::Timeout)?
}

async fn operation(
    pair: &Pair<'_>,
    id: &str,
    method: Method,
    state: &str,
) -> Result<Value, Failure> {
    let value = pair
        .http
        .call(method, &endpoint(id), Some(json!({})))
        .await?
        .ok()?;
    check(value["navigationId"] == id && value["state"] == state && value["pending"] == false)?;
    Ok(value)
}

fn events(root: &Path) -> Result<Vec<Value>, Failure> {
    std::fs::read_to_string(root.join("workspace/lsp-events.jsonl"))
        .map_err(|_| Failure::Setup)?
        .lines()
        .map(|line| serde_json::from_str(line).map_err(|_| Failure::WrongObservation))
        .collect()
}

pub(super) async fn prepare(
    pair: &Pair<'_>,
    plaintext_source: &str,
    stage: &mut &'static str,
    checks: &mut Vec<&'static str>,
) -> Result<Retained, Failure> {
    *stage = "navigation_source";
    let value = pair
        .http
        .post(
            "/api/code/buffers",
            json!({"sessionId":SESSION,"path":"lsp-worktree/navigation.rs"}),
        )
        .await?;
    let source = value["resourceId"]
        .as_str()
        .ok_or(Failure::WrongObservation)?
        .to_owned();
    let value = pair
        .http
        .call(Method::PUT, &exercise::endpoint(&source), Some(json!({})))
        .await?
        .ok()?;
    check(value["state"] == "open")?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if events(pair.root).is_ok_and(|events| {
                events.iter().any(|event| {
                    event["method"] == "textDocument/didOpen"
                        && event["document"]
                            .as_str()
                            .is_some_and(|uri| uri.ends_with("/navigation.rs"))
                })
            }) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| Failure::Timeout)?;
    let path = format!("{}/navigations", exercise::endpoint(&source));
    Http::new(pair.address)?.denied(Method::POST, &path, Some(json!({"content":content(SOURCE),"position":{"row":0,"column":3},"query":"definition"}))).await?;
    let mut groups = Vec::new();
    let mut selected = 0;
    for query in [
        "definition",
        "declaration",
        "typeDefinition",
        "implementation",
        "references",
    ] {
        *stage = "navigation_acquisition";
        let prepared = pair
            .http
            .post(
                &path,
                json!({"content":content(SOURCE),"position":{"row":0,"column":3},"query":query}),
            )
            .await?;
        let id = prepared["navigationId"]
            .as_str()
            .ok_or(Failure::WrongObservation)?
            .to_owned();
        check(
            prepared["state"] == "prepared"
                && prepared["locations"] == json!([])
                && prepared.get("navigation").is_none(),
        )?;
        let retained = if query == "definition" {
            *stage = "navigation_lost_execute_reply";
            let gate = pair.proxy.hold("codeNavigationExecute")?;
            let execute_endpoint = endpoint(&id);
            let call = pair
                .http
                .call(Method::PUT, &execute_endpoint, Some(json!({})));
            tokio::pin!(call);
            tokio::select! { held = gate.held() => held?, _ = &mut call => return Err(Failure::WrongObservation) }
            gate.discard();
            check(call.await?.status == StatusCode::BAD_GATEWAY)?;
            operation(pair, &id, Method::PUT, "unknown").await?;
            let denied = pair
                .http
                .call(Method::DELETE, &endpoint(&id), Some(json!({})))
                .await?;
            check(denied.status == StatusCode::CONFLICT)?;
            let value = settled(pair, &id, "retained").await?;
            check(count(pair, "codeNavigationExecute")? == 1)?;
            checks.push("lost_real_navigation_execute_reply_original_query_without_replay");
            value
        } else {
            operation(pair, &id, Method::PUT, "retained").await?
        };
        let locations = retained["locations"]
            .as_array()
            .ok_or(Failure::WrongObservation)?;
        check(locations.len() == 3)?;
        for location in locations {
            let text = match location["path"].as_str() {
                Some("lsp-worktree/destination-a.rs") => A,
                Some("lsp-worktree/destination-b.rs") => B,
                _ => return Err(Failure::WrongObservation),
            };
            check(
                location["content"] == content(text)
                    && location["start"] == json!({"row":0,"column":4})
                    && location["end"] == json!({"row":0,"column":6}),
            )?;
        }
        if query == "definition" {
            selected = u64::try_from(
                locations
                    .iter()
                    .position(|location| location["path"] == "lsp-worktree/destination-a.rs")
                    .ok_or(Failure::WrongObservation)?,
            )
            .map_err(|_| Failure::WrongObservation)?;
        }
        check(operation(pair, &id, Method::PUT, "retained").await? == retained)?;
        check(settled(pair, &id, "retained").await? == retained)?;
        groups.push(id);
    }
    check(count(pair, "codeNavigationExecute")? == 5)?;
    let events = events(pair.root)?;
    for query in [
        "definition",
        "declaration",
        "typeDefinition",
        "implementation",
        "references",
    ] {
        check(
            events
                .iter()
                .filter(|event| event["method"] == format!("textDocument/{query}"))
                .count()
                == 1,
        )?;
    }
    let group = groups.remove(0);
    for id in groups {
        operation(pair, &id, Method::DELETE, "released").await?;
    }
    // Keep a separate EMPTY group for reconnect/restart refusal. Another
    // nonempty group must not accidentally keep the tested destinations alive
    // and mask a broken ordinary-owner handoff after the parent releases.
    let prepared = pair
        .http
        .post(
            &format!("{}/navigations", exercise::endpoint(plaintext_source)),
            json!({"content":content(TEXT),"position":{"row":0,"column":0},"query":"definition"}),
        )
        .await?;
    let other = prepared["navigationId"]
        .as_str()
        .ok_or(Failure::WrongObservation)?
        .to_owned();
    let empty = operation(pair, &other, Method::PUT, "retained").await?;
    check(empty["locations"] == json!([]))?;
    let value = pair
        .http
        .call(
            Method::DELETE,
            &exercise::endpoint(&source),
            Some(json!({})),
        )
        .await?
        .ok()?;
    check(value["state"] == "released")?;
    checks.push("enrolled_protocol21_all_five_navigation_kinds_unicode_original_owner");
    Ok(Retained {
        group,
        other,
        destination: selected,
        target: None,
    })
}

pub(super) async fn handoff(
    pair: &Pair<'_>,
    mut retained: Retained,
    stage: &mut &'static str,
    checks: &mut Vec<&'static str>,
) -> Result<Retained, Failure> {
    *stage = "navigation_destination_after_uninstall";
    let path = format!("{}/destinations", endpoint(&retained.group));
    let body = json!({"destination":retained.destination,"content":content(A)});
    let gate = pair.proxy.hold("codeNavigationDestination")?;
    exercise::cancel(pair, &gate, Method::POST, &path, body.clone()).await?;
    gate.release();
    let value = settled(pair, &retained.group, "retained").await?;
    let destination = &value["destinations"][0];
    check(
        destination["state"] == "prepared" && destination["destination"] == retained.destination,
    )?;
    let target = destination["resourceId"]
        .as_str()
        .ok_or(Failure::WrongObservation)?
        .to_owned();
    check(
        pair.http.post(&path, body).await? == value
            && count(pair, "codeNavigationDestination")? == 1,
    )?;
    let opened = pair
        .http
        .call(Method::PUT, &exercise::endpoint(&target), Some(json!({})))
        .await?
        .ok()?;
    check(opened["state"] == "open")?;
    checks.push("cancelled_destination_observer_handoff_to_ordinary_owner_after_uninstall");
    *stage = "navigation_lost_release_reply";
    let before = count(pair, "codeNavigationRelease")?;
    let gate = pair.proxy.hold("codeNavigationRelease")?;
    let release_endpoint = endpoint(&retained.group);
    let call = pair
        .http
        .call(Method::DELETE, &release_endpoint, Some(json!({})));
    tokio::pin!(call);
    tokio::select! { held = gate.held() => held?, _ = &mut call => return Err(Failure::WrongObservation) }
    gate.discard();
    check(call.await?.status == StatusCode::BAD_GATEWAY)?;
    operation(pair, &retained.group, Method::DELETE, "release_unknown").await?;
    settled(pair, &retained.group, "released").await?;
    operation(pair, &retained.group, Method::DELETE, "released").await?;
    check(count(pair, "codeNavigationRelease")? == before + 1)?;
    checks.push("lost_real_parent_release_reply_original_query_without_replay");
    retained.target = Some(target);
    Ok(retained)
}

pub(super) async fn after_path_removal(
    pair: &Pair<'_>,
    retained: &Retained,
    stage: &mut &'static str,
    checks: &mut Vec<&'static str>,
) -> Result<(), Failure> {
    *stage = "navigation_independent_destination";
    let target = retained
        .target
        .as_deref()
        .ok_or(Failure::WrongObservation)?;
    let value = pair.http.post(&format!("{}/read", exercise::endpoint(target)), json!({"kind":"content","content":content(A),"query":{"kind":"hover","position":{"row":0,"column":4}}})).await?;
    check(
        value["resourceId"] == target
            && value["result"]["content"] == content(A)
            && value["result"]["result"]["kind"] == "hover"
            && value["result"]["result"]["contents"]
                .as_array()
                .is_some_and(|contents| !contents.is_empty()),
    )?;
    let released = pair
        .http
        .call(Method::DELETE, &exercise::endpoint(target), Some(json!({})))
        .await?
        .ok()?;
    check(released["state"] == "released")?;
    checks.push("destination_content_read_after_parent_release_and_path_removal");
    Ok(())
}

pub(super) async fn unavailable(
    pair: &Pair<'_>,
    retained: &Retained,
    status: StatusCode,
) -> Result<(), Failure> {
    let commands = pair.proxy.counts()?.commands;
    for method in [Method::GET, Method::PUT, Method::DELETE] {
        let response = pair
            .http
            .call(method, &endpoint(&retained.other), Some(json!({})))
            .await?;
        check(response.status == status)?;
    }
    check(pair.proxy.counts()?.commands == commands)
}
