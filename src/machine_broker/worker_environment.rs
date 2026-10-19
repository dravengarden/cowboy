//! Signed Provider scrubbing covers both ambient and configured worker values.
use std::collections::{BTreeMap, BTreeSet};
use tokio::process::Command;

pub(super) fn apply(
    command: &mut Command,
    configured: &BTreeMap<String, String>,
    removed: &BTreeSet<String>,
    prefixes: &BTreeSet<String>,
    projected: &BTreeMap<String, String>,
) {
    command.envs(configured);
    for name in removed {
        command.env_remove(name);
    }
    for name in std::env::vars_os()
        .map(|(name, _)| name)
        .chain(configured.keys().map(std::ffi::OsString::from))
    {
        if name
            .to_str()
            .is_some_and(|name| prefixes.iter().any(|prefix| name.starts_with(prefix)))
        {
            command.env_remove(name);
        }
    }
    // Credential projections are authoritative even when their names share
    // a scrubbed prefix. Never run inherited cleanup after this step.
    command.envs(projected);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_credentials_are_scrubbed_before_signed_projection() {
        let configured = BTreeMap::from([
            ("COWBOY_TEST_AUTH_SECRET".into(), "other-account".into()),
            ("COWBOY_TEST_AUTH_HOME".into(), "/other-account".into()),
            ("COWBOY_TEST_EXPLICIT".into(), "remove".into()),
            ("COWBOY_TEST_KEEP".into(), "keep".into()),
        ]);
        let projected =
            BTreeMap::from([("COWBOY_TEST_AUTH_HOME".into(), "/selected-account".into())]);
        let mut command = Command::new("unused-fixture");
        apply(
            &mut command,
            &configured,
            &BTreeSet::from(["COWBOY_TEST_EXPLICIT".into()]),
            &BTreeSet::from(["COWBOY_TEST_AUTH_".into()]),
            &projected,
        );
        let environment: BTreeMap<_, _> = command.as_std().get_envs().collect();
        let value = |name| environment.get(std::ffi::OsStr::new(name)).copied();
        assert_eq!(value("COWBOY_TEST_AUTH_SECRET"), Some(None));
        assert_eq!(value("COWBOY_TEST_EXPLICIT"), Some(None));
        assert_eq!(
            value("COWBOY_TEST_AUTH_HOME"),
            Some(Some(std::ffi::OsStr::new("/selected-account")))
        );
        assert_eq!(
            value("COWBOY_TEST_KEEP"),
            Some(Some(std::ffi::OsStr::new("keep")))
        );
    }
}
