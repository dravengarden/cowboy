//! cowboy::memory — the native memory subsystem (the fold of mnemosyne, p5).
//!
//! Owns (in Phases B–C) the file+git memory store, the keyword index, the
//! queue+debounce, the tidy timer, and the in-process janitor that judges
//! candidates and applies resolve-ops. Agents READ the store with plain
//! `rg`/`cat` (taught by the `memory` skill); they WRITE only through
//! `cowboy mem record` → the daemon's queue → the janitor. There is NO MCP: the
//! janitor is a cowboy `system` session whose exact turn reply returns through
//! ACP completion, so it calls no tools and needs no admin socket/shim/approval.
//!
//! Phase A (this commit): scaffold only — the `cowboy mem` subcommand exists and
//! the write VALIDATION guardrail is in place; the store/index/queue/janitor +
//! the `/api/memory/record` endpoint land in Phases B–C.

// The pure-logic ports of mnemosyne's core (p5 Phase B): the file+git store,
// the tier router, the keyword index, the debounce queue, and the resolve-op
// applier. These are integration-free (no axum, no Hub, no tokio tasks); Phase C
// wires them into the daemon.
pub mod apply;
pub mod index;
pub mod janitor;
pub mod queue;
pub mod store;
pub mod tier;

pub use index::Index;
pub use janitor::Janitor;
pub use queue::{Config as QueueConfig, Mutation, Op, Queue};
pub use store::{Memory, MemoryType, Store};

use crate::cli::{MemArgs, MemCommand, MemRecordArgs};

/// `cowboy mem …` entry point — the validated memory WRITE path. Reads are NOT
/// here: agents read the store directly with `rg`/`cat` (see the `memory` skill).
pub async fn mem_cli(args: MemArgs) -> anyhow::Result<()> {
    match args.command {
        MemCommand::Record(a) => record(a).await,
        MemCommand::Forget { name } => forget(&name).await,
    }
}

/// The local daemon's base URL. The CLI is a thin client: it POSTs the validated
/// proposal to the running `cowboy serve` (default bind `127.0.0.1:3333`), which
/// enqueues it → debounce → the janitor dedups/judges/commits. The CLI NEVER
/// writes a memory file itself — that is the guardrail (D2).
const DAEMON_BASE: &str = "http://127.0.0.1:3333";

async fn record(a: MemRecordArgs) -> anyhow::Result<()> {
    // Validate BEFORE touching the network — the frontmatter guardrail.
    validate_record(&a.name, &a.description, &a.mem_type)?;
    let body = a.body.join("\n");
    let payload = serde_json::json!({
        "name": a.name,
        "description": a.description,
        "type": a.mem_type,
        "tier": a.tier,
        "body": body,
    });
    post_proposal("/api/memory/record", &payload).await?;
    println!("recorded proposal {:?} (queued for the janitor)", a.name);
    Ok(())
}

async fn forget(name: &str) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("cowboy mem forget: a memory name is required");
    }
    // Soft-archive only (the janitor moves it to archive/), never hard-delete.
    let payload = serde_json::json!({ "name": name });
    post_proposal("/api/memory/forget", &payload).await?;
    println!("forget proposal {name:?} (queued for the janitor)");
    Ok(())
}

/// POST a JSON proposal to a daemon memory endpoint. Surfaces a clear "is cowboy
/// serve running?" hint on connection refused, and the daemon's body on a non-2xx
/// (e.g. the 404 the endpoints return when `--memory-enabled` is off).
async fn post_proposal(path: &str, payload: &serde_json::Value) -> anyhow::Result<()> {
    let url = format!("{DAEMON_BASE}{path}");
    let resp = reqwest::Client::new()
        .post(&url)
        .json(payload)
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                anyhow::anyhow!(
                    "cannot reach the cowboy daemon at {DAEMON_BASE} — is `cowboy serve` running \
                     (with --memory-enabled)? underlying error: {e}"
                )
            } else {
                anyhow::anyhow!("POST {url}: {e}")
            }
        })?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("POST {url} → {status}: {body}");
    }
    Ok(())
}

/// Validate a record proposal's frontmatter BEFORE it can reach the store — the
/// guardrail (D2) that stops an agent committing a malformed/mis-tiered memory.
fn validate_record(name: &str, description: &str, mem_type: &str) -> anyhow::Result<()> {
    let kebab = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    if name.is_empty() || !name.chars().all(kebab) {
        anyhow::bail!("name must be non-empty kebab-case (a-z0-9-), got {name:?}");
    }
    if description.trim().is_empty() {
        anyhow::bail!("description is required (it is the recall hook)");
    }
    const TYPES: [&str; 4] = ["user", "feedback", "project", "reference"];
    if !TYPES.contains(&mem_type) {
        anyhow::bail!("type must be one of {TYPES:?}, got {mem_type:?}");
    }
    Ok(())
}
