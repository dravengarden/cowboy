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

/// Inline image payloads below this encoded size stay in the event row.
const INLINE_IMAGE_LIMIT_BYTES: usize = 32 * 1024;

/// Replaces `data` on a worker's prompt echo image when Cowboy stored those
/// exact bytes before dispatch (`CoreCommand::Prompt::echo_artifacts`). The
/// Controller resolves it to the artifact URL before the event is recorded.
pub const ECHO_ARTIFACT_FIELD: &str = "artifactSha256";

/// Content digest of an inline image payload large enough to be externalized.
#[must_use]
pub fn large_image_digest(encoded: &str) -> Option<String> {
    if encoded.len() < INLINE_IMAGE_LIMIT_BYTES {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    Some(format!("{:x}", Sha256::digest(&bytes)))
}

/// Swap the bytes of a prompt-echo image for their digest when the digest is
/// one Cowboy stored before dispatch. Other updates are left untouched.
pub fn reference_echoed_image(update: &mut serde_json::Value, stored: &[String]) {
    if stored.is_empty()
        || update
            .get("sessionUpdate")
            .and_then(serde_json::Value::as_str)
            != Some("user_message_chunk")
    {
        return;
    }
    let Some(content) = update
        .get_mut("content")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    if content.get("type").and_then(serde_json::Value::as_str) != Some("image") {
        return;
    }
    let Some(digest) = content
        .get("data")
        .and_then(serde_json::Value::as_str)
        .and_then(large_image_digest)
        .filter(|digest| stored.contains(digest))
    else {
        return;
    };
    content.remove("data");
    content.insert(
        ECHO_ARTIFACT_FIELD.to_owned(),
        serde_json::Value::String(digest),
    );
}

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

    /// Store a prompt's large inline images before it is dispatched to a
    /// worker, so the worker may echo them by digest instead of returning the
    /// bytes over the Machine link. Returns whether any image was stored.
    pub fn store_prompt_images(&self, content: &[serde_json::Value]) -> Result<bool> {
        let mut stored = false;
        for block in content {
            if block.get("type").and_then(serde_json::Value::as_str) != Some("image") {
                continue;
            }
            let Some(data) = block.get("data").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let mime = block
                .get("mimeType")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("image/png");
            stored |= self.put(data, mime)?.is_some();
        }
        Ok(stored)
    }

    fn put(&self, encoded: &str, mime: &str) -> Result<Option<String>> {
        // Small icons cost more as separate HTTP requests; externalize only
        // payloads large enough to materially affect JSONB/WS history.
        if encoded.len() < INLINE_IMAGE_LIMIT_BYTES {
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
            refresh_age(&path)?;
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
        return resolve_echo_reference(store, object, mime);
    };
    if let Some(url) = store.put(data, mime)? {
        object.remove("data");
        object.insert("url".to_owned(), serde_json::Value::String(url));
    }
    Ok(())
}

fn resolve_echo_reference(
    store: &ArtifactStore,
    object: &mut serde_json::Map<String, serde_json::Value>,
    mime: &str,
) -> Result<()> {
    let Some(digest) = object
        .get(ECHO_ARTIFACT_FIELD)
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(());
    };
    let name = format!("{digest}.{}", extension_for_mime(mime));
    let Some(path) = store.path(&name) else {
        tracing::warn!(artifact = %name, "echoed image reference has no stored artifact");
        return Ok(());
    };
    refresh_age(&path)?;
    object.remove(ECHO_ARTIFACT_FIELD);
    object.insert(
        "url".to_owned(),
        serde_json::Value::String(format!("/api/artifacts/{name}")),
    );
    Ok(())
}

fn refresh_age(path: &std::path::Path) -> Result<()> {
    std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now()))
        .with_context(|| format!("refreshing artifact {}", path.display()))
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
    fn prompt_echo_reference_resolves_to_the_inline_artifact_url() {
        let root =
            std::env::temp_dir().join(format!("cowboy-artifact-echo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = ArtifactStore::new(root.clone()).unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(vec![9_u8; 40_000]);
        let image = serde_json::json!({"type":"image","data":data,"mimeType":"image/jpeg"});
        let small = serde_json::json!({"type":"image","data":"c2hvdA==","mimeType":"image/png"});
        assert!(
            !store
                .store_prompt_images(std::slice::from_ref(&small))
                .unwrap()
        );
        assert!(store.store_prompt_images(&[image.clone(), small]).unwrap());

        let mut inline = image.clone();
        store.externalize_images(&mut inline).unwrap();
        let digest = large_image_digest(&data).unwrap();
        let mut echo = serde_json::json!({"sessionUpdate":"user_message_chunk","content":image});
        reference_echoed_image(&mut echo, std::slice::from_ref(&digest));
        assert!(echo["content"].get("data").is_none());
        store.externalize_images(&mut echo).unwrap();
        assert_eq!(echo["content"], inline);

        let unknown = "b".repeat(64);
        let mut missing = serde_json::json!({
            "type":"image","mimeType":"image/jpeg",ECHO_ARTIFACT_FIELD:unknown,
        });
        store.externalize_images(&mut missing).unwrap();
        assert!(
            missing.get("url").is_none(),
            "an unknown digest is never resolved"
        );
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
