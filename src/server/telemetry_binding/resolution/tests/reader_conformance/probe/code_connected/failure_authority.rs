//! Real lost replies under an ended original login, never synthesized errors.
use super::*;
use reqwest::{Method, StatusCode};

pub(super) async fn lost_reply(
    pair: &mut Pair<'_>,
    password: &str,
    path: &str,
    method: Method,
    body: Option<Value>,
    command: &'static str,
) -> Result<(), Failure> {
    check(matches!(
        command,
        "readBufferLease" | "codeSyncQuery" | "codeNavigationQuery"
    ))?;
    let mut reader = Http::new(pair.address)?;
    reader.login(password).await?;
    let original = std::mem::replace(&mut pair.http, reader);
    let mut expected = pair.proxy.counts()?.commands;
    let gate = pair.proxy.hold(command)?;
    {
        let started = std::time::Instant::now();
        let call = pair.http.call(method.clone(), path, body.clone());
        tokio::pin!(call);
        tokio::select! {
            held = gate.held() => held?,
            _ = &mut call => return Err(Failure::WrongObservation),
        }
        check(
            pair.http
                .call(Method::POST, "/api/auth/logout", Some(json!({})))
                .await?
                .status
                == StatusCode::OK,
        )?;
        // The actual native result has reached the relay. Lose just that
        // correlated reply and wait the production 40-second command timeout.
        gate.discard();
        let reply = call.await?;
        check(started.elapsed() >= Duration::from_secs(39))?;
        check(
            reply.status == StatusCode::UNAUTHORIZED
                && reply.no_store
                && !reply.has_etag
                && reply.value.is_null(),
        )?;
    }
    *expected.entry(command.into()).or_default() += 1;
    if command == "readBufferLease" {
        *expected
            .entry("bufferLeaseContentSupport".into())
            .or_default() += 1;
    }
    check(pair.proxy.counts()?.commands == expected)?;
    pair.http.denied(method, path, body).await?;
    check(pair.proxy.counts()?.commands == expected)?;
    pair.http = original;
    Ok(())
}

pub(super) async fn read(
    pair: &mut Pair<'_>,
    password: &str,
    resource: &str,
) -> Result<(), Failure> {
    let path = format!("{}/read", exercise::endpoint(resource));
    let body = json!({"kind":"content", "content":{"sha256":sha256(TEXT.as_bytes()),
        "utf8Bytes":TEXT.len()},"query":{"kind":"symbols"}});
    lost_reply(
        pair,
        password,
        &path,
        Method::POST,
        Some(body.clone()),
        "readBufferLease",
    )
    .await?;
    let mut expected = pair.proxy.counts()?.commands;
    let value = pair.http.post(&path, body.clone()).await?;
    check(
        value["resourceId"] == resource
            && value["result"]["content"] == body["content"]
            && value["result"]["result"]["kind"] == "observed",
    )?;
    for command in ["bufferLeaseContentSupport", "readBufferLease"] {
        *expected.entry(command.into()).or_default() += 1;
    }
    check(pair.proxy.counts()?.commands == expected)
}
