//! Stable Cowboy Code data provider.
//!
//! HTTP handlers depend on these product-level values rather than Git or Zed
//! wire types. The local provider is intentionally replaceable by a
//! version-pinned Zed adapter without changing the browser contract.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

const MAX_CHANGES: usize = 1_000;
const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeList {
    pub head: Option<String>,
    pub changes: Vec<CodeChange>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffDocument {
    pub path: String,
    pub text: String,
    pub added: usize,
    pub removed: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDocument {
    pub path: String,
    pub text: String,
    pub size: u64,
    pub truncated: bool,
}

pub struct LocalCodeProvider<'a> {
    root: &'a Path,
}

impl<'a> LocalCodeProvider<'a> {
    #[must_use]
    pub fn new(root: &'a Path) -> Self {
        Self { root }
    }

    pub fn changes(&self) -> Result<ChangeList, String> {
        ensure_git_worktree(self.root)?;
        let output = git_output(
            self.root,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            4 * 1024 * 1024,
        )?;
        let mut fields = output
            .split(|byte| *byte == 0)
            .filter(|field| !field.is_empty());
        let mut changes = Vec::new();
        while let Some(field) = fields.next() {
            if field.len() < 4 {
                continue;
            }
            let xy = &field[..2];
            let path = String::from_utf8_lossy(&field[3..]).into_owned();
            let renamed = xy.contains(&b'R') || xy.contains(&b'C');
            let old_path = if renamed {
                fields
                    .next()
                    .map(|value| String::from_utf8_lossy(value).into_owned())
            } else {
                None
            };
            changes.push(CodeChange {
                path,
                old_path,
                status: classify_status(xy),
            });
            if changes.len() > MAX_CHANGES {
                break;
            }
        }
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        let truncated = changes.len() > MAX_CHANGES;
        changes.truncate(MAX_CHANGES);
        let head = git_output(self.root, &["rev-parse", "--short=12", "HEAD"], 128)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        Ok(ChangeList {
            head,
            changes,
            truncated,
        })
    }

    pub fn diff(
        &self,
        relative: &str,
        context: usize,
        show_whitespace: bool,
    ) -> Result<DiffDocument, String> {
        let relative = safe_relative(relative)?;
        ensure_git_worktree(self.root)?;
        let path = relative.to_string_lossy().replace('\\', "/");
        let mut args = vec![
            "diff".to_owned(),
            "--no-ext-diff".to_owned(),
            "--no-color".to_owned(),
            format!("--unified={}", context.min(100)),
        ];
        if !show_whitespace {
            args.push("--ignore-all-space".to_owned());
        }
        args.extend(["HEAD".to_owned(), "--".to_owned(), path.clone()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut bytes = git_output(self.root, &refs, MAX_DIFF_BYTES + 1)?;
        if bytes.is_empty() && !git_path_is_tracked(self.root, &path) {
            bytes = untracked_diff(self.root, &relative, MAX_DIFF_BYTES + 1)?;
        }
        let truncated = bytes.len() > MAX_DIFF_BYTES;
        bytes.truncate(MAX_DIFF_BYTES);
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let (added, removed) = count_diff_lines(&text);
        Ok(DiffDocument {
            path,
            text,
            added,
            removed,
            truncated,
        })
    }

    pub fn file(&self, relative: &str) -> Result<FileDocument, String> {
        let relative = safe_relative(relative)?;
        let canonical_root = self
            .root
            .canonicalize()
            .map_err(|error| format!("workspace unavailable: {error}"))?;
        let canonical_file = self
            .root
            .join(&relative)
            .canonicalize()
            .map_err(|_| "file not found".to_owned())?;
        if !canonical_file.starts_with(&canonical_root) || !canonical_file.is_file() {
            return Err("file not found".to_owned());
        }
        let file = File::open(&canonical_file).map_err(|error| error.to_string())?;
        let size = file.metadata().map_err(|error| error.to_string())?.len();
        let mut bytes = Vec::with_capacity((size as usize).min(MAX_FILE_BYTES + 1));
        file.take((MAX_FILE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.contains(&0) {
            return Err("binary file".to_owned());
        }
        let truncated = bytes.len() > MAX_FILE_BYTES;
        bytes.truncate(MAX_FILE_BYTES);
        while std::str::from_utf8(&bytes).is_err() && !bytes.is_empty() {
            bytes.pop();
        }
        Ok(FileDocument {
            path: relative.to_string_lossy().replace('\\', "/"),
            text: String::from_utf8(bytes).map_err(|error| error.to_string())?,
            size,
            truncated,
        })
    }
}

fn classify_status(xy: &[u8]) -> ChangeStatus {
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

fn safe_relative(relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("invalid path".to_owned());
    }
    Ok(path.to_path_buf())
}

fn ensure_git_worktree(root: &Path) -> Result<(), String> {
    git_output(root, &["rev-parse", "--is-inside-work-tree"], 32)
        .and_then(|output| {
            if output.starts_with(b"true") {
                Ok(output)
            } else {
                Err("not a git worktree".to_owned())
            }
        })
        .map(|_| ())
}

fn git_path_is_tracked(root: &Path, path: &str) -> bool {
    git_output(root, &["ls-files", "--error-unmatch", "--", path], 1024).is_ok()
}

fn git_output(root: &Path, args: &[&str], limit: usize) -> Result<Vec<u8>, String> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("starting git: {error}"))?;
    let mut stdout = child.stdout.take().ok_or("missing git stdout")?;
    let mut bytes = Vec::new();
    stdout
        .by_ref()
        .take(limit as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("reading git output: {error}"))?;
    drop(stdout);
    let output = child
        .wait_with_output()
        .map_err(|error| format!("waiting for git: {error}"))?;
    if output.status.success() {
        Ok(bytes)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

fn untracked_diff(root: &Path, relative: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let canonical_root = root.canonicalize().map_err(|error| error.to_string())?;
    let path = root.join(relative);
    let canonical_file = path
        .canonicalize()
        .map_err(|_| "file not found".to_owned())?;
    if !canonical_file.starts_with(canonical_root) || !canonical_file.is_file() {
        return Err("file not found".to_owned());
    }
    let file = File::open(canonical_file).map_err(|error| error.to_string())?;
    let mut content = String::new();
    file.take(limit as u64)
        .read_to_string(&mut content)
        .map_err(|_| "binary file".to_owned())?;
    let display = relative.to_string_lossy().replace('\\', "/");
    let mut output = Vec::new();
    writeln!(output, "diff --git a/{display} b/{display}").map_err(|error| error.to_string())?;
    writeln!(output, "new file mode 100644").map_err(|error| error.to_string())?;
    writeln!(output, "--- /dev/null").map_err(|error| error.to_string())?;
    writeln!(output, "+++ b/{display}").map_err(|error| error.to_string())?;
    writeln!(output, "@@ -0,0 +1,{} @@", content.lines().count())
        .map_err(|error| error.to_string())?;
    for line in content.lines() {
        writeln!(output, "+{line}").map_err(|error| error.to_string())?;
        if output.len() >= limit {
            break;
        }
    }
    output.truncate(limit);
    Ok(output)
}

fn count_diff_lines(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            added += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            removed += 1;
        }
    }
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::{ChangeStatus, LocalCodeProvider};
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cowboy-code-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "cowboy@example.invalid"])
            .current_dir(&dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Cowboy Test"])
            .current_dir(&dir)
            .status()
            .unwrap();
        fs::write(dir.join("tracked.rs"), "fn old() {}\n").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(&dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-qm", "initial"])
            .current_dir(&dir)
            .status()
            .unwrap();
        dir
    }

    #[test]
    fn local_provider_lists_changes_and_builds_diffs() {
        let dir = scratch("changes");
        fs::write(dir.join("tracked.rs"), "fn new() {}\n").unwrap();
        fs::write(dir.join("new.txt"), "hello\n").unwrap();
        let provider = LocalCodeProvider::new(&dir);
        let changes = provider.changes().unwrap();
        assert_eq!(changes.changes.len(), 2);
        assert!(changes.changes.iter().any(|change| {
            change.path == "tracked.rs" && change.status == ChangeStatus::Modified
        }));
        assert!(changes.changes.iter().any(|change| {
            change.path == "new.txt" && change.status == ChangeStatus::Untracked
        }));
        let tracked = provider.diff("tracked.rs", 3, true).unwrap();
        assert!(tracked.text.contains("-fn old() {}"));
        assert!(tracked.text.contains("+fn new() {}"));
        let untracked = provider.diff("new.txt", 3, true).unwrap();
        assert!(untracked.text.contains("+hello"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_reads_are_bounded_to_the_worktree() {
        let dir = scratch("files");
        let provider = LocalCodeProvider::new(&dir);
        assert_eq!(provider.file("tracked.rs").unwrap().text, "fn old() {}\n");
        assert!(provider.file("../secret").is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
