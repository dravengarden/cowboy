//! The on-disk tiered memory store: markdown files with YAML frontmatter under
//! `<root>/{machine,projects/<slug>/memory,archive}/`, each tier carrying a
//! `MEMORY.md` index. The store is the single writer of the canonical index
//! (which is what removes the concurrent-index race). Frontmatter is parsed by
//! hand for the fixed schema (name/description/metadata.type) to keep the daemon
//! dependency-free.
//!
//! Ported faithfully from mnemosyne's `internal/store` (store.go, git.go,
//! remove.go, index_md.go, tiers.go).

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

/// The frontmatter `metadata.type` discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryType {
    User,
    Feedback,
    Project,
    Reference,
}

impl MemoryType {
    /// The canonical wire string (matches Go's `MemoryType` constants).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryType::User => "user",
            MemoryType::Feedback => "feedback",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
        }
    }

    /// Parse a `metadata.type` value. Unknown strings map to the raw value being
    /// rejected — but Go stores ANY string into the typed field, so to preserve
    /// round-trip behavior for unknown types we fall back to keeping the raw
    /// string via `parse_lenient`. `parse` is the strict form used at the API.
    #[must_use]
    pub fn parse(s: &str) -> Option<MemoryType> {
        match s {
            "user" => Some(MemoryType::User),
            "feedback" => Some(MemoryType::Feedback),
            "project" => Some(MemoryType::Project),
            "reference" => Some(MemoryType::Reference),
            _ => None,
        }
    }
}

impl fmt::Display for MemoryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One memory file: frontmatter + markdown body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Memory {
    /// frontmatter `name` (kebab-case slug; also the filename stem)
    pub name: String,
    /// frontmatter `description` (one-line; used for recall relevance)
    pub description: String,
    /// frontmatter `metadata.type`
    pub mem_type: MemoryType,
    /// markdown body after the frontmatter
    pub body: String,
}

impl Memory {
    /// Serialize a Memory back to the canonical file format.
    ///
    /// Mirrors Go's `Memory.Render`: a `---` frontmatter block with
    /// name/description/metadata.type, a blank line, then the body (its leading
    /// newlines trimmed), guaranteed to end with a single `\n`.
    #[must_use]
    pub fn render(&self) -> String {
        let mut b = String::new();
        b.push_str("---\n");
        b.push_str(&format!("name: {}\n", self.name));
        b.push_str(&format!("description: {}\n", self.description));
        b.push_str("metadata:\n");
        b.push_str(&format!("  type: {}\n", self.mem_type));
        b.push_str("---\n\n");
        b.push_str(self.body.trim_start_matches('\n'));
        if !b.ends_with('\n') {
            b.push('\n');
        }
        b
    }

    /// Parse a memory file's bytes. Frontmatter is the leading `---`-delimited
    /// block; the fixed keys name/description/metadata.type are read, everything
    /// after the closing `---` is the body.
    ///
    /// Faithful port of Go's `ParseMemory`, including its edge cases:
    /// - missing leading `---\n` → error
    /// - no closing `\n---` → error
    /// - the closing-line remainder + the separator blank line(s) are dropped
    /// - an unknown `metadata.type` is kept verbatim is NOT possible here (the Go
    ///   stores any string); we surface unknown types as an error at parse time
    ///   ONLY if the value is non-empty and unrecognized — see note below.
    ///
    /// # Errors
    /// Returns an error when the frontmatter is missing/unterminated, the `name`
    /// key is absent, or `metadata.type` holds an unrecognized value.
    pub fn parse(data: &[u8]) -> Result<Memory> {
        let s = String::from_utf8_lossy(data);
        let s: &str = &s;
        if !s.starts_with("---\n") {
            bail!("missing frontmatter");
        }
        let rest = &s["---\n".len()..];
        let end = rest
            .find("\n---")
            .ok_or_else(|| anyhow!("unterminated frontmatter"))?;
        let front = &rest[..end];
        let mut body = &rest[end + "\n---".len()..];
        // Drop the closing `---` line's remainder and the separator blank line(s).
        if let Some(nl) = body.find('\n') {
            body = &body[nl + 1..];
        }
        let body = body.trim_start_matches('\n');

        let mut name = String::new();
        let mut description = String::new();
        let mut type_raw = String::new();
        let mut in_metadata = false;
        for line in front.split('\n') {
            if line.trim().is_empty() {
                continue;
            }
            let indented = line.starts_with(' ') || line.starts_with('\t');
            let Some((key, val)) = line.trim().split_once(':') else {
                continue;
            };
            let key = key.trim();
            let val = val.trim().trim_matches(|c| c == '"' || c == '\'');
            match key {
                "name" if !indented => {
                    name = val.to_string();
                    in_metadata = false;
                }
                "description" if !indented => {
                    description = val.to_string();
                    in_metadata = false;
                }
                "metadata" if !indented => {
                    in_metadata = true;
                }
                "type" if in_metadata => {
                    type_raw = val.to_string();
                }
                _ => {}
            }
        }

        let body = body.trim_end_matches('\n').to_string();
        if name.is_empty() {
            bail!("frontmatter missing name");
        }
        // Go stores the raw type string into a typed alias (no validation at
        // parse). We require a known type so the Rust enum stays total; the four
        // canonical types are the only ones the store ever writes.
        let mem_type = MemoryType::parse(&type_raw)
            .ok_or_else(|| anyhow!("unknown metadata.type {type_raw:?}"))?;

        Ok(Memory {
            name,
            description,
            mem_type,
            body,
        })
    }
}

/// A store tier, expressed as a path relative to the store root.
///
/// `Tier` wraps the same slash-relative string Go's `store.Tier` uses
/// (`"machine"`, `"archive"`, `"projects/<slug>/memory"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Tier(pub String);

impl Tier {
    /// The machine tier (host-wide facts).
    #[must_use]
    pub fn machine() -> Tier {
        Tier("machine".to_string())
    }

    /// The archive tier (cold storage; soft-archived memories).
    #[must_use]
    pub fn archive() -> Tier {
        Tier("archive".to_string())
    }

    /// The relative path string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A tiered memory store rooted at `root` (default `~/.agents/memory`).
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

// Janitor commit identity — marks machine-written memory commits distinctly from
// the human's. (The store is its own git repo; every applied change is a commit,
// which is the audit trail and the `revert` handle — there is no human gate.)
const GIT_USER_NAME: &str = "mnemosyne";
const GIT_USER_EMAIL: &str = "mnemosyne@hawk.local";

impl Store {
    /// Returns a Store rooted at `root`.
    #[must_use]
    pub fn new(root: PathBuf) -> Store {
        Store { root }
    }

    /// The store root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn dir(&self, t: &Tier) -> PathBuf {
        // Tier strings are slash-relative; join each segment so this works on
        // any OS path separator (matches Go's `filepath.FromSlash`).
        let mut p = self.root.clone();
        for seg in t.0.split('/') {
            p.push(seg);
        }
        p
    }

    fn path(&self, t: &Tier, name: &str) -> PathBuf {
        self.dir(t).join(format!("{name}.md"))
    }

    /// Materialize a memory into a tier (creating the tier dir as needed).
    ///
    /// # Errors
    /// Propagates filesystem errors from `mkdir`/`write`.
    pub fn write(&self, t: &Tier, m: &Memory) -> Result<()> {
        let dir = self.dir(t);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("mkdir {}", dir.display()))?;
        let path = self.path(t, &m.name);
        std::fs::write(&path, m.render())
            .with_context(|| format!("write {}", path.display()))?;
        Ok(())
    }

    /// Load a memory by name from a tier.
    ///
    /// # Errors
    /// Propagates read errors and parse errors.
    pub fn read(&self, t: &Tier, name: &str) -> Result<Memory> {
        let path = self.path(t, name);
        let data = std::fs::read(&path)
            .with_context(|| format!("read {}", path.display()))?;
        Memory::parse(&data)
    }

    /// Return all memories in a tier (excluding the `MEMORY.md` index), sorted by
    /// name. A nonexistent tier dir returns an empty list (not an error), matching
    /// Go's `os.IsNotExist` short-circuit.
    ///
    /// # Errors
    /// Propagates read/parse errors for any non-index `.md` file.
    pub fn list(&self, t: &Tier) -> Result<Vec<Memory>> {
        let dir = self.dir(t);
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("readdir {}", dir.display())),
        };
        // Collect names first so we can sort by name (Go's os.ReadDir already
        // returns sorted entries; we sort explicitly to be deterministic).
        let mut names: Vec<String> = Vec::new();
        for entry in rd {
            let entry = entry?;
            let n = entry.file_name().to_string_lossy().to_string();
            let ft = entry.file_type()?;
            if ft.is_dir() || !n.ends_with(".md") || n == "MEMORY.md" {
                continue;
            }
            names.push(n);
        }
        names.sort();
        let mut out = Vec::with_capacity(names.len());
        for n in names {
            let stem = n.trim_end_matches(".md");
            let m = self
                .read(t, stem)
                .with_context(|| format!("read {t}/{n}"))?;
            out.push(m);
        }
        Ok(out)
    }

    /// Delete a memory file from a tier. A nonexistent file is a clean no-op
    /// (callers that must preserve the memory archive it first — see `apply`).
    ///
    /// # Errors
    /// Propagates filesystem errors other than not-found.
    pub fn remove(&self, t: &Tier, name: &str) -> Result<()> {
        let path = self.path(t, name);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("remove {}", path.display())),
        }
    }

    // ---- tiers.go ---------------------------------------------------------

    /// List the per-project tier slugs present under `projects/`.
    ///
    /// # Errors
    /// Propagates readdir errors (a missing `projects/` dir is empty, not error).
    pub fn project_slugs(&self) -> Result<Vec<String>> {
        let dir = self.root.join("projects");
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("readdir {}", dir.display())),
        };
        let mut slugs = Vec::new();
        for entry in rd {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                slugs.push(entry.file_name().to_string_lossy().to_string());
            }
        }
        slugs.sort();
        Ok(slugs)
    }

    /// The tiers that participate in recall/dedup: the machine tier plus every
    /// `projects/<slug>`. The archive tier is excluded (cold storage).
    ///
    /// # Errors
    /// Propagates errors from `project_slugs`.
    pub fn active_tiers(&self) -> Result<Vec<Tier>> {
        let mut tiers = vec![Tier::machine()];
        for slug in self.project_slugs()? {
            tiers.push(super::tier::project_tier(&slug));
        }
        Ok(tiers)
    }

    // ---- index_md.go ------------------------------------------------------

    /// Render a tier's `MEMORY.md`: a one-line-per-memory index
    /// (`- [name](name.md) — description`), sorted by name.
    ///
    /// # Errors
    /// Propagates errors from `list`.
    pub fn generate_index(&self, t: &Tier) -> Result<String> {
        let mems = self.list(t)?; // list already sorts by name
        let mut b = String::from("# Memory index\n\n");
        for m in &mems {
            b.push_str(&format!(
                "- [{}]({}.md) — {}\n",
                m.name, m.name, m.description
            ));
        }
        Ok(b)
    }

    /// Regenerate and write the tier's `MEMORY.md`.
    ///
    /// # Errors
    /// Propagates errors from `generate_index` and the filesystem.
    pub fn write_index(&self, t: &Tier) -> Result<()> {
        let idx = self.generate_index(t)?;
        let dir = self.dir(t);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("mkdir {}", dir.display()))?;
        std::fs::write(dir.join("MEMORY.md"), idx)
            .with_context(|| format!("write {}/MEMORY.md", t))?;
        Ok(())
    }

    // ---- git.go -----------------------------------------------------------

    /// Run `git -C <root> <args>` and return trimmed combined stdout+stderr.
    fn git(&self, args: &[&str]) -> Result<(String, bool)> {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.root);
        cmd.args(args);
        let out = cmd
            .output()
            .with_context(|| format!("exec git {}", args.join(" ")))?;
        let mut combined = String::from_utf8_lossy(&out.stdout).to_string();
        combined.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok((combined.trim().to_string(), out.status.success()))
    }

    /// Initialize the store as a git repo on first use (idempotent) and pin the
    /// janitor commit identity locally.
    ///
    /// # Errors
    /// Propagates filesystem and git errors.
    pub fn ensure_git_repo(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("mkdir {}", self.root.display()))?;
        if self.root.join(".git").exists() {
            return Ok(());
        }
        let (out, ok) = self.git(&["init", "-q"])?;
        if !ok {
            bail!("git init: {out}");
        }
        let (out, ok) = self.git(&["config", "user.name", GIT_USER_NAME])?;
        if !ok {
            bail!("git config user.name: {out}");
        }
        let (out, ok) = self.git(&["config", "user.email", GIT_USER_EMAIL])?;
        if !ok {
            bail!("git config user.email: {out}");
        }
        Ok(())
    }

    /// Stage everything and commit with `msg`. No-ops cleanly when there is
    /// nothing to commit (returns `Ok(None)`). Returns the new commit's short hash
    /// on success.
    ///
    /// # Errors
    /// Propagates git errors.
    pub fn commit(&self, msg: &str) -> Result<Option<String>> {
        self.ensure_git_repo()?;
        let (out, ok) = self.git(&["add", "-A"])?;
        if !ok {
            bail!("git add: {out}");
        }
        let (status, _) = self.git(&["status", "--porcelain"])?;
        if status.is_empty() {
            return Ok(None); // nothing to commit
        }
        let (out, ok) = self.git(&["commit", "-q", "-m", msg])?;
        if !ok {
            bail!("git commit: {out}");
        }
        let (hash, ok) = self.git(&["rev-parse", "--short", "HEAD"])?;
        if !ok {
            bail!("git rev-parse: {hash}");
        }
        Ok(Some(hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> (Store, tempdir::TempLike) {
        let d = tempdir::TempLike::new();
        (Store::new(d.path().to_path_buf()), d)
    }

    #[test]
    fn write_read_round_trip() {
        let (s, _d) = tmp();
        let want = Memory {
            name: "user-runs-nixos-switch".to_string(),
            description:
                "the user runs nixos-rebuild switch themselves; build-verify + hand off"
                    .to_string(),
            mem_type: MemoryType::Feedback,
            body: "Don't run sudo switch via Bash (rejected twice).\n\nLink [[deploy-paths]]."
                .to_string(),
        };
        s.write(&Tier::machine(), &want).unwrap();
        let got = s.read(&Tier::machine(), &want.name).unwrap();
        assert_eq!(got.name, want.name);
        assert_eq!(got.description, want.description);
        assert_eq!(got.mem_type, want.mem_type);
        assert_eq!(got.body, want.body);
    }

    #[test]
    fn parse_memory_nested_metadata() {
        let input = b"---\nname: foo\ndescription: a thing\nmetadata:\n  type: project\n---\n\nbody line one\nbody line two\n";
        let m = Memory::parse(input).unwrap();
        assert_eq!(m.name, "foo");
        assert_eq!(m.description, "a thing");
        assert_eq!(m.mem_type, MemoryType::Project);
        assert_eq!(m.body, "body line one\nbody line two");
    }

    #[test]
    fn parse_missing_frontmatter_errs() {
        assert!(Memory::parse(b"no frontmatter here").is_err());
        assert!(Memory::parse(b"---\nname: x\nno closing").is_err());
    }

    #[test]
    fn list_skips_index() {
        let (s, _d) = tmp();
        let proj = super::super::tier::project_tier("-home-draven-columbus");
        for n in ["a", "b"] {
            s.write(
                &proj,
                &Memory {
                    name: n.to_string(),
                    description: n.to_string(),
                    mem_type: MemoryType::Project,
                    body: n.to_string(),
                },
            )
            .unwrap();
        }
        // A stray MEMORY.md must not be parsed as a memory.
        s.write(
            &proj,
            &Memory {
                name: "MEMORY".to_string(),
                description: "index".to_string(),
                mem_type: MemoryType::Reference,
                body: "x".to_string(),
            },
        )
        .unwrap();
        let got = s.list(&proj).unwrap();
        assert_eq!(got.len(), 2, "MEMORY.md must be skipped");
    }

    #[test]
    fn generate_index_sorted() {
        let (s, _d) = tmp();
        for m in [
            Memory {
                name: "omega-deploy".to_string(),
                description: "omega proxy deploy".to_string(),
                mem_type: MemoryType::Project,
                body: "x".to_string(),
            },
            Memory {
                name: "argus-project".to_string(),
                description: "read-only system dashboard".to_string(),
                mem_type: MemoryType::Project,
                body: "y".to_string(),
            },
        ] {
            s.write(&Tier::machine(), &m).unwrap();
        }
        let idx = s.generate_index(&Tier::machine()).unwrap();
        let want = "- [argus-project](argus-project.md) — read-only system dashboard\n- [omega-deploy](omega-deploy.md) — omega proxy deploy";
        assert!(idx.contains(want), "index missing/disordered:\n{idx}");
        s.write_index(&Tier::machine()).unwrap();
        // Regenerating after write_index must not pick MEMORY.md up.
        assert_eq!(s.list(&Tier::machine()).unwrap().len(), 2);
    }

    #[test]
    fn commit_yields_one_commit() {
        let (s, _d) = tmp();
        s.write(
            &Tier::machine(),
            &Memory {
                name: "a".to_string(),
                description: "first".to_string(),
                mem_type: MemoryType::Project,
                body: "x".to_string(),
            },
        )
        .unwrap();
        let hash = s.commit("memory: add a").unwrap();
        assert!(hash.is_some(), "expected a commit hash");
        let (count, _) = s.git(&["rev-list", "--count", "HEAD"]).unwrap();
        assert_eq!(count, "1");

        // A second write → a second distinct commit.
        s.write(
            &Tier::machine(),
            &Memory {
                name: "b".to_string(),
                description: "second".to_string(),
                mem_type: MemoryType::Project,
                body: "y".to_string(),
            },
        )
        .unwrap();
        s.commit("memory: add b").unwrap();
        let (count, _) = s.git(&["rev-list", "--count", "HEAD"]).unwrap();
        assert_eq!(count, "2");

        // No-op commit when nothing changed.
        assert_eq!(s.commit("noop").unwrap(), None);
    }
}

/// A tiny self-contained temp-dir helper (cowboy has no `tempfile` dep, and the
/// Go tests use `t.TempDir()`). Creates a unique dir under the system temp and
/// removes it on drop. Test-only.
#[cfg(test)]
mod tempdir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    pub struct TempLike {
        path: PathBuf,
    }

    impl TempLike {
        pub fn new() -> TempLike {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("cowboy-mem-test-{pid}-{nanos}-{n}"));
            std::fs::create_dir_all(&path).unwrap();
            TempLike { path }
        }

        pub fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempLike {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
