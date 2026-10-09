//! Private parent-scoped ingress hosted by the existing Machine process.
//! The callback must check Controller authority for every action, including
//! observations. Possession of local wiring alone never authorizes a launch.

use std::{
    future::Future,
    io,
    os::unix::fs::{
        DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
    },
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use rand::RngCore as _;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    net::{UnixListener, UnixStream},
    task::{JoinHandle, JoinSet},
};

use super::protocol::{Action, LocalRequest, MAX_FRAME_BYTES};

pub struct Gateway {
    task: JoinHandle<()>,
    directory: PathBuf,
}

impl Gateway {
    /// Each immutable authorization gets a fresh directory. Existing directories
    /// are deliberately refused: deleting a stale socket requires the Machine's
    /// lifecycle owner to prove its previous listener is gone first.
    pub fn bind<F, Fut>(directory: &Path, dispatch: F) -> io::Result<Self>
    where
        F: Fn(Action) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Value> + Send + 'static,
    {
        if !directory.is_absolute() || directory.join("socket").as_os_str().len() > 100 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let parent = directory.parent().ok_or(io::ErrorKind::InvalidInput)?;
        let metadata = std::fs::symlink_metadata(parent)?;
        if !metadata.is_dir()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || parent.canonicalize()? != parent
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        std::fs::DirBuilder::new().mode(0o700).create(directory)?;
        match Self::bind_new(directory, dispatch) {
            Ok(gateway) => Ok(gateway),
            Err(error) => {
                let _ = std::fs::remove_file(directory.join("context.json"));
                let _ = std::fs::remove_file(directory.join("socket"));
                let _ = std::fs::remove_dir(directory);
                Err(error)
            }
        }
    }

    fn bind_new<F, Fut>(directory: &Path, dispatch: F) -> io::Result<Self>
    where
        F: Fn(Action) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Value> + Send + 'static,
    {
        let socket = directory.join("socket");
        let listener = UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let mut entropy = [0u8; 32];
        rand::rngs::OsRng
            .try_fill_bytes(&mut entropy)
            .map_err(|_| io::ErrorKind::Other)?;
        let capability = entropy
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let capability_digest: [u8; 32] = Sha256::digest(capability.as_bytes()).into();
        let context = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.join("context.json"))?;
        serde_json::to_writer(
            &context,
            &json!({"schema":1,"socket":socket,"capability":capability}),
        )?;
        context.sync_all()?;
        std::fs::File::open(directory)?.sync_all()?;
        let dispatch = Arc::new(dispatch);
        let task = tokio::spawn(async move {
            // All accepted sockets live in this set; revocation/drop aborts them
            // together with the listener, rather than leaving detached waiters.
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    biased;
                    _ = clients.join_next(), if !clients.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break };
                        // Refuse excess connections without allocating another
                        // task. A disconnected Start remains admission-unknown.
                        if clients.len() >= 32 { drop(stream); continue; }
                        let dispatch = Arc::clone(&dispatch);
                        clients.spawn(async move {
                            let _ = serve(stream, capability_digest, dispatch).await;
                        });
                    }
                }
            }
        });
        Ok(Self {
            task,
            directory: directory.to_owned(),
        })
    }

    pub fn context_path(&self) -> PathBuf {
        self.directory.join("context.json")
    }
}

impl Drop for Gateway {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_file(self.directory.join("context.json"));
        let _ = std::fs::remove_file(self.directory.join("socket"));
        let _ = std::fs::remove_dir(&self.directory);
    }
}

async fn serve<F, Fut>(
    stream: UnixStream,
    capability_digest: [u8; 32],
    dispatch: Arc<F>,
) -> io::Result<()>
where
    F: Fn(Action) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Value> + Send,
{
    if stream.peer_cred()?.uid() != rustix::process::geteuid().as_raw() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let mut reader = BufReader::new(stream);
    let input = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut reader)).await;
    let request = match input {
        Ok(Ok(bytes)) => LocalRequest::parse(&bytes).ok(),
        _ => None,
    };
    let response = if let Some(request) = request {
        let supplied: [u8; 32] = Sha256::digest(request.capability.as_bytes()).into();
        if supplied != capability_digest {
            refusal("permission_denied", "not_submitted")
        } else {
            let timeout = Duration::from_millis(request.action.wait_ms() + 2000);
            match tokio::time::timeout(timeout, dispatch(request.action)).await {
                Ok(value) if value.get("schema").and_then(Value::as_u64) == Some(1) => value,
                _ => refusal("outcome_unknown", "unknown"),
            }
        }
    } else {
        refusal("invalid_contract", "not_submitted")
    };
    let mut bytes = serde_json::to_vec(&response)?;
    if bytes.len() > 4 * 1024 * 1024 - 1 {
        bytes = serde_json::to_vec(&refusal("outcome_unknown", "unknown"))?;
    }
    bytes.push(b'\n');
    tokio::time::timeout(Duration::from_secs(2), reader.get_mut().write_all(&bytes))
        .await
        .map_err(|_| io::ErrorKind::TimedOut)??;
    Ok(())
}

fn refusal(code: &str, admission: &str) -> Value {
    json!({"schema":1,"state":"error","error":{"code":code,"admission":admission}})
}

async fn read_frame(reader: &mut BufReader<UnixStream>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let end = chunk.iter().position(|byte| *byte == b'\n');
        let count = end.map_or(chunk.len(), |index| index + 1);
        if bytes.len() + count > MAX_FRAME_BYTES {
            return Err(io::ErrorKind::InvalidData.into());
        }
        bytes.extend_from_slice(&chunk[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    async fn request(socket: &Path, value: Value) -> Value {
        let mut stream = UnixStream::connect(socket).await.unwrap();
        stream
            .write_all(format!("{value}\n").as_bytes())
            .await
            .unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[tokio::test]
    async fn separate_parent_gateways_reject_crossed_tokens_and_unknown_fields() {
        let root = std::env::temp_dir().join(format!(
            "cw-gw-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        let first = Gateway::bind(&root.join("a"), move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            async { json!({"schema":1,"parent":"a"}) }
        })
        .unwrap();
        let second = Gateway::bind(&root.join("b"), |_| async {
            json!({"schema":1,"parent":"b"})
        })
        .unwrap();
        let a: Value =
            serde_json::from_slice(&std::fs::read(first.context_path()).unwrap()).unwrap();
        let b: Value =
            serde_json::from_slice(&std::fs::read(second.context_path()).unwrap()).unwrap();
        let socket = Path::new(a["socket"].as_str().unwrap());
        let mut frame =
            json!({"schema":1,"capability":a["capability"],"action":{"kind":"capabilities"}});
        assert_eq!(request(socket, frame.clone()).await["parent"], "a");
        frame["capability"] = b["capability"].clone();
        assert_eq!(
            request(socket, frame.clone()).await["error"]["code"],
            "permission_denied"
        );
        frame["capability"] = a["capability"].clone();
        frame["action"]["parent_session_id"] = json!("b");
        assert_eq!(
            request(socket, frame).await["error"]["code"],
            "invalid_contract"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(Gateway::bind(&root.join("a"), |_| async { json!({"schema":1}) }).is_err());
        drop(first);
        assert!(UnixStream::connect(socket).await.is_err());
        drop(second);
        std::fs::remove_dir(root).unwrap();
    }
}
