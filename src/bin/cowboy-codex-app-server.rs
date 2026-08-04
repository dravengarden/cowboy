//! Transparent Codex CLI wrapper for Cowboy-managed ACP workers.
//!
//! `codex-acp` resumes through Codex App Server. Older adapter releases omit
//! `excludeTurns`, making App Server serialize the full historic thread even
//! for ACP `session/resume`, whose contract explicitly returns no history.
//! This wrapper preserves every Codex invocation and NDJSON frame while adding
//! that one idempotent request option. `session/load` remains correct because
//! the adapter follows resume with an explicit `thread/read(includeTurns=true)`.

#![warn(clippy::pedantic)]

use std::ffi::OsString;
use std::process::{ExitCode, Stdio};

use anyhow::{Context as _, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::process::Command;

const REAL_CODEX_ENV: &str = "COWBOY_CODEX_REAL_CMD";

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("cowboy-codex-app-server: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<ExitCode> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let real_codex = std::env::var_os(REAL_CODEX_ENV).unwrap_or_else(|| OsString::from("codex"));

    if args.first().and_then(|arg| arg.to_str()) != Some("app-server") {
        let status = Command::new(&real_codex)
            .args(&args)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .with_context(|| format!("running {}", real_codex.to_string_lossy()))?;
        return Ok(exit_code(status));
    }

    let mut child = Command::new(&real_codex)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("starting {} app-server", real_codex.to_string_lossy()))?;
    let child_stdin = child.stdin.take().context("Codex app-server stdin")?;
    let mut child_stdout = child.stdout.take().context("Codex app-server stdout")?;
    let mut child_stderr = child.stderr.take().context("Codex app-server stderr")?;

    let mut input = tokio::spawn(forward_requests(
        BufReader::new(tokio::io::stdin()),
        child_stdin,
    ));
    let output =
        tokio::spawn(
            async move { tokio::io::copy(&mut child_stdout, &mut tokio::io::stdout()).await },
        );
    let errors =
        tokio::spawn(
            async move { tokio::io::copy(&mut child_stderr, &mut tokio::io::stderr()).await },
        );

    let status = tokio::select! {
        status = child.wait() => status.context("waiting for Codex app-server")?,
        forwarded = &mut input => {
            forwarded.context("joining Codex request proxy")??;
            child.wait().await.context("waiting for Codex app-server after stdin closed")?
        }
    };
    input.abort();
    output.await.context("joining Codex response proxy")??;
    errors.await.context("joining Codex stderr proxy")??;
    Ok(exit_code(status))
}

async fn forward_requests(
    mut input: BufReader<tokio::io::Stdin>,
    mut output: tokio::process::ChildStdin,
) -> std::io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line).await? == 0 {
            break;
        }
        output.write_all(&rewrite_request_line(&line)).await?;
        output.flush().await?;
    }
    output.shutdown().await
}

fn rewrite_request_line(line: &[u8]) -> Vec<u8> {
    let newline = line.ends_with(b"\n");
    let payload = line.strip_suffix(b"\n").unwrap_or(line);
    let payload = payload.strip_suffix(b"\r").unwrap_or(payload);
    let Ok(mut message) = serde_json::from_slice::<Value>(payload) else {
        return line.to_vec();
    };
    if message.get("method").and_then(Value::as_str) != Some("thread/resume") {
        return line.to_vec();
    }
    let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) else {
        return line.to_vec();
    };
    params.insert("excludeTurns".to_owned(), Value::Bool(true));

    let Ok(mut rewritten) = serde_json::to_vec(&message) else {
        return line.to_vec();
    };
    if newline {
        rewritten.push(b'\n');
    }
    rewritten
}

fn exit_code(status: std::process::ExitStatus) -> ExitCode {
    status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .map_or(ExitCode::FAILURE, ExitCode::from)
}

#[cfg(test)]
mod tests {
    use super::rewrite_request_line;

    #[test]
    fn resume_requests_exclude_historic_turns() {
        let rewritten = rewrite_request_line(
            br#"{"id":4,"method":"thread/resume","params":{"threadId":"thread-1"}}
"#,
        );
        let message: serde_json::Value = serde_json::from_slice(&rewritten).expect("valid JSON");

        assert_eq!(
            message.pointer("/params/excludeTurns"),
            Some(&serde_json::Value::Bool(true))
        );
        assert_eq!(
            message
                .pointer("/params/threadId")
                .and_then(|value| value.as_str()),
            Some("thread-1")
        );
        assert!(rewritten.ends_with(b"\n"));
    }

    #[test]
    fn an_existing_false_resume_option_is_overridden() {
        let rewritten = rewrite_request_line(
            br#"{"method":"thread/resume","params":{"excludeTurns":false}}
"#,
        );
        let message: serde_json::Value = serde_json::from_slice(&rewritten).expect("valid JSON");

        assert_eq!(
            message.pointer("/params/excludeTurns"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn every_other_frame_is_byte_exact() {
        for frame in [
            b"{\"id\":1,\"method\":\"initialize\",\"params\":{}}\n".as_slice(),
            b"{\"id\":2,\"method\":\"thread/read\",\"params\":{}}\r\n".as_slice(),
            b"not-json\n".as_slice(),
        ] {
            assert_eq!(rewrite_request_line(frame), frame);
        }
    }
}
