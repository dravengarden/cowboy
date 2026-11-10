//! The closed, versioned table of account-usage families that decides which
//! browser identities may share a Machine browser profile. It lives in code
//! on purpose: a family never comes from the mutable Provider catalog, so it
//! changes only with a reviewed Cowboy change (bump `VERSION` with it). A
//! Provider or card missing here has no family and never joins a share.
//! See `docs/machine-browser.md`, "Shared browser profiles".

/// Version of the table; bump with any change to it.
pub const VERSION: u32 = 1;

/// Named-account Provider id → family. A named account's Provider never
/// changes, so neither does its family.
pub const PROVIDERS: &[(&str, &str)] = &[
    ("claude-code", "anthropic"),
    ("codex", "openai"),
    ("grok", "xai"),
    ("claude-deepseek", "deepseek"),
    ("codex-deepseek", "deepseek"),
    ("deepseek-harness", "deepseek"),
];

/// Legacy default usage cards (by card id) → family: the card itself.
pub const CARDS: &[&str] = &["anthropic", "deepseek", "openai", "xai"];

pub fn provider(provider_id: &str) -> Option<&'static str> {
    PROVIDERS
        .iter()
        .find(|(provider, _)| *provider == provider_id)
        .map(|(_, family)| *family)
}

pub fn card(card_id: &str) -> Option<&'static str> {
    CARDS.iter().find(|card| **card == card_id).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the table: changing it is a reviewed decision about which
    /// accounts may ever share one browser profile.
    #[test]
    fn the_family_table_is_pinned() {
        assert_eq!(VERSION, 1);
        assert_eq!(
            PROVIDERS,
            &[
                ("claude-code", "anthropic"),
                ("codex", "openai"),
                ("grok", "xai"),
                ("claude-deepseek", "deepseek"),
                ("codex-deepseek", "deepseek"),
                ("deepseek-harness", "deepseek"),
            ]
        );
        assert_eq!(CARDS, &["anthropic", "deepseek", "openai", "xai"]);
        assert_eq!(provider("claude-code"), Some("anthropic"));
        assert_eq!(provider("future-provider"), None);
        assert_eq!(card("xai"), Some("xai"));
        assert_eq!(card("future-cloud"), None);
        // Every Provider family is also a card family.
        for (_, family) in PROVIDERS {
            assert_eq!(card(family), Some(*family));
        }
    }
}
