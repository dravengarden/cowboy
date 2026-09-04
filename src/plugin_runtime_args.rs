//! First-party ACP launch values owned by plugin payloads.
//!
//! `runtime.arguments` and string `runtime.environment` entries in
//! `plugins/<id>/provider.json` are the source of truth for controller npx
//! fallbacks, Machine env pins, DeepSeek window/token budgets, and the
//! package-less Codex DeepSeek `config.toml`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

const CLAUDE_CODE: &str = include_str!("../plugins/claude-code/provider.json");
const CLAUDE_CODE_HOST: &str = include_str!("../plugins/claude-code/host.json");
const CLAUDE_DEEPSEEK: &str = include_str!("../plugins/claude-deepseek/provider.json");
const CLAUDE_DEEPSEEK_HOST: &str = include_str!("../plugins/claude-deepseek/host.json");
const CODEX: &str = include_str!("../plugins/codex/provider.json");
const CODEX_HOST: &str = include_str!("../plugins/codex/host.json");
const CODEX_DEEPSEEK: &str = include_str!("../plugins/codex-deepseek/provider.json");
const CODEX_DEEPSEEK_HOST: &str = include_str!("../plugins/codex-deepseek/host.json");
const GEMINI: &str = include_str!("../plugins/gemini/provider.json");
const GEMINI_HOST: &str = include_str!("../plugins/gemini/host.json");
const GROK: &str = include_str!("../plugins/grok/provider.json");
const GROK_HOST: &str = include_str!("../plugins/grok/host.json");

/// First plugin for a slot is the canonical occupant used for disable aliases
/// and component→provider mapping.
const ADAPTER_HOSTS: &[(&str, &str)] = &[
    ("codex", CODEX_HOST),
    ("codex-deepseek", CODEX_DEEPSEEK_HOST),
    ("grok", GROK_HOST),
    ("gemini", GEMINI_HOST),
    ("claude-code", CLAUDE_CODE_HOST),
    ("claude-deepseek", CLAUDE_DEEPSEEK_HOST),
];

/// First plugin to occupy a Machine component slot owns that slot's npm channel.
const NPM_PAYLOADS: &[(&str, &str)] = &[
    ("codex", CODEX),
    ("grok", GROK),
    ("gemini", GEMINI),
    ("claude-code", CLAUDE_CODE),
    ("claude-deepseek", CLAUDE_DEEPSEEK),
    ("codex-deepseek", CODEX_DEEPSEEK),
];

struct AdapterIndex {
    plugin_to_slot: BTreeMap<&'static str, &'static str>,
    slot_to_primary: BTreeMap<&'static str, &'static str>,
    slot_to_plugins: BTreeMap<&'static str, Vec<&'static str>>,
}

fn adapter_index() -> &'static AdapterIndex {
    static VALUE: OnceLock<AdapterIndex> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut plugin_to_slot = BTreeMap::new();
        let mut slot_to_primary = BTreeMap::new();
        let mut slot_to_plugins: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
        for (plugin_id, source) in ADAPTER_HOSTS {
            let Some(slot) = optional_adapter_slot(plugin_id, source) else {
                continue;
            };
            plugin_to_slot.insert(*plugin_id, slot);
            slot_to_plugins.entry(slot).or_default().push(*plugin_id);
            slot_to_primary.entry(slot).or_insert(*plugin_id);
        }
        AdapterIndex {
            plugin_to_slot,
            slot_to_primary,
            slot_to_plugins,
        }
    })
}

struct NpmCatalog {
    by_component: BTreeMap<String, BTreeMap<String, &'static str>>,
    by_plugin: BTreeMap<String, &'static str>,
}

#[must_use]
pub(crate) fn npm_package_for_component(kind: &str, slot: &str) -> Option<&'static str> {
    npm_catalog()
        .by_component
        .get(kind)
        .and_then(|slots| slots.get(slot))
        .copied()
}

#[must_use]
pub(crate) fn npx_package_for_plugin(plugin_id: &str) -> &'static str {
    npm_catalog()
        .by_plugin
        .get(plugin_id)
        .copied()
        .unwrap_or_else(|| panic!("{plugin_id} provider.json must declare an npm launch package"))
}

#[must_use]
pub(crate) fn npx_prefix(plugin_id: &str) -> [&'static str; 2] {
    ["-y", npx_package_for_plugin(plugin_id)]
}

fn npm_catalog() -> &'static NpmCatalog {
    static VALUE: OnceLock<NpmCatalog> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut by_component = BTreeMap::new();
        let mut by_plugin = BTreeMap::new();
        for (plugin_id, source) in NPM_PAYLOADS {
            collect_npm_packages(plugin_id, source, &mut by_component, &mut by_plugin);
        }
        NpmCatalog {
            by_component,
            by_plugin,
        }
    })
}

fn collect_npm_packages(
    plugin_id: &str,
    source: &str,
    packages: &mut BTreeMap<String, BTreeMap<String, &'static str>>,
    npx: &mut BTreeMap<String, &'static str>,
) {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} provider.json must parse: {error}"));
    let Some(runtime) = value.get("runtime") else {
        return;
    };
    let mut dependencies = BTreeMap::new();
    if let Some(entries) = runtime
        .get("dependencies")
        .and_then(serde_json::Value::as_array)
    {
        for entry in entries {
            let Some(id) = entry.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(package) = entry
                .get("source")
                .and_then(serde_json::Value::as_str)
                .and_then(npm_package_from_registry_url)
            else {
                continue;
            };
            let leaked: &'static str = Box::leak(package.to_owned().into_boxed_str());
            dependencies.insert(id.to_owned(), leaked);
        }
    }
    let Some(platforms) = runtime
        .get("platforms")
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    let mut plugin_cli = None;
    let mut plugin_adapter = None;
    for platform in platforms {
        let Some(components) = platform
            .get("private_components")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for component in components {
            let Some(kind) = component.get("kind").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(slot) = component.get("slot").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(dependency) = component
                .get("dependency")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let Some(package) = dependencies.get(dependency).copied() else {
                continue;
            };
            packages
                .entry(kind.to_owned())
                .or_default()
                .entry(slot.to_owned())
                .or_insert(package);
            match kind {
                "provider_adapter" => {
                    plugin_adapter.get_or_insert(package);
                }
                "provider_cli" => {
                    plugin_cli.get_or_insert(package);
                }
                _ => {}
            }
        }
    }
    if let Some(package) = plugin_adapter.or(plugin_cli) {
        npx.entry(plugin_id.to_owned()).or_insert(package);
    }
}

fn npm_package_from_registry_url(source: &str) -> Option<&str> {
    let rest = source.strip_prefix("https://registry.npmjs.org/")?;
    let (package, _) = rest.split_once("/-/")?;
    (!package.is_empty()
        && package.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'@' | b'/' | b'-' | b'_' | b'.')
        }))
    .then_some(package)
}

#[must_use]
pub(crate) fn acp_env_key(plugin_id: &str, suffix: &str) -> String {
    format!(
        "COWBOY_ACP_{}_{suffix}",
        plugin_id.replace('-', "_").to_ascii_uppercase()
    )
}

#[must_use]
pub(crate) fn adapter_plugins() -> Vec<(&'static str, &'static str)> {
    adapter_index()
        .plugin_to_slot
        .iter()
        .map(|(plugin, slot)| (*plugin, *slot))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathDetect {
    pub plugin_id: &'static str,
    pub command: &'static str,
    pub args: Option<String>,
}

/// PATH probes for canonical slot occupants, owned by provider.json entrypoints.
#[must_use]
pub(crate) fn path_detect() -> Vec<PathDetect> {
    let index = adapter_index();
    let mut detects = Vec::new();
    for (plugin_id, source) in NPM_PAYLOADS {
        let Some(&slot) = index.plugin_to_slot.get(plugin_id) else {
            continue;
        };
        if index.slot_to_primary.get(slot) != Some(plugin_id) {
            continue;
        }
        let Some(command) = runtime_entrypoint(plugin_id, source) else {
            continue;
        };
        let args = runtime_is_cli_only(plugin_id, source)
            .then(|| parse(plugin_id, source))
            .filter(|arguments| !arguments.is_empty())
            .map(|arguments| join_env(&arguments));
        detects.push(PathDetect {
            plugin_id,
            command,
            args,
        });
    }
    detects
}

fn entrypoints() -> &'static BTreeMap<&'static str, &'static str> {
    static VALUE: OnceLock<BTreeMap<&'static str, &'static str>> = OnceLock::new();
    VALUE.get_or_init(|| {
        NPM_PAYLOADS
            .iter()
            .filter_map(|(plugin_id, source)| {
                runtime_entrypoint(plugin_id, source).map(|command| (*plugin_id, command))
            })
            .collect()
    })
}

/// Occupancy slots in first-party host order.
#[must_use]
pub(crate) fn occupancy_slots() -> Vec<&'static str> {
    let mut slots = Vec::new();
    for (plugin_id, _) in ADAPTER_HOSTS {
        let Some(slot) = adapter_slot_for_provider(plugin_id) else {
            continue;
        };
        if !slots.contains(&slot) {
            slots.push(slot);
        }
    }
    slots
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CliAuthKind {
    Exit,
    GeminiEnv,
    GrokJson,
}

pub(crate) struct CliAuth {
    pub kind: CliAuthKind,
    pub argv: Vec<&'static str>,
}

#[must_use]
pub(crate) fn cli_auth_for_slot(slot: &str) -> Option<&'static CliAuth> {
    let primary = provider_for_adapter_slot(slot)?;
    cli_auths().get(primary)
}

fn cli_auths() -> &'static BTreeMap<&'static str, CliAuth> {
    static VALUE: OnceLock<BTreeMap<&'static str, CliAuth>> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut auths = BTreeMap::new();
        for (plugin_id, source) in ADAPTER_HOSTS {
            if let Some(auth) = parse_cli_auth(plugin_id, source) {
                auths.insert(*plugin_id, auth);
            }
        }
        auths
    })
}

fn parse_cli_auth(plugin_id: &str, source: &str) -> Option<CliAuth> {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} host.json must parse: {error}"));
    let kind = value.get("cli_auth").and_then(serde_json::Value::as_str)?;
    let kind = match kind {
        "exit" => CliAuthKind::Exit,
        "gemini-env" => CliAuthKind::GeminiEnv,
        "grok-json" => CliAuthKind::GrokJson,
        other => panic!("{plugin_id} cli_auth {other} is unsupported"),
    };
    let argv = value
        .get("cli_auth_argv")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            let text = item
                .as_str()
                .unwrap_or_else(|| panic!("{plugin_id} cli_auth_argv entries must be strings"));
            if text.is_empty() || text.len() > 256 || text.contains('\0') || text.contains("..") {
                panic!("{plugin_id} cli_auth_argv is invalid");
            }
            Box::leak(text.to_owned().into_boxed_str()) as &'static str
        })
        .collect::<Vec<_>>();
    match kind {
        CliAuthKind::Exit if argv.is_empty() => {
            panic!("{plugin_id} cli_auth exit needs cli_auth_argv")
        }
        CliAuthKind::GeminiEnv | CliAuthKind::GrokJson if !argv.is_empty() => {
            panic!("{plugin_id} cli_auth_argv is only valid for exit probes")
        }
        _ => {}
    }
    Some(CliAuth { kind, argv })
}

/// CLI binary name for an occupancy slot.
#[must_use]
pub(crate) fn cli_command_for_slot(slot: &str) -> Option<&'static str> {
    let occupancy = adapter_slot_for_provider(slot)?;
    let primary = provider_for_adapter_slot(occupancy)?;
    Some(cli_executable(primary).unwrap_or(occupancy))
}

/// Staged adapter command for an occupancy slot or plugin id.
#[must_use]
pub(crate) fn adapter_entrypoint(slot: &str) -> Option<&'static str> {
    let occupancy = adapter_slot_for_provider(slot)?;
    let primary = provider_for_adapter_slot(occupancy)?;
    entrypoints().get(primary).copied()
}

#[must_use]
pub(crate) fn provider_for_adapter_slot(slot: &str) -> Option<&'static str> {
    adapter_index().slot_to_primary.get(slot).copied()
}

/// Session provider ids that occupy a Machine adapter/CLI slot.
#[must_use]
pub(crate) fn occupancy_provider_ids(slot: &str) -> Vec<&'static str> {
    let index = adapter_index();
    let Some((&static_slot, plugins)) = index.slot_to_plugins.get_key_value(slot) else {
        return Vec::new();
    };
    let mut ids = vec![static_slot];
    for plugin in plugins {
        if !ids.contains(plugin) {
            ids.push(*plugin);
        }
    }
    ids
}

struct CliExecutable {
    command: &'static str,
    env: Option<&'static str>,
}

/// Command name of a sibling CLI that adapters should prefer over a bundled binary.
#[must_use]
pub(crate) fn cli_executable(plugin_id: &str) -> Option<&'static str> {
    cli_executables().get(plugin_id).map(|row| row.command)
}

#[must_use]
pub(crate) fn cli_executable_env(plugin_id: &str) -> Option<&'static str> {
    cli_executables().get(plugin_id).and_then(|row| row.env)
}

#[must_use]
pub(crate) fn cli_executable_plugins() -> Vec<(&'static str, &'static str)> {
    ADAPTER_HOSTS
        .iter()
        .filter_map(|(plugin_id, _)| cli_executable(plugin_id).map(|command| (*plugin_id, command)))
        .collect()
}

fn cli_executables() -> &'static BTreeMap<&'static str, CliExecutable> {
    static VALUE: OnceLock<BTreeMap<&'static str, CliExecutable>> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut commands = BTreeMap::new();
        for (plugin_id, source) in ADAPTER_HOSTS {
            if source.contains("\"cli_executable\"") {
                commands.insert(*plugin_id, parse_cli_executable(plugin_id, source));
            }
        }
        commands
    })
}

fn parse_cli_executable(plugin_id: &str, source: &str) -> CliExecutable {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} host.json must parse: {error}"));
    let text = value
        .get("cli_executable")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("{plugin_id} host.json must declare cli_executable"));
    if text.is_empty()
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || text.starts_with('-')
        || text.ends_with('-')
        || text.contains("--")
    {
        panic!("{plugin_id} cli_executable is invalid");
    }
    let env = value
        .get("cli_executable_env")
        .and_then(serde_json::Value::as_str)
        .map(|text| {
            if !valid_isolated_home_env(text) {
                panic!("{plugin_id} cli_executable_env is invalid");
            }
            Box::leak(text.to_owned().into_boxed_str()) as &'static str
        });
    CliExecutable {
        command: Box::leak(text.to_owned().into_boxed_str()),
        env,
    }
}

/// Adapter slot occupied by a session provider plugin, if declared.
#[must_use]
pub(crate) fn adapter_slot_for_provider(provider: &str) -> Option<&'static str> {
    let index = adapter_index();
    if let Some(&slot) = index.plugin_to_slot.get(provider) {
        return Some(slot);
    }
    index
        .slot_to_plugins
        .get_key_value(provider)
        .map(|(slot, _)| *slot)
}

struct IsolatedHome {
    env: Option<&'static str>,
    shell: bool,
    shell_env: Vec<&'static str>,
    shell_acp_key: Option<&'static str>,
}

fn isolated_homes() -> &'static BTreeMap<&'static str, IsolatedHome> {
    static VALUE: OnceLock<BTreeMap<&'static str, IsolatedHome>> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut homes = BTreeMap::new();
        for (plugin_id, source) in ADAPTER_HOSTS {
            if !(source.contains("\"isolated_home_env\"") || source.contains("\"isolated_shell\""))
            {
                continue;
            }
            homes.insert(*plugin_id, parse_isolated_home(plugin_id, source));
        }
        homes
    })
}

/// Package-less adapter home variable declared by `host.json`.
#[must_use]
pub(crate) fn isolated_home_env(plugin_id: &str) -> Option<&'static str> {
    isolated_homes().get(plugin_id).and_then(|home| home.env)
}

/// Whether the adapter requires a readiness-checked bash/zsh path.
#[must_use]
pub(crate) fn isolated_shell(plugin_id: &str) -> bool {
    isolated_homes()
        .get(plugin_id)
        .is_some_and(|home| home.shell)
}

/// Plugins that pin `COWBOY_ACP_<ID>_SHELL` across the Machine worker boundary.
#[must_use]
pub(crate) fn isolated_shell_plugins() -> Vec<&'static str> {
    isolated_homes()
        .iter()
        .filter(|(_, home)| home.shell)
        .map(|(plugin_id, _)| *plugin_id)
        .collect()
}

#[must_use]
pub(crate) fn isolated_shell_env(plugin_id: &str) -> &'static [&'static str] {
    isolated_homes()
        .get(plugin_id)
        .map(|home| home.shell_env.as_slice())
        .unwrap_or(&[])
}

#[must_use]
pub(crate) fn isolated_shell_acp_key(plugin_id: &str) -> Option<&'static str> {
    isolated_homes()
        .get(plugin_id)
        .and_then(|home| home.shell_acp_key)
}

fn parse_isolated_home(plugin_id: &str, source: &str) -> IsolatedHome {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} host.json must parse: {error}"));
    let env = value
        .get("isolated_home_env")
        .and_then(serde_json::Value::as_str)
        .map(|text| {
            if !valid_isolated_home_env(text) {
                panic!("{plugin_id} isolated_home_env is invalid");
            }
            Box::leak(text.to_owned().into_boxed_str()) as &'static str
        });
    let shell = value
        .get("isolated_shell")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let shell_env = value
        .get("isolated_shell_env")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            let text = item.as_str().unwrap_or_else(|| {
                panic!("{plugin_id} isolated_shell_env entries must be strings")
            });
            if !valid_isolated_home_env(text) {
                panic!("{plugin_id} isolated_shell_env is invalid");
            }
            Box::leak(text.to_owned().into_boxed_str()) as &'static str
        })
        .collect();
    let shell_acp_key =
        shell.then(|| Box::leak(acp_env_key(plugin_id, "SHELL").into_boxed_str()) as &'static str);
    IsolatedHome {
        env,
        shell,
        shell_env,
        shell_acp_key,
    }
}

fn valid_isolated_home_env(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase() || byte == b'_')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Display name declared by the account's host plugin.
#[must_use]
pub(crate) fn usage_product_label(account: &str) -> Option<&'static str> {
    usage_products().get(account).copied()
}

#[must_use]
pub(crate) fn usage_accounts() -> Vec<&'static str> {
    usage_account_rows()
        .iter()
        .map(|(_, account)| *account)
        .collect()
}

#[must_use]
pub(crate) fn provider_info_urls_env(account: &str) -> String {
    format!(
        "COWBOY_PROVIDER_INFO_{}_URLS",
        account.replace('-', "_").to_ascii_uppercase()
    )
}

#[must_use]
pub(crate) fn usage_reset_id_for_collector(collector: &str) -> Option<&'static str> {
    usage_collector_resets().get(collector).copied()
}

fn usage_collector_resets() -> &'static BTreeMap<String, &'static str> {
    static VALUE: OnceLock<BTreeMap<String, &'static str>> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut resets = BTreeMap::new();
        for (plugin_id, source) in ADAPTER_HOSTS {
            let value: serde_json::Value = serde_json::from_str(source)
                .unwrap_or_else(|error| panic!("{plugin_id} host.json must parse: {error}"));
            let Some(usage) = value.get("usage") else {
                continue;
            };
            let Some(collector) = usage.get("collector").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(reset) = usage
                .get("reset")
                .and_then(serde_json::Value::as_str)
                .filter(|reset| !reset.is_empty())
            else {
                continue;
            };
            let leaked: &'static str = Box::leak(reset.to_owned().into_boxed_str());
            resets.entry(collector.to_owned()).or_insert(leaked);
        }
        resets
    })
}

fn usage_account_rows() -> &'static Vec<(u16, &'static str)> {
    static VALUE: OnceLock<Vec<(u16, &'static str)>> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut rows = Vec::new();
        for (plugin_id, source) in ADAPTER_HOSTS {
            let value: serde_json::Value = serde_json::from_str(source)
                .unwrap_or_else(|error| panic!("{plugin_id} host.json must parse: {error}"));
            let Some(usage) = value.get("usage") else {
                continue;
            };
            let Some(account) = usage.get("account").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let order = usage
                .get("order")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u16::try_from(value).ok())
                .unwrap_or(u16::MAX);
            let leaked: &'static str = Box::leak(account.to_owned().into_boxed_str());
            if !rows.iter().any(|(_, existing)| *existing == leaked) {
                rows.push((order, leaked));
            }
        }
        rows.sort_by_key(|(order, account)| (*order, *account));
        rows
    })
}

#[must_use]
pub(crate) fn usage_activity_agent_ids(account: &str) -> &'static [&'static str] {
    usage_activity_agents()
        .get(account)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn usage_activity_agents() -> &'static BTreeMap<String, Vec<&'static str>> {
    static VALUE: OnceLock<BTreeMap<String, Vec<&'static str>>> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut agents = BTreeMap::new();
        for (plugin_id, source) in ADAPTER_HOSTS {
            let value: serde_json::Value = serde_json::from_str(source)
                .unwrap_or_else(|error| panic!("{plugin_id} host.json must parse: {error}"));
            let Some(usage) = value.get("usage") else {
                continue;
            };
            let Some(account) = usage.get("account").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(entries) = usage
                .get("activity_agents")
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            let ids: Vec<&'static str> = entries
                .iter()
                .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
                .filter(|id| !id.is_empty())
                .map(|id| {
                    let leaked: &'static str = Box::leak(id.to_owned().into_boxed_str());
                    leaked
                })
                .collect();
            if !ids.is_empty() {
                agents.entry(account.to_owned()).or_insert(ids);
            }
        }
        agents
    })
}

fn usage_products() -> &'static BTreeMap<String, &'static str> {
    static VALUE: OnceLock<BTreeMap<String, &'static str>> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut products = BTreeMap::new();
        for (plugin_id, source) in ADAPTER_HOSTS {
            let value: serde_json::Value = serde_json::from_str(source)
                .unwrap_or_else(|error| panic!("{plugin_id} host.json must parse: {error}"));
            let Some(usage) = value.get("usage") else {
                continue;
            };
            let Some(account) = usage.get("account").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(product) = usage
                .get("product")
                .and_then(serde_json::Value::as_str)
                .filter(|label| !label.is_empty())
            else {
                continue;
            };
            let leaked: &'static str = Box::leak(product.to_owned().into_boxed_str());
            products.entry(account.to_owned()).or_insert(leaked);
        }
        products
    })
}

/// PostgreSQL CASE that maps session/plugin ids onto adapter slots.
#[must_use]
pub(crate) fn diagnostic_agent_case_sql(column: &str) -> String {
    let index = adapter_index();
    let mut by_slot: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (plugin, slot) in &index.plugin_to_slot {
        by_slot.entry(*slot).or_default().push(*plugin);
    }
    if by_slot.is_empty() {
        return "NULL".to_owned();
    }
    let mut sql = String::from("CASE");
    for (slot, mut plugins) in by_slot {
        if !plugins.contains(&slot) {
            plugins.push(slot);
        }
        plugins.sort_unstable();
        plugins.dedup();
        let list = plugins
            .iter()
            .map(|id| format!("'{id}'"))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" WHEN {column} IN ({list}) THEN '{slot}'"));
    }
    sql.push_str(" ELSE NULL END");
    sql
}

#[must_use]
pub(crate) fn normalize_disabled_provider_slot(slot: &str) -> String {
    let index = adapter_index();
    if let Some(&adapter) = index.plugin_to_slot.get(slot)
        && index.slot_to_primary.get(adapter).copied() == Some(slot)
        && adapter != slot
    {
        return adapter.to_owned();
    }
    slot.to_owned()
}

#[must_use]
pub(crate) fn adapter_runtime_enabled(slot: &str, disabled: &[String]) -> bool {
    adapter_occupants(slot)
        .iter()
        .any(|id| !disabled.iter().any(|candidate| candidate == id))
}

fn adapter_occupants(slot: &str) -> Vec<&'static str> {
    let index = adapter_index();
    let Some((&static_slot, plugins)) = index.slot_to_plugins.get_key_value(slot) else {
        return Vec::new();
    };
    let primary = index.slot_to_primary.get(slot).copied();
    let mut ids: Vec<&'static str> = plugins
        .iter()
        .copied()
        .filter(|plugin| Some(*plugin) != primary || *plugin == static_slot)
        .collect();
    if !ids.contains(&static_slot) {
        ids.push(static_slot);
    }
    ids
}

#[must_use]
pub(crate) fn loopback_origin_for(plugin_id: &str) -> Option<&'static str> {
    loopback_gateways()
        .get(plugin_id)
        .map(|gateway| gateway.origin)
}

/// Inventory copy declared next to `loopback_origin`.
#[must_use]
pub(crate) fn loopback_detail(plugin_id: &str) -> Option<&'static str> {
    loopback_gateways()
        .get(plugin_id)
        .and_then(|gateway| gateway.detail)
}

/// Whether the loopback occupant also needs the package-less model catalog.
#[must_use]
pub(crate) fn loopback_requires_catalog(plugin_id: &str) -> bool {
    loopback_gateways()
        .get(plugin_id)
        .is_some_and(|gateway| gateway.requires_catalog)
}

/// Plugins that expose a loopback gateway, in host occupancy order.
#[must_use]
pub(crate) fn loopback_plugins() -> Vec<(&'static str, &'static str)> {
    ADAPTER_HOSTS
        .iter()
        .filter_map(|(plugin_id, _)| {
            loopback_origin_for(plugin_id).map(|origin| (*plugin_id, origin))
        })
        .collect()
}

struct LoopbackGateway {
    origin: &'static str,
    detail: Option<&'static str>,
    requires_catalog: bool,
    catalog: Option<&'static str>,
    env: Option<&'static str>,
}

#[must_use]
pub(crate) fn loopback_env(plugin_id: &str) -> Option<&'static str> {
    loopback_gateways()
        .get(plugin_id)
        .and_then(|gateway| gateway.env)
}

#[must_use]
pub(crate) fn loopback_catalog(plugin_id: &str) -> Option<&'static str> {
    loopback_gateways()
        .get(plugin_id)
        .and_then(|gateway| gateway.catalog)
}

#[must_use]
pub(crate) fn available_loopback_catalog(plugin_id: &str) -> Option<std::path::PathBuf> {
    let catalog = std::path::PathBuf::from(loopback_catalog(plugin_id)?);
    catalog.is_file().then_some(catalog)
}

fn loopback_gateways() -> &'static BTreeMap<&'static str, LoopbackGateway> {
    static VALUE: OnceLock<BTreeMap<&'static str, LoopbackGateway>> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut gateways = BTreeMap::new();
        for (plugin_id, source) in ADAPTER_HOSTS {
            if source.contains("\"loopback_origin\"") {
                gateways.insert(*plugin_id, parse_loopback_gateway(plugin_id, source));
            }
        }
        gateways
    })
}

#[must_use]
pub(crate) fn claude_deepseek_loopback_origin() -> &'static str {
    loopback_origin_for("claude-deepseek")
        .expect("claude-deepseek host.json must declare loopback_origin")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CacheProtectionPolicy {
    pub min_hit_tokens: u64,
    pub min_hit_label: &'static str,
    pub interval_ms: u32,
    pub interval_label: &'static str,
    pub option_name: &'static str,
    pub option_description: &'static str,
    pub option_on: &'static str,
    pub option_off: &'static str,
}

#[must_use]
pub(crate) fn deepseek_cache_protection() -> CacheProtectionPolicy {
    static VALUE: OnceLock<CacheProtectionPolicy> = OnceLock::new();
    *VALUE.get_or_init(|| {
        for (plugin_id, source) in ADAPTER_HOSTS {
            if source.contains("\"cache_protection\"") {
                return cache_protection(plugin_id, source);
            }
        }
        panic!("a host.json must declare usage.cache_protection")
    })
}

#[must_use]
pub(crate) fn codex_deepseek_loopback_origin() -> &'static str {
    loopback_origin_for("codex-deepseek")
        .expect("codex-deepseek host.json must declare loopback_origin")
}

#[must_use]
#[cfg(feature = "full")]
pub(crate) fn loopback_info_url(origin: &str) -> String {
    format!("{origin}/provider-info")
}

#[must_use]
#[cfg(feature = "full")]
pub(crate) fn loopback_info_urls() -> String {
    ADAPTER_HOSTS
        .iter()
        .filter_map(|(plugin_id, _)| loopback_origin_for(plugin_id))
        .map(loopback_info_url)
        .collect::<Vec<_>>()
        .join(",")
}

#[must_use]
#[cfg(feature = "full")]
pub(crate) fn deepseek_info_urls() -> String {
    loopback_info_urls()
}

#[must_use]
pub(crate) fn launch_plugin_ids() -> Vec<&'static str> {
    NPM_PAYLOADS
        .iter()
        .map(|(plugin_id, _)| *plugin_id)
        .collect()
}

#[must_use]
pub(crate) fn cli_arguments(plugin_id: &str) -> &'static [&'static str] {
    cli_argument_index()
        .get(plugin_id)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn cli_argument_index() -> &'static BTreeMap<&'static str, Vec<&'static str>> {
    static VALUE: OnceLock<BTreeMap<&'static str, Vec<&'static str>>> = OnceLock::new();
    VALUE.get_or_init(|| {
        NPM_PAYLOADS
            .iter()
            .map(|(plugin_id, source)| (*plugin_id, string_cli_arguments(plugin_id, source)))
            .collect()
    })
}

fn string_cli_arguments(plugin_id: &str, source: &str) -> Vec<&'static str> {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} provider.json must parse: {error}"));
    let Some(arguments) = value
        .get("runtime")
        .and_then(|runtime| runtime.get("arguments"))
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };
    if arguments.iter().any(|argument| !argument.is_string()) {
        return Vec::new();
    }
    arguments
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(|text| Box::leak(text.to_owned().into_boxed_str()) as &'static str)
        .collect()
}

#[must_use]
pub(crate) fn environment(plugin_id: &str) -> &'static [(&'static str, &'static str)] {
    environment_index()
        .get(plugin_id)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn environment_index() -> &'static BTreeMap<&'static str, Vec<(&'static str, &'static str)>> {
    static VALUE: OnceLock<BTreeMap<&'static str, Vec<(&'static str, &'static str)>>> =
        OnceLock::new();
    VALUE.get_or_init(|| {
        NPM_PAYLOADS
            .iter()
            .map(|(plugin_id, source)| (*plugin_id, environment_strings(plugin_id, source)))
            .collect()
    })
}

#[must_use]
pub(crate) fn remove_environment(plugin_id: &str) -> &'static [&'static str] {
    remove_environment_index()
        .get(plugin_id)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn remove_environment_index() -> &'static BTreeMap<&'static str, Vec<&'static str>> {
    static VALUE: OnceLock<BTreeMap<&'static str, Vec<&'static str>>> = OnceLock::new();
    VALUE.get_or_init(|| {
        NPM_PAYLOADS
            .iter()
            .map(|(plugin_id, source)| {
                (
                    *plugin_id,
                    string_list(plugin_id, source, "remove_environment"),
                )
            })
            .collect()
    })
}

#[must_use]
pub(crate) fn remove_environment_prefixes(plugin_id: &str) -> &'static [&'static str] {
    remove_environment_prefix_index()
        .get(plugin_id)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn remove_environment_prefix_index() -> &'static BTreeMap<&'static str, Vec<&'static str>> {
    static VALUE: OnceLock<BTreeMap<&'static str, Vec<&'static str>>> = OnceLock::new();
    VALUE.get_or_init(|| {
        NPM_PAYLOADS
            .iter()
            .map(|(plugin_id, source)| {
                (
                    *plugin_id,
                    string_list(plugin_id, source, "remove_environment_prefixes"),
                )
            })
            .collect()
    })
}

#[must_use]
pub(crate) fn grok() -> &'static [&'static str] {
    cli_arguments("grok")
}

#[must_use]
pub(crate) fn grok_env() -> String {
    join_env(grok())
}

#[must_use]
pub(crate) fn codex() -> &'static [&'static str] {
    cli_arguments("codex")
}

#[must_use]
pub(crate) fn gemini() -> &'static [&'static str] {
    cli_arguments("gemini")
}

#[must_use]
pub(crate) fn gemini_env() -> String {
    join_env(gemini())
}

#[must_use]
pub(crate) fn claude_deepseek_environment() -> &'static [(&'static str, &'static str)] {
    environment("claude-deepseek")
}

#[must_use]
pub(crate) fn claude_deepseek_remove_env() -> &'static [&'static str] {
    remove_environment("claude-deepseek")
}

#[must_use]
pub(crate) fn claude_deepseek_remove_env_prefixes() -> &'static [&'static str] {
    remove_environment_prefixes("claude-deepseek")
}

#[must_use]
pub(crate) fn codex_deepseek_environment() -> &'static [(&'static str, &'static str)] {
    environment("codex-deepseek")
}

#[must_use]
pub(crate) fn codex_deepseek_remove_env() -> &'static [&'static str] {
    remove_environment("codex-deepseek")
}

#[must_use]
pub(crate) fn codex_deepseek_remove_env_prefixes() -> &'static [&'static str] {
    remove_environment_prefixes("codex-deepseek")
}

#[must_use]
pub(crate) fn claude_deepseek_auto_compact_window() -> &'static str {
    required_environment_value(
        claude_deepseek_environment(),
        "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
    )
}

#[must_use]
pub(crate) fn claude_deepseek_max_output_tokens() -> &'static str {
    required_environment_value(
        claude_deepseek_environment(),
        "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
    )
}

#[must_use]
pub(crate) fn codex_deepseek_context_window() -> &'static str {
    static VALUE: OnceLock<&'static str> = OnceLock::new();
    VALUE.get_or_init(|| {
        argument_assignment("codex-deepseek", CODEX_DEEPSEEK, "model_context_window")
    })
}

#[must_use]
pub(crate) fn codex_deepseek_auto_compact_token_limit() -> &'static str {
    static VALUE: OnceLock<&'static str> = OnceLock::new();
    VALUE.get_or_init(|| {
        argument_assignment(
            "codex-deepseek",
            CODEX_DEEPSEEK,
            "model_auto_compact_token_limit",
        )
    })
}

#[must_use]
pub(crate) fn isolated_config_toml(
    plugin_id: &str,
    catalog: &Path,
    sidecar_origin: &str,
) -> String {
    let source = NPM_PAYLOADS
        .iter()
        .find(|(id, _)| *id == plugin_id)
        .map(|(_, source)| *source)
        .unwrap_or_else(|| panic!("{plugin_id} provider.json is missing"));
    let mut top = Vec::new();
    let mut tables: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for (key, value) in config_assignments(plugin_id, source, sidecar_origin) {
        if let Some((table, field)) = key.rsplit_once('.') {
            if let Some((_, fields)) = tables.iter_mut().find(|(name, _)| name == table) {
                fields.push((field.to_owned(), value));
            } else {
                tables.push((table.to_owned(), vec![(field.to_owned(), value)]));
            }
        } else {
            top.push((key, value));
        }
    }
    let catalog_entry = (
        "model_catalog_json".to_owned(),
        toml_quoted(&catalog.to_string_lossy()),
    );
    match top
        .iter()
        .position(|(key, _)| key == "model_reasoning_effort")
    {
        Some(index) => top.insert(index + 1, catalog_entry),
        None => top.push(catalog_entry),
    }

    let mut rendered = String::new();
    for (key, value) in &top {
        rendered.push_str(key);
        rendered.push_str(" = ");
        rendered.push_str(value);
        rendered.push('\n');
    }
    for (table, fields) in &tables {
        rendered.push('\n');
        rendered.push('[');
        rendered.push_str(table);
        rendered.push_str("]\n");
        for (key, value) in fields {
            rendered.push_str(key);
            rendered.push_str(" = ");
            rendered.push_str(value);
            rendered.push('\n');
        }
    }
    rendered
}

fn runtime_entrypoint(plugin_id: &str, source: &str) -> Option<&'static str> {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} provider.json must parse: {error}"));
    let text = value
        .get("runtime")
        .and_then(|runtime| runtime.get("entrypoint"))
        .and_then(serde_json::Value::as_str)
        .filter(|entrypoint| {
            !entrypoint.is_empty()
                && entrypoint
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })?;
    Some(Box::leak(text.to_owned().into_boxed_str()))
}

fn runtime_is_cli_only(plugin_id: &str, source: &str) -> bool {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} provider.json must parse: {error}"));
    let kinds: Vec<&str> = value
        .get("runtime")
        .and_then(|runtime| runtime.get("platforms"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|platform| {
            platform
                .get("private_components")
                .and_then(serde_json::Value::as_array)
        })
        .flatten()
        .filter_map(|component| component.get("kind").and_then(serde_json::Value::as_str))
        .collect();
    kinds.iter().any(|kind| *kind == "provider_cli")
        && !kinds.iter().any(|kind| *kind == "provider_adapter")
}

fn parse(plugin_id: &str, source: &str) -> Vec<&'static str> {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} provider.json must parse: {error}"));
    value
        .get("runtime")
        .and_then(|runtime| runtime.get("arguments"))
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("{plugin_id} provider.json must declare runtime.arguments"))
        .iter()
        .map(|argument| {
            let text = argument
                .as_str()
                .unwrap_or_else(|| panic!("{plugin_id} runtime argument must be a string"));
            let leaked: &'static str = Box::leak(text.to_owned().into_boxed_str());
            leaked
        })
        .collect()
}

fn environment_strings(plugin_id: &str, source: &str) -> Vec<(&'static str, &'static str)> {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} provider.json must parse: {error}"));
    value
        .get("runtime")
        .and_then(|runtime| runtime.get("environment"))
        .and_then(serde_json::Value::as_object)
        .unwrap_or_else(|| panic!("{plugin_id} provider.json must declare runtime.environment"))
        .iter()
        .filter_map(|(key, value)| {
            value.as_str().map(|text| {
                let key: &'static str = Box::leak(key.clone().into_boxed_str());
                let text: &'static str = Box::leak(text.to_owned().into_boxed_str());
                (key, text)
            })
        })
        .collect()
}

const MAX_CACHE_MIN_HIT_TOKENS: u64 = 10_000_000;
const MAX_CACHE_INTERVAL_MS: u32 = 7 * 24 * 60 * 60 * 1_000;

fn cache_protection(plugin_id: &str, source: &str) -> CacheProtectionPolicy {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} host.json must parse: {error}"));
    let row = value
        .pointer("/usage/cache_protection")
        .unwrap_or_else(|| panic!("{plugin_id} host.json must declare usage.cache_protection"));
    let min_hit_tokens = row
        .get("min_hit_tokens")
        .and_then(serde_json::Value::as_u64)
        .filter(|tokens| (1..=MAX_CACHE_MIN_HIT_TOKENS).contains(tokens))
        .unwrap_or_else(|| panic!("{plugin_id} cache_protection.min_hit_tokens is invalid"));
    let interval_ms = row
        .get("interval_ms")
        .and_then(serde_json::Value::as_u64)
        .and_then(|milliseconds| u32::try_from(milliseconds).ok())
        .filter(|milliseconds| (1..=MAX_CACHE_INTERVAL_MS).contains(milliseconds))
        .unwrap_or_else(|| panic!("{plugin_id} cache_protection.interval_ms is invalid"));
    CacheProtectionPolicy {
        min_hit_tokens,
        min_hit_label: required_cache_copy(plugin_id, row, "min_hit_label", 256),
        interval_ms,
        interval_label: required_cache_copy(plugin_id, row, "interval_label", 256),
        option_name: required_cache_copy(plugin_id, row, "option_name", 256),
        option_description: required_cache_copy(plugin_id, row, "option_description", 512),
        option_on: required_cache_copy(plugin_id, row, "option_on", 256),
        option_off: required_cache_copy(plugin_id, row, "option_off", 256),
    }
}

fn required_cache_copy(
    plugin_id: &str,
    row: &serde_json::Value,
    field: &str,
    max: usize,
) -> &'static str {
    let text = row
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|label| !label.is_empty() && label.len() <= max && !label.contains('\0'))
        .unwrap_or_else(|| panic!("{plugin_id} cache_protection.{field} is invalid"));
    Box::leak(text.to_owned().into_boxed_str())
}

fn parse_loopback_gateway(plugin_id: &str, source: &str) -> LoopbackGateway {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} host.json must parse: {error}"));
    let text = value
        .get("loopback_origin")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("{plugin_id} host.json must declare loopback_origin"));
    let port = text
        .strip_prefix("http://127.0.0.1:")
        .unwrap_or_else(|| panic!("{plugin_id} loopback_origin must be http://127.0.0.1:<port>"));
    if port.is_empty()
        || !port.bytes().all(|byte| byte.is_ascii_digit())
        || !port.parse::<u16>().is_ok_and(|port| port > 0)
    {
        panic!("{plugin_id} loopback_origin port is invalid");
    }
    let detail = value
        .get("loopback_detail")
        .and_then(serde_json::Value::as_str)
        .map(|text| {
            if text.is_empty()
                || text.len() > 64
                || text.contains('\0')
                || text.chars().any(char::is_control)
            {
                panic!("{plugin_id} loopback_detail is invalid");
            }
            Box::leak(text.to_owned().into_boxed_str()) as &'static str
        });
    let requires_catalog = value
        .get("loopback_requires_catalog")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let env = value
        .get("loopback_env")
        .and_then(serde_json::Value::as_str)
        .map(|text| {
            if !valid_isolated_home_env(text) {
                panic!("{plugin_id} loopback_env is invalid");
            }
            Box::leak(text.to_owned().into_boxed_str()) as &'static str
        });
    let catalog = value
        .get("loopback_catalog")
        .and_then(serde_json::Value::as_str)
        .map(|text| {
            if !valid_loopback_catalog(text) {
                panic!("{plugin_id} loopback_catalog is invalid");
            }
            Box::leak(text.to_owned().into_boxed_str()) as &'static str
        });
    if requires_catalog && catalog.is_none() {
        panic!("{plugin_id} loopback_requires_catalog needs loopback_catalog");
    }
    LoopbackGateway {
        origin: Box::leak(text.to_owned().into_boxed_str()),
        detail,
        requires_catalog,
        catalog,
        env,
    }
}

fn valid_loopback_catalog(value: &str) -> bool {
    (1..=512).contains(&value.len())
        && value.starts_with('/')
        && !value.contains('\0')
        && !value.contains("//")
        && !value.split('/').any(|component| component == "..")
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.'))
}

fn optional_adapter_slot(plugin_id: &str, source: &str) -> Option<&'static str> {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} host.json must parse: {error}"));
    let text = value
        .get("adapter_slot")
        .and_then(serde_json::Value::as_str)?;
    if text.is_empty()
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || text.starts_with('-')
        || text.ends_with('-')
        || text.contains("--")
    {
        panic!("{plugin_id} adapter_slot is invalid");
    }
    Some(Box::leak(text.to_owned().into_boxed_str()))
}

fn required_environment_value(entries: &[(&'static str, &'static str)], key: &str) -> &'static str {
    entries
        .iter()
        .find_map(|(name, value)| (*name == key).then_some(*value))
        .unwrap_or_else(|| panic!("plugin environment missing {key}"))
}

fn string_list(plugin_id: &str, source: &str, field: &str) -> Vec<&'static str> {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} provider.json must parse: {error}"));
    value
        .get("runtime")
        .and_then(|runtime| runtime.get(field))
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("{plugin_id} provider.json must declare runtime.{field}"))
        .iter()
        .map(|item| {
            let text = item
                .as_str()
                .unwrap_or_else(|| panic!("{plugin_id} runtime.{field} entries must be strings"));
            let leaked: &'static str = Box::leak(text.to_owned().into_boxed_str());
            leaked
        })
        .collect()
}

fn config_assignments(
    plugin_id: &str,
    source: &str,
    sidecar_origin: &str,
) -> Vec<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} provider.json must parse: {error}"));
    let arguments = value
        .get("runtime")
        .and_then(|runtime| runtime.get("arguments"))
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("{plugin_id} provider.json must declare runtime.arguments"));
    let mut assignments = Vec::new();
    let mut expect_assignment = false;
    for argument in arguments {
        if expect_assignment {
            expect_assignment = false;
            assignments.push(assignment_from_runtime_value(
                plugin_id,
                argument,
                sidecar_origin,
            ));
            continue;
        }
        match argument.as_str() {
            Some("-c") => expect_assignment = true,
            Some(text) if text.contains('=') => {
                assignments.push(split_assignment(plugin_id, text));
            }
            _ => {}
        }
    }
    if expect_assignment {
        panic!("{plugin_id} runtime.arguments ended with a dangling -c");
    }
    assignments
}

fn assignment_from_runtime_value(
    plugin_id: &str,
    argument: &serde_json::Value,
    sidecar_origin: &str,
) -> (String, String) {
    if let Some(text) = argument.as_str() {
        return split_assignment(plugin_id, text);
    }
    let object = argument.as_object().unwrap_or_else(|| {
        panic!("{plugin_id} runtime argument must be a string or sidecar binding")
    });
    let source = object
        .get("source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("{plugin_id} runtime binding must declare source"));
    if source != "sidecar_url" {
        panic!("{plugin_id} package-less config only resolves sidecar_url bindings");
    }
    let prefix = object
        .get("prefix")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let suffix = object
        .get("suffix")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    split_assignment(plugin_id, &format!("{prefix}{sidecar_origin}{suffix}"))
}

fn split_assignment(plugin_id: &str, text: &str) -> (String, String) {
    let (key, value) = text
        .split_once('=')
        .unwrap_or_else(|| panic!("{plugin_id} runtime assignment must be key=value, got {text}"));
    (key.to_owned(), encode_toml_value(value))
}

fn encode_toml_value(raw: &str) -> String {
    if raw.starts_with('{') && raw.ends_with('}') {
        raw.replace('{', "{ ")
            .replace('}', " }")
            .replace('=', " = ")
            .replace(',', ", ")
    } else {
        raw.to_owned()
    }
}

fn toml_quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn argument_assignment(plugin_id: &str, source: &str, key: &str) -> &'static str {
    let value: serde_json::Value = serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("{plugin_id} provider.json must parse: {error}"));
    let prefix = format!("{key}=");
    let text = value
        .get("runtime")
        .and_then(|runtime| runtime.get("arguments"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .find_map(|argument| argument.strip_prefix(prefix.as_str()))
        .map(|raw| raw.trim_matches('"'))
        .unwrap_or_else(|| {
            panic!("{plugin_id} provider.json must declare runtime.arguments {key}=...")
        });
    Box::leak(text.to_owned().into_boxed_str())
}

fn join_env(args: &[&str]) -> String {
    args.iter()
        .map(|argument| posix_quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn posix_quote(argument: &str) -> String {
    if argument
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        argument.to_owned()
    } else {
        format!("'{}'", argument.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_party_runtime_args_are_owned_by_plugin_payloads() {
        assert!(codex().windows(2).any(|pair| {
            pair == ["-c", "approval_policy=never"] || pair == ["-c", "approval_policy=\"never\""]
        }));
        assert_eq!(gemini(), ["--acp"]);
        assert_eq!(gemini_env(), "--acp");
        assert!(grok().contains(&"--no-auto-update"));
        assert!(grok().contains(&"--experimental-memory"));
        assert!(grok().contains(&"agent"));
        assert!(grok().contains(&"stdio"));
        assert!(grok_env().contains("AGENTS.md"));
        assert_eq!(usage_product_label("xai"), Some("xAI"));
        assert_eq!(usage_product_label("openai"), Some("OpenAI"));
        assert_eq!(usage_product_label("deepseek"), Some("DeepSeek"));
        assert_eq!(usage_product_label("unknown"), None);
        assert_eq!(claude_deepseek_auto_compact_window(), "819200");
        assert_eq!(claude_deepseek_max_output_tokens(), "128000");
        assert!(
            claude_deepseek_environment()
                .iter()
                .any(|(key, value)| *key == "ANTHROPIC_AUTH_TOKEN"
                    && *value == "cowboy-local-credential-boundary")
        );
        assert!(
            !claude_deepseek_environment()
                .iter()
                .any(|(key, _)| *key == "ANTHROPIC_BASE_URL")
        );
        assert!(claude_deepseek_remove_env().contains(&"DISABLE_AUTO_COMPACT"));
        assert_eq!(
            claude_deepseek_remove_env_prefixes(),
            ["ANTHROPIC_", "CLAUDE_", "DEEPSEEK_"]
        );
        assert_eq!(
            required_environment_value(codex_deepseek_environment(), "MODEL_PROVIDER"),
            "deepseek-local"
        );
        assert!(codex_deepseek_remove_env().is_empty());
        assert_eq!(
            codex_deepseek_remove_env_prefixes(),
            ["CHATGPT_", "CODEX_", "DEEPSEEK_", "OPENAI_"]
        );
        assert_eq!(codex_deepseek_context_window(), "680000");
        assert_eq!(codex_deepseek_auto_compact_token_limit(), "646000");
        assert_eq!(
            npm_package_for_component("provider_cli", "codex"),
            Some("@openai/codex")
        );
        assert_eq!(
            npm_package_for_component("provider_adapter", "codex"),
            Some("@agentclientprotocol/codex-acp")
        );
        assert_eq!(
            npm_package_for_component("provider_cli", "claude"),
            Some("@anthropic-ai/claude-code")
        );
        assert_eq!(
            npm_package_for_component("provider_adapter", "claude"),
            Some("@agentclientprotocol/claude-agent-acp")
        );
        assert_eq!(
            npm_package_for_component("provider_cli", "gemini"),
            Some("@google/gemini-cli")
        );
        assert_eq!(
            npm_package_for_component("provider_cli", "grok"),
            Some("@xai-official/grok")
        );
        assert_eq!(npm_package_for_component("provider_adapter", "grok"), None);
        assert_eq!(
            npm_package_for_component("provider_cli", "arbitrary-package"),
            None
        );
        assert_eq!(
            npx_package_for_plugin("claude-code"),
            "@agentclientprotocol/claude-agent-acp"
        );
        assert_eq!(
            npx_package_for_plugin("claude-deepseek"),
            "@agentclientprotocol/claude-agent-acp"
        );
        assert_eq!(
            npx_package_for_plugin("codex"),
            "@agentclientprotocol/codex-acp"
        );
        assert_eq!(
            npx_package_for_plugin("codex-deepseek"),
            "@agentclientprotocol/codex-acp"
        );
        assert_eq!(npx_package_for_plugin("gemini"), "@google/gemini-cli");
        assert_eq!(npx_package_for_plugin("grok"), "@xai-official/grok");
        assert_eq!(npx_prefix("grok"), ["-y", "@xai-official/grok"]);
        assert!(
            adapter_plugins()
                .iter()
                .any(|(id, slot)| *id == "claude-code" && *slot == "claude")
        );
        let detect = path_detect();
        assert!(detect.iter().any(|row| row.plugin_id == "codex"
            && row.command == "codex-acp"
            && row.args.is_none()));
        assert!(detect.iter().any(|row| row.plugin_id == "grok"
            && row.command == "grok"
            && row.args.as_deref() == Some(grok_env().as_str())));
        assert!(detect.iter().any(|row| row.plugin_id == "gemini"
            && row.command == "gemini"
            && row.args.as_deref() == Some(gemini_env().as_str())));
        assert!(detect.iter().any(|row| row.plugin_id == "claude-code"
            && row.command == "claude-agent-acp"
            && row.args.is_none()));
        assert!(!detect.iter().any(|row| row.plugin_id == "claude-deepseek"));
        assert!(!detect.iter().any(|row| row.plugin_id == "codex-deepseek"));
        assert_eq!(occupancy_slots(), ["codex", "grok", "gemini", "claude"]);
        assert_eq!(cli_command_for_slot("claude"), Some("claude"));
        assert_eq!(cli_command_for_slot("codex"), Some("codex"));
        assert_eq!(
            cli_auth_for_slot("codex").map(|auth| (auth.kind, auth.argv.as_slice())),
            Some((CliAuthKind::Exit, ["login", "status"].as_slice()))
        );
        assert_eq!(
            cli_auth_for_slot("claude").map(|auth| (auth.kind, auth.argv.as_slice())),
            Some((CliAuthKind::Exit, ["auth", "status", "--json"].as_slice()))
        );
        assert_eq!(
            cli_auth_for_slot("gemini").map(|auth| auth.kind),
            Some(CliAuthKind::GeminiEnv)
        );
        assert_eq!(
            cli_auth_for_slot("grok").map(|auth| auth.kind),
            Some(CliAuthKind::GrokJson)
        );
        assert_eq!(adapter_entrypoint("codex"), Some("codex-acp"));
        assert_eq!(adapter_entrypoint("claude"), Some("claude-agent-acp"));
        assert_eq!(adapter_entrypoint("claude-code"), Some("claude-agent-acp"));
        assert_eq!(adapter_entrypoint("gemini"), Some("gemini"));
        assert_eq!(adapter_entrypoint("grok"), Some("grok"));
        assert_eq!(adapter_entrypoint("future"), None);
        assert_eq!(
            acp_env_key("claude-code", "CMD"),
            "COWBOY_ACP_CLAUDE_CODE_CMD"
        );
        assert_eq!(
            acp_env_key("claude-deepseek", "SHELL"),
            "COWBOY_ACP_CLAUDE_DEEPSEEK_SHELL"
        );
        assert_eq!(
            loopback_origin_for("codex-deepseek"),
            Some("http://127.0.0.1:61137")
        );
        assert_eq!(
            loopback_origin_for("claude-deepseek"),
            Some("http://127.0.0.1:61138")
        );
        assert_eq!(loopback_origin_for("grok"), None);
        assert_eq!(usage_activity_agent_ids("deepseek"), ["codex", "claude"]);
        assert_eq!(usage_activity_agent_ids("openai"), &[] as &[&str]);
        assert_eq!(
            usage_accounts(),
            ["openai", "xai", "anthropic", "deepseek", "gemini"]
        );
        assert_eq!(
            provider_info_urls_env("deepseek"),
            "COWBOY_PROVIDER_INFO_DEEPSEEK_URLS"
        );
        assert_eq!(
            usage_reset_id_for_collector("openai-appserver"),
            Some("codex")
        );
        assert_eq!(usage_reset_id_for_collector("xai-billing"), Some("xai"));
        assert_eq!(usage_reset_id_for_collector("deepseek-store"), None);
        assert_eq!(codex_deepseek_loopback_origin(), "http://127.0.0.1:61137");
        assert_eq!(claude_deepseek_loopback_origin(), "http://127.0.0.1:61138");
        assert_eq!(deepseek_cache_protection().min_hit_tokens, 64_000);
        assert_eq!(deepseek_cache_protection().min_hit_label, "64K");
        assert_eq!(deepseek_cache_protection().interval_ms, 28_800_000);
        assert_eq!(deepseek_cache_protection().interval_label, "8h");
        assert_eq!(deepseek_cache_protection().option_name, "Cache protection");
        assert_eq!(deepseek_cache_protection().option_on, "Auto · recommended");
        assert_eq!(deepseek_cache_protection().option_off, "Off");
        assert!(
            deepseek_cache_protection()
                .option_description
                .contains("64K")
        );
        #[cfg(feature = "full")]
        assert_eq!(
            loopback_info_urls(),
            "http://127.0.0.1:61137/provider-info,http://127.0.0.1:61138/provider-info"
        );
        #[cfg(feature = "full")]
        assert_eq!(deepseek_info_urls(), loopback_info_urls());
        let rendered = isolated_config_toml(
            "codex-deepseek",
            Path::new(
                "/nix/var/nix/profiles/columbus-components/codex-deepseek/share/codex-deepseek/codex-models.json",
            ),
            codex_deepseek_loopback_origin(),
        );
        assert!(rendered.starts_with("model = \"deepseek-v4-flash\""));
        assert!(rendered.contains(
            "model_catalog_json = \"/nix/var/nix/profiles/columbus-components/codex-deepseek/share/codex-deepseek/codex-models.json\""
        ));
        assert!(rendered.contains(&format!(
            "base_url = \"{}/v1\"",
            codex_deepseek_loopback_origin()
        )));
        assert!(rendered.contains("[model_providers.deepseek-local]"));
        assert!(rendered.contains("[features]\nmemories = true"));
        assert!(rendered.contains("[memories]\ndisable_on_external_context = true"));
        assert!(!rendered.contains("api.openai.com"));
        assert_eq!(provider_for_adapter_slot("claude"), Some("claude-code"));
        assert_eq!(provider_for_adapter_slot("grok"), Some("grok"));
        assert_eq!(provider_for_adapter_slot("codex"), Some("codex"));
        assert_eq!(
            occupancy_provider_ids("claude"),
            ["claude", "claude-code", "claude-deepseek"]
        );
        assert_eq!(occupancy_provider_ids("codex"), ["codex", "codex-deepseek"]);
        assert_eq!(occupancy_provider_ids("grok"), ["grok"]);
        assert_eq!(adapter_slot_for_provider("claude-deepseek"), Some("claude"));
        assert_eq!(adapter_slot_for_provider("codex-deepseek"), Some("codex"));
        assert_eq!(isolated_home_env("codex-deepseek"), Some("CODEX_HOME"));
        assert_eq!(
            isolated_home_env("claude-deepseek"),
            Some("CLAUDE_CONFIG_DIR")
        );
        assert_eq!(isolated_home_env("codex"), None);
        assert!(isolated_shell("claude-deepseek"));
        assert!(!isolated_shell("codex-deepseek"));
        assert!(!isolated_shell("claude-code"));
        assert_eq!(isolated_shell_plugins(), ["claude-deepseek"]);
        assert_eq!(cli_executable("claude-code"), Some("claude"));
        assert_eq!(
            cli_executable_env("claude-code"),
            Some("CLAUDE_CODE_EXECUTABLE")
        );
        assert_eq!(cli_executable("codex"), None);
        assert_eq!(cli_executable_plugins(), [("claude-code", "claude")]);
        assert_eq!(
            isolated_shell_env("claude-deepseek"),
            ["CLAUDE_CODE_SHELL", "SHELL"]
        );
        assert_eq!(
            isolated_shell_acp_key("claude-deepseek"),
            Some("COWBOY_ACP_CLAUDE_DEEPSEEK_SHELL")
        );
        assert_eq!(loopback_env("claude-deepseek"), Some("ANTHROPIC_BASE_URL"));
        assert_eq!(loopback_env("codex-deepseek"), None);
        assert_eq!(
            loopback_catalog("codex-deepseek"),
            Some(
                "/nix/var/nix/profiles/columbus-components/codex-deepseek/share/codex-deepseek/codex-models.json"
            )
        );
        assert_eq!(loopback_catalog("claude-deepseek"), None);
        assert_eq!(available_loopback_catalog("claude-deepseek"), None);
        assert!(launch_plugin_ids().contains(&"grok"));
        assert!(cli_arguments("codex-deepseek").is_empty());
        assert!(!cli_arguments("grok").is_empty());
        assert!(remove_environment("claude-deepseek").contains(&"ENABLE_CLAUDEAI_MCP_SERVERS"));
        assert_eq!(
            loopback_plugins(),
            [
                ("codex-deepseek", "http://127.0.0.1:61137"),
                ("claude-deepseek", "http://127.0.0.1:61138"),
            ]
        );
        assert_eq!(
            loopback_detail("codex-deepseek"),
            Some("loopback Responses gateway")
        );
        assert_eq!(
            loopback_detail("claude-deepseek"),
            Some("isolated loopback Anthropic Messages gateway")
        );
        assert!(loopback_requires_catalog("codex-deepseek"));
        assert!(!loopback_requires_catalog("claude-deepseek"));
        assert!(
            diagnostic_agent_case_sql("session.provider").contains("WHEN session.provider IN (")
        );
        assert!(diagnostic_agent_case_sql("session.provider").contains("'codex-deepseek'"));
        assert_eq!(normalize_disabled_provider_slot("claude-code"), "claude");
        assert_eq!(
            normalize_disabled_provider_slot("claude-deepseek"),
            "claude-deepseek"
        );
        assert_eq!(normalize_disabled_provider_slot("gemini"), "gemini");
        assert!(adapter_runtime_enabled("claude", &["claude".to_owned()]));
        assert!(adapter_runtime_enabled(
            "claude",
            &["claude-deepseek".to_owned()]
        ));
        assert!(!adapter_runtime_enabled(
            "claude",
            &["claude".to_owned(), "claude-deepseek".to_owned()]
        ));
        assert!(adapter_runtime_enabled("codex", &["codex".to_owned()]));
        assert!(!adapter_runtime_enabled(
            "codex",
            &["codex".to_owned(), "codex-deepseek".to_owned()]
        ));
    }
}
