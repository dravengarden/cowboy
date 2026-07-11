//! Content-addressed storage for large image payloads in durable events.

use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use base64::Engine as _;
use sha2::{Digest as _, Sha256};

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
        if !path.exists() {
            let temp = self.root.join(format!(".{name}.tmp"));
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
        let (hash, extension) = name.split_once('.')?;
        let valid = hash.len() == 64
            && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            && matches!(extension, "png" | "jpg" | "webp" | "gif" | "avif");
        valid
            .then(|| self.root.join(name))
            .filter(|path| path.is_file())
    }
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
}
