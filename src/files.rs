//! File-reference search for the composer's `@` picker.
//!
//! Given a session's working directory and a query, returns a ranked list of
//! relative file paths. The walk is gitignore-aware (so build / vendor trees
//! like `target/` and `node_modules/` never show up), hidden files are
//! skipped, and ranking is fuzzy (nucleo, the matcher Helix / Zed use) when
//! there's a query, or a "most useful first" heuristic (shallow + recently
//! modified) when the query is empty.

use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use ignore::WalkBuilder;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// Hard cap on files collected from the walk, bounding latency on huge trees.
/// Gitignore pruning already removes the heavy directories; this is a backstop
/// for an un-ignored monorepo so a keystroke can't stall the runtime thread.
const MAX_SCANNED: usize = 20_000;

const DEFAULT_TREE_EXCLUSIONS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    "coverage",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
}

struct Entry {
    /// Path relative to the session cwd, forward-slashed for display + match.
    rel: String,
    /// Directory nesting (0 = top-level file). Used to bias toward "central"
    /// files in both the empty-query and tie-break orderings.
    depth: usize,
    mtime: SystemTime,
}

/// Walk `root` and return up to `limit` relative file paths ranked for `query`.
/// Blocking (does filesystem I/O) — call it from `spawn_blocking`.
#[must_use]
pub fn search(root: &Path, query: &str, limit: usize) -> Vec<String> {
    let entries = collect(root);
    let trimmed = query.trim();
    if trimmed.is_empty() {
        rank_common(entries, limit)
    } else {
        rank_fuzzy(&entries, trimmed, limit)
    }
}

/// Return one stable, sorted directory page for a lazy tree UI.
///
/// The boolean is true when more matching entries existed than the requested
/// limit. The path is canonicalized before scanning to contain symlink escapes.
pub fn directory(
    root: &Path,
    relative: &str,
    limit: usize,
) -> Result<(Vec<DirectoryEntry>, bool), &'static str> {
    let relative = safe_relative_path(relative)?;
    let target = root.join(&relative);
    let canonical_root = root.canonicalize().map_err(|_| "workspace unavailable")?;
    let canonical_target = target.canonicalize().map_err(|_| "directory not found")?;
    if !canonical_target.starts_with(&canonical_root) || !canonical_target.is_dir() {
        return Err("directory not found");
    }

    let mut entries = Vec::new();
    let walk = WalkBuilder::new(&canonical_target)
        .parents(true)
        .max_depth(Some(1))
        .build();
    for dirent in walk.flatten().skip(1) {
        let name = dirent.file_name().to_string_lossy();
        if DEFAULT_TREE_EXCLUSIONS.contains(&name.as_ref()) {
            continue;
        }
        let Some(file_type) = dirent.file_type() else {
            continue;
        };
        let path = relative.join(dirent.file_name());
        entries.push(DirectoryEntry {
            name: name.into_owned(),
            path: path.to_string_lossy().replace('\\', "/"),
            is_directory: file_type.is_dir(),
        });
    }
    entries.sort_by(|a, b| {
        b.is_directory
            .cmp(&a.is_directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let truncated = entries.len() > limit;
    entries.truncate(limit);
    Ok((entries, truncated))
}

fn safe_relative_path(relative: &str) -> Result<PathBuf, &'static str> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
            && !relative.is_empty()
    {
        return Err("invalid directory");
    }
    Ok(path.to_path_buf())
}

fn collect(root: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    // Defaults already respect .gitignore / .git/info/exclude and skip hidden
    // entries; `parents(true)` also honours a .gitignore above `root` (e.g. a
    // worktree opened inside a larger ignored tree).
    let walk = WalkBuilder::new(root).parents(true).build();
    for dirent in walk.flatten() {
        if out.len() >= MAX_SCANNED {
            break;
        }
        // Files only — referencing a directory isn't useful to the agent.
        if !dirent.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = dirent.path();
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.is_empty() {
            continue;
        }
        let depth = rel.components().count().saturating_sub(1);
        let mtime = dirent
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        out.push(Entry {
            rel: rel_str,
            depth,
            mtime,
        });
    }
    out
}

// Empty-query ordering: shallow files first (the top-level README, Cargo.toml,
// flake.nix are what you reference most), then most-recently-modified within a
// depth — recency is a good proxy for "what I'm working on right now".
fn rank_common(mut entries: Vec<Entry>, limit: usize) -> Vec<String> {
    entries.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| b.mtime.cmp(&a.mtime))
            .then_with(|| a.rel.cmp(&b.rel))
    });
    entries.into_iter().take(limit).map(|e| e.rel).collect()
}

fn rank_fuzzy(entries: &[Entry], query: &str, limit: usize) -> Vec<String> {
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, &Entry)> = entries
        .iter()
        .filter_map(|e| {
            let haystack = Utf32Str::new(&e.rel, &mut buf);
            pattern.score(haystack, &mut matcher).map(|s| (s, e))
        })
        .collect();
    // Highest score first; tie-break toward shallower then shorter paths so the
    // most "central" match wins, then lexical for a stable order.
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.depth.cmp(&b.1.depth))
            .then_with(|| a.1.rel.len().cmp(&b.1.rel.len()))
            .then_with(|| a.1.rel.cmp(&b.1.rel))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, e)| e.rel.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{directory, search};
    use std::fs;
    use std::path::PathBuf;

    // A uniquely-named scratch tree (per test name, so parallel tests don't
    // share a directory) with a known shape. No `.git`, so this exercises the
    // ranking, not gitignore — real session cwds are git repos where the walk's
    // default gitignore handling applies.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cowboy-files-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("deep/a/b")).unwrap();
        fs::write(dir.join("README.md"), "r").unwrap();
        fs::write(dir.join("src/main.rs"), "m").unwrap();
        fs::write(dir.join("src/files.rs"), "f").unwrap();
        fs::write(dir.join("deep/a/b/c.txt"), "c").unwrap();
        dir
    }

    #[test]
    fn empty_query_puts_shallow_files_first() {
        let dir = scratch("empty");
        let out = search(&dir, "", 10);
        assert_eq!(out.first().map(String::as_str), Some("README.md"));
        let readme = out.iter().position(|p| p == "README.md").unwrap();
        let deep = out.iter().position(|p| p == "deep/a/b/c.txt").unwrap();
        assert!(readme < deep, "shallow file must outrank the deep one");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fuzzy_query_ranks_subsequence_matches() {
        let dir = scratch("fuzzy");
        let out = search(&dir, "files", 10);
        assert_eq!(out.first().map(String::as_str), Some("src/files.rs"));
        assert!(
            !out.iter().any(|p| p == "README.md"),
            "non-matching paths are dropped, not just deprioritised",
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn directory_is_shallow_sorted_and_reports_truncation() {
        let dir = scratch("tree");
        let (out, truncated) = directory(&dir, "", 2).unwrap();
        assert_eq!(
            out.iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["deep", "src",]
        );
        assert!(truncated);
        let (src, truncated) = directory(&dir, "src", 10).unwrap();
        assert_eq!(
            src.iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["files.rs", "main.rs"]
        );
        assert!(!truncated);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn directory_rejects_parent_traversal_and_skips_heavy_defaults() {
        let dir = scratch("safe");
        fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        fs::write(dir.join("node_modules/pkg/index.js"), "x").unwrap();
        let (out, _) = directory(&dir, "", 20).unwrap();
        assert!(!out.iter().any(|entry| entry.name == "node_modules"));
        assert_eq!(directory(&dir, "../", 20), Err("invalid directory"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
