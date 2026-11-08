//! Private NDJSON bridge for a trusted local host. Never expose this transport remotely.
use cowboy_codeact_runtime::{Host, Sandbox};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{Mutex, oneshot},
};
type Waiters = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;
struct Bridge {
    waiters: Waiters,
    output: Mutex<tokio::io::Stdout>,
    sequence: AtomicU64,
}
impl Host for Bridge {
    async fn call(&self, op: &str, args: Value, remaining: Duration) -> Result<Value, String> {
        let id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let (send, receive) = oneshot::channel();
        self.waiters.lock().await.insert(id, send);
        let mut frame =
            serde_json::to_vec(&json!({"id":id,"op":op,"args":args})).map_err(|_| "protocol")?;
        frame.push(b'\n');
        {
            let mut output = self.output.lock().await;
            output
                .write_all(&frame)
                .await
                .map_err(|_| "host_unavailable")?;
            output.flush().await.map_err(|_| "host_unavailable")?;
        }
        let result = tokio::time::timeout(remaining, receive).await;
        self.waiters.lock().await.remove(&id);
        let value = result
            .map_err(|_| "timeout")?
            .map_err(|_| "host_unavailable")?;
        if let Some(code) = value
            .get("error")
            .and_then(|v| v.get("code"))
            .and_then(Value::as_str)
        {
            return Err(code.to_owned());
        }
        value
            .get("ok")
            .cloned()
            .ok_or_else(|| "host_protocol".to_owned())
    }
    fn finish(&self, done: bool, frame: Value) -> Value {
        if !done {
            return json!({"state":"error","error":frame["error"]});
        }
        let value = &frame["value"];
        if serde_json::to_vec(value).map_or(usize::MAX, |v| v.len()) > 16 * 1024 {
            return json!({"state":"error","error":{"code":"result_too_large"}});
        }
        json!({"state":"ok","value":value})
    }
    fn call_error(&self, _op: &str, _echo: &str, code: String) -> Value {
        json!({"code":code})
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(6..=8).contains(&args.len()) {
        return Err(
            "usage: cowboy-codeact PROGRAM RUNNER BUN PRLIMIT BWRAP [RUNTIME_DIRECTORY [BUN_LOADER]]".into(),
        );
    }
    let source = std::fs::read_to_string(&args[1])?;
    let runner = std::fs::read_to_string(&args[2])?;
    if source.len() > 32 * 1024 || runner.len() > 64 * 1024 {
        return Err("source budget exceeded".into());
    }
    let waiters: Waiters = Arc::default();
    let incoming = waiters.clone();
    let mut reader = tokio::spawn(async move {
        let mut input = BufReader::new(tokio::io::stdin());
        loop {
            let mut line = String::new();
            use tokio::io::AsyncReadExt as _;
            let read = (&mut input).take(256 * 1024 + 1).read_line(&mut line).await;
            if !matches!(read, Ok(1..=262144)) || !line.ends_with('\n') {
                break;
            }
            let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                break;
            };
            if let Some(id) = frame["id"].as_u64()
                && let Some(waiter) = incoming.lock().await.remove(&id)
            {
                let _ = waiter.send(frame);
            }
        }
        incoming.lock().await.clear();
    });
    let host = Bridge {
        waiters,
        output: Mutex::new(tokio::io::stdout()),
        sequence: AtomicU64::new(1),
    };
    let sandbox = Sandbox {
        runtime_directory: args.get(6).map(Into::into),
        bun_loader: args.get(7).map(Into::into),
        prlimit: std::fs::canonicalize(&args[4])?,
        bwrap: std::fs::canonicalize(&args[5])?,
        bun: std::fs::canonicalize(&args[3])?,
    };
    let result = tokio::select! {
        biased;
        _ = &mut reader => json!({"state":"error","error":{"code":"host_unavailable"}}),
        result = sandbox.execute(&source, true, 30_000, &runner, &host) => result,
    };
    reader.abort();
    let mut output = serde_json::to_vec(&json!({"done":result}))?;
    output.push(b'\n');
    let mut stdout = host.output.lock().await;
    stdout.write_all(&output).await?;
    stdout.flush().await?;
    Ok(())
}
