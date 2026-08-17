//! Versioned filesystem/Git adapter for remote Cowboy machines.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::code_review::{CodeProvider as _, DiffScope, LocalCodeProvider};

#[derive(Debug, Deserialize)]
pub struct CodeAdapterRequest {
    pub root: String,
    #[serde(flatten)]
    pub operation: CodeOperation,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodeOperation {
    Manifest,
    Directory {
        path: String,
        limit: usize,
    },
    Search {
        query: String,
        limit: usize,
    },
    Changes,
    Repository {
        #[serde(default)]
        after: Option<String>,
    },
    Commit {
        oid: String,
    },
    CommitDiff {
        oid: String,
        path: String,
    },
    Diff {
        path: String,
        context: usize,
        show_whitespace: bool,
        scope: DiffScope,
    },
    File {
        path: String,
        cursor: Option<String>,
    },
    FileRaw {
        path: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CodeAdapterResponse {
    Manifest(crate::code_review::WorktreeManifest),
    Directory(crate::code_review::CodeTreePage),
    Search(Vec<String>),
    Changes(crate::code_review::ChangeList),
    Repository(crate::code_review::GitRepositorySnapshot),
    Commit(crate::code_review::GitCommitDetail),
    CommitDiff(crate::code_review::DiffDocument),
    Diff(crate::code_review::DiffDocument),
    File(crate::code_review::FileDocument),
    FileRaw(crate::code_review::RawFileDocument),
}

pub async fn serve(socket: &Path, roots: Vec<PathBuf>) -> Result<()> {
    let roots = prepare_trusted_roots(roots)?;
    if let Some(parent) = socket.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if tokio::fs::try_exists(socket).await? {
        tokio::fs::remove_file(socket).await?;
    }
    let listener = UnixListener::bind(socket)?;
    loop {
        let (stream, _) = listener.accept().await?;
        let roots = roots.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_one(stream, &roots).await {
                tracing::warn!(%error, "code adapter request failed");
            }
        });
    }
}

fn prepare_trusted_roots(roots: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    if roots.is_empty() {
        bail!("at least one trusted workspace root is required");
    }
    roots
        .into_iter()
        .map(|root| {
            let (root, available) = crate::workspace_roots::resolve_configured_root(&root)
                .with_context(|| format!("anchoring trusted workspace {}", root.display()))?;
            if available && !root.is_dir() {
                bail!("trusted workspace {} is not a directory", root.display());
            }
            Ok(root)
        })
        .collect()
}

async fn serve_one(stream: UnixStream, roots: &[PathBuf]) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    BufReader::new(read).read_line(&mut line).await?;
    let response = match serde_json::from_str::<CodeAdapterRequest>(&line)
        .context("decoding code adapter request")
        .and_then(|request| execute(request, roots))
    {
        Ok(value) => serde_json::json!({ "ok": true, "value": value }),
        Err(error) => serde_json::json!({ "ok": false, "error": error.to_string() }),
    };
    write.write_all(response.to_string().as_bytes()).await?;
    write.write_all(b"\n").await?;
    write.shutdown().await?;
    Ok(())
}

fn execute(request: CodeAdapterRequest, roots: &[PathBuf]) -> Result<CodeAdapterResponse> {
    let root = PathBuf::from(&request.root).canonicalize()?;
    if !roots
        .iter()
        .any(|trusted| crate::workspace_roots::canonical_target_within_root(&root, trusted))
    {
        bail!("workspace is outside the trusted Machine roots");
    }
    let provider = LocalCodeProvider::new(root);
    Ok(match request.operation {
        CodeOperation::Manifest => {
            CodeAdapterResponse::Manifest(provider.manifest().map_err(anyhow::Error::msg)?)
        }
        CodeOperation::Directory { path, limit } => CodeAdapterResponse::Directory(
            provider
                .directory(&path, limit)
                .map_err(anyhow::Error::msg)?,
        ),
        CodeOperation::Search { query, limit } => {
            CodeAdapterResponse::Search(provider.search(&query, limit))
        }
        CodeOperation::Changes => {
            CodeAdapterResponse::Changes(provider.changes().map_err(anyhow::Error::msg)?)
        }
        CodeOperation::Repository { after } => CodeAdapterResponse::Repository(
            provider
                .repository(after.as_deref())
                .map_err(anyhow::Error::msg)?,
        ),
        CodeOperation::Commit { oid } => {
            CodeAdapterResponse::Commit(provider.commit(&oid).map_err(anyhow::Error::msg)?)
        }
        CodeOperation::CommitDiff { oid, path } => CodeAdapterResponse::CommitDiff(
            provider
                .commit_diff(&oid, &path)
                .map_err(anyhow::Error::msg)?,
        ),
        CodeOperation::Diff {
            path,
            context,
            show_whitespace,
            scope,
        } => CodeAdapterResponse::Diff(
            provider
                .diff_snapshot(&path, context, show_whitespace, scope)
                .map_err(anyhow::Error::msg)?,
        ),
        CodeOperation::File { path, cursor } => CodeAdapterResponse::File(
            provider
                .file_page(&path, cursor.as_deref())
                .map_err(anyhow::Error::msg)?,
        ),
        CodeOperation::FileRaw { path } => {
            CodeAdapterResponse::FileRaw(provider.file_raw(&path).map_err(anyhow::Error::msg)?)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn request(socket: &Path, payload: serde_json::Value) -> serde_json::Value {
        let mut stream = UnixStream::connect(socket).await.unwrap();
        stream
            .write_all(format!("{payload}\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        BufReader::new(stream)
            .read_line(&mut response)
            .await
            .unwrap();
        serde_json::from_str(&response).unwrap()
    }

    #[test]
    fn serves_files_only_below_a_trusted_workspace() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-code-adapter-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("hello.rs"), "fn hello() {}\n").unwrap();
        std::fs::write(root.join("mark.png"), [0x89, b'P', b'N', b'G', 0, 1, 2]).unwrap();
        let root = root.canonicalize().unwrap();
        let response = execute(
            CodeAdapterRequest {
                root: root.display().to_string(),
                operation: CodeOperation::File {
                    path: "hello.rs".to_owned(),
                    cursor: None,
                },
            },
            std::slice::from_ref(&root),
        )
        .unwrap();
        let CodeAdapterResponse::File(file) = response else {
            panic!("wrong response")
        };
        assert_eq!(file.text, "fn hello() {}\n");
        let raw = execute(
            CodeAdapterRequest {
                root: root.display().to_string(),
                operation: CodeOperation::FileRaw {
                    path: "mark.png".to_owned(),
                },
            },
            std::slice::from_ref(&root),
        )
        .unwrap();
        let CodeAdapterResponse::FileRaw(file) = raw else {
            panic!("wrong response")
        };
        assert_eq!(file.media_type, "image/png");
        assert_eq!(file.bytes[1], b'P');
        let outside = execute(
            CodeAdapterRequest {
                root: root.parent().unwrap().display().to_string(),
                operation: CodeOperation::File {
                    path: "not-allowed".to_owned(),
                    cursor: None,
                },
            },
            std::slice::from_ref(&root),
        );
        assert!(outside.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn pending_roots_do_not_block_available_roots_and_activate_without_restart() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-code-adapter-pending-{}",
            std::process::id()
        ));
        let available = root.join("available");
        let pending = root.join("pending");
        let socket = root.join("adapter.sock");
        std::fs::create_dir_all(&available).unwrap();
        std::fs::write(available.join("available.txt"), "ready\n").unwrap();

        let server_socket = socket.clone();
        let server_roots = vec![available.clone(), pending.clone()];
        let server = tokio::spawn(async move { serve(&server_socket, server_roots).await });
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(socket.exists(), "adapter did not start with a pending root");
        assert!(!server.is_finished());

        let available_response = request(
            &socket,
            serde_json::json!({
                "root": available,
                "type": "file",
                "path": "available.txt",
                "cursor": null
            }),
        )
        .await;
        assert_eq!(available_response["ok"], true);
        assert_eq!(available_response["value"]["value"]["text"], "ready\n");

        std::fs::create_dir(&pending).unwrap();
        std::fs::write(pending.join("synchronized.txt"), "arrived\n").unwrap();
        let synchronized_response = request(
            &socket,
            serde_json::json!({
                "root": pending,
                "type": "file",
                "path": "synchronized.txt",
                "cursor": null
            }),
        )
        .await;
        assert_eq!(synchronized_response["ok"], true);
        assert_eq!(synchronized_response["value"]["value"]["text"], "arrived\n");

        #[cfg(unix)]
        {
            std::fs::remove_dir_all(&pending).unwrap();
            let outside = root.with_extension("outside");
            std::fs::create_dir(&outside).unwrap();
            std::fs::write(outside.join("secret.txt"), "must not escape\n").unwrap();
            std::os::unix::fs::symlink(&outside, &pending).unwrap();
            let escaped_response = request(
                &socket,
                serde_json::json!({
                    "root": pending,
                    "type": "file",
                    "path": "secret.txt",
                    "cursor": null
                }),
            )
            .await;
            assert_eq!(escaped_response["ok"], false);
            std::fs::remove_dir_all(outside).unwrap();
        }

        server.abort();
        let _ = server.await;
        std::fs::remove_dir_all(root).unwrap();
    }
}
