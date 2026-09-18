//! Test-only stdio LSP for the isolated native navigation gate. Never packaged
//! or resolved through PATH. It exercises Zed's actual cross-file buffer sharing,
//! not a real language implementation or production LSP dependency.
use std::fs::OpenOptions;
use std::io::{BufRead as _, Read as _, Write as _};
use std::path::PathBuf;

use anyhow::{Context as _, Result, ensure};
use serde_json::{Value, json};

fn main() -> Result<()> {
    let root = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("fixture root required")?,
    );
    ensure!(root.is_absolute() && root.is_dir(), "invalid fixture root");
    let mut audit = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("lsp-events.jsonl"))?;
    let workspace = root.join("lsp-worktree");
    let uri = |name: &str| format!("file://{}/{}", workspace.display(), name);
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    while let Some(message) = receive(&mut input)? {
        let method = message["method"].as_str().unwrap_or_default();
        let document = message["params"]["textDocument"]["uri"].as_str();
        writeln!(audit, "{}", json!({"method": method, "document": document}))?;
        audit.flush()?;
        if method == "exit" {
            break;
        }
        let Some(id) = message.get("id") else {
            continue;
        };
        let result = match method {
            "initialize" => json!({"capabilities": {
                "positionEncoding": "utf-16",
                "textDocumentSync": {"openClose": true, "change": 1},
                "definitionProvider": true, "declarationProvider": true,
                "typeDefinitionProvider": true, "implementationProvider": true,
                "referencesProvider": true, "hoverProvider": true
            }}),
            "textDocument/definition"
            | "textDocument/declaration"
            | "textDocument/typeDefinition"
            | "textDocument/implementation"
            | "textDocument/references" => {
                let range = json!({
                    "start": {"line": 0, "character": 4},
                    "end": {"line": 0, "character": 6}
                });
                if document.is_some_and(|uri| uri.ends_with("reject-locations.rs")) {
                    json!(
                        (0..257)
                            .map(|_| json!({"uri": uri("never-acquire.rs"), "range": range}))
                            .collect::<Vec<_>>()
                    )
                } else if document.is_some_and(|uri| uri.ends_with("reject-targets.rs")) {
                    json!((0..33).map(|i| json!({"uri": uri(&format!("never-acquire-{i}.rs")), "range": range})).collect::<Vec<_>>())
                } else if document.is_some_and(|uri| uri.ends_with("reject-external.rs")) {
                    json!([
                        {"uri": uri("destination-a.rs"), "range": range},
                        {"uri": format!("file://{}/outside.rs", root.display()), "range": range}
                    ])
                } else if document.is_some_and(|uri| uri.ends_with("reject-range.rs")) {
                    json!([{"uri": uri("destination-a.rs"), "range": {
                        "start": {"line":0,"character":5}, "end": {"line":0,"character":6}
                    }}])
                } else {
                    json!([
                        {"uri": uri("destination-a.rs"), "range": range},
                        {"uri": uri("destination-b.rs"), "range": range},
                        {"uri": uri("destination-a.rs"), "range": range}
                    ])
                }
            }
            "textDocument/hover" => json!({"contents": {
                "kind": "plaintext", "value": "owned native destination fixture"
            }}),
            _ => Value::Null,
        };
        let response = if method == "textDocument/definition"
            && document.is_some_and(|uri| uri.ends_with("reject-lsp.rs"))
        {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32801,"message":"content modified"}})
        } else {
            json!({"jsonrpc": "2.0", "id": id, "result": result})
        };
        let bytes = serde_json::to_vec(&response)?;
        write!(output, "Content-Length: {}\r\n\r\n", bytes.len())?;
        output.write_all(&bytes)?;
        output.flush()?;
    }
    Ok(())
}

fn receive(input: &mut impl std::io::BufRead) -> Result<Option<Value>> {
    let mut length = None;
    let mut headers = 0;
    loop {
        let mut line = String::new();
        let read = input.take(8_193).read_line(&mut line)?;
        if read == 0 && headers == 0 {
            return Ok(None);
        }
        headers += read;
        ensure!(read != 0 && headers <= 8_192, "invalid LSP framing");
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            ensure!(length.is_none(), "duplicate LSP length");
            length = Some(value.trim().parse::<usize>()?);
        }
    }
    let length = length.context("LSP length missing")?;
    ensure!(length <= 1024 * 1024, "fixture input exceeds limit");
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}
