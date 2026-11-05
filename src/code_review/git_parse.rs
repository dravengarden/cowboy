//! Parsers for git plumbing output: status codes, history, worktrees,
//! numstat, commit file lists and diff line counts.

use super::*;

pub(super) fn has_staged_change(xy: &[u8]) -> bool {
    xy.first()
        .is_some_and(|value| !matches!(value, b' ' | b'?'))
}

pub(super) fn has_unstaged_change(xy: &[u8]) -> bool {
    xy == b"??" || xy.get(1).is_some_and(|value| !matches!(value, b' '))
}

pub(super) fn classify_status(xy: &[u8]) -> ChangeStatus {
    if xy.iter().any(|value| matches!(value, b'U')) || matches!(xy, b"AA" | b"DD") {
        ChangeStatus::Conflicted
    } else if xy == b"??" {
        ChangeStatus::Untracked
    } else if xy.contains(&b'R') || xy.contains(&b'C') {
        ChangeStatus::Renamed
    } else if xy.contains(&b'D') {
        ChangeStatus::Deleted
    } else if xy.contains(&b'A') {
        ChangeStatus::Added
    } else {
        ChangeStatus::Modified
    }
}

pub(super) fn parse_git_history(bytes: &[u8]) -> Vec<GitCommitSummary> {
    bytes
        .split(|byte| *byte == 0x1e)
        .filter_map(|record| {
            let record = record.strip_prefix(b"\n").unwrap_or(record);
            if record.is_empty() {
                return None;
            }
            let mut fields = record.splitn(6, |byte| *byte == 0x1f);
            let oid = String::from_utf8_lossy(fields.next()?).trim().to_owned();
            let parents = String::from_utf8_lossy(fields.next()?)
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            let author = String::from_utf8_lossy(fields.next()?).to_string();
            let authored_at = String::from_utf8_lossy(fields.next()?).to_string();
            let subject = String::from_utf8_lossy(fields.next()?).to_string();
            let decorations = String::from_utf8_lossy(fields.next()?)
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect();
            Some(GitCommitSummary {
                oid,
                parents,
                author,
                authored_at,
                subject,
                decorations,
            })
        })
        .collect()
}

pub(super) fn parse_git_worktrees(bytes: &[u8], current: &Path) -> Vec<GitWorktreeSummary> {
    String::from_utf8_lossy(bytes)
        .split("\n\n")
        .filter_map(|record| {
            let mut worktree = GitWorktreeSummary {
                path: String::new(),
                head: None,
                branch: None,
                bare: false,
                detached: false,
                locked: None,
                prunable: None,
                current: false,
            };
            for line in record.lines() {
                let (key, value) = line.split_once(' ').unwrap_or((line, ""));
                match key {
                    "worktree" => worktree.path = value.to_owned(),
                    "HEAD" => worktree.head = Some(value.to_owned()),
                    "branch" => {
                        worktree.branch = Some(
                            value
                                .strip_prefix("refs/heads/")
                                .unwrap_or(value)
                                .to_owned(),
                        );
                    }
                    "bare" => worktree.bare = true,
                    "detached" => worktree.detached = true,
                    "locked" => worktree.locked = Some(value.to_owned()),
                    "prunable" => worktree.prunable = Some(value.to_owned()),
                    _ => {}
                }
            }
            if worktree.path.is_empty() {
                return None;
            }
            worktree.current = Path::new(&worktree.path)
                .canonicalize()
                .is_ok_and(|path| path == current);
            Some(worktree)
        })
        .collect()
}

pub(super) fn apply_line_counts(change: &mut CodeChange, counts: &LineCounts) {
    if let Some((additions, deletions)) = counts.get(&change.path) {
        change.additions = Some(*additions);
        change.deletions = Some(*deletions);
    }
}

/// `git diff --numstat -z`: `added\tremoved\tpath\0`, or for a rename an empty
/// path followed by `old\0new\0`. Binary files count `-` and are left out.
pub(super) fn parse_numstat(bytes: &[u8]) -> LineCounts {
    let mut counts = LineCounts::new();
    let mut fields = bytes.split(|byte| *byte == 0);
    while let Some(field) = fields.next() {
        let text = String::from_utf8_lossy(field);
        let mut columns = text.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) =
            (columns.next(), columns.next(), columns.next())
        else {
            continue;
        };
        let path = if path.is_empty() {
            fields.next();
            let Some(renamed) = fields.next() else { break };
            String::from_utf8_lossy(renamed).into_owned()
        } else {
            path.to_owned()
        };
        if let (Ok(added), Ok(removed)) = (added.parse(), removed.parse()) {
            counts.insert(path, (added, removed));
        }
    }
    counts
}

pub(super) fn parse_commit_files(bytes: &[u8]) -> Vec<GitCommitFile> {
    let mut fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut files = Vec::new();
    while let Some(raw_status) = fields.next() {
        let status_code = raw_status.first().copied().unwrap_or(b'M');
        let Some(first_path) = fields.next() else {
            break;
        };
        let (old_path, path) = if matches!(status_code, b'R' | b'C') {
            let Some(next_path) = fields.next() else {
                break;
            };
            (
                Some(String::from_utf8_lossy(first_path).to_string()),
                String::from_utf8_lossy(next_path).to_string(),
            )
        } else {
            (None, String::from_utf8_lossy(first_path).to_string())
        };
        let status = match status_code {
            b'A' => ChangeStatus::Added,
            b'D' => ChangeStatus::Deleted,
            b'R' | b'C' => ChangeStatus::Renamed,
            _ => ChangeStatus::Modified,
        };
        files.push(GitCommitFile {
            path,
            old_path,
            status,
        });
    }
    files
}

pub(super) fn count_diff_lines(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    let mut in_hunk = false;
    for line in diff.lines() {
        if line.starts_with("diff ") {
            in_hunk = false;
        } else if line.starts_with("@@") {
            in_hunk = true;
        } else if in_hunk && line.starts_with('+') {
            added += 1;
        } else if in_hunk && line.starts_with('-') {
            removed += 1;
        }
    }
    (added, removed)
}
