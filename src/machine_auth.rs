//! In-memory OpenSSH `PROTOCOL.sshsig` Ed25519 verification layered over
//! `cowboy_wire::machine_auth`.
//!
//! Signing and key generation stay on `cowboy_wire::machine_auth` so the
//! retained-worker wire interface remains unchanged; verification runs purely
//! in memory via `ring::signature::ED25519` without temp files or subprocesses.

#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

use anyhow::{Context as _, Result, bail, ensure};
use base64::Engine as _;
use ring::signature::{ED25519, UnparsedPublicKey};
use sha2::Digest as _;

pub use cowboy_wire::machine_auth::{
    LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE, MACHINE_SIGNATURE_NAMESPACE, MachineIdentity,
    PROVIDER_AUTH_SIGNATURE_NAMESPACE, fingerprint, validate_public_key,
};

const SSH_SIGNATURE_HEADER: &[u8] = b"-----BEGIN SSH SIGNATURE-----";
const SSH_SIGNATURE_FOOTER: &str = "-----END SSH SIGNATURE-----";
const SSHSIG_MAGIC: &[u8; 6] = b"SSHSIG";
const SSHSIG_VERSION: u32 = 1;
const MAX_SSH_SIGNATURE_BYTES: usize = 16 * 1_024;

/// Verify a challenge in memory without materializing a private/shared key or
/// spawning a subprocess. Untrusted public-key comments are discarded before
/// parsing the Ed25519 key.
pub fn verify(public_key: &str, challenge: &[u8], signature: &str) -> Result<bool> {
    verify_namespaced(
        public_key,
        MACHINE_SIGNATURE_NAMESPACE,
        challenge,
        signature,
    )
}

/// Verify a proof in one of Cowboy's closed signing namespaces purely in memory.
pub fn verify_namespaced(
    public_key: &str,
    namespace: &str,
    proof: &[u8],
    signature: &str,
) -> Result<bool> {
    validate_namespace(namespace)?;
    let normalized = validate_public_key(public_key)?;
    let (expected_pubkey_blob, raw_pubkey) = parse_ed25519_public_key(&normalized)?;
    let decoded_signature = decode_ssh_signature(signature)?;
    let Some(blob) = dearmor_ssh_signature(&decoded_signature) else {
        return Ok(false);
    };
    let Some(parsed) = parse_sshsig_blob(&blob) else {
        return Ok(false);
    };
    if parsed.pubkey_blob != expected_pubkey_blob || parsed.namespace != namespace.as_bytes() {
        return Ok(false);
    }
    let digest: Vec<u8> = match parsed.hash_algorithm {
        b"sha512" => sha2::Sha512::digest(proof).to_vec(),
        b"sha256" => sha2::Sha256::digest(proof).to_vec(),
        _ => return Ok(false),
    };
    let mut signed = Vec::with_capacity(
        SSHSIG_MAGIC.len()
            + 4
            + parsed.namespace.len()
            + 4
            + parsed.reserved.len()
            + 4
            + parsed.hash_algorithm.len()
            + 4
            + digest.len(),
    );
    signed.extend_from_slice(SSHSIG_MAGIC);
    write_ssh_string(&mut signed, parsed.namespace);
    write_ssh_string(&mut signed, parsed.reserved);
    write_ssh_string(&mut signed, parsed.hash_algorithm);
    write_ssh_string(&mut signed, &digest);
    let verifier = UnparsedPublicKey::new(&ED25519, raw_pubkey);
    Ok(verifier.verify(&signed, &parsed.signature).is_ok())
}

struct ParsedSshSig<'a> {
    pubkey_blob: &'a [u8],
    namespace: &'a [u8],
    reserved: &'a [u8],
    hash_algorithm: &'a [u8],
    signature: [u8; 64],
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

fn parse_ed25519_public_key(normalized_public_key: &str) -> Result<(Vec<u8>, [u8; 32])> {
    let body = normalized_public_key
        .split_once(' ')
        .context("normalized Machine public key has no body")?
        .1;
    let blob = base64::engine::general_purpose::STANDARD
        .decode(body)
        .context("decoding Machine public key")?;
    let mut cursor = blob.as_slice();
    let key_type = read_ssh_string(&mut cursor).context("reading SSH public key type")?;
    let raw_key = read_ssh_string(&mut cursor).context("reading SSH Ed25519 public key bytes")?;
    ensure!(
        cursor.is_empty() && key_type == b"ssh-ed25519" && raw_key.len() == 32,
        "Machine public key is not a valid Ed25519 OpenSSH key"
    );
    let key_bytes: [u8; 32] = raw_key
        .try_into()
        .map_err(|_| anyhow::anyhow!("Ed25519 public key must be 32 bytes"))?;
    Ok((blob, key_bytes))
}

fn dearmor_ssh_signature(armored: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(armored).ok()?;
    let header = std::str::from_utf8(SSH_SIGNATURE_HEADER).ok()?;
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if lines.len() < 3
        || lines.first() != Some(&header)
        || lines.last() != Some(&SSH_SIGNATURE_FOOTER)
    {
        return None;
    }
    let body_lines = &lines[1..lines.len() - 1];
    let mut encoded = String::new();
    for line in body_lines {
        if line.trim_start().starts_with("-----") {
            return None;
        }
        encoded.extend(
            line.chars()
                .filter(|ch| !ch.is_ascii_whitespace() && *ch != '\x0b'),
        );
    }
    if encoded.is_empty() {
        return None;
    }
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()
}

fn parse_sshsig_blob(blob: &[u8]) -> Option<ParsedSshSig<'_>> {
    let mut cursor = blob.strip_prefix(SSHSIG_MAGIC)?;
    let version = read_u32(&mut cursor)?;
    if version != SSHSIG_VERSION {
        return None;
    }
    let pubkey_blob = read_ssh_string(&mut cursor)?;
    let namespace = read_ssh_string(&mut cursor)?;
    let reserved = read_ssh_string(&mut cursor)?;
    let hash_algorithm = read_ssh_string(&mut cursor)?;
    let sig_blob = read_ssh_string(&mut cursor)?;
    if !cursor.is_empty() || namespace.is_empty() {
        return None;
    }
    let mut sig_cursor = sig_blob;
    let sig_type = read_ssh_string(&mut sig_cursor)?;
    let sig_bytes = read_ssh_string(&mut sig_cursor)?;
    if !sig_cursor.is_empty() || sig_type != b"ssh-ed25519" {
        return None;
    }
    let signature: [u8; 64] = sig_bytes.try_into().ok()?;
    Some(ParsedSshSig {
        pubkey_blob,
        namespace,
        reserved,
        hash_algorithm,
        signature,
    })
}

fn read_u32(cursor: &mut &[u8]) -> Option<u32> {
    let (head, tail) = cursor.split_at_checked(4)?;
    *cursor = tail;
    Some(u32::from_be_bytes(head.try_into().ok()?))
}

fn read_ssh_string<'a>(cursor: &mut &'a [u8]) -> Option<&'a [u8]> {
    let len = usize::try_from(read_u32(cursor)?).ok()?;
    let (head, tail) = cursor.split_at_checked(len)?;
    *cursor = tail;
    Some(head)
}

fn write_ssh_string(out: &mut Vec<u8>, bytes: &[u8]) {
    let len = u32::try_from(bytes.len()).expect("SSHSIG field length fits in u32");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    use super::*;

    fn armor_sshsig_blob(blob: &[u8]) -> String {
        format!(
            "-----BEGIN SSH SIGNATURE-----\n{}\n-----END SSH SIGNATURE-----\n",
            base64::engine::general_purpose::STANDARD.encode(blob)
        )
    }

    #[test]
    fn verify_in_memory_matches_openssh_for_sha512_and_base64_armor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let identity = MachineIdentity::load_or_create(dir.path()).expect("create identity");
        let signature = identity.sign(b"challenge-a").expect("sign");
        assert!(verify(identity.public_key(), b"challenge-a", &signature).expect("verify"));
        let encoded_signature = base64::engine::general_purpose::STANDARD.encode(&signature);
        assert!(
            verify(identity.public_key(), b"challenge-a", &encoded_signature)
                .expect("verify base64 signature")
        );
        assert!(!verify(identity.public_key(), b"challenge-b", &signature).expect("reject"));
        let legacy = identity
            .sign_namespaced(
                LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
                b"legacy-release",
            )
            .expect("sign legacy");
        assert!(
            verify_namespaced(
                identity.public_key(),
                LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
                b"legacy-release",
                &legacy,
            )
            .expect("verify legacy")
        );
    }

    #[test]
    fn verify_namespaced_supports_sha256_and_armor_whitespace() {
        let dir_a = tempfile::tempdir().expect("tempdir a");
        let dir_b = tempfile::tempdir().expect("tempdir b");
        let id_a = MachineIdentity::load_or_create(dir_a.path()).expect("create identity a");
        let id_b = MachineIdentity::load_or_create(dir_b.path()).expect("create identity b");

        let mut child = Command::new("ssh-keygen")
            .args(["-Y", "sign", "-f"])
            .arg(id_a.private_key_path())
            .args([
                "-n",
                cowboy_plugin_sdk::PLUGIN_RELEASE_SIGNATURE_NAMESPACE,
                "-O",
                "hashalg=sha256",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn ssh-keygen sha256 sign");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(b"sha256-proof")
            .expect("write proof");
        let output = child.wait_with_output().expect("wait sha256 sign");
        assert!(output.status.success(), "sha256 sign failed");
        let sha256_sig = String::from_utf8(output.stdout).expect("utf8 sig");
        let ns = cowboy_plugin_sdk::PLUGIN_RELEASE_SIGNATURE_NAMESPACE;

        assert!(
            verify_namespaced(id_a.public_key(), ns, b"sha256-proof", &sha256_sig).expect("sha256")
        );
        assert!(
            !verify_namespaced(id_b.public_key(), ns, b"sha256-proof", &sha256_sig)
                .expect("wrong key")
        );
        assert!(
            !verify_namespaced(
                id_a.public_key(),
                MACHINE_SIGNATURE_NAMESPACE,
                b"sha256-proof",
                &sha256_sig
            )
            .expect("wrong ns")
        );

        // Blank lines and horizontal whitespace inside the armor body match OpenSSH dearmor.
        let mut lines: Vec<String> = sha256_sig.lines().map(str::to_owned).collect();
        lines.insert(2, String::new());
        let (head, tail) = lines[1].split_at(lines[1].len() / 2);
        lines[1] = format!("  {head}\x0b{tail} \t ");
        let spaced = format!("{}\n\n", lines.join("\n"));
        assert!(
            verify_namespaced(id_a.public_key(), ns, b"sha256-proof", &spaced)
                .expect("whitespace in armor body")
        );
    }

    #[test]
    fn verify_namespaced_rejects_malformed_sshsig_blobs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let identity = MachineIdentity::load_or_create(dir.path()).expect("create identity");
        let ns = cowboy_plugin_sdk::PLUGIN_RELEASE_SIGNATURE_NAMESPACE;
        let sig = identity
            .sign_namespaced(ns, b"malformed-proof")
            .expect("sign proof");

        let trailing_armor = format!("{sig}trailing-garbage\n");
        assert!(
            !verify_namespaced(
                identity.public_key(),
                ns,
                b"malformed-proof",
                &trailing_armor
            )
            .expect("trailing armor")
        );

        let mut trailing_blob = dearmor_ssh_signature(sig.as_bytes()).expect("dearmor");
        trailing_blob.push(0x00);
        assert!(
            !verify_namespaced(
                identity.public_key(),
                ns,
                b"malformed-proof",
                &armor_sshsig_blob(&trailing_blob)
            )
            .expect("trailing blob")
        );

        let mut tampered = dearmor_ssh_signature(sig.as_bytes()).expect("dearmor");
        *tampered.last_mut().expect("non-empty blob") ^= 0x01;
        assert!(
            !verify_namespaced(
                identity.public_key(),
                ns,
                b"malformed-proof",
                &armor_sshsig_blob(&tampered)
            )
            .expect("tampered sig")
        );

        let mut bad_ver = dearmor_ssh_signature(sig.as_bytes()).expect("dearmor");
        bad_ver[9] = 2;
        assert!(
            !verify_namespaced(
                identity.public_key(),
                ns,
                b"malformed-proof",
                &armor_sshsig_blob(&bad_ver)
            )
            .expect("bad version")
        );
    }
}
