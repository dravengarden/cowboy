//! Stable Cowboy Code data provider.
//!
//! HTTP handlers depend on these product-level values rather than Git or Zed
//! wire types. The local provider is intentionally replaceable by a
//! version-pinned Zed adapter without changing the browser contract.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::Digest as _;

const MAX_CHANGES: usize = 1_000;
const MAX_DIFF_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const FILE_PAGE_BYTES: usize = 256 * 1024;
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub staged: bool,
    pub unstaged: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffScope {
    Combined,
    Staged,
    Unstaged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeList {
    pub head: Option<String>,
    pub revision: String,
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
    pub revision: String,
    pub text: String,
    pub size: u64,
    pub truncated: bool,
    pub next_cursor: Option<String>,
    pub limited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeManifest {
    pub provider: &'static str,
    pub revision: String,
    pub head: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeTreeEntry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeTreePage {
    pub entries: Vec<CodeTreeEntry>,
    pub truncated: bool,
}

pub trait CodeProvider {
    fn manifest(&self) -> Result<WorktreeManifest, String>;
    fn directory(&self, relative: &str, limit: usize) -> Result<CodeTreePage, String>;
    fn search(&self, query: &str, limit: usize) -> Vec<String>;
    fn changes(&self) -> Result<ChangeList, String>;
    fn diff_snapshot(
        &self,
        relative: &str,
        context: usize,
        show_whitespace: bool,
        scope: DiffScope,
    ) -> Result<DiffDocument, String>;
    fn file_page(&self, relative: &str, cursor: Option<&str>) -> Result<FileDocument, String>;
}

pub struct LocalCodeProvider {
    root: PathBuf,
}

impl LocalCodeProvider {
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    fn head(&self) -> Option<String> {
        git_output(&self.root, &["rev-parse", "--short=12", "HEAD"], 128)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn worktree_revision(head: Option<&str>, status: &[u8]) -> String {
        let mut digest = sha2::Sha256::new();
        if let Some(head) = head {
            digest.update(head.as_bytes());
        }
        digest.update([0]);
        digest.update(status);
        format!("{:x}", digest.finalize())
    }

    #[cfg(test)]
    fn file(&self, relative: &str) -> Result<FileDocument, String> {
        self.file_page(relative, None)
    }
}

impl CodeProvider for LocalCodeProvider {
    fn manifest(&self) -> Result<WorktreeManifest, String> {
        ensure_git_worktree(&self.root)?;
        let status = git_output(
            &self.root,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            4 * 1024 * 1024,
        )?;
        let head = self.head();
        Ok(WorktreeManifest {
            provider: "local",
            revision: Self::worktree_revision(head.as_deref(), &status),
            head,
        })
    }

    fn directory(&self, relative: &str, limit: usize) -> Result<CodeTreePage, String> {
        let (entries, truncated) =
            crate::files::directory(&self.root, relative, limit).map_err(str::to_owned)?;
        Ok(CodeTreePage {
            entries: entries
                .into_iter()
                .map(|entry| CodeTreeEntry {
                    name: entry.name,
                    path: entry.path,
                    is_directory: entry.is_directory,
                })
                .collect(),
            truncated,
        })
    }

    fn search(&self, query: &str, limit: usize) -> Vec<String> {
        crate::files::search(&self.root, query, limit)
    }

    fn changes(&self) -> Result<ChangeList, String> {
        ensure_git_worktree(&self.root)?;
        let output = git_output(
            &self.root,
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
                staged: has_staged_change(xy),
                unstaged: has_unstaged_change(xy),
            });
            if changes.len() > MAX_CHANGES {
                break;
            }
        }
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        let truncated = changes.len() > MAX_CHANGES;
        changes.truncate(MAX_CHANGES);
        let head = self.head();
        Ok(ChangeList {
            revision: Self::worktree_revision(head.as_deref(), &output),
            head,
            changes,
            truncated,
        })
    }

    fn diff_snapshot(
        &self,
        relative: &str,
        context: usize,
        show_whitespace: bool,
        scope: DiffScope,
    ) -> Result<DiffDocument, String> {
        let relative = safe_relative(relative)?;
        ensure_git_worktree(&self.root)?;
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
        match scope {
            DiffScope::Combined => args.push("HEAD".to_owned()),
            DiffScope::Staged => {
                args.push("--cached".to_owned());
                args.push("HEAD".to_owned());
            }
            DiffScope::Unstaged => {}
        }
        args.extend(["--".to_owned(), path.clone()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut bytes = git_output(&self.root, &refs, MAX_DIFF_SNAPSHOT_BYTES + 1)?;
        if bytes.is_empty() && scope != DiffScope::Staged && !git_path_is_tracked(&self.root, &path)
        {
            bytes = untracked_diff(&self.root, &relative, MAX_DIFF_SNAPSHOT_BYTES + 1)?;
        }
        let truncated = bytes.len() > MAX_DIFF_SNAPSHOT_BYTES;
        bytes.truncate(MAX_DIFF_SNAPSHOT_BYTES);
        while std::str::from_utf8(&bytes).is_err() && !bytes.is_empty() {
            bytes.pop();
        }
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

    fn file_page(&self, relative: &str, cursor: Option<&str>) -> Result<FileDocument, String> {
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
        let mut file = File::open(&canonical_file).map_err(|error| error.to_string())?;
        let size = file.metadata().map_err(|error| error.to_string())?.len();
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        let revision = file_revision(&relative, &metadata);
        let offset = match cursor {
            Some(cursor) => {
                let (cursor_revision, offset) = parse_file_cursor(cursor)?;
                if cursor_revision != revision {
                    return Err("file snapshot changed".to_owned());
                }
                offset
            }
            None => 0,
        };
        let available = (size as usize).min(MAX_FILE_BYTES);
        if offset >= available && !(offset == 0 && available == 0) {
            return Err("invalid file cursor".to_owned());
        }
        file.seek(SeekFrom::Start(offset as u64))
            .map_err(|error| error.to_string())?;
        let read_limit = available.saturating_sub(offset).min(FILE_PAGE_BYTES + 4);
        let mut bytes = Vec::with_capacity(read_limit);
        file.take(read_limit as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.contains(&0) {
            return Err("binary file".to_owned());
        }
        let valid_len = match std::str::from_utf8(&bytes) {
            Ok(_) => bytes.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => return Err("file is not UTF-8".to_owned()),
        };
        let mut end = valid_len.min(FILE_PAGE_BYTES);
        if offset + end < available
            && let Some(newline) = bytes[..end].iter().rposition(|byte| *byte == b'\n')
        {
            end = newline + 1;
        }
        bytes.truncate(end);
        let next_offset = offset + end;
        let next_cursor = (next_offset < available).then(|| format!("{revision}:{next_offset}"));
        let limited = size as usize > MAX_FILE_BYTES;
        Ok(FileDocument {
            path: relative.to_string_lossy().replace('\\', "/"),
            revision,
            text: String::from_utf8(bytes).map_err(|error| error.to_string())?,
            size,
            truncated: next_cursor.is_some() || limited,
            next_cursor,
            limited,
        })
    }
}

fn file_revision(relative: &Path, metadata: &std::fs::Metadata) -> String {
    let mut digest = sha2::Sha256::new();
    digest.update(relative.to_string_lossy().as_bytes());
    for value in [
        metadata.dev(),
        metadata.ino(),
        metadata.size(),
        metadata.mtime() as u64,
        metadata.mtime_nsec() as u64,
        metadata.ctime() as u64,
        metadata.ctime_nsec() as u64,
    ] {
        digest.update(value.to_le_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn parse_file_cursor(cursor: &str) -> Result<(&str, usize), String> {
    let (revision, offset) = cursor
        .rsplit_once(':')
        .ok_or_else(|| "invalid file cursor".to_owned())?;
    if revision.len() != 64 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid file cursor".to_owned());
    }
    let offset = offset
        .parse::<usize>()
        .map_err(|_| "invalid file cursor".to_owned())?;
    Ok((revision, offset))
}

fn has_staged_change(xy: &[u8]) -> bool {
    xy.first()
        .is_some_and(|value| !matches!(value, b' ' | b'?'))
}

fn has_unstaged_change(xy: &[u8]) -> bool {
    xy == b"??" || xy.get(1).is_some_and(|value| !matches!(value, b' '))
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
    if output.status.success() || bytes.len() == limit {
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
    use super::{ChangeStatus, CodeProvider as _, DiffScope, LocalCodeProvider};
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
        assert_eq!(changes.revision, provider.manifest().unwrap().revision);
        assert!(
            provider
                .search("tracked", 20)
                .contains(&"tracked.rs".to_owned())
        );
        assert_eq!(changes.changes.len(), 2);
        assert!(changes.changes.iter().any(|change| {
            change.path == "tracked.rs"
                && change.status == ChangeStatus::Modified
                && !change.staged
                && change.unstaged
        }));
        assert!(changes.changes.iter().any(|change| {
            change.path == "new.txt"
                && change.status == ChangeStatus::Untracked
                && !change.staged
                && change.unstaged
        }));
        let tracked = provider
            .diff_snapshot("tracked.rs", 3, true, DiffScope::Combined)
            .unwrap();
        assert!(tracked.text.contains("-fn old() {}"));
        assert!(tracked.text.contains("+fn new() {}"));
        let untracked = provider
            .diff_snapshot("new.txt", 3, true, DiffScope::Unstaged)
            .unwrap();
        assert!(untracked.text.contains("+hello"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn manifest_revision_changes_with_visible_worktree_state() {
        let dir = scratch("manifest");
        let provider = LocalCodeProvider::new(&dir);
        let clean = provider.manifest().unwrap();
        assert_eq!(clean.provider, "local");
        assert!(clean.head.is_some());
        fs::write(dir.join("tracked.rs"), "fn changed() {}\n").unwrap();
        let changed = provider.manifest().unwrap();
        assert_ne!(clean.revision, changed.revision);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn partial_changes_keep_staged_and_unstaged_views_separate() {
        let dir = scratch("partial");
        fs::write(dir.join("tracked.rs"), "fn staged() {}\n").unwrap();
        Command::new("git")
            .args(["add", "tracked.rs"])
            .current_dir(&dir)
            .status()
            .unwrap();
        fs::write(dir.join("tracked.rs"), "fn unstaged() {}\n").unwrap();

        let provider = LocalCodeProvider::new(&dir);
        let change = provider.changes().unwrap().changes.remove(0);
        assert!(change.staged);
        assert!(change.unstaged);

        let staged = provider
            .diff_snapshot("tracked.rs", 3, true, DiffScope::Staged)
            .unwrap();
        assert!(staged.text.contains("+fn staged() {}"));
        assert!(!staged.text.contains("unstaged"));

        let unstaged = provider
            .diff_snapshot("tracked.rs", 3, true, DiffScope::Unstaged)
            .unwrap();
        assert!(unstaged.text.contains("-fn staged() {}"));
        assert!(unstaged.text.contains("+fn unstaged() {}"));
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

    #[test]
    fn large_files_page_on_lines_and_reject_changed_snapshots() {
        let dir = scratch("large-file");
        let content: String = (0..40_000)
            .map(|index| format!("line {index:05}\n"))
            .collect();
        fs::write(dir.join("large.txt"), &content).unwrap();
        let provider = LocalCodeProvider::new(&dir);
        let first = provider.file("large.txt").unwrap();
        assert!(first.text.ends_with('\n'));
        let cursor = first.next_cursor.clone().expect("second page");
        let second = provider.file_page("large.txt", Some(&cursor)).unwrap();
        assert!(second.text.ends_with('\n'));
        assert_ne!(first.text, second.text);

        fs::write(dir.join("large.txt"), format!("{content}changed\n")).unwrap();
        assert_eq!(
            provider.file_page("large.txt", Some(&cursor)).unwrap_err(),
            "file snapshot changed"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
