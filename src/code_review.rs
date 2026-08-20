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

use serde::{Deserialize, Serialize};
use sha2::Digest as _;

const MAX_CHANGES: usize = 1_000;
const MAX_HISTORY_COMMITS: usize = 128;
const MAX_COMMIT_FILES: usize = 1_000;
const MAX_DIFF_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const FILE_PAGE_BYTES: usize = 256 * 1024;
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
const LOCAL_PROVIDER_REVISION: &[u8] = b"local-v2-project-projection";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
    pub staged: bool,
    pub unstaged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffScope {
    Combined,
    Staged,
    Unstaged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeList {
    pub head: Option<String>,
    pub revision: String,
    pub changes: Vec<CodeChange>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffDocument {
    pub path: String,
    pub text: String,
    pub added: usize,
    pub removed: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDocument {
    pub path: String,
    pub revision: String,
    pub text: String,
    pub size: u64,
    pub truncated: bool,
    pub next_cursor: Option<String>,
    pub limited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawFileDocument {
    pub path: String,
    pub revision: String,
    pub media_type: String,
    #[serde(with = "base64_std")]
    pub bytes: Vec<u8>,
    pub size: u64,
}

mod base64_std {
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(serde::de::Error::custom)
    }
}

pub fn preview_media_type(path: &str) -> Option<&'static str> {
    let ext = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeManifest {
    pub provider: String,
    pub revision: String,
    pub head: Option<String>,
    pub project: String,
    pub branch: Option<String>,
    pub worktree: Option<String>,
    pub change_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitSummary {
    pub oid: String,
    pub parents: Vec<String>,
    pub author: String,
    pub authored_at: String,
    pub subject: String,
    pub decorations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitWorktreeSummary {
    pub path: String,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub bare: bool,
    pub detached: bool,
    pub locked: Option<String>,
    pub prunable: Option<String>,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRepositorySnapshot {
    pub commits: Vec<GitCommitSummary>,
    pub history_truncated: bool,
    pub worktrees: Vec<GitWorktreeSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: ChangeStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitDetail {
    pub oid: String,
    pub parents: Vec<String>,
    pub author: String,
    pub author_email: String,
    pub authored_at: String,
    pub message: String,
    pub files: Vec<GitCommitFile>,
    pub files_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeTreeEntry {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub ignored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeTreePage {
    pub entries: Vec<CodeTreeEntry>,
    pub truncated: bool,
}

pub trait CodeProvider {
    fn manifest(&self) -> Result<WorktreeManifest, String>;
    fn directory(&self, relative: &str, limit: usize) -> Result<CodeTreePage, String>;
    fn search(&self, query: &str, limit: usize) -> Vec<String>;
    fn changes(&self) -> Result<ChangeList, String>;
    fn repository(&self, after: Option<&str>) -> Result<GitRepositorySnapshot, String>;
    fn commit(&self, oid: &str) -> Result<GitCommitDetail, String>;
    fn commit_diff(&self, oid: &str, relative: &str) -> Result<DiffDocument, String>;
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
    aggregate_root: Option<PathBuf>,
}

impl LocalCodeProvider {
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            aggregate_root: aggregate_checkout_root(&root),
            root,
        }
    }

    fn projected_projects(&self) -> Vec<(String, PathBuf)> {
        let Some(aggregate_root) = self.aggregate_root.as_deref() else {
            return Vec::new();
        };
        let definitions = aggregate_root.join("project-defs");
        let projects = aggregate_root.join("projects");
        let Ok(children) = std::fs::read_dir(&definitions) else {
            return Vec::new();
        };
        let mut projected = children
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                if name.starts_with('.')
                    || !entry.path().join("project.toml").is_file()
                    || !projects.join(&name).is_dir()
                {
                    return None;
                }
                Some((name.clone(), projects.join(name)))
            })
            .collect::<Vec<_>>();
        projected.sort_by(|left, right| left.0.cmp(&right.0));
        projected
    }

    /// Map a Code-tree path onto the Zed worktree that can actually open it.
    /// Session-local files stay on this checkout. Aggregate
    /// `projects/<name>/...` files are registered checkouts outside the
    /// session worktree and must be leased there, or hover/outline always
    /// fail with "buffer is not open".
    #[must_use]
    pub fn language_buffer_key(&self, relative: &str) -> Option<(PathBuf, PathBuf)> {
        let relative = Path::new(relative);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return None;
        }
        let local = self.root.join(relative);
        if let Ok(canonical) = local.canonicalize() {
            let Ok(canonical_root) = self.root.canonicalize() else {
                return None;
            };
            if canonical.starts_with(&canonical_root) && canonical.is_file() {
                return Some((canonical_root, relative.to_path_buf()));
            }
        }
        let (project_root, inner) = self.projected_path(relative)?;
        let canonical_root = project_root.canonicalize().ok()?;
        let canonical = project_root.join(&inner).canonicalize().ok()?;
        if !canonical.starts_with(&canonical_root) || !canonical.is_file() {
            return None;
        }
        Some((canonical_root, inner))
    }

    fn projected_path(&self, relative: &Path) -> Option<(PathBuf, PathBuf)> {
        let mut components = relative.components();
        if !matches!(
            components.next()?,
            Component::Normal(component) if component == std::ffi::OsStr::new("projects")
        ) {
            return None;
        }
        let Component::Normal(project) = components.next()? else {
            return None;
        };
        let project = project.to_str()?;
        let (_, source) = self
            .projected_projects()
            .into_iter()
            .find(|(name, _)| name == project)?;
        let inner = components.collect::<PathBuf>();
        Some((source, inner))
    }

    fn resolved_file(&self, relative: &Path) -> Result<PathBuf, String> {
        let local = self.root.join(relative);
        if let Ok(canonical) = local.canonicalize() {
            let canonical_root = self
                .root
                .canonicalize()
                .map_err(|error| format!("workspace unavailable: {error}"))?;
            if canonical.starts_with(canonical_root) && canonical.is_file() {
                return Ok(canonical);
            }
        }
        let (project_root, inner) = self
            .projected_path(relative)
            .ok_or_else(|| "file not found".to_owned())?;
        let canonical_root = project_root
            .canonicalize()
            .map_err(|_| "file not found".to_owned())?;
        let canonical = project_root
            .join(inner)
            .canonicalize()
            .map_err(|_| "file not found".to_owned())?;
        if !canonical.starts_with(canonical_root) || !canonical.is_file() {
            return Err("file not found".to_owned());
        }
        Ok(canonical)
    }

    fn head(&self) -> Option<String> {
        git_output(&self.root, &["rev-parse", "--short=12", "HEAD"], 128)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn branch(&self) -> Option<String> {
        git_output(
            &self.root,
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
            1024,
        )
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    }

    fn repository_identity(&self) -> (String, Option<String>) {
        let top_level = git_output(
            &self.root,
            &["rev-parse", "--path-format=absolute", "--show-toplevel"],
            4096,
        )
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| PathBuf::from(value.trim()));
        let common_dir = git_output(
            &self.root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            4096,
        )
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|value| PathBuf::from(value.trim()));
        let repository_root = common_dir
            .as_deref()
            .and_then(Path::parent)
            .filter(|path| path.file_name().is_some())
            .or(top_level.as_deref())
            .unwrap_or(&self.root);
        let project = repository_root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("worktree")
            .to_owned();
        let worktree = top_level
            .as_deref()
            .filter(|path| *path != repository_root)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .map(str::to_owned);
        (project, worktree)
    }

    fn worktree_revision(head: Option<&str>, status: &[u8]) -> String {
        let mut digest = sha2::Sha256::new();
        digest.update(LOCAL_PROVIDER_REVISION);
        digest.update([0]);
        if let Some(head) = head {
            digest.update(head.as_bytes());
        }
        digest.update([0]);
        digest.update(status);
        format!("{:x}", digest.finalize())
    }

    fn status_change_count(status: &[u8]) -> usize {
        let mut fields = status
            .split(|byte| *byte == 0)
            .filter(|field| !field.is_empty());
        let mut count = 0;
        while let Some(field) = fields.next() {
            if field.len() < 4 {
                continue;
            }
            count += 1;
            let xy = &field[..2];
            if xy.contains(&b'R') || xy.contains(&b'C') {
                // Porcelain v1 -z emits the second rename/copy path as the next
                // NUL-delimited field; it belongs to the same changed file.
                let _ = fields.next();
            }
        }
        count
    }

    #[cfg(test)]
    fn file(&self, relative: &str) -> Result<FileDocument, String> {
        self.file_page(relative, None)
    }

    fn history_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<GitRepositorySnapshot, String> {
        ensure_git_worktree(&self.root)?;
        let limit = limit.max(1);
        let skip = match after {
            None => 0,
            Some(oid) => history_skip_after(
                &git_output(
                    &self.root,
                    &["rev-list", "--all", "--topo-order"],
                    8 * 1024 * 1024,
                )?,
                safe_oid(oid)?,
            )?,
        };
        let max_count = format!("--max-count={}", limit + 1);
        let skip_flag = format!("--skip={skip}");
        let log = git_output(
            &self.root,
            &[
                "log",
                "--all",
                "--topo-order",
                "--date=iso-strict",
                &skip_flag,
                &max_count,
                "--format=%H%x1f%P%x1f%an%x1f%aI%x1f%s%x1f%D%x1e",
            ],
            4 * 1024 * 1024,
        )?;
        let mut commits = parse_git_history(&log);
        let history_truncated = commits.len() > limit;
        commits.truncate(limit);
        let worktree_bytes = git_output(
            &self.root,
            &["worktree", "list", "--porcelain"],
            1024 * 1024,
        )?;
        let current = self
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        Ok(GitRepositorySnapshot {
            commits,
            history_truncated,
            worktrees: parse_git_worktrees(&worktree_bytes, &current),
        })
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
        let (project, worktree) = self.repository_identity();
        Ok(WorktreeManifest {
            provider: "local".to_owned(),
            revision: Self::worktree_revision(head.as_deref(), &status),
            head,
            project,
            branch: self.branch(),
            worktree,
            change_count: Self::status_change_count(&status),
        })
    }

    fn directory(&self, relative: &str, limit: usize) -> Result<CodeTreePage, String> {
        let relative_path = if relative.is_empty() {
            PathBuf::new()
        } else {
            safe_relative(relative)?
        };
        let local = crate::files::directory(&self.root, relative, limit);
        let (mut entries, mut truncated) = if let Ok(page) = local {
            page
        } else if let Some((project_root, inner)) = self.projected_path(&relative_path) {
            let inner = inner.to_string_lossy();
            let (entries, truncated) =
                crate::files::directory(&project_root, &inner, limit).map_err(str::to_owned)?;
            // `files::directory` returns paths relative to the project root,
            // including `inner` (for example, `docs/architecture` when the
            // requested virtual path is `projects/cowboy/docs`). Prefix with
            // the virtual project root, not the requested directory, or every
            // nested projected page repeats its current directory segment.
            let project_prefix = relative_path
                .components()
                .take(2)
                .map(|component| component.as_os_str())
                .collect::<PathBuf>();
            (
                entries
                    .into_iter()
                    .map(|mut entry| {
                        entry.path = project_prefix
                            .join(&entry.path)
                            .to_string_lossy()
                            .replace('\\', "/");
                        entry
                    })
                    .collect(),
                truncated,
            )
        } else {
            return Err("directory not found".to_owned());
        };
        if relative == "projects" {
            for (name, _) in self.projected_projects() {
                if entries.iter().any(|entry| entry.name == name) {
                    continue;
                }
                entries.push(crate::files::DirectoryEntry {
                    name: name.clone(),
                    path: format!("projects/{name}"),
                    is_directory: true,
                    ignored: false,
                });
            }
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            truncated |= entries.len() > limit;
            entries.truncate(limit);
        }
        Ok(CodeTreePage {
            entries: entries
                .into_iter()
                .map(|entry| CodeTreeEntry {
                    name: entry.name,
                    path: entry.path,
                    is_directory: entry.is_directory,
                    ignored: entry.ignored,
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

    fn repository(&self, after: Option<&str>) -> Result<GitRepositorySnapshot, String> {
        self.history_page(after, MAX_HISTORY_COMMITS)
    }

    fn commit(&self, oid: &str) -> Result<GitCommitDetail, String> {
        ensure_git_worktree(&self.root)?;
        let oid = safe_oid(oid)?;
        let metadata = git_output(
            &self.root,
            &[
                "show",
                "-s",
                "--date=iso-strict",
                "--format=%H%x00%P%x00%an%x00%ae%x00%aI%x00%B",
                oid,
            ],
            256 * 1024,
        )?;
        let mut fields = metadata.splitn(6, |byte| *byte == 0);
        let resolved_oid = utf8_field(fields.next(), "commit oid")?;
        let parents = utf8_field(fields.next(), "commit parents")?
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let author = utf8_field(fields.next(), "commit author")?;
        let author_email = utf8_field(fields.next(), "commit author email")?;
        let authored_at = utf8_field(fields.next(), "commit date")?;
        let message = String::from_utf8_lossy(fields.next().unwrap_or_default())
            .trim_end()
            .to_owned();
        let changed = git_output(
            &self.root,
            &[
                "show",
                "--root",
                "--first-parent",
                "--format=",
                "--name-status",
                "-z",
                "-M",
                oid,
                "--",
            ],
            4 * 1024 * 1024,
        )?;
        let mut files = parse_commit_files(&changed);
        let files_truncated = files.len() > MAX_COMMIT_FILES;
        files.truncate(MAX_COMMIT_FILES);
        Ok(GitCommitDetail {
            oid: resolved_oid,
            parents,
            author,
            author_email,
            authored_at,
            message,
            files,
            files_truncated,
        })
    }

    fn commit_diff(&self, oid: &str, relative: &str) -> Result<DiffDocument, String> {
        ensure_git_worktree(&self.root)?;
        let oid = safe_oid(oid)?;
        let relative = safe_relative(relative)?;
        let display = relative.to_string_lossy().replace('\\', "/");
        let bytes = git_output(
            &self.root,
            &[
                "show",
                "--first-parent",
                "--format=",
                "--no-ext-diff",
                "--no-color",
                "--find-renames",
                "--unified=3",
                oid,
                "--",
                &display,
            ],
            MAX_DIFF_SNAPSHOT_BYTES + 1,
        )?;
        let truncated = bytes.len() > MAX_DIFF_SNAPSHOT_BYTES;
        let text =
            String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_DIFF_SNAPSHOT_BYTES)]).to_string();
        let (added, removed) = count_diff_lines(&text);
        Ok(DiffDocument {
            path: display,
            text,
            added,
            removed,
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
        let canonical_file = self.resolved_file(&relative)?;
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

impl LocalCodeProvider {
    pub fn file_raw(&self, relative: &str) -> Result<RawFileDocument, String> {
        let media_type = preview_media_type(relative)
            .ok_or_else(|| "file is not a previewable media type".to_owned())?;
        let relative = safe_relative(relative)?;
        let canonical_file = self.resolved_file(&relative)?;
        let metadata = canonical_file
            .metadata()
            .map_err(|error| error.to_string())?;
        if metadata.len() > MAX_FILE_BYTES as u64 {
            return Err("file too large".to_owned());
        }
        let bytes = std::fs::read(&canonical_file).map_err(|error| error.to_string())?;
        let size = u64::try_from(bytes.len()).map_err(|error| error.to_string())?;
        Ok(RawFileDocument {
            path: relative.to_string_lossy().replace('\\', "/"),
            revision: file_revision(&relative, &metadata),
            media_type: media_type.to_owned(),
            bytes,
            size,
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

#[cfg(feature = "full")]
pub(crate) fn cached_file_page(
    relative: &str,
    bytes: Vec<u8>,
    revision: String,
    cursor: Option<&str>,
) -> Result<FileDocument, String> {
    let size = u64::try_from(bytes.len()).map_err(|error| error.to_string())?;
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
    let available = bytes.len();
    if offset >= available && !(offset == 0 && available == 0) {
        return Err("invalid file cursor".to_owned());
    }
    let mut page = bytes[offset..available.min(offset + FILE_PAGE_BYTES + 4)].to_vec();
    if page.contains(&0) {
        return Err("binary file".to_owned());
    }
    let valid_len = match std::str::from_utf8(&page) {
        Ok(_) => page.len(),
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(_) => return Err("file is not UTF-8".to_owned()),
    };
    let mut end = valid_len.min(FILE_PAGE_BYTES);
    if offset + end < available
        && let Some(newline) = page[..end].iter().rposition(|byte| *byte == b'\n')
    {
        end = newline + 1;
    }
    page.truncate(end);
    let next_offset = offset + end;
    Ok(FileDocument {
        path: relative.to_owned(),
        revision: revision.clone(),
        text: String::from_utf8(page).map_err(|error| error.to_string())?,
        size,
        truncated: next_offset < available,
        next_cursor: (next_offset < available).then(|| format!("{revision}:{next_offset}")),
        limited: false,
    })
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

fn utf8_field(field: Option<&[u8]>, name: &str) -> Result<String, String> {
    let field = field.ok_or_else(|| format!("missing {name}"))?;
    String::from_utf8(field.to_vec()).map_err(|_| format!("invalid {name}"))
}

fn history_skip_after(listed: &[u8], after: &str) -> Result<usize, String> {
    let after = after.as_bytes();
    let mut skip = 0;
    for line in listed.split(|&byte| byte == b'\n') {
        let hash = line.trim_ascii();
        if hash.is_empty() {
            continue;
        }
        skip += 1;
        if hash == after || (after.len() >= 7 && hash.starts_with(after)) {
            return Ok(skip);
        }
    }
    Err("history cursor is not in this repository".to_owned())
}

fn safe_oid(oid: &str) -> Result<&str, String> {
    if (7..=64).contains(&oid.len()) && oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(oid)
    } else {
        Err("invalid commit oid".to_owned())
    }
}

fn parse_git_history(bytes: &[u8]) -> Vec<GitCommitSummary> {
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

fn parse_git_worktrees(bytes: &[u8], current: &Path) -> Vec<GitWorktreeSummary> {
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

fn parse_commit_files(bytes: &[u8]) -> Vec<GitCommitFile> {
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

fn aggregate_checkout_root(root: &Path) -> Option<PathBuf> {
    let common_dir = git_output(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        4096,
    )
    .ok()
    .and_then(|bytes| String::from_utf8(bytes).ok())?;
    let common_dir = PathBuf::from(common_dir.trim()).canonicalize().ok()?;
    if common_dir.file_name()? != std::ffi::OsStr::new(".git") {
        return None;
    }
    let checkout = common_dir.parent()?.canonicalize().ok()?;
    (checkout.join("project-defs").is_dir() && checkout.join("projects").is_dir())
        .then_some(checkout)
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
    use super::{
        ChangeStatus, CodeProvider as _, DiffScope, LocalCodeProvider, RawFileDocument,
        preview_media_type,
    };
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
    fn repository_history_commit_detail_and_worktrees_are_read_only() {
        let dir = scratch("repository");
        fs::write(dir.join("tracked.rs"), "fn second() {}\n").unwrap();
        Command::new("git")
            .args(["add", "tracked.rs"])
            .current_dir(&dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-qm", "second commit"])
            .current_dir(&dir)
            .status()
            .unwrap();
        let linked = dir.with_extension("linked");
        let _ = fs::remove_dir_all(&linked);
        Command::new("git")
            .args([
                "worktree",
                "add",
                "--detach",
                linked.to_str().unwrap(),
                "HEAD~1",
            ])
            .current_dir(&dir)
            .status()
            .unwrap();

        let provider = LocalCodeProvider::new(&dir);
        let repository = provider.repository(None).unwrap();
        assert_eq!(repository.commits[0].subject, "second commit");
        assert!(repository.commits[0].parents.len() == 1);
        assert_eq!(repository.worktrees.len(), 2);
        assert!(repository.worktrees.iter().any(|worktree| worktree.current));
        assert!(repository.worktrees.iter().any(|worktree| {
            worktree.path == linked.display().to_string() && worktree.detached
        }));

        let commit = provider.commit(&repository.commits[0].oid).unwrap();
        assert!(commit.message.starts_with("second commit"));
        assert_eq!(commit.files.len(), 1);
        assert_eq!(commit.files[0].path, "tracked.rs");
        let diff = provider
            .commit_diff(&repository.commits[0].oid, "tracked.rs")
            .unwrap();
        assert!(diff.text.contains("-fn old() {}"));
        assert!(diff.text.contains("+fn second() {}"));
        assert!(provider.commit("--all").is_err());
        assert!(
            provider
                .commit_diff(&repository.commits[0].oid, "../tracked.rs")
                .is_err()
        );

        for index in 0..3 {
            fs::write(dir.join("tracked.rs"), format!("fn n{index}() {{}}\n")).unwrap();
            Command::new("git")
                .args(["commit", "-qam", &format!("page {index}")])
                .current_dir(&dir)
                .status()
                .unwrap();
        }
        let first = provider.history_page(None, 2).unwrap();
        assert_eq!(first.commits.len(), 2);
        assert!(first.history_truncated);
        let second = provider
            .history_page(Some(&first.commits[1].oid), 2)
            .unwrap();
        assert!(second.commits.iter().all(|commit| {
            commit.oid != first.commits[0].oid && commit.oid != first.commits[1].oid
        }));
        assert!(!second.commits.is_empty());

        Command::new("git")
            .args(["worktree", "remove", "--force", linked.to_str().unwrap()])
            .current_dir(&dir)
            .status()
            .unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn manifest_revision_changes_with_visible_worktree_state() {
        let dir = scratch("manifest");
        let provider = LocalCodeProvider::new(&dir);
        let clean = provider.manifest().unwrap();
        assert_eq!(clean.provider, "local");
        assert!(clean.head.is_some());
        assert_eq!(clean.project, "cowboy-code-manifest");
        assert!(clean.branch.is_some());
        assert_eq!(clean.worktree, None);
        assert_eq!(clean.change_count, 0);
        fs::write(dir.join("tracked.rs"), "fn changed() {}\n").unwrap();
        let changed = provider.manifest().unwrap();
        assert_ne!(clean.revision, changed.revision);
        assert_eq!(changed.change_count, 1);
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
        assert!(provider.file("").is_err());
        assert!(provider.file("../secret").is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn directory_accepts_empty_path_as_worktree_root() {
        let dir = scratch("directory-root");
        let provider = LocalCodeProvider::new(&dir);
        let root = provider.directory("", 100).unwrap();

        assert!(root.entries.iter().any(|entry| entry.name == "tracked.rs"));
        assert!(provider.directory(".", 100).is_err());
        assert!(provider.directory("../", 100).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn aggregate_worktrees_project_registered_checkouts_into_code_tree() {
        let dir = scratch("aggregate-projects");
        fs::create_dir_all(dir.join("project-defs/external")).unwrap();
        fs::create_dir_all(dir.join("projects/tracked")).unwrap();
        fs::write(
            dir.join("project-defs/external/project.toml"),
            "name = \"external\"\n",
        )
        .unwrap();
        fs::write(dir.join("projects/tracked/README.md"), "tracked\n").unwrap();
        fs::write(dir.join(".gitignore"), "/projects/*\n").unwrap();
        Command::new("git")
            .args([
                "add",
                "-f",
                ".gitignore",
                "project-defs/external/project.toml",
                "projects/tracked/README.md",
            ])
            .current_dir(&dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-qm", "add aggregate layout"])
            .current_dir(&dir)
            .status()
            .unwrap();
        fs::create_dir_all(dir.join("projects/external/src")).unwrap();
        fs::write(
            dir.join("projects/external/src/lib.rs"),
            "pub fn projected() {}\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("projects/external/src/nested")).unwrap();
        fs::write(
            dir.join("projects/external/src/nested/lib.rs"),
            "pub fn nested() {}\n",
        )
        .unwrap();
        let linked = dir.with_extension("aggregate-linked");
        let _ = fs::remove_dir_all(&linked);
        Command::new("git")
            .args([
                "worktree",
                "add",
                "--detach",
                linked.to_str().unwrap(),
                "HEAD",
            ])
            .current_dir(&dir)
            .status()
            .unwrap();

        let provider = LocalCodeProvider::new(&linked);
        let projects = provider.directory("projects", 100).unwrap();
        assert!(projects.entries.iter().any(|entry| entry.name == "tracked"));
        assert!(
            projects
                .entries
                .iter()
                .any(|entry| entry.name == "external")
        );
        let external = provider.directory("projects/external", 100).unwrap();
        assert_eq!(external.entries[0].path, "projects/external/src");
        let external_src = provider.directory("projects/external/src", 100).unwrap();
        assert_eq!(external_src.entries[0].path, "projects/external/src/nested");
        let external_nested = provider
            .directory("projects/external/src/nested", 100)
            .unwrap();
        assert_eq!(
            external_nested.entries[0].path,
            "projects/external/src/nested/lib.rs"
        );
        assert_eq!(
            provider.file("projects/external/src/lib.rs").unwrap().text,
            "pub fn projected() {}\n"
        );
        assert!(provider.file("projects/unregistered/secret").is_err());
        let (worktree, inner) = provider
            .language_buffer_key("projects/external/src/lib.rs")
            .expect("projected files should lease against the registered checkout");
        assert_eq!(
            worktree,
            dir.join("projects/external").canonicalize().unwrap()
        );
        assert_eq!(inner, PathBuf::from("src/lib.rs"));
        assert!(
            provider
                .language_buffer_key("projects/unregistered/secret")
                .is_none()
        );

        Command::new("git")
            .args(["worktree", "remove", "--force", linked.to_str().unwrap()])
            .current_dir(&dir)
            .status()
            .unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preview_media_type_covers_review_image_and_svg_paths() {
        assert_eq!(preview_media_type("logos/heimdall.png"), Some("image/png"));
        assert_eq!(preview_media_type("icon.SVG"), Some("image/svg+xml"));
        assert_eq!(preview_media_type("photo.JPEG"), Some("image/jpeg"));
        assert_eq!(preview_media_type("diagram.mmd"), None);
        assert_eq!(preview_media_type("main.rs"), None);
    }

    #[test]
    fn file_raw_serves_previewable_bytes_and_rejects_text() {
        let dir = scratch("file-raw");
        fs::write(dir.join("mark.png"), [0x89, b'P', b'N', b'G', 0, 1, 2]).unwrap();
        fs::write(dir.join("note.md"), "# hi\n").unwrap();
        let provider = LocalCodeProvider::new(&dir);
        let raw = provider.file_raw("mark.png").unwrap();
        assert_eq!(raw.media_type, "image/png");
        assert_eq!(raw.bytes[1], b'P');
        assert_eq!(
            provider.file_raw("note.md").unwrap_err(),
            "file is not a previewable media type"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_raw_adapter_json_encodes_bytes_as_base64() {
        let raw = RawFileDocument {
            path: "mark.png".to_owned(),
            revision: "abc".to_owned(),
            media_type: "image/png".to_owned(),
            bytes: vec![0x89, b'P', b'N', b'G'],
            size: 4,
        };
        let value = serde_json::to_value(&raw).unwrap();
        assert_eq!(value["bytes"], "iVBORw==");
        let decoded = serde_json::from_value::<RawFileDocument>(value).unwrap();
        assert_eq!(decoded.bytes, raw.bytes);
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
