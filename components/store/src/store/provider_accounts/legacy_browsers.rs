//! Machine browsers of the legacy default usage cards: the Service-collected
//! default account of each account-usage provider (`anthropic`, `deepseek`,
//! `openai`, `xai`, ...). Such a card is not a registry account, so its
//! browser needs its own identity and its own owner enablement.
//!
//! The browser identity of a card is a deterministic UUID derived from the
//! owner and the card id. It carries UUID version 8, so it can never equal a
//! registry identity (always version 4) nor the literal `default`, and it is
//! a valid Machine `ProviderAccountId` and directory name: the Machine keeps
//! the card's browser under `provider-auth/accounts/<key>/browser`, apart
//! from the legacy default's native credentials under `provider-auth/`.
//! Enablement is one row per owner and card in `provider_legacy_browsers`
//! (absent = off), changed only through the account registry writer.
use super::super::{StorageBackend, Store};
use crate::provider_account::ProviderAccountId;
use anyhow::Result;
use sha2::Digest as _;

const DOMAIN: &[u8] = b"cowboy-legacy-browser-account-v1\0";

/// The browser identity of `owner`'s legacy default usage card `card`.
pub fn account(owner: &str, card: &str) -> ProviderAccountId {
    let mut hasher = sha2::Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(owner.as_bytes());
    hasher.update([0]);
    hasher.update(card.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    // RFC 9562 version 8 (custom) with the RFC variant.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    ProviderAccountId::try_from(uuid::Uuid::from_bytes(bytes).hyphenated().to_string())
        .expect("canonical lowercase UUID")
}

/// Whether `id` has the shape of a legacy card browser identity. Registry
/// identities are version 4 and never match.
pub fn is_account(id: &ProviderAccountId) -> bool {
    uuid::Uuid::parse_str(id.as_str()).is_ok_and(|parsed| parsed.get_version_num() == 8)
}

/// A card id names an account-usage provider: the Provider id alphabet.
pub fn card_valid(card: &str) -> bool {
    super::provider_id_valid(card)
}

impl Store {
    /// The legacy default usage cards whose browser `owner` enabled.
    pub async fn legacy_browser_cards(&self, owner: &str) -> Result<Vec<String>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_scalar::<_, String>("SELECT card_id FROM provider_legacy_browsers WHERE owner_user_id=$1 ORDER BY card_id")
                .bind(owner).fetch_all(&$db.pool).await?
        }; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_keys_are_distinct_stable_and_never_registry_identities() {
        let cards = ["anthropic", "deepseek", "openai", "xai"];
        let mut keys = std::collections::BTreeSet::new();
        for owner in ["owner", "other"] {
            for card in cards {
                let key = account(owner, card);
                assert_eq!(key, account(owner, card));
                assert!(is_account(&key));
                assert!(!key.is_default());
                assert!(keys.insert(key.clone()), "{owner}/{card} collides");
                // A valid Machine account id and a single path component.
                assert_eq!(key.as_str().len(), 36);
                assert!(
                    key.as_str()
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
                );
            }
        }
        // Registry identities are version 4 and never parse as a card key.
        for _ in 0..64 {
            let registry: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
            assert!(!is_account(&registry));
        }
        assert!(!is_account(&ProviderAccountId::default()));
        assert!(card_valid("deepseek") && !card_valid("") && !card_valid("Deep Seek"));
    }
}
