//! Ed25519 machine identity using the OpenSSH implementation available on the
//! supported macOS and Linux hosts.
//!
//! Provider credentials never pass through this module. The private key stays
//! in the Machine state root; Cowboy stores only the normalized public key.

#![warn(clippy::pedantic)]

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use sha2::Digest as _;

pub(crate) const MACHINE_SIGNATURE_NAMESPACE: &str = "cowboy-machine-v1";
pub(crate) const PROVIDER_AUTH_SIGNATURE_NAMESPACE: &str = "cowboy-provider-auth-v1";
/// Verification-only compatibility domain for Provider generations installed
/// before generic Plugin releases replaced Provider release envelopes.
pub(crate) const LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE: &str = "cowboy-provider-release-v1";
const SSH_SIGNATURE_HEADER: &[u8] = b"-----BEGIN SSH SIGNATURE-----";
const MAX_SSH_SIGNATURE_BYTES: usize = 16 * 1_024;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct MachineIdentity {
    private_key: PathBuf,
    public_key: String,
    ssh_keygen: PathBuf,
}

impl MachineIdentity {
    /// Load an existing identity or create one atomically with owner-only
    /// permissions.
    ///
    /// # Errors
    /// Returns when the state directory is unsafe/incomplete or `ssh-keygen`
    /// cannot create or read an Ed25519 identity.
    pub fn load_or_create(state_dir: &Path) -> Result<Self> {
        let ssh_keygen = resolve_ssh_keygen()?;
        fs::create_dir_all(state_dir)
            .with_context(|| format!("creating Machine state dir {}", state_dir.display()))?;
        fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("securing Machine state dir {}", state_dir.display()))?;
        let private_key = state_dir.join("identity_ed25519");
        let public_path = state_dir.join("identity_ed25519.pub");
        if !private_key.exists() || !public_path.exists() {
            if private_key.exists() || public_path.exists() {
                bail!("incomplete Machine identity in {}", state_dir.display());
            }
            let status = Command::new(&ssh_keygen)
                .args([
                    "-q",
                    "-t",
                    "ed25519",
                    "-N",
                    "",
                    "-C",
                    "cowboy-machine",
                    "-f",
                ])
                .arg(&private_key)
                .status()
                .context("running ssh-keygen for Machine identity")?;
            if !status.success() {
                bail!("ssh-keygen failed creating Machine identity");
            }
        }
        fs::set_permissions(&private_key, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("securing {}", private_key.display()))?;
        let public_key = normalize_public_key(
            &fs::read_to_string(&public_path)
                .with_context(|| format!("reading {}", public_path.display()))?,
        )?;
        Ok(Self {
            private_key,
            public_key,
            ssh_keygen,
        })
    }

    #[must_use]
    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    #[must_use]
    pub fn private_key_path(&self) -> &Path {
        &self.private_key
    }

    /// Sign a controller challenge. OpenSSH emits an armored SSH signature,
    /// which remains opaque to the Machine protocol.
    ///
    /// # Errors
    /// Returns when the local identity cannot be read or `ssh-keygen` fails.
    pub fn sign(&self, challenge: &[u8]) -> Result<String> {
        self.sign_namespaced(MACHINE_SIGNATURE_NAMESPACE, challenge)
    }

    /// Sign an application-owned proof in an explicit OpenSSH namespace.
    /// Namespaces keep a valid Machine challenge signature from being replayed
    /// as a Plugin release or credential-distribution authorization.
    pub(crate) fn sign_namespaced(&self, namespace: &str, proof: &[u8]) -> Result<String> {
        validate_namespace(namespace)?;
        let mut child = Command::new(&self.ssh_keygen)
            .args(["-Y", "sign", "-f"])
            .arg(&self.private_key)
            .args(["-n", namespace])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("starting ssh-keygen signature")?;
        child
            .stdin
            .take()
            .context("opening ssh-keygen stdin")?
            .write_all(proof)
            .context("writing signature proof")?;
        let output = child
            .wait_with_output()
            .context("waiting for ssh-keygen sign")?;
        if !output.status.success() {
            bail!(
                "ssh-keygen sign failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        String::from_utf8(output.stdout).context("SSH signature is not UTF-8")
    }
}

/// Verify a challenge without ever materializing a private/shared key on the
/// controller. Untrusted public-key comments are discarded before the
/// `allowed_signers` file is constructed.
///
/// # Errors
/// Returns when the public key is malformed or the verification process cannot
/// be executed. A validly executed mismatch returns `Ok(false)`.
pub fn verify(public_key: &str, challenge: &[u8], signature: &str) -> Result<bool> {
    verify_namespaced(
        public_key,
        MACHINE_SIGNATURE_NAMESPACE,
        challenge,
        signature,
    )
}

/// Verify a proof in one of Cowboy's closed signing namespaces.
///
/// # Errors
/// Returns when the key, namespace, or verification process is invalid. A
/// cryptographic mismatch returns `Ok(false)`.
pub(crate) fn verify_namespaced(
    public_key: &str,
    namespace: &str,
    proof: &[u8],
    signature: &str,
) -> Result<bool> {
    let ssh_keygen = resolve_ssh_keygen()?;
    validate_namespace(namespace)?;
    let public_key = normalize_public_key(public_key)?;
    let temp = VerificationTemp::new()?;
    fs::write(&temp.allowed_signers, format!("machine {public_key}\n"))
        .context("writing temporary allowed signers")?;
    fs::write(&temp.signature, decode_ssh_signature(signature)?)
        .context("writing temporary SSH signature")?;
    let mut child = Command::new(ssh_keygen)
        .args(["-Y", "verify", "-f"])
        .arg(&temp.allowed_signers)
        .args(["-I", "machine", "-n", namespace, "-s"])
        .arg(&temp.signature)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("starting ssh-keygen verification")?;
    child
        .stdin
        .take()
        .context("opening ssh-keygen verification stdin")?
        .write_all(proof)
        .context("writing verification challenge")?;
    Ok(child
        .wait()
        .context("waiting for ssh-keygen verification")?
        .success())
}

fn decode_ssh_signature(signature: &str) -> Result<Vec<u8>> {
    ensure!(
        signature.len() <= MAX_SSH_SIGNATURE_BYTES * 2,
        "SSH signature is too large"
    );
    let decoded = if signature.as_bytes().starts_with(SSH_SIGNATURE_HEADER) {
        signature.as_bytes().to_vec()
    } else {
        base64::engine::general_purpose::STANDARD
            .decode(signature)
            .context("decoding SSH signature")?
    };
    ensure!(
        decoded.len() <= MAX_SSH_SIGNATURE_BYTES && decoded.starts_with(SSH_SIGNATURE_HEADER),
        "invalid SSH signature encoding"
    );
    Ok(decoded)
}

fn resolve_ssh_keygen() -> Result<PathBuf> {
    let mut candidates = trusted_ssh_keygen_paths();
    if let Some(path) = std::env::var_os("PATH") {
        for candidate in std::env::split_paths(&path).map(|directory| directory.join("ssh-keygen"))
        {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }

    let mut rejected = Vec::new();
    for candidate in candidates {
        match validate_ssh_keygen_candidate(&candidate) {
            Ok(executable) => match supports_ssh_signatures(&executable) {
                Ok(true) => return Ok(executable),
                Ok(false) => rejected.push(format!("{} (no SSHSIG support)", executable.display())),
                Err(error) => rejected.push(format!("{} ({error})", executable.display())),
            },
            Err(error) if candidate.exists() => {
                rejected.push(format!("{} ({error})", candidate.display()));
            }
            Err(_) => {}
        }
    }

    let detail = if rejected.is_empty() {
        "no candidate was installed".to_owned()
    } else {
        rejected.join(", ")
    };
    bail!(
        "Cowboy needs a trusted OpenSSH ssh-keygen with SSH signature support (-Y), but {detail}. macOS normally provides /usr/bin/ssh-keygen; on Linux install the OpenSSH client package. Cowboy will not fall back to an incompatible key or signature format."
    )
}

fn trusted_ssh_keygen_paths() -> Vec<PathBuf> {
    [
        "/usr/bin/ssh-keygen",
        "/bin/ssh-keygen",
        "/opt/homebrew/bin/ssh-keygen",
        "/usr/local/bin/ssh-keygen",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

fn validate_ssh_keygen_candidate(candidate: &Path) -> Result<PathBuf> {
    let executable = candidate
        .canonicalize()
        .with_context(|| format!("resolving {}", candidate.display()))?;
    let metadata = executable
        .metadata()
        .with_context(|| format!("inspecting {}", executable.display()))?;
    anyhow::ensure!(metadata.is_file(), "not a regular file");
    let mode = metadata.permissions().mode();
    anyhow::ensure!(mode & 0o111 != 0, "not executable");
    anyhow::ensure!(mode & 0o022 == 0, "group- or world-writable");
    Ok(executable)
}

fn supports_ssh_signatures(executable: &Path) -> Result<bool> {
    let output = Command::new(executable)
        .args(["-Y", "sign"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .with_context(|| format!("probing {}", executable.display()))?;
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    Ok(diagnostic.contains("too few arguments") && diagnostic.contains("namespace"))
}

fn validate_namespace(namespace: &str) -> Result<()> {
    if matches!(
        namespace,
        MACHINE_SIGNATURE_NAMESPACE
            | PROVIDER_AUTH_SIGNATURE_NAMESPACE
            | LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE
            | cowboy_plugin_sdk::PLUGIN_RELEASE_SIGNATURE_NAMESPACE
    ) {
        Ok(())
    } else {
        bail!("unsupported Cowboy signature namespace")
    }
}

/// Validate and strip an untrusted SSH key down to its type and key body.
///
/// # Errors
/// Returns when the value is not a well-formed Ed25519 SSH public key.
pub fn validate_public_key(value: &str) -> Result<String> {
    normalize_public_key(value)
}

/// Return the conventional OpenSSH SHA-256 fingerprint for an enrolled key.
///
/// # Errors
/// Returns when the public key is malformed.
pub fn fingerprint(public_key: &str) -> Result<String> {
    let normalized = normalize_public_key(public_key)?;
    let body = normalized
        .split_once(' ')
        .context("normalized Machine public key has no body")?
        .1;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(body)
        .context("decoding Machine public key")?;
    let digest = sha2::Sha256::digest(decoded);
    Ok(format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    ))
}

fn normalize_public_key(value: &str) -> Result<String> {
    let mut fields = value.split_whitespace();
    let kind = fields.next().context("Machine public key has no type")?;
    let body = fields.next().context("Machine public key has no body")?;
    if kind != "ssh-ed25519" {
        bail!("Machine identity must be an Ed25519 SSH key");
    }
    if body.is_empty()
        || !body
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        bail!("Machine public key body is malformed");
    }
    Ok(format!("{kind} {body}"))
}

struct VerificationTemp {
    directory: PathBuf,
    allowed_signers: PathBuf,
    signature: PathBuf,
}

impl VerificationTemp {
    fn new() -> Result<Self> {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "cowboy-machine-verify-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&directory)
            .with_context(|| format!("creating verification dir {}", directory.display()))?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        let allowed_signers = directory.join("allowed_signers");
        let signature = directory.join("signature");
        for path in [&allowed_signers, &signature] {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .with_context(|| format!("creating {}", path.display()))?;
        }
        Ok(Self {
            directory,
            allowed_signers,
            signature,
        })
    }
}

impl Drop for VerificationTemp {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.allowed_signers);
        let _ = fs::remove_file(&self.signature);
        let _ = fs::remove_dir(&self.directory);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_signs_only_the_exact_challenge() {
        let directory = std::env::temp_dir().join(format!(
            "cowboy-machine-identity-test-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let identity = MachineIdentity::load_or_create(&directory).expect("create identity");
        let signature = identity.sign(b"challenge-a").expect("sign");
        assert!(verify(identity.public_key(), b"challenge-a", &signature).expect("verify"));
        let encoded_signature = base64::engine::general_purpose::STANDARD.encode(&signature);
        assert!(
            verify(identity.public_key(), b"challenge-a", &encoded_signature)
                .expect("verify base64 signature")
        );
        assert!(!verify(identity.public_key(), b"challenge-b", &signature).expect("reject"));
        let legacy_signature = identity
            .sign_namespaced(
                LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
                b"legacy-release",
            )
            .expect("sign legacy release");
        assert!(
            verify_namespaced(
                identity.public_key(),
                LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
                b"legacy-release",
                &legacy_signature,
            )
            .expect("verify legacy release")
        );
        fs::remove_file(directory.join("identity_ed25519")).expect("private key cleanup");
        fs::remove_file(directory.join("identity_ed25519.pub")).expect("public key cleanup");
        fs::remove_dir(directory).expect("identity directory cleanup");
    }

    #[test]
    fn public_key_comments_and_injection_are_not_preserved() {
        let normalized =
            normalize_public_key("ssh-ed25519 QUJD comment\nmalicious *").expect("normalize");
        assert_eq!(normalized, "ssh-ed25519 QUJD");
        assert!(normalize_public_key("ssh-rsa QUJD").is_err());
        assert_eq!(
            fingerprint("ssh-ed25519 QUJD comment").expect("fingerprint"),
            "SHA256:tdQEXD9Gb6kf4sxqvnkjKhpXzfEE96JucW4KHieJ33g"
        );
    }

    #[test]
    fn ssh_keygen_resolver_requires_a_secure_sshsig_capable_executable() {
        let executable = resolve_ssh_keygen().expect("resolve OpenSSH ssh-keygen");
        assert!(validate_ssh_keygen_candidate(&executable).is_ok());
        assert!(supports_ssh_signatures(&executable).expect("probe SSHSIG support"));

        let directory = std::env::temp_dir().join(format!(
            "cowboy-insecure-ssh-keygen-test-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).expect("create test directory");
        let insecure = directory.join("ssh-keygen");
        fs::write(&insecure, "#!/bin/sh\nexit 1\n").expect("write fake ssh-keygen");
        fs::set_permissions(&insecure, fs::Permissions::from_mode(0o777))
            .expect("make fake ssh-keygen insecure");
        let error = validate_ssh_keygen_candidate(&insecure).expect_err("reject writable tool");
        assert!(error.to_string().contains("group- or world-writable"));
        fs::remove_file(insecure).expect("remove fake ssh-keygen");
        fs::remove_dir(directory).expect("remove test directory");
    }
}
