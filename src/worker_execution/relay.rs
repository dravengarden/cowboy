//! Relayed execution requests of one endpoint connection. Each admitted
//! request runs to its end on its own task, independent of the socket that
//! carried it: closing the socket never aborts a request the keeper may have
//! admitted. A mutation whose reply could not be delivered to the agent side
//! (the socket closed first, or the write failed) has an outcome the agent
//! never learned, so the session's ledger records it as uncertain.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::Client;

/// Whether the extension broker owns correlated-ack settlement for this reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AckState {
    /// Ordinary Phase-1 or native request: `Relays` / `unread_effects` owns it.
    None,
    /// User-MCP reply awaiting correlated `cowboy/extensionCallAck({ callId })`.
    /// Top-level `cowboyAck` frame field is attached by `Done::frame()`.
    Pending(String),
    /// Broker already converted or settled this user-MCP reply (e.g. socket
    /// closed before registration or `MaybeSent` already converted the sentinel).
    /// All four relay paths skip `unread_effects` to prevent duplicate uncertainty.
    AlreadySettled,
}

/// One finished relay: its request id, method and reply.
pub(super) struct Done {
    pub id: Value,
    method: String,
    /// Decided before the request ran (an exit forgets the process).
    carries_result: bool,
    reply: Value,
    ack: AckState,
    /// Counted in flight until this reply was handled (written to the unit,
    /// or recorded undelivered): dropped with it.
    _in_flight: crate::account_sandbox::RelayGuard,
    /// Keeps the ledger admission/result-relay count held until delivery or
    /// undelivered classification completes (dropped after `delivered`/`undelivered`).
    _ledger_guard: Option<super::ledger::SettlementGuard>,
    /// Reserved synchronously at execution socket ingress before `tokio::spawn`
    /// so cooperative crashed-worker drain cannot seal and emit a clean receipt
    /// ahead of queued `cowboy/uncertainEffect` or result-bearing cleanup relays (R8-H2).
    _ingress_guard: Option<super::ledger::UncertainReportGuard>,
}

impl Done {
    /// The reply frame for the agent side.
    pub fn frame(&self) -> Result<String> {
        let mut reply = self.reply.clone();
        let object = reply.as_object_mut().context("invalid execution result")?;
        object.insert("id".to_owned(), self.id.clone());
        if let AckState::Pending(call_id) = &self.ack {
            // Top-level frame field outside `result`/`error` so a malicious
            // user server can never spoof or suppress ack metadata.
            object.insert("cowboyAck".to_owned(), json!({"callId": call_id}));
        }
        Ok(reply.to_string())
    }
}

fn handle_undelivered(client: &Client, done: &Done) {
    match &done.ack {
        AckState::None => {
            client.undelivered(&done.method, done.carries_result, &done.reply);
        }
        AckState::Pending(call_id) => {
            if let Some(broker) = client.extensions.get() {
                broker.fail_pending_ack(call_id);
            }
        }
        AckState::AlreadySettled => {}
    }
}

pub(super) struct Relays {
    client: Arc<Client>,
    connection_id: u64,
    sender: mpsc::UnboundedSender<Done>,
    receiver: Option<mpsc::UnboundedReceiver<Done>>,
    pending: usize,
}

impl Relays {
    pub fn new(client: Arc<Client>, connection_id: u64) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            client,
            connection_id,
            sender,
            receiver: Some(receiver),
            pending: 0,
        }
    }

    #[must_use]
    pub fn pending(&self) -> usize {
        self.pending
    }

    pub fn spawn(
        &mut self,
        id: Value,
        method: String,
        params: Value,
        in_flight: crate::account_sandbox::RelayGuard,
    ) {
        let ingress_guard = if method == crate::account_sandbox::DRAINED
            || method == crate::account_sandbox::EXTENSION_CALL_ACK
        {
            None
        } else if let Some(ledger) = self.client.ledger() {
            match ledger.enter_uncertain_report() {
                Ok(guard) => Some(guard),
                Err(reason) => {
                    self.pending += 1;
                    let done = Done {
                        id,
                        method,
                        carries_result: false,
                        reply: json!({"error": {"code": -32000, "message": reason}}),
                        ack: AckState::None,
                        _in_flight: in_flight,
                        _ledger_guard: None,
                        _ingress_guard: None,
                    };
                    let _ = self.sender.send(done);
                    return;
                }
            }
        } else {
            None
        };
        let (client, sender, connection_id) = (
            Arc::clone(&self.client),
            self.sender.clone(),
            self.connection_id,
        );
        self.pending += 1;
        tokio::spawn(async move {
            let carries_result = client.carries_result(&method, &params);
            let (reply, ledger_guard, ack) = client
                .relay_guarded(method.clone(), params, connection_id)
                .await;
            let done = Done {
                id,
                method,
                carries_result,
                reply,
                ack,
                _in_flight: in_flight,
                _ledger_guard: ledger_guard,
                _ingress_guard: ingress_guard,
            };
            if let Err(mpsc::error::SendError(done)) = sender.send(done) {
                handle_undelivered(&client, &done);
                drop(done);
            }
        });
    }

    pub async fn next(&mut self) -> Done {
        let Some(receiver) = self.receiver.as_mut().filter(|_| self.pending > 0) else {
            return std::future::pending().await;
        };
        match receiver.recv().await {
            Some(done) => {
                self.pending -= 1;
                done
            }
            None => std::future::pending().await,
        }
    }

    pub fn delivered(&self, done: &Done) {
        if !matches!(done.ack, AckState::None) {
            return;
        }
        if self.client.relays.draining() {
            self.client.relays.written(&super::ledger::unread_effects(
                &done.method,
                done.carries_result,
                &done.reply,
            ));
        }
    }

    pub fn undelivered(&self, done: &Done) {
        handle_undelivered(&self.client, done);
    }
}

impl Drop for Relays {
    fn drop(&mut self) {
        let Some(mut receiver) = self.receiver.take() else {
            return;
        };
        if self.pending == 0 {
            return;
        }
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            while let Some(done) = receiver.recv().await {
                handle_undelivered(&client, &done);
            }
        });
    }
}

static NEXT_MUTATION_CONNECTION_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

pub(super) fn next_mutation_connection_id() -> u64 {
    NEXT_MUTATION_CONNECTION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub(super) struct BrokerConnectionGuard {
    pub client: Arc<Client>,
    pub connection_id: u64,
}

impl Drop for BrokerConnectionGuard {
    fn drop(&mut self) {
        if let Some(broker) = self.client.extensions.get() {
            broker.connection_closed(self.connection_id);
        }
    }
}

/// Bounded in-flight ID tracking for one mutation socket, giving worker-local
/// control requests (`cowboy/uncertainEffect`, `cowboy/drained`, cap 4) and
/// correlated user-MCP call acknowledgements (`cowboy/extensionCallAck`, cap 8)
/// admission lanes independent of the 16 forwarded execution requests.
pub(super) struct MutationInflightIds {
    forwarded: std::collections::HashSet<String>,
    control: std::collections::HashSet<String>,
    ack: std::collections::HashSet<String>,
}

impl MutationInflightIds {
    const MAX_FORWARDED: usize = 16;
    const MAX_CONTROL: usize = 4;
    const MAX_ACK: usize = 8;

    pub fn new() -> Self {
        Self {
            forwarded: std::collections::HashSet::new(),
            control: std::collections::HashSet::new(),
            ack: std::collections::HashSet::new(),
        }
    }

    pub fn is_control(method: &str) -> bool {
        matches!(
            method,
            super::ledger::UNCERTAIN_REPORT | crate::account_sandbox::DRAINED
        )
    }

    pub fn is_ack(method: &str) -> bool {
        method == crate::account_sandbox::EXTENSION_CALL_ACK
    }

    pub fn try_admit(&mut self, id: &Value, method: &str) -> Result<bool> {
        let key = id.to_string();
        anyhow::ensure!(
            super::valid_id(id)
                && !self.forwarded.contains(&key)
                && !self.control.contains(&key)
                && !self.ack.contains(&key),
            "invalid or duplicate execution request id"
        );
        let (set, limit) = if Self::is_ack(method) {
            (&mut self.ack, Self::MAX_ACK)
        } else if Self::is_control(method) {
            (&mut self.control, Self::MAX_CONTROL)
        } else {
            (&mut self.forwarded, Self::MAX_FORWARDED)
        };
        if set.len() >= limit {
            return Ok(false);
        }
        set.insert(key);
        Ok(true)
    }

    pub fn remove(&mut self, id: &Value) {
        let key = id.to_string();
        self.forwarded.remove(&key);
        self.control.remove(&key);
        self.ack.remove(&key);
    }

    pub fn pending_for(&self, method: &str) -> usize {
        if Self::is_ack(method) {
            self.ack.len()
        } else if Self::is_control(method) {
            self.control.len()
        } else {
            self.forwarded.len()
        }
    }
}
