use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use cowboy_plugin_sdk::{
    PLUGIN_RELEASE_SIGNATURE_NAMESPACE, PluginKind, PluginManifest, PluginPackage, PluginPayload,
    PluginRelease, PluginRuntimeArtifacts, RELEASE_SCHEMA_VERSION,
};
use cowboy_provider_sdk::{PlatformTarget, StandardProviderSource, build_package};

fn main() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let Some(command) = arguments.first().map(|value| value.to_string_lossy()) else {
        return usage();
    };
    match command.as_ref() {
        "build" => build(&arguments[1..]),
        "set-artifact-url" => set_artifact_url(&arguments[1..]),
        "bind-runtime" => bind_runtime(&arguments[1..]),
        "sign" => sign(&arguments[1..]),
        "verify" => verify(&arguments[1..]),
        "inspect" => inspect(&arguments[1..]),
        _ => usage(),
    }
}

fn usage<T>() -> Result<T> {
    bail!(
        "usage:\n  cowboy-plugin-pack build <plugin-dir> <output.cowboy-plugin> [artifact-url]\n  cowboy-plugin-pack set-artifact-url <artifact> <release.json> <immutable-https-url>\n  cowboy-plugin-pack bind-runtime <artifact> <release.json> <runtime-artifacts.json>\n  cowboy-plugin-pack sign <artifact> <release.json> <private-key>\n  cowboy-plugin-pack verify <artifact> <release.json> <public-key>\n  cowboy-plugin-pack inspect <artifact>"
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
    write_atomic(&output, &bytes)?;
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
        release_schema: RELEASE_SCHEMA_VERSION,
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
        signature: String::new(),
        supported_platforms,
        runtime_artifacts: Vec::new(),
    };
    if package.authentication_provider().is_some() {
        release.artifact_digest = release.computed_artifact_digest()?;
    }
    let release_path = output.with_extension("release.json");
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
        arguments.len() == 3,
        "sign requires artifact, release, and private key"
    );
    let artifact = PathBuf::from(&arguments[0]);
    let release_path = PathBuf::from(&arguments[1]);
    let bytes = std::fs::read(&artifact)?;
    let mut release: PluginRelease = read_json(&release_path)?;
    "pending-signature".clone_into(&mut release.signature);
    release.validate_bytes(&bytes)?;
    release.signature = ssh_sign(Path::new(&arguments[2]), &release.proof())?;
    write_json_atomic(&release_path, &release)
}

fn verify(arguments: &[std::ffi::OsString]) -> Result<()> {
    ensure!(
        arguments.len() == 3,
        "verify requires artifact, release, and public key"
    );
    let bytes = std::fs::read(&arguments[0])?;
    let release: PluginRelease = read_json(Path::new(&arguments[1]))?;
    release.validate_bytes(&bytes)?;
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
