//! Finite vocabulary for schema-one client metrics sent as Prometheus text.

#[cfg(feature = "machine-host")]
fn known_instrument(name: &str) -> bool {
    matches!(
        name,
        "websocket_probe_duration_ms"
            | "websocket_connect_duration_ms"
            | "websocket_reconnect_duration_ms"
            | "websocket_reconnect_success"
            | "long_task_count"
            | "long_task_duration_ms"
            | "navigation_duration_ms"
            | "command_duration_ms"
            | "first_output_duration_ms"
    )
}

fn values(key: &str) -> Option<&'static [&'static str]> {
    Some(match key {
        "platform" => &["ios", "macos", "web", "other"],
        "surface" => &["mobile", "desktop", "other"],
        "connection" => &["initial", "reconnect", "other"],
        "transport" => &["websocket", "http", "other"],
        "reason" => &[
            "initial",
            "online",
            "visibility",
            "retry",
            "heartbeat_timeout",
            "transport_error",
            "close",
            "manual",
            "other",
        ],
        _ => return None,
    })
}

#[cfg(feature = "full")]
pub(crate) fn dimension<'a>(key: &'a str, value: &'a str) -> Option<(&'a str, &'a str)> {
    if matches!(key, "platform" | "surface") {
        return None; // set by the client envelope
    }
    let allowed = values(key)?;
    Some((
        key,
        if allowed.contains(&value) {
            value
        } else {
            "other"
        },
    ))
}

/// Admit only the sample grammar emitted by Cowboy. This also excludes
/// directives, escaped values and OpenMetrics exemplars in the original body.
#[cfg(feature = "machine-host")]
pub(crate) fn admitted(body: &str) -> bool {
    body.lines().all(sample)
}

#[cfg(feature = "machine-host")]
fn sample(line: &str) -> bool {
    let Some((instrument, remainder)) = line.split_once('{') else {
        return false;
    };
    let Some((labels, numbers)) = remainder.split_once("} ") else {
        return false;
    };
    if !instrument
        .strip_prefix("cowboy_client_")
        .is_some_and(known_instrument)
    {
        return false;
    }
    let mut keys = std::collections::BTreeSet::new();
    if !labels.split(',').all(|label| {
        let Some((key, value)) = label.split_once('=') else {
            return false;
        };
        let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"'));
        value.is_some_and(|v| values(key).is_some_and(|allowed| allowed.contains(&v)))
            && keys.insert(key)
    }) {
        return false;
    }
    let mut numbers = numbers.split_ascii_whitespace();
    numbers
        .next()
        .is_some_and(|n| n.parse::<f64>().is_ok_and(f64::is_finite))
        && numbers.next().is_some_and(|n| n.parse::<i64>().is_ok())
        && numbers.next().is_none()
}

#[cfg(all(test, feature = "machine-host"))]
mod tests {
    use super::*;

    #[test]
    fn requires_finite_names_and_labels_in_original_sample_grammar() {
        let valid =
            "cowboy_client_long_task_count{platform=\"web\",reason=\"other\"} 3 1788825600000\n";
        assert!(admitted(valid));
        for bad in [
            valid.replace("other", "private-path"),
            valid.replace("reason", "session_id"),
            valid.replace("reason=\"other\"", "platform=\"ios\""),
            valid.replace("long_task_count", "private_session_id"),
            valid.replace(" 3 ", " NaN "),
            valid.replace('\n', " # {call_id=\"private\"} 3\n"),
            format!("# HELP cowboy_client_long_task_count private description\n{valid}"),
        ] {
            assert!(!admitted(&bad), "{bad}");
        }
    }
}
