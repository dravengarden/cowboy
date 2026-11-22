//! The retired legacy default usage cards: the Service `default` account of
//! each account-usage family, collected Provider-wide by the Service and
//! signed in at Service scope. Named accounts replaced every card; see
//! `docs/legacy-card-retirement.md`.
//!
//! A card's Machine browser identity outlives it: an adopted account uses
//! the key as its registry id (the profile stayed where it was), and a
//! retired card key with a retained placement can still be read, stopped and
//! reset (never opened), so its owner can wipe the profile. The key is a
//! deterministic UUID version 8 derived from the owner and the card, so it is
//! never the literal `default` and never a fresh registry identity (version
//! 4). The Machine keeps it under `provider-auth/accounts/<key>/browser`.
use crate::provider_account::ProviderAccountId;
use sha2::Digest as _;

const DOMAIN: &[u8] = b"cowboy-legacy-browser-account-v1\0";

/// Card ids (account-usage families). Closed: no card can appear again.
pub const CARDS: &[&str] = &["anthropic", "deepseek", "openai", "xai"];

/// The Service reset lanes the cards' collectors used (their Plugins' reset
/// ids and the card ids), closed like [`CARDS`]: a schedule persisted on one
/// is cancelled at startup even when its Plugin binding is absent.
pub const RETIRED_RESET_LANES: &[&str] = &["anthropic", "codex", "deepseek", "openai", "xai"];

/// Whether `provider_id` belongs to a card family, so its `default` account
/// was a legacy card and is retired. A named-only Provider (Antigravity) has
/// no card.
pub fn retired_provider(provider_id: &str) -> bool {
    super::browser_families::provider(provider_id).is_some_and(|family| CARDS.contains(&family))
}

/// The browser identity of `owner`'s legacy card `card`.
pub fn browser_key(owner: &str, card: &str) -> ProviderAccountId {
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

/// Whether `id` has the shape of a card browser key (UUID version 8).
/// Fresh registry identities are version 4 and never match; an adopted
/// account's id does, so check the registry first.
pub fn is_browser_key(id: &ProviderAccountId) -> bool {
    uuid::Uuid::parse_str(id.as_str()).is_ok_and(|parsed| parsed.get_version_num() == 8)
}

/// Every Provider whose `default` account is a retired card, in table order.
pub fn retired_providers() -> Vec<&'static str> {
    super::browser_families::PROVIDERS
        .iter()
        .map(|(provider, _)| *provider)
        .filter(|provider| retired_provider(provider))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_keys_are_distinct_stable_and_never_fresh_registry_identities() {
        let mut keys = std::collections::BTreeSet::new();
        for owner in ["owner", "other"] {
            for card in CARDS {
                let key = browser_key(owner, card);
                assert_eq!(key, browser_key(owner, card));
                assert!(is_browser_key(&key));
                assert!(!key.is_default());
                assert!(keys.insert(key.clone()), "{owner}/{card} collides");
                assert_eq!(key.as_str().len(), 36);
            }
        }
        // Pinned: adopted accounts already use these ids.
        assert_eq!(
            browser_key("owner", "openai").as_str(),
            "ef4f37f5-6e65-812a-a5c4-7415a699a88c"
        );
        for _ in 0..64 {
            let registry: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
            assert!(!is_browser_key(&registry));
        }
        assert!(!is_browser_key(&ProviderAccountId::default()));
    }

    #[test]
    fn every_card_family_provider_and_only_those_is_retired() {
        for provider in [
            "claude-code",
            "codex",
            "grok",
            "claude-deepseek",
            "codex-deepseek",
            "deepseek-harness",
        ] {
            assert!(retired_provider(provider), "{provider}");
        }
        for provider in ["antigravity", "future-provider", "", "anthropic"] {
            assert!(!retired_provider(provider), "{provider}");
        }
        assert_eq!(
            retired_providers(),
            [
                "claude-code",
                "codex",
                "grok",
                "claude-deepseek",
                "codex-deepseek",
                "deepseek-harness"
            ]
        );
    }
}
