//! Tier routing: the machine tier for host-wide facts, or `projects/<slug>` for
//! a caller inside a project. The slug matches Claude Code's keying (sanitized
//! canonical git root, e.g. `-home-draven-columbus`) so the store lines up with
//! CC's native auto-memory.
//!
//! Ported faithfully from mnemosyne's `internal/tier` (tier.go, slug.go).

#[cfg(test)]
use std::process::Command;

use super::store::Tier;

/// The per-project tier for a sanitized slug (e.g. `-home-draven-columbus`).
///
/// The trailing `/memory` matches Claude Code's EXACT auto-memory path: with
/// `CLAUDE_CODE_REMOTE_MEMORY_DIR=<storeRoot>`, CC reads/writes
/// `<storeRoot>/projects/<slug>/memory/`, so the janitor must manage that same
/// dir (else CC-native memories and the janitor diverge).
///
/// Mirrors Go's `store.ProjectTier`.
#[must_use]
pub fn project_tier(slug: &str) -> Tier {
    Tier(format!("projects/{slug}/memory"))
}

/// The project slug for a `projects/<slug>/memory` tier, or `""` for the machine
/// (or any non-project) tier. Mirrors Go's `tier.Slug`.
#[must_use]
#[cfg(test)]
pub fn slug_of(t: &Tier) -> String {
    if let Some(rest) = t.as_str().strip_prefix("projects/") {
        rest.strip_suffix("/memory").unwrap_or(rest).to_string()
    } else {
        String::new()
    }
}

/// Convert an absolute path to CC's filesystem-safe slug: every non-alphanumeric
/// byte becomes `-` (e.g. `/home/draven/columbus` → `-home-draven-columbus`). Not
/// collapsed — matches CC byte-for-byte. Mirrors Go's `tier.Sanitize`.
#[must_use]
#[cfg(test)]
pub fn sanitize(p: &str) -> String {
    let mut b = String::with_capacity(p.len());
    for c in p.chars() {
        if c.is_ascii_alphanumeric() {
            b.push(c);
        } else {
            b.push('-');
        }
    }
    b
}

/// The context of a memory mutation. `machine` forces the machine tier;
/// otherwise `cwd` is resolved to its canonical git root and sanitized to a slug.
/// Mirrors Go's `tier.Caller`.
#[derive(Debug, Clone, Default)]
#[cfg(test)]
pub struct Caller {
    pub cwd: String,
    pub machine: bool,
}

#[cfg(test)]
type GitRootResolver<'a> = Option<&'a dyn Fn(&str) -> Option<String>>;

/// Resolve a directory to its canonical git toplevel via `git -C <dir> rev-parse
/// --show-toplevel`. Returns `None` when the dir is not inside a git repo.
#[cfg(test)]
fn git_root(dir: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if root.is_empty() {
        None
    } else {
        Some(root)
    }
}

/// Resolve a `Caller` to its tier. A machine caller, or a cwd not inside any git
/// repo, routes to the machine tier; otherwise to `projects/<sanitized-root>`.
///
/// `git_root_override` lets tests inject the git-root resolver (matching the Go
/// `tier.GitRoot` package var). Pass `None` to use the real `git` shell-out.
#[must_use]
#[cfg(test)]
pub fn route(c: &Caller, git_root_override: GitRootResolver<'_>) -> Tier {
    if c.machine {
        return Tier::machine();
    }
    let root = match git_root_override {
        Some(f) => f(&c.cwd),
        None => git_root(&c.cwd),
    };
    match root {
        // Not in a repo → treat as machine-scoped rather than erroring.
        None => Tier::machine(),
        Some(r) if r.is_empty() => Tier::machine(),
        Some(r) => project_tier(&sanitize(&r)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_matches_cc_keying() {
        assert_eq!(sanitize("/home/draven/columbus"), "-home-draven-columbus");
        assert_eq!(sanitize("/etc/nixos"), "-etc-nixos");
        assert_eq!(sanitize("/home/draven"), "-home-draven");
    }

    #[test]
    fn project_tier_has_memory_suffix() {
        assert_eq!(
            project_tier("-home-draven-columbus").as_str(),
            "projects/-home-draven-columbus/memory"
        );
    }

    #[test]
    fn slug_of_round_trips() {
        let t = project_tier("-home-draven-columbus");
        assert_eq!(slug_of(&t), "-home-draven-columbus");
        assert_eq!(slug_of(&Tier::machine()), "");
    }

    #[test]
    fn route_project_and_machine() {
        // Project caller: cwd resolves to a git root → projects/<slug>.
        let fake_root = |_dir: &str| Some("/home/draven/columbus".to_string());
        let got = route(
            &Caller {
                cwd: "/home/draven/columbus/projects/x".to_string(),
                machine: false,
            },
            Some(&fake_root),
        );
        assert_eq!(got, project_tier("-home-draven-columbus"));

        // Machine-scoped caller → machine tier (no git lookup).
        let got = route(
            &Caller {
                cwd: String::new(),
                machine: true,
            },
            Some(&fake_root),
        );
        assert_eq!(got, Tier::machine());

        // cwd not in any repo → machine tier (graceful).
        let not_repo = |_dir: &str| None;
        let got = route(
            &Caller {
                cwd: "/tmp/nowhere".to_string(),
                machine: false,
            },
            Some(&not_repo),
        );
        assert_eq!(got, Tier::machine());
    }
}
