//! Service Plugin Catalog retention.
//!
//! Every published release stays in the Catalog until a retention pass proves
//! that nothing can still resolve it: no recoverable session pins it, no Device
//! reports it installed or targets it in an install operation, no Controller
//! host policy pins it, and newer releases of the same Plugin cover the window
//! `plugins.catalog_keep_releases` requests. A retired release loses its
//! release marker first, so a concurrent reader never sees a package without
//! its commit; an artifact directory goes only when no retained release in any
//! Catalog root still names it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result};

use super::PluginCatalog;

/// A release or artifact directory younger than this is never retired, so a
/// publication in progress and the readers that just learned about it settle
/// before retention judges it.
pub(crate) const MIN_RELEASE_AGE: Duration = Duration::from_hours(24);

/// Catalog identities that must stay, as lowercase hex artifact digests.
#[derive(Debug, Default, Clone)]
pub(crate) struct CatalogKeepSet {
    digests: BTreeSet<String>,
}

impl CatalogKeepSet {
    /// Accepts `sha256:<hex>` or bare hex; anything else is not an identity
    /// and is ignored.
    pub(crate) fn insert(&mut self, digest: &str) {
        if let Some(digest) = normalize_digest(digest) {
            self.digests.insert(digest);
        }
    }

    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.digests.len()
    }

    fn contains(&self, digest: &str) -> bool {
        self.digests.contains(digest)
    }
}

fn normalize_digest(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    (hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_lowercase())
}

#[derive(Debug, Default, serde::Serialize)]
pub(crate) struct CatalogRetentionOutcome {
    pub retained: usize,
    pub retired: Vec<String>,
    pub artifact_directories_removed: usize,
    pub freed_bytes: u64,
}

/// One release marker as retention reads it: only the identity it needs, so a
/// marker newer than the strict Catalog reader still counts as present.
#[derive(Debug, Clone, serde::Deserialize)]
struct ReleaseIdentity {
    plugin_id: String,
    plugin_version: String,
    artifact_digest: String,
    #[serde(default)]
    package_digest: String,
    #[serde(default)]
    artifact_url: String,
    #[serde(default)]
    runtime_artifacts: Vec<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct Release {
    root: PathBuf,
    /// File stem shared by the marker, package and host bundle.
    stem: String,
    plugin_id: String,
    plugin_version: String,
    artifact_digest: String,
    version: Option<semver::Version>,
    published_at: SystemTime,
    /// Every `artifacts/<digest>` directory this release's URLs or digests name.
    artifact_dirs: BTreeSet<String>,
}

impl Release {
    fn marker(&self) -> PathBuf {
        self.root.join(format!("{}.release.json", self.stem))
    }

    fn package(&self) -> PathBuf {
        self.root.join(format!("{}.cowboy-plugin", self.stem))
    }

    fn host_bundle(&self) -> PathBuf {
        self.root.join(format!("{}.hostbundle.json", self.stem))
    }

    fn label(&self) -> String {
        format!(
            "{} {} {}",
            self.plugin_id,
            self.plugin_version,
            &self.artifact_digest[..12]
        )
    }
}

/// Collect every 64-hex artifact directory a release names: the package and
/// component URLs the Controller serves plus their declared digests.
fn artifact_dirs(identity: &ReleaseIdentity) -> BTreeSet<String> {
    let mut dirs = BTreeSet::new();
    if let Some(digest) = normalize_digest(&identity.package_digest) {
        dirs.insert(digest);
    }
    let mut urls = vec![identity.artifact_url.clone()];
    let mut values = identity.runtime_artifacts.clone();
    while let Some(value) = values.pop() {
        match value {
            serde_json::Value::Object(object) => {
                for (key, value) in object {
                    match value {
                        serde_json::Value::String(text)
                            if key == "artifact_url" || key == "artifact_digest" =>
                        {
                            urls.push(text);
                        }
                        other => values.push(other),
                    }
                }
            }
            serde_json::Value::Array(items) => values.extend(items),
            _ => {}
        }
    }
    for url in urls {
        if let Some(digest) = normalize_digest(&url) {
            dirs.insert(digest);
            continue;
        }
        if let Some(start) = url.find("/plugin-artifacts/") {
            let rest = &url[start + "/plugin-artifacts/".len()..];
            let hex = rest.split('/').next().unwrap_or_default();
            if let Some(digest) = normalize_digest(hex) {
                dirs.insert(digest);
            }
        }
    }
    dirs
}

struct Scan {
    releases: Vec<Release>,
    /// Markers this reader could not identify. Their artifact references are
    /// unknown, so no artifact directory may be judged unreferenced.
    unreadable: usize,
}

fn scan_root(root: &Path, scan: &mut Scan) -> Result<()> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading Plugin Catalog {}", root.display()));
        }
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".release.json") else {
            continue;
        };
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            scan.unreadable += 1;
            continue;
        };
        if !metadata.is_file() {
            scan.unreadable += 1;
            continue;
        }
        let identity = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ReleaseIdentity>(&bytes).ok());
        let Some(identity) = identity else {
            tracing::warn!(path = %path.display(), "Plugin release marker is unreadable; retention leaves it");
            scan.unreadable += 1;
            continue;
        };
        let Some(artifact_digest) = normalize_digest(&identity.artifact_digest) else {
            tracing::warn!(path = %path.display(), "Plugin release marker has no artifact digest; retention leaves it");
            scan.unreadable += 1;
            continue;
        };
        scan.releases.push(Release {
            root: root.to_path_buf(),
            stem: stem.to_owned(),
            plugin_id: identity.plugin_id.clone(),
            plugin_version: identity.plugin_version.clone(),
            artifact_digest,
            version: semver::Version::parse(&identity.plugin_version).ok(),
            published_at: metadata.modified().unwrap_or_else(|_| SystemTime::now()),
            artifact_dirs: artifact_dirs(&identity),
        });
    }
    Ok(())
}

struct Plan {
    retain: Vec<Release>,
    retire: Vec<Release>,
}

fn plan(
    releases: Vec<Release>,
    keep: &CatalogKeepSet,
    pinned: &BTreeSet<String>,
    keep_releases: usize,
    now: SystemTime,
) -> Plan {
    let mut versions: BTreeMap<String, BTreeSet<semver::Version>> = BTreeMap::new();
    for release in &releases {
        if let Some(version) = &release.version {
            versions
                .entry(release.plugin_id.clone())
                .or_default()
                .insert(version.clone());
        }
    }
    let newest: BTreeMap<String, BTreeSet<semver::Version>> = versions
        .into_iter()
        .map(|(plugin_id, versions)| {
            (
                plugin_id,
                versions.into_iter().rev().take(keep_releases).collect(),
            )
        })
        .collect();
    let mut plan = Plan {
        retain: Vec::new(),
        retire: Vec::new(),
    };
    for release in releases {
        let covered_by_newest = release.version.as_ref().is_some_and(|version| {
            newest
                .get(&release.plugin_id)
                .is_some_and(|set| set.contains(version))
        });
        let fresh = now
            .duration_since(release.published_at)
            .is_ok_and(|age| age < MIN_RELEASE_AGE)
            || release.published_at > now;
        let retain = release.version.is_none()
            || covered_by_newest
            || keep.contains(&release.artifact_digest)
            || pinned.contains(&release.artifact_digest)
            || fresh;
        if retain {
            plan.retain.push(release);
        } else {
            plan.retire.push(release);
        }
    }
    plan
}

fn remove_file_size(path: &Path) -> Result<u64> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            std::fs::remove_file(path)
                .with_context(|| format!("removing retired Plugin file {}", path.display()))?;
            Ok(metadata.len())
        }
        Ok(_) => Ok(0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
}

fn tree_bytes(path: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(metadata) = std::fs::symlink_metadata(&current) else {
            continue;
        };
        if metadata.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&current) {
                pending.extend(entries.flatten().map(|entry| entry.path()));
            }
        } else {
            total += metadata.len();
        }
    }
    total
}

/// Retire releases nothing resolves any more, then the artifact directories no
/// retained release in any root still names. Runs on a blocking thread.
fn retire_in_roots(
    roots: &[PathBuf],
    keep: &CatalogKeepSet,
    pinned: &BTreeSet<String>,
    keep_releases: usize,
    now: SystemTime,
) -> Result<CatalogRetentionOutcome> {
    let mut scan = Scan {
        releases: Vec::new(),
        unreadable: 0,
    };
    for root in roots {
        scan_root(root, &mut scan)?;
    }
    let plan = plan(scan.releases, keep, pinned, keep_releases, now);
    let mut outcome = CatalogRetentionOutcome {
        retained: plan.retain.len(),
        ..CatalogRetentionOutcome::default()
    };
    for release in &plan.retire {
        // The marker is the publication commit; without it the package is
        // invisible to every reader before its bytes go.
        outcome.freed_bytes += remove_file_size(&release.marker())?;
        outcome.freed_bytes += remove_file_size(&release.package())?;
        outcome.freed_bytes += remove_file_size(&release.host_bundle())?;
        outcome.retired.push(release.label());
    }
    if scan.unreadable > 0 {
        tracing::warn!(
            unreadable = scan.unreadable,
            "Plugin Catalog has unreadable release markers; artifact directories are kept"
        );
        return Ok(outcome);
    }
    let referenced: BTreeSet<&str> = plan
        .retain
        .iter()
        .flat_map(|release| release.artifact_dirs.iter().map(String::as_str))
        .collect();
    for root in roots {
        let artifacts = root.join("artifacts");
        let entries = match std::fs::read_dir(&artifacts) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading Plugin artifacts {}", artifacts.display()));
            }
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(digest) = normalize_digest(&name) else {
                continue;
            };
            if referenced.contains(digest.as_str()) {
                continue;
            }
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            let fresh = metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_none_or(|age| age < MIN_RELEASE_AGE);
            if !metadata.is_dir() || fresh {
                continue;
            }
            let bytes = tree_bytes(&path);
            std::fs::remove_dir_all(&path).with_context(|| {
                format!("removing unreferenced Plugin artifacts {}", path.display())
            })?;
            outcome.artifact_directories_removed += 1;
            outcome.freed_bytes += bytes;
        }
    }
    Ok(outcome)
}

impl PluginCatalog {
    /// Retire Catalog releases outside `keep`, the host policy pins and the
    /// newest `keep_releases` versions of each Plugin, along with the artifact
    /// directories only they named. `keep_releases == 0` retires nothing. The
    /// caller refreshes the Catalog afterwards so readers drop the entries.
    pub(crate) async fn retire_unreferenced(
        &self,
        keep: CatalogKeepSet,
        keep_releases: i64,
    ) -> Result<CatalogRetentionOutcome> {
        let keep_releases = usize::try_from(keep_releases).unwrap_or(0);
        if keep_releases == 0 {
            return Ok(CatalogRetentionOutcome::default());
        }
        let roots = self.roots.clone();
        let pinned: BTreeSet<String> = self
            .host_policy
            .pinned_artifact_digests()
            .filter_map(normalize_digest)
            .collect();
        // Serialize with refresh and publication; both read the same roots.
        let _refresh = self.refresh_lock.lock().await;
        tokio::task::spawn_blocking(move || {
            retire_in_roots(&roots, &keep, &pinned, keep_releases, SystemTime::now())
        })
        .await
        .context("Plugin Catalog retention task")?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish(
        root: &Path,
        plugin: &str,
        version: &str,
        seed: u8,
        components: &[u8],
        age: Duration,
    ) -> String {
        let digest = format!("{seed:02x}").repeat(32);
        let package_digest = format!("{:02x}", seed + 0x40).repeat(32);
        let stem = format!("{plugin}-{version}-{digest}");
        let components: Vec<serde_json::Value> = components
            .iter()
            .map(|component| {
                let component_digest = format!("{component:02x}").repeat(32);
                serde_json::json!({
                    "kind": "agent_cli",
                    "artifact_url": format!("https://cowboy.test/plugin-artifacts/{component_digest}/cli.tar.gz"),
                    "artifact_digest": format!("sha256:{component_digest}"),
                })
            })
            .collect();
        let marker = serde_json::json!({
            "release_schema": 4,
            "plugin_id": plugin,
            "plugin_version": version,
            "package_digest": format!("sha256:{package_digest}"),
            "artifact_digest": format!("sha256:{digest}"),
            "artifact_url": format!("https://cowboy.test/plugin-artifacts/{package_digest}/{plugin}.cowboy-plugin"),
            "runtime_artifacts": [{"os": "linux", "architecture": "x86_64", "components": components}],
        });
        std::fs::write(root.join(format!("{stem}.cowboy-plugin")), b"package").unwrap();
        std::fs::write(root.join(format!("{stem}.hostbundle.json")), b"{}").unwrap();
        std::fs::write(
            root.join(format!("{stem}.release.json")),
            serde_json::to_vec(&marker).unwrap(),
        )
        .unwrap();
        let mut dirs = vec![package_digest];
        dirs.extend(components.iter().map(|component| {
            component["artifact_digest"]
                .as_str()
                .unwrap()
                .trim_start_matches("sha256:")
                .to_owned()
        }));
        for dir in dirs {
            let directory = root.join("artifacts").join(dir);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("blob"), vec![1_u8; 1024]).unwrap();
            age_path(&directory, age);
        }
        age_path(&root.join(format!("{stem}.release.json")), age);
        digest
    }

    fn age_path(path: &Path, age: Duration) {
        let modified = SystemTime::now() - age;
        std::fs::File::open(path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }

    fn present(root: &Path, plugin: &str, version: &str, digest: &str) -> bool {
        root.join(format!("{plugin}-{version}-{digest}.release.json"))
            .exists()
    }

    #[test]
    fn retention_keeps_referenced_pinned_newest_and_fresh_releases() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("catalog");
        std::fs::create_dir_all(root.join("artifacts")).unwrap();
        let old = Duration::from_hours(72);
        // codex: 0.9.0 is stale, 1.0.0 is pinned by a session, 1.1.0 shares a
        // component with 1.2.0, 1.2.0 and 1.3.0 are the newest two and 1.3.0
        // was published an hour ago.
        let stale = publish(&root, "codex", "0.9.0", 0x05, &[0x15], old);
        let session_pinned = publish(&root, "codex", "1.0.0", 0x01, &[0x11], old);
        let shared_component = publish(&root, "codex", "1.1.0", 0x02, &[0x12], old);
        let newest = publish(&root, "codex", "1.2.0", 0x03, &[0x12, 0x13], old);
        let fresh = publish(
            &root,
            "codex",
            "1.3.0",
            0x04,
            &[0x14],
            Duration::from_hours(1),
        );
        // claude: the host policy pins 2.0.0; 2.2.0 and 2.3.0 are the newest two.
        let policy_pinned = publish(&root, "claude", "2.0.0", 0x06, &[0x16], old);
        let superseded = publish(&root, "claude", "2.1.0", 0x07, &[0x17], old);
        let claude_newer = publish(&root, "claude", "2.2.0", 0x08, &[0x18], old);
        let claude_newest = publish(&root, "claude", "2.3.0", 0x09, &[0x19], old);
        let mut keep = CatalogKeepSet::default();
        keep.insert(&format!("sha256:{session_pinned}"));
        let pinned = BTreeSet::from([policy_pinned.clone()]);

        let outcome = retire_in_roots(
            std::slice::from_ref(&root),
            &keep,
            &pinned,
            2,
            SystemTime::now(),
        )
        .unwrap();

        assert!(present(&root, "codex", "1.0.0", &session_pinned));
        assert!(present(&root, "codex", "1.2.0", &newest));
        assert!(present(&root, "codex", "1.3.0", &fresh));
        assert!(present(&root, "claude", "2.0.0", &policy_pinned));
        assert!(present(&root, "claude", "2.2.0", &claude_newer));
        assert!(present(&root, "claude", "2.3.0", &claude_newest));
        assert!(!present(&root, "codex", "0.9.0", &stale));
        assert!(!present(&root, "codex", "1.1.0", &shared_component));
        assert!(!present(&root, "claude", "2.1.0", &superseded));
        assert!(
            !root
                .join(format!("codex-0.9.0-{stale}.cowboy-plugin"))
                .exists()
        );
        assert!(
            !root
                .join(format!("codex-0.9.0-{stale}.hostbundle.json"))
                .exists()
        );
        let artifacts = root.join("artifacts");
        assert!(
            artifacts.join("12".repeat(32)).exists(),
            "a component 1.2.0 still names stays"
        );
        assert!(
            !artifacts.join("15".repeat(32)).exists(),
            "a component only 0.9.0 named goes"
        );
        assert!(
            !artifacts.join("42".repeat(32)).exists(),
            "1.1.0's package directory goes"
        );
        assert!(
            artifacts.join("41".repeat(32)).exists(),
            "1.0.0's package directory stays"
        );
        assert!(
            !artifacts.join("17".repeat(32)).exists(),
            "a component only 2.1.0 named goes"
        );
        assert_eq!(outcome.retired.len(), 3);
        assert_eq!(outcome.retained, 6);
        assert_eq!(outcome.artifact_directories_removed, 5);
        assert!(outcome.freed_bytes >= 5 * 1024);

        // Nothing is left to retire on a second pass.
        let again = retire_in_roots(&[root], &keep, &pinned, 2, SystemTime::now()).unwrap();
        assert!(again.retired.is_empty());
        assert_eq!(again.artifact_directories_removed, 0);
    }

    #[test]
    fn an_unreadable_marker_keeps_every_artifact_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("catalog");
        std::fs::create_dir_all(root.join("artifacts")).unwrap();
        let old = Duration::from_hours(72);
        let stale = publish(&root, "codex", "0.9.0", 0x05, &[0x15], old);
        let _newest = publish(&root, "codex", "1.0.0", 0x01, &[0x11], old);
        std::fs::write(root.join("future-9.9.9-ffff.release.json"), b"{not json").unwrap();
        let orphan = root.join("artifacts").join("ee".repeat(32));
        std::fs::create_dir_all(&orphan).unwrap();
        age_path(&orphan, old);

        let outcome = retire_in_roots(
            std::slice::from_ref(&root),
            &CatalogKeepSet::default(),
            &BTreeSet::new(),
            1,
            SystemTime::now(),
        )
        .unwrap();

        assert!(!present(&root, "codex", "0.9.0", &stale));
        assert_eq!(outcome.artifact_directories_removed, 0);
        assert!(root.join("artifacts").join("15".repeat(32)).exists());
        assert!(orphan.exists());
    }

    #[test]
    fn a_second_root_shares_references_with_the_primary() {
        let dir = tempfile::tempdir().unwrap();
        let primary = dir.path().join("catalog");
        let legacy = dir.path().join("legacy");
        std::fs::create_dir_all(primary.join("artifacts")).unwrap();
        std::fs::create_dir_all(legacy.join("artifacts")).unwrap();
        let old = Duration::from_hours(72);
        // The newest release lives in the primary root but its component blob
        // was published under the legacy root, which the Controller still reads.
        let _newest = publish(&primary, "codex", "2.0.0", 0x01, &[0x11], old);
        let legacy_copy = legacy.join("artifacts").join("11".repeat(32));
        std::fs::create_dir_all(&legacy_copy).unwrap();
        age_path(&legacy_copy, old);
        let stale = publish(&legacy, "codex", "1.0.0", 0x02, &[0x12], old);

        let outcome = retire_in_roots(
            &[primary.clone(), legacy.clone()],
            &CatalogKeepSet::default(),
            &BTreeSet::new(),
            1,
            SystemTime::now(),
        )
        .unwrap();

        assert!(!present(&legacy, "codex", "1.0.0", &stale));
        assert!(
            legacy_copy.exists(),
            "a directory the newest release names stays in every root"
        );
        assert!(!legacy.join("artifacts").join("12".repeat(32)).exists());
        assert_eq!(
            outcome.retired,
            vec![format!("codex 1.0.0 {}", &stale[..12])]
        );
    }
}
