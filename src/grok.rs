//! Shared Grok Build launch contract used by controller and Machine slices.

#[cfg(feature = "full")]
pub(crate) const PROJECT_RULES_BOOTSTRAP: &str =
    "Read and follow the closest AGENTS.md project instructions before taking any action.";

#[cfg(feature = "machine-host")]
pub(crate) const RUNTIME_ARGS_ENV: &str = "--no-auto-update --experimental-memory --rules 'Read and follow the closest AGENTS.md project instructions before taking any action.' agent --always-approve --no-leader stdio";

#[cfg(feature = "full")]
pub(crate) const RUNTIME_ARGS: &[&str] = &[
    "--no-auto-update",
    "--experimental-memory",
    "--rules",
    PROJECT_RULES_BOOTSTRAP,
    "agent",
    "--always-approve",
    "--no-leader",
    "stdio",
];
