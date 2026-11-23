//! Physical Session cwd replacement, independent from native ownership.
use super::*;
use reqwest::{Method, StatusCode};

pub(super) const SESSION: &str = "sess-904";
pub(super) const FILE: &str = "session-identity.txt";

pub(super) async fn run(
    pair: &Pair<'_>,
    stage: &mut &'static str,
    checks: &mut Vec<&'static str>,
) -> Result<(), Failure> {
    *stage = "machine_owned_session_root_identity";
    let root = pair.root.join(root_identity::ROOT);
    let text = "s".repeat(300_000);
    std::fs::write(root.join(FILE), &text).map_err(|_| Failure::Setup)?;
    let path = format!("/api/code/sessions/{SESSION}/file?path={FILE}");
    let first = pair.http.get(&path).await?;
    let cursor = first["nextCursor"]
        .as_str()
        .ok_or(Failure::WrongObservation)?;
    let continuation = format!("{path}&cursor={cursor}");
    let conditional = pair
        .http
        .call_conditional(Method::GET, &path, None, Some("*"))
        .await?;
    check(conditional.status == StatusCode::NOT_MODIFIED && conditional.has_etag)?;

    // Hold a real reply that would produce 304 for these identical bytes.
    // Replace only the root object before handing it back to the Controller.
    let gate = pair.proxy.hold("coreSessionFile")?;
    let request = pair
        .http
        .call_conditional(Method::GET, &path, None, Some("*"));
    tokio::pin!(request);
    tokio::select! {
        held = gate.held() => held?,
        _ = &mut request => return Err(Failure::WrongObservation),
    }
    std::fs::rename(&root, pair.root.join("retired-session-root")).map_err(|_| Failure::Setup)?;
    std::fs::create_dir(&root).map_err(|_| Failure::Setup)?;
    std::fs::write(root.join(FILE), &text).map_err(|_| Failure::Setup)?;
    gate.release();
    let stale = request.await?;
    check(
        stale.status == StatusCode::GONE
            && stale.no_store
            && !stale.has_etag
            && stale.value.is_null(),
    )?;
    checks.push("machine_owned_session_root_discards_real_conditional_reply_after_replacement");

    let before = pair.proxy.counts()?.commands;
    let expired = pair.http.call(Method::GET, &continuation, None).await?;
    check(expired.status == StatusCode::GONE && expired.no_store && !expired.has_etag)?;
    check(pair.proxy.counts()?.commands == before)?;
    let fresh = pair.http.get(&path).await?;
    check(fresh["text"] == first["text"] && fresh["nextCursor"].is_string())?;
    check(
        pair.proxy.counts()?.session_root_observations > 0
            && pair.proxy.counts()?.session_root_verifications > 0,
    )?;
    checks.push("machine_owned_session_root_expires_old_page_without_io_and_allows_fresh_read");
    Ok(())
}
