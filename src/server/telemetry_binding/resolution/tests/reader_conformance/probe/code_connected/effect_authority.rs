//! Preserve real native outcomes, but never disclose them to the revoked
//! original login. A fresh original-user login only observes/releases by ID.
use super::*;
use reqwest::{Method, StatusCode};

async fn held_logout(
    pair: &mut Pair<'_>,
    password: &str,
    path: &str,
    method: Method,
    command: &'static str,
) -> Result<(), Failure> {
    let mut reader = Http::new(pair.address)?;
    reader.login(password).await?;
    let original = std::mem::replace(&mut pair.http, reader);
    let body = (method != Method::GET).then(|| json!({}));
    let before = pair.proxy.counts()?.commands;
    let gate = pair.proxy.hold(command)?;
    {
        // Keep the failing client on Pair until success, so negative evidence
        // identifies the actual held operation rather than an older request.
        let request = pair.http.call(method.clone(), path, body.clone());
        tokio::pin!(request);
        tokio::select! {
            held = gate.held() => held?,
            _ = &mut request => return Err(Failure::WrongObservation),
        }
        let logout = pair
            .http
            .call(Method::POST, "/api/auth/logout", Some(json!({})))
            .await?;
        check(logout.status == StatusCode::OK)?;
        gate.release();
        let reply = request.await?;
        check(
            reply.status == StatusCode::UNAUTHORIZED
                && reply.no_store
                && !reply.has_etag
                && reply.value.is_null(),
        )?;
    }
    let mut expected = before;
    *expected.entry(command.into()).or_default() += 1;
    check(pair.proxy.counts()?.commands == expected)?;
    pair.http.denied(method, path, body).await?;
    check(pair.proxy.counts()?.commands == expected)?;
    pair.http = original;
    Ok(())
}

async fn snapshot(
    pair: &Pair<'_>,
    path: &str,
    method: Method,
    id: &str,
    state: &str,
) -> Result<(), Failure> {
    let body = (method != Method::GET).then(|| json!({}));
    let value = pair.http.call(method, path, body).await?.ok()?;
    check(value == json!({"apiVersion":1,"resourceId":id,"state":state,"pending":false}))
}

pub(super) async fn run(
    pair: &mut Pair<'_>,
    password: &str,
    stage: &mut &'static str,
    checks: &mut Vec<&'static str>,
) -> Result<(), Failure> {
    *stage = "owned_open_original_login_revocation";
    let id = exercise::prepare(pair).await?;
    let path = exercise::endpoint(&id);
    held_logout(pair, password, &path, Method::PUT, "openBufferLease").await?;
    let before = pair.proxy.counts()?.commands;
    // The admitted Open was actually recorded despite response refusal.
    // Repeating PUT observes that outcome without admitting another Open.
    snapshot(pair, &path, Method::PUT, &id, "open").await?;
    check(pair.proxy.counts()?.commands == before)?;
    snapshot(pair, &path, Method::GET, &id, "open").await?;
    let mut expected = before;
    *expected.entry("queryBufferLease".into()).or_default() += 1;
    check(pair.proxy.counts()?.commands == expected)?;
    checks.push(*stage);

    *stage = "owned_query_original_login_revocation";
    held_logout(pair, password, &path, Method::GET, "queryBufferLease").await?;
    let before = pair.proxy.counts()?.commands;
    snapshot(pair, &path, Method::PUT, &id, "open").await?;
    check(pair.proxy.counts()?.commands == before)?;
    checks.push(*stage);

    *stage = "owned_release_original_login_revocation";
    held_logout(pair, password, &path, Method::DELETE, "releaseBufferLease").await?;
    let before = pair.proxy.counts()?.commands;
    for method in [Method::GET, Method::DELETE] {
        snapshot(pair, &path, method, &id, "released").await?;
    }
    let refused = pair.http.call(Method::PUT, &path, Some(json!({}))).await?;
    check(refused.status == StatusCode::CONFLICT)?;
    // Terminal evidence remains locally queryable; no repeated native release
    // or attempted reopen, and no other owner was implicitly closed.
    check(pair.proxy.counts()?.commands == before)?;
    checks.push(*stage);
    Ok(())
}
