//! Explicit, host-owned delegation for the Controller's private Unix endpoint.
//! Possessing a repository checkout or sending an HTTP header grants no authority.

use anyhow::{Context as _, Result, bail, ensure};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub(crate) const SOCKET_NAME: &str = "control.sock";
const GRANT_NAME: &str = "grant.json";

pub(crate) fn uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

pub(crate) fn directory(data_dir: &Path) -> Result<PathBuf> {
    let directory = data_dir.canonicalize()?.join("local-operator");
    match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error).context("creating local Operator directory"),
    }
    private_directory(&directory)?;
    Ok(directory)
}

fn private_directory(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && !metadata.is_symlink()
            && metadata.uid() == uid()
            && metadata.mode() & 0o777 == 0o700,
        "local Operator directory must be owned by the Service account with mode 0700"
    );
    Ok(())
}

pub(crate) fn private_file(path: &Path, create: bool) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(create)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == uid()
            && metadata.nlink() == 1
            && metadata.mode() & 0o777 == 0o600,
        "local Operator file must be private and owned by the Service account"
    );
    Ok(file)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema: u16,
    generation: String,
}

#[derive(PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    generation: String,
}

fn policy(directory: &Path) -> Result<Identity> {
    private_directory(directory)?;
    let file = private_file(&directory.join(GRANT_NAME), false)?;
    let metadata = file.metadata()?;
    ensure!(metadata.len() <= 4096, "local Operator policy is too large");
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 4096, "local Operator policy is too large");
    let policy: Policy = serde_json::from_slice(&bytes)?;
    ensure!(
        policy.schema == 1
            && policy.generation.len() == 64
            && policy.generation.bytes().all(|b| b.is_ascii_hexdigit()),
        "unsupported local Operator policy"
    );
    Ok(Identity {
        device: metadata.dev(),
        inode: metadata.ino(),
        generation: policy.generation,
    })
}

// Never serialized or reconstructed from an Actor, request body or journal.
pub(crate) struct Grant {
    directory: PathBuf,
    identity: Identity,
    peer_uid: u32,
    live: Arc<AtomicBool>,
}

impl Grant {
    pub(crate) fn capture(directory: &Path, peer_uid: u32, live: Arc<AtomicBool>) -> Result<Self> {
        ensure!(
            peer_uid == uid() || peer_uid == 0,
            "local Operator peer is not the Service account"
        );
        ensure!(
            live.load(Ordering::Acquire),
            "local Operator listener is stopping"
        );
        Ok(Self {
            directory: directory.to_owned(),
            identity: policy(directory)?,
            peer_uid,
            live,
        })
    }

    pub(crate) fn current(&self) -> bool {
        self.live.load(Ordering::Acquire)
            && policy(&self.directory).is_ok_and(|identity| identity == self.identity)
    }

    pub(crate) fn actor(&self) -> crate::plugin_operation::Actor {
        // Reserved host namespace inside the existing bounded Admin actor codec.
        // This does not create or impersonate a browser admin account.
        crate::plugin_operation::Actor::Admin {
            account: format!("unix-uid:{}", self.peer_uid),
        }
    }
}

fn enable(directory: &Path) -> Result<()> {
    private_directory(directory)?;
    let policy = Policy {
        schema: 1,
        generation: crate::product_auth::new_session_token()?,
    };
    let temporary = directory.join(format!("grant-{}.tmp", policy.generation));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(&policy)?)?;
        file.sync_all()?;
        std::fs::rename(&temporary, directory.join(GRANT_NAME))?;
        File::open(directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[derive(Args)]
pub(crate) struct OperatorArgs {
    /// Controller data directory, on the Controller host under its Service account.
    #[arg(
        long,
        env = "COWBOY_DATA_DIR",
        default_value = "/var/lib/cowboy",
        global = true
    )]
    data_dir: PathBuf,
    #[command(subcommand)]
    command: OperatorCommand,
}

#[derive(Subcommand)]
enum OperatorCommand {
    /// Explicitly enable local host delegation; replacing a grant revokes old operations.
    Enable,
    /// Revoke the local grant, including not-yet-dispatched effects.
    Disable,
    /// Inspect the local endpoint and its authenticated host identity.
    Status,
    /// Stop every automatic convergence path Service-wide until it is resumed.
    /// Already dispatched work finishes under its existing transaction.
    Freeze {
        /// Recorded with the stop so the next person knows why it is there.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Resume automatic convergence.
    Unfreeze,
    /// List trusted published Plugin releases.
    Catalog,
    /// Refresh the signed Catalog through the running Controller.
    RefreshCatalog,
    /// Read the registered Machine's installed Plugin inventory.
    Inspect {
        #[arg(long)]
        machine: String,
    },
    /// Install an exact signed release through the durable Service/Machine transaction.
    #[command(alias = "upgrade")]
    Install {
        #[arg(long)]
        machine: String,
        #[arg(long)]
        plugin: String,
        #[arg(long)]
        version: String,
        #[arg(long)]
        digest: String,
        /// Reuse this identity to observe a lost response. It never replays an install.
        #[arg(long)]
        operation_id: String,
    },
    /// Converge installed Plugins to the Catalog's newest ready releases.
    /// Dry run by default. The `--machine` order is the rollout order and its
    /// first entry is the canary: a Machine that does not converge stops the
    /// rest. Digests are resolved from the Catalog, never authored.
    Converge {
        /// Rollout order. Repeatable. Omitted: every connected Machine.
        #[arg(long)]
        machine: Vec<String>,
        /// Bound the run to these Plugins. Repeatable.
        #[arg(long)]
        plugin: Vec<String>,
        /// Submit the upgrades. Without it nothing is installed.
        #[arg(long)]
        apply: bool,
    },
    /// Read durable installation receipts without repeating any operation.
    Operations {
        #[arg(long)]
        machine: String,
        #[arg(long)]
        plugin: String,
    },
    /// Query and commit the exact terminal Machine receipt for one fenced install.
    /// This never repeats the installation or infers its outcome from inventory.
    ReconcileInstall {
        #[arg(long)]
        machine: String,
        #[arg(long)]
        plugin: String,
        #[arg(long)]
        operation_id: String,
    },
    /// Read account usage, optionally refreshing one account through its installed Plugin.
    Usage {
        #[arg(long)]
        refresh: Option<String>,
    },
}

pub(crate) async fn run(args: OperatorArgs) -> Result<()> {
    let directory = directory(&args.data_dir)?;
    match args.command {
        OperatorCommand::Enable => {
            enable(&directory)?;
            println!(
                "{}",
                json!({"schema":1,"local_operator":"enabled","uid":uid(),"socket":directory.join(SOCKET_NAME)})
            );
            return Ok(());
        }
        OperatorCommand::Disable => {
            match std::fs::remove_file(directory.join(GRANT_NAME)) {
                Ok(()) => File::open(&directory)?.sync_all()?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            println!("{}", json!({"schema":1,"local_operator":"disabled"}));
            return Ok(());
        }
        OperatorCommand::Freeze { reason } => {
            let freeze = crate::machine_convergence::ConvergenceFreeze::new(&args.data_dir);
            let record = freeze
                .freeze(
                    &format!("unix-uid:{}", uid()),
                    reason.as_deref(),
                    crate::usage::now_ms(),
                )
                .context("recording the convergence freeze")?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"schema":1,"convergence":"frozen","record":record})
                )?
            );
            return Ok(());
        }
        OperatorCommand::Unfreeze => {
            crate::machine_convergence::ConvergenceFreeze::new(&args.data_dir)
                .thaw()
                .context("removing the convergence freeze")?;
            println!("{}", json!({"schema":1,"convergence":"running"}));
            return Ok(());
        }
        _ => {}
    }
    let client = reqwest::Client::builder()
        .unix_socket(directory.join(SOCKET_NAME))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(330))
        .build()?;
    if let OperatorCommand::Converge {
        machine,
        plugin,
        apply,
    } = &args.command
    {
        let freeze = crate::machine_convergence::ConvergenceFreeze::new(&args.data_dir);
        if *apply {
            // A stop must reach every path that could install, including this
            // one. A dry run stays available: reading is how you decide to
            // resume.
            if let Some(record) = freeze.current() {
                bail!(
                    "convergence is frozen by {} ({}); run `cowboy operator unfreeze` before applying",
                    if record.actor.is_empty() {
                        "an unknown actor"
                    } else {
                        &record.actor
                    },
                    record.reason.as_deref().unwrap_or("no reason recorded")
                );
            }
            eprintln!(
                "Convergence submits durable installations. A lost response requires receipt inspection before the same run is repeated."
            );
        }
        return converge::run(&client, &args.data_dir, machine, plugin, *apply).await;
    }
    let (method, segments, body, operation) = match args.command {
        OperatorCommand::Status => (reqwest::Method::GET, vec!["status".into()], None, None),
        OperatorCommand::Catalog => (reqwest::Method::GET, vec!["plugins".into()], None, None),
        OperatorCommand::RefreshCatalog => (
            reqwest::Method::POST,
            vec!["plugins".into(), "refresh".into()],
            None,
            None,
        ),
        OperatorCommand::Inspect { machine } => (
            reqwest::Method::GET,
            vec!["machines".into(), machine, "plugins".into()],
            None,
            None,
        ),
        OperatorCommand::Install {
            machine,
            plugin,
            version,
            digest,
            operation_id,
        } => {
            ensure!(
                crate::plugin_operation::installation::valid_operation_id(&operation_id),
                "invalid operation ID"
            );
            eprintln!(
                "Operation {operation_id}: a failed or lost response requires receipt inspection before another identity is used."
            );
            (
                reqwest::Method::POST,
                vec![
                    "machines".into(),
                    machine,
                    "plugins".into(),
                    plugin,
                    "install".into(),
                ],
                Some(json!({"version":version,"digest":digest,"operation_id":operation_id})),
                Some(operation_id),
            )
        }
        OperatorCommand::Operations { machine, plugin } => (
            reqwest::Method::GET,
            vec![
                "machines".into(),
                machine,
                "plugins".into(),
                plugin,
                "operations".into(),
            ],
            None,
            None,
        ),
        OperatorCommand::ReconcileInstall {
            machine,
            plugin,
            operation_id,
        } => {
            ensure!(
                crate::plugin_operation::installation::valid_operation_id(&operation_id),
                "invalid operation ID"
            );
            (
                reqwest::Method::POST,
                vec![
                    "machines".into(),
                    machine,
                    "plugins".into(),
                    plugin,
                    "operations".into(),
                    operation_id.clone(),
                    "reconcile".into(),
                ],
                None,
                Some(operation_id),
            )
        }
        OperatorCommand::Usage { refresh } => match refresh {
            Some(provider) => (
                reqwest::Method::POST,
                vec!["usage".into(), provider],
                None,
                None,
            ),
            None => (reqwest::Method::GET, vec!["usage".into()], None, None),
        },
        OperatorCommand::Enable
        | OperatorCommand::Disable
        | OperatorCommand::Freeze { .. }
        | OperatorCommand::Unfreeze
        | OperatorCommand::Converge { .. } => unreachable!(),
    };
    let mut url = reqwest::Url::parse("http://localhost/v1/")?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|()| anyhow::anyhow!("invalid local endpoint"))?;
        path.pop_if_empty();
        for segment in segments {
            path.push(&segment);
        }
    }
    let mut request = client.request(method, url);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let mut response = request.send().await.context(
        "local Operator request failed; inspect the original operation before retrying a write",
    )?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.context(
        "local Operator response was interrupted; inspect the original operation before retrying a write",
    )? {
        ensure!(
            bytes.len() + chunk.len() <= 4 * 1024 * 1024,
            "local Operator response exceeds limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let data: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"schema":1,"http_status":status.as_u16(),"operation_id":operation,"data":data})
        )?
    );
    if !status.is_success() {
        bail!("local Operator returned HTTP {status}; inspect receipts before retrying a write");
    }
    Ok(())
}

mod converge;

#[cfg(test)]
mod tests;
