//! Per-session working-context budgets for the `DeepSeek` agent lanes.
//!
//! `DeepSeek` V4 Flash and Pro both expose a 1M provider window, but the agent
//! runtimes reserve and compact that space differently. Keep the public model
//! ids unchanged and project a separate Cowboy-owned config option instead of
//! inventing provider model aliases.

#![warn(clippy::pedantic)]

#[cfg(feature = "full")]
pub const CONFIG_ID: &str = "deepseek_context";
pub const SESSION_CONTEXT_WINDOW_ENV: &str = "COWBOY_SESSION_CONTEXT_WINDOW";
pub const SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV: &str = "COWBOY_SESSION_AUTO_COMPACT_TOKEN_LIMIT";

const CLAUDE_DEFAULT_PROFILE: &str = "830k";
#[cfg(feature = "full")]
const CODEX_DEFAULT_PROFILE: &str = "680k";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    pub profile_id: &'static str,
    pub context_window: u64,
    pub auto_compact_token_limit: u64,
}

#[derive(Debug, Clone, Copy)]
struct Profile {
    id: &'static str,
    #[cfg(feature = "full")]
    label: &'static str,
    context_window: u64,
}

const PROFILES: &[Profile] = &[
    Profile {
        id: "128k",
        #[cfg(feature = "full")]
        label: "128K",
        context_window: 128_000,
    },
    Profile {
        id: "256k",
        #[cfg(feature = "full")]
        label: "256K",
        context_window: 256_000,
    },
    Profile {
        id: "512k",
        #[cfg(feature = "full")]
        label: "512K",
        context_window: 512_000,
    },
    Profile {
        id: "680k",
        #[cfg(feature = "full")]
        label: "680K",
        context_window: 680_000,
    },
    Profile {
        id: "830k",
        #[cfg(feature = "full")]
        label: "830K",
        context_window: 830_000,
    },
];

fn supported_model(provider: &str, model: Option<&str>) -> bool {
    if !matches!(provider, "claude-deepseek" | "codex-deepseek") {
        return false;
    }
    let model = model.unwrap_or("default").trim().to_ascii_lowercase();
    model == "default"
        || model.starts_with("deepseek-v4-flash")
        || model.starts_with("deepseek-v4-pro")
}

#[must_use]
#[cfg(feature = "full")]
pub fn default_profile(provider: &str, model: Option<&str>) -> Option<&'static str> {
    if !supported_model(provider, model) {
        return None;
    }
    Some(match provider {
        "claude-deepseek" => CLAUDE_DEFAULT_PROFILE,
        "codex-deepseek" => CODEX_DEFAULT_PROFILE,
        _ => unreachable!("supported_model rejected non-DeepSeek provider"),
    })
}

/// Resolve one user-visible profile to the runtime-specific working window and
/// compaction threshold.
///
/// Claude reserves 128K output and uses the user's threshold directly, except
/// for the largest profile where the explicit 819.2K safety line leaves extra
/// room below `DeepSeek`'s 1M request ceiling. Codex advertises the selected
/// window to App Server and follows its model catalog's 95% compaction policy;
/// the 680K default therefore compacts at 646K and leaves room for `DeepSeek`'s
/// documented 384K maximum output.
pub fn resolve(
    provider: &str,
    model: Option<&str>,
    profile_id: &str,
) -> Result<ContextBudget, String> {
    if !supported_model(provider, model) {
        return Err(format!(
            "context budgets are unavailable for provider {provider:?} model {:?}",
            model.unwrap_or("default")
        ));
    }
    let profile = PROFILES
        .iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("unknown DeepSeek context profile {profile_id:?}"))?;
    let auto_compact_token_limit = match provider {
        "claude-deepseek" if profile.id == CLAUDE_DEFAULT_PROFILE => 819_200,
        "claude-deepseek" => profile.context_window,
        "codex-deepseek" => profile.context_window.saturating_mul(95) / 100,
        _ => unreachable!("supported_model rejected non-DeepSeek provider"),
    };
    Ok(ContextBudget {
        profile_id: profile.id,
        context_window: profile.context_window,
        auto_compact_token_limit,
    })
}

#[must_use]
#[cfg(feature = "full")]
pub fn launch_budget(
    provider: &str,
    model: Option<&str>,
    requested_profile: Option<&str>,
) -> Option<ContextBudget> {
    let default = default_profile(provider, model)?;
    resolve(provider, model, requested_profile.unwrap_or(default))
        .or_else(|_| resolve(provider, model, default))
        .ok()
}

#[must_use]
pub fn from_launch_values(
    provider: &str,
    context_window: u64,
    compact_limit: u64,
) -> Option<ContextBudget> {
    PROFILES.iter().find_map(|profile| {
        resolve(provider, None, profile.id).ok().filter(|budget| {
            budget.context_window == context_window
                && budget.auto_compact_token_limit == compact_limit
        })
    })
}

#[must_use]
#[cfg(feature = "full")]
pub fn config_option(
    provider: &str,
    model: Option<&str>,
    requested_profile: Option<&str>,
) -> Option<serde_json::Value> {
    let selected = launch_budget(provider, model, requested_profile)?;
    let description = match provider {
        "claude-deepseek" => {
            "DeepSeek V4 working context. The recommended 830K profile compacts at 819.2K and reserves 128K output. Changing it restarts only this idle session."
        }
        "codex-deepseek" => {
            "DeepSeek V4 working context. Codex compacts at 95%; 680K is recommended to leave room for DeepSeek's 384K maximum output. Changing it restarts only this idle session."
        }
        _ => return None,
    };
    let options = PROFILES
        .iter()
        .map(|profile| {
            let name = match (provider, profile.id) {
                ("claude-deepseek", CLAUDE_DEFAULT_PROFILE)
                | ("codex-deepseek", CODEX_DEFAULT_PROFILE) => {
                    format!("{} · recommended", profile.label)
                }
                ("codex-deepseek", "830k") => format!("{} · large", profile.label),
                _ => profile.label.to_owned(),
            };
            serde_json::json!({ "value": profile.id, "name": name })
        })
        .collect::<Vec<_>>();
    Some(serde_json::json!({
        "id": CONFIG_ID,
        "name": "Context budget",
        "description": description,
        "category": "model_config",
        "type": "select",
        "currentValue": selected.profile_id,
        "options": options,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "full")]
    fn current_v4_models_share_provider_specific_profiles() {
        for model in ["deepseek-v4-flash", "deepseek-v4-pro[1m]"] {
            let claude = launch_budget("claude-deepseek", Some(model), None).unwrap();
            assert_eq!(claude.profile_id, "830k");
            assert_eq!(claude.context_window, 830_000);
            assert_eq!(claude.auto_compact_token_limit, 819_200);

            let codex = launch_budget("codex-deepseek", Some(model), None).unwrap();
            assert_eq!(codex.profile_id, "680k");
            assert_eq!(codex.context_window, 680_000);
            assert_eq!(codex.auto_compact_token_limit, 646_000);
        }
    }

    #[test]
    fn every_requested_profile_resolves_for_both_deepseek_lanes() {
        for profile in ["128k", "256k", "512k", "680k", "830k"] {
            assert!(resolve("claude-deepseek", None, profile).is_ok());
            assert!(resolve("codex-deepseek", None, profile).is_ok());
        }
        assert!(resolve("claude-code", None, "830k").is_err());
        assert!(resolve("codex-deepseek", None, "1m").is_err());
    }

    #[test]
    #[cfg(feature = "full")]
    fn config_option_marks_the_correct_recommendation() {
        let claude = config_option("claude-deepseek", None, None).unwrap();
        assert_eq!(claude["currentValue"], "830k");
        assert_eq!(claude["options"][4]["name"], "830K · recommended");

        let codex = config_option("codex-deepseek", None, None).unwrap();
        assert_eq!(codex["currentValue"], "680k");
        assert_eq!(codex["options"][3]["name"], "680K · recommended");
        assert_eq!(codex["options"][4]["name"], "830K · large");
        assert!(config_option("codex", None, None).is_none());
        assert!(config_option("claude-code", None, None).is_none());
    }

    #[test]
    fn budget_environment_uses_worker_only_names() {
        assert_eq!(SESSION_CONTEXT_WINDOW_ENV, "COWBOY_SESSION_CONTEXT_WINDOW");
        assert_eq!(
            SESSION_AUTO_COMPACT_TOKEN_LIMIT_ENV,
            "COWBOY_SESSION_AUTO_COMPACT_TOKEN_LIMIT"
        );
        assert!(from_launch_values("claude-deepseek", 830_000, 819_200).is_some());
        assert!(from_launch_values("claude-deepseek", 830_000, 830_000).is_none());
        assert!(from_launch_values("claude-code", 830_000, 819_200).is_none());
    }
}
