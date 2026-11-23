//! Relayed execution requests of one endpoint connection. Each admitted
//! request runs to its end on its own task, independent of the socket that
//! carried it: closing the socket never aborts a request the keeper may have
//! admitted. A mutation whose reply could not be delivered to the agent side
//! (the socket closed first, or the write failed) has an outcome the agent
//! never learned, so the session's ledger records it as uncertain.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use serde_json::Value;
use tokio::sync::mpsc;

use super::Client;

/// One finished relay: its request id, method and reply.
pub(super) struct Done {
    pub id: Value,
    method: String,
    /// Decided before the request ran (an exit forgets the process).
    carries_result: bool,
    reply: Value,
    /// Counted in flight until this reply was handled (written to the unit,
    /// or recorded undelivered): dropped with it.
    _in_flight: crate::account_sandbox::RelayGuard,
}

impl Done {
    /// The reply frame for the agent side.
    pub fn frame(&self) -> Result<String> {
        let mut reply = self.reply.clone();
        reply
            .as_object_mut()
            .context("invalid execution result")?
            .insert("id".to_owned(), self.id.clone());
        Ok(reply.to_string())
    }
}

pub(super) struct Relays {
    client: Arc<Client>,
    sender: mpsc::UnboundedSender<Done>,
    receiver: Option<mpsc::UnboundedReceiver<Done>>,
    pending: usize,
}

impl Relays {
    pub fn new(client: Arc<Client>) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            client,
            sender,
            receiver: Some(receiver),
            pending: 0,
        }
    }

    #[must_use]
    pub fn pending(&self) -> usize {
        self.pending
    }

    /// Relay one request on its own task.
    pub fn spawn(&mut self, id: Value, method: String, params: Value) {
        let (client, sender) = (Arc::clone(&self.client), self.sender.clone());
        self.pending += 1;
        let in_flight = self.client.relays.enter();
        tokio::spawn(async move {
            let carries_result = client.carries_result(&method, &params);
            let reply = client.relay(method.clone(), params).await;
            let done = Done {
                id,
                method,
                carries_result,
                reply,
                _in_flight: in_flight,
            };
            if let Err(mpsc::error::SendError(done)) = sender.send(done) {
                client.undelivered(&done.method, done.carries_result, &done.reply);
            }
        });
    }

    /// The next finished relay; pending forever while none is outstanding.
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

    /// `done`'s reply never reached the agent side.
    pub fn undelivered(&self, done: &Done) {
        self.client
            .undelivered(&done.method, done.carries_result, &done.reply);
    }
}

impl Drop for Relays {
    /// The connection ended: every relay still running finishes on its own
    /// and its reply, which nobody will read, is recorded as undelivered.
    fn drop(&mut self) {
        let Some(mut receiver) = self.receiver.take() else {
            return;
        };
        if self.pending == 0 {
            return;
        }
        let client = Arc::clone(&self.client);
        // `self.sender` drops with `self`; the channel closes once every
        // running relay has sent its reply.
        tokio::spawn(async move {
            while let Some(done) = receiver.recv().await {
                client.undelivered(&done.method, done.carries_result, &done.reply);
            }
        });
    }
}
