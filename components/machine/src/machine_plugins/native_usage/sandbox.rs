//! Native quota of an account-sandbox Provider (Antigravity). The account's
//! credential lives only in the daemon's account home, so no collector runs
//! as the Machine user: the daemon runs the account's own read-only quota
//! report in a `usage` unit and answers its validated projection, which this
//! module maps onto the generic `rateLimitsByLimitId` usage document every
//! account card already renders.
use super::*;
use crate::account_sandbox as daemon;
use serde_json::{Value, json};

impl MachinePluginStore {
    /// Collect through the daemon on the Provider's active generation
    /// `active_digest` (the account's credential does not belong to a
    /// release; the daemon checks its signed-in generation). `observation` (this account's usage
    /// guard) is held until the daemon answered, even when the admission
    /// deadline ends the wait first, so reports of one account never overlap.
    pub(super) async fn collect_sandbox_usage(
        &self,
        invocation: &Invocation,
        active_digest: &str,
        observation: tokio::sync::OwnedMutexGuard<()>,
    ) -> Result<Value> {
        let profile = invocation.profile.clone();
        let digest = active_digest.to_owned();
        let account = self
            .for_account(&profile.account_id)
            .context("native_usage_profile_changed")?;
        let report = tokio::time::timeout(
            invocation.remaining()?,
            tokio::task::spawn_blocking(move || {
                let _observation = observation;
                let mut connection =
                    account.account_sandbox_connection(&profile.provider_id, &digest)?;
                let collect = |client: &mut daemon::Client| {
                    client.collect_usage(
                        &connection.key,
                        profile.generation,
                        &connection.runtime_sha256,
                    )
                };
                match collect(&mut connection.client) {
                    // The daemon evicted this generation's runtime from its
                    // bounded store: register it again and retry once.
                    Err(error) if daemon::DaemonError::code_of(&error) == Some("not_found") => {
                        daemon::register_file(
                            &mut connection.client,
                            &connection.runtime_archive,
                            &connection.runtime_sha256,
                        )?;
                        collect(&mut connection.client)
                    }
                    result => result,
                }
            }),
        )
        .await
        .context("native_usage_busy")?
        .context("native_usage_collector_failed")?
        .map_err(|error| {
            let code = sandbox_error_code(&error);
            tracing::debug!(error = %format!("{error:#}"), code, "account sandbox usage failed");
            anyhow::anyhow!(code)
        })?;
        if report.outcome == "authentication_required" {
            bail!("native_usage_authentication_required");
        }
        if report.outcome == "location_unsupported" {
            bail!("native_usage_location_unsupported");
        }
        let quota = report.quota.context("native_usage_collector_failed")?;
        Ok(json!({"status": "available", "rate_limits": rate_limits(&quota)}))
    }

    /// Collect Antigravity quota directly via `agy-proxy --cowboy-usage`
    /// inside the account's persistent `native/home`, enforcing `ipv4_only_v1`
    /// on Linux via a transient `systemd-run --user` unit.
    pub(super) async fn collect_direct_antigravity_usage(
        &self,
        invocation: &Invocation,
        proxy_command: &Path,
        home: &Path,
        network: Option<cowboy_provider_sdk::RuntimeNetworkPolicy>,
        observation: tokio::sync::OwnedMutexGuard<()>,
    ) -> Result<Value> {
        let _observation = observation;
        let remaining = invocation.remaining()?.min(Duration::from_secs(25));
        let (mut cmd, mut unit_guard) =
            direct_antigravity_usage_command(proxy_command, home, network);
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().context("native_usage_collector_unavailable")?;
        let mut stdout = child
            .stdout
            .take()
            .context("native_usage_collector_failed")?;
        let read_task = tokio::spawn(async move {
            use tokio::io::AsyncReadExt as _;
            let mut buf = Vec::new();
            let limit = 256 * 1024_u64;
            let n = (&mut stdout).take(limit + 1).read_to_end(&mut buf).await?;
            ensure!(n as u64 <= limit, "usage report exceeded output limit");
            Ok::<Vec<u8>, anyhow::Error>(buf)
        });
        let status = tokio::time::timeout(remaining, child.wait())
            .await
            .map_err(|_| anyhow::anyhow!("native_usage_busy"))?
            .context("native_usage_collector_failed")?;
        unit_guard.disarm();
        let output = tokio::time::timeout(Duration::from_secs(2), read_task)
            .await
            .context("native_usage_collector_failed")?
            .context("native_usage_collector_failed")?
            .context("native_usage_collector_failed")?;
        match interpret_antigravity_usage(status.code(), &output)? {
            DirectUsageOutcome::AuthenticationRequired => {
                bail!("native_usage_authentication_required");
            }
            DirectUsageOutcome::LocationUnsupported => {
                bail!("native_usage_location_unsupported");
            }
            DirectUsageOutcome::Available(quota) => {
                Ok(json!({"status": "available", "rate_limits": rate_limits(&quota)}))
            }
        }
    }
}

struct UsageUnitGuard(Option<String>);

impl UsageUnitGuard {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for UsageUnitGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(unit) = self.0.take() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "stop", "--no-block", &unit])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

fn direct_antigravity_usage_command(
    proxy_command: &Path,
    home: &Path,
    network: Option<cowboy_provider_sdk::RuntimeNetworkPolicy>,
) -> (tokio::process::Command, UsageUnitGuard) {
    #[cfg(target_os = "linux")]
    if let Some(cowboy_provider_sdk::RuntimeNetworkPolicy::Ipv4OnlyV1) = network {
        let unit = format!("cowboy-usage-{}.service", uuid::Uuid::new_v4().simple());
        let mut cmd = tokio::process::Command::new("systemd-run");
        cmd.env_clear();
        for outer in ["PATH", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"] {
            if let Some(value) = std::env::var_os(outer) {
                cmd.env(outer, value);
            }
        }
        cmd.args([
            "--user",
            "--quiet",
            "--wait",
            "--pipe",
            "--collect",
            "--service-type=exec",
            "--property=KillMode=control-group",
            "--property=Restart=no",
            "--property=RuntimeMaxSec=30s",
            "--property=TimeoutStopSec=5s",
            "--property=RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK",
            &format!("--unit={unit}"),
            &format!("--working-directory={}", home.display()),
            &format!("--setenv=HOME={}", home.display()),
            "--setenv=AGY_CLI_DISABLE_AUTO_UPDATE=true",
            "--setenv=DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent/cowboy-antigravity-no-bus",
            "--setenv=LANG=C.UTF-8",
            "--setenv=TZ=UTC",
        ]);
        if let Ok(path) = std::env::var("PATH") {
            cmd.arg(format!("--setenv=PATH={path}"));
        }
        for proxy_var in [
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "NIX_SSL_CERT_FILE",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
        ] {
            if let Ok(value) = std::env::var(proxy_var) {
                cmd.arg(format!("--setenv={proxy_var}={value}"));
            }
        }
        cmd.arg("--").arg(proxy_command).arg("--cowboy-usage");
        return (cmd, UsageUnitGuard(Some(unit)));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = network;
    let mut cmd = tokio::process::Command::new(proxy_command);
    cmd.arg("--cowboy-usage")
        .env_clear()
        .env("HOME", home)
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent/cowboy-antigravity-no-bus",
        )
        .env("LANG", "C.UTF-8")
        .env("TZ", "UTC")
        .current_dir(home);
    for keep in [
        "PATH",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "NIX_SSL_CERT_FILE",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        if let Some(value) = std::env::var_os(keep) {
            cmd.env(keep, value);
        }
    }
    (cmd, UsageUnitGuard(None))
}

#[derive(Debug, PartialEq)]
enum DirectUsageOutcome {
    Available(Value),
    AuthenticationRequired,
    LocationUnsupported,
}

fn interpret_antigravity_usage(exit: Option<i32>, output: &[u8]) -> Result<DirectUsageOutcome> {
    let text = String::from_utf8_lossy(output);
    let envelope = text
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| line.starts_with('{'))
        .find_map(|line| serde_json::from_str::<serde_json::Map<String, Value>>(line).ok());
    let Some(envelope) = envelope else {
        if text.contains("Authentication required") || text.contains("not logged in") {
            return Ok(DirectUsageOutcome::AuthenticationRequired);
        }
        bail!("native_usage_collector_failed");
    };
    let succeeded = exit == Some(0)
        && envelope
            .get("status")
            .is_none_or(|status| status == "SUCCESS");
    if !succeeded {
        let error = envelope
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if [
            "log in",
            "logged in",
            "authenticat",
            "sign in",
            "credential",
        ]
        .iter()
        .any(|needle| error.contains(needle))
        {
            return Ok(DirectUsageOutcome::AuthenticationRequired);
        }
        if error.contains("location") {
            return Ok(DirectUsageOutcome::LocationUnsupported);
        }
        bail!("native_usage_collector_failed");
    }
    if envelope
        .get("num_turns")
        .is_some_and(|turns| turns.as_u64() != Some(0))
    {
        bail!("native_usage_collector_failed");
    }
    let command = envelope
        .get("command")
        .and_then(Value::as_object)
        .context("native_usage_collector_failed")?;
    if command
        .get("name")
        .and_then(Value::as_str)
        .map(|name| name.trim_start_matches('/'))
        != Some("usage")
    {
        bail!("native_usage_collector_failed");
    }
    let document = match command
        .get("data")
        .context("native_usage_collector_failed")?
    {
        Value::Object(_) => command["data"].clone(),
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .filter(Value::is_object)
            .context("native_usage_collector_failed")?,
        _ => bail!("native_usage_collector_failed"),
    };
    let quota = project_antigravity_quota(&document).context("native_usage_collector_failed")?;
    Ok(DirectUsageOutcome::Available(quota))
}

fn project_antigravity_quota(document: &Value) -> Result<Value> {
    const MAX_GROUPS: usize = 32;
    const MAX_BUCKETS: usize = 128;
    const MAX_LABEL: usize = 80;
    let groups = document
        .get("groups")
        .and_then(Value::as_array)
        .context("missing quota groups")?;
    ensure!(groups.len() <= MAX_GROUPS, "too many quota groups");
    let mut bucket_count = 0_usize;
    let mut projected = Vec::with_capacity(groups.len());
    for group in groups {
        let name =
            clean_quota_label(group.get("name"), MAX_LABEL)?.context("missing group name")?;
        let raw_buckets = match group.get("buckets") {
            None | Some(Value::Null) => &[][..],
            Some(Value::Array(items)) => items.as_slice(),
            Some(_) => bail!("invalid buckets"),
        };
        let mut buckets = Vec::with_capacity(raw_buckets.len());
        for bucket in raw_buckets {
            bucket_count += 1;
            ensure!(bucket_count <= MAX_BUCKETS, "too many quota buckets");
            let bucket_name =
                clean_quota_label(bucket.get("name"), MAX_LABEL)?.context("missing bucket name")?;
            let mut out = json!({"name": bucket_name});
            if let Some(window) = clean_quota_label(bucket.get("window"), 32)? {
                out["window"] = json!(window);
            }
            if let Some(disabled) = bucket.get("disabled") {
                out["disabled"] = json!(disabled.as_bool().context("invalid disabled flag")?);
            }
            if let Some(fraction) = bucket.get("remaining_fraction").filter(|v| !v.is_null()) {
                let value = fraction.as_f64().context("invalid remaining_fraction")?;
                ensure!(
                    value.is_finite() && (0.0..=1.0).contains(&value),
                    "remaining_fraction out of range"
                );
                out["remaining_fraction"] = json!(value);
            }
            if let Some(time) = bucket.get("reset_time").filter(|v| !v.is_null()) {
                let text = time.as_str().context("invalid reset_time")?;
                if !text.is_empty() {
                    let parsed = chrono::DateTime::parse_from_rfc3339(text)
                        .context("invalid RFC 3339 reset_time")?;
                    out["reset_at"] = json!(parsed.timestamp());
                }
            }
            buckets.push(out);
        }
        projected.push(json!({"name": name, "buckets": buckets}));
    }
    Ok(json!({"groups": projected}))
}

fn clean_quota_label(value: Option<&Value>, limit: usize) -> Result<Option<String>> {
    let text = match value {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(text)) => text.trim(),
        Some(_) => bail!("quota label is not a string"),
    };
    ensure!(
        !text
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')),
        "quota label has control characters"
    );
    Ok((!text.is_empty()).then(|| {
        text.chars()
            .take(limit)
            .collect::<String>()
            .trim_end()
            .to_owned()
    }))
}

/// The closed native usage code of a daemon failure.
fn sandbox_error_code(error: &anyhow::Error) -> &'static str {
    match daemon::DaemonError::code_of(error) {
        // Another unit holds the account (its runtime bound or a sign-in).
        Some("limit" | "fenced") => "native_usage_busy",
        // The daemon's credential is not this login's generation.
        Some("precondition") => "native_usage_profile_changed",
        // A daemon that predates `collect_usage`.
        Some("invalid_request") => "machine_usage_update_required",
        Some(_) => "native_usage_collector_failed",
        None => "native_usage_collector_unavailable",
    }
}

/// One card row: buckets of a group sharing the same quota.
struct Row<'a> {
    names: Vec<&'a str>,
    fraction: f64,
    reset_at: Option<i64>,
    window: Option<u32>,
}

/// The daemon's quota projection as `rateLimitsByLimitId`. Each vendor group
/// (a model family such as Gemini Pro or Flash) becomes one row when its
/// buckets share one quota, otherwise one row per distinct quota. Disabled
/// buckets and buckets without a remaining fraction are not shown. Keys
/// keep the vendor's order.
pub(super) fn rate_limits(quota: &Value) -> Value {
    let mut limits = serde_json::Map::new();
    let groups = quota["groups"].as_array().map_or(&[][..], Vec::as_slice);
    for (group_index, group) in groups.iter().enumerate() {
        let Some(group_name) = group["name"].as_str() else {
            continue;
        };
        let mut rows: Vec<Row<'_>> = Vec::new();
        for bucket in group["buckets"].as_array().map_or(&[][..], Vec::as_slice) {
            let (Some(name), Some(fraction)) = (
                bucket["name"].as_str(),
                bucket["remaining_fraction"].as_f64(),
            ) else {
                continue;
            };
            if bucket["disabled"] == true {
                continue;
            }
            let reset_at = bucket["reset_at"].as_i64();
            let window = bucket["window"].as_str().and_then(window_minutes);
            match rows.iter_mut().find(|row| {
                (row.fraction - fraction).abs() < 1e-9
                    && row.reset_at == reset_at
                    && row.window == window
            }) {
                Some(row) => row.names.push(name),
                None => rows.push(Row {
                    names: vec![name],
                    fraction,
                    reset_at,
                    window,
                }),
            }
        }
        // Rows told apart by their windows keep the family name; the window
        // label follows it on the card.
        let windows_distinct = rows.iter().all(|row| row.window.is_some())
            && rows
                .iter()
                .enumerate()
                .all(|(index, row)| rows[..index].iter().all(|other| other.window != row.window));
        let single = rows.len() == 1 || windows_distinct;
        for (row_index, row) in rows.iter().enumerate() {
            let label = if single {
                group_name.to_owned()
            } else if let [name] = row.names.as_slice() {
                (*name).to_owned()
            } else {
                format!("{} +{}", row.names[0], row.names.len() - 1)
            };
            let used = ((1.0 - row.fraction) * 1000.0).round() / 10.0;
            let mut primary = json!({"usedPercent": used.clamp(0.0, 100.0)});
            if let Some(reset_at) = row.reset_at {
                primary["resetsAt"] = json!(reset_at);
            }
            if let Some(window) = row.window {
                primary["windowDurationMins"] = json!(window);
            }
            limits.insert(
                format!("{group_index:02}{row_index:02}-{}", slug(&label)),
                json!({"limitName": label, "primary": primary}),
            );
        }
    }
    json!({"rateLimitsByLimitId": limits})
}

/// Minutes of a vendor window label such as `5h`, `5 hours`, `daily` or
/// `weekly`; `None` when it names no known period.
fn window_minutes(window: &str) -> Option<u32> {
    let text = window.trim().to_ascii_lowercase();
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let count = if digits.is_empty() {
        1
    } else {
        digits.parse::<u32>().ok().filter(|count| *count > 0)?
    };
    let unit = text[digits.len()..].trim_start_matches([' ', '-']);
    let minutes = match unit {
        "m" | "min" | "mins" | "minute" | "minutes" => 1,
        "h" | "hr" | "hrs" | "hour" | "hours" | "hourly" => 60,
        "d" | "day" | "days" | "daily" => 1440,
        "w" | "wk" | "week" | "weeks" | "weekly" => 10_080,
        "month" | "months" | "monthly" => 43_200,
        _ => return None,
    };
    count.checked_mul(minutes)
}

fn slug(label: &str) -> String {
    let mut slug = String::new();
    for character in label.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').chars().take(48).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_sharing_one_quota_become_one_row() {
        let quota = json!({"groups": [
            {"name": "Gemini Pro", "buckets": [
                {"name": "Gemini 3 Pro (High)", "remaining_fraction": 0.8, "reset_at": 1_795_179_600},
                {"name": "Gemini 3 Pro (Low)", "remaining_fraction": 0.8, "reset_at": 1_795_179_600},
            ]},
            {"name": "Gemini Flash", "buckets": [
                {"name": "Gemini 3 Flash", "remaining_fraction": 1.0},
                {"name": "Gemini 2.5 Flash Lite", "disabled": true, "remaining_fraction": 0.0},
                {"name": "Unknown", "window": "5h"},
            ]},
        ]});
        assert_eq!(
            rate_limits(&quota),
            json!({"rateLimitsByLimitId": {
                "0000-gemini-pro": {"limitName": "Gemini Pro",
                    "primary": {"usedPercent": 20.0, "resetsAt": 1_795_179_600}},
                "0100-gemini-flash": {"limitName": "Gemini Flash", "primary": {"usedPercent": 0.0}},
            }})
        );
    }

    #[test]
    fn distinct_quotas_of_a_family_keep_their_names_or_windows() {
        let quota = json!({"groups": [
            {"name": "Claude", "buckets": [
                {"name": "Claude Sonnet", "remaining_fraction": 0.5},
                {"name": "Claude Opus", "remaining_fraction": 0.0, "reset_at": 10},
                {"name": "Claude Opus Thinking", "remaining_fraction": 0.0, "reset_at": 10},
            ]},
            {"name": "Gemini Pro", "buckets": [
                {"name": "Pro 5h", "window": "5 hours", "remaining_fraction": 0.333_33},
                {"name": "Pro weekly", "window": "Weekly", "remaining_fraction": 0.9},
            ]},
        ]});
        let limits = rate_limits(&quota)["rateLimitsByLimitId"].clone();
        let names: Vec<(&str, &str)> = limits
            .as_object()
            .unwrap()
            .iter()
            .map(|(id, limit)| (id.as_str(), limit["limitName"].as_str().unwrap()))
            .collect();
        assert_eq!(
            names,
            [
                ("0000-claude-sonnet", "Claude Sonnet"),
                ("0001-claude-opus-1", "Claude Opus +1"),
                ("0100-gemini-pro", "Gemini Pro"),
                ("0101-gemini-pro", "Gemini Pro"),
            ]
        );
        assert_eq!(
            limits["0001-claude-opus-1"]["primary"]["usedPercent"],
            100.0
        );
        assert_eq!(limits["0100-gemini-pro"]["primary"]["usedPercent"], 66.7);
        assert_eq!(
            limits["0100-gemini-pro"]["primary"]["windowDurationMins"],
            300
        );
        assert_eq!(
            limits["0101-gemini-pro"]["primary"]["windowDurationMins"],
            10_080
        );
        assert_eq!(rate_limits(&json!({})), json!({"rateLimitsByLimitId": {}}));
    }

    #[test]
    fn windows_parse_only_known_periods() {
        for (text, minutes) in [
            ("5h", Some(300)),
            ("5 hours", Some(300)),
            ("5-hour", Some(300)),
            ("daily", Some(1440)),
            ("Weekly", Some(10_080)),
            ("30m", Some(30)),
            ("monthly", Some(43_200)),
            ("0h", None),
            ("rolling", None),
            ("", None),
        ] {
            assert_eq!(window_minutes(text), minutes, "{text}");
        }
    }

    #[test]
    fn daemon_refusals_map_to_closed_codes() {
        let refusal = |code: &str| {
            anyhow::Error::new(daemon::DaemonError {
                code: code.into(),
                message: "x".into(),
            })
        };
        assert_eq!(sandbox_error_code(&refusal("limit")), "native_usage_busy");
        assert_eq!(sandbox_error_code(&refusal("fenced")), "native_usage_busy");
        assert_eq!(
            sandbox_error_code(&refusal("precondition")),
            "native_usage_profile_changed"
        );
        assert_eq!(
            sandbox_error_code(&refusal("invalid_request")),
            "machine_usage_update_required"
        );
        assert_eq!(
            sandbox_error_code(&refusal("internal")),
            "native_usage_collector_failed"
        );
        assert_eq!(
            sandbox_error_code(&anyhow::anyhow!("unreachable")),
            "native_usage_collector_unavailable"
        );
        // A refusal under added context keeps its code.
        assert_eq!(
            sandbox_error_code(&refusal("limit").context("collecting")),
            "native_usage_busy"
        );
    }

    #[test]
    fn direct_antigravity_usage_interprets_quota_auth_and_location() {
        let ok = serde_json::json!({
            "status": "SUCCESS",
            "num_turns": 0,
            "command": {
                "name": "/usage",
                "data": {
                    "groups": [{
                        "name": "Gemini Pro",
                        "buckets": [{
                            "name": "Gemini 4 Neon",
                            "window": "5h",
                            "remaining_fraction": 0.75,
                            "reset_time": "2026-11-28T15:00:00Z",
                        }]
                    }]
                }
            }
        });
        let DirectUsageOutcome::Available(quota) =
            interpret_antigravity_usage(Some(0), format!("banner\n{ok}\n").as_bytes()).unwrap()
        else {
            panic!("expected available quota");
        };
        let limits = rate_limits(&quota);
        assert_eq!(
            limits["rateLimitsByLimitId"]["0000-gemini-pro"]["primary"]["usedPercent"],
            25.0
        );
        assert_eq!(
            limits["rateLimitsByLimitId"]["0000-gemini-pro"]["primary"]["windowDurationMins"],
            300
        );
        let auth = serde_json::json!({"status": "ERROR", "error": "Please log in again"});
        assert_eq!(
            interpret_antigravity_usage(Some(1), auth.to_string().as_bytes()).unwrap(),
            DirectUsageOutcome::AuthenticationRequired
        );
        let loc = serde_json::json!({"status": "ERROR", "error": "Unsupported location"});
        assert_eq!(
            interpret_antigravity_usage(Some(1), loc.to_string().as_bytes()).unwrap(),
            DirectUsageOutcome::LocationUnsupported
        );
        assert!(interpret_antigravity_usage(Some(0), b"{}").is_err());
    }
}
