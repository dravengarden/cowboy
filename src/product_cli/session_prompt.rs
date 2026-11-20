//! `cowboy session prompt --notify` and `cowboy session wait`: hand work to
//! another session, then either end the turn and receive a Cowboy notice or
//! stay in the turn until that prompt's turn ends.

use anyhow::{Result, bail, ensure};
use clap::Args;
use reqwest::Method;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

use super::Client;
use crate::managed_calls::cli::{exchange, wiring};
use crate::managed_calls::protocol::Action;

const POLL: Duration = Duration::from_secs(3);

#[derive(Args)]
pub(super) struct PromptArgs {
    session: String,
    text: String,
    /// Delivery intent; current_turn requires native support on the running worker.
    #[arg(long, value_enum, default_value = "next_turn")]
    delivery: crate::send_delivery::SendDelivery,
    /// Prioritize the queued message without interrupting.
    #[arg(long)]
    front: bool,
    /// Stable identity for retrying the same submission after a lost response.
    #[arg(long)]
    operation: Option<String>,
    /// From inside a Cowboy session: when this prompt's turn ends, Cowboy
    /// queues one notice into the calling session. End your turn instead
    /// of polling. Next-turn delivery only.
    #[arg(long, conflicts_with = "delivery")]
    notify: bool,
}

#[derive(Args)]
pub(super) struct WaitArgs {
    session: String,
    prompt: String,
    #[arg(long, default_value_t = 540, value_parser = clap::value_parser!(u64).range(1..=3600))]
    timeout_seconds: u64,
}

/// `cowboy session prompt`: queue a prompt through this computer's login, or
/// with `--notify` through the calling session's Agent API.
pub(super) async fn prompt(client: &Client, args: PromptArgs) -> Result<()> {
    let PromptArgs {
        session,
        text,
        delivery,
        front,
        operation,
        notify,
    } = args;
    if notify {
        return prompt_notified(session, text, front, operation).await;
    }
    crate::send_delivery::SendDelivery::resolve(Some(delivery), false, front)
        .map_err(anyhow::Error::msg)?;
    let operation = operation.unwrap_or_else(|| format!("cli:{}", uuid::Uuid::new_v4()));
    eprintln!("operation: {operation}");
    let target = if delivery == crate::send_delivery::SendDelivery::NextTurn {
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
    eprintln!("wait: cowboy session wait {session} {operation}");
    Ok(())
}

/// Queue `text` into `session` through this session's Agent API, which
/// knows the sender, so the Controller can notify it when the turn ends.
async fn prompt_notified(
    session: String,
    text: String,
    front: bool,
    operation: Option<String>,
) -> Result<()> {
    let context = match wiring() {
        Ok(context) => context,
        Err(code) => bail!(
            "--notify works only inside a Cowboy session ({code}); \
             use cowboy_execute control.apply queue.submit with notify:true"
        ),
    };
    let operation = operation.unwrap_or_else(|| format!("cli-{}", uuid::Uuid::new_v4()));
    if !crate::managed_calls::valid_id(&operation) {
        bail!("--operation with --notify may use only letters, digits, '-', '_' and '.'");
    }
    eprintln!("operation: {operation}");
    let mut input = json!({"session_id": session, "text": text, "notify": true});
    if front {
        input["front"] = json!(true);
    }
    let action = Action::AgentInvoke {
        action: "control.apply".into(),
        arguments: json!({"name": "queue.submit", "input": input}),
        operation: Some(operation),
    };
    let reply = exchange(context, action, 0).await;
    println!("{}", serde_json::to_string(&reply)?);
    Ok(())
}

/// Wait until the turn of `prompt` (the id printed by `session prompt`)
/// ends in `session`, or `timeout` passes. Prints one JSON line; a timeout
/// is not an error and the same command continues the wait.
pub(super) async fn wait(client: &Client, args: WaitArgs) -> Result<()> {
    let WaitArgs {
        session,
        prompt,
        timeout_seconds,
    } = args;
    let timeout = Duration::from_secs(timeout_seconds);
    let started = Instant::now();
    let mut ran = false;
    loop {
        let text = client
            .expect(
                Method::GET,
                &format!("/api/sessions/{session}/prompts/{prompt}"),
                None,
            )
            .await?;
        let mut progress: Value = serde_json::from_str(&text)?;
        if let Some(done) = finished(&progress, &mut ran) {
            progress["state"] = json!(done);
            println!("{}", serde_json::to_string(&progress)?);
            return Ok(());
        }
        if started.elapsed() + POLL > timeout {
            progress["next"] = json!(format!("cowboy session wait {session} {prompt}"));
            println!("{}", serde_json::to_string(&progress)?);
            return Ok(());
        }
        tokio::time::sleep(POLL).await;
    }
}

/// The final state for one observation, or `None` to keep waiting. A prompt
/// whose evidence left the hot window after it ran has ended once the
/// session is no longer busy.
fn finished(progress: &Value, ran: &mut bool) -> Option<&'static str> {
    match progress["state"].as_str() {
        Some("queued") => None,
        Some("running") => {
            *ran = true;
            None
        }
        Some("settled") => Some("settled"),
        Some("unknown") if *ran => {
            (!matches!(progress["status"].as_str(), Some("busy" | "starting"))).then_some("settled")
        }
        Some("unknown") => Some("withdrawn"),
        _ => Some("gone"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_ends_with_the_prompts_turn() {
        let mut ran = false;
        let state = |state: &str, status: &str| json!({"state": state, "status": status});
        assert_eq!(finished(&state("queued", "busy"), &mut ran), None);
        assert_eq!(finished(&state("running", "busy"), &mut ran), None);
        assert!(ran);
        assert_eq!(finished(&state("unknown", "busy"), &mut ran), None);
        assert_eq!(
            finished(&state("unknown", "idle"), &mut ran),
            Some("settled")
        );
        assert_eq!(
            finished(&state("settled", "idle"), &mut ran),
            Some("settled")
        );
        let mut fresh = false;
        assert_eq!(
            finished(&state("unknown", "idle"), &mut fresh),
            Some("withdrawn")
        );
    }
}
