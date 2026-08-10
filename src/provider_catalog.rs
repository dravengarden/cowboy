use std::path::PathBuf;

pub(crate) const CODEX_DEEPSEEK_CATALOG: &str = "/nix/var/nix/profiles/columbus-components/codex-deepseek/share/codex-deepseek/codex-models.json";

/// Return the independently deployed DeepSeek-only model catalog.
#[must_use]
pub(crate) fn available_codex_deepseek_catalog() -> Option<PathBuf> {
    let catalog = PathBuf::from(CODEX_DEEPSEEK_CATALOG);
    catalog.is_file().then_some(catalog)
}

#[cfg(test)]
mod tests {
    #[test]
    fn catalog_is_owned_by_the_component_profile() {
        assert_eq!(
            super::CODEX_DEEPSEEK_CATALOG,
            "/nix/var/nix/profiles/columbus-components/codex-deepseek/share/codex-deepseek/codex-models.json"
        );
        assert!(!super::CODEX_DEEPSEEK_CATALOG.starts_with("/etc/"));
    }
}
