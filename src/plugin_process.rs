//! Bounded execution for signed Plugin host commands.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

use anyhow::{Context as _, ensure};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::Command;

pub(crate) const MAX_PLUGIN_COMMAND_INPUT_BYTES: usize = 1024 * 1024;
const MAX_PLUGIN_COMMAND_STDOUT_BYTES: usize = 1024 * 1024;
const MAX_PLUGIN_COMMAND_STDERR_BYTES: usize = 64 * 1024;
const PLUGIN_COMMAND_TIMEOUT: Duration = Duration::from_secs(12);
static PLUGIN_COMMAND_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Own a freshly spawned process group across success, failure and future
/// cancellation. The leader must be spawned with `process_group(0)`.
pub(crate) struct PluginProcessGroup {
    process_id: u32,
    cgroup: Option<PathBuf>,
}

impl PluginProcessGroup {
    pub(crate) fn new(process_id: u32) -> Self {
        let owner = format!(
            "plugin-{}-{}",
            std::process::id(),
            PLUGIN_COMMAND_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let cgroup = crate::cgroup::create(&owner);
        if let Some(directory) = &cgroup {
            crate::cgroup::add_pid(directory, process_id);
        }
        Self { process_id, cgroup }
    }
}

impl Drop for PluginProcessGroup {
    fn drop(&mut self) {
        if let Some(directory) = &self.cgroup {
            crate::cgroup::kill_and_remove(directory);
        }
        #[cfg(unix)]
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &format!("-{}", self.process_id)])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[derive(Debug)]
pub(crate) struct PluginCommandOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct PluginCommandFailure {
    pub(crate) started: bool,
    #[allow(dead_code)]
    pub(crate) timed_out: bool,
    pub(crate) error: anyhow::Error,
}

#[cfg(feature = "full")]
pub(crate) async fn run_plugin_command(
    program: &str,
    args: &[String],
    input: &[u8],
) -> Result<PluginCommandOutput, PluginCommandFailure> {
    run_plugin_command_with_timeout(program, args, input, PLUGIN_COMMAND_TIMEOUT).await
}

/// Run a signed Plugin command with a closed, caller-supplied environment.
/// No ambient Machine or Controller variable is inherited.
#[cfg(feature = "machine-host")]
pub(crate) async fn run_plugin_command_with_environment(
    program: &str,
    args: &[String],
    input: &[u8],
    environment: &BTreeMap<String, String>,
) -> Result<PluginCommandOutput, PluginCommandFailure> {
    run_plugin_command_with_timeout_and_environment(
        program,
        args,
        input,
        PLUGIN_COMMAND_TIMEOUT,
        Some(environment),
    )
    .await
}

#[cfg(any(feature = "full", test))]
async fn run_plugin_command_with_timeout(
    program: &str,
    args: &[String],
    input: &[u8],
    timeout: Duration,
) -> Result<PluginCommandOutput, PluginCommandFailure> {
    run_plugin_command_with_timeout_and_environment(program, args, input, timeout, None).await
}

async fn run_plugin_command_with_timeout_and_environment(
    program: &str,
    args: &[String],
    input: &[u8],
    timeout: Duration,
    environment: Option<&BTreeMap<String, String>>,
) -> Result<PluginCommandOutput, PluginCommandFailure> {
    if input.len() > MAX_PLUGIN_COMMAND_INPUT_BYTES {
        return Err(PluginCommandFailure {
            started: false,
            timed_out: false,
            error: anyhow::anyhow!("plugin command request is too large"),
        });
    }
    let mut command = plugin_command(program);
    if let Err(error) = configure_plugin_command(&mut command, program, args, environment) {
        return Err(PluginCommandFailure {
            started: false,
            timed_out: false,
            error,
        });
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.as_std_mut().process_group(0);
    let mut child = command.spawn().map_err(|error| PluginCommandFailure {
        started: false,
        timed_out: false,
        error: anyhow::Error::new(error).context("spawn plugin command"),
    })?;
    let _process_group = child.id().map(PluginProcessGroup::new);
    let run = async move {
        let mut stdin = child.stdin.take().context("plugin command stdin")?;
        let stdout = child.stdout.take().context("plugin command stdout")?;
        let stderr = child.stderr.take().context("plugin command stderr")?;
        let write_input = async move {
            stdin
                .write_all(input)
                .await
                .context("write plugin command request")?;
            stdin.shutdown().await.context("close plugin command stdin")
        };
        let wait = async move { child.wait().await.context("wait for plugin command") };
        let ((), stdout, stderr, status) = tokio::try_join!(
            write_input,
            read_bounded(
                stdout,
                MAX_PLUGIN_COMMAND_STDOUT_BYTES,
                "plugin command stdout",
            ),
            read_bounded(
                stderr,
                MAX_PLUGIN_COMMAND_STDERR_BYTES,
                "plugin command stderr",
            ),
            wait,
        )?;
        Ok(PluginCommandOutput {
            status,
            stdout,
            stderr,
        })
    };
    match tokio::time::timeout(timeout, run).await {
        Ok(result) => result.map_err(|error| PluginCommandFailure {
            started: true,
            timed_out: false,
            error,
        }),
        Err(error) => Err(PluginCommandFailure {
            started: true,
            timed_out: true,
            error: anyhow::Error::new(error).context("plugin command timed out"),
        }),
    }
}

fn plugin_command(program: &str) -> Command {
    if program != "@plugin-js" {
        return Command::new(program);
    }
    let installed = std::env::current_exe()
        .ok()
        .and_then(|executable| {
            executable
                .parent()
                .map(|parent| parent.join("cowboy-plugin-js"))
        })
        .filter(|candidate| candidate.is_file());
    Command::new(installed.unwrap_or_else(|| PathBuf::from("deno")))
}

fn configure_plugin_command(
    command: &mut Command,
    program: &str,
    args: &[String],
    environment: Option<&BTreeMap<String, String>>,
) -> anyhow::Result<()> {
    if program != "@plugin-js" {
        command.args(args);
        if let Some(environment) = environment {
            command.env_clear().envs(environment);
        }
        return Ok(());
    }
    let normalized = if let Some(environment) = environment {
        plugin_js_arguments_with_env(args, |name| environment.get(name).cloned())?
    } else {
        plugin_js_arguments(args)?
    };
    command.args(normalized);
    command.env_clear();
    for name in plugin_environment_names(args)? {
        if let Some(value) = environment
            .and_then(|environment| environment.get(&name).cloned())
            .or_else(|| {
                environment
                    .is_none()
                    .then(|| std::env::var(&name).ok())
                    .flatten()
            })
        {
            command.env(name, value);
        }
    }
    if let Some(parent) = args
        .iter()
        .find(|argument| {
            Path::new(argument)
                .extension()
                .is_some_and(|value| value == "js")
        })
        .and_then(|entry| Path::new(entry).parent())
    {
        command.current_dir(parent);
    }
    Ok(())
}

fn plugin_js_arguments(args: &[String]) -> anyhow::Result<Vec<String>> {
    plugin_js_arguments_with_env(args, |name| std::env::var(name).ok())
}

fn plugin_js_arguments_with_env(
    args: &[String],
    get_env: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Vec<String>> {
    ensure!(
        args.first().is_some_and(|argument| argument == "run"),
        "Plugin JavaScript command must use run"
    );
    const HOST_FIXED_OPTIONS: &[&str] = &[
        "--quiet",
        "--no-prompt",
        "--no-config",
        "--no-remote",
        "--no-npm",
    ];
    let mut normalized = vec!["run".to_owned()];
    normalized.extend(HOST_FIXED_OPTIONS.iter().map(|value| (*value).to_owned()));
    for argument in &args[1..] {
        if HOST_FIXED_OPTIONS.contains(&argument.as_str()) {
            continue;
        }
        let permission = [
            (
                "--allow-read=",
                crate::plugin_host::PluginProcessGrantKind::Read,
            ),
            (
                "--allow-run=",
                crate::plugin_host::PluginProcessGrantKind::Run,
            ),
            (
                "--allow-net=",
                crate::plugin_host::PluginProcessGrantKind::Net,
            ),
        ]
        .into_iter()
        .find_map(|(prefix, kind)| {
            argument
                .strip_prefix(prefix)
                .map(|values| (prefix, kind, values))
        });
        if let Some((prefix, kind, values)) = permission {
            let values = expand_plugin_permission_values(kind, values, &get_env)?;
            if !values.is_empty() {
                normalized.push(format!("{prefix}{}", values.join(",")));
            }
        } else {
            normalized.push(argument.clone());
        }
    }
    Ok(normalized)
}

fn expand_plugin_permission_values(
    kind: crate::plugin_host::PluginProcessGrantKind,
    values: &str,
    get_env: &impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Vec<String>> {
    use crate::plugin_host::PluginProcessGrant;

    let mut expanded = BTreeSet::new();
    for value in values.split(',') {
        match crate::plugin_host::parse_plugin_process_grant(value, kind)? {
            PluginProcessGrant::Literal(value) => {
                expanded.insert(value.to_owned());
            }
            PluginProcessGrant::Environment { name, suffix } => {
                let Some(value) = get_env(name) else {
                    continue;
                };
                let value = format!("{value}{suffix}");
                validate_dynamic_permission_value(kind, &value, name)?;
                expanded.insert(value);
            }
            PluginProcessGrant::EnvironmentUrlHosts { name } => {
                let Some(value) = get_env(name) else {
                    continue;
                };
                for configured in value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    let url = url::Url::parse(configured)
                        .with_context(|| format!("Plugin network URL in {name} is invalid"))?;
                    ensure!(
                        matches!(url.scheme(), "http" | "https")
                            && url.username().is_empty()
                            && url.password().is_none(),
                        "Plugin network URL in {name} is not an HTTP origin"
                    );
                    let host = url
                        .host_str()
                        .with_context(|| format!("Plugin network URL in {name} has no host"))?;
                    let host = if host.contains(':') {
                        format!("[{host}]")
                    } else {
                        host.to_owned()
                    };
                    let port = url.port_or_known_default().with_context(|| {
                        format!("Plugin network URL in {name} has no known port")
                    })?;
                    expanded.insert(format!("{host}:{port}"));
                }
            }
        }
    }
    Ok(expanded.into_iter().collect())
}

fn validate_dynamic_permission_value(
    kind: crate::plugin_host::PluginProcessGrantKind,
    value: &str,
    environment_name: &str,
) -> anyhow::Result<()> {
    ensure!(
        (1..=4096).contains(&value.len()) && !value.contains(['\0', ',']),
        "Plugin permission value from {environment_name} is invalid"
    );
    match kind {
        crate::plugin_host::PluginProcessGrantKind::Read => ensure!(
            Path::new(value).is_absolute()
                && value != "/"
                && !Path::new(value)
                    .components()
                    .any(|component| component == std::path::Component::ParentDir),
            "Plugin read path from {environment_name} is not bounded"
        ),
        crate::plugin_host::PluginProcessGrantKind::Run => ensure!(
            !value.bytes().any(|byte| byte.is_ascii_whitespace())
                && (Path::new(value).is_absolute()
                    || value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                    })),
            "Plugin executable from {environment_name} is invalid"
        ),
        crate::plugin_host::PluginProcessGrantKind::Net => {
            anyhow::bail!("raw environment values cannot grant Plugin network access")
        }
    }
    Ok(())
}

fn plugin_environment_names(args: &[String]) -> anyhow::Result<BTreeSet<String>> {
    const BASELINE: &[&str] = &[
        "HOME",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "LOGNAME",
        "NIX_SSL_CERT_FILE",
        "PATH",
        "SHELL",
        "SSL_CERT_DIR",
        "SSL_CERT_FILE",
        "TMPDIR",
        "USER",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
    ];
    let mut names = BASELINE
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    for argument in args {
        if argument == "--allow-env" {
            anyhow::bail!("Plugin JavaScript environment permission must be scoped");
        }
        let Some(grants) = argument.strip_prefix("--allow-env=") else {
            continue;
        };
        for name in grants.split(',') {
            ensure!(
                !name.is_empty(),
                "Plugin JavaScript environment grant is empty"
            );
            names.insert(name.to_owned());
        }
    }
    Ok(names)
}

async fn read_bounded(
    reader: impl AsyncRead + Unpin,
    limit: usize,
    label: &'static str,
) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(u64::try_from(limit + 1).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .await
        .with_context(|| format!("read {label}"))?;
    ensure!(bytes.len() <= limit, "{label} is too large");
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn command_output_is_bounded_before_the_child_exits() {
        let args = [
            "-c".to_owned(),
            (MAX_PLUGIN_COMMAND_STDOUT_BYTES + 1).to_string(),
            "/dev/zero".to_owned(),
        ];
        let failure = run_plugin_command_with_timeout("head", &args, b"", Duration::from_secs(2))
            .await
            .expect_err("oversized output must fail closed");
        assert!(failure.started);
        assert!(failure.error.to_string().contains("stdout is too large"));
    }

    #[tokio::test]
    async fn command_timeout_reports_that_execution_started() {
        let args = ["10".to_owned()];
        let failure =
            run_plugin_command_with_timeout("sleep", &args, b"", Duration::from_millis(20))
                .await
                .expect_err("hung command must time out");
        assert!(failure.started);
        assert!(failure.timed_out);
        assert!(failure.error.to_string().contains("timed out"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn command_completion_reaps_process_group_descendants() {
        let args = [
            "-c".to_owned(),
            "trap '' HUP; sleep 60 </dev/null >/dev/null 2>&1 & printf '%s\\n' \"$!\"".to_owned(),
        ];
        let output = run_plugin_command_with_timeout("sh", &args, b"", Duration::from_secs(2))
            .await
            .expect("plugin command fixture");
        assert!(output.status.success());
        let descendant = String::from_utf8(output.stdout).unwrap();
        let descendant = descendant.trim();
        for _ in 0..50 {
            let status = Command::new("kill")
                .arg("-0")
                .arg(descendant)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .unwrap();
            if !status.success() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("plugin command descendant {descendant} survived cleanup");
    }

    #[test]
    fn plugin_javascript_runtime_is_hermetic_by_default() {
        let args = vec![
            "run".to_owned(),
            "--quiet".to_owned(),
            "--allow-env=PLUGIN_TOKEN".to_owned(),
            "/srv/plugin/collector/index.js".to_owned(),
        ];
        let normalized = plugin_js_arguments(&args).unwrap();
        for option in ["--no-config", "--no-remote", "--no-npm", "--no-prompt"] {
            assert!(normalized.iter().any(|argument| argument == option));
        }
        assert_eq!(
            normalized
                .iter()
                .filter(|argument| argument.as_str() == "--quiet")
                .count(),
            1
        );
        let environment = plugin_environment_names(&args).unwrap();
        assert!(environment.contains("PLUGIN_TOKEN"));
        assert!(environment.contains("PATH"));
        assert!(!environment.contains("DATABASE_URL"));
        assert!(plugin_environment_names(&["--allow-env".to_owned()]).is_err());
    }

    #[test]
    fn plugin_javascript_permissions_expand_only_typed_environment_bindings() {
        let args = vec![
            "run".to_owned(),
            "--allow-env=PLUGIN_HOME,PLUGIN_COMMAND,PLUGIN_URLS".to_owned(),
            "--allow-read=${ENV:PLUGIN_HOME}/auth.json".to_owned(),
            "--allow-run=grok,${ENV:PLUGIN_COMMAND}".to_owned(),
            "--allow-net=${ENV_URL_HOSTS:PLUGIN_URLS}".to_owned(),
            "/srv/plugin/collector/index.js".to_owned(),
        ];
        let normalized = plugin_js_arguments_with_env(&args, |name| match name {
            "PLUGIN_HOME" => Some("/srv/account".to_owned()),
            "PLUGIN_COMMAND" => Some("/opt/grok".to_owned()),
            "PLUGIN_URLS" => {
                Some("https://api.example.test/v1,http://127.0.0.1:9000/account".to_owned())
            }
            _ => None,
        })
        .unwrap();
        assert!(
            normalized
                .iter()
                .any(|value| value == "--allow-read=/srv/account/auth.json")
        );
        assert!(
            normalized
                .iter()
                .any(|value| value == "--allow-run=/opt/grok,grok")
        );
        assert!(
            normalized
                .iter()
                .any(|value| { value == "--allow-net=127.0.0.1:9000,api.example.test:443" })
        );
        assert!(normalized.iter().all(|value| !value.contains("${ENV")));
    }
}
