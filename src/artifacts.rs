//! Content-addressed storage for large image payloads in durable events.

use std::collections::HashSet;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result};
use base64::Engine as _;
use sha2::{Digest as _, Sha256};

static ARTIFACT_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct ArtifactStore {
    root: PathBuf,
}

impl ArtifactStore {
    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root)
            .with_context(|| format!("creating artifact directory {}", root.display()))?;
        Ok(Self { root })
    }

    /// Replace embedded ACP image data with a stable HTTP reference. Live ACP
    /// traffic remains inline; this runs only on the copy written to history.
    pub fn externalize_images(&self, value: &mut serde_json::Value) -> Result<()> {
        match value {
            serde_json::Value::Array(values) => {
                for value in values {
                    self.externalize_images(value)?;
                }
            }
            serde_json::Value::Object(object) => {
                if object.get("type").and_then(serde_json::Value::as_str) == Some("image") {
                    let mime = object
                        .get("mimeType")
                        .or_else(|| object.get("media_type"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("image/png")
                        .to_owned();
                    externalize_object(self, object, &mime)?;
                    if let Some(serde_json::Value::Object(source)) = object.get_mut("source") {
                        let source_mime = source
                            .get("media_type")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(&mime)
                            .to_owned();
                        externalize_object(self, source, &source_mime)?;
                    }
                }
                for child in object.values_mut() {
                    self.externalize_images(child)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn put(&self, encoded: &str, mime: &str) -> Result<Option<String>> {
        // Small icons cost more as separate HTTP requests; externalize only
        // payloads large enough to materially affect JSONB/WS history.
        if encoded.len() < 32 * 1024 {
            return Ok(None);
        }
        let bytes = match base64::engine::general_purpose::STANDARD.decode(encoded) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(%error, "leaving malformed image data inline");
                return Ok(None);
            }
        };
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let extension = extension_for_mime(mime);
        let name = format!("{hash}.{extension}");
        let path = self.root.join(&name);
        if path.exists() {
            // Refresh the age of a content-addressed object when a new event
            // reuses it. The event row is committed after this method returns;
            // the GC grace period therefore also protects that in-flight
            // reference from a concurrent sweep.
            std::fs::File::options()
                .write(true)
                .open(&path)
                .and_then(|file| file.set_modified(SystemTime::now()))
                .with_context(|| format!("refreshing artifact {}", path.display()))?;
        } else {
            let temp = self.root.join(format!(
                ".{name}.{}.{}.tmp",
                std::process::id(),
                ARTIFACT_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = std::fs::File::create(&temp)
                .with_context(|| format!("creating artifact {}", temp.display()))?;
            file.write_all(&bytes).context("writing artifact")?;
            file.sync_all().context("syncing artifact")?;
            match std::fs::rename(&temp, &path) {
                Ok(()) => {}
                Err(error) if path.exists() => {
                    let _ = std::fs::remove_file(temp);
                    tracing::debug!(%error, artifact = %name, "artifact won concurrent write");
                }
                Err(error) => return Err(error).context("publishing artifact"),
            }
        }
        Ok(Some(format!("/api/artifacts/{name}")))
    }

    pub fn path(&self, name: &str) -> Option<PathBuf> {
        valid_artifact_name(name)
            .then(|| self.root.join(name))
            .filter(|path| path.is_file())
    }

    /// Delete content-addressed artifacts that no retained event references.
    /// A grace period prevents racing a file written just before its event row
    /// commits. Shared artifacts survive until their final reference is gone.
    pub fn prune_unreferenced(
        &self,
        referenced: &HashSet<String>,
        minimum_age: Duration,
    ) -> Result<u64> {
        let now = SystemTime::now();
        let mut removed = 0_u64;
        for entry in std::fs::read_dir(&self.root)
            .with_context(|| format!("reading artifact directory {}", self.root.display()))?
        {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !valid_artifact_name(&name) || referenced.contains(&name) {
                continue;
            }
            let metadata = entry.metadata()?;
            if !metadata.is_file()
                || now
                    .duration_since(metadata.modified().unwrap_or(now))
                    .unwrap_or_default()
                    < minimum_age
            {
                continue;
            }
            std::fs::remove_file(entry.path())
                .with_context(|| format!("removing unreferenced artifact {name}"))?;
            removed = removed.saturating_add(1);
        }
        Ok(removed)
    }
}

pub fn collect_references(value: &serde_json::Value, output: &mut HashSet<String>) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                collect_references(value, output);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                collect_references(value, output);
            }
        }
        serde_json::Value::String(value) => {
            if let Some(name) = value.strip_prefix("/api/artifacts/")
                && valid_artifact_name(name)
            {
                output.insert(name.to_owned());
            }
        }
        _ => {}
    }
}

fn valid_artifact_name(name: &str) -> bool {
    let Some((hash, extension)) = name.split_once('.') else {
        return false;
    };
    hash.len() == 64
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        && matches!(extension, "png" | "jpg" | "webp" | "gif" | "avif")
}

fn externalize_object(
    store: &ArtifactStore,
    object: &mut serde_json::Map<String, serde_json::Value>,
    mime: &str,
) -> Result<()> {
    let Some(data) = object.get("data").and_then(serde_json::Value::as_str) else {
        return Ok(());
    };
    if let Some(url) = store.put(data, mime)? {
        object.remove("data");
        object.insert("url".to_owned(), serde_json::Value::String(url));
    }
    Ok(())
}

fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/avif" => "avif",
        _ => "png",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn externalizes_large_images_but_keeps_small_ones_inline() {
        let root = std::env::temp_dir().join(format!("cowboy-artifacts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = ArtifactStore::new(root.clone()).unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(vec![7_u8; 40_000]);
        let mut value = serde_json::json!({"type":"image","data":data,"mimeType":"image/jpeg"});
        store.externalize_images(&mut value).unwrap();
        assert!(value.get("data").is_none());
        let url = value["url"].as_str().unwrap();
        assert_eq!(std::path::Path::new(url).extension().unwrap(), "jpg");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn garbage_collection_preserves_shared_references() {
        let root = std::env::temp_dir().join(format!("cowboy-artifact-gc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = ArtifactStore::new(root.clone()).unwrap();
        let name = format!("{}.png", "a".repeat(64));
        std::fs::write(root.join(&name), b"image").unwrap();
        let mut referenced = HashSet::new();
        referenced.insert(name.clone());
        assert_eq!(
            store
                .prune_unreferenced(&referenced, Duration::ZERO)
                .unwrap(),
            0
        );
        referenced.clear();
        assert_eq!(
            store
                .prune_unreferenced(&referenced, Duration::ZERO)
                .unwrap(),
            1
        );
        assert!(!root.join(name).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
