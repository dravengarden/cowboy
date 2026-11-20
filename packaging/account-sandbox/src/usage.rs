//! `collect_usage`: the account's quota report. A `usage` unit runs the
//! pinned `agy` in print mode with the fixed `/usage` command
//! ([`crate::units::COMMANDS`]) as the account's own identity over its own
//! home, exactly like a runtime unit of the current signed-in generation (it
//! counts toward a configured runtime bound and is excluded by login and
//! logout). The CLI
//! answers a leading supported slash command locally from Cloud Code's
//! quota summary, without a conversation or a model turn. The daemon bounds
//! the run and its output and answers only the validated projection below:
//! group and bucket names, an optional window label, the remaining fraction
//! and the reset time. Vendor text never leaves the daemon otherwise.

use std::io::Read as _;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::daemon::{Conn, Daemon, Reply};
use crate::protocol::{Code, Mode, Outcome, decode_base64, fail};
use crate::start::StartRequest;

/// Bound of one report's run; the Machine's own admission is 30 s.
#[cfg(not(test))]
const USAGE_TIMEOUT: Duration = Duration::from_secs(25);
#[cfg(test)]
const USAGE_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest stdout accepted from a report.
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_GROUPS: usize = 32;
/// Most buckets over all groups.
const MAX_BUCKETS: usize = 128;
const MAX_NAME_CHARS: usize = 80;
const MAX_WINDOW_CHARS: usize = 32;
const MAX_ID_BYTES: usize = 128;

impl Daemon {
    pub(crate) fn collect_usage(
        self: &Arc<Self>,
        conn: &Conn,
        key: &str,
        credential_generation: u64,
        runtime_sha256: &str,
    ) -> Outcome<Reply> {
        let started = self.start(
            conn,
            &StartRequest {
                key,
                mode: Mode::Usage,
                credential_generation,
                runtime_sha256,
                session_id: None,
                descriptor_b64: None,
            },
        )?;
        let unit = started.value["unit"]
            .as_str()
            .ok_or_else(|| fail(Code::Internal, "usage unit has no name"))?
            .to_owned();
        let mut fds = started.fds.into_iter();
        // Closing stdin gives print mode EOF; stderr is drained and dropped.
        drop(fds.next());
        let stdout = fds
            .next()
            .ok_or_else(|| fail(Code::Internal, "usage unit has no stdout"))?;
        let (sender, output) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(read_bounded(std::fs::File::from(stdout)));
        });
        if let Some(stderr) = fds.next() {
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut std::fs::File::from(stderr), &mut std::io::sink());
            });
        }
        let exit = self.wait_exit(&unit, USAGE_TIMEOUT);
        // Stop (on a timeout) and confirm the unit gone before answering.
        self.stop_unit(&unit)?;
        let output = output
            .recv_timeout(Duration::from_secs(2))
            .ok()
            .flatten()
            .ok_or_else(|| {
                fail(
                    Code::Internal,
                    "the usage report's output did not end within its bound",
                )
            })?;
        let mut report = interpret(exit, &output)?;
        report["unit"] = json!(unit);
        Ok(Reply::value(report))
    }
}

/// Everything the pipe yields, or `None` beyond [`MAX_OUTPUT_BYTES`] (the
/// rest is still drained so the unit never blocks on a full pipe).
fn read_bounded(mut reader: impl std::io::Read) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    let read = (&mut reader)
        .take(MAX_OUTPUT_BYTES as u64 + 1)
        .read_to_end(&mut output)
        .ok()?;
    if read > MAX_OUTPUT_BYTES {
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
        return None;
    }
    Some(output)
}

/// The reply of one finished report: `{"outcome":"available","quota":{..}}`
/// or `{"outcome":"authentication_required"}`; anything else is a fixed
/// `internal` failure without vendor text.
pub fn interpret(exit: Option<i32>, output: &[u8]) -> Outcome<Value> {
    let text = String::from_utf8_lossy(output);
    let envelope = text
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| line.starts_with('{'))
        .find_map(|line| serde_json::from_str::<Map<String, Value>>(line).ok());
    let Some(envelope) = envelope else {
        // Without a usable token print mode starts its interactive sign-in.
        if text.contains("Authentication required") || text.contains("not logged in") {
            return Ok(json!({"outcome": "authentication_required"}));
        }
        return Err(fail(
            Code::Internal,
            format!("the usage report exited {exit:?} without a JSON result"),
        ));
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
            return Ok(json!({"outcome": "authentication_required"}));
        }
        return Err(fail(
            Code::Internal,
            format!("the usage report failed (exit {exit:?})"),
        ));
    }
    if envelope
        .get("num_turns")
        .is_some_and(|turns| turns.as_u64() != Some(0))
    {
        return Err(fail(Code::Internal, "the usage report ran a model turn"));
    }
    let command = envelope
        .get("command")
        .and_then(Value::as_object)
        .ok_or_else(|| fail(Code::Internal, "the usage report has no command result"))?;
    if command
        .get("name")
        .and_then(Value::as_str)
        .map(|name| name.trim_start_matches('/'))
        != Some("usage")
    {
        return Err(fail(
            Code::Internal,
            "the usage report answered another command",
        ));
    }
    let document = command_data(command.get("data"))
        .ok_or_else(|| fail(Code::Internal, "the usage report has no quota document"))?;
    let quota = project(&document).map_err(|error| fail(Code::Internal, format!("{error:#}")))?;
    Ok(json!({"outcome": "available", "quota": quota}))
}

/// The command's payload: a JSON object, or its JSON text or base64 bytes.
fn command_data(data: Option<&Value>) -> Option<Value> {
    match data? {
        Value::Object(_) => data.cloned(),
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .or_else(|| {
                decode_base64(text)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            })
            .filter(Value::is_object),
        _ => None,
    }
}

/// `{"groups":[{"name","buckets":[{"name","id"?,"window"?,"disabled"?,
/// "remaining_fraction"?,"reset_at"?}]}]}`, `reset_at` in Unix seconds.
/// Unknown vendor fields are dropped; a malformed known field refuses the
/// whole report.
pub fn project(document: &Value) -> anyhow::Result<Value> {
    let groups = document
        .get("groups")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("the quota document has no groups"))?;
    anyhow::ensure!(groups.len() <= MAX_GROUPS, "too many quota groups");
    let mut total = 0;
    let mut projected = Vec::with_capacity(groups.len());
    for group in groups {
        let name = label(group.get("name"), MAX_NAME_CHARS)?
            .ok_or_else(|| anyhow::anyhow!("a quota group has no name"))?;
        let buckets = match group.get("buckets") {
            None | Some(Value::Null) => &Vec::new(),
            Some(Value::Array(buckets)) => buckets,
            Some(_) => anyhow::bail!("quota group buckets are not a list"),
        };
        total += buckets.len();
        anyhow::ensure!(total <= MAX_BUCKETS, "too many quota buckets");
        let buckets = buckets
            .iter()
            .map(bucket)
            .collect::<anyhow::Result<Vec<_>>>()?;
        projected.push(json!({"name": name, "buckets": buckets}));
    }
    Ok(json!({"groups": projected}))
}

fn bucket(value: &Value) -> anyhow::Result<Value> {
    let name = label(value.get("name"), MAX_NAME_CHARS)?
        .ok_or_else(|| anyhow::anyhow!("a quota bucket has no name"))?;
    let mut out = json!({"name": name});
    if let Some(id) = value.get("id").and_then(Value::as_str).filter(|id| {
        !id.is_empty() && id.len() <= MAX_ID_BYTES && id.bytes().all(|byte| byte.is_ascii_graphic())
    }) {
        out["id"] = json!(id);
    }
    if let Some(window) = label(value.get("window"), MAX_WINDOW_CHARS)? {
        out["window"] = json!(window);
    }
    match value.get("disabled") {
        None | Some(Value::Null | Value::Bool(false)) => {}
        Some(Value::Bool(true)) => out["disabled"] = json!(true),
        Some(_) => anyhow::bail!("a quota bucket's disabled flag is not a boolean"),
    }
    match value.get("remaining_fraction") {
        None | Some(Value::Null) => {}
        Some(fraction) => {
            let fraction = fraction
                .as_f64()
                .filter(|fraction| (-1e-9..=1.0 + 1e-9).contains(fraction))
                .ok_or_else(|| anyhow::anyhow!("a remaining fraction is out of range"))?;
            out["remaining_fraction"] = json!(fraction.clamp(0.0, 1.0));
        }
    }
    match value.get("reset_time") {
        None | Some(Value::Null) => {}
        Some(Value::String(time)) if time.is_empty() => {}
        Some(Value::String(time)) => {
            out["reset_at"] = json!(
                rfc3339_seconds(time)
                    .ok_or_else(|| anyhow::anyhow!("a reset time is not RFC 3339"))?
            );
        }
        Some(_) => anyhow::bail!("a reset time is not a string"),
    }
    Ok(out)
}

/// Printable text, trimmed, at most `limit` characters (longer is cut);
/// `None` when absent or empty.
fn label(value: Option<&Value>, limit: usize) -> anyhow::Result<Option<String>> {
    let text = match value {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(text)) => text.trim(),
        Some(_) => anyhow::bail!("a quota label is not a string"),
    };
    anyhow::ensure!(
        !text
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')),
        "a quota label has control characters"
    );
    Ok((!text.is_empty()).then(|| {
        text.chars()
            .take(limit)
            .collect::<String>()
            .trim_end()
            .to_owned()
    }))
}

/// Unix seconds of `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)`.
pub fn rfc3339_seconds(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = bytes.get(range)?;
        part.iter().all(u8::is_ascii_digit).then(|| {
            part.iter()
                .fold(0_i64, |value, digit| value * 10 + i64::from(digit - b'0'))
        })
    };
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    let (hour, minute, second) = (digits(11..13)?, digits(14..16)?, digits(17..19)?);
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let end = fraction
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(fraction.len());
        if end == 0 {
            return None;
        }
        rest = &fraction[end..];
    }
    let offset = match rest.as_bytes() {
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let offset = rest.get(1..3)?.parse::<i64>().ok()? * 3600
                + rest.get(4..6)?.parse::<i64>().ok()? * 60;
            if *sign == b'+' { offset } else { -offset }
        }
        _ => return None,
    };
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(data: &Value) -> Vec<u8> {
        let envelope = json!({
            "conversation_id": "", "status": "SUCCESS", "response": "",
            "duration_seconds": 0.4, "num_turns": 0,
            "usage": {"input_tokens": 0, "output_tokens": 0, "thinking_tokens": 0,
                "cache_read_tokens": 0, "total_tokens": 0},
            "command": {"name": "usage", "data": data},
        });
        format!("some banner\n{envelope}\n").into_bytes()
    }

    fn quota() -> Value {
        json!({"description": "Quota for your plan", "groups": [
            {"name": "Gemini Pro", "description": "Pro models", "buckets": [
                {"id": "gemini-3-pro", "name": "Gemini 3 Pro", "window": "5h",
                 "remaining_fraction": 0.8, "reset_time": "2026-11-20T13:00:00Z",
                 "remaining_amount": 12},
            ]},
            {"name": "Gemini Flash", "buckets": [
                {"name": "Gemini 3 Flash", "remaining_fraction": 1, "disabled": false},
                {"name": "Gemini 2.5 Flash Lite", "disabled": true},
            ]},
        ]})
    }

    #[test]
    fn a_successful_report_is_projected() {
        let value = interpret(Some(0), &report(&quota())).unwrap();
        assert_eq!(
            value,
            json!({"outcome": "available", "quota": {"groups": [
                {"name": "Gemini Pro", "buckets": [{"name": "Gemini 3 Pro", "id": "gemini-3-pro",
                    "window": "5h", "remaining_fraction": 0.8, "reset_at": 1_795_179_600}]},
                {"name": "Gemini Flash", "buckets": [
                    {"name": "Gemini 3 Flash", "remaining_fraction": 1.0},
                    {"name": "Gemini 2.5 Flash Lite", "disabled": true},
                ]},
            ]}})
        );
        // The payload may also arrive as JSON text or base64 bytes.
        let text = Value::String(quota().to_string());
        assert_eq!(interpret(Some(0), &report(&text)).unwrap(), value);
        let encoded = Value::String(base64(quota().to_string().as_bytes()));
        assert_eq!(interpret(Some(0), &report(&encoded)).unwrap(), value);
    }

    #[test]
    fn failures_carry_no_vendor_text_and_auth_is_recognised() {
        let auth = b"Authentication required. Please visit the URL to log in:\n  https://accounts.google.com/o/oauth2/auth?x\n";
        assert_eq!(
            interpret(None, auth).unwrap(),
            json!({"outcome": "authentication_required"})
        );
        let failed = json!({"status": "ERROR", "error": "You are not logged into Antigravity.", "num_turns": 0});
        assert_eq!(
            interpret(Some(1), failed.to_string().as_bytes()).unwrap(),
            json!({"outcome": "authentication_required"})
        );
        let other =
            json!({"status": "ERROR", "error": "secret vendor detail /home/x", "num_turns": 0});
        let error = interpret(Some(1), other.to_string().as_bytes()).unwrap_err();
        assert_eq!(error.code, Code::Internal);
        assert!(!error.message.contains("secret"), "{}", error.message);
        assert_eq!(
            interpret(Some(0), b"plain text only").unwrap_err().code,
            Code::Internal
        );
    }

    #[test]
    fn a_model_turn_or_another_command_is_refused() {
        let mut turn: Value =
            serde_json::from_slice(report(&quota()).split(|b| *b == b'\n').nth(1).unwrap())
                .unwrap();
        turn["num_turns"] = json!(1);
        let error = interpret(Some(0), turn.to_string().as_bytes()).unwrap_err();
        assert!(error.message.contains("model turn"));
        turn["num_turns"] = json!(0);
        turn["command"]["name"] = json!("credits");
        assert!(interpret(Some(0), turn.to_string().as_bytes()).is_err());
        turn["command"]["name"] = json!("/usage");
        assert!(interpret(Some(0), turn.to_string().as_bytes()).is_ok());
    }

    #[test]
    fn malformed_documents_are_refused() {
        for bad in [
            json!({}),
            json!({"groups": {}}),
            json!({"groups": [{"buckets": []}]}),
            json!({"groups": [{"name": "G", "buckets": [{"name": "B", "remaining_fraction": 1.5}]}]}),
            json!({"groups": [{"name": "G", "buckets": [{"name": "B", "remaining_fraction": "0.5"}]}]}),
            json!({"groups": [{"name": "G", "buckets": [{"name": "B", "reset_time": "tomorrow"}]}]}),
            json!({"groups": [{"name": "G\u{7}", "buckets": []}]}),
            json!({"groups": [{"name": "G", "buckets": [{"remaining_fraction": 0.5}]}]}),
        ] {
            assert!(project(&bad).is_err(), "{bad}");
        }
        let many = json!({"groups": (0..33).map(|i| json!({"name": format!("G{i}")})).collect::<Vec<_>>()});
        assert!(project(&many).is_err());
        let long = json!({"groups": [{"name": "x".repeat(200), "buckets": [{"name": "B", "id": "bad id"}]}]});
        let projected = project(&long).unwrap();
        assert_eq!(projected["groups"][0]["name"].as_str().unwrap().len(), 80);
        assert!(projected["groups"][0]["buckets"][0].get("id").is_none());
    }

    #[test]
    fn rfc3339_times_parse() {
        assert_eq!(rfc3339_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339_seconds("2026-11-20T13:00:00Z"), Some(1_795_179_600));
        assert_eq!(
            rfc3339_seconds("2026-11-20T13:00:00.123456Z"),
            Some(1_795_179_600)
        );
        assert_eq!(
            rfc3339_seconds("2026-11-20T14:00:00+01:00"),
            Some(1_795_179_600)
        );
        assert_eq!(
            rfc3339_seconds("2024-02-29T00:00:00-00:30"),
            Some(1_709_166_600)
        );
        for bad in [
            "2026-11-20",
            "2026-11-20 13:00:00Z",
            "2026-13-01T00:00:00Z",
            "2025-02-29T00:00:00Z",
            "2026-11-20T13:00:00",
            "2026-11-20T13:00:00.Z",
            "2026-11-20T13:00:00+0100",
        ] {
            assert_eq!(rfc3339_seconds(bad), None, "{bad}");
        }
    }

    #[test]
    fn output_beyond_the_bound_is_refused() {
        assert_eq!(read_bounded(&b"abc"[..]), Some(b"abc".to_vec()));
        let big = vec![b'x'; MAX_OUTPUT_BYTES + 1];
        assert_eq!(read_bounded(&big[..]), None);
    }

    fn base64(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let word = chunk
                .iter()
                .enumerate()
                .fold(0_u32, |word, (i, &b)| word | u32::from(b) << (16 - 8 * i));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(TABLE[((word >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }
}
