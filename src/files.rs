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
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde::Deserialize;

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
    pub ignored: bool,
}

#[derive(Default, Deserialize)]
struct ZedScanSettings {
    file_scan_exclusions: Option<Vec<String>>,
    file_scan_inclusions: Option<Vec<String>>,
}

struct TreeScanPolicy {
    gitignore: Gitignore,
    zed_exclusions: Gitignore,
    zed_inclusions: Gitignore,
}

fn jsonc_to_json(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            output.push(ch);
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for comment in chars.by_ref() {
                if comment == '\n' {
                    output.push('\n');
                    break;
                }
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut previous = '\0';
            for comment in chars.by_ref() {
                if previous == '*' && comment == '/' {
                    break;
                }
                previous = comment;
            }
            continue;
        }
        output.push(ch);
    }

    let mut without_trailing_commas = String::with_capacity(output.len());
    let mut chars = output.chars().peekable();
    in_string = false;
    escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            without_trailing_commas.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            without_trailing_commas.push(ch);
            continue;
        }
        if ch == ',' {
            let mut lookahead = chars.clone();
            if lookahead
                .find(|next| !next.is_whitespace())
                .is_some_and(|next| next == '}' || next == ']')
            {
                continue;
            }
        }
        without_trailing_commas.push(ch);
    }
    without_trailing_commas
}

fn zed_scan_settings(root: &Path) -> ZedScanSettings {
    let path = root.join(".zed/settings.json");
    let Ok(bytes) = std::fs::read(path) else {
        return ZedScanSettings::default();
    };
    if bytes.len() > 1024 * 1024 {
        return ZedScanSettings::default();
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return ZedScanSettings::default();
    };
    serde_json::from_str(&jsonc_to_json(&text)).unwrap_or_default()
}

fn pattern_matcher(root: &Path, patterns: Option<&[String]>) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root);
    for pattern in patterns.unwrap_or_default() {
        let _ = builder.add_line(None, pattern);
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

fn tree_scan_policy(root: &Path, relative: &Path) -> TreeScanPolicy {
    let settings = zed_scan_settings(root);
    let mut git_root = root.to_path_buf();
    let mut candidate = root.to_path_buf();
    for component in relative.components() {
        if let Component::Normal(part) = component {
            candidate.push(part);
            if candidate.join(".git").exists() {
                git_root.clone_from(&candidate);
            }
        }
    }
    let mut gitignore = GitignoreBuilder::new(&git_root);
    let git_exclude = git_root.join(".git/info/exclude");
    if git_exclude.is_file() {
        let _ = gitignore.add(git_exclude);
    }
    let mut directory = git_root.clone();
    let root_ignore = directory.join(".gitignore");
    if root_ignore.is_file() {
        let _ = gitignore.add(root_ignore);
    }
    let nested_relative = relative
        .strip_prefix(
            git_root
                .strip_prefix(root)
                .unwrap_or_else(|_| Path::new("")),
        )
        .unwrap_or(relative);
    for component in nested_relative.components() {
        if let Component::Normal(part) = component {
            directory.push(part);
            let nested = directory.join(".gitignore");
            if nested.is_file() {
                let _ = gitignore.add(nested);
            }
        }
    }
    TreeScanPolicy {
        gitignore: gitignore.build().unwrap_or_else(|_| Gitignore::empty()),
        zed_exclusions: pattern_matcher(root, settings.file_scan_exclusions.as_deref()),
        zed_inclusions: pattern_matcher(root, settings.file_scan_inclusions.as_deref()),
    }
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

/// Return one stable, sorted filesystem directory page for a lazy tree UI.
///
/// The boolean is true when more matching entries existed than the requested
/// limit. Zed `file_scan_exclusions` and Cowboy's heavyweight defaults are hard
/// exclusions. Gitignored entries remain explicitly browsable (matching Zed's
/// project panel), but are marked so the client never speculatively preloads
/// them unless a Zed `file_scan_inclusions` pattern opts them back in.
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
    let policy = tree_scan_policy(&canonical_root, &relative);

    let mut entries = Vec::new();
    let children = std::fs::read_dir(canonical_target).map_err(|_| "directory unavailable")?;
    for dirent in children.flatten() {
        let name = dirent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || DEFAULT_TREE_EXCLUSIONS.contains(&name.as_str()) {
            continue;
        }
        let Ok(file_type) = dirent.file_type() else {
            continue;
        };
        if file_type.is_symlink() && (name == "result" || name.starts_with("result-")) {
            continue;
        }
        let path = relative.join(&name);
        let absolute_path = canonical_root.join(&path);
        let is_directory = file_type.is_dir();
        if policy
            .zed_exclusions
            .matched_path_or_any_parents(&absolute_path, is_directory)
            .is_ignore()
        {
            continue;
        }
        let included = policy
            .zed_inclusions
            .matched_path_or_any_parents(&absolute_path, is_directory)
            .is_ignore();
        let nested_repository = is_directory && absolute_path.join(".git").exists();
        let ignored = !nested_repository
            && !included
            && policy
                .gitignore
                .matched_path_or_any_parents(&absolute_path, is_directory)
                .is_ignore();
        entries.push(DirectoryEntry {
            name,
            path: path.to_string_lossy().replace('\\', "/"),
            is_directory,
            ignored,
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

    #[test]
    fn directory_keeps_gitignored_child_worktrees_visible() {
        let dir = scratch("ignored-worktrees");
        fs::create_dir_all(dir.join("projects/standalone/.git")).unwrap();
        fs::create_dir_all(dir.join("projects/standalone/src")).unwrap();
        fs::write(dir.join(".gitignore"), "/projects/*\n").unwrap();
        fs::write(
            dir.join("projects/standalone/src/lib.rs"),
            "pub fn visible() {}\n",
        )
        .unwrap();

        let (projects, truncated) = directory(&dir, "projects", 20).unwrap();
        assert_eq!(
            projects
                .iter()
                .map(|entry| (entry.name.as_str(), entry.is_directory, entry.ignored))
                .collect::<Vec<_>>(),
            vec![("standalone", true, false)],
        );
        assert!(!truncated);
        let (standalone, _) = directory(&dir, "projects/standalone", 20).unwrap();
        assert!(standalone.iter().any(|entry| entry.name == "src"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn directory_honors_zed_scan_exclusions_and_inclusions() {
        let dir = scratch("zed-policy");
        fs::create_dir_all(dir.join(".zed")).unwrap();
        fs::create_dir_all(dir.join("vendor")).unwrap();
        fs::create_dir_all(dir.join("keep")).unwrap();
        fs::write(dir.join(".gitignore"), "/vendor\n/keep\n").unwrap();
        fs::write(
            dir.join(".zed/settings.json"),
            r#"{
                // Zed settings are JSONC and commonly retain trailing commas.
                "file_scan_exclusions": ["**/vendor",],
                "file_scan_inclusions": ["keep",],
            }"#,
        )
        .unwrap();

        let (entries, _) = directory(&dir, "", 20).unwrap();
        assert!(!entries.iter().any(|entry| entry.name == "vendor"));
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.name == "keep")
                .map(|entry| entry.ignored),
            Some(false),
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn directory_skips_nix_result_links_without_hiding_source() {
        use std::os::unix::fs::symlink;

        let dir = scratch("nix-results");
        symlink(dir.join("src"), dir.join("result")).unwrap();
        symlink(dir.join("src"), dir.join("result-web")).unwrap();

        let (entries, _) = directory(&dir, "", 20).unwrap();
        assert!(entries.iter().any(|entry| entry.name == "src"));
        assert!(!entries.iter().any(|entry| entry.name == "result"));
        assert!(!entries.iter().any(|entry| entry.name == "result-web"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
