//! cowboy::memory — the native memory subsystem (the fold of mnemosyne, p5).
//!
//! Owns (in Phases B–C) the file+git memory store, the keyword index, the
//! queue+debounce, the tidy timer, and the in-process janitor that judges
//! candidates and applies resolve-ops. Agents READ the store with plain
//! `rg`/`cat` (taught by the `memory` skill); they WRITE only through
//! `cowboy mem record` → the daemon's queue → the janitor. There is NO MCP: the
//! janitor is a cowboy `system` session whose reply cowboy reads IN-PROCESS
//! (`Hub::snapshot`), so it calls no tools and needs no admin socket/shim/
//! approval.
//!
//! Phase A (this commit): scaffold only — the `cowboy mem` subcommand exists and
//! the write VALIDATION guardrail is in place; the store/index/queue/janitor +
//! the `/api/memory/record` endpoint land in Phases B–C.

use crate::cli::{MemArgs, MemCommand, MemRecordArgs};

/// `cowboy mem …` entry point — the validated memory WRITE path. Reads are NOT
/// here: agents read the store directly with `rg`/`cat` (see the `memory` skill).
pub async fn mem_cli(args: MemArgs) -> anyhow::Result<()> {
    match args.command {
        MemCommand::Record(a) => record(a).await,
        MemCommand::Forget { name } => forget(&name).await,
    }
}

async fn record(a: MemRecordArgs) -> anyhow::Result<()> {
    validate_record(&a.name, &a.description, &a.mem_type)?;
    // Phase C: POST the proposal to the running daemon's `/api/memory/record`,
    // which enqueues it → debounce → the janitor dedups/judges/commits. The CLI
    // NEVER writes a file itself — that is the guardrail.
    anyhow::bail!(
        "cowboy mem record: input validated, but the daemon record endpoint \
         lands in p5 Phase C (not wired yet)"
    )
}

async fn forget(name: &str) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("cowboy mem forget: a memory name is required");
    }
    // Phase C: soft-archive only (move to archive/), never hard-delete.
    anyhow::bail!("cowboy mem forget: lands in p5 Phase C (not wired yet)")
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
