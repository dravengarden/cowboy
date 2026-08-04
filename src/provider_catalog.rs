use std::path::{Path, PathBuf};

pub(crate) const CODEX_DEEPSEEK_CATALOG: &str = "/nix/var/nix/profiles/columbus-components/codex-deepseek/share/codex-deepseek/codex-models.json";
pub(crate) const CODEX_DEEPSEEK_LEGACY_CATALOG: &str = "/etc/codex-deepseek/codex-models.json";

/// Return the first deployed DeepSeek-only model catalog. The legacy path is a
/// bounded migration fallback for Machine generations that can roll before the
/// independent component profile is initialized; neither path contains or
/// references standard OpenAI Codex state.
#[must_use]
pub(crate) fn available_codex_deepseek_catalog() -> Option<PathBuf> {
    first_available_catalog(&[
        Path::new(CODEX_DEEPSEEK_CATALOG),
        Path::new(CODEX_DEEPSEEK_LEGACY_CATALOG),
    ])
}

pub(crate) fn first_available_catalog(paths: &[&Path]) -> Option<PathBuf> {
    paths
        .iter()
        .map(|path| (*path).to_path_buf())
        .find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    #[test]
    fn component_profile_precedes_the_bounded_legacy_fallback() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-codex-deepseek-catalog-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let profile = root.join("profile/catalog.json");
        let legacy = root.join("legacy/catalog.json");
        std::fs::create_dir_all(legacy.parent().expect("legacy parent")).expect("legacy dir");
        std::fs::write(&legacy, "legacy").expect("legacy catalog");
        assert_eq!(
            super::first_available_catalog(&[&profile, &legacy]),
            Some(legacy.clone())
        );

        std::fs::create_dir_all(profile.parent().expect("profile parent")).expect("profile dir");
        std::fs::write(&profile, "profile").expect("profile catalog");
        assert_eq!(
            super::first_available_catalog(&[&profile, &legacy]),
            Some(profile)
        );
        std::fs::remove_dir_all(root).expect("remove fixture");
    }
}
