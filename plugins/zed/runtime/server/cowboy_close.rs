// SPDX-License-Identifier: GPL-3.0-or-later
//! Confirm native peer ownership removal, not heap reclamation or LSP quiescence.
//! Protocol 2 retains the original outcome for query-only recovery. Receipt
//! retirement never permits the same operation to execute again.
use super::*;
use proto::cowboy_close_buffers_response::Outcome;

pub(super) struct State {
    instance: [u8; 16],
    receipts: HashMap<(PeerId, u64), proto::CowboyCloseBuffersResponse>,
    last_operation: HashMap<PeerId, u64>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            instance: rand::random(),
            receipts: HashMap::default(),
            last_operation: HashMap::default(),
        }
    }
}

impl BufferStore {
    pub(super) async fn handle_cowboy_close_buffers(
        this: Entity<Self>,
        envelope: TypedEnvelope<proto::CowboyCloseBuffers>,
        mut cx: AsyncApp,
    ) -> Result<proto::CowboyCloseBuffersResponse> {
        let peer = envelope.sender_id;
        this.update(&mut cx, |this, cx| {
            this.cowboy_close_buffers(envelope.payload, peer, cx)
        })
    }

    fn cowboy_close_buffers(
        &mut self,
        request: proto::CowboyCloseBuffers,
        peer: PeerId,
        cx: &mut Context<Self>,
    ) -> Result<proto::CowboyCloseBuffersResponse> {
        anyhow::ensure!(
            matches!(self.state, BufferStoreState::Local(_))
                && request.project_id == proto::REMOTE_SERVER_PROJECT_ID
                && matches!(request.protocol, 1 | 2),
            "unsupported private native close"
        );
        let mut response = proto::CowboyCloseBuffersResponse {
            protocol: request.protocol,
            instance: self.cowboy_close.instance.to_vec(),
            outcome: Outcome::Supported as i32,
            buffer_ids: Vec::new(),
            operation_id: request.operation_id,
        };
        if request.instance.is_empty()
            && request.buffer_ids.is_empty()
            && request.operation_id == 0
            && request.action == 0
        {
            return Ok(response);
        }
        anyhow::ensure!(
            request.instance == self.cowboy_close.instance
                && !request.buffer_ids.is_empty()
                && request.buffer_ids.len() <= 33
                && request.buffer_ids[0] != 0
                && request.buffer_ids.windows(2).all(|ids| ids[0] < ids[1]),
            "invalid original native close"
        );
        anyhow::ensure!(
            (request.protocol == 1 && request.operation_id == 0 && request.action == 0)
                || (request.protocol == 2
                    && request.operation_id > 0
                    && (0..=2).contains(&request.action)),
            "invalid original native close operation"
        );
        let key = (peer, request.operation_id);
        response.buffer_ids = request.buffer_ids.clone();
        if request.protocol == 2 {
            if let Some(saved) = self.cowboy_close.receipts.get(&key) {
                anyhow::ensure!(
                    saved.buffer_ids == request.buffer_ids,
                    "native close operation changed"
                );
                if request.action != 2 {
                    return Ok(saved.clone());
                }
            }
            if request.action == 2 {
                if self
                    .cowboy_close
                    .last_operation
                    .get(&peer)
                    .is_none_or(|last| request.operation_id > *last)
                {
                    response.outcome = Outcome::Unknown as i32;
                    return Ok(response);
                }
                self.cowboy_close.receipts.remove(&key);
                response.outcome = Outcome::Forgotten as i32;
                return Ok(response);
            }
            if request.action == 1
                || self
                    .cowboy_close
                    .last_operation
                    .get(&peer)
                    .is_some_and(|last| request.operation_id <= *last)
            {
                response.outcome = Outcome::Unknown as i32;
                return Ok(response);
            }
            if self.cowboy_close.receipts.len() >= 4096
                || (!self.cowboy_close.last_operation.contains_key(&peer)
                    && self.cowboy_close.last_operation.len() >= 64)
            {
                response.outcome = Outcome::Capacity as i32;
                return Ok(response);
            }
        }
        let ids = request
            .buffer_ids
            .iter()
            .copied()
            .map(BufferId::new)
            .collect::<Result<Vec<_>>>()?;
        response.outcome = Outcome::Refused as i32;
        // Validate the complete set before removing any peer. No await or path
        // lookup can substitute a resource between this check and the removal.
        if let Some(shared) = self.shared_buffers.get_mut(&peer)
            && ids.iter().all(|id| shared.contains_key(id))
        {
            for id in ids {
                shared.remove(&id);
                cx.emit(BufferStoreEvent::SharedBufferClosed(peer, id));
            }
            if shared.is_empty() {
                self.shared_buffers.remove(&peer);
            }
            response.outcome = Outcome::Closed as i32;
        }
        if request.protocol == 2 {
            self.cowboy_close
                .last_operation
                .insert(peer, request.operation_id);
            self.cowboy_close.receipts.insert(key, response.clone());
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests;
