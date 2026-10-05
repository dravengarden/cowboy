//! Which variables a Machine copies into an execution target's closed
//! environment. Kept apart from the launch contract wire types so that the
//! policy can change without touching the retained worker interface.

/// Variables every target environment carries from the Machine when set.
pub const BASE: [&str; 12] = [
    "HOME",
    "USER",
    "LOGNAME",
    "PATH",
    "SHELL",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
];

/// Upper bound on base plus operator-declared target variables.
pub const MAX: usize = 32;

/// Whether a Machine operator may add `name` to target environments, for
/// host tool locations such as a worktree or cache root. Cowboy, Codex and
/// Provider namespaces, credential-shaped names, and loader or shell startup
/// hooks stay closed.
pub fn operator_name_allowed(name: &str) -> bool {
    const RESERVED_PREFIXES: [&str; 10] = [
        "COWBOY_",
        "CODEX_",
        "ANTHROPIC_",
        "CLAUDE_",
        "OPENAI_",
        "DEEPSEEK_",
        "GEMINI_",
        "GROK_",
        "LD_",
        "DYLD_",
    ];
    const RESERVED_SUFFIXES: [&str; 5] = ["_KEY", "_TOKEN", "_SECRET", "_PASSWORD", "_CREDENTIALS"];
    const RESERVED: [&str; 6] = [
        "BASH_ENV",
        "ENV",
        "IFS",
        "PROMPT_COMMAND",
        "SHELLOPTS",
        "NODE_OPTIONS",
    ];
    name.len() <= 64
        && name
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        && !BASE.contains(&name)
        && !RESERVED.contains(&name)
        && !RESERVED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        && !RESERVED_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

/// Whether a launch contract may carry `name`.
pub fn allowed(name: &str) -> bool {
    BASE.contains(&name) || operator_name_allowed(name)
}
