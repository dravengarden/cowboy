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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CodeAdapterResponse {
    Manifest(crate::code_review::WorktreeManifest),
    Directory(crate::code_review::CodeTreePage),
    Search(Vec<String>),
    Changes(crate::code_review::ChangeList),
    Diff(crate::code_review::DiffDocument),
    File(crate::code_review::FileDocument),
}

pub async fn serve(socket: &Path, roots: Vec<PathBuf>) -> Result<()> {
    let roots = roots
        .into_iter()
        .map(|root| {
            root.canonicalize()
                .with_context(|| format!("canonicalizing {}", root.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    if roots.is_empty() {
        bail!("at least one trusted workspace root is required");
    }
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
        .any(|trusted| root == *trusted || root.starts_with(trusted))
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_files_only_below_a_trusted_workspace() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-code-adapter-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("hello.rs"), "fn hello() {}\n").unwrap();
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
}
