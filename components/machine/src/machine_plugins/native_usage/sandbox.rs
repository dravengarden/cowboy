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
}
