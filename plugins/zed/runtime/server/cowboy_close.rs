// SPDX-License-Identifier: GPL-3.0-or-later
//! Confirm native peer ownership removal, not heap reclamation or LSP quiescence.
//! The adapter owns one-use dispatch. There is no native replay/recovery ticket.
use super::*;
use proto::cowboy_close_buffers_response::Outcome;

pub(super) struct State {
    instance: [u8; 16],
}

impl Default for State {
    fn default() -> Self {
        Self {
            instance: rand::random(),
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
                && request.protocol == 1,
            "unsupported private native close"
        );
        let mut response = proto::CowboyCloseBuffersResponse {
            protocol: 1,
            instance: self.cowboy_close.instance.to_vec(),
            outcome: Outcome::Supported as i32,
            buffer_ids: Vec::new(),
        };
        if request.instance.is_empty() && request.buffer_ids.is_empty() {
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
        let ids = request
            .buffer_ids
            .iter()
            .copied()
            .map(BufferId::new)
            .collect::<Result<Vec<_>>>()?;
        response.buffer_ids = request.buffer_ids;
        response.outcome = Outcome::Refused as i32;
        let Some(shared) = self.shared_buffers.get_mut(&peer) else {
            return Ok(response);
        };
        // Validate the complete set before removing any peer. No await or path
        // lookup can substitute a resource between this check and the removal.
        if ids.iter().any(|id| !shared.contains_key(id)) {
            return Ok(response);
        }
        for id in ids {
            shared.remove(&id);
            cx.emit(BufferStoreEvent::SharedBufferClosed(peer, id));
        }
        if shared.is_empty() {
            self.shared_buffers.remove(&peer);
        }
        response.outcome = Outcome::Closed as i32;
        Ok(response)
    }
}

#[cfg(test)]
mod tests;
