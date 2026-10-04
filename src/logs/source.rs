use super::data::{Coverage, Entry, MAX_SCAN, Metrics, Page, Query};
use super::storage::{SqliteStore, private, private_read};
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::io::{BufRead as _, BufReader, Read as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Read authority is the host account. Remote implementations run the identical
/// bounded query at the source; they never copy a database or ship an idle feed.
pub(crate) trait LogSource {
    fn query(&self, query: &Query) -> Result<Page>;
    fn metrics(&self, query: &Query) -> Result<Metrics>;
    fn status(&self) -> Result<Value>;
}
impl LogSource for SqliteStore {
    fn query(&self, query: &Query) -> Result<Page> {
        SqliteStore::query(self, query)
    }
    fn metrics(&self, query: &Query) -> Result<Metrics> {
        SqliteStore::metrics(self, query)
    }
    fn status(&self) -> Result<Value> {
        SqliteStore::status(self)
    }
}

pub(crate) struct JsonlSource {
    pub directory: PathBuf,
}
impl JsonlSource {
    fn scan(&self, query: &Query) -> Result<(Vec<Entry>, Coverage)> {
        query.validate()?;
        ensure!(
            query.after.is_none(),
            "legacy JSONL does not support stable cursors; narrow the time window"
        );
        private(&self.directory, true)?;
        let mut entries = Vec::new();
        let mut coverage = Coverage::default();
        let start = Instant::now();
        let mut bytes = 0;
        for index in 0..32 {
            let path = self.directory.join(if index == 0 {
                "telemetry.jsonl".into()
            } else {
                format!("telemetry.jsonl.{index}")
            });
            if !path.try_exists()? {
                continue;
            }
            let before = private(&path, false)?;
            coverage.segments += 1;
            coverage.retained_bytes += before.len();
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&path)?;
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            let mut offset = 0u64;
            loop {
                line.clear();
                let n = std::io::Read::by_ref(&mut reader)
                    .take(super::data::MAX_RECORD_BYTES as u64 + 1)
                    .read_line(&mut line)?;
                if n == 0 {
                    break;
                }
                bytes += n;
                if coverage.scanned >= MAX_SCAN
                    || bytes > 16 * 1024 * 1024
                    || start.elapsed() > Duration::from_secs(5)
                {
                    coverage.truncated = true;
                    coverage.issues.push("legacy_scan_budget_exhausted".into());
                    return Ok((entries, coverage));
                }
                coverage.scanned += 1;
                let start_offset = offset;
                offset += n as u64;
                if n > super::data::MAX_RECORD_BYTES || !line.ends_with('\n') {
                    coverage
                        .issues
                        .push("legacy_partial_or_oversized_record".into());
                    break;
                }
                let row: Result<Value> = serde_json::from_str(&line).context("legacy JSON");
                let observed = row
                    .as_ref()
                    .ok()
                    .and_then(|r| r["_cowboy_observed_ms"].as_i64());
                let entry = row
                    .context("legacy JSON")
                    .and_then(|v| Entry::legacy(&v, super::now_ms()));
                let Ok(mut entry) = entry else {
                    if !coverage
                        .issues
                        .iter()
                        .any(|s| s == "legacy_unreadable_record")
                    {
                        coverage.issues.push("legacy_unreadable_record".into());
                    }
                    continue;
                };
                // Old files have no durable ingestion timestamps. Expose this
                // limitation, and use the preserved source time for filtering.
                entry.observed_ms = observed.unwrap_or(entry.timestamp_ms);
                let mut identity = Sha256::new();
                identity.update(before.dev().to_le_bytes());
                identity.update(before.ino().to_le_bytes());
                identity.update(start_offset.to_le_bytes());
                identity.update(line.as_bytes());
                entry.id = super::data::hex(&identity.finalize()[..16]);
                entry.project_attributes()?;
                if matches(query, &entry) {
                    entries.push(entry);
                }
            }
            if private(&path, false).is_err()
                || before.modified().ok()
                    != std::fs::metadata(&path)
                        .ok()
                        .and_then(|m| m.modified().ok())
            {
                coverage
                    .issues
                    .push("legacy_segment_changed_during_read".into());
            }
        }
        coverage
            .issues
            .push("legacy_source_clock_and_best_effort_snapshot".into());
        entries.sort_by(|a, b| (b.observed_ms, &b.id).cmp(&(a.observed_ms, &a.id)));
        coverage.oldest_observed_ms = entries.last().map(|e| e.observed_ms);
        coverage.newest_observed_ms = entries.first().map(|e| e.observed_ms);
        Ok((entries, coverage))
    }
}
fn matches(q: &Query, e: &Entry) -> bool {
    q.id.as_ref().is_none_or(|id| id == &e.id)
        && e.observed_ms >= q.from_ms
        && e.observed_ms < q.to_ms
        && e.severity >= q.min_severity
        && [
            (&q.session, &e.session),
            (&q.machine, &e.machine),
            (&q.environment, &e.environment),
            (&q.trace_id, &e.trace_id),
            (&q.event, &e.event),
            (&q.service, &e.service),
            (&q.signal, &e.signal),
        ]
        .iter()
        .all(|(filter, value)| filter.as_ref().is_none_or(|s| s == *value))
        && q.contains
            .as_ref()
            .is_none_or(|s| e.body.contains(s) || e.event.contains(s))
}
impl LogSource for JsonlSource {
    fn query(&self, q: &Query) -> Result<Page> {
        let (mut items, mut coverage) = self.scan(q)?;
        coverage.truncated |= items.len() > q.limit;
        items.truncate(q.limit);
        if !q.include_protobuf {
            for e in &mut items {
                e.protobuf.clear();
            }
        }
        Ok(Page {
            schema: "cowboy.logs.page/v1".into(),
            items,
            next_cursor: None,
            coverage,
        })
    }
    fn metrics(&self, q: &Query) -> Result<Metrics> {
        let (entries, coverage) = self.scan(q)?;
        let mut groups = std::collections::BTreeMap::new();
        for e in entries {
            super::analysis::accumulate(&mut groups, &e);
        }
        Ok(Metrics {
            schema: "cowboy.logs.metrics/v1".into(),
            from_ms: q.from_ms,
            to_ms: q.to_ms,
            groups,
            coverage,
        })
    }
    fn status(&self) -> Result<Value> {
        private(&self.directory, true)?;
        Ok(
            json!({"backend":"legacy_jsonl","directory":self.directory,"live_health":"unknown","read_only":true}),
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Backend {
    #[default]
    Sqlite,
    Jsonl,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Source {
    pub name: String,
    pub directory: PathBuf,
    #[serde(default)]
    pub backend: Backend,
    #[serde(default)]
    pub ssh: Option<String>,
    #[serde(default)]
    pub ssh_config: Option<PathBuf>,
    /// Explicit use of the host's existing noninteractive sudo delegation.
    #[serde(default)]
    pub sudo_user: Option<String>,
    #[serde(default = "default_command")]
    pub command: String,
}
pub(crate) fn default_command() -> String {
    "cowboy".into()
}
impl Source {
    fn local(&self) -> Result<Box<dyn LogSource>> {
        Ok(match self.backend {
            Backend::Sqlite => Box::new(SqliteStore::open(self.directory.clone(), false)?),
            Backend::Jsonl => Box::new(JsonlSource {
                directory: self.directory.clone(),
            }),
        })
    }
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.is_empty() && self.name.len() <= 128 && self.directory.is_absolute(),
            "invalid diagnostic source"
        );
        if let Some(alias) = &self.ssh {
            ensure!(
                !alias.is_empty()
                    && alias.len() <= 128
                    && !alias.starts_with('-')
                    && alias
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"@._-".contains(&b)),
                "SSH source requires a stable alias"
            );
        }
        if let Some(user) = &self.sudo_user {
            ensure!(
                self.ssh.is_some()
                    && !user.is_empty()
                    && user.len() <= 64
                    && !user.starts_with('-')
                    && user
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                "sudo-user requires an SSH source and a plain host account name"
            );
        }
        ensure!(
            self.command == "cowboy"
                || (self.command.starts_with('/')
                    && self.command.len() <= 4096
                    && !self.command.chars().any(char::is_control)),
            "remote command must be cowboy or an absolute executable path"
        );
        Ok(())
    }
    pub(crate) async fn execute(&self, operation: Operation) -> Result<Value> {
        self.validate()?;
        if self.ssh.is_some() {
            return self.remote(operation).await;
        }
        let source = self.clone();
        tokio::task::spawn_blocking(move || execute_local(&source, operation)).await?
    }
    async fn remote(&self, operation: Operation) -> Result<Value> {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut command = tokio::process::Command::new("ssh");
        command.args([
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=5",
            "-o",
            "ConnectionAttempts=1",
            "-o",
            "ServerAliveInterval=5",
            "-o",
            "ServerAliveCountMax=1",
        ]);
        if let Some(config) = &self.ssh_config {
            command.arg("-F").arg(config);
        }
        let prefix = self
            .sudo_user
            .as_ref()
            .map(|user| format!("sudo -n -u {} -- ", quote(user)))
            .unwrap_or_default();
        command
            .arg("--")
            .arg(self.ssh.as_ref().context("remote source missing")?)
            .arg(format!("{prefix}{} logs rpc", quote(&self.command)));
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().context("starting remote log reader")?;
        let mut source = self.clone();
        source.ssh = None;
        source.ssh_config = None;
        source.sudo_user = None;
        let request = serde_json::to_vec(&Rpc {
            schema: 1,
            source,
            operation,
        })?;
        ensure!(request.len() <= 16 * 1024, "remote query exceeds limit");
        let mut input = child.stdin.take().context("remote reader input missing")?;
        let output = child
            .stdout
            .take()
            .context("remote reader output missing")?;
        let result = tokio::time::timeout(Duration::from_secs(20), async {
            input.write_all(&request).await?;
            input.shutdown().await?;
            drop(input);
            let mut bytes = Vec::new();
            output
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .await?;
            ensure!(
                bytes.len() <= 8 * 1024 * 1024,
                "remote response exceeds limit"
            );
            ensure!(
                child.wait().await?.success(),
                "remote diagnostic query failed"
            );
            let reply: Reply =
                serde_json::from_slice(&bytes).context("invalid remote diagnostic reply")?;
            ensure!(reply.schema == 1, "unsupported remote diagnostic protocol");
            reply.result.ok_or_else(|| {
                anyhow::anyhow!(
                    "remote source unavailable: {}",
                    reply.error.unwrap_or_else(|| "unknown".into())
                )
            })
        })
        .await;
        match result {
            Ok(value) => value,
            Err(_) => {
                let _ = child.kill().await;
                anyhow::bail!("remote diagnostic query timed out")
            }
        }
    }
}
pub(crate) fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Operation {
    Query {
        query: Query,
    },
    Metrics {
        query: Query,
    },
    Analyze {
        query: Query,
        baseline: Option<Box<Query>>,
    },
    Status,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rpc {
    pub schema: u16,
    pub source: Source,
    pub operation: Operation,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Reply {
    pub schema: u16,
    pub result: Option<Value>,
    pub error: Option<String>,
}

pub(crate) fn execute_local(source: &Source, operation: Operation) -> Result<Value> {
    source.validate()?;
    ensure!(
        source.ssh.is_none() && source.ssh_config.is_none(),
        "recursive remote queries are forbidden"
    );
    let reader = source.local()?;
    match operation {
        Operation::Query { query } => Ok(serde_json::to_value(reader.query(&query)?)?),
        Operation::Metrics { query } => Ok(serde_json::to_value(reader.metrics(&query)?)?),
        Operation::Analyze { query, baseline } => Ok(super::analysis::analyze(
            reader.metrics(&query)?,
            baseline.as_ref().map(|q| reader.metrics(q)).transpose()?,
        )),
        Operation::Status => reader.status(),
    }
}

pub(crate) fn load_sources(path: &std::path::Path) -> Result<Vec<Source>> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct File {
        schema: u16,
        sources: Vec<Source>,
    }
    let file: File = serde_json::from_slice(&private_read(path, 64 * 1024)?)
        .context("invalid diagnostic sources file")?;
    ensure!(
        file.schema == 1 && (1..=16).contains(&file.sources.len()),
        "invalid source count or schema"
    );
    let mut names = std::collections::BTreeSet::new();
    for source in &file.sources {
        source.validate()?;
        ensure!(names.insert(&source.name), "duplicate source name");
    }
    Ok(file.sources)
}
