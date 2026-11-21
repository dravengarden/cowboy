//! The retired legacy default usage cards: the Service `default` account of
//! each account-usage family, collected Provider-wide by the Service and
//! signed in at Service scope. Named accounts replaced every card; see
//! `docs/legacy-card-retirement.md`.

/// Card ids (account-usage families). Closed: no card can appear again.
pub const CARDS: &[&str] = &["anthropic", "deepseek", "openai", "xai"];

/// Whether `provider_id` belongs to a card family, so its `default` account
/// was a legacy card and is retired. A named-only Provider (Antigravity) has
/// no card.
pub fn retired_provider(provider_id: &str) -> bool {
    super::browser_families::provider(provider_id).is_some_and(|family| CARDS.contains(&family))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
