//! The closed, versioned table of account-usage families that decides which
//! browser identities may share a Machine browser profile. It lives in code
//! on purpose: a family never comes from the mutable Provider catalog, so it
//! changes only with a reviewed Cowboy change (bump `VERSION` with it). A
//! Provider missing here has no family and never joins a share. Version 3
//! dropped the retired legacy default cards as members.
//! See `docs/machine-browser.md`, "Shared browser profiles".

/// Version of the table; bump with any change to it.
pub const VERSION: u32 = 3;

/// Named-account Provider id → family. A named account's Provider never
/// changes, so neither does its family.
pub const PROVIDERS: &[(&str, &str)] = &[
    ("claude-code", "anthropic"),
    ("codex", "openai"),
    ("grok", "xai"),
    ("claude-deepseek", "deepseek"),
    ("codex-deepseek", "deepseek"),
    ("deepseek-harness", "deepseek"),
    ("antigravity", "google"),
];

pub fn provider(provider_id: &str) -> Option<&'static str> {
    PROVIDERS
        .iter()
        .find(|(provider, _)| *provider == provider_id)
        .map(|(_, family)| *family)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the table: changing it is a reviewed decision about which
    /// accounts may ever share one browser profile.
    #[test]
    fn the_family_table_is_pinned() {
        assert_eq!(VERSION, 3);
        assert_eq!(
            PROVIDERS,
            &[
                ("claude-code", "anthropic"),
                ("codex", "openai"),
                ("grok", "xai"),
                ("claude-deepseek", "deepseek"),
                ("codex-deepseek", "deepseek"),
                ("deepseek-harness", "deepseek"),
                ("antigravity", "google"),
            ]
        );
        assert_eq!(provider("claude-code"), Some("anthropic"));
        assert_eq!(provider("future-provider"), None);
        assert_eq!(provider("antigravity"), Some("google"));
    }
}
