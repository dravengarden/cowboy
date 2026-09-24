//! The colocated topology: the Controller reads a Machine's advertised root on
//! its own filesystem, so no Machine command is sent at all. This is the shape
//! the primary deployment actually runs, and it is the branch the remote checks
//! above can never reach.
//!
//! Every assertion here is anchored on the relay's command count, because that
//! is the only observable that distinguishes "executed here" from "executed
//! there" — the bytes are identical either way.
use super::*;
use reqwest::{Method, StatusCode};

/// Advertised by the Machine but read only through the colocated topology, so
/// replacing it disturbs no Session, native owner or installed Plugin.
pub(super) const ROOT: &str = "colocated-root";
pub(super) const FILE: &str = "colocated.txt";
const TEXT: &str = "the controller executed this read\n";

pub(super) fn seed(root: &Path) -> Result<()> {
    std::fs::create_dir(root.join(ROOT))?;
    std::fs::write(root.join(ROOT).join(FILE), TEXT)?;
    Ok(())
}

fn path() -> String {
    format!("/api/code/sessions/workspace::{MACHINE}::{ROOT}/file?path={FILE}")
}

/// Reconnecting after a restart can replace a connection while a read is in
/// flight, which correctly fences that read. Settle on a steady state first,
/// so the measurement below observes the executor and not the reconnect.
async fn settle(pair: &Pair<'_>) -> Result<(), Failure> {
    tokio::time::timeout(DEADLINE, async {
        loop {
            if pair.http.call(Method::GET, &path(), None).await?.status == StatusCode::OK {
                return Ok::<(), Failure>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .map_err(|_| Failure::Timeout)?
}

async fn read(pair: &Pair<'_>) -> Result<(), Failure> {
    let value = pair.http.get(&path()).await?;
    check(value["apiVersion"] == 1 && value["path"] == FILE)?;
    check(value["text"] == TEXT && value["truncated"] == false)?;
    check(value["nextCursor"].is_null() && value["size"] == TEXT.len())
}

/// Restart both processes into the colocated shape and exercise the branch the
/// remote checks cannot reach: a permitted local Machine's root is read by the
/// Controller itself, a replacement ends that observation, and a fresh
/// inventory restores it — all without a single Machine command.
pub(super) async fn run(
    pair: &mut Pair<'_>,
    password: &str,
    stage: &mut &'static str,
    checks: &mut Vec<&'static str>,
) -> Result<(), Failure> {
    *stage = "colocated_restart";
    // Both processes end before either restarts: the Machine must re-announce
    // its mode, and the Controller must observe it fresh.
    if let Some(mut machine) = pair.machine.take() {
        machine.finish_with_reaper(true).await?;
    }
    if let Some(mut controller) = pair.controller.take() {
        controller.finish_with_reaper(true).await?;
    }
    pair.machine_local = true;
    pair.colocated_permission = true;
    pair.start_controller().await?;
    pair.http.login(password).await?;
    pair.start_machine()?;
    let connections = pair.proxy.counts()?.connections;
    pair.connected(connections + 1).await?;

    *stage = "colocated_execution";
    settle(pair).await?;
    let before = pair.proxy.counts()?.commands;
    read(pair).await?;
    // The decisive observation: a permitted local Machine's root is read here,
    // so the Machine is never asked. Identical bytes over the wire would prove
    // nothing; the absence of the command is the proof.
    check(pair.proxy.counts()?.commands == before)?;

    *stage = "colocated_replacement";
    let root = pair.root.join(ROOT);
    std::fs::remove_dir_all(&root).map_err(|_| Failure::Setup)?;
    std::fs::create_dir(&root).map_err(|_| Failure::Setup)?;
    std::fs::write(root.join(FILE), TEXT).map_err(|_| Failure::Setup)?;
    // Same path, same bytes, different object: the observation this route was
    // taken against has ended, and the Controller refuses before reading.
    let replaced = pair.http.call(Method::GET, &path(), None).await?;
    check(replaced.status == StatusCode::GONE && replaced.no_store)?;
    check(!replaced.has_etag && replaced.value.is_null())?;
    check(pair.proxy.counts()?.commands == before)?;

    // A refusal is a fence, not a lost root: a fresh inventory observes the
    // live object and reads resume, still without asking the Machine.
    let refreshed = pair
        .http
        .call(
            Method::POST,
            &format!("/api/machines/{MACHINE}/refresh"),
            Some(json!({})),
        )
        .await?;
    check(refreshed.status == StatusCode::OK)?;
    let after_refresh = pair.proxy.counts()?.commands;
    read(pair).await?;
    check(pair.proxy.counts()?.commands == after_refresh)?;

    // Withdrawing the permission and observing the executor change is NOT
    // asserted here: restarting the Controller a second time in quick
    // succession makes the fixture Machine reconnect repeatedly, and a flapping
    // fixture would make this gate unreliable rather than more convincing. The
    // permission matrix — including a remote Machine claiming local mode — is
    // covered exhaustively by `colocated_execution_is_denied_until_a_machine_is_named`
    // and `a_declaration_and_a_permission_are_both_required`. The relay still
    // classifies a `coreColocatedFile` command, so an unexpected dispatch from
    // this root would be counted above rather than silently ignored.
    checks.push("colocated_reads_execute_here_only_by_permission_and_fence_a_replaced_root");
    Ok(())
}
