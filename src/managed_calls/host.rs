//! Machine-hosted call ingress for parents whose tools execute here.
//!
//! The Controller installs one gateway per parent grant. A tool process finds
//! it through a stable per-session context file that Cowboy injects as
//! `COWBOY_CALL_CONTEXT`; the file is atomically replaced when a new grant
//! (for example a replacement worker incarnation) supersedes the old one.
//! Every action is forwarded to the Controller over the authenticated Machine
//! channel, which re-derives authority before acting. A lost uplink fails the
//! waiting action; it never queues the request for later delivery.

use std::collections::HashMap;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::sync::{mpsc, oneshot};

use super::gateway::Gateway;
use super::protocol::{Action, Grant};
use crate::machine_protocol::MachineEvent;

/// Stable context file for one session below a Machine state directory.
/// Both the broker (local parents) and execution keepers (remote parents)
/// derive the same path, so process start parameters never carry it.
pub fn context_file(state_dir: &Path, session_id: &str) -> Option<PathBuf> {
    super::valid_id(session_id).then(|| {
        state_dir
            .join("calls")
            .join("sessions")
            .join(format!("{session_id}.json"))
    })
}

struct Installed {
    grant_id: String,
    _gateway: Gateway,
}

pub struct CallHost {
    sockets: PathBuf,
    sessions: PathBuf,
    installed: Mutex<HashMap<String, Installed>>,
    uplink: Mutex<Option<(u64, mpsc::UnboundedSender<MachineEvent>)>>,
    pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    next: AtomicU64,
    instance: String,
}

/// Keeps one Controller connection attached. Dropping it fails every action
/// still waiting for that connection's reply.
pub struct Uplink {
    host: Weak<CallHost>,
    id: u64,
}

impl Drop for Uplink {
    fn drop(&mut self) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let mut uplink = host.uplink.lock();
        if uplink.as_ref().is_some_and(|(id, _)| *id == self.id) {
            *uplink = None;
            drop(uplink);
            host.pending.lock().clear();
        }
    }
}

fn private_directory(path: &Path) -> Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.mode() & 0o077 == 0,
        "managed call directory is not private"
    );
    Ok(())
}

fn refusal(code: &str, admission: &str) -> Value {
    json!({"schema":1,"state":"error","error":{"code":code,"admission":admission}})
}

impl CallHost {
    /// Context files live with the Machine state; sockets use a short private
    /// namespace because Unix socket paths are bounded.
    pub fn new(state_dir: &Path) -> Result<Arc<Self>> {
        let calls = state_dir.join("calls");
        private_directory(&calls)?;
        let sessions = calls.join("sessions");
        private_directory(&sessions)?;
        // Any existing context refers to a gateway of a previous process.
        // Remove it rather than letting a tool reach a stale socket path.
        for entry in std::fs::read_dir(&sessions)? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let _ = std::fs::remove_file(path);
            }
        }
        let uid = rustix::process::geteuid().as_raw();
        let temporary = std::fs::canonicalize(std::env::temp_dir())?;
        let root = temporary.join(format!("cowboy-calls-{uid}"));
        private_directory(&root)?;
        let mut entropy = [0u8; 4];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut entropy);
        let instance = entropy.iter().map(|b| format!("{b:02x}")).collect();
        let sockets = root.join(&instance);
        private_directory(&sockets)?;
        Ok(Arc::new(Self {
            sockets,
            sessions,
            installed: Mutex::new(HashMap::new()),
            uplink: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            instance,
        }))
    }

    pub fn context_path(&self, session_id: &str) -> Option<PathBuf> {
        super::valid_id(session_id).then(|| self.sessions.join(format!("{session_id}.json")))
    }

    pub fn attach(self: &Arc<Self>, events: mpsc::UnboundedSender<MachineEvent>) -> Uplink {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        *self.uplink.lock() = Some((id, events));
        self.pending.lock().clear();
        Uplink {
            host: Arc::downgrade(self),
            id,
        }
    }

    /// Complete one forwarded action. Unknown or late replies are ignored.
    pub fn reply(&self, request_id: &str, response: Value) {
        if let Some(waiter) = self.pending.lock().remove(request_id) {
            let _ = waiter.send(response);
        }
    }

    pub fn install(self: &Arc<Self>, grant: &Grant) -> Result<()> {
        ensure!(grant.validate(), "invalid managed call grant");
        let mut installed = self.installed.lock();
        if installed
            .get(&grant.parent_session_id)
            .is_some_and(|current| current.grant_id == grant.grant_id)
        {
            return Ok(());
        }
        // Revoke first: a tool must never read a context that names a socket
        // whose grant the Controller has already replaced.
        if installed.remove(&grant.parent_session_id).is_some() {
            let _ = std::fs::remove_file(
                self.sessions
                    .join(format!("{}.json", grant.parent_session_id)),
            );
        }
        let digest = format!("{:x}", Sha256::digest(grant.grant_id.as_bytes()));
        let directory = self.sockets.join(&digest[..16]);
        let host = Arc::downgrade(self);
        let forwarded = grant.clone();
        let gateway = Gateway::bind(&directory, move |action| {
            let host = host.clone();
            let grant = forwarded.clone();
            async move {
                match host.upgrade() {
                    Some(host) => host.forward(grant, action).await,
                    None => refusal("gateway_unavailable", "not_submitted"),
                }
            }
        })
        .context("binding managed call gateway")?;
        let context = std::fs::read(gateway.context_path())?;
        let target = self
            .context_path(&grant.parent_session_id)
            .context("invalid parent session")?;
        let partial = self.sessions.join(format!(
            ".{}.{}.partial",
            grant.parent_session_id, self.instance
        ));
        let _ = std::fs::remove_file(&partial);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&partial)?;
        std::io::Write::write_all(&mut &file, &context)?;
        file.sync_all()?;
        std::fs::rename(&partial, &target)?;
        installed.insert(
            grant.parent_session_id.clone(),
            Installed {
                grant_id: grant.grant_id.clone(),
                _gateway: gateway,
            },
        );
        Ok(())
    }

    /// Removing an unknown or already replaced grant is a successful no-op.
    pub fn revoke(&self, grant: &Grant) {
        let mut installed = self.installed.lock();
        if installed
            .get(&grant.parent_session_id)
            .is_some_and(|current| current.grant_id == grant.grant_id)
        {
            installed.remove(&grant.parent_session_id);
            if let Some(path) = self.context_path(&grant.parent_session_id) {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    pub fn installed_grant(&self, parent: &str) -> Option<String> {
        self.installed
            .lock()
            .get(parent)
            .map(|current| current.grant_id.clone())
    }

    async fn forward(&self, grant: Grant, action: Action) -> Value {
        let submits = action.submits();
        let unknown = if submits {
            refusal("outcome_unknown", "unknown")
        } else {
            refusal("gateway_unavailable", "not_submitted")
        };
        let wait = Duration::from_millis(action.wait_ms()) + Duration::from_secs(10);
        let request_id = format!(
            "call-{}-{}",
            self.instance,
            self.next.fetch_add(1, Ordering::Relaxed)
        );
        let (sender, receiver) = oneshot::channel();
        {
            let uplink = self.uplink.lock();
            let Some((_, events)) = uplink.as_ref() else {
                return refusal("gateway_unavailable", "not_submitted");
            };
            self.pending.lock().insert(request_id.clone(), sender);
            if events
                .send(MachineEvent::ManagedCall {
                    request_id: request_id.clone(),
                    grant,
                    action: Box::new(action),
                })
                .is_err()
            {
                self.pending.lock().remove(&request_id);
                return refusal("gateway_unavailable", "not_submitted");
            }
        }
        let result = tokio::time::timeout(wait, receiver).await;
        self.pending.lock().remove(&request_id);
        match result {
            Ok(Ok(value)) if value.get("schema").and_then(Value::as_u64) == Some(1) => value,
            _ => unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
    use tokio::net::UnixStream;

    async fn request(context: &Path, action: Value) -> Value {
        let wiring: Value = serde_json::from_slice(&std::fs::read(context).unwrap()).unwrap();
        let mut stream = UnixStream::connect(wiring["socket"].as_str().unwrap())
            .await
            .unwrap();
        let frame = json!({"schema":1,"capability":wiring["capability"],"action":action});
        stream
            .write_all(format!("{frame}\n").as_bytes())
            .await
            .unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn state() -> PathBuf {
        let root = std::env::temp_dir().join(format!("cw-host-{}", rand::random::<u64>()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        root
    }

    #[tokio::test]
    async fn forwards_through_the_current_uplink_and_replacement_revokes_the_old_context() {
        let root = state();
        let host = CallHost::new(&root).unwrap();
        let grant = Grant {
            grant_id: "grant-1".into(),
            parent_session_id: "parent-1".into(),
        };
        host.install(&grant).unwrap();
        host.install(&grant).unwrap();
        let context = context_file(&root, "parent-1").unwrap();
        assert_eq!(host.context_path("parent-1").unwrap(), context);
        let old_wiring = std::fs::read(&context).unwrap();
        // Without an uplink nothing can be submitted.
        let refused = request(&context, json!({"kind":"capabilities"})).await;
        assert_eq!(refused["error"]["admission"], "not_submitted");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let uplink = host.attach(tx);
        let waiting = tokio::spawn({
            let context = context.clone();
            async move { request(&context, json!({"kind":"capabilities"})).await }
        });
        let Some(MachineEvent::ManagedCall {
            request_id,
            grant: forwarded,
            ..
        }) = rx.recv().await
        else {
            panic!("managed call event")
        };
        assert_eq!(forwarded, grant);
        host.reply(&request_id, json!({"schema":1,"state":"ok"}));
        assert_eq!(waiting.await.unwrap()["state"], "ok");
        // A Start whose uplink disappears has an unknown outcome.
        let waiting = tokio::spawn({
            let context = context.clone();
            async move {
                request(
                    &context,
                    json!({"kind":"start","provider":"codex","wait_ms":0,"request":{
                        "schema":1,"request_id":"r-1","purpose":"review","instruction":"x",
                        "context":{"scope":"current-worktree"},"access":"read-only",
                        "conversation":{"mode":"fresh"}}}),
                )
                .await
            }
        });
        assert!(rx.recv().await.is_some());
        drop(uplink);
        assert_eq!(waiting.await.unwrap()["error"]["admission"], "unknown");
        let replacement = Grant {
            grant_id: "grant-2".into(),
            parent_session_id: "parent-1".into(),
        };
        host.install(&replacement).unwrap();
        assert_ne!(std::fs::read(&context).unwrap(), old_wiring);
        host.revoke(&grant);
        assert!(context.exists());
        host.revoke(&replacement);
        assert!(!context.exists());
        assert_eq!(host.installed_grant("parent-1"), None);
        drop(host);
        let _ = std::fs::remove_dir_all(root);
    }
}
