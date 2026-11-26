//! Target-local `~/.claude.json` user MCP server resolution, validation, and
//! keeper-owned lifecycle state (docs/antigravity-user-mcp-r2-design.md).
//!
//! All owner configuration (`~/.claude.json`, server `env`, `args`, executable
//! paths) stays strictly inside the target keeper. Only `{ name, scope,
//! fingerprint }` crosses to the worker.

use anyhow::{Context as _, Result, ensure};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::io::Read as _;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use tokio::sync::watch;

/// Initialization capability key advertised by ordinary (non-managed) keepers.
/// Kept byte-identical to `src/worker_execution/extension_broker.rs` by test.
pub const USER_MCP_INITIALIZATION_KEY: &str = "cowboyUserMcp";
/// Keeper method listing valid stdio user MCP servers from `~/.claude.json`.
pub const USER_MCP_LIST_METHOD: &str = "cowboy/listUserMcp";
/// Keeper method starting one user MCP server via native `process/start`
/// without `COWBOY_CALL_CONTEXT` and with `envPolicy.inherit = "none"`.
pub const USER_MCP_START_METHOD: &str = "cowboy/startUserMcp";
/// Keeper method stopping targeted, allowlist-excluded, or all user MCP servers.
pub const USER_MCP_STOP_METHOD: &str = "cowboy/stopUserMcp";
/// Process ID sub-prefix under the existing protected `cowboy-worker-mcp-`
/// namespace so every previously released worker isolates user MCP servers.
pub const USER_MCP_PROCESS_PREFIX: &str = "cowboy-worker-mcp-user-";

const MAX_CLAUDE_JSON_BYTES: u64 = 4 * 1024 * 1024;
const MAX_USER_MCP_SERVERS: usize = 16;
const MAX_OMITTED_NOTES: usize = 32;
pub(super) const MAX_ACTIVE_USER_SERVERS: usize = 32;
const MAX_STOPPED_TOMBSTONES: usize = 512;
const MAX_ARGS: usize = 128;
const MAX_ARG_BYTES: usize = 4096;
const MAX_ENV_ENTRIES: usize = 128;
const MAX_ENV_VALUE_BYTES: usize = 8192;
const MAX_GIT_WALK_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerGate {
    pub enabled: bool,
    pub allowlist: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedServer {
    pub name: String,
    pub scope: &'static str,
    pub fingerprint: String,
    pub logical_executable: PathBuf,
    pub args: Vec<String>,
    pub child_env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmittedServer {
    pub name: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedCatalog {
    pub servers: Vec<ResolvedServer>,
    pub omitted: Vec<OmittedServer>,
}

/// Start lifecycle of a keeper-owned user MCP server (Rule E1 confirmation).
pub(super) enum StartState {
    /// `process/start` was admitted and sent to the executor, reply not yet observed.
    /// Value: `None` = pending, `Some(true)` = confirmed started, `Some(false)` = start error.
    Pending(watch::Sender<Option<bool>>),
    /// Executor replied `result` to `process/start`.
    Confirmed,
}

pub(super) struct ActiveUserMcpProcess {
    pub server_name: String,
    pub native_id: u64,
    pub start_state: StartState,
}

#[derive(Default)]
pub(super) struct UserMcpState {
    pub stop_epoch: u64,
    pub latest_allowlist: Option<Vec<String>>,
    stopped_order: VecDeque<String>,
    stopped_set: HashSet<String>,
    pub active_processes: HashMap<String, ActiveUserMcpProcess>,
    native_id_to_process: HashMap<u64, String>,
    stop_seq: u64,
}

impl UserMcpState {
    pub fn next_stop_operation_id(&mut self) -> String {
        self.stop_seq = self.stop_seq.saturating_add(1);
        format!("cowboy-stop-user-mcp-{}", self.stop_seq)
    }

    pub fn is_tombstoned(&self, process_id: &str) -> bool {
        self.stopped_set.contains(process_id)
    }

    pub fn record_tombstone(&mut self, process_id: &str) {
        if !self.stopped_set.insert(process_id.to_owned()) {
            return;
        }
        self.stopped_order.push_back(process_id.to_owned());
        while self.stopped_order.len() > MAX_STOPPED_TOMBSTONES {
            if let Some(oldest) = self.stopped_order.pop_front() {
                self.stopped_set.remove(&oldest);
            }
        }
    }

    pub fn register_pending(&mut self, process_id: String, server_name: String, native_id: u64) {
        let (tx, _rx) = watch::channel(None);
        self.native_id_to_process
            .insert(native_id, process_id.clone());
        self.active_processes.insert(
            process_id,
            ActiveUserMcpProcess {
                server_name,
                native_id,
                start_state: StartState::Pending(tx),
            },
        );
    }

    /// Called strictly AFTER dropping `Host.ledger` lock.
    pub fn on_native_reply(&mut self, native_id: u64, succeeded: bool) {
        let Some(process_id) = self.native_id_to_process.remove(&native_id) else {
            return;
        };
        if succeeded {
            if let Some(entry) = self.active_processes.get_mut(&process_id) {
                if let StartState::Pending(tx) = &entry.start_state {
                    let _ = tx.send(Some(true));
                }
                entry.start_state = StartState::Confirmed;
            }
        } else if let Some(entry) = self.active_processes.remove(&process_id)
            && let StartState::Pending(tx) = entry.start_state
        {
            let _ = tx.send(Some(false));
        }
    }

    /// Called strictly AFTER dropping `Host.ledger` lock when executor reports
    /// `process/exited` or `process/closed`.
    pub fn on_process_ended(&mut self, process_id: &str) {
        if let Some(entry) = self.active_processes.get(process_id)
            && matches!(entry.start_state, StartState::Confirmed)
        {
            let removed = self.active_processes.remove(process_id);
            if let Some(removed) = removed {
                self.native_id_to_process.remove(&removed.native_id);
            }
        }
    }

    /// Called when the backend executor stops or is lost: wake any waiters on
    /// pending starts and clear active processes (executor owns the process group).
    pub fn on_backend_lost(&mut self) {
        for (_, entry) in self.active_processes.drain() {
            if let StartState::Pending(tx) = entry.start_state {
                let _ = tx.send(Some(false));
            }
        }
        self.native_id_to_process.clear();
    }

    pub fn remove_confirmed_or_failed(&mut self, process_id: &str) {
        if let Some(removed) = self.active_processes.remove(process_id) {
            self.native_id_to_process.remove(&removed.native_id);
        }
    }
}

/// Validates `cowboy-worker-mcp-user-<32 lowercase hex chars>`.
#[must_use]
pub fn valid_user_mcp_process_id(id: &str) -> bool {
    let Some(suffix) = id.strip_prefix(USER_MCP_PROCESS_PREFIX) else {
        return false;
    };
    suffix.len() == 32
        && suffix
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[must_use]
pub fn valid_server_name(name: &str) -> bool {
    crate::config::valid_mcp_server_name(name) && !name.to_ascii_lowercase().starts_with("cowboy")
}

/// Standard RFC 2104 HMAC-SHA-256 over the existing `sha2` crate (zero new
/// dependencies and zero `Cargo.lock` change).
#[must_use]
pub fn hmac_sha256_hex(key: &[u8; 32], message: &[u8]) -> String {
    const BLOCK: usize = 64;
    let mut ipad = [0x36_u8; BLOCK];
    let mut opad = [0x5c_u8; BLOCK];
    for (index, byte) in key.iter().enumerate() {
        ipad[index] ^= byte;
        opad[index] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let digest = outer.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

/// Neutral, read-only directory outside `<keeper_state_dir>` and Machine state:
/// `/tmp/cowboy-user-mcp-<uid>/<incarnation>` (parent `0700`, leaf `0500`).
pub fn neutral_cwd_root() -> PathBuf {
    PathBuf::from(format!(
        "/tmp/cowboy-user-mcp-{}",
        rustix::process::geteuid().as_raw()
    ))
}

pub fn create_neutral_cwd(incarnation: &str) -> Result<PathBuf> {
    ensure!(
        crate::execution_protocol::valid_operation_id(incarnation),
        "invalid incarnation for user MCP neutral cwd"
    );
    let root = neutral_cwd_root();
    let uid = rustix::process::geteuid().as_raw();
    match std::fs::symlink_metadata(&root) {
        Ok(meta) => {
            ensure!(
                meta.is_dir()
                    && !meta.file_type().is_symlink()
                    && meta.uid() == uid
                    && meta.mode() & 0o077 == 0,
                "user MCP neutral cwd root has unsafe ownership or permissions"
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&root)
                .context("creating user MCP neutral cwd root")?;
            let meta = std::fs::symlink_metadata(&root)?;
            ensure!(
                meta.is_dir()
                    && !meta.file_type().is_symlink()
                    && meta.uid() == uid
                    && meta.mode() & 0o077 == 0,
                "user MCP neutral cwd root verification failed"
            );
        }
        Err(error) => return Err(error).context("inspecting user MCP neutral cwd root"),
    }
    let leaf = root.join(incarnation);
    if let Ok(meta) = std::fs::symlink_metadata(&leaf) {
        ensure!(
            meta.is_dir() && !meta.file_type().is_symlink() && meta.uid() == uid,
            "existing user MCP neutral cwd leaf is unsafe"
        );
        std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(0o500))?;
    } else {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&leaf)
            .context("creating user MCP neutral cwd")?;
        std::fs::set_permissions(&leaf, std::fs::Permissions::from_mode(0o500))?;
    }
    let meta = std::fs::symlink_metadata(&leaf)?;
    ensure!(
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && meta.uid() == uid
            && meta.mode() & 0o777 == 0o500,
        "user MCP neutral cwd leaf verification failed"
    );
    Ok(leaf)
}

pub fn remove_neutral_cwd(path: &Path) {
    if let Ok(meta) = std::fs::symlink_metadata(path)
        && meta.is_dir()
        && !meta.file_type().is_symlink()
        && meta.uid() == rustix::process::geteuid().as_raw()
    {
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
        let _ = std::fs::remove_dir(path);
    }
}

pub fn error_reply(code: &'static str, message: &str) -> Value {
    json!({
        "error": {
            "code": -32000,
            "message": message,
            "data": { "code": code },
        }
    })
}

/// Validate and extract the Manager-injected `managerGate` object, returning
/// both the parsed gate and a clone of `params` with `managerGate` removed
/// (so ledger admission digests stay invariant across allowlist edits).
pub fn extract_manager_gate(params: &Value) -> Result<(ManagerGate, Value), Value> {
    let Some(object) = params.as_object() else {
        return Err(error_reply("user_mcp_invalid", "invalid user MCP params"));
    };
    let Some(gate_obj) = object.get("managerGate").and_then(Value::as_object) else {
        return Err(error_reply(
            "user_mcp_manager_gate_required",
            "user MCP manager gate is required",
        ));
    };
    let Some(enabled) = gate_obj.get("enabled").and_then(Value::as_bool) else {
        return Err(error_reply(
            "user_mcp_manager_gate_required",
            "user MCP manager gate enabled flag missing",
        ));
    };
    let allowlist = match gate_obj.get("allowlist") {
        None => Vec::new(),
        Some(Value::Array(items)) if items.len() <= 32 => {
            let mut list = Vec::with_capacity(items.len());
            for item in items {
                let Some(name) = item
                    .as_str()
                    .filter(|s| crate::config::valid_mcp_server_name(s))
                else {
                    return Err(error_reply(
                        "user_mcp_invalid",
                        "invalid server name in managerGate.allowlist",
                    ));
                };
                list.push(name.to_owned());
            }
            list
        }
        _ => {
            return Err(error_reply(
                "user_mcp_invalid",
                "invalid managerGate.allowlist",
            ));
        }
    };
    if !enabled {
        return Err(error_reply(
            "user_mcp_disabled",
            "user MCP servers disabled by device configuration",
        ));
    }
    let mut clean = object.clone();
    clean.remove("managerGate");
    Ok((ManagerGate { enabled, allowlist }, Value::Object(clean)))
}

/// Resolve and validate stdio user MCP servers from `$HOME/.claude.json`.
pub fn resolve_catalog(
    contract_env: &BTreeMap<String, String>,
    state_dir: &Path,
    source_path: &Path,
    cwd: &Path,
    allowlist: &[String],
    ephemeral_key: &[u8; 32],
) -> ResolvedCatalog {
    let clean_env = clean_base_environment(contract_env, state_dir);
    let Some(home) = clean_env.get("HOME").filter(|h| !h.is_empty()) else {
        return ResolvedCatalog::default();
    };
    let claude_json = Path::new(home).join(".claude.json");
    let raw = match read_claude_json(&claude_json) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return ResolvedCatalog::default(),
        Err(reason) => {
            return ResolvedCatalog {
                servers: Vec::new(),
                omitted: vec![OmittedServer {
                    name: ".claude.json".to_owned(),
                    reason,
                }],
            };
        }
    };
    let Ok(Value::Object(root)) = serde_json::from_slice::<Value>(&raw) else {
        return ResolvedCatalog {
            servers: Vec::new(),
            omitted: vec![OmittedServer {
                name: ".claude.json".to_owned(),
                reason: "invalid_config",
            }],
        };
    };

    let project_root = find_git_toplevel(source_path).unwrap_or_else(|| source_path.to_path_buf());
    let project_entry = match_project_entry(&root, &project_root, source_path);

    let disabled: HashSet<String> = project_entry
        .and_then(|obj| obj.get("disabledMcpServers"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    let mut merged: BTreeMap<String, (&'static str, &Map<String, Value>)> = BTreeMap::new();
    if let Some(user_servers) = root.get("mcpServers").and_then(Value::as_object) {
        for (name, val) in user_servers {
            if let Some(obj) = val.as_object() {
                merged.insert(name.clone(), ("user", obj));
            }
        }
    }
    if let Some(local_servers) = project_entry
        .and_then(|obj| obj.get("mcpServers"))
        .and_then(Value::as_object)
    {
        for (name, val) in local_servers {
            if let Some(obj) = val.as_object() {
                merged.insert(name.clone(), ("local", obj));
            }
        }
    }

    let allow_set: Option<HashSet<&str>> = if allowlist.is_empty() {
        None
    } else {
        Some(allowlist.iter().map(String::as_str).collect())
    };

    let forbidden = forbidden_prefixes(source_path, &project_root, cwd);
    let path_var = clean_env.get("PATH").map(String::as_str).unwrap_or("");

    let mut catalog = ResolvedCatalog::default();
    for (name, (scope, config)) in merged {
        if disabled.contains(&name) {
            continue;
        }
        if !valid_server_name(&name) {
            push_omitted(&mut catalog.omitted, name, "invalid_config");
            continue;
        }
        // Apply allowlist BEFORE the 16-server cap (Round 2 Medium 2).
        if let Some(allowed) = &allow_set
            && !allowed.contains(name.as_str())
        {
            push_omitted(&mut catalog.omitted, name, "not_allowed");
            continue;
        }
        if catalog.servers.len() >= MAX_USER_MCP_SERVERS {
            push_omitted(&mut catalog.omitted, name, "server_limit");
            continue;
        }
        match validate_server(
            &name,
            scope,
            config,
            &clean_env,
            path_var,
            &forbidden,
            ephemeral_key,
        ) {
            Ok(server) => catalog.servers.push(server),
            Err(reason) => push_omitted(&mut catalog.omitted, name, reason),
        }
    }
    catalog
}

fn push_omitted(omitted: &mut Vec<OmittedServer>, name: String, reason: &'static str) {
    if omitted.len() < MAX_OMITTED_NOTES {
        let safe_name: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            .take(64)
            .collect();
        omitted.push(OmittedServer {
            name: if safe_name.is_empty() {
                "invalid".to_owned()
            } else {
                safe_name
            },
            reason,
        });
    }
}

fn read_claude_json(path: &Path) -> Result<Option<Vec<u8>>, &'static str> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("invalid_config"),
    };
    let meta = file.metadata().map_err(|_| "invalid_config")?;
    if !meta.is_file() || meta.len() > MAX_CLAUDE_JSON_BYTES {
        return Err("invalid_config");
    }
    let mut bytes = Vec::new();
    file.take(MAX_CLAUDE_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "invalid_config")?;
    if bytes.len() as u64 > MAX_CLAUDE_JSON_BYTES {
        return Err("invalid_config");
    }
    Ok(Some(bytes))
}

fn clean_base_environment(
    contract_env: &BTreeMap<String, String>,
    state_dir: &Path,
) -> BTreeMap<String, String> {
    let canonical_state = std::fs::canonicalize(state_dir).ok();
    contract_env
        .iter()
        .filter(|(key, value)| {
            if key.to_ascii_uppercase().starts_with("COWBOY_") {
                return false;
            }
            if key.eq_ignore_ascii_case("CODEX_HOME") {
                let path = Path::new(value);
                if path.starts_with(state_dir) {
                    return false;
                }
                if let Some(canon) = &canonical_state
                    && std::fs::canonicalize(path)
                        .ok()
                        .is_some_and(|p| p.starts_with(canon))
                {
                    return false;
                }
            }
            // CLAUDE_PROJECT_DIR intentionally omitted (Round 3 Medium 1).
            if key.eq_ignore_ascii_case("CLAUDE_PROJECT_DIR") {
                return false;
            }
            true
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn find_git_toplevel(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    for _ in 0..MAX_GIT_WALK_DEPTH {
        if current.join(".git").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
    None
}

fn match_project_entry<'a>(
    root: &'a Map<String, Value>,
    project_root: &Path,
    source_path: &Path,
) -> Option<&'a Map<String, Value>> {
    let projects = root.get("projects")?.as_object()?;
    let canon_root = std::fs::canonicalize(project_root).ok();
    let canon_source = std::fs::canonicalize(source_path).ok();

    let mut source_match: Option<&'a Map<String, Value>> = None;
    for (key, value) in projects {
        let Some(entry) = value.as_object() else {
            continue;
        };
        let key_path = Path::new(key);
        let canon_key = std::fs::canonicalize(key_path).ok();
        if key_path == project_root
            || (canon_root.is_some() && canon_key.as_ref() == canon_root.as_ref())
        {
            return Some(entry);
        }
        if source_match.is_none()
            && (key_path == source_path
                || (canon_source.is_some() && canon_key.as_ref() == canon_source.as_ref()))
        {
            source_match = Some(entry);
        }
    }
    source_match
}

fn forbidden_prefixes(source_path: &Path, project_root: &Path, cwd: &Path) -> BTreeSet<PathBuf> {
    let mut set = BTreeSet::new();
    for path in [source_path, project_root, cwd] {
        if path.is_absolute() {
            set.insert(path.to_path_buf());
        }
        if let Ok(canonical) = std::fs::canonicalize(path) {
            set.insert(canonical);
        }
    }
    set
}

fn validate_server(
    name: &str,
    scope: &'static str,
    config: &Map<String, Value>,
    clean_env: &BTreeMap<String, String>,
    path_var: &str,
    forbidden: &BTreeSet<PathBuf>,
    ephemeral_key: &[u8; 32],
) -> Result<ResolvedServer, &'static str> {
    // Stdio only: absent `type` or `"stdio"`.
    if let Some(kind) = config.get("type").and_then(Value::as_str)
        && kind != "stdio"
    {
        return Err("unsupported_transport");
    }
    if config.get("type").is_some_and(|v| !v.is_string()) {
        return Err("unsupported_transport");
    }
    if config.contains_key("url") && !config.contains_key("command") {
        return Err("unsupported_transport");
    }

    let raw_command = config
        .get("command")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("invalid_config")?;
    let (command, unresolved_cmd) = expand_vars(raw_command, clean_env);
    if unresolved_cmd {
        return Err("unresolved_variable");
    }
    if command.is_empty() || command.contains('\0') {
        return Err("invalid_config");
    }
    // Reject relative commands containing `/` (e.g. `./server.js`, `../bin/mcp`).
    if !command.starts_with('/') && command.contains('/') {
        return Err("invalid_config");
    }

    let raw_args = match config.get("args") {
        None => Vec::new(),
        Some(Value::Array(items)) if items.len() <= MAX_ARGS => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let text = item.as_str().ok_or("invalid_config")?;
                out.push(text);
            }
            out
        }
        _ => return Err("invalid_config"),
    };
    let mut args = Vec::with_capacity(raw_args.len());
    for raw_arg in raw_args {
        let (expanded, unresolved) = expand_vars(raw_arg, clean_env);
        if unresolved {
            return Err("unresolved_variable");
        }
        if expanded.len() > MAX_ARG_BYTES || expanded.contains('\0') {
            return Err("invalid_config");
        }
        if arg_violates_repo_guardrail(&expanded, forbidden) {
            return Err("invalid_config");
        }
        args.push(expanded);
    }

    let mut server_env = BTreeMap::new();
    if let Some(env_val) = config.get("env") {
        let env_obj = env_val.as_object().ok_or("invalid_config")?;
        if env_obj.len() > MAX_ENV_ENTRIES {
            return Err("invalid_config");
        }
        for (key, val) in env_obj {
            if key.is_empty()
                || key.len() > 256
                || key.contains('=')
                || key.contains('\0')
                || key.to_ascii_uppercase().starts_with("COWBOY_")
                || key.eq_ignore_ascii_case("CLAUDE_PROJECT_DIR")
            {
                return Err("invalid_config");
            }
            let raw_val = val.as_str().ok_or("invalid_config")?;
            let (expanded, unresolved) = expand_vars(raw_val, clean_env);
            if unresolved {
                return Err("unresolved_variable");
            }
            if expanded.len() > MAX_ENV_VALUE_BYTES || expanded.contains('\0') {
                return Err("invalid_config");
            }
            server_env.insert(key.clone(), expanded);
        }
    }

    // Resolve logical executable preserving symlinks (`argv[0]` parity for
    // venv/bunx/uvx/Nix wrappers); canonicalize strictly for lexical check.
    let logical_executable = resolve_logical_executable(&command, path_var)?;
    let canonical_executable =
        std::fs::canonicalize(&logical_executable).map_err(|_| "invalid_config")?;
    if path_in_forbidden(&logical_executable, forbidden)
        || path_in_forbidden(&canonical_executable, forbidden)
    {
        return Err("invalid_config");
    }

    let mut child_env = clean_env.clone();
    for (k, v) in &server_env {
        child_env.insert(k.clone(), v.clone());
    }

    let canonical_payload = serde_json::to_vec(&json!({
        "name": name,
        "scope": scope,
        "executable": logical_executable.to_string_lossy(),
        "args": args,
        "env": server_env,
    }))
    .map_err(|_| "invalid_config")?;
    let fingerprint = hmac_sha256_hex(ephemeral_key, &canonical_payload);

    Ok(ResolvedServer {
        name: name.to_owned(),
        scope,
        fingerprint,
        logical_executable,
        args,
        child_env,
    })
}

/// Expands `${VAR}` and `${VAR:-default}` against `env`. Returns `(expanded,
/// had_unresolved)` so callers can omit servers whose `${VAR}` was absent from
/// `contract.environment` with closed reason `"unresolved_variable"`.
pub fn expand_vars(input: &str, env: &BTreeMap<String, String>) -> (String, bool) {
    let mut out = String::with_capacity(input.len());
    let mut unresolved = false;
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'{'
            && let Some(end_rel) = input[i + 2..].find('}')
        {
            let inner = &input[i + 2..i + 2 + end_rel];
            let (var_name, fallback) = match inner.split_once(":-") {
                Some((name, default)) => (name, Some(default)),
                None => (inner, None),
            };
            if valid_env_var_name(var_name) {
                if let Some(value) = env.get(var_name) {
                    out.push_str(value);
                } else if let Some(default) = fallback {
                    out.push_str(default);
                } else {
                    unresolved = true;
                    out.push_str(&input[i..=i + 2 + end_rel]);
                }
                i += 3 + end_rel;
                continue;
            }
        }
        let ch = input[i..].chars().next().expect("valid utf8 boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, unresolved)
}

fn valid_env_var_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn resolve_logical_executable(command: &str, path_var: &str) -> Result<PathBuf, &'static str> {
    if command.starts_with('/') {
        let candidate = PathBuf::from(command);
        if is_executable_file(&candidate) {
            return Ok(candidate);
        }
        return Err("invalid_config");
    }
    for dir in std::env::split_paths(path_var) {
        if !dir.is_absolute() {
            continue;
        }
        let candidate = dir.join(command);
        if is_executable_file(&candidate) {
            return Ok(candidate);
        }
    }
    Err("invalid_config")
}

fn is_executable_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && (meta.permissions().mode() & 0o111) != 0)
}

fn path_in_forbidden(path: &Path, forbidden: &BTreeSet<PathBuf>) -> bool {
    forbidden.iter().any(|prefix| path.starts_with(prefix))
}

fn arg_violates_repo_guardrail(arg: &str, forbidden: &BTreeSet<PathBuf>) -> bool {
    let candidates: [&str; 2] = match arg.split_once('=') {
        Some((_, rhs)) => [arg, rhs],
        None => [arg, ""],
    };
    for candidate in candidates {
        if candidate.is_empty() {
            continue;
        }
        if candidate == "."
            || candidate == ".."
            || candidate.starts_with("./")
            || candidate.starts_with("../")
        {
            return true;
        }
        let path = Path::new(candidate);
        if path.is_absolute() {
            if path_in_forbidden(path, forbidden) {
                return true;
            }
            if let Ok(canonical) = std::fs::canonicalize(path)
                && path_in_forbidden(&canonical, forbidden)
            {
                return true;
            }
        }
    }
    false
}

/// Build native `process/start` params for a validated user MCP server using
/// `envPolicy.inherit = "none"` (empirically verified on `codex 0.159.3`) and
/// the external read-only neutral cwd.
pub fn build_native_start_params(
    process_id: &str,
    server: &ResolvedServer,
    neutral_cwd: &Path,
) -> Result<Value, &'static str> {
    let cwd_url = url::Url::from_file_path(neutral_cwd)
        .map_err(|()| "invalid_config")?
        .to_string();
    let mut argv = Vec::with_capacity(1 + server.args.len());
    argv.push(server.logical_executable.to_string_lossy().into_owned());
    argv.extend(server.args.iter().cloned());
    let set: Map<String, Value> = server
        .child_env
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    Ok(json!({
        "processId": process_id,
        "argv": argv,
        "cwd": cwd_url,
        "env": {},
        "tty": false,
        "pipeStdin": true,
        "arg0": null,
        "envPolicy": {
            "inherit": "none",
            "ignoreDefaultExcludes": true,
            "exclude": [],
            "set": Value::Object(set),
            "includeOnly": [],
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    fn make_executable(path: &Path) {
        std::fs::write(path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231_vector_2_length_and_determinism() {
        let key = [0x42_u8; 32];
        let a = hmac_sha256_hex(&key, b"hello");
        let b = hmac_sha256_hex(&key, b"hello");
        let c = hmac_sha256_hex(&key, b"world");
        assert_eq!(a.len(), 64);
        assert_eq!(a, b);
        assert_ne!(a, c);
        // Verify against RFC 4231 test case 1 with 32-byte key (0x0b * 20 padded to 32):
        let mut rfc_key = [0_u8; 32];
        rfc_key[..20].fill(0x0b);
        assert_eq!(
            hmac_sha256_hex(&rfc_key, b"Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn resolves_user_and_local_via_git_toplevel_and_alias_ignoring_dot_mcp_json() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        let bin = dir.path().join("bin");
        let repo = dir.path().join("real-repo");
        let sub = repo.join("packages/app");
        let alias = dir.path().join("alias-repo");
        let worktree = dir.path().join("worktree");
        let state_dir = dir.path().join("keeper-state");
        for d in [&home, &bin, &sub, &worktree, &state_dir] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        symlink(&repo, &alias).unwrap();

        let real_tool = bin.join("real-tool");
        make_executable(&real_tool);
        let sym_tool = bin.join("venv-python");
        symlink(&real_tool, &sym_tool).unwrap();

        // Repo `.mcp.json` MUST be ignored.
        std::fs::write(
            repo.join(".mcp.json"),
            r#"{"mcpServers":{"evil-repo":{"command":"venv-python"}}}"#,
        )
        .unwrap();

        let claude = json!({
            "mcpServers": {
                "global-srv": { "command": "venv-python", "args": ["--flag", "${KEEP:-def}"], "env": { "LD_LIBRARY_PATH": "/opt/lib" } },
                "disabled-srv": { "command": "venv-python" },
                "remote-sse": { "type": "sse", "url": "https://example.test/sse" },
                "missing-var": { "command": "venv-python", "env": { "TOKEN": "${GITHUB_TOKEN}" } },
                "cowboy-spoof": { "command": "venv-python" },
                "bad-cowboy-env": { "command": "venv-python", "env": { "COWBOY_CALL_CONTEXT": "/x" } }
            },
            "projects": {
                alias.to_string_lossy().to_string(): {
                    "mcpServers": {
                        "local-srv": { "command": "venv-python", "args": ["--port", "9222"] }
                    },
                    "disabledMcpServers": ["disabled-srv"]
                }
            }
        });
        std::fs::write(home.join(".claude.json"), claude.to_string()).unwrap();

        let mut env = BTreeMap::new();
        env.insert("HOME".into(), home.to_string_lossy().into_owned());
        env.insert("PATH".into(), bin.to_string_lossy().into_owned());
        env.insert(
            "CODEX_HOME".into(),
            state_dir
                .join("executor-home")
                .to_string_lossy()
                .into_owned(),
        );
        env.insert("CLAUDE_PROJECT_DIR".into(), "/should-not-leak".into());

        let catalog = resolve_catalog(&env, &state_dir, &sub, &worktree, &[], &[7_u8; 32]);
        let names: Vec<&str> = catalog.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["global-srv", "local-srv"]);
        // Symlink argv[0] preserved (not realpath!):
        assert_eq!(catalog.servers[0].logical_executable, sym_tool);
        assert_eq!(catalog.servers[0].args, vec!["--flag", "def"]);
        assert_eq!(
            catalog.servers[0]
                .child_env
                .get("LD_LIBRARY_PATH")
                .map(String::as_str),
            Some("/opt/lib")
        );
        assert!(!catalog.servers[0].child_env.contains_key("CODEX_HOME"));
        assert!(
            !catalog.servers[0]
                .child_env
                .contains_key("CLAUDE_PROJECT_DIR")
        );

        let omitted: HashMap<&str, &str> = catalog
            .omitted
            .iter()
            .map(|o| (o.name.as_str(), o.reason))
            .collect();
        assert_eq!(omitted.get("remote-sse"), Some(&"unsupported_transport"));
        assert_eq!(omitted.get("missing-var"), Some(&"unresolved_variable"));
        assert_eq!(omitted.get("cowboy-spoof"), Some(&"invalid_config"));
        assert_eq!(omitted.get("bad-cowboy-env"), Some(&"invalid_config"));
        assert!(!omitted.contains_key("evil-repo"));
    }

    #[test]
    fn allowlist_applies_before_16_server_truncation_and_guardrails_block_repo_paths() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        let bin = dir.path().join("bin");
        let source = dir.path().join("source");
        let worktree = dir.path().join("worktree");
        let state_dir = dir.path().join("state");
        for d in [&home, &bin, &source, &worktree, &state_dir] {
            std::fs::create_dir_all(d).unwrap();
        }
        let ok_bin = bin.join("ok-mcp");
        make_executable(&ok_bin);
        let repo_bin = source.join("repo-mcp");
        make_executable(&repo_bin);
        let wt_bin = worktree.join("wt-mcp");
        make_executable(&wt_bin);

        let mut servers = Map::new();
        for i in 0..20 {
            servers.insert(
                format!("srv-{i:02}"),
                json!({ "command": ok_bin.to_string_lossy() }),
            );
        }
        servers.insert(
            "z_target_GitHub".into(),
            json!({ "command": ok_bin.to_string_lossy() }),
        );
        servers.insert("rel-cmd".into(), json!({ "command": "./server.js" }));
        servers.insert(
            "in-source".into(),
            json!({ "command": repo_bin.to_string_lossy() }),
        );
        servers.insert(
            "in-worktree".into(),
            json!({ "command": wt_bin.to_string_lossy() }),
        );
        servers.insert(
            "arg-rel".into(),
            json!({ "command": ok_bin.to_string_lossy(), "args": ["./local.js"] }),
        );
        servers.insert(
            "arg-source".into(),
            json!({ "command": ok_bin.to_string_lossy(), "args": [source.join("script.sh").to_string_lossy()] }),
        );
        servers.insert(
            "arg-wt".into(),
            json!({ "command": ok_bin.to_string_lossy(), "args": [format!("--cfg={}", worktree.join("x").display())] }),
        );
        std::fs::write(
            home.join(".claude.json"),
            json!({ "mcpServers": servers }).to_string(),
        )
        .unwrap();

        let mut env = BTreeMap::new();
        env.insert("HOME".into(), home.to_string_lossy().into_owned());
        env.insert("PATH".into(), bin.to_string_lossy().into_owned());

        // Allowlist selects entry #21 alphabetically (past the 16 cap).
        let allowed = resolve_catalog(
            &env,
            &state_dir,
            &source,
            &worktree,
            &["z_target_GitHub".to_owned()],
            &[9_u8; 32],
        );
        assert_eq!(allowed.servers.len(), 1);
        assert_eq!(allowed.servers[0].name, "z_target_GitHub");

        // All 6 guardrail violations are rejected.
        for bad in [
            "rel-cmd",
            "in-source",
            "in-worktree",
            "arg-rel",
            "arg-source",
            "arg-wt",
        ] {
            let res = resolve_catalog(
                &env,
                &state_dir,
                &source,
                &worktree,
                &[bad.to_owned()],
                &[9_u8; 32],
            );
            assert!(res.servers.is_empty(), "{bad} should be rejected");
            assert_eq!(
                res.omitted.iter().find(|o| o.name == bad).map(|o| o.reason),
                Some("invalid_config")
            );
        }
    }

    #[test]
    fn neutral_cwd_is_mode_0500_outside_keeper_state_and_cleaned_up() {
        let inc = format!("test-inc-{:016x}", rand::random::<u64>());
        let cwd = create_neutral_cwd(&inc).unwrap();
        let meta = std::fs::symlink_metadata(&cwd).unwrap();
        assert!(meta.is_dir() && !meta.file_type().is_symlink());
        assert_eq!(meta.mode() & 0o777, 0o500);
        assert!(cwd.starts_with(neutral_cwd_root()));
        remove_neutral_cwd(&cwd);
        assert!(!cwd.exists());
    }

    #[test]
    #[cfg(feature = "full")]
    fn keeper_and_worker_user_mcp_constants_match_byte_for_byte() {
        assert_eq!(
            USER_MCP_INITIALIZATION_KEY,
            cowboy::worker_execution::USER_MCP_INITIALIZATION_KEY
        );
        assert_eq!(
            USER_MCP_LIST_METHOD,
            cowboy::worker_execution::USER_MCP_LIST_METHOD
        );
        assert_eq!(
            USER_MCP_START_METHOD,
            cowboy::worker_execution::USER_MCP_START_METHOD
        );
        assert_eq!(
            USER_MCP_STOP_METHOD,
            cowboy::worker_execution::USER_MCP_STOP_METHOD
        );
        assert_eq!(
            USER_MCP_PROCESS_PREFIX,
            cowboy::worker_execution::USER_MCP_PROCESS_PREFIX
        );
    }
}
