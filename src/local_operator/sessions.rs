//! Session listing and Provider updates through the host endpoint, so the
//! Controller host needs no device login to keep sessions current.

use anyhow::{Result, ensure};
use serde_json::{Value, json};

use super::converge::read_json;

fn sessions_of(data: &Value) -> Vec<Value> {
    data.get("sessions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn text<'a>(session: &'a Value, key: &str) -> &'a str {
    session.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn waiting_version(session: &Value) -> Option<&str> {
    session
        .pointer("/provider_update_available/version")
        .and_then(Value::as_str)
}

async fn list(client: &reqwest::Client) -> Result<Vec<Value>> {
    let (status, data) = read_json(client, reqwest::Method::GET, &["sessions"], None).await?;
    ensure!(
        status == 200,
        "listing sessions returned HTTP {status}: {data}"
    );
    Ok(sessions_of(&data))
}

pub(super) async fn run_list(client: &reqwest::Client, outdated: bool, json: bool) -> Result<()> {
    let mut sessions = list(client).await?;
    if outdated {
        sessions.retain(|session| waiting_version(session).is_some());
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    for session in &sessions {
        println!(
            "{}\t{}\t{}\t{} {}{}\t{}",
            text(session, "id"),
            text(session, "status"),
            text(session, "machine_id"),
            text(session, "provider"),
            text(session, "provider_version"),
            waiting_version(session)
                .map(|version| format!(" -> {version}"))
                .unwrap_or_default(),
            text(session, "title"),
        );
    }
    Ok(())
}

pub(super) async fn run_update(
    client: &reqwest::Client,
    named: Vec<String>,
    all: bool,
    except: &[String],
    force: bool,
    when_idle: bool,
) -> Result<()> {
    let ids: Vec<String> = if all {
        list(client)
            .await?
            .iter()
            .filter(|session| waiting_version(session).is_some())
            .map(|session| text(session, "id").to_owned())
            .filter(|id| !except.contains(id))
            .collect()
    } else {
        ensure!(!named.is_empty(), "name sessions or pass --all");
        named
    };
    if ids.is_empty() {
        println!("no session has an update waiting");
        return Ok(());
    }
    let mut refused = 0_usize;
    for id in &ids {
        let (status, data) = read_json(
            client,
            reqwest::Method::POST,
            &["sessions", id, "provider-update"],
            Some(json!({ "force": force, "when_idle": when_idle })),
        )
        .await?;
        if (200..300).contains(&status) {
            println!(
                "{id}\t{}\t{} -> {}",
                text(&data, "state"),
                text(&data, "from"),
                text(&data, "to")
            );
        } else {
            refused += 1;
            let reason = data
                .as_str()
                .map_or_else(|| data.to_string(), ToOwned::to_owned);
            eprintln!("{id}\trefused (HTTP {status}): {reason}");
        }
    }
    ensure!(
        refused == 0,
        "{refused} of {} sessions were not updated",
        ids.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_updates_come_from_the_published_offer() {
        let data = json!({"sessions":[
            {"id":"a","provider_update_available":{"version":"2.0.0"}},
            {"id":"b"},
        ]});
        let sessions = sessions_of(&data);
        assert_eq!(waiting_version(&sessions[0]), Some("2.0.0"));
        assert_eq!(waiting_version(&sessions[1]), None);
        assert!(sessions_of(&json!({})).is_empty());
    }
}
