//! Leaf product-credential helpers: username and password rules, argon2id
//! password hashing, random identifiers and session tokens, cookie parsing
//! and secret-safe JSON decoding. It names no other module, so
//! authentication, stores and operators can share it without joining a
//! module cycle.

use std::io::Read as _;
use std::sync::OnceLock;

use anyhow::{Context as _, Result};
use argon2::Argon2;
use axum::http::{HeaderMap, header};
use password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};

const USER_ID_HEX_LEN: usize = 32;

/// Trim, lowercase, and accept 1–64 of `[a-z0-9._-]`.
///
/// # Errors
/// Returns when the handle is empty, longer than 64, or uses other characters.
pub fn normalize_username(username: &str) -> Result<String> {
    let username = username.trim().to_ascii_lowercase();
    anyhow::ensure!(
        (1..=64).contains(&username.len())
            && username.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || byte == b'.'
                    || byte == b'_'
                    || byte == b'-'
            }),
        "username must be 1-64 lowercase letters, digits, '.', '_' or '-'"
    );
    Ok(username)
}

/// Product passwords are 10–128 characters and must not equal the username.
///
/// # Errors
/// Returns when the password is too short, too long, or matches the username.
pub fn ensure_password(password: &str, username: &str) -> Result<()> {
    anyhow::ensure!(
        (10..=128).contains(&password.len()),
        "password must be 10-128 characters"
    );
    anyhow::ensure!(password != username, "password cannot be the username");
    Ok(())
}

/// 32-hex identifier from `/dev/urandom`.
///
/// # Errors
/// Returns when the OS random device cannot be read.
pub fn new_user_id() -> Result<String> {
    hex_from_urandom(USER_ID_HEX_LEN / 2)
}

/// Hash `password` to an `argon2id` PHC string. Salt lives inside the PHC.
///
/// # Errors
/// Returns when randomness cannot be read or argon2id hashing fails.
pub fn hash_password(password: &str) -> Result<String> {
    let salt_bytes = random_bytes(16)?;
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|error| anyhow::anyhow!("encoding argon2 salt: {error}"))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| anyhow::anyhow!("hashing password: {error}"))
}

/// Verify `password` against a stored PHC string.
#[must_use]
pub fn verify_password(password: &str, password_hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(password_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Run argon2id verify against a process-lifetime dummy hash.
///
/// Unknown-user login calls this so the 401 is not enumerable by hash timing.
#[must_use]
pub fn verify_unknown_user_password(password: &str) -> bool {
    verify_password(password, dummy_password_hash())
}

/// 32-byte hex cookie secret (64 hex chars).
///
/// # Errors
/// Returns when the OS random device cannot be read.
pub fn new_session_token() -> Result<String> {
    hex_from_urandom(32)
}

#[must_use]
pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    let prefix = format!("{name}=");
    cookie.split(';').find_map(|part| {
        let part = part.trim();
        part.strip_prefix(&prefix).map(ToOwned::to_owned)
    })
}

fn dummy_password_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let secret = hex_from_urandom(32).expect("reading dummy password secret");
        hash_password(&secret).expect("hashing dummy unknown-user password")
    })
}

pub(crate) fn hex_from_urandom(byte_len: usize) -> Result<String> {
    let bytes = random_bytes(byte_len)?;
    Ok(bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut output, byte| {
            use std::fmt::Write as _;
            write!(output, "{byte:02x}").expect("writing to a String cannot fail");
            output
        },
    ))
}

pub(crate) fn random_bytes(len: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0_u8; len];
    std::fs::File::open("/dev/urandom")
        .context("opening OS randomness")?
        .read_exact(&mut bytes)
        .context("reading OS randomness")?;
    Ok(bytes)
}

/// JSON's default errors can echo invalid values and unknown field names from
/// secret-bearing configuration. Keep only the category and source position.
pub fn decode_private_json<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    name: &'static str,
) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|error| {
        anyhow::anyhow!(
            "decoding {name}: {:?} error at line {}, column {}",
            error.classify(),
            error.line(),
            error.column()
        )
    })
}
