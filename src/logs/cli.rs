use super::{
    data::{Level, Query, now_ms},
    source::{Backend, Operation, Reply, Rpc, Source},
    storage::{Policy, SqliteStore},
};
use anyhow::{Context as _, Result, ensure};
use clap::{Args, Subcommand};
use prost::Message as _;
use serde_json::json;
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::time::Duration;

pub(crate) fn command_schema() -> serde_json::Value {
    fn describe(command: &clap::Command) -> serde_json::Value {
        json!({"name":command.get_name(),"description":command.get_about().map(ToString::to_string),
            "arguments":command.get_arguments().map(|a|json!({
                "name":a.get_id().as_str(),"long":a.get_long(),"help":a.get_help().map(ToString::to_string),
                "required":a.is_required_set(),"global":a.is_global_set(),
                "action":format!("{:?}",a.get_action()),
                "defaults":a.get_default_values().iter().map(|s|s.to_string_lossy()).collect::<Vec<_>>(),
                "values":a.get_value_parser().possible_values().map(|v|v.map(|p|p.get_name().to_owned()).collect::<Vec<_>>())
            })).collect::<Vec<_>>(),
            "commands":command.get_subcommands().map(describe).collect::<Vec<_>>()})
    }
    let mut command = LogsArgs::augment_args(clap::Command::new("logs"));
    command.build();
    let mut schema = super::data::schema();
    schema["cli"] = describe(&command);
    schema["exit_codes"] = json!({"success":0,"error":1,"allow_partial":"source failures remain in JSON even when explicitly allowed"});
    schema
}

#[derive(Args)]
pub struct LogsArgs {
    /// Private local store, or source-side directory when --ssh is selected.
    #[arg(long, global = true, conflicts_with = "sources")]
    directory: Option<PathBuf>,
    /// Stable SSH alias. Filtering and aggregation run on this host.
    #[arg(long, global = true, conflicts_with = "sources")]
    ssh: Option<String>,
    /// Use this existing SSH configuration, preserving its host-key policy.
    #[arg(long, global = true, requires = "ssh")]
    ssh_config: Option<PathBuf>,
    /// Read as this service account through existing sudo -n host delegation.
    #[arg(long, global = true, requires = "ssh")]
    sudo_user: Option<String>,
    /// Installed source-side Cowboy executable (cowboy or an absolute path).
    #[arg(long, global = true, default_value = "cowboy")]
    remote_command: String,
    /// Private schema-1 source file. At most 16 local/SSH sources; no background shipping.
    #[arg(long, global = true)]
    sources: Option<PathBuf>,
    /// Read a retained legacy Controller JSONL directory through the same query port.
    #[arg(long, global = true, value_enum, default_value = "sqlite")]
    backend: Backend,
    /// Exit successfully when a requested source is unavailable (reported in JSON).
    #[arg(long, global = true)]
    allow_partial: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Machine-readable command schema, bounds, semantics and metric definitions.
    Schema,
    /// Search retained OTel records, with bounded pages and an opaque cursor.
    Query(QueryArgs),
    /// Compute counts, rates and duration histograms at each source.
    Metrics(QueryArgs),
    /// Compare with the preceding equal time window and return evidence-linked findings.
    Analyze(QueryArgs),
    /// Poll recent records as JSONL; every frame reports gaps and truncation.
    Tail(WatchArgs),
    /// Periodic, read-only analysis. Use a native scheduler to persist execution.
    Watch(WatchArgs),
    /// Inspect retention, writer losses/failures and maintenance state.
    Status,
    /// Configure this local store. All active writers reload it within 30 seconds.
    Configure(ConfigArgs),
    /// Enforce local expiry/capacity now; may run periodically under a host scheduler.
    Maintain,
    /// Export one bounded page as an official OTLP protobuf request on stdout.
    Export {
        #[command(flatten)]
        query: QueryArgs,
        #[arg(long,value_parser=["logs","metrics","traces"])]
        signal: String,
    },
    /// Bounded read-only JSON RPC over stdin/stdout, used by the SSH source adapter.
    Rpc,
}

#[derive(Clone, Args)]
struct QueryArgs {
    /// Exact evidence ID returned by query/analysis.
    #[arg(long)]
    id: Option<String>,
    /// Relative lookback (30m, 1h, 7d), Unix milliseconds, or RFC3339 timestamp.
    #[arg(long, default_value = "1h")]
    from: String,
    #[arg(long, default_value = "now")]
    to: String,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    machine: Option<String>,
    #[arg(long)]
    environment: Option<String>,
    #[arg(long)]
    trace_id: Option<String>,
    #[arg(long)]
    event: Option<String>,
    #[arg(long)]
    service: Option<String>,
    #[arg(long)]
    contains: Option<String>,
    #[arg(long, value_enum)]
    level: Option<Level>,
    #[arg(long, default_value_t = 100)]
    limit: usize,
    /// Continue with the same filters; the cursor preserves the original time window.
    #[arg(long)]
    after: Option<String>,
}
impl QueryArgs {
    fn query(&self) -> Result<Query> {
        let now = now_ms();
        let query = Query {
            id: self.id.clone(),
            from_ms: timestamp(&self.from, now)?,
            to_ms: timestamp(&self.to, now)?,
            session: self.session.clone(),
            machine: self.machine.clone(),
            environment: self.environment.clone(),
            trace_id: self.trace_id.clone(),
            event: self.event.clone(),
            service: self.service.clone(),
            signal: None,
            min_severity: self.level.map_or(0, Level::number),
            contains: self.contains.clone(),
            limit: self.limit,
            after: self.after.clone(),
            include_protobuf: false,
        };
        query.validate()?;
        Ok(query)
    }
}
#[derive(Args)]
struct WatchArgs {
    #[command(flatten)]
    query: QueryArgs,
    /// Poll interval; at least one second. Remote watch transfers summaries only.
    #[arg(long, default_value = "5m")]
    every: String,
    /// Zero runs until interrupted; a positive value bounds the number of frames.
    #[arg(long, default_value_t = 0)]
    iterations: u64,
}
#[derive(Args)]
struct ConfigArgs {
    #[arg(long, default_value = "7d")]
    retain: String,
    /// Defaults to the shorter of one day and the configured retention.
    #[arg(long)]
    rotate: Option<String>,
    #[arg(long,default_value_t=8*1024*1024)]
    segment_bytes: u64,
    #[arg(long,default_value_t=128*1024*1024)]
    max_bytes: u64,
    /// Also return sampled runtime spans through the Controller. Default: local only.
    #[arg(long, default_value_t = false)]
    forward_runtime: bool,
}

pub(crate) fn duration(value: &str) -> Result<Duration> {
    let (digits, multiplier) = if let Some(v) = value.strip_suffix('s') {
        (v, 1)
    } else if let Some(v) = value.strip_suffix('m') {
        (v, 60)
    } else if let Some(v) = value.strip_suffix('h') {
        (v, 3600)
    } else if let Some(v) = value.strip_suffix('d') {
        (v, 86400)
    } else {
        anyhow::bail!("duration requires s, m, h or d suffix")
    };
    let seconds = digits
        .parse::<u64>()
        .ok()
        .and_then(|v| v.checked_mul(multiplier))
        .context("invalid duration")?;
    ensure!(
        (1..=90 * 86400).contains(&seconds),
        "duration must be between one second and 90 days"
    );
    Ok(Duration::from_secs(seconds))
}
fn timestamp(value: &str, now: i64) -> Result<i64> {
    if value == "now" {
        return Ok(now);
    }
    if let Ok(timestamp) = value.parse::<i64>() {
        return Ok(timestamp);
    }
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(time.timestamp_millis());
    }
    Ok(now - duration(value)?.as_millis() as i64)
}
fn baseline(query: &Query) -> Option<Box<Query>> {
    let width = query.to_ms - query.from_ms;
    if query.from_ms < width {
        return None;
    }
    let mut old = query.clone();
    old.to_ms = query.from_ms;
    old.from_ms -= width;
    old.after = None;
    Some(Box::new(old))
}
fn default_directory() -> Result<PathBuf> {
    if let Some(directory) = std::env::var_os("COWBOY_LOGS_DIR") {
        return Ok(directory.into());
    }
    if let Some(state) = std::env::var_os("COWBOY_MACHINE_STATE_DIR") {
        return Ok(PathBuf::from(state).join("logs"));
    }
    if let Some(state) = std::env::var_os("COWBOY_DATA_DIR") {
        return Ok(PathBuf::from(state).join("logs"));
    }
    Ok(
        PathBuf::from(std::env::var_os("HOME").context("set --directory for logs")?)
            .join(".local/state/cowboy-logs"),
    )
}

impl LogsArgs {
    fn sources(&self) -> Result<Vec<Source>> {
        if let Some(path) = &self.sources {
            return super::source::load_sources(path);
        }
        ensure!(
            self.ssh.is_none() || self.directory.is_some(),
            "remote queries require an explicit source-side --directory"
        );
        let source = Source {
            name: self.ssh.clone().unwrap_or_else(|| "local".into()),
            directory: self
                .directory
                .clone()
                .map(Ok)
                .unwrap_or_else(default_directory)?,
            backend: self.backend.clone(),
            ssh: self.ssh.clone(),
            ssh_config: self.ssh_config.clone(),
            sudo_user: self.sudo_user.clone(),
            command: self.remote_command.clone(),
        };
        source.validate()?;
        Ok(vec![source])
    }
    /// Execute one bounded diagnostic command.
    ///
    /// # Errors
    /// Invalid options, unavailable sources and incomplete exports return an error.
    pub async fn run(self) -> Result<()> {
        if matches!(self.command, Command::Schema) {
            println!("{}", command_schema());
            return Ok(());
        }
        if matches!(self.command, Command::Rpc) {
            return rpc();
        }
        let sources = self.sources()?;
        match self.command {
            Command::Schema | Command::Rpc => unreachable!(),
            Command::Query(args) => {
                report(
                    &sources,
                    Operation::Query {
                        query: args.query()?,
                    },
                    self.allow_partial,
                )
                .await
            }
            Command::Metrics(args) => {
                report(
                    &sources,
                    Operation::Metrics {
                        query: args.query()?,
                    },
                    self.allow_partial,
                )
                .await
            }
            Command::Analyze(args) => {
                let query = args.query()?;
                report(
                    &sources,
                    Operation::Analyze {
                        baseline: baseline(&query),
                        query,
                    },
                    self.allow_partial,
                )
                .await
            }
            Command::Status => report(&sources, Operation::Status, self.allow_partial).await,
            Command::Configure(args) => {
                let source = single_local(&sources)?;
                let retain_seconds = duration(&args.retain)?.as_secs();
                let policy = Policy {
                    schema: 1,
                    retain_seconds,
                    rotate_seconds: args
                        .rotate
                        .as_deref()
                        .map(duration)
                        .transpose()?
                        .map_or(retain_seconds.min(86400), |v| v.as_secs()),
                    segment_bytes: args.segment_bytes,
                    max_bytes: args.max_bytes,
                    forward_runtime: args.forward_runtime,
                };
                policy.validate()?;
                let store = SqliteStore::open(source.directory.clone(), true)?;
                store.configure(policy)?;
                store.maintain(now_ms())?;
                println!("{}", store.status()?);
                Ok(())
            }
            Command::Maintain => {
                let source = single_local(&sources)?;
                let store = SqliteStore::open(source.directory.clone(), false)?;
                store.maintain(now_ms())?;
                println!("{}", store.status()?);
                Ok(())
            }
            Command::Export { query, signal } => export(&sources, query.query()?, &signal).await,
            Command::Watch(args) => watch(&sources, args, false, self.allow_partial).await,
            Command::Tail(args) => watch(&sources, args, true, self.allow_partial).await,
        }
    }
}
fn single_local(sources: &[Source]) -> Result<&Source> {
    ensure!(
        sources.len() == 1
            && sources[0].ssh.is_none()
            && matches!(sources[0].backend, Backend::Sqlite),
        "configuration and maintenance require one local SQLite source"
    );
    Ok(&sources[0])
}
async fn report(sources: &[Source], operation: Operation, allow_partial: bool) -> Result<()> {
    let mut results = Vec::new();
    let mut complete = true;
    let mut bytes = 0usize;
    // Bounded source count, sequential network queries. No log is transferred
    // for metric/analysis commands; each remote host computes its own summary.
    for source in sources {
        let result = source.execute(operation.clone()).await.and_then(|value| {
            bytes += serde_json::to_vec(&value)?.len();
            ensure!(
                bytes <= 16 * 1024 * 1024,
                "combined report exceeds 16 MiB; narrow the query or select fewer sources"
            );
            Ok(value)
        });
        match result {
            Ok(result) => results.push(json!({"source":source.name,"result":result})),
            Err(error) => {
                complete = false;
                results.push(json!({"source":source.name,"error":{"code":"source_unavailable","message":error.to_string()},"complete":false}));
            }
        }
    }
    println!(
        "{}",
        json!({"schema":"cowboy.logs.report/v1","sources_complete":complete,"results":results})
    );
    ensure!(
        complete || allow_partial,
        "one or more diagnostic sources unavailable; see structured result"
    );
    Ok(())
}
async fn watch(sources: &[Source], args: WatchArgs, tail: bool, allow_partial: bool) -> Result<()> {
    ensure!(
        args.query.after.is_none(),
        "watch/tail cannot continue a historical page cursor"
    );
    let interval = duration(&args.every)?;
    let mut iteration = 0;
    let mut seen = std::collections::BTreeMap::new();
    loop {
        let query = args.query.query()?;
        if tail {
            tail_frame(sources, query, &mut seen, allow_partial).await?;
        } else {
            report(
                sources,
                Operation::Analyze {
                    baseline: baseline(&query),
                    query,
                },
                allow_partial,
            )
            .await?;
        }
        std::io::stdout().flush()?;
        iteration += 1;
        if args.iterations > 0 && iteration >= args.iterations {
            return Ok(());
        }
        tokio::select! {_=tokio::time::sleep(interval)=>{},_=tokio::signal::ctrl_c()=>return Ok(())}
    }
}
async fn tail_frame(
    sources: &[Source],
    query: Query,
    seen: &mut std::collections::BTreeMap<(String, String), i64>,
    allow_partial: bool,
) -> Result<()> {
    seen.retain(|_, at| *at >= query.from_ms);
    let mut results = Vec::new();
    let mut complete = true;
    let mut bytes = 0usize;
    for source in sources {
        let mut q = query.clone();
        let mut pages = Vec::new();
        let mut failure = None;
        for index in 0..16 {
            let result = source
                .execute(Operation::Query { query: q.clone() })
                .await
                .and_then(|v| Ok(serde_json::from_value::<super::data::Page>(v)?));
            let mut page = match result {
                Ok(page) => page,
                Err(error) => {
                    failure = Some(error.to_string());
                    complete = false;
                    break;
                }
            };
            q.after = page.next_cursor.clone();
            page.items
                .retain(|e| !seen.contains_key(&(source.name.clone(), e.id.clone())));
            let size = serde_json::to_vec(&page)?.len();
            if bytes.saturating_add(size) > 16 * 1024 * 1024 {
                page.items.clear();
                page.coverage.truncated = true;
                page.coverage
                    .issues
                    .push("tail_byte_budget_exhausted".into());
                pages.push(page);
                break;
            }
            bytes += size;
            for e in &page.items {
                if seen.len() < 100_000 {
                    seen.insert((source.name.clone(), e.id.clone()), e.observed_ms);
                } else if !page
                    .coverage
                    .issues
                    .iter()
                    .any(|s| s == "tail_dedup_budget_exhausted")
                {
                    page.coverage
                        .issues
                        .push("tail_dedup_budget_exhausted".into());
                }
            }
            if index == 15 && q.after.is_some() {
                page.coverage
                    .issues
                    .push("tail_page_budget_exhausted".into());
            }
            pages.push(page);
            if q.after.is_none() {
                break;
            }
        }
        results.push(json!({"source":source.name,"pages":pages,"error":failure}));
    }
    println!(
        "{}",
        json!({"schema":"cowboy.logs.tail/v1","sources_complete":complete,"results":results})
    );
    ensure!(
        complete || allow_partial,
        "one or more tail sources unavailable"
    );
    Ok(())
}
fn rpc() -> Result<()> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 16 * 1024, "log RPC request exceeds limit");
    let request: Rpc = serde_json::from_slice(&bytes).context("invalid log RPC request")?;
    ensure!(request.schema == 1, "unsupported log RPC schema");
    let result = super::source::execute_local(&request.source, request.operation);
    let reply = match result {
        Ok(result) => Reply {
            schema: 1,
            result: Some(result),
            error: None,
        },
        Err(e) => Reply {
            schema: 1,
            result: None,
            error: Some(e.to_string()),
        },
    };
    let bytes = serde_json::to_vec(&reply)?;
    ensure!(
        bytes.len() <= 8 * 1024 * 1024,
        "log RPC response exceeds limit; narrow the query"
    );
    std::io::stdout().write_all(&bytes)?;
    Ok(())
}
async fn export(sources: &[Source], mut query: Query, signal: &str) -> Result<()> {
    use opentelemetry_proto::tonic::collector::{
        logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
        trace::v1::ExportTraceServiceRequest,
    };
    ensure!(
        sources.len() == 1,
        "OTLP export selects one source at a time"
    );
    query.signal = Some(signal.into());
    query.include_protobuf = true;
    let value = sources[0].execute(Operation::Query { query }).await?;
    let page: super::data::Page = serde_json::from_value(value)?;
    ensure!(
        page.next_cursor.is_none() && !page.coverage.truncated,
        "export would be truncated; narrow the time window or raise --limit"
    );
    let bytes = match signal {
        "logs" => {
            let mut request = ExportLogsServiceRequest::default();
            for entry in page.items {
                request.resource_logs.extend(
                    ExportLogsServiceRequest::decode(entry.protobuf.as_slice())?.resource_logs,
                );
            }
            request.encode_to_vec()
        }
        "metrics" => {
            let mut request = ExportMetricsServiceRequest::default();
            for entry in page.items {
                request.resource_metrics.extend(
                    ExportMetricsServiceRequest::decode(entry.protobuf.as_slice())?
                        .resource_metrics,
                );
            }
            request.encode_to_vec()
        }
        "traces" => {
            let mut request = ExportTraceServiceRequest::default();
            for entry in page.items {
                request.resource_spans.extend(
                    ExportTraceServiceRequest::decode(entry.protobuf.as_slice())?.resource_spans,
                );
            }
            request.encode_to_vec()
        }
        _ => anyhow::bail!("unsupported OTLP signal"),
    };
    std::io::stdout().write_all(&bytes)?;
    Ok(())
}
