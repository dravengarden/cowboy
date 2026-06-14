//! The keyword/tag index over the memory store. It answers queries with NO model
//! — this is what makes recall/dedup-prefilter cheap and keeps the model off the
//! read path. Tokens from a memory's name (weight 3), description (weight 2), and
//! body (weight 1) are scored against the query.
//!
//! Ported faithfully from mnemosyne's `internal/index/index.go`. The one additive
//! shape change: `query` takes a `limit` (Phase-C wants ranked *compact* hits),
//! whereas Go's `Query` returned all overlapping hits — pass `0`/`usize::MAX` for
//! the Go-identical "no cap" behavior.

use std::collections::HashMap;

use super::store::{Memory, Store, Tier};

/// An index entry pairs a memory with the tier it lives in. Mirrors Go's
/// `index.Entry`.
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub tier: Tier,
    pub memory: Memory,
}

/// A scored query result. Carries the flat fields Phase-C wants (`name`,
/// `description`, `tier`, `score`) plus the full `memory` for callers that need
/// the body (Go's `Hit` carried the whole `Memory`).
#[derive(Debug, Clone)]
pub struct Hit {
    pub name: String,
    pub description: String,
    pub tier: Tier,
    pub score: i64,
    pub memory: Memory,
}

/// An in-memory keyword index. Rebuild it when the store changes.
pub struct Index {
    entries: Vec<IndexEntry>,
    weights: Vec<HashMap<String, i64>>, // token -> weight, parallel to entries
}

/// Tokenize like Go's `tokenize`: lowercase, split on every non-letter/non-digit
/// rune, drop tokens shorter than 2 chars.
fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|f| f.chars().count() >= 2)
        .map(str::to_string)
        .collect()
}

impl Index {
    /// Build an index over the given entries. Mirrors Go's `index.New`.
    #[must_use]
    pub fn new(entries: Vec<IndexEntry>) -> Index {
        let mut weights = Vec::with_capacity(entries.len());
        for e in &entries {
            let mut w: HashMap<String, i64> = HashMap::new();
            for t in tokenize(&e.memory.name) {
                *w.entry(t).or_insert(0) += 3;
            }
            for t in tokenize(&e.memory.description) {
                *w.entry(t).or_insert(0) += 2;
            }
            for t in tokenize(&e.memory.body) {
                *w.entry(t).or_insert(0) += 1;
            }
            weights.push(w);
        }
        Index { entries, weights }
    }

    /// Build an index over a store's active tiers (machine + every project).
    /// Mirrors how the daemon assembles the recall index from the store.
    ///
    /// # Errors
    /// Propagates store list errors.
    pub fn from_store(s: &Store) -> anyhow::Result<Index> {
        let mut entries = Vec::new();
        for t in s.active_tiers()? {
            for m in s.list(&t)? {
                entries.push(IndexEntry {
                    tier: t.clone(),
                    memory: m,
                });
            }
        }
        Ok(Index::new(entries))
    }

    /// Return entries whose tokens overlap `q`, ranked by summed weight (desc),
    /// ties broken by name (asc). Entries with zero overlap are excluded. `limit`
    /// caps the result (use `0` for "no cap"; Go's `Query` had no cap).
    ///
    /// Mirrors Go's `index.Query` plus the Phase-C `limit`.
    #[must_use]
    pub fn query(&self, q: &str, limit: usize) -> Vec<Hit> {
        let qt = tokenize(q);
        let mut hits: Vec<Hit> = Vec::new();
        for (i, e) in self.entries.iter().enumerate() {
            let mut score: i64 = 0;
            for t in &qt {
                score += self.weights[i].get(t).copied().unwrap_or(0);
            }
            if score > 0 {
                hits.push(Hit {
                    name: e.memory.name.clone(),
                    description: e.memory.description.clone(),
                    tier: e.tier.clone(),
                    score,
                    memory: e.memory.clone(),
                });
            }
        }
        hits.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.name.cmp(&b.name))
        });
        if limit != 0 && hits.len() > limit {
            hits.truncate(limit);
        }
        hits
    }

    /// The highest-scoring existing entry for a candidate's name+description, or
    /// `None` if nothing overlaps. The dedup pre-filter on add (the cheap,
    /// no-model first pass). Mirrors Go's `index.BestDuplicate`.
    #[must_use]
    pub fn best_duplicate(&self, cand: &Memory) -> Option<Hit> {
        let q = format!("{} {}", cand.name, cand.description);
        self.query(&q, 0).into_iter().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store::MemoryType;

    fn mem(name: &str, desc: &str, body: &str) -> Memory {
        Memory {
            name: name.to_string(),
            description: desc.to_string(),
            mem_type: MemoryType::Project,
            body: body.to_string(),
        }
    }

    fn entry(m: Memory) -> IndexEntry {
        IndexEntry {
            tier: Tier::machine(),
            memory: m,
        }
    }

    #[test]
    fn query_ranks_by_keyword() {
        let ix = Index::new(vec![
            entry(mem(
                "omega-deploy",
                "omega sing-box proxy NixOS service deploy",
                "TUN cutover",
            )),
            entry(mem(
                "postgres-upgrade",
                "postgresql major version dump restore",
                "pg_dumpall",
            )),
            entry(mem(
                "cowboy-deploy",
                "cowboy nixos switch restarts approval channel",
                "deploy",
            )),
        ]);

        let hits = ix.query("nixos deploy", 0);
        assert!(!hits.is_empty(), "expected hits for 'nixos deploy'");
        for h in &hits {
            assert_ne!(
                h.name, "postgres-upgrade",
                "postgres-upgrade should not match 'nixos deploy'"
            );
        }
        assert!(
            hits[0].name == "omega-deploy" || hits[0].name == "cowboy-deploy",
            "unexpected top hit: {}",
            hits[0].name
        );
    }

    #[test]
    fn query_no_overlap_empty() {
        let ix = Index::new(vec![entry(mem("a", "totally unrelated", "x"))]);
        assert!(ix.query("zzz quantum kangaroo", 0).is_empty());
    }

    #[test]
    fn query_respects_limit() {
        let ix = Index::new(vec![
            entry(mem("omega-deploy", "deploy nixos", "x")),
            entry(mem("cowboy-deploy", "deploy nixos", "y")),
        ]);
        assert_eq!(ix.query("deploy", 1).len(), 1);
        assert_eq!(ix.query("deploy", 0).len(), 2);
    }

    #[test]
    fn best_duplicate() {
        let ix = Index::new(vec![entry(mem(
            "user-runs-nixos-switch",
            "the user runs nixos-rebuild switch themselves",
            "",
        ))]);
        let dup = ix
            .best_duplicate(&mem(
                "user-switches-nixos",
                "user runs the nixos switch personally",
                "",
            ))
            .expect("expected a duplicate candidate");
        assert_eq!(dup.name, "user-runs-nixos-switch");
    }
}
