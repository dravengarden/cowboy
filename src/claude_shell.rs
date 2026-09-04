//! Resolve a bash/zsh path for plugins whose adapter requires one.
//!
//! The override env is `COWBOY_ACP_<PLUGIN_ID>_SHELL`, where `<PLUGIN_ID>` is
//! the plugin id upper-cased with `-`→`_` (e.g. `COWBOY_ACP_CLAUDE_DEEPSEEK_SHELL`).

use std::path::Path;

fn override_key(plugin_id: &str) -> String {
    format!(
        "COWBOY_ACP_{}_SHELL",
        plugin_id.replace('-', "_").to_ascii_uppercase()
    )
}

fn supported_name(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(std::ffi::OsStr::to_str),
        Some("bash" | "zsh")
    )
}

fn supported_executable(path: &Path) -> bool {
    if !path.is_absolute() || !supported_name(path) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn resolve_with(
    plugin_id: &str,
    get_env: &impl Fn(&str) -> Option<String>,
    usable: impl Fn(&Path) -> bool,
) -> Option<String> {
    let override_shell = get_env(&override_key(plugin_id))
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_owned());
    if let Some(shell) = override_shell.as_deref()
        && usable(Path::new(shell))
    {
        return Some(shell.to_owned());
    }

    if let Some(shell) = get_env("SHELL").filter(|value| !value.trim().is_empty())
        && usable(Path::new(shell.trim()))
    {
        return Some(shell.trim().to_owned());
    }

    if let Some(path) = get_env("PATH") {
        for name in ["bash", "zsh"] {
            for directory in std::env::split_paths(std::ffi::OsStr::new(&path)) {
                let candidate = directory.join(name);
                if usable(&candidate)
                    && let Some(candidate) = candidate.to_str()
                {
                    return Some(candidate.to_owned());
                }
            }
        }
    }

    // Claude Code deliberately accepts only bash or zsh, not a generic `sh`.
    // The system profile is NixOS's stable, generation-independent entry point;
    // the remaining paths cover ordinary Linux and macOS installations.
    for candidate in [
        "/run/current-system/sw/bin/bash",
        "/bin/bash",
        "/usr/bin/bash",
        "/usr/local/bin/bash",
        "/opt/homebrew/bin/bash",
        "/run/current-system/sw/bin/zsh",
        "/bin/zsh",
        "/usr/bin/zsh",
        "/usr/local/bin/zsh",
        "/opt/homebrew/bin/zsh",
    ] {
        if usable(Path::new(candidate)) {
            return Some(candidate.to_owned());
        }
    }
    None
}

pub(crate) fn resolve(
    plugin_id: &str,
    get_env: &impl Fn(&str) -> Option<String>,
) -> Option<String> {
    resolve_with(plugin_id, get_env, supported_executable)
}

pub(crate) fn available(plugin_id: &str) -> bool {
    resolve(plugin_id, &|key| std::env::var(key).ok()).is_some()
}

#[cfg(test)]
mod tests {
    use super::{override_key, resolve_with, supported_executable, supported_name};

    #[test]
    fn rejects_generic_sh_even_when_it_is_executable() {
        assert!(!supported_executable(std::path::Path::new("/bin/sh")));
    }

    #[test]
    fn shell_name_is_the_executable_not_a_parent_directory() {
        assert!(!supported_name(std::path::Path::new(
            "/opt/bash-tools/bin/fish"
        )));
        assert!(supported_name(std::path::Path::new("/opt/shells/bin/bash")));
    }

    #[test]
    fn override_wins_when_it_is_supported() {
        assert_eq!(
            override_key("claude-deepseek"),
            "COWBOY_ACP_CLAUDE_DEEPSEEK_SHELL"
        );
        assert_eq!(override_key("codex"), "COWBOY_ACP_CODEX_SHELL");
        assert_eq!(
            resolve_with(
                "claude-deepseek",
                &|key| match key {
                    "COWBOY_ACP_CLAUDE_DEEPSEEK_SHELL" => {
                        Some(" /custom/bin/bash ".to_owned())
                    }
                    "SHELL" => Some("/bin/fish".to_owned()),
                    "PATH" => Some("/profile/bin:/fallback/bin".to_owned()),
                    _ => None,
                },
                |path| matches!(
                    path.to_str(),
                    Some("/custom/bin/bash" | "/profile/bin/bash")
                ),
            ),
            Some("/custom/bin/bash".to_owned())
        );
    }

    #[test]
    fn invalid_override_falls_back_to_path() {
        assert_eq!(
            resolve_with(
                "claude-deepseek",
                &|key| match key {
                    "COWBOY_ACP_CLAUDE_DEEPSEEK_SHELL" => {
                        Some("/missing/bin/bash".to_owned())
                    }
                    "SHELL" => Some("/bin/fish".to_owned()),
                    "PATH" => Some("/profile/bin:/fallback/bin".to_owned()),
                    _ => None,
                },
                |path| path == std::path::Path::new("/profile/bin/bash"),
            ),
            Some("/profile/bin/bash".to_owned())
        );
    }

    #[test]
    fn unavailable_shell_fails_closed() {
        assert_eq!(resolve_with("claude-deepseek", &|_| None, |_| false), None);
    }
}
