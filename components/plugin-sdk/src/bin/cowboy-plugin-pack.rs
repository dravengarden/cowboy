use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use cowboy_plugin_sdk::{
    HOST_BUNDLE_SCHEMA, PLUGIN_RELEASE_SIGNATURE_NAMESPACE, PluginHostSpec, PluginKind,
    PluginManifest, PluginPackage, PluginPayload, PluginRelease, PluginRuntimeArtifacts,
    RELEASE_SCHEMA_MIN_VERSION, RELEASE_SCHEMA_VERSION,
};
use cowboy_provider_sdk::{PlatformTarget, StandardProviderSource, build_package};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const MAX_HOST_BUNDLE_FILES: usize = 32;
const MAX_HOST_BUNDLE_FILE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HostBundle {
    schema: String,
    plugin_id: String,
    plugin_version: String,
    package_digest: String,
    files: BTreeMap<String, String>,
}

fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let Some(command) = arguments.first().map(|value| value.to_string_lossy()) else {
        return usage();
    };
    match command.as_ref() {
        "version" if arguments.len() == 1 => {
            println!(
                "cowboy-plugin-pack {}",
                cowboy_plugin_sdk::PLUGIN_SDK_VERSION
            );
            Ok(())
        }
        "build" => build(&arguments[1..]),
        "set-artifact-url" => set_artifact_url(&arguments[1..]),
        "bind-host" => bind_host(&arguments[1..]),
        "bind-runtime" => bind_runtime(&arguments[1..]),
        "sign" => sign(&arguments[1..]),
        "verify" => verify(&arguments[1..]),
        "inspect" => inspect(&arguments[1..]),
        _ => usage(),
    }
}

fn usage<T>() -> Result<T> {
    bail!(
        "usage:\n  cowboy-plugin-pack build <plugin-dir> <output.cowboy-plugin> [artifact-url]\n  cowboy-plugin-pack set-artifact-url <artifact> <release.json> <immutable-https-url>\n  cowboy-plugin-pack bind-host <artifact> <release.json> <hostbundle.json>\n  cowboy-plugin-pack bind-runtime <artifact> <release.json> <runtime-artifacts.json>\n  cowboy-plugin-pack sign <artifact> <release.json> <private-key> [hostbundle.json]\n  cowboy-plugin-pack verify <artifact> <release.json> <public-key> [hostbundle.json]\n  cowboy-plugin-pack inspect <artifact>"
    )
}

fn build(arguments: &[std::ffi::OsString]) -> Result<()> {
    ensure!(
        (2..=3).contains(&arguments.len()),
        "build requires plugin directory and output"
    );
    let root = PathBuf::from(&arguments[0]);
    let output = PathBuf::from(&arguments[1]);
    let manifest: PluginManifest = read_json(&root.join("plugin.json"))?;
    let payload = match manifest.kind {
        PluginKind::AgentProvider => {
            let source: StandardProviderSource = read_json(&root.join(&manifest.entrypoint))?;
            PluginPayload::AgentProvider(Box::new(build_package(source.compile()?)?))
        }
        PluginKind::AuthenticationProvider => {
            PluginPayload::AuthenticationProvider(read_json(&root.join(&manifest.entrypoint))?)
        }
        PluginKind::CodeIntelligence => {
            PluginPayload::CodeIntelligence(read_json(&root.join(&manifest.entrypoint))?)
        }
    };
    let component_release = manifest.component_release.clone();
    let package = PluginPackage::new(manifest, component_release, payload)?;
    let bytes = package.canonical_bytes()?;
    let digest = PluginPackage::artifact_digest(&bytes);
    let supported_platforms = match &package.payload {
        PluginPayload::AgentProvider(provider) => provider
            .manifest
            .runtime
            .platforms
            .iter()
            .map(|platform| PlatformTarget {
                os: platform.os.clone(),
                architecture: platform.architecture.clone(),
            })
            .collect(),
        PluginPayload::AuthenticationProvider(_) => Vec::new(),
        PluginPayload::CodeIntelligence(contract) => contract.supported_platforms.clone(),
    };
    let mut release = PluginRelease {
        release_schema: RELEASE_SCHEMA_MIN_VERSION,
        plugin_id: package.manifest.id.clone(),
        plugin_version: package.manifest.version.clone(),
        plugin_kind: package.manifest.kind,
        package_digest: digest.clone(),
        artifact_digest: String::new(),
        artifact_url: arguments.get(2).map_or_else(
            || output.display().to_string(),
            |value| value.to_string_lossy().into_owned(),
        ),
        publisher: package.manifest.publisher.clone(),
        contract_fingerprint: package.contract_fingerprint.clone(),
        component_release: package.component_release.clone(),
        host_bundle_digest: None,
        signature: String::new(),
        supported_platforms,
        runtime_artifacts: Vec::new(),
    };
    let host_bundle = build_host_bundle(&root, &release)?;
    package.validate_host_contract(host_bundle.as_ref().map(|bundle| &bundle.files))?;
    let host_bundle_path = output.with_extension("hostbundle.json");
    let host_bundle_bytes = if let Some(host_bundle) = host_bundle {
        let mut host_bundle_bytes = serde_json::to_vec_pretty(&host_bundle)?;
        host_bundle_bytes.push(b'\n');
        release.release_schema = RELEASE_SCHEMA_VERSION;
        release.host_bundle_digest =
            Some(format!("sha256:{:x}", Sha256::digest(&host_bundle_bytes)));
        Some(host_bundle_bytes)
    } else {
        ensure!(
            !host_bundle_path.exists(),
            "hostless build output has a stale host bundle"
        );
        None
    };
    if package.authentication_provider().is_some() || host_bundle_bytes.is_some() {
        release.artifact_digest = release.computed_artifact_digest()?;
    }
    let release_path = output.with_extension("release.json");
    write_atomic(&output, &bytes)?;
    if let Some(host_bundle_bytes) = host_bundle_bytes {
        write_atomic(&host_bundle_path, &host_bundle_bytes)?;
    }
    write_json_atomic(&release_path, &release)?;
    println!(
        "{}\t{}\t{}",
        package.manifest.id,
        digest,
        release_path.display()
    );
    Ok(())
}

fn set_artifact_url(arguments: &[std::ffi::OsString]) -> Result<()> {
    ensure!(
        arguments.len() == 3,
        "set-artifact-url requires artifact, release, and URL"
    );
    let artifact = PathBuf::from(&arguments[0]);
    let release_path = PathBuf::from(&arguments[1]);
    let url = arguments[2].to_string_lossy().into_owned();
    ensure!(
        url.starts_with("https://")
            && !url.contains("latest")
            && !url.bytes().any(|byte| byte.is_ascii_whitespace()),
        "plugin artifact URL must be immutable HTTPS"
    );
    let bytes = std::fs::read(&artifact)?;
    let package = PluginPackage::from_bytes(&bytes)?;
    let mut release: PluginRelease = read_json(&release_path)?;
    ensure!(
        release.signature.is_empty(),
        "cannot rewrite a signed plugin release"
    );
    ensure!(
        release.plugin_id == package.manifest.id
            && release.plugin_version == package.manifest.version
            && release.package_digest == PluginPackage::artifact_digest(&bytes),
        "release does not match plugin package"
    );
    release.artifact_url = url;
    write_json_atomic(&release_path, &release)
}

fn bind_host(arguments: &[std::ffi::OsString]) -> Result<()> {
    ensure!(
        arguments.len() == 3,
        "bind-host requires artifact, release, and host bundle"
    );
    let artifact = PathBuf::from(&arguments[0]);
    let release_path = PathBuf::from(&arguments[1]);
    let host_bundle = PathBuf::from(&arguments[2]);
    let bytes = std::fs::read(&artifact)?;
    let package = PluginPackage::from_bytes(&bytes)?;
    let mut release: PluginRelease = read_json(&release_path)?;
    ensure!(
        release.signature.is_empty(),
        "cannot rebind a signed plugin release"
    );
    ensure!(
        release.plugin_id == package.manifest.id
            && release.plugin_version == package.manifest.version
            && release.package_digest == PluginPackage::artifact_digest(&bytes),
        "release does not match plugin package"
    );
    let digest = validate_host_bundle_identity(&package, &release, &host_bundle)?;
    release.release_schema = RELEASE_SCHEMA_VERSION;
    release.host_bundle_digest = Some(digest);
    release.artifact_digest = release.computed_artifact_digest()?;
    write_json_atomic(&release_path, &release)
}

fn bind_runtime(arguments: &[std::ffi::OsString]) -> Result<()> {
    ensure!(
        arguments.len() == 3,
        "bind-runtime requires artifact, release, and runtime manifest"
    );
    let artifact = PathBuf::from(&arguments[0]);
    let release_path = PathBuf::from(&arguments[1]);
    let bytes = std::fs::read(&artifact)?;
    let package = PluginPackage::from_bytes(&bytes)?;
    let mut release: PluginRelease = read_json(&release_path)?;
    ensure!(
        release.signature.is_empty(),
        "cannot rebind a signed plugin release"
    );
    release.runtime_artifacts = read_json::<Vec<PluginRuntimeArtifacts>>(Path::new(&arguments[2]))?;
    release.artifact_digest = release.computed_artifact_digest()?;
    "pending-signature".clone_into(&mut release.signature);
    release.validate_for(&package)?;
    release.signature.clear();
    write_json_atomic(&release_path, &release)
}

fn sign(arguments: &[std::ffi::OsString]) -> Result<()> {
    ensure!(
        (3..=4).contains(&arguments.len()),
        "sign requires artifact, release, private key, and any bound host bundle"
    );
    let artifact = PathBuf::from(&arguments[0]);
    let release_path = PathBuf::from(&arguments[1]);
    let bytes = std::fs::read(&artifact)?;
    let mut release: PluginRelease = read_json(&release_path)?;
    "pending-signature".clone_into(&mut release.signature);
    let package = release.validate_bytes(&bytes)?;
    validate_host_bundle_binding(&package, &release, arguments.get(3).map(Path::new))?;
    release.signature = ssh_sign(Path::new(&arguments[2]), &release.proof())?;
    write_json_atomic(&release_path, &release)
}

fn verify(arguments: &[std::ffi::OsString]) -> Result<()> {
    ensure!(
        (3..=4).contains(&arguments.len()),
        "verify requires artifact, release, public key, and any bound host bundle"
    );
    let bytes = std::fs::read(&arguments[0])?;
    let release: PluginRelease = read_json(Path::new(&arguments[1]))?;
    let package = release.validate_bytes(&bytes)?;
    validate_host_bundle_binding(&package, &release, arguments.get(3).map(Path::new))?;
    let public_key = normalize_public_key(&std::fs::read_to_string(&arguments[2])?)?;
    ensure!(
        ssh_verify(&public_key, &release.proof(), &release.signature)?,
        "invalid plugin signature"
    );
    println!(
        "{}\t{}\tverified",
        release.plugin_id, release.artifact_digest
    );
    Ok(())
}

fn validate_host_bundle_binding(
    package: &PluginPackage,
    release: &PluginRelease,
    path: Option<&Path>,
) -> Result<()> {
    match (release.host_bundle_digest.as_deref(), path) {
        (Some(expected), Some(path)) => {
            let actual = validate_host_bundle_identity(package, release, path)?;
            ensure!(actual == expected, "plugin host bundle digest mismatch");
            Ok(())
        }
        (Some(_), None) => bail!("bound plugin host bundle is missing"),
        (None, Some(_)) => bail!("plugin host bundle is not bound by the release"),
        (None, None) => package.validate_host_contract(None),
    }
}

fn validate_host_bundle_identity(
    package: &PluginPackage,
    release: &PluginRelease,
    path: &Path,
) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("reading host bundle {}", path.display()))?;
    let bundle: HostBundle = serde_json::from_slice(&bytes)
        .with_context(|| format!("decoding host bundle {}", path.display()))?;
    validate_host_bundle(&bundle)?;
    package.validate_host_contract(Some(&bundle.files))?;
    ensure!(
        bundle.plugin_id == release.plugin_id
            && bundle.plugin_version == release.plugin_version
            && bundle.package_digest == release.package_digest,
        "plugin host bundle identity mismatch"
    );
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn build_host_bundle(root: &Path, release: &PluginRelease) -> Result<Option<HostBundle>> {
    let host_path = root.join("host.json");
    let host_metadata = match std::fs::symlink_metadata(&host_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", host_path.display()));
        }
    };
    ensure!(
        host_metadata.file_type().is_file(),
        "plugin host.json is not a regular file"
    );
    reject_plugin_ui(&root.join("ui"))?;
    let mut files = BTreeMap::from([(
        "host.json".to_owned(),
        std::fs::read_to_string(&host_path)
            .with_context(|| format!("reading {}", host_path.display()))?,
    )]);
    let collector_root = root.join("collector");
    match std::fs::symlink_metadata(&collector_root) {
        Ok(metadata) => {
            ensure!(
                metadata.file_type().is_dir(),
                "plugin collector source is not a directory"
            );
            collect_host_files(&collector_root, "collector", &mut files)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", collector_root.display()));
        }
    }
    let bundle = HostBundle {
        schema: HOST_BUNDLE_SCHEMA.to_owned(),
        plugin_id: release.plugin_id.clone(),
        plugin_version: release.plugin_version.clone(),
        package_digest: release.package_digest.clone(),
        files,
    };
    validate_host_bundle(&bundle)?;
    Ok(Some(bundle))
}

fn reject_plugin_ui(root: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("inspecting {}", root.display())),
    };
    ensure!(
        metadata.file_type().is_dir(),
        "plugin UI source is not a directory: {}",
        root.display()
    );
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => return Err(error).with_context(|| format!("reading {}", root.display())),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            reject_plugin_ui(&path)?;
        } else {
            bail!("plugin host UI code is forbidden: {}", path.display());
        }
    }
    Ok(())
}

fn collect_host_files(
    root: &Path,
    relative_root: &str,
    files: &mut BTreeMap<String, String>,
) -> Result<()> {
    let mut entries = std::fs::read_dir(root)
        .with_context(|| format!("reading {}", root.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("plugin host file name is not UTF-8"))?;
        let relative = format!("{relative_root}/{name}");
        validate_host_bundle_path(&relative)?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_host_files(&entry.path(), &relative, files)?;
        } else if file_type.is_file() {
            ensure!(
                matches!(
                    entry.path().extension().and_then(std::ffi::OsStr::to_str),
                    Some("js" | "json")
                ),
                "unsupported collector file in plugin host bundle: {relative}"
            );
            files.insert(
                relative,
                std::fs::read_to_string(entry.path())
                    .with_context(|| format!("reading {}", entry.path().display()))?,
            );
        } else {
            bail!("plugin host source is not a regular file: {relative}");
        }
    }
    Ok(())
}

fn validate_host_bundle(bundle: &HostBundle) -> Result<()> {
    ensure!(
        bundle.schema == HOST_BUNDLE_SCHEMA,
        "unsupported plugin host bundle schema"
    );
    ensure!(
        !bundle.files.is_empty() && bundle.files.len() <= MAX_HOST_BUNDLE_FILES,
        "plugin host bundle file set is empty or too large"
    );
    for (path, content) in &bundle.files {
        validate_host_bundle_path(path)?;
        if path != "host.json" {
            ensure!(
                matches!(
                    Path::new(path)
                        .extension()
                        .and_then(std::ffi::OsStr::to_str),
                    Some("js" | "json")
                ),
                "unsupported collector file in plugin host bundle: {path}"
            );
        }
        ensure!(
            !content.is_empty() && content.len() <= MAX_HOST_BUNDLE_FILE_BYTES,
            "plugin host bundle file {path} is empty or oversized"
        );
        ensure!(
            !content.contains('\0'),
            "plugin host bundle file {path} contains NUL"
        );
    }
    let host = bundle
        .files
        .get("host.json")
        .context("plugin host bundle is missing host.json")?;
    let host = PluginHostSpec::from_json(host.as_bytes())
        .context("plugin host.json fails semantic validation")?;
    host.validate_runtime_files(&bundle.files)
        .context("plugin host runtime files are invalid")?;
    Ok(())
}

fn validate_host_bundle_path(path: &str) -> Result<()> {
    if path == "host.json" {
        return Ok(());
    }
    let relative = path
        .strip_prefix("collector/")
        .context("plugin host bundle contains an unsupported path")?;
    ensure!(!relative.is_empty(), "plugin host bundle path is empty");
    for part in relative.split('/') {
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                }),
            "unsafe plugin host bundle path {path}"
        );
    }
    Ok(())
}

fn inspect(arguments: &[std::ffi::OsString]) -> Result<()> {
    ensure!(arguments.len() == 1, "inspect requires one artifact");
    let package = PluginPackage::from_bytes(&std::fs::read(&arguments[0])?)?;
    println!("{}", serde_json::to_string_pretty(&package)?);
    Ok(())
}

fn ssh_sign(private_key: &Path, proof: &[u8]) -> Result<String> {
    let mut child = Command::new("ssh-keygen")
        .args(["-Y", "sign", "-f"])
        .arg(private_key)
        .args(["-n", PLUGIN_RELEASE_SIGNATURE_NAMESPACE])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .context("opening ssh-keygen stdin")?
        .write_all(proof)?;
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "ssh-keygen sign failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(base64::engine::general_purpose::STANDARD.encode(output.stdout))
}

fn ssh_verify(public_key: &str, proof: &[u8], signature: &str) -> Result<bool> {
    let root = tempfile::tempdir()?;
    let allowed = root.path().join("allowed_signers");
    let signature_path = root.path().join("release.sig");
    std::fs::write(&allowed, format!("cowboy-plugin {public_key}\n"))?;
    ensure!(signature.len() <= 32 * 1_024, "SSH signature is too large");
    let signature = if signature.starts_with("-----BEGIN SSH SIGNATURE-----") {
        signature.as_bytes().to_vec()
    } else {
        base64::engine::general_purpose::STANDARD.decode(signature)?
    };
    ensure!(
        signature.len() <= 16 * 1_024 && signature.starts_with(b"-----BEGIN SSH SIGNATURE-----"),
        "invalid SSH signature encoding"
    );
    std::fs::write(&signature_path, signature)?;
    let mut child = Command::new("ssh-keygen")
        .args(["-Y", "verify", "-f"])
        .arg(&allowed)
        .args([
            "-I",
            "cowboy-plugin",
            "-n",
            PLUGIN_RELEASE_SIGNATURE_NAMESPACE,
            "-s",
        ])
        .arg(&signature_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .context("opening ssh-keygen stdin")?
        .write_all(proof)?;
    Ok(child.wait()?.success())
}

fn normalize_public_key(value: &str) -> Result<String> {
    let fields = value.split_whitespace().collect::<Vec<_>>();
    ensure!(fields.len() >= 2, "invalid SSH public key");
    ensure!(fields[0].starts_with("ssh-"), "unsupported SSH public key");
    base64::engine::general_purpose::STANDARD.decode(fields[1])?;
    Ok(format!("{} {}", fields[0], fields[1]))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("decoding {}", path.display()))
}

fn write_json_atomic(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("output has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> PluginRelease {
        PluginRelease {
            release_schema: RELEASE_SCHEMA_MIN_VERSION,
            plugin_id: "example".to_owned(),
            plugin_version: "1.0.0".to_owned(),
            plugin_kind: PluginKind::AuthenticationProvider,
            package_digest: format!("sha256:{}", "a".repeat(64)),
            artifact_digest: String::new(),
            artifact_url: "cowboy-plugin://example".to_owned(),
            publisher: "example-publisher".to_owned(),
            contract_fingerprint: format!("sha256:{}", "b".repeat(64)),
            component_release: "1.0.0".to_owned(),
            host_bundle_digest: None,
            signature: String::new(),
            supported_platforms: Vec::new(),
            runtime_artifacts: Vec::new(),
        }
    }

    #[test]
    fn sdk_build_collects_only_bounded_host_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("host.json"), r#"{"schema_version":1}"#).unwrap();
        std::fs::create_dir(root.path().join("collector")).unwrap();
        std::fs::write(
            root.path().join("collector/index.js"),
            "export const collect = true;",
        )
        .unwrap();
        let bundle = build_host_bundle(root.path(), &release())
            .unwrap()
            .expect("host bundle");
        assert!(bundle.files.contains_key("host.json"));
        assert!(bundle.files.contains_key("collector/index.js"));
        assert!(validate_host_bundle(&bundle).is_ok());
    }

    #[test]
    fn sdk_build_rejects_ui_code_and_unsupported_sidecars() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("host.json"), r#"{"schema_version":1}"#).unwrap();
        std::fs::create_dir(root.path().join("ui")).unwrap();
        std::fs::write(root.path().join("ui/index.js"), "export default 1;").unwrap();
        let error = build_host_bundle(root.path(), &release()).unwrap_err();
        assert!(error.to_string().contains("UI code is forbidden"));

        std::fs::remove_dir_all(root.path().join("ui")).unwrap();
        std::fs::create_dir(root.path().join("collector")).unwrap();
        std::fs::write(root.path().join("collector/run.sh"), "exit 0").unwrap();
        let error = build_host_bundle(root.path(), &release()).unwrap_err();
        assert!(error.to_string().contains("unsupported collector file"));
    }

    #[test]
    fn sdk_build_rejects_invalid_host_semantics_and_runtime_links() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("host.json"),
            r#"{"schema_version":1,"slots":["provider.usage"]}"#,
        )
        .unwrap();
        let error = build_host_bundle(root.path(), &release()).unwrap_err();
        assert!(
            format!("{error:#}").contains("plugin slots require a data-only renderer declaration")
        );

        std::fs::write(
            root.path().join("host.json"),
            r#"{"schema_version":1,"usage":{"account":"future","collector":"command","collector_argv":["@plugin-js","run","${PLUGIN_DIR}/collector/missing.js"]}}"#,
        )
        .unwrap();
        let error = build_host_bundle(root.path(), &release()).unwrap_err();
        assert!(format!("{error:#}").contains("collector/missing.js"));
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Hermetic build/bind/sign/verify and downgrade conformance.
    fn local_authentication_build_and_signature_bind_the_required_host() {
        let source = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let artifact = output.path().join("future-login.cowboy-plugin");
        let release_path = artifact.with_extension("release.json");
        let host_path = artifact.with_extension("hostbundle.json");
        write_json_atomic(
            &source.path().join("plugin.json"),
            &serde_json::json!({
                "schema_version":1,"id":"future-login","version":"1.0.0",
                "component_release":"2.4.0","publisher":"fixture","kind":"authentication_provider",
                "entrypoint":"authentication.json","components":[
                    {"id":"cowboy.plugin-contract","version":"1.5.0"},
                    {"id":"cowboy.plugin-sdk","version":cowboy_plugin_sdk::PLUGIN_SDK_VERSION}
                ]
            }),
        )
        .unwrap();
        write_json_atomic(
            &source.path().join("authentication.json"),
            &serde_json::json!({
                "schema_version":2,"id":"future-login","version":"1.0.0",
                "display_name":"Future login","button_label":"Continue",
                "protocol":{"kind":"local_password","configuration":{}}
            }),
        )
        .unwrap();
        write_json_atomic(
            &source.path().join("host.json"),
            &serde_json::json!({
                "schema_version":1,"slots":["login.method"],
                "ui":{"schema_version":1,"renderers":{"login.method":"login-password-v1"}}
            }),
        )
        .unwrap();
        build(&[source.path().into(), artifact.clone().into_os_string()]).unwrap();
        let package = PluginPackage::from_bytes(&std::fs::read(&artifact).unwrap()).unwrap();
        let unsigned: PluginRelease = read_json(&release_path).unwrap();
        assert_eq!(unsigned.release_schema, 2);
        assert!(unsigned.signature.is_empty());
        assert!(unsigned.runtime_artifacts.is_empty());
        assert!(unsigned.supported_platforms.is_empty());
        let url = format!(
            "https://plugins.example/artifacts/{}/future-login.cowboy-plugin",
            &unsigned.package_digest[7..]
        );
        set_artifact_url(&[
            artifact.clone().into_os_string(),
            release_path.clone().into_os_string(),
            url.into(),
        ])
        .unwrap();

        let key = output.path().join("fixture-key");
        let keygen = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "fixture", "-f"])
            .arg(&key)
            .output()
            .unwrap();
        assert!(keygen.status.success(), "fixture key generation failed");
        sign(&[
            artifact.clone().into_os_string(),
            release_path.clone().into_os_string(),
            key.into_os_string(),
            host_path.clone().into_os_string(),
        ])
        .unwrap();
        verify(&[
            artifact.clone().into_os_string(),
            release_path.clone().into_os_string(),
            output.path().join("fixture-key.pub").into_os_string(),
            host_path.into_os_string(),
        ])
        .unwrap();
        let signed: PluginRelease = read_json(&release_path).unwrap();
        signed.validate_for(&package).unwrap();
        let mut downgraded = signed;
        downgraded.release_schema = 1;
        downgraded.host_bundle_digest = None;
        downgraded.artifact_digest = downgraded.computed_artifact_digest().unwrap();
        assert!(
            downgraded
                .validate_for(&package)
                .unwrap_err()
                .to_string()
                .contains("requires a release-bound host")
        );

        // The builder and bind/sign/verify entrypoints must all reject
        // protocol-confused host data, not just validate its JSON shape.
        let mut bundle: HostBundle =
            read_json(&artifact.with_extension("hostbundle.json")).unwrap();
        let mut wrong: serde_json::Value =
            serde_json::from_str(&bundle.files["host.json"]).unwrap();
        wrong["ui"]["renderers"]["login.method"] = serde_json::json!("login-oidc-v1");
        bundle
            .files
            .insert("host.json".to_owned(), wrong.to_string());
        let wrong_path = output.path().join("wrong.hostbundle.json");
        write_json_atomic(&wrong_path, &bundle).unwrap();
        assert!(validate_host_bundle_identity(&package, &unsigned, &wrong_path).is_err());
        write_json_atomic(&source.path().join("host.json"), &wrong).unwrap();
        assert!(
            build(&[
                source.path().into(),
                output.path().join("wrong.cowboy-plugin").into_os_string()
            ])
            .is_err()
        );
        assert!(!output.path().join("wrong.cowboy-plugin").exists());
    }
}
