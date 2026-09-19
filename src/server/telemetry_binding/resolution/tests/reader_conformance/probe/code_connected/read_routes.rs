//! Actual authenticated core file pages, not a native buffer ownership claim.
use super::*;
use reqwest::{Method, StatusCode};

pub(super) const FILE: &str = "route-read.txt";

pub(super) async fn prepare(pair: &Pair<'_>) -> Result<String, Failure> {
    // A non-BMP scalar straddles the core adapter's 256 KiB page boundary.
    let text = format!("{}🙂{}", "r".repeat(256 * 1024 - 1), "z".repeat(10_000));
    std::fs::write(pair.root.join("workspace").join(FILE), &text).map_err(|_| Failure::Setup)?;
    let path = format!("/api/code/sessions/{SESSION}/file?path={FILE}");
    let first = pair.http.get(&path).await?;
    let cursor = first["nextCursor"]
        .as_str()
        .ok_or(Failure::WrongObservation)?;
    let (identity, offset) = cursor.split_once(':').ok_or(Failure::WrongObservation)?;
    check(identity.len() == 64 && identity.bytes().all(|b| b.is_ascii_hexdigit()))?;
    check(offset == (256 * 1024 - 1).to_string())?;
    let next = format!("{path}&cursor={cursor}");
    let second = pair.http.get(&next).await?;
    check(first["apiVersion"] == 1 && second["apiVersion"] == 1)?;
    check(first["path"] == FILE && second["path"] == FILE)?;
    check(first["revision"] == second["revision"] && first["size"] == text.len())?;
    check(first["truncated"] == true && second["truncated"] == false)?;
    check(
        first["limited"] == false && second["limited"] == false && second["nextCursor"].is_null(),
    )?;
    let first_text = first["text"].as_str().ok_or(Failure::WrongObservation)?;
    let second_text = second["text"].as_str().ok_or(Failure::WrongObservation)?;
    check(first_text.len() == 256 * 1024 - 1 && second_text.starts_with('🙂'))?;
    check(format!("{first_text}{second_text}") == text)?;
    check(pair.proxy.counts()?.commands.get("coreFile") == Some(&2))?;
    Ok(next)
}

pub(super) async fn refused(pair: &Pair<'_>, continuation: &str) -> Result<(), Failure> {
    let before = pair.proxy.counts()?.commands;
    let response = pair.http.call(Method::GET, continuation, None).await?;
    check(response.status == StatusCode::GONE)?;
    // The actual HTTP helper also requires no-store/no-ETag on this 410.
    // Refusal must precede native I/O even though the source path was removed.
    check(pair.proxy.counts()?.commands == before)
}
