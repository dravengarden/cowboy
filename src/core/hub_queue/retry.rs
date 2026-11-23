//! Authoritative turn retry: re-dispatches the last prompt using its proven
//! original dispatch origin and the retrying caller as trigger.

use super::*;
use crate::prompt_origin::{DeliveryOrigin, RequestCaller, compute_dispatch_origin};

impl Hub {
    /// Overlay "Retry" for an errored/crashed turn: re-run the last user prompt
    /// (reviving the session). No-op if there's no prior prompt.
    ///
    /// `resolved_original` is the `(cmid, origin)` pair looked up
    /// asynchronously from durable `prompt_dispatches` before acquiring the
    /// Hub lock. It is used only if it still matches the session's current
    /// last echoed prompt `cmid`. If no dispatch record exists, original
    /// provenance is treated as non-owner.
    pub fn retry_turn(
        &self,
        session_id: &str,
        caller: &RequestCaller,
        resolved_original: Option<(String, DeliveryOrigin)>,
    ) {
        if self.is_managed_child(session_id) {
            return;
        }
        let trigger = DeliveryOrigin::from_caller(caller);
        let (prompt, mut content, status, retry_cmid, already_queued, original_origin) = {
            let sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get(session_id) else {
                tracing::warn!(session = %session_id, "retry_turn: unknown session — no-op");
                return;
            };
            let Some(last_user_seq) = s.log.iter().rev().find_map(|envelope| {
                let Event::Update { update } = &envelope.event else {
                    return None;
                };
                (update
                    .get("sessionUpdate")
                    .and_then(serde_json::Value::as_str)
                    == Some("user_message_chunk"))
                .then_some(envelope.seq)
            }) else {
                // Startup can fail before the first prompt reaches the native
                // agent. That prompt is still queued, with its original origin.
                drop(sessions);
                self.drain_head(session_id, true, true, Some(caller));
                return;
            };
            let LastUserPrompt {
                text: prompt,
                content,
                cmid: echoed_cmid,
            } = last_user_prompt(&s.log);
            let original_origin = echoed_cmid.as_deref().and_then(|c| {
                s.dispatched_origins
                    .get(c)
                    .map(|record| record.origin.clone())
                    .or_else(|| {
                        resolved_original
                            .as_ref()
                            .filter(|(resolved_cmid, _)| resolved_cmid == c)
                            .map(|(_, origin)| origin.clone())
                    })
            });
            let dispatch_origin = compute_dispatch_origin(original_origin.as_ref(), Some(&trigger));
            let base_cmid = format!("cowboy-retry:{session_id}:{last_user_seq}");
            let retry_cmid = if s
                .dispatched_origins
                .get(&base_cmid)
                .is_some_and(|existing| existing.origin != dispatch_origin)
            {
                format!("{base_cmid}:{}", uuid::Uuid::new_v4().simple())
            } else {
                base_cmid
            };
            if s.in_flight
                || s.queue
                    .iter()
                    .any(|message| message.cmid.as_deref() == Some(retry_cmid.as_str()))
            {
                tracing::info!(session = %session_id, "retry_turn: identical retry already pending — no-op");
                return;
            }
            let already_queued = s
                .queue
                .iter()
                .find(|message| {
                    if echoed_cmid.is_some() && message.cmid == echoed_cmid {
                        return true;
                    }
                    if prompt.trim().is_empty() {
                        attachment_signature(&message.content) == attachment_signature(&content)
                    } else {
                        message.text == prompt
                    }
                })
                .map(|message| message.id.clone());
            (
                prompt,
                content,
                s.meta.status,
                retry_cmid,
                already_queued,
                original_origin,
            )
        };
        if prompt.trim().is_empty() && content.is_empty() {
            tracing::warn!(session = %session_id, ?status, "retry_turn: no prior prompt to retry — no-op");
            return;
        }
        if let Some(id) = already_queued {
            tracing::info!(session = %session_id, "retry_turn: rejected prompt already queued — sending it without duplication");
            self.request_send_queued(session_id, &id, caller);
            return;
        }
        if let Err(reference) = self.restore_externalized_images(&mut content) {
            tracing::warn!(session = %session_id, ?status, %reference, "retry_turn: attachment no longer stored — not retrying");
            if matches!(
                status,
                Status::Crashed | Status::Exited | Status::Interrupted
            ) {
                self.set_status(
                    session_id,
                    status,
                    Some(
                        "Retry needs an attachment that is no longer stored; send the prompt again with the file."
                            .to_owned(),
                    ),
                );
            }
            return;
        }
        tracing::info!(session = %session_id, ?status, prompt_len = prompt.len(), blocks = content.len(), "retry_turn: re-submitting last prompt");
        let _ = self.force_submit_with_provenance(
            session_id,
            prompt,
            content,
            Some(retry_cmid),
            true,
            original_origin,
            trigger,
        );
        self.drain_head(session_id, true, true, Some(caller));
    }
}
