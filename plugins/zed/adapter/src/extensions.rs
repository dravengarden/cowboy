//! Advertise Zed's already imported remote extensions to the private server.
use super::*;

pub(super) struct ExtensionManifest {
    pub(super) id: String,
    pub(super) version: String,
}

pub(super) fn extension_manifest(contents: &str) -> Result<ExtensionManifest> {
    fn string_field(contents: &str, wanted: &str) -> Option<String> {
        contents
            .lines()
            .map(str::trim)
            .take_while(|line| !line.starts_with('['))
            .filter_map(|line| line.split_once('='))
            .find_map(|(key, value)| {
                (key.trim() == wanted).then(|| {
                    value
                        .trim()
                        .strip_prefix('"')?
                        .strip_suffix('"')
                        .map(str::to_owned)
                })?
            })
    }
    let id = string_field(contents, "id").context("extension manifest has no string id")?;
    let version =
        string_field(contents, "version").context("extension manifest has no string version")?;
    Ok(ExtensionManifest { id, version })
}

impl ZedRuntime {
    pub(super) async fn sync_extensions(&self, data_home: &Path) -> Result<usize> {
        let root = data_home.join("zed/remote_extensions");
        let mut directory = match tokio::fs::read_dir(&root).await {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        let mut extensions = Vec::new();
        while let Some(entry) = directory.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let manifest_path = entry.path().join("extension.toml");
            // Zed may leave a partially downloaded extension directory behind
            // (manifest present, wasm absent). Advertising it makes the server
            // reject the entire sync. Ignore it until the official Zed import
            // becomes complete on a later adapter restart.
            if !entry.path().join("extension.wasm").is_file() {
                continue;
            }
            let manifest = match tokio::fs::read_to_string(&manifest_path).await {
                Ok(contents) => extension_manifest(&contents)
                    .with_context(|| format!("invalid {}", manifest_path.display()))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            extensions.push(proto::Extension {
                id: manifest.id,
                version: manifest.version,
                dev: false,
            });
        }
        extensions.sort_by(|left, right| left.id.cmp(&right.id));
        let count = extensions.len();
        let response = tokio::time::timeout(
            Duration::from_secs(30),
            self.request(proto::envelope::Payload::SyncExtensions(
                proto::SyncExtensions { extensions },
            )),
        )
        .await
        .context("Zed extension sync timed out")??;
        let Some(proto::envelope::Payload::SyncExtensionsResponse(response)) = response.payload
        else {
            bail!("Zed returned the wrong SyncExtensions response");
        };
        if !response.missing_extensions.is_empty() {
            let missing = response
                .missing_extensions
                .iter()
                .map(|extension| extension.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("Zed could not load extensions: {missing}");
        }
        Ok(count)
    }
}
