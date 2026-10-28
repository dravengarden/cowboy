//! The stdio MCP server native Claude starts for a target MCP server: the
//! native counterpart of `runtime/mcp-proxy.mjs`, speaking the same bridge
//! protocol. It runs the target's command on the target and relays its stdin,
//! stdout and stderr, so native owns the MCP session itself: initialize,
//! tools, instructions, cancellation and reconnection. It ends with the target
//! process, and ends that process when native closes it.
//!
//! A session keeps one relay per target server resident for its whole life.
//! A Node relay holds a V8 heap; this one holds a few pages.
//!
//! `cowboy-mcp-relay task-wait <job>` is the native counterpart of
//! `runtime/task-wait.mjs`, the equally long-lived command behind a native
//! background task that stands for a target command left running.

use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Map, Value, json};

/// A stop is best effort: an exiting relay never waits on a stuck bridge.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
/// Bridge requests carry at most this many stdin bytes each.
const WRITE_CHUNK: usize = 64 * 1024;
/// The session variables native set for this server reach the target one.
const SESSION_VARIABLES: [&str; 2] = ["CLAUDECODE", "CLAUDE_CODE_SESSION_ID"];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 && args[1] == "--version" {
        println!("cowboy-mcp-relay {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.len() == 3 && args[1] == "task-wait" {
        exit(task_wait(&args[2]));
    }
    if args.len() != 3 {
        eprintln!("usage: cowboy-mcp-relay <context.json> <server> | task-wait <job>");
        exit(2);
    }
    // Before any thread starts, so every thread inherits the mask and only
    // the signal thread receives them.
    let signals = block_signals();
    let code = run(&args[1], &args[2], signals).unwrap_or_else(|error| {
        eprintln!("Target MCP server unavailable: {error}");
        1
    });
    let _ = io::stdout().flush();
    exit(code);
}

struct Bridge {
    socket: String,
    authorization: String,
}

struct Session {
    bridge: Bridge,
    id: OnceLock<String>,
    stopped: AtomicBool,
}

impl Session {
    /// Ends the target process once; later calls do nothing.
    fn stop(&self) {
        if let Some(id) = self.id.get()
            && !self.stopped.swap(true, Ordering::SeqCst)
        {
            let _ = self
                .bridge
                .post("/mcp-stop", &json!({ "id": id }), Some(STOP_TIMEOUT));
        }
    }
}

/// The launcher bridge named by a session context file.
fn bridge(context: &str) -> Result<Bridge, String> {
    let context: Value = serde_json::from_slice(
        &std::fs::read(context).map_err(|error| format!("context: {error}"))?,
    )
    .map_err(|error| format!("context: {error}"))?;
    let field = |name: &str| {
        context[name]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("context: missing {name}"))
    };
    Ok(Bridge {
        socket: field("socketPath")?,
        authorization: format!("Bearer {}", field("bridgeToken")?),
    })
}

/// Waits for a target command and returns its exit status. Native owns the
/// task: its registry, its completion notification and TaskStop. A stopped
/// command never returns, as natively a stopped task reports nothing; TaskStop
/// or the session's end terminates this process instead.
fn task_wait(job: &str) -> i32 {
    let Some(bridge) = std::env::var("COWBOY_CLAUDE_CONTEXT")
        .ok()
        .and_then(|context| bridge(&context).ok())
    else {
        return 1;
    };
    let mut after_seq = Value::Null;
    let mut failures = 0;
    loop {
        let reply = match bridge.post(
            "/task-wait",
            &json!({ "id": job, "afterSeq": after_seq }),
            None,
        ) {
            Ok(reply) => {
                failures = 0;
                reply
            }
            Err(_) => {
                // The launcher restarts with its bridge; the task ends with it
                // anyway.
                failures += 1;
                if failures > 5 {
                    return 1;
                }
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        if reply["closed"] == true {
            return reply["exitCode"]
                .as_i64()
                .and_then(|code| i32::try_from(code).ok())
                .unwrap_or(1);
        }
        // Stay alive (an ended process would read as a completion).
        if reply["stopped"] == true || reply["gone"] == true {
            loop {
                thread::park();
            }
        }
        if reply["unavailable"] == true {
            thread::sleep(Duration::from_secs(1));
        }
        if reply["afterSeq"].is_i64() {
            after_seq = reply["afterSeq"].clone();
        }
    }
}

fn run(context: &str, server: &str, signals: libc::sigset_t) -> Result<i32, String> {
    let session = Arc::new(Session {
        bridge: bridge(context)?,
        id: OnceLock::new(),
        stopped: AtomicBool::new(false),
    });

    // Native ends a server it closes; the target's ends with it.
    {
        let session = session.clone();
        thread::spawn(move || {
            let mut signal = 0;
            // SAFETY: `signals` is an initialized set blocked in every thread.
            unsafe { libc::sigwait(&signals, &mut signal) };
            session.stop();
            let _ = io::stdout().flush();
            exit(143);
        });
    }

    let env: Map<String, Value> = SESSION_VARIABLES
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| ((*name).to_owned(), Value::String(value)))
        })
        .collect();
    let started =
        session
            .bridge
            .post("/mcp-start", &json!({ "server": server, "env": env }), None)?;
    let id = started["id"]
        .as_str()
        .ok_or("the bridge returned no process")?
        .to_owned();
    let _ = session.id.set(id.clone());

    // Native's messages reach the target in order.
    {
        let session = session.clone();
        let id = id.clone();
        thread::spawn(move || {
            let mut input = io::stdin().lock();
            let mut buffer = vec![0; WRITE_CHUNK];
            loop {
                let read = match input.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => break,
                };
                let data = STANDARD.encode(&buffer[..read]);
                match session
                    .bridge
                    .post("/mcp-write", &json!({ "id": id, "data": data }), None)
                {
                    Ok(reply) if reply["status"] == "accepted" => {}
                    _ => break,
                }
            }
            // Native closed the server: it ends on the target too.
            session.stop();
        });
    }

    let mut after_seq = Value::Null;
    let mut failures = 0;
    loop {
        let reply = match session.bridge.post(
            "/mcp-read",
            &json!({ "id": id, "afterSeq": after_seq }),
            None,
        ) {
            Ok(reply) => {
                failures = 0;
                reply
            }
            Err(_) => {
                failures += 1;
                if failures > 5 {
                    session.stop();
                    return Ok(1);
                }
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        for chunk in reply["chunks"].as_array().into_iter().flatten() {
            let data = STANDARD
                .decode(chunk["data"].as_str().unwrap_or_default())
                .map_err(|error| format!("output: {error}"))?;
            // A slow reader holds the next page back rather than buffering
            // here: these writes block.
            let written = if chunk["stream"] == "stderr" {
                io::stderr().lock().write_all(&data)
            } else {
                let mut stdout = io::stdout().lock();
                stdout.write_all(&data).and_then(|()| stdout.flush())
            };
            if written.is_err() {
                // Native is gone; so is its server.
                session.stop();
                return Ok(1);
            }
        }
        after_seq = reply["afterSeq"].clone();
        if reply["lost"] == true {
            eprintln!(
                "Target MCP server output exceeded what the executor retains; the server was stopped."
            );
        }
        if reply["closed"] == true {
            session.stop();
            return Ok(reply["exitCode"]
                .as_i64()
                .and_then(|code| i32::try_from(code).ok())
                .unwrap_or(1));
        }
    }
}

fn block_signals() -> libc::sigset_t {
    // SAFETY: plain POSIX signal-set initialization of a local value.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::sigaddset(&mut set, signal);
        }
        libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        set
    }
}

impl Bridge {
    /// One authenticated JSON POST over the bridge's Unix socket.
    fn post(&self, path: &str, body: &Value, timeout: Option<Duration>) -> Result<Value, String> {
        let failed = |error: io::Error| format!("{path}: {error}");
        let mut stream = UnixStream::connect(&self.socket).map_err(failed)?;
        stream.set_read_timeout(timeout).map_err(failed)?;
        stream.set_write_timeout(timeout).map_err(failed)?;
        let body = body.to_string();
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.authorization,
            body.len()
        );
        stream.write_all(head.as_bytes()).map_err(failed)?;
        stream.write_all(body.as_bytes()).map_err(failed)?;
        let (status, payload) = read_response(&mut BufReader::new(stream)).map_err(failed)?;
        let value: Value =
            serde_json::from_slice(&payload).map_err(|error| format!("{path}: {error}"))?;
        if status == 200 {
            Ok(value)
        } else {
            Err(value["deny"].as_str().unwrap_or("failed").to_owned())
        }
    }
}

/// An HTTP/1.1 response's status and body, with a length, chunked or
/// read-to-close body.
fn read_response(reader: &mut impl BufRead) -> io::Result<(u16, Vec<u8>)> {
    let invalid = |what: &str| io::Error::new(ErrorKind::InvalidData, what.to_owned());
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| invalid("malformed status line"))?;
    let mut length = None;
    let mut chunked = false;
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err(invalid("truncated headers"));
        }
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let value = value.trim();
            if name.eq_ignore_ascii_case("content-length") {
                length = Some(value.parse().map_err(|_| invalid("bad length"))?);
            } else if name.eq_ignore_ascii_case("transfer-encoding") {
                chunked = value.eq_ignore_ascii_case("chunked");
            }
        }
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            line.clear();
            reader.read_line(&mut line)?;
            let size = line.trim_end().split(';').next().unwrap_or_default();
            let size = usize::from_str_radix(size, 16).map_err(|_| invalid("bad chunk"))?;
            if size == 0 {
                break;
            }
            let start = body.len();
            body.resize(start + size, 0);
            reader.read_exact(&mut body[start..])?;
            line.clear();
            reader.read_line(&mut line)?;
        }
    } else if let Some(length) = length {
        body.resize(length, 0);
        reader.read_exact(&mut body)?;
    } else {
        reader.read_to_end(&mut body)?;
    }
    Ok((status, body))
}

#[cfg(test)]
mod tests {
    use super::read_response;

    #[test]
    fn reads_length_chunked_and_close_delimited_bodies() {
        let length = b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"id\":\"a\"}\n";
        assert_eq!(
            read_response(&mut &length[..]).unwrap(),
            (200, b"{\"id\":\"a\"}\n".to_vec())
        );
        let chunked = b"HTTP/1.1 403 Forbidden\r\ntransfer-encoding: chunked\r\n\r\n4\r\n{\"de\r\n9;x=1\r\nny\":\"no\"}\r\n0\r\n\r\n";
        assert_eq!(
            read_response(&mut &chunked[..]).unwrap(),
            (403, b"{\"deny\":\"no\"}".to_vec())
        );
        let closed = b"HTTP/1.0 200 OK\r\n\r\n{}";
        assert_eq!(
            read_response(&mut &closed[..]).unwrap(),
            (200, b"{}".to_vec())
        );
        assert!(read_response(&mut &b"HTTP/1.1 200 OK\r\n"[..]).is_err());
    }
}
