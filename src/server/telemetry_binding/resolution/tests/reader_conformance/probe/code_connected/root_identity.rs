//! Actual Machine-owned Workspace root identity through the supplied
//! Controller and Machine. Replacing the root is a real filesystem event on
//! the isolated fixture; nothing is stubbed and no timeout is shortened.
use super::*;
use reqwest::{Method, StatusCode};

/// A second advertised root, separate from the Session fixture workspace so
/// replacing it disturbs no native owner, worktree or installed Plugin.
pub(super) const ROOT: &str = "identity-root";
pub(super) const FILE: &str = "identity.txt";
const TEXT: &str = "machine owned root identity\n";

pub(super) fn seed(root: &Path) -> Result<()> {
    std::fs::create_dir(root.join(ROOT))?;
    std::fs::write(root.join(ROOT).join(FILE), TEXT)?;
    Ok(())
}

fn path() -> String {
    format!("/api/code/sessions/workspace::{MACHINE}::{ROOT}/file?path={FILE}")
}

async fn read(pair: &Pair<'_>) -> Result<(), Failure> {
    let value = pair.http.get(&path()).await?;
    check(value["apiVersion"] == 1 && value["path"] == FILE)?;
    check(value["text"] == TEXT && value["truncated"] == false)?;
    check(value["nextCursor"].is_null() && value["size"] == TEXT.len())
}

fn dispatched(pair: &Pair<'_>) -> Result<u32, Failure> {
    Ok(pair
        .proxy
        .counts()?
        .commands
        .get("coreSwapFile")
        .copied()
        .unwrap_or(0))
}

/// Replace the advertised root with a different directory object holding the
/// same path and the same bytes, then require refusal before the next read.
/// Only the underlying object changes: content equality cannot hide it.
pub(super) async fn run(
    pair: &Pair<'_>,
    stage: &mut &'static str,
    checks: &mut Vec<&'static str>,
) -> Result<(), Failure> {
    *stage = "machine_owned_root_identity";
    // The Session fixture route is independent of this advertised root and
    // must not dispatch or be fenced by anything below.
    let session_reads = pair.proxy.counts()?.commands.get("coreFile").copied();
    read(pair).await?;
    check(dispatched(pair)? == 1)?;

    let root = pair.root.join(ROOT);
    std::fs::remove_dir_all(&root).map_err(|_| Failure::Setup)?;
    std::fs::create_dir(&root).map_err(|_| Failure::Setup)?;
    std::fs::write(root.join(FILE), TEXT).map_err(|_| Failure::Setup)?;

    // No inventory refresh has happened: the Controller still holds its old
    // observation and dispatches once. The Machine owns the identity and must
    // refuse before reading the replacement object.
    let replaced = pair.http.call(Method::GET, &path(), None).await?;
    check(replaced.status == StatusCode::GONE && replaced.no_store)?;
    check(!replaced.has_etag && replaced.value.is_null())?;
    check(dispatched(pair)? == 2)?;

    // That refusal ended the Controller observation, so cached bytes, ETags
    // and conditional replies for the old root are unreachable too, with no
    // further dispatch. This is an ended observation, not a rollback.
    let ended = pair.http.call(Method::GET, &path(), None).await?;
    check(ended.status == StatusCode::NOT_FOUND && ended.value.is_null())?;
    check(dispatched(pair)? == 2)?;

    // An explicit inventory refresh re-observes the live root and mints a new
    // identity. A fence is not a permanent loss of the configured root.
    let refreshed = pair
        .http
        .call(
            Method::POST,
            &format!("/api/machines/{MACHINE}/refresh"),
            Some(json!({})),
        )
        .await?;
    // The Machine sends its inventory before acknowledging the command on the
    // same ordered connection, so no polling is needed or permitted here.
    check(refreshed.status == StatusCode::OK)?;
    read(pair).await?;
    check(dispatched(pair)? == 3)?;
    check(pair.proxy.counts()?.commands.get("coreFile").copied() == session_reads)?;
    checks.push("machine_owned_root_identity_refuses_a_replaced_advertised_root");
    Ok(())
}
