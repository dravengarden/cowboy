//! `cowboy session prompt` (including `--notify`) and `cowboy session wait`:
//! hand work to another session, then either end the turn and receive a
//! Cowboy notice or stay in the turn until that prompt's turn ends.

use anyhow::{Result, bail, ensure};
use clap::Args;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

use super::Client;
use crate::managed_calls::cli::{exchange, wiring};
use crate::managed_calls::protocol::Action;
use crate::send_delivery::SendDelivery;

/// One progress request waits at most this long on the Controller, below
/// the client's 60 s HTTP timeout.
const LONG_POLL: Duration = Duration::from_secs(50);

#[derive(Args)]
pub(super) struct PromptArgs {
    session: String,
    text: String,
    /// Delivery intent; current_turn requires native support on the running worker.
    #[arg(long, value_enum, default_value = "next_turn")]
    delivery: SendDelivery,
    /// Prioritize the queued message without interrupting.
    #[arg(long)]
    front: bool,
    /// Stable identity for retrying the same submission after a lost response.
    #[arg(long)]
    operation: Option<String>,
    /// From inside a Cowboy session: when this prompt's turn ends, Cowboy
    /// queues one notice into the calling session. End your turn instead
    /// of polling. Next-turn delivery only; needs no product login.
    #[arg(long)]
    pub(super) notify: bool,
}

#[derive(Args)]
pub(super) struct WaitArgs {
    session: String,
    prompt: String,
    #[arg(long, default_value_t = 540, value_parser = clap::value_parser!(u64).range(1..=3600))]
    timeout_seconds: u64,
}

/// A prompt id usable in a URL path and a printed shell command unchanged.
fn plain_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}

/// `cowboy session prompt` through this computer's login.
pub(super) async fn prompt(client: &Client, args: PromptArgs) -> Result<()> {
    let PromptArgs {
        session,
        text,
        delivery,
        front,
        operation,
        notify: _,
    } = args;
    SendDelivery::resolve(Some(delivery), false, front).map_err(anyhow::Error::msg)?;
    let operation = operation.unwrap_or_else(|| format!("cli:{}", uuid::Uuid::new_v4()));
    eprintln!("operation: {operation}");
    let target = if delivery == SendDelivery::NextTurn {
        Value::Null
    } else {
        let options = client
            .expect(
                Method::GET,
                &format!("/api/sessions/{session}/delivery-options"),
                None,
            )
            .await?;
        let options: Value = serde_json::from_str(&options)?;
        ensure!(
            !options["target"].is_null(),
            "No active delivery target; choose next_turn"
        );
        options["target"].clone()
    };
    let body = json!({ "text": text, "delivery": delivery, "front": front, "operation": operation, "target": target });
    let receipt = client
        .expect(
            Method::POST,
            &format!("/api/sessions/{session}/prompt"),
            Some(&body),
        )
        .await?;
    println!("{session}\t{}", receipt.trim());
    if delivery == SendDelivery::NextTurn && plain_id(&operation) {
        eprintln!("wait: cowboy session wait {session} {operation}");
    }
    Ok(())
}

/// `--notify`: queue through this session's Agent API, which knows the
/// sender, so the Controller can notify it when the turn ends. Prints the
/// JSON reply; a refusal or an uncertain admission exits non-zero.
pub(super) async fn prompt_notified(args: PromptArgs) -> Result<()> {
    if args.delivery != SendDelivery::NextTurn {
        bail!("--notify needs next_turn delivery");
    }
    let context = match wiring() {
        Ok(context) => context,
        Err(code) => bail!(
            "--notify works only inside a Cowboy session ({code}); \
             use cowboy_execute control.apply queue.submit with notify:true"
        ),
    };
    let operation = args
        .operation
        .unwrap_or_else(|| format!("cli-{}", uuid::Uuid::new_v4()));
    if !crate::managed_calls::valid_id(&operation) {
        bail!("--operation with --notify may use only letters, digits, '-', '_' and '.'");
    }
    eprintln!("operation: {operation}");
    let mut input = json!({"session_id": args.session, "text": args.text, "notify": true});
    if args.front {
        input["front"] = json!(true);
    }
    let action = Action::AgentInvoke {
        action: "control.apply".into(),
        arguments: json!({"name": "queue.submit", "input": input}),
        operation: Some(operation),
    };
    let reply = exchange(context, action, 0).await;
    println!("{}", serde_json::to_string(&reply)?);
    if !notify_accepted(&reply) {
        bail!("not accepted: see the printed reply; observe a pending one with its operation");
    }
    Ok(())
}

/// Whether the Agent API accepted a `queue.submit` with `--notify`.
fn notify_accepted(reply: &Value) -> bool {
    reply["state"] == "ok"
        && reply.get("error").is_none_or(Value::is_null)
        && reply["value"]["status"] == "accepted"
}

/// Wait until the turn of `prompt` (the id printed by `session prompt`)
/// ends in `session`, or the timeout passes. Prints one JSON line; a timeout
/// is not an error and its `next` continues the wait.
pub(super) async fn wait(client: &Client, args: WaitArgs) -> Result<()> {
    let WaitArgs {
        session,
        prompt,
        timeout_seconds,
    } = args;
    if !plain_id(&session) || !plain_id(&prompt) {
        bail!("session and prompt ids may use only letters, digits, '-', '_', '.' and ':'");
    }
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
    let mut ran = false;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait_ms = remaining.min(LONG_POLL).as_millis();
        let path = format!("/api/sessions/{session}/prompts/{prompt}?wait_ms={wait_ms}");
        let (status, text) = client.request(Method::GET, &path, None).await?;
        let mut progress: Value = if status == StatusCode::NOT_FOUND {
            json!({"session_id": session, "prompt": prompt, "state": "gone"})
        } else {
            ensure!(status.is_success(), "GET {path}: {status}: {}", text.trim());
            serde_json::from_str(&text)?
        };
        let done = finished(&progress, &mut ran);
        if done.is_none() && remaining > LONG_POLL {
            continue;
        }
        match done {
            Some(state) => progress["state"] = json!(state),
            None => progress["next"] = json!(format!("cowboy session wait {session} {prompt}")),
        }
        println!("{}", serde_json::to_string(&progress)?);
        return Ok(());
    }
}

/// The final state for one observation, or `None` to keep waiting. A requeue
/// starts a new dispatch attempt. A prompt that ran and whose evidence left
/// the hot window has ended once the session is no longer busy; one never
/// seen running is `unknown` (removed, moved to drafts, or evicted).
fn finished(progress: &Value, ran: &mut bool) -> Option<&'static str> {
    match progress["state"].as_str() {
        Some("queued") => {
            *ran = false;
            None
        }
        Some("running") => {
            *ran = true;
            None
        }
        Some("finished") => Some("finished"),
        Some("stopped") => Some("stopped"),
        Some("unknown") if *ran => {
            (!matches!(progress["status"].as_str(), Some("busy" | "starting"))).then_some("ended")
        }
        Some("unknown") => Some("unknown"),
        _ => Some("gone"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_follows_the_prompts_own_turn() {
        let mut ran = false;
        let state = |state: &str, status: &str| json!({"state": state, "status": status});
        assert_eq!(finished(&state("running", "busy"), &mut ran), None);
        assert_eq!(finished(&state("queued", "busy"), &mut ran), None);
        assert!(!ran, "a requeue starts a new attempt");
        assert_eq!(finished(&state("running", "busy"), &mut ran), None);
        assert_eq!(finished(&state("unknown", "busy"), &mut ran), None);
        assert_eq!(finished(&state("unknown", "idle"), &mut ran), Some("ended"));
        assert_eq!(
            finished(&state("finished", "crashed"), &mut ran),
            Some("finished")
        );
        assert_eq!(
            finished(&state("stopped", "crashed"), &mut ran),
            Some("stopped")
        );
        let mut fresh = false;
        assert_eq!(
            finished(&state("unknown", "idle"), &mut fresh),
            Some("unknown")
        );
        assert_eq!(finished(&json!({"state":"gone"}), &mut fresh), Some("gone"));
    }

    #[test]
    fn only_plain_ids_reach_a_path_or_a_printed_command() {
        assert!(plain_id("cli:1f0e-2a"));
        assert!(plain_id("agent-abc.1_2"));
        for bad in ["", "batch#2", "a/b", "a b", "a?b", "a'b"] {
            assert!(!plain_id(bad), "{bad}");
        }
    }

    #[test]
    fn notify_acceptance_matches_the_agent_api_envelope() {
        assert!(notify_accepted(&json!({
            "schema": 1,
            "state": "ok",
            "value": {"status": "accepted"}
        })));
        for rejected in [
            json!({"schema": 1, "state": "ok", "value": {"status": "pending"}}),
            json!({"schema": 1, "state": "error", "error": {"code": "not_found"}}),
            json!({"error": "gateway_unavailable", "admission": "not_submitted"}),
            json!({"result": {"status": "accepted"}}),
            json!({"state": "ok", "error": "unexpected", "value": {"status": "accepted"}}),
        ] {
            assert!(!notify_accepted(&rejected), "{rejected}");
        }
    }
}
