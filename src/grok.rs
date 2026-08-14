//! Shared Grok Build launch contract used by controller and Machine slices.

#[cfg(feature = "machine-host")]
pub(crate) const RUNTIME_ARGS_ENV: &str =
    "--no-auto-update --experimental-memory agent --always-approve --no-leader stdio";

#[cfg(feature = "full")]
pub(crate) const RUNTIME_ARGS: &[&str] = &[
    "--no-auto-update",
    "--experimental-memory",
    "agent",
    "--always-approve",
    "--no-leader",
    "stdio",
];
