//! Compile-time inventories for first-party plugin payloads.
//!
//! The build script discovers source files from the plugin trees. Keeping the
//! inventory here gives full and Machine-only builds the same source of truth
//! without making either build profile list Provider ids.

pub(crate) const PROVIDER_SOURCES: &[(&str, &str)] =
    include!(concat!(env!("OUT_DIR"), "/first_party_provider_sources.rs"));

pub(crate) const HOST_SOURCES: &[(&str, &str)] =
    include!(concat!(env!("OUT_DIR"), "/first_party_host_sources.rs"));

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    use super::{HOST_SOURCES, PROVIDER_SOURCES};

    fn on_disk_ids(file: &str, roots: &[&str]) -> BTreeSet<String> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        roots
            .iter()
            .flat_map(|relative| {
                fs::read_dir(root.join(relative))
                    .unwrap_or_else(|error| panic!("reading {relative}: {error}"))
                    .filter_map(Result::ok)
            })
            .filter_map(|entry| {
                let path = entry.path();
                (path.is_dir() && path.join(file).is_file())
                    .then(|| entry.file_name().to_string_lossy().into_owned())
            })
            .collect()
    }

    #[test]
    fn discovered_provider_sources_match_first_party_tree() {
        let discovered = PROVIDER_SOURCES
            .iter()
            .map(|(id, _)| (*id).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            discovered,
            on_disk_ids("provider.json", &["plugins", "examples/authentication"])
        );
    }

    #[test]
    fn discovered_host_sources_match_first_party_tree() {
        let discovered = HOST_SOURCES
            .iter()
            .map(|(id, _)| (*id).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            discovered,
            on_disk_ids("host.json", &["plugins", "examples/authentication"])
        );
    }
}
