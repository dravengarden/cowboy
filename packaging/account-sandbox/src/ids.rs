//! Account identity: validated caller ids, the derived account key and the
//! static system user name (`docs/antigravity-real-accounts-design.md`,
//! "Sandbox daemon").

use anyhow::{Result, bail, ensure};
use sha2::{Digest as _, Sha256};

/// Domain separator of the key derivation.
const KEY_DOMAIN: &[u8] = b"cowboy-account-v1";
/// Longest accepted `service_id` or `account_id`.
pub const MAX_ID_BYTES: usize = 256;
/// Hex characters of an account key.
pub const KEY_LEN: usize = 16;
/// Prefix of every account user, unit and slice name.
pub const USER_PREFIX: &str = "cga-";

/// Validate a caller-supplied id: non-empty, at most 256 bytes, printable
/// ASCII (space through tilde).
pub fn validate_id(field: &str, value: &str) -> Result<()> {
    ensure!(!value.is_empty(), "{field} is empty");
    ensure!(
        value.len() <= MAX_ID_BYTES,
        "{field} exceeds {MAX_ID_BYTES} bytes"
    );
    ensure!(
        value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)),
        "{field} is not printable ASCII"
    );
    Ok(())
}

/// `hex(sha256("cowboy-account-v1" || 0x00 || service_id || 0x00 ||
/// account_id))[..16]`.
#[must_use]
pub fn derive_key(service_id: &str, account_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(KEY_DOMAIN);
    hasher.update([0]);
    hasher.update(service_id.as_bytes());
    hasher.update([0]);
    hasher.update(account_id.as_bytes());
    let mut key = hex::encode(hasher.finalize());
    key.truncate(KEY_LEN);
    key
}

/// `cga-<key>`: 20 characters, within systemd's 31-character user limit.
#[must_use]
pub fn user_name(key: &str) -> String {
    format!("{USER_PREFIX}{key}")
}

/// Validate a key received in a request: exactly 16 lowercase hex characters.
pub fn validate_key(key: &str) -> Result<()> {
    if key.len() != KEY_LEN
        || !key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("account key must be {KEY_LEN} lowercase hex characters");
    }
    Ok(())
}

/// Validate a lowercase hex digest of `len` characters.
pub fn validate_hex(field: &str, value: &str, len: usize) -> Result<()> {
    ensure!(
        value.len() == len
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "{field} must be {len} lowercase hex characters"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_matches_the_documented_derivation() {
        let mut hasher = Sha256::new();
        hasher.update(b"cowboy-account-v1\0svc\0acct");
        let expected = &hex::encode(hasher.finalize())[..16];
        assert_eq!(derive_key("svc", "acct"), expected);
        assert_eq!(derive_key("svc", "acct").len(), KEY_LEN);
        assert_eq!(user_name(&derive_key("svc", "acct")).len(), 20);
    }

    #[test]
    fn separator_prevents_concatenation_collisions() {
        assert_ne!(derive_key("ab", "c"), derive_key("a", "bc"));
    }

    #[test]
    fn ids_are_validated() {
        validate_id("service_id", "cowboy:service/1 x~").unwrap();
        assert!(validate_id("service_id", "").is_err());
        assert!(validate_id("service_id", &"a".repeat(257)).is_err());
        validate_id("service_id", &"a".repeat(256)).unwrap();
        assert!(validate_id("service_id", "a\nb").is_err());
        assert!(validate_id("service_id", "caf\u{e9}").is_err());
        assert!(validate_id("service_id", "a\x7f").is_err());
    }

    #[test]
    fn keys_and_digests_are_validated() {
        validate_key("0123456789abcdef").unwrap();
        assert!(validate_key("0123456789ABCDEF").is_err());
        assert!(validate_key("0123456789abcde").is_err());
        assert!(validate_key("../../../etc/xx").is_err());
        validate_hex("sha256", &"a".repeat(64), 64).unwrap();
        assert!(validate_hex("sha256", &"g".repeat(64), 64).is_err());
    }
}
