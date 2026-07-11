//! Materializing queued mutations / janitor resolve-ops into the store, one git
//! commit per batch. Deletes are soft-archived, never hard-deleted.
//!
//! Ported faithfully from mnemosyne's `internal/apply` (apply.go → `Batch`,
//! resolve.go → `Resolve`).

use std::collections::HashSet;

use anyhow::{Context, Result};

use super::queue::{Mutation, Op};
use super::store::{Memory, Store, Tier};
use super::tier::project_tier;

fn tier_from_slug(slug: &str) -> Tier {
    if slug.is_empty() {
        Tier::machine()
    } else {
        project_tier(slug)
    }
}

/// The P1 naive applier: materialize a batch of queued mutations directly and
/// commit once, with NO dedup/merge judgment. Deletes are soft-archived. Returns
/// the commit hash (`None` if nothing changed). Mirrors Go's `apply.Batch`.
///
/// # Errors
/// Propagates store write/read/remove and git errors.
pub fn batch(s: &Store, muts: &[Mutation]) -> Result<Option<String>> {
    let mut affected: HashSet<Tier> = HashSet::new();
    for m in muts {
        let t = tier_from_slug(&m.slug);
        match m.op {
            Op::Add | Op::Update => {
                s.write(&t, &m.memory)?;
                affected.insert(t);
            }
            Op::Delete => {
                // Soft-archive: copy to the archive tier, then remove from source.
                if let Ok(existing) = s.read(&t, &m.memory.name) {
                    s.write(&Tier::archive(), &existing)?;
                    affected.insert(Tier::archive());
                }
                s.remove(&t, &m.memory.name)?;
                affected.insert(t);
            }
        }
    }
    write_indexes(s, &affected)?;
    s.commit(&format!(
        "memory: apply batch of {} (P1 naive applier)",
        muts.len()
    ))
}

/// A materialization primitive the janitor session emits after judging a batch.
/// Mirrors Go's `apply.ResolveKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveKind {
    /// create or overwrite a memory in `tier`
    Write,
    /// soft-archive `name` from `from` (eviction)
    Archive,
    /// move `name` from `from` → `tier` (e.g. promote)
    Move,
}

/// One materialization step. Mirrors Go's `apply.ResolveOp`.
#[derive(Debug, Clone)]
pub struct ResolveOp {
    pub kind: ResolveKind,
    /// destination (write/move)
    pub tier: Tier,
    /// source (archive/move)
    pub from: Tier,
    /// payload for write
    pub memory: Memory,
    /// target for archive/move
    pub name: String,
}

impl ResolveOp {
    /// A `write` op for `memory` into `tier`.
    #[must_use]
    pub fn write(tier: Tier, memory: Memory) -> ResolveOp {
        ResolveOp {
            kind: ResolveKind::Write,
            tier,
            from: Tier::machine(),
            memory,
            name: String::new(),
        }
    }

    /// An `archive` op for `name` from `from`.
    #[must_use]
    pub fn archive(from: Tier, name: impl Into<String>) -> ResolveOp {
        ResolveOp {
            kind: ResolveKind::Archive,
            tier: Tier::machine(),
            from,
            memory: empty_memory(),
            name: name.into(),
        }
    }

    /// A `move` op for `name` from `from` → `tier`.
    #[must_use]
    pub fn move_op(from: Tier, tier: Tier, name: impl Into<String>) -> ResolveOp {
        ResolveOp {
            kind: ResolveKind::Move,
            tier,
            from,
            memory: empty_memory(),
            name: name.into(),
        }
    }
}

fn empty_memory() -> Memory {
    Memory {
        name: String::new(),
        description: String::new(),
        mem_type: super::store::MemoryType::Reference,
        body: String::new(),
    }
}

/// Materialize the session's decisions DIRECTLY (no queue, no enqueue — this is
/// the path that breaks the write-back loop) and commit once. Deletes are always
/// soft-archives. Returns the commit hash (`None` if nothing changed). Mirrors
/// Go's `apply.Resolve`.
///
/// # Errors
/// Propagates store and git errors, and errors on an unknown resolve kind (kept
/// for parity with Go's `default` branch — unreachable with the typed enum).
pub fn resolve(s: &Store, ops: &[ResolveOp]) -> Result<Option<String>> {
    let mut affected: HashSet<Tier> = HashSet::new();
    for op in ops {
        match op.kind {
            ResolveKind::Write => {
                s.write(&op.tier, &op.memory)?;
                affected.insert(op.tier.clone());
            }
            ResolveKind::Archive => {
                soft_archive(s, &op.from, &op.name)?;
                affected.insert(op.from.clone());
                affected.insert(Tier::archive());
            }
            ResolveKind::Move => {
                let m = s
                    .read(&op.from, &op.name)
                    .with_context(|| format!("move {}: read {}", op.name, op.from))?;
                s.write(&op.tier, &m)?;
                s.remove(&op.from, &op.name)?;
                affected.insert(op.from.clone());
                affected.insert(op.tier.clone());
            }
        }
    }
    write_indexes(s, &affected)?;
    s.commit(&format!(
        "memory: resolve {} op(s) (janitor judgment)",
        ops.len()
    ))
}

/// Copy a memory to the archive tier (still recall-able), then remove it from its
/// source. Never a hard delete. Mirrors Go's `softArchive`.
fn soft_archive(s: &Store, from: &Tier, name: &str) -> Result<()> {
    if let Ok(m) = s.read(from, name) {
        s.write(&Tier::archive(), &m)?;
    }
    s.remove(from, name)
}

fn write_indexes(s: &Store, affected: &HashSet<Tier>) -> Result<()> {
    // Sort for deterministic write order (HashSet iteration is unordered; Go's
    // map iteration is too, but the indexes are independent so order is moot —
    // sorting just makes the behavior reproducible).
    let mut tiers: Vec<&Tier> = affected.iter().collect();
    tiers.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    for t in tiers {
        s.write_index(t)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store::MemoryType;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

    fn tmp_store() -> (Store, std::path::PathBuf) {
        let pid = std::process::id();
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cowboy-apply-test-{pid}-{id}"));
        std::fs::create_dir_all(&path).unwrap();
        (Store::new(path.clone()), path)
    }

    fn cleanup(p: &std::path::Path) {
        let _ = std::fs::remove_dir_all(p);
    }

    fn seed(s: &Store, t: &Tier, name: &str) {
        s.write(
            t,
            &Memory {
                name: name.to_string(),
                description: format!("{name} desc"),
                mem_type: MemoryType::Project,
                body: name.to_string(),
            },
        )
        .unwrap();
    }

    #[test]
    fn resolve_merge_and_evict() {
        let (s, dir) = tmp_store();
        seed(&s, &Tier::machine(), "a");
        seed(&s, &Tier::machine(), "b");
        seed(&s, &Tier::machine(), "d");

        // merge a,b → c ; evict d
        let hash = resolve(
            &s,
            &[
                ResolveOp::write(
                    Tier::machine(),
                    Memory {
                        name: "c".to_string(),
                        description: "merged a+b".to_string(),
                        mem_type: MemoryType::Project,
                        body: "merged".to_string(),
                    },
                ),
                ResolveOp::archive(Tier::machine(), "a"),
                ResolveOp::archive(Tier::machine(), "b"),
                ResolveOp::archive(Tier::machine(), "d"),
            ],
        )
        .unwrap();
        assert!(hash.is_some(), "expected a commit");

        let machine = s.list(&Tier::machine()).unwrap();
        assert_eq!(machine.len(), 1);
        assert_eq!(machine[0].name, "c");

        let archive = s.list(&Tier::archive()).unwrap();
        assert_eq!(archive.len(), 3, "archive should hold a,b,d");

        // reversibility: nothing hard-deleted — a is recall-able from archive.
        assert!(s.read(&Tier::archive(), "a").is_ok());
        cleanup(&dir);
    }

    #[test]
    fn resolve_promote() {
        let (s, dir) = tmp_store();
        let proj = project_tier("-home-draven-columbus");
        s.write(
            &proj,
            &Memory {
                name: "x".to_string(),
                description: "cross-cutting".to_string(),
                mem_type: MemoryType::Project,
                body: "b".to_string(),
            },
        )
        .unwrap();
        resolve(
            &s,
            &[ResolveOp::move_op(proj.clone(), Tier::machine(), "x")],
        )
        .unwrap();
        assert!(s.read(&Tier::machine(), "x").is_ok());
        assert_eq!(s.list(&proj).unwrap().len(), 0);
        cleanup(&dir);
    }

    #[test]
    fn batch_add_and_soft_delete() {
        let (s, dir) = tmp_store();
        let add = Mutation {
            op: Op::Add,
            memory: Memory {
                name: "k".to_string(),
                description: "kept".to_string(),
                mem_type: MemoryType::Project,
                body: "v".to_string(),
            },
            slug: String::new(),
            cmid: String::new(),
        };
        batch(&s, &[add]).unwrap();
        assert!(s.read(&Tier::machine(), "k").is_ok());

        let del = Mutation {
            op: Op::Delete,
            memory: Memory {
                name: "k".to_string(),
                description: String::new(),
                mem_type: MemoryType::Project,
                body: String::new(),
            },
            slug: String::new(),
            cmid: String::new(),
        };
        batch(&s, &[del]).unwrap();
        // Soft-archived: gone from machine, present in archive.
        assert!(s.read(&Tier::machine(), "k").is_err());
        assert!(s.read(&Tier::archive(), "k").is_ok());
        cleanup(&dir);
    }
}
