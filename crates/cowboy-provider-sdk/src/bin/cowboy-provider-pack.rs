use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail, ensure};
use cowboy_provider_sdk::{
    PROVIDER_RELEASE_SIGNATURE_NAMESPACE, PlatformRuntimeArtifacts, PlatformTarget,
    ProviderPackage, ProviderRelease, RELEASE_SCHEMA_VERSION, build_package_file,
};

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
        // Preserve the original two-positional-argument interface for scripts
        // created during SDK v1 development.
        _ => build(&arguments),
    }
}

fn usage<T>() -> Result<T> {
    bail!(
        "usage:\n  cowboy-provider-pack build <provider.json> <output.cowboy-provider> [artifact-url]\n  cowboy-provider-pack set-artifact-url <artifact> <release.json> <immutable-https-url>\n  cowboy-provider-pack bind-runtime <artifact> <release.json> <runtime-artifacts.json>\n  cowboy-provider-pack sign <artifact> <release.json> <private-key>\n  cowboy-provider-pack verify <artifact> <release.json> <public-key>\n  cowboy-provider-pack inspect <artifact>"
    )
}

fn build(arguments: &[std::ffi::OsString]) -> Result<()> {
    let source = arguments
        .first()
        .map(PathBuf::from)
        .context("missing Provider source")?;
    let output = arguments
        .get(1)
        .map(PathBuf::from)
        .context("missing Provider output")?;
    let artifact_url = arguments
        .get(2)
        .map(|value| value.to_string_lossy().into_owned());
    ensure!(arguments.len() <= 3, "too many build arguments");
    let (package, digest) = build_package_file(&source, &output)?;
    let release = ProviderRelease {
        release_schema: RELEASE_SCHEMA_VERSION,
        provider_id: package.manifest.id.clone(),
        provider_version: package.manifest.version.clone(),
        package_digest: digest.clone(),
        artifact_digest: String::new(),
        artifact_url: artifact_url.unwrap_or_else(|| output.display().to_string()),
        publisher: package.manifest.publisher.clone(),
        contract_fingerprint: package.contract_fingerprint.clone(),
        signature: String::new(),
        supported_platforms: package
            .manifest
            .runtime
            .platforms
            .iter()
            .map(|payload| PlatformTarget {
                os: payload.os.clone(),
                architecture: payload.architecture.clone(),
            })
            .collect(),
        runtime_artifacts: Vec::new(),
    };
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
        "set-artifact-url requires artifact, release, and immutable HTTPS URL"
    );
    let artifact = PathBuf::from(&arguments[0]);
    let release_path = PathBuf::from(&arguments[1]);
    let artifact_url = arguments[2].to_string_lossy().into_owned();
    let bytes = std::fs::read(&artifact)?;
    let package = ProviderPackage::from_bytes(&bytes)?;
    let mut release: ProviderRelease = serde_json::from_slice(&std::fs::read(&release_path)?)?;
    ensure!(
        release.signature.is_empty(),
        "cannot rewrite a signed Provider release"
    );
    ensure!(
        artifact_url.starts_with("https://")
            && !artifact_url.contains("latest")
            && !artifact_url.bytes().any(|byte| byte.is_ascii_whitespace()),
        "Provider artifact URL must be immutable HTTPS"
    );
    ensure!(
        release.provider_id == package.manifest.id
            && release.provider_version == package.manifest.version
            && release.package_digest == ProviderPackage::artifact_digest(&bytes),
        "release does not match Provider package"
    );
    release.artifact_url = artifact_url;
    write_json_atomic(&release_path, &release)?;
    println!("{}\t{}", release.provider_id, release.artifact_url);
    Ok(())
}

fn bind_runtime(arguments: &[std::ffi::OsString]) -> Result<()> {
    ensure!(
        arguments.len() == 3,
        "bind-runtime requires artifact, release, and runtime artifact manifest"
    );
    let artifact = PathBuf::from(&arguments[0]);
    let release_path = PathBuf::from(&arguments[1]);
    let runtime_path = PathBuf::from(&arguments[2]);
    let bytes =
        std::fs::read(&artifact).with_context(|| format!("reading {}", artifact.display()))?;
    let package = ProviderPackage::from_bytes(&bytes)?;
    let mut release: ProviderRelease = serde_json::from_slice(&std::fs::read(&release_path)?)?;
    ensure!(
        release.signature.is_empty(),
        "cannot rebind a signed Provider release"
    );
    release.runtime_artifacts = serde_json::from_slice::<Vec<PlatformRuntimeArtifacts>>(
        &std::fs::read(&runtime_path)
            .with_context(|| format!("reading {}", runtime_path.display()))?,
    )?;
    release.artifact_digest = release.computed_artifact_digest()?;
    // A local sentinel lets the common install-time validator prove the
    // complete binding before any release signature is created.
    release.signature = "pending-signature".to_owned();
    release.validate_for(&package)?;
    ensure!(
        release.package_digest == ProviderPackage::artifact_digest(&bytes),
        "release package digest mismatch"
    );
    release.signature.clear();
    write_json_atomic(&release_path, &release)?;
    println!(
        "{}\t{}\t{} runtime targets bound",
        release.provider_id,
        release.artifact_digest,
        release.runtime_artifacts.len()
    );
    Ok(())
}

fn sign(arguments: &[std::ffi::OsString]) -> Result<()> {
    ensure!(
        arguments.len() == 3,
        "sign requires artifact, release, and private key"
    );
    let artifact = PathBuf::from(&arguments[0]);
    let release_path = PathBuf::from(&arguments[1]);
    let private_key = PathBuf::from(&arguments[2]);
    let bytes =
        std::fs::read(&artifact).with_context(|| format!("reading {}", artifact.display()))?;
    let mut release: ProviderRelease = serde_json::from_slice(&std::fs::read(&release_path)?)?;
    // validate_bytes requires a non-empty signature. Use a local sentinel only
    // for structural validation; it is never persisted or included in proof().
    release.signature = "pending-signature".to_owned();
    release.validate_bytes(&bytes)?;
    release.signature = ssh_sign(&private_key, &release.proof())?;
    write_json_atomic(&release_path, &release)?;
    println!("{}\t{}", release.provider_id, release.artifact_digest);
    Ok(())
}

fn verify(arguments: &[std::ffi::OsString]) -> Result<()> {
    ensure!(
        arguments.len() == 3,
        "verify requires artifact, release, and public key"
    );
    let artifact = PathBuf::from(&arguments[0]);
    let release_path = PathBuf::from(&arguments[1]);
    let public_key_path = PathBuf::from(&arguments[2]);
    let bytes = std::fs::read(&artifact)?;
    let release: ProviderRelease = serde_json::from_slice(&std::fs::read(&release_path)?)?;
    release.validate_bytes(&bytes)?;
    let public_key = normalize_public_key(&std::fs::read_to_string(&public_key_path)?)?;
    ensure!(
        ssh_verify(&public_key, &release.proof(), &release.signature)?,
        "invalid Provider release signature"
    );
    println!(
        "{}\t{}\tverified",
        release.provider_id, release.artifact_digest
    );
    Ok(())
}

fn inspect(arguments: &[std::ffi::OsString]) -> Result<()> {
    ensure!(arguments.len() == 1, "inspect requires one artifact");
    let package = ProviderPackage::from_bytes(&std::fs::read(&arguments[0])?)?;
    println!("{}", serde_json::to_string_pretty(&package)?);
    Ok(())
}

fn ssh_sign(private_key: &Path, proof: &[u8]) -> Result<String> {
    let mut child = Command::new("ssh-keygen")
        .args(["-Y", "sign", "-f"])
        .arg(private_key)
        .args(["-n", PROVIDER_RELEASE_SIGNATURE_NAMESPACE])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting ssh-keygen Provider release signature")?;
    child
        .stdin
        .take()
        .context("opening ssh-keygen stdin")?
        .write_all(proof)?;
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "ssh-keygen sign failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    String::from_utf8(output.stdout).context("SSH signature is not UTF-8")
}

fn ssh_verify(public_key: &str, proof: &[u8], signature: &str) -> Result<bool> {
    let directory = tempfile::Builder::new()
        .prefix("cowboy-provider-verify-")
        .tempdir()?;
    let allowed = directory.path().join("allowed_signers");
    let signature_path = directory.path().join("release.sig");
    std::fs::write(&allowed, format!("cowboy-provider {public_key}\n"))?;
    std::fs::write(&signature_path, signature)?;
    let mut child = Command::new("ssh-keygen")
        .args(["-Y", "verify", "-f"])
        .arg(&allowed)
        .args([
            "-I",
            "cowboy-provider",
            "-n",
            PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
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
        .context("opening ssh-keygen verify stdin")?
        .write_all(proof)?;
    Ok(child.wait()?.success())
}

fn normalize_public_key(value: &str) -> Result<String> {
    let mut fields = value.split_whitespace();
    let kind = fields.next().context("public key type is missing")?;
    let body = fields.next().context("public key body is missing")?;
    ensure!(
        kind == "ssh-ed25519",
        "Provider publisher key must be Ed25519"
    );
    ensure!(!body.is_empty(), "Provider publisher key body is empty");
    Ok(format!("{kind} {body}"))
}

fn write_json_atomic(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let temporary = path.with_extension("json.partial");
    std::fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
