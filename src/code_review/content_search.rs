//! Bounded content search over one worktree (`search@1`).
//!
//! It runs no project code and needs no warm index, so it also serves
//! restricted worktrees. The walk is gitignore-aware, skips hidden entries,
//! never follows symlinks and stays inside the worktree root: unlike the Code
//! tree it does not project registered checkouts of an aggregate worktree.
//! Every dimension is bounded (files, bytes, time, matches, line length); a
//! bound that stops the scan is reported, so an empty or short answer is never
//! presented as complete.

use std::io::Read as _;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const DEFAULT_LIMIT: usize = 100;
pub const MAX_LIMIT: usize = 500;
pub const MAX_QUERY_BYTES: usize = 256;
const MAX_PER_FILE: usize = 20;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 50_000;
const MAX_LINE_CHARS: usize = 400;
const DEADLINE: Duration = Duration::from_secs(4);
/// Same binary heuristic as Git: a NUL in the first 8000 bytes.
const BINARY_PROBE: usize = 8000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContentQuery {
    pub query: String,
    /// Treat `query` as a regular expression instead of literal text.
    #[serde(default)]
    pub regex: bool,
    /// Case sensitivity; `None` is smart case (sensitive only with an uppercase letter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_sensitive: Option<bool>,
    /// Worktree-relative path prefix that limits the search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentMatch {
    pub path: String,
    /// 1-based line number.
    pub line: usize,
    /// 1-based UTF-16 column of the first match on the line, as editors count.
    pub column: usize,
    pub text: String,
    pub text_truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Limit,
    Files,
    Bytes,
    Deadline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentSearchResult {
    pub matches: Vec<ContentMatch>,
    /// `None` only when every eligible file was scanned.
    pub stopped: Option<StopReason>,
    pub files_scanned: usize,
    /// Files skipped as binary or over the per-file size bound.
    pub files_skipped: usize,
    /// Files whose further matches were omitted after the per-file bound.
    pub files_capped: usize,
}

fn matcher(query: &ContentQuery) -> Result<regex::Regex, String> {
    if query.query.is_empty() || query.query.len() > MAX_QUERY_BYTES {
        return Err(format!("query must be 1 to {MAX_QUERY_BYTES} bytes"));
    }
    if query.query.chars().any(|c| c == '\n' || c == '\r') {
        return Err("query must be a single line".to_owned());
    }
    let pattern = if query.regex {
        query.query.clone()
    } else {
        regex::escape(&query.query)
    };
    let sensitive = query
        .case_sensitive
        .unwrap_or_else(|| query.query.chars().any(char::is_uppercase));
    regex::RegexBuilder::new(&pattern)
        .case_insensitive(!sensitive)
        .size_limit(1 << 20)
        .dfa_size_limit(1 << 20)
        .build()
        .map_err(|error| format!("invalid pattern: {error}"))
}

fn start(root: &Path, prefix: Option<&str>) -> Result<std::path::PathBuf, String> {
    let Some(prefix) = prefix.filter(|prefix| !prefix.is_empty()) else {
        return Ok(root.to_path_buf());
    };
    let relative = Path::new(prefix.trim_end_matches('/'));
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("invalid search path".to_owned());
    }
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    let canonical = root
        .join(relative)
        .canonicalize()
        .map_err(|_| "search path does not exist".to_owned())?;
    if !canonical.starts_with(&canonical_root) {
        return Err("search path is outside the worktree".to_owned());
    }
    Ok(root.join(relative))
}

fn line_match(line: &str, pattern: &regex::Regex) -> Option<(usize, String, bool)> {
    let found = pattern.find(line)?;
    let column = line[..found.start()].encode_utf16().count() + 1;
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let mut text: String = trimmed.chars().take(MAX_LINE_CHARS).collect();
    let truncated = text.len() < trimmed.len();
    if truncated {
        text.push('…');
    }
    Some((column, text, truncated))
}

/// # Errors
/// An invalid query or search path. Bounds never produce an error.
pub fn search(root: &Path, query: &ContentQuery) -> Result<ContentSearchResult, String> {
    let pattern = matcher(query)?;
    let limit = query.limit.clamp(1, MAX_LIMIT);
    let walk = ignore::WalkBuilder::new(start(root, query.path.as_deref())?)
        .follow_links(false)
        .hidden(true)
        .parents(true)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();
    let deadline = Instant::now() + DEADLINE;
    let mut result = ContentSearchResult {
        matches: Vec::new(),
        stopped: None,
        files_scanned: 0,
        files_skipped: 0,
        files_capped: 0,
    };
    let mut bytes = 0_u64;
    for entry in walk.flatten() {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if Instant::now() >= deadline {
            result.stopped = Some(StopReason::Deadline);
            break;
        }
        if result.files_scanned >= MAX_FILES {
            result.stopped = Some(StopReason::Files);
            break;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let Some(relative) = relative.to_str() else {
            continue;
        };
        let size = entry.metadata().map_or(0, |metadata| metadata.len());
        if size > MAX_FILE_BYTES {
            result.files_skipped += 1;
            continue;
        }
        if bytes + size > MAX_TOTAL_BYTES {
            result.stopped = Some(StopReason::Bytes);
            break;
        }
        let mut content = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        let Ok(file) = std::fs::File::open(entry.path()) else {
            continue;
        };
        if file.take(MAX_FILE_BYTES).read_to_end(&mut content).is_err() {
            continue;
        }
        bytes += content.len() as u64;
        result.files_scanned += 1;
        if content[..content.len().min(BINARY_PROBE)].contains(&0) {
            result.files_skipped += 1;
            continue;
        }
        let text = String::from_utf8_lossy(&content);
        let mut in_file = 0;
        for (index, line) in text.split('\n').enumerate() {
            let Some((column, line_text, text_truncated)) = line_match(line, &pattern) else {
                continue;
            };
            if result.matches.len() >= limit {
                result.stopped = Some(StopReason::Limit);
                return Ok(result);
            }
            result.matches.push(ContentMatch {
                path: relative.to_owned(),
                line: index + 1,
                column,
                text: line_text,
                text_truncated,
            });
            in_file += 1;
            if in_file >= MAX_PER_FILE {
                result.files_capped += 1;
                break;
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
