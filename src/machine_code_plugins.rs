//! Machine-owned code runtimes. A worktree keeps its exact generation until
//! its last buffer/worktree lease closes; other worktrees can use a newer
//! generation concurrently. Legacy sockets participate in the same routing.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Weak};
use std::time::Duration;

use crate::plugin_process::PluginProcessGroup;
use anyhow::{Context as _, Result, bail, ensure};
use cowboy_plugin_sdk::{CodeIntelligenceRuntime, CodeRuntimeArgument, CodeRuntimeCommand};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

mod buffer_leases;
mod buffer_navigation;
mod buffer_sync;
#[cfg(test)]
mod navigation_fixture;
#[cfg(test)]
mod sync_fixture;

const MAX_CODE_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_WORKTREE_ROUTES: usize = 4096;

#[derive(Clone)]
pub(crate) struct CodeLaunchPlan {
    pub plugin_id: String,
    pub generation_digest: String,
    pub runtime: CodeIntelligenceRuntime,
    pub commands: BTreeMap<String, PathBuf>,
    pub home: PathBuf,
}

pub(crate) enum CodeRuntimeSelection {
    Installed(CodeLaunchPlan),
    Legacy(PathBuf),
}

enum CodeTarget {
    Installed(Arc<RunningCodeRuntime>),
    Legacy(PathBuf),
}

impl CodeTarget {
    fn socket(&self) -> &Path {
        match self {
            Self::Installed(runtime) => &runtime.socket,
            Self::Legacy(socket) => socket,
        }
    }
}

#[derive(Default)]
struct WorktreeRoute {
    target: Option<CodeTarget>,
    worktree_leases: u64,
    buffers: BTreeSet<(String, String)>,
    owned_buffers: BTreeSet<buffer_leases::LeaseRef>,
    owned_navigations: BTreeSet<crate::machine_protocol::code_buffer_navigation::NavigationRef>,
}

impl WorktreeRoute {
    fn observe(&mut self, request: &Value, response: &Value) -> Result<bool> {
        match request.get("type").and_then(Value::as_str) {
            Some("openWorktree" | "closeWorktree" | "ensureWorktree") => {
                self.worktree_leases = response
                    .get("leases")
                    .and_then(Value::as_u64)
                    .context("code runtime returned invalid worktree leases")?;
            }
            Some("openBuffer" | "closeBuffer") => {
                let key = (
                    request["path"]
                        .as_str()
                        .context("buffer path is missing")?
                        .to_owned(),
                    request["leaseId"]
                        .as_str()
                        .context("buffer lease is missing")?
                        .to_owned(),
                );
                if request["type"] == "openBuffer" {
                    self.buffers.insert(key);
                } else {
                    self.buffers.remove(&key);
                }
            }
            _ => {}
        }
        Ok(matches!(
            request["type"].as_str(),
            Some("closeBuffer" | "closeWorktree")
        ) && self.is_idle())
    }

    fn is_idle(&self) -> bool {
        self.worktree_leases == 0
            && self.buffers.is_empty()
            && self.owned_buffers.is_empty()
            && self.owned_navigations.is_empty()
    }
}

#[derive(Default)]
pub(crate) struct CodeRuntimeHost {
    routes: Mutex<WorktreeRoutes>,
    engines: Mutex<BTreeMap<(String, String), Weak<RunningCodeRuntime>>>,
    buffer_leases: buffer_leases::Routes,
    buffer_navigation: buffer_navigation::Operations,
    buffer_sync: buffer_sync::Operations,
}

type WorktreeRoutes = BTreeMap<(String, PathBuf), Arc<Mutex<WorktreeRoute>>>;

impl CodeRuntimeHost {
    pub(crate) async fn navigate(
        &self,
        invocation: crate::machine_plugins::CodeNavigationInvocation,
    ) -> Result<crate::machine_protocol::code_buffer_navigation::Snapshot> {
        self.buffer_navigation
            .execute(invocation, &self.buffer_leases)
            .await
    }
    pub(crate) async fn synchronize(
        &self,
        invocation: crate::machine_plugins::CodeBufferSyncInvocation,
    ) -> Result<crate::machine_protocol::code_buffer_sync::Snapshot> {
        self.buffer_sync
            .execute(invocation, &self.buffer_leases)
            .await
    }
    #[cfg(test)]
    pub async fn live_generation_count(&self) -> usize {
        self.engines
            .lock()
            .await
            .values()
            .filter(|engine| engine.strong_count() > 0)
            .count()
    }

    pub async fn request(
        &self,
        plugin_id: &str,
        payload: &Value,
        select: impl FnOnce() -> Result<CodeRuntimeSelection>,
    ) -> Result<Value> {
        // Private synchronization is never a generic Code RPC. Only the
        // separate connection-issued core invocation can enter that executor.
        ensure!(
            !matches!(
                payload["type"].as_str(),
                Some(
                    "prepareBufferSync"
                        | "bufferSync"
                        | "prepareBufferNavigation"
                        | "bufferNavigation"
                        | "readBufferNavigation"
                        | "prepareNavigationBuffer"
                )
            ),
            "private buffer operations require separate core authority"
        );
        self.buffer_sync.expire_inert();
        self.buffer_navigation.expire_inert();
        let reservation = match buffer_leases::Command::parse(payload)? {
            Some(command) if command.is_prepare() => Some(self.buffer_leases.reserve().await?),
            Some(command) => {
                return self
                    .buffer_leases
                    .request(plugin_id, command, payload)
                    .await;
            }
            None => None,
        };
        let Some(worktree) = request_worktree(payload)? else {
            let target = self.target(select()?).await?;
            return exchange(target.socket(), payload).await;
        };
        let route = {
            let mut routes = self.routes.lock().await;
            // Only remove empty routes with no queued caller. A close and a
            // simultaneous reopen must serialize on the same worktree lock.
            routes.retain(|_, route| {
                Arc::strong_count(route) > 1
                    || route
                        .try_lock()
                        .map_or(true, |route| route.target.is_some())
            });
            let key = (plugin_id.to_owned(), worktree);
            ensure!(
                routes.contains_key(&key) || routes.len() < MAX_WORKTREE_ROUTES,
                "too many active code worktrees; close an existing worktree first"
            );
            Arc::clone(routes.entry(key).or_default())
        };
        let retained_route = Arc::clone(&route);
        let mut route = route.lock().await;
        if route.target.is_none() {
            route.target = Some(self.target(select()?).await?);
        }
        let target = route
            .target
            .as_ref()
            .context("code runtime route disappeared")?;
        if reservation.is_some() && matches!(target, CodeTarget::Legacy(_)) {
            bail!("owned buffer leases require an installed Code Plugin generation");
        }
        if let CodeTarget::Installed(runtime) = target
            && !runtime.is_running()?
        {
            // Runtime death loses its protocol state. Report the loss; never
            // replay a stateful request against a replacement or legacy CLI.
            *route = WorktreeRoute::default();
            bail!("code runtime exited; reopen the worktree before retrying");
        }
        if let Some(reservation) = reservation {
            return self
                .buffer_leases
                .prepare(plugin_id, reservation, retained_route, &mut route, payload)
                .await;
        }
        let response = exchange(target.socket(), payload).await?;
        if route.observe(payload, &response)? {
            *route = WorktreeRoute::default();
        }
        Ok(response)
    }

    async fn target(&self, selection: CodeRuntimeSelection) -> Result<CodeTarget> {
        match selection {
            CodeRuntimeSelection::Legacy(socket) => Ok(CodeTarget::Legacy(socket)),
            CodeRuntimeSelection::Installed(plan) => {
                let mut engines = self.engines.lock().await;
                engines.retain(|_, engine| engine.strong_count() > 0);
                let key = (plan.plugin_id.clone(), plan.generation_digest.clone());
                if let Some(engine) = engines.get(&key).and_then(Weak::upgrade) {
                    return Ok(CodeTarget::Installed(engine));
                }
                let engine = Arc::new(RunningCodeRuntime::start(&plan).await?);
                engines.insert(key, Arc::downgrade(&engine));
                Ok(CodeTarget::Installed(engine))
            }
        }
    }
}

fn request_worktree(payload: &Value) -> Result<Option<PathBuf>> {
    let kind = payload
        .get("type")
        .and_then(Value::as_str)
        .context("code request type is missing")?;
    if kind == "health" {
        return Ok(None);
    }
    let field = if matches!(kind, "ensureWorktree" | "openWorktree" | "closeWorktree") {
        "path"
    } else {
        "worktree"
    };
    let path = payload
        .get(field)
        .and_then(Value::as_str)
        .context("code request worktree is missing")?;
    ensure!(
        Path::new(path).is_absolute(),
        "code request worktree must be absolute"
    );
    Ok(Some(
        Path::new(path)
            .canonicalize()
            .context("canonicalizing code request worktree")?,
    ))
}

/// The package readiness command checks the real adapter/server pair before
/// the active generation link moves. A successful probe does not install it.
pub(crate) async fn probe_code_runtime(plan: &CodeLaunchPlan) -> Result<()> {
    let runtime = RunningCodeRuntime::start(plan).await?;
    runtime.stop().await;
    Ok(())
}

struct PrivateRuntimeDirectory(PathBuf);

impl PrivateRuntimeDirectory {
    fn create() -> Result<Self> {
        // Unix socket paths are small (104 bytes on Darwin). The random,
        // exclusively-created 0700 directory is process-owned, not a shared
        // mutable /tmp slot or a user-selected executable location.
        let path = PathBuf::from(format!("/tmp/cw-code-{:032x}", rand::random::<u128>()));
        DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
}

impl Drop for PrivateRuntimeDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct RunningCodeRuntime {
    child: parking_lot::Mutex<Child>,
    process_group: parking_lot::Mutex<Option<PluginProcessGroup>>,
    socket: PathBuf,
    _directory: PrivateRuntimeDirectory,
}

impl RunningCodeRuntime {
    async fn start(plan: &CodeLaunchPlan) -> Result<Self> {
        let directory = PrivateRuntimeDirectory::create()?;
        let socket = directory.0.join("adapter.sock");
        let state_dir = directory.0.join("state");
        for path in [&plan.home, &state_dir] {
            fs::create_dir_all(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        let mut command = runtime_command(plan, &plan.runtime.launch, &socket, &state_dir)?;
        command.as_std_mut().process_group(0);
        let child = command.spawn().context("starting installed code runtime")?;
        let process_id = child.id().context("code runtime has no process id")?;
        let runtime = Self {
            child: parking_lot::Mutex::new(child),
            process_group: parking_lot::Mutex::new(Some(PluginProcessGroup::new(process_id))),
            socket,
            _directory: directory,
        };
        let mut probe =
            runtime_command(plan, &plan.runtime.readiness, &runtime.socket, &state_dir)?;
        probe.as_std_mut().process_group(0);
        let mut probe = probe
            .spawn()
            .context("starting code runtime readiness probe")?;
        let _probe_group = probe.id().map(PluginProcessGroup::new);
        let result = tokio::time::timeout(
            Duration::from_millis(plan.runtime.readiness_timeout_ms),
            probe.wait(),
        )
        .await;
        let _ = probe.kill().await;
        let _ = probe.wait().await;
        ensure!(
            result
                .context("code runtime readiness timed out")??
                .success(),
            "installed code runtime failed readiness"
        );
        ensure!(
            runtime.is_running()?,
            "code runtime exited during readiness"
        );
        Ok(runtime)
    }

    async fn stop(mut self) {
        self.kill_group();
        // Move no process handle into an unowned background task. The group
        // has been killed; wait for the leader before its runtime dir drops.
        let _ = self.child.get_mut().wait().await;
    }

    fn is_running(&self) -> Result<bool> {
        if self.child.lock().try_wait()?.is_none() {
            return Ok(true);
        }
        // Retained unknown lease evidence must not keep an exited adapter's
        // descendants alive. Consume only this runtime's original group owner,
        // once; later observations and final Drop cannot signal it again.
        self.process_group.lock().take();
        Ok(false)
    }

    fn kill_group(&mut self) {
        self.process_group.get_mut().take();
    }
}

impl Drop for RunningCodeRuntime {
    fn drop(&mut self) {
        self.kill_group();
        let _ = self.child.get_mut().start_kill();
    }
}

fn runtime_command(
    plan: &CodeLaunchPlan,
    invocation: &CodeRuntimeCommand,
    socket: &Path,
    state_dir: &Path,
) -> Result<Command> {
    let executable = plan
        .commands
        .get(&invocation.command)
        .context("undeclared installed code command")?;
    let mut command = Command::new(executable);
    for argument in &invocation.arguments {
        match argument {
            CodeRuntimeArgument::Literal { value } => {
                command.arg(value);
            }
            CodeRuntimeArgument::ComponentCommand { command: name } => {
                command.arg(
                    plan.commands
                        .get(name)
                        .context("missing installed code component")?,
                );
            }
            CodeRuntimeArgument::Socket => {
                command.arg(socket);
            }
            CodeRuntimeArgument::StateDirectory => {
                command.arg(state_dir);
            }
        }
    }
    command
        .env_clear()
        .env("HOME", &plan.home)
        .env("TMPDIR", state_dir)
        .env("XDG_CONFIG_HOME", plan.home.join(".config"))
        .env("XDG_CACHE_HOME", plan.home.join(".cache"))
        .env("XDG_DATA_HOME", plan.home.join(".local/share"))
        .env("XDG_STATE_HOME", plan.home.join(".local/state"))
        .current_dir(&plan.home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // Host tools (Git, project language servers) are allowed in a trusted
    // workspace; the Plugin's own adapter/server are always absolute bindings.
    for name in ["PATH", "SSL_CERT_FILE", "SSL_CERT_DIR", "NIX_SSL_CERT_FILE"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    Ok(command)
}

pub(crate) async fn exchange(socket: &Path, payload: &Value) -> Result<Value> {
    let request = serde_json::to_vec(payload)?;
    ensure!(
        request.len() <= MAX_CODE_MESSAGE_BYTES,
        "code request exceeds its size limit"
    );
    let response = tokio::time::timeout(Duration::from_secs(35), async {
        let stream = tokio::time::timeout(Duration::from_secs(2), UnixStream::connect(socket))
            .await
            .context("code runtime connect timed out")??;
        let (read, mut write) = stream.into_split();
        write.write_all(&request).await?;
        write.write_all(b"\n").await?;
        write.shutdown().await?;
        let mut bytes = Vec::new();
        BufReader::new(read.take((MAX_CODE_MESSAGE_BYTES + 1) as u64))
            .read_until(b'\n', &mut bytes)
            .await?;
        ensure!(
            bytes.len() <= MAX_CODE_MESSAGE_BYTES && bytes.last() == Some(&b'\n'),
            "code runtime response is truncated or exceeds its size limit"
        );
        let response: Value = serde_json::from_slice(&bytes)?;
        ensure!(
            response.get("ok").and_then(Value::as_bool) != Some(false)
                && response.get("type").and_then(Value::as_str) != Some("error"),
            "code runtime rejected request: {}",
            response
                .get("message")
                .or_else(|| response.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("unspecified error")
        );
        Ok::<_, anyhow::Error>(response.get("value").cloned().unwrap_or(response))
    })
    .await
    .context("code runtime response timed out")??;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufRead as _, Write as _};

    #[tokio::test]
    async fn private_sync_cannot_use_generic_rpc_or_a_read_lease_as_authority() {
        let host = CodeRuntimeHost::default();
        for kind in [
            "prepareBufferSync",
            "bufferSync",
            "prepareBufferNavigation",
            "bufferNavigation",
            "readBufferNavigation",
            "prepareNavigationBuffer",
        ] {
            for extra_path in [false, true] {
                let mut request = json!({"type":kind,"purpose":"refresh_from_disk",
                    "lease":{"instance":"a".repeat(32),"id":"0000000000000001"}});
                if extra_path {
                    request["worktree"] = json!("/tmp");
                }
                let error = host
                    .request("zed", &request, || panic!("resolved a replacement runtime"))
                    .await
                    .unwrap_err();
                assert!(error.to_string().contains("separate core authority"));
                assert!(host.routes.lock().await.is_empty());
                assert_eq!(host.live_generation_count().await, 0);
            }
        }
    }

    pub(super) fn fixture_plan(root: &Path, generation: &str) -> CodeLaunchPlan {
        let arguments = |phase: &str| {
            vec![
                CodeRuntimeArgument::Literal {
                    value: "--ignored".to_owned(),
                },
                CodeRuntimeArgument::Literal {
                    value: "--exact".to_owned(),
                },
                CodeRuntimeArgument::Literal {
                    value: "machine_code_plugins::tests::runtime_fixture_child".to_owned(),
                },
                CodeRuntimeArgument::Literal {
                    value: format!("fixture-{phase}"),
                },
                CodeRuntimeArgument::Socket,
                CodeRuntimeArgument::StateDirectory,
                CodeRuntimeArgument::Literal {
                    value: format!("generation-{generation}"),
                },
            ]
        };
        CodeLaunchPlan {
            plugin_id: "fixture-code".to_owned(),
            generation_digest: generation.to_owned(),
            commands: BTreeMap::from([("fixture".to_owned(), std::env::current_exe().unwrap())]),
            home: root.join(generation),
            runtime: CodeIntelligenceRuntime {
                components: vec![],
                launch: CodeRuntimeCommand {
                    command: "fixture".to_owned(),
                    arguments: arguments("serve"),
                },
                readiness: CodeRuntimeCommand {
                    command: "fixture".to_owned(),
                    arguments: arguments("probe"),
                },
                readiness_timeout_ms: 5_000,
            },
        }
    }

    // Use this test binary as a credential-free adapter fixture. It is never
    // entered by an ordinary test run; only an exact subprocess invocation.
    #[test]
    #[ignore = "owned code runtime subprocess fixture"]
    fn runtime_fixture_child() {
        let args: Vec<_> = std::env::args().collect();
        let socket = args
            .iter()
            .find(|arg| arg.ends_with("/adapter.sock"))
            .unwrap();
        if args
            .iter()
            .any(|arg| arg == "fixture-probe" || arg == "fixture-fail")
        {
            for _ in 0..200 {
                if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(socket) {
                    stream.write_all(b"{\"type\":\"health\"}\n").unwrap();
                    stream.shutdown(std::net::Shutdown::Write).unwrap();
                    let mut line = String::new();
                    std::io::BufReader::new(stream)
                        .read_line(&mut line)
                        .unwrap();
                    assert_eq!(
                        serde_json::from_str::<Value>(&line).unwrap()["type"],
                        "health"
                    );
                    assert!(
                        !args.iter().any(|arg| arg == "fixture-fail"),
                        "intentional readiness failure"
                    );
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("fixture adapter did not become ready");
        }
        assert!(args.iter().any(|arg| arg == "fixture-serve"));
        let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
        fs::set_permissions(socket, fs::Permissions::from_mode(0o600)).unwrap();
        let mut descendant = std::process::Command::new("sleep")
            .arg("300")
            .spawn()
            .unwrap();
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        fs::write(home.join("descendant.pid"), descendant.id().to_string()).unwrap();
        let generation = args
            .iter()
            .find(|arg| arg.starts_with("generation-"))
            .unwrap();
        let mut owned = BTreeMap::new();
        let mut synchronization = super::sync_fixture::Fixture::default();
        let mut navigation = super::navigation_fixture::Fixture::default();
        let mut next_lease = 0_u64;
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            std::io::BufReader::new(&stream)
                .read_line(&mut line)
                .unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let kind = request["type"].as_str().unwrap();
            if let Some(response) =
                navigation.reply(&request, &home, generation, &mut owned, &mut next_lease)
            {
                if let Some(response) = response {
                    let _ = writeln!(stream, "{response}");
                }
                continue;
            }
            if let Some(response) = synchronization.reply(&request, &home, generation) {
                if let Some(response) = response {
                    let _ = writeln!(stream, "{response}");
                }
                continue;
            }
            if matches!(
                kind,
                "prepareBuffer"
                    | "openBufferLease"
                    | "queryBufferLease"
                    | "releaseBufferLease"
                    | "readBufferLease"
            ) {
                let lease = if kind == "prepareBuffer" {
                    next_lease += 1;
                    let id = format!("{next_lease:016x}");
                    owned.insert(id.clone(), "prepared");
                    json!({"instance":format!("{:032x}", std::process::id()), "id":id})
                } else {
                    request["lease"].clone()
                };
                let id = lease["id"].as_str().unwrap();
                if kind == "readBufferLease" {
                    assert_eq!(owned[id], "open", "read of a non-open fixture buffer");
                    let result = if request["request"]["kind"] == "symbols" {
                        json!({"kind":"symbols","symbols":[]})
                    } else {
                        json!({"kind":"language","diagnosticsState":"observed","diagnostics":[{
                            "start":{"row":0,"column":0}, "end":{"row":0,"column":1},
                            "severity":1,"source":null,"message":generation}],
                            "inlayHints":[],"semanticTokens":[]})
                    };
                    let mut response = json!({"type":"bufferLeaseRead","api_version":1,"lease":lease,"opened_version":[],"result":result});
                    if generation == "generation-bad-read" {
                        response["lease"]["instance"] = json!("f".repeat(32));
                    }
                    let _ = writeln!(stream, "{response}");
                    continue;
                }
                if kind == "prepareBuffer" && generation == "generation-pause-prepare" {
                    fs::write(home.join("prepare-requested"), "ready").unwrap();
                    for _ in 0..500 {
                        if home.join("resume-prepare").exists() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    assert!(
                        home.join("resume-prepare").exists(),
                        "fixture preparation never resumed"
                    );
                }
                if kind == "openBufferLease" {
                    owned.insert(id.to_owned(), "open");
                    if generation == "generation-lost" {
                        continue;
                    }
                    if generation == "generation-paused" {
                        fs::write(home.join("open-requested"), "ready").unwrap();
                        for _ in 0..500 {
                            if home.join("resume-open").exists() {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        assert!(
                            home.join("resume-open").exists(),
                            "fixture open never resumed"
                        );
                    }
                }
                if kind == "releaseBufferLease" {
                    owned.insert(id.to_owned(), "released");
                    if generation == "generation-lost-close" {
                        continue;
                    }
                }
                let mut response = json!({"type":"bufferLease", "api_version":1, "lease":lease, "state":owned[id]});
                if generation == "generation-mismatch" && kind == "releaseBufferLease" {
                    response["lease"]["instance"] = json!("f".repeat(32));
                }
                if generation == "generation-bad-prepare" && kind == "prepareBuffer" {
                    response["api_version"] = json!(99);
                }
                let _ = writeln!(stream, "{response}");
                continue;
            }
            let response = json!({
                "type": if kind == "health" { "health" } else { "worktree" },
                "leases": u8::from(kind == "openWorktree" || kind == "openBuffer"),
                "generation": generation, "descendant": descendant.id(),
                "home": home,
                "credential_environment": std::env::vars().any(|(key, _)| key.starts_with("ANTHROPIC_") || key.starts_with("COWBOY_AUTH_")),
            });
            writeln!(stream, "{response}").unwrap();
        }
        let _ = descendant.kill();
        let _ = descendant.wait();
    }

    pub(super) async fn assert_process_stopped(pid: u32) {
        for _ in 0..100 {
            let output = Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .await
                .unwrap();
            let state = String::from_utf8(output.stdout).unwrap();
            if state.trim().is_empty() || state.trim().starts_with('Z') {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("owned code runtime descendant survived teardown: {pid}");
    }

    #[tokio::test]
    async fn exact_generations_coexist_until_each_worktree_drains() {
        let root = PrivateRuntimeDirectory::create().unwrap();
        let host = CodeRuntimeHost::default();
        let old = fixture_plan(&root.0, "old");
        let new = fixture_plan(&root.0, "new");
        fs::create_dir_all(root.0.join("worktree-a")).unwrap();
        fs::create_dir_all(root.0.join("worktree-b")).unwrap();
        let open = |name: &str| json!({"type": "openBuffer", "worktree": root.0.join(name), "path": "a.rs", "leaseId": "tab"});
        let first = host
            .request("fixture-code", &open("worktree-a"), || {
                Ok(CodeRuntimeSelection::Installed(old.clone()))
            })
            .await
            .unwrap();
        assert_eq!(first["generation"], "generation-old");
        assert_eq!(first["credential_environment"], false);
        // A retry of the same buffer lease does not add a second lease.
        let retry = host
            .request("fixture-code", &open("worktree-a"), || {
                panic!("active generation was reselected")
            })
            .await
            .unwrap();
        assert_eq!(retry["generation"], first["generation"]);
        let second = host
            .request("fixture-code", &open("worktree-b"), || {
                Ok(CodeRuntimeSelection::Installed(new.clone()))
            })
            .await
            .unwrap();
        assert_eq!(second["generation"], "generation-new");
        assert_ne!(first["home"], second["home"]);
        let close = |name: &str| json!({"type": "closeBuffer", "worktree": root.0.join(name), "path": "a.rs", "leaseId": "tab"});
        host.request("fixture-code", &close("worktree-a"), || {
            panic!("draining worktree changed generation")
        })
        .await
        .unwrap();
        assert_process_stopped(u32::try_from(first["descendant"].as_u64().unwrap()).unwrap()).await;
        let reopened = host
            .request("fixture-code", &open("worktree-a"), || {
                Ok(CodeRuntimeSelection::Installed(new))
            })
            .await
            .unwrap();
        assert_eq!(reopened["descendant"], second["descendant"]);
        host.request("fixture-code", &close("worktree-a"), || {
            panic!("reselected")
        })
        .await
        .unwrap();
        host.request("fixture-code", &close("worktree-b"), || {
            panic!("reselected")
        })
        .await
        .unwrap();
        assert_process_stopped(u32::try_from(second["descendant"].as_u64().unwrap()).unwrap())
            .await;
    }

    #[tokio::test]
    async fn failed_readiness_reaps_adapter_descendants() {
        let root = PrivateRuntimeDirectory::create().unwrap();
        let mut plan = fixture_plan(&root.0, "failed");
        for argument in &mut plan.runtime.readiness.arguments {
            if let CodeRuntimeArgument::Literal { value } = argument
                && value == "fixture-probe"
            {
                *value = "fixture-fail".to_owned();
            }
        }
        assert!(
            probe_code_runtime(&plan)
                .await
                .unwrap_err()
                .to_string()
                .contains("readiness")
        );
        let pid = fs::read_to_string(plan.home.join("descendant.pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_process_stopped(pid).await;
    }

    #[tokio::test]
    async fn legacy_worktree_lease_survives_new_runtime_selection() {
        let root = PrivateRuntimeDirectory::create().unwrap();
        let legacy = RunningCodeRuntime::start(&fixture_plan(&root.0, "legacy"))
            .await
            .unwrap();
        let host = CodeRuntimeHost::default();
        let open = json!({"type": "openWorktree", "path": root.0, "trusted": true});
        host.request("fixture-code", &open, || {
            Ok(CodeRuntimeSelection::Legacy(legacy.socket.clone()))
        })
        .await
        .unwrap();
        let close = json!({"type": "closeWorktree", "path": root.0});
        let response = host
            .request("fixture-code", &close, || panic!("legacy lease was stolen"))
            .await
            .unwrap();
        assert_eq!(response["generation"], "generation-legacy");
        assert!(
            host.request("fixture-code", &open, || bail!("Plugin was uninstalled"))
                .await
                .unwrap_err()
                .to_string()
                .contains("uninstalled")
        );
        legacy.stop().await;
    }

    #[test]
    fn closing_one_buffer_does_not_drop_other_buffer_or_worktree_leases() {
        let mut route = WorktreeRoute::default();
        route
            .observe(&json!({"type": "openWorktree"}), &json!({"leases": 1}))
            .unwrap();
        for id in ["a", "b"] {
            route
                .observe(
                    &json!({"type": "openBuffer", "path": "a.rs", "leaseId": id}),
                    &json!({}),
                )
                .unwrap();
        }
        assert!(
            !route
                .observe(&json!({"type": "closeWorktree"}), &json!({"leases": 0}))
                .unwrap()
        );
        assert!(
            !route
                .observe(
                    &json!({"type": "closeBuffer", "path": "a.rs", "leaseId": "a"}),
                    &json!({})
                )
                .unwrap()
        );
        assert!(
            route
                .observe(
                    &json!({"type": "closeBuffer", "path": "a.rs", "leaseId": "b"}),
                    &json!({})
                )
                .unwrap()
        );
    }
}
