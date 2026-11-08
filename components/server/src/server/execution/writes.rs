//! Observed executor writes: a hint, not a durable receipt.
//!
//! Every native `fs/writeFile` passes through `forward`. Its target path and
//! the SHA-256 of its LF-normalized bytes (the native mirror's text identity)
//! are kept in bounded Controller memory once the executor reports
//! completion, including a completion observed later by its operation ID.
//! There is no turn or tool identity, nothing survives a Controller restart,
//! and other writers (shells, helpers, other processes) are not seen. A later
//! write of the same path replaces the earlier observation.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Instant;

use base64::Engine as _;
use parking_lot::Mutex;
use sha2::{Digest as _, Sha256};

use crate::execution_protocol::{Command, Outcome, Response as KeeperResponse};
use crate::machine_protocol::execution::Response;

const MAX_WRITES: usize = 4096;
const MAX_PENDING: usize = 256;
/// Larger writes are not hashed; the native mirror refuses them anyway.
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::server) struct Write {
    pub sequence: u64,
    pub sha256: String,
    pub utf8_bytes: usize,
}

#[derive(Default)]
struct Log {
    sequence: u64,
    writes: HashMap<(String, String), (Write, Instant)>,
    /// Admitted writes whose completion has not been observed yet.
    pending: HashMap<(String, String), (String, String, usize)>,
    /// Running apply helpers by process ID: target path, expected digest and
    /// the stdout read so far.
    helpers: HashMap<(String, String), (String, String, String)>,
    /// Helper output reads still pending, by operation ID: their process ID.
    reads: HashMap<(String, String), String>,
}

/// Bounded helper stdout; an outcome is one short JSON line.
const MAX_HELPER_OUTPUT: usize = 8 * 1024;

/// The target path and expected after-digest of an inline apply request.
/// Large requests travel in a file and are not observed.
fn helper_request(params: &serde_json::Value) -> Option<(String, String, String)> {
    let argv: Vec<&str> = params["argv"]
        .as_array()?
        .iter()
        .map(serde_json::Value::as_str)
        .collect::<Option<_>>()?;
    let index = argv
        .windows(2)
        .position(|pair| pair == ["apply", "--request"])?;
    let encoded = argv.get(index + 2)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let request: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let path = target_path(request["path"].as_str()?)?;
    let expect = request["expect"]
        .as_str()
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))?;
    Some((
        params["processId"].as_str()?.to_owned(),
        path,
        expect.to_owned(),
    ))
}

/// The helper's applied outcome for exactly the expected digest.
fn applied(output: &str, expect: &str) -> Option<usize> {
    output.lines().find_map(|line| {
        let outcome: serde_json::Value = serde_json::from_str(line).ok()?;
        (outcome["schema"] == 1 && outcome["status"] == "applied" && outcome["sha256"] == expect)
            .then(|| {
                outcome["size"]
                    .as_u64()
                    .and_then(|size| usize::try_from(size).ok())
            })
            .flatten()
    })
}

fn helper_output(log: &mut Log, session: &str, process: &str, reply: &serde_json::Value) {
    let key = (session.to_owned(), process.to_owned());
    let Some((path, expect, output)) = log.helpers.get_mut(&key) else {
        return;
    };
    for chunk in reply["chunks"].as_array().into_iter().flatten() {
        if chunk["stream"] != "stdout" {
            continue;
        }
        let Some(bytes) = chunk["chunk"]
            .as_str()
            .and_then(|data| base64::engine::general_purpose::STANDARD.decode(data).ok())
        else {
            continue;
        };
        if output.len() + bytes.len() <= MAX_HELPER_OUTPUT {
            output.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    if let Some(size) = applied(output, expect) {
        // The helper's digest is of the written bytes; LF files share it with
        // the native mirror, CRLF files never match.
        let identity = (path.clone(), expect.clone(), size);
        log.helpers.remove(&key);
        record(log, session, identity);
    } else if reply["closed"] == true || output.len() >= MAX_HELPER_OUTPUT {
        // Exit can precede the last output; only a closed pipe is final.
        log.helpers.remove(&key);
    }
}

static LOG: LazyLock<Mutex<Log>> = LazyLock::new(Mutex::default);

/// An absolute target path: native callers send `file://` URLs; a plain
/// absolute path is accepted too. Relative or non-file locations are not.
fn target_path(value: &str) -> Option<String> {
    if value.starts_with('/') {
        return Some(value.to_owned());
    }
    let url = url::Url::parse(value).ok()?;
    (url.scheme() == "file" && url.host().is_none())
        .then(|| url.to_file_path().ok())
        .flatten()?
        .to_str()
        .map(str::to_owned)
}

/// The written path and identity of an `fs/writeFile` call, if hashable.
fn identity(params: &serde_json::Value) -> Option<(String, String, usize)> {
    let path = target_path(params["path"].as_str()?)?;
    let data = params["dataBase64"].as_str()?;
    if data.len() > MAX_BYTES / 3 * 4 + 4 {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    let text = std::str::from_utf8(&bytes).ok()?.replace("\r\n", "\n");
    Some((
        path,
        format!("{:x}", Sha256::digest(text.as_bytes())),
        text.len(),
    ))
}

fn completed(response: &Response) -> Option<bool> {
    match response {
        Response::Call {
            response:
                KeeperResponse::Operation {
                    outcome: Outcome::Completed { reply },
                },
        } => Some(reply.get("error").is_none()),
        Response::Call {
            response:
                KeeperResponse::Operation {
                    outcome: Outcome::Pending,
                },
        } => None,
        _ => Some(false),
    }
}

fn record(log: &mut Log, session: &str, (path, sha256, utf8_bytes): (String, String, usize)) {
    if log.writes.len() >= MAX_WRITES
        && let Some(oldest) = log
            .writes
            .iter()
            .min_by_key(|(_, (_, at))| *at)
            .map(|(key, _)| key.clone())
    {
        log.writes.remove(&oldest);
    }
    log.sequence += 1;
    let write = Write {
        sequence: log.sequence,
        sha256,
        utf8_bytes,
    };
    log.writes
        .insert((session.to_owned(), path), (write, Instant::now()));
}

pub(super) fn observe(session: &str, command: &Command, response: &Response) {
    match command {
        Command::Invoke { invocation, .. } if invocation.method == "fs/writeFile" => {
            let Some(identity) = identity(&invocation.params) else {
                return;
            };
            let mut log = LOG.lock();
            match completed(response) {
                Some(true) => record(&mut log, session, identity),
                Some(false) => {}
                None if log.pending.len() < MAX_PENDING => {
                    log.pending.insert(
                        (session.to_owned(), invocation.operation_id.clone()),
                        identity,
                    );
                }
                None => {}
            }
        }
        Command::Invoke { invocation, .. } if invocation.method == "process/start" => {
            let Some((process, path, expect)) = helper_request(&invocation.params) else {
                return;
            };
            let mut log = LOG.lock();
            if completed(response) == Some(false) || log.helpers.len() >= MAX_PENDING {
                return;
            }
            log.helpers
                .insert((session.to_owned(), process), (path, expect, String::new()));
        }
        Command::Invoke { invocation, .. } if invocation.method == "process/read" => {
            let Some(process) = invocation.params["processId"].as_str() else {
                return;
            };
            let mut log = LOG.lock();
            if !log
                .helpers
                .contains_key(&(session.to_owned(), process.to_owned()))
            {
                return;
            }
            match response {
                Response::Call {
                    response:
                        KeeperResponse::Operation {
                            outcome: Outcome::Completed { reply },
                        },
                } => helper_output(&mut log, session, process, reply),
                // A waiting read answers later through its operation ID.
                Response::Call {
                    response:
                        KeeperResponse::Operation {
                            outcome: Outcome::Pending,
                        },
                } if log.reads.len() < MAX_PENDING => {
                    log.reads.insert(
                        (session.to_owned(), invocation.operation_id.clone()),
                        process.to_owned(),
                    );
                }
                _ => {}
            }
        }
        Command::Observe { operation_id, .. } => {
            let mut log = LOG.lock();
            let key = (session.to_owned(), operation_id.clone());
            if completed(response).is_some()
                && let Some(process) = log.reads.remove(&key)
            {
                if let Response::Call {
                    response:
                        KeeperResponse::Operation {
                            outcome: Outcome::Completed { reply },
                        },
                } = response
                {
                    helper_output(&mut log, session, &process, reply);
                }
                return;
            }
            match completed(response) {
                None => {}
                Some(done) => {
                    if let Some(identity) = log.pending.remove(&key)
                        && done
                    {
                        record(&mut log, session, identity);
                    }
                }
            }
        }
        _ => {}
    }
}

/// The latest observed write of an absolute target path in this session.
pub(in crate::server) fn latest(session: &str, path: &str) -> Option<Write> {
    LOG.lock()
        .writes
        .get(&(session.to_owned(), path.to_owned()))
        .map(|(write, _)| write.clone())
}

#[cfg(test)]
mod tests;
