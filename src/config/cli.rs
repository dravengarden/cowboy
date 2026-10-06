//! `cowboy config`: inspect, validate and safely change Service or Device
//! configuration. Every command accepts `--format json` with stable fields so
//! agents can act on the result without parsing prose.

use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::Context as _;
use clap::{Args, Subcommand, ValueEnum};

use super::{Config, Scope, check_file, check_text, closest, example, install, json_schema};

#[derive(Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the configuration file path that would be used.
    Path(Target),
    /// Validate a configuration file without changing anything. Exit status 1
    /// means errors; use `--file <candidate>` as a pre-flight before applying
    /// or before a rolling update replaces a running instance.
    Check(Target),
    /// Print every effective value and whether it came from the file or the
    /// default.
    Show(Target),
    /// Describe one setting, or list all settings of a scope.
    Explain {
        #[arg(long, value_enum)]
        scope: Scope,
        /// Dotted key such as `capacity.max_sessions`.
        key: Option<String>,
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Print the machine-readable schema of a scope.
    Schema {
        #[arg(long, value_enum)]
        scope: Scope,
    },
    /// Write a commented template listing every setting at its default.
    Init {
        #[command(flatten)]
        target: Target,
        /// Print the template instead of writing it.
        #[arg(long)]
        print: bool,
    },
    /// Show what a candidate file would change and which changes need a restart.
    Diff {
        #[command(flatten)]
        target: Target,
        #[arg(long)]
        candidate: PathBuf,
    },
    /// Validate a candidate, keep the current file in `config/history/`, and
    /// atomically install the candidate. A running process applies live
    /// settings within seconds; restart settings wait for the next restart.
    Apply {
        #[command(flatten)]
        target: Target,
        #[arg(long)]
        candidate: PathBuf,
    },
}

#[derive(Args)]
struct Target {
    /// Which configuration: the Service (Controller) or this Device (Machine).
    #[arg(long, value_enum)]
    scope: Scope,
    /// Explicit configuration file. Defaults to
    /// `<data-dir>/config/cowboy-service.toml` or
    /// `<state-dir>/config/cowboy-device.toml`.
    #[arg(long)]
    file: Option<PathBuf>,
    /// Service data directory, the same as `cowboy serve --data-dir`.
    #[arg(long, env = "COWBOY_DATA_DIR", default_value = "/var/lib/cowboy")]
    data_dir: PathBuf,
    /// Device state directory, the same as `cowboy-machine --state-dir`.
    #[arg(long, env = "COWBOY_MACHINE_STATE_DIR")]
    state_dir: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
}

impl Target {
    fn path(&self) -> anyhow::Result<PathBuf> {
        if let Some(file) = &self.file {
            return Ok(file.clone());
        }
        match self.scope {
            Scope::Service => Ok(self.scope.path_in(&self.data_dir)),
            Scope::Device => self
                .state_dir
                .as_ref()
                .map(|root| self.scope.path_in(root))
                .context("a Device configuration needs --state-dir (or COWBOY_MACHINE_STATE_DIR) or --file"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Text,
    Json,
}

impl ConfigArgs {
    /// # Errors
    /// When the configuration is invalid (after printing the full report) or
    /// a file cannot be read or written.
    pub fn run(self) -> anyhow::Result<()> {
        match self.command {
            ConfigCommand::Path(target) => {
                let path = target.path()?;
                print(target.format, &serde_json::json!({ "path": path }), || {
                    format!("{}\n", path.display())
                });
                Ok(())
            }
            ConfigCommand::Check(target) => {
                let report = check_file(target.scope, &target.path()?);
                print(target.format, &report.to_json(), || report.render_text());
                anyhow::ensure!(report.is_valid(), "configuration is invalid");
                Ok(())
            }
            ConfigCommand::Show(target) => {
                let report = check_file(target.scope, &target.path()?);
                let Some(config) = &report.config else {
                    print(target.format, &report.to_json(), || report.render_text());
                    anyhow::bail!("configuration is invalid");
                };
                print(target.format, &config.to_json(), || render_show(config));
                Ok(())
            }
            ConfigCommand::Explain { scope, key, format } => explain(scope, key.as_deref(), format),
            ConfigCommand::Schema { scope } => {
                println!("{}", serde_json::to_string_pretty(&json_schema(scope))?);
                Ok(())
            }
            ConfigCommand::Init { target, print } => {
                let text = example(target.scope);
                if print {
                    print!("{text}");
                    return Ok(());
                }
                let path = target.path()?;
                anyhow::ensure!(
                    !path.exists(),
                    "{} already exists; edit it, or use `cowboy config apply --candidate`",
                    path.display()
                );
                install(target.scope, &path, &text)?;
                println!("wrote {}", path.display());
                Ok(())
            }
            ConfigCommand::Diff { target, candidate } => {
                let (current, next, report) = compare(&target, &candidate)?;
                let changes = current.changes(&next);
                print(
                    target.format,
                    &serde_json::json!({ "changes": changes, "diagnostics": report.diagnostics }),
                    || render_changes(&changes),
                );
                Ok(())
            }
            ConfigCommand::Apply { target, candidate } => {
                let (current, next, _) = compare(&target, &candidate)?;
                let text = std::fs::read_to_string(&candidate)
                    .with_context(|| format!("reading {}", candidate.display()))?;
                let path = target.path()?;
                install(target.scope, &path, &text)?;
                let changes = current.changes(&next);
                print(
                    target.format,
                    &serde_json::json!({ "installed": path, "changes": changes }),
                    || format!("installed {}\n{}", path.display(), render_changes(&changes)),
                );
                Ok(())
            }
        }
    }
}

/// Current effective configuration and a validated candidate. Refuses (with
/// the full report) when either is invalid.
fn compare(
    target: &Target,
    candidate: &std::path::Path,
) -> anyhow::Result<(Config, Config, super::Report)> {
    let current = check_file(target.scope, &target.path()?);
    let rendered = current.render_text();
    let current = current.config.with_context(|| {
        format!("the current configuration is invalid; fix it first\n{rendered}")
    })?;
    let text = std::fs::read_to_string(candidate)
        .with_context(|| format!("reading {}", candidate.display()))?;
    let report = check_text(target.scope, &text, Some(candidate.to_owned()));
    let Some(next) = report.config.clone() else {
        print(target.format, &report.to_json(), || report.render_text());
        anyhow::bail!("candidate configuration is invalid");
    };
    Ok((current, next, report))
}

fn explain(scope: Scope, key: Option<&str>, format: Format) -> anyhow::Result<()> {
    let schema = json_schema(scope);
    let fields = schema["fields"].as_array().cloned().unwrap_or_default();
    let selected = match key {
        None => fields,
        Some(key) => {
            let matching = fields
                .into_iter()
                .filter(|field| {
                    field["key"]
                        .as_str()
                        .is_some_and(|name| name == key || name.starts_with(&format!("{key}.")))
                })
                .collect::<Vec<_>>();
            if matching.is_empty() {
                let known = scope
                    .fields()
                    .iter()
                    .map(|field| field.key)
                    .collect::<Vec<_>>();
                let hint = closest(key, &known)
                    .map_or_else(String::new, |found| format!("; did you mean `{found}`?"));
                anyhow::bail!("`{key}` is not a {} setting{hint}", scope.as_str());
            }
            matching
        }
    };
    print(format, &serde_json::Value::Array(selected.clone()), || {
        selected
            .iter()
            .map(|field| {
                format!(
                    "{}\n  {}\n  accepts: {}\n  default: {}\n  reload: {}\n",
                    field["key"].as_str().unwrap_or_default(),
                    field["description"]
                        .as_str()
                        .unwrap_or_default()
                        .replace('\n', "\n  "),
                    field["accepts"].as_str().unwrap_or_default(),
                    field["default"],
                    field["reload"].as_str().unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    Ok(())
}

fn render_show(config: &Config) -> String {
    let mut out = String::new();
    for (field, value, source) in config.entries() {
        let source = match source {
            super::Source::Default => "default",
            super::Source::File => "file",
        };
        let _ = writeln!(out, "{} = {}  # {source}", field.key, value.to_toml());
    }
    out
}

fn render_changes(changes: &[super::Change]) -> String {
    if changes.is_empty() {
        return "no effective changes\n".to_owned();
    }
    let mut out = String::new();
    for change in changes {
        let reload = match change.reload {
            super::Reload::Live => "applies live",
            super::Reload::Restart => "needs a restart",
        };
        let _ = writeln!(
            out,
            "{}: {} -> {}  ({reload})",
            change.key, change.before, change.after
        );
    }
    out
}

fn print(format: Format, json: &serde_json::Value, text: impl FnOnce() -> String) {
    match format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(json).unwrap_or_else(|_| json.to_string())
        ),
        Format::Text => print!("{}", text()),
    }
}
