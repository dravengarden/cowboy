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

use anyhow::{Context as _, Result, bail};

const SIGNATURE_NAMESPACE: &str = "cowboy-machine-v1";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct MachineIdentity {
    private_key: PathBuf,
    public_key: String,
}

impl MachineIdentity {
    /// Load an existing identity or create one atomically with owner-only
    /// permissions.
    ///
    /// # Errors
    /// Returns when the state directory is unsafe/incomplete or `ssh-keygen`
    /// cannot create or read an Ed25519 identity.
    pub fn load_or_create(state_dir: &Path) -> Result<Self> {
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
            let status = Command::new("ssh-keygen")
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
        })
    }

    #[must_use]
    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    /// Sign a controller challenge. OpenSSH emits an armored SSH signature,
    /// which remains opaque to the Machine protocol.
    ///
    /// # Errors
    /// Returns when the local identity cannot be read or `ssh-keygen` fails.
    pub fn sign(&self, challenge: &[u8]) -> Result<String> {
        let mut child = Command::new("ssh-keygen")
            .args(["-Y", "sign", "-f"])
            .arg(&self.private_key)
            .args(["-n", SIGNATURE_NAMESPACE])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("starting ssh-keygen signature")?;
        child
            .stdin
            .take()
            .context("opening ssh-keygen stdin")?
            .write_all(challenge)
            .context("writing Machine challenge")?;
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
    let public_key = normalize_public_key(public_key)?;
    let temp = VerificationTemp::new()?;
    fs::write(&temp.allowed_signers, format!("machine {public_key}\n"))
        .context("writing temporary allowed signers")?;
    fs::write(&temp.signature, signature).context("writing temporary SSH signature")?;
    let mut child = Command::new("ssh-keygen")
        .args(["-Y", "verify", "-f"])
        .arg(&temp.allowed_signers)
        .args(["-I", "machine", "-n", SIGNATURE_NAMESPACE, "-s"])
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
        .write_all(challenge)
        .context("writing verification challenge")?;
    Ok(child
        .wait()
        .context("waiting for ssh-keygen verification")?
        .success())
}

/// Validate and strip an untrusted SSH key down to its type and key body.
///
/// # Errors
/// Returns when the value is not a well-formed Ed25519 SSH public key.
pub fn validate_public_key(value: &str) -> Result<String> {
    normalize_public_key(value)
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
        assert!(!verify(identity.public_key(), b"challenge-b", &signature).expect("reject"));
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
    }
}
