// Pure translation from durable queue outbox mutations to Inbound WS commands.

import type { Attachment } from "./attachments";
import { type DraftActivation, draftActivationSourceId } from "./queueMutators";
import type { ContentBlock, DraftSchedule, Inbound, SessionMeta } from "./protocol";
import { type WrapUpOutboxFields, wrapUpActivateFields, wrapUpSubmitFields } from "./wrapUpDelivery";

export interface QueueMutationRow extends WrapUpOutboxFields {
  readonly id: string;
  readonly cmid?: string;
  readonly text: string;
  readonly attachments: Attachment[];
  readonly schedule?: DraftSchedule;
}

export function buildQueueMutationCommand(
  sessionId: string,
  m: { name: string; id: string; args: unknown },
  baseDrafts: readonly QueueMutationRow[],
  sessions: readonly SessionMeta[],
  contentOf: (text: string, attachments: readonly Attachment[]) => ContentBlock[],
  draftWrapUpSupported = true,
): Inbound | null {
  const args = m.args as { id?: string; row?: QueueMutationRow; issuedAtMs?: number };
  if (m.name === "addDraft" && args.row !== undefined) {
    return {
      type: "add_draft",
      session_id: sessionId,
      text: args.row.text,
      content: contentOf(args.row.text, args.row.attachments),
      cmid: m.id,
    };
  }
  if (m.name === "scheduleDraft" && args.row?.schedule !== undefined) {
    return {
      type: "schedule_draft",
      session_id: sessionId,
      cmid: m.id,
      text: args.row.text,
      content: contentOf(args.row.text, args.row.attachments),
      fire_at_ms: args.row.schedule.fire_at_ms,
      delivery: args.row.schedule.delivery,
    };
  }
  if (
    m.name === "rescheduleDraft" &&
    args.id !== undefined &&
    args.row?.schedule !== undefined
  ) {
    return {
      type: "schedule_draft",
      session_id: sessionId,
      id: args.id,
      text: args.row.text,
      content: contentOf(args.row.text, args.row.attachments),
      fire_at_ms: args.row.schedule.fire_at_ms,
      delivery: args.row.schedule.delivery,
    };
  }
  if (
    (m.name === "editDraft" || m.name === "editQueue") &&
    args.id !== undefined &&
    args.row !== undefined
  ) {
    const targetId = m.name === "editDraft"
      ? draftActivationSourceId(
        m.args as { id: string; sourceCmid?: string },
        baseDrafts,
      )
      : args.id;
    return targetId === null ? null : {
      type: m.name === "editDraft" ? "edit_draft" : "edit_queued",
      session_id: sessionId,
      id: targetId,
      text: args.row.text,
      content: contentOf(args.row.text, args.row.attachments),
      cmid: m.id,
    };
  }
  if (m.name === "removeDraft" && args.id !== undefined) {
    return { type: "remove_draft", session_id: sessionId, id: args.id, cmid: m.id };
  }
  if (m.name === "removeQueue" && args.id !== undefined) {
    return { type: "remove_queued", session_id: sessionId, id: args.id, cmid: m.id };
  }
  if (m.name === "unscheduleDraft" && args.id !== undefined) {
    return { type: "unschedule_draft", session_id: sessionId, id: args.id };
  }
  if (m.name === "activateDraft" && args.id !== undefined) {
    const activation = m.args as DraftActivation;
    const id = draftActivationSourceId(activation, baseDrafts);
    return id === null ? null : {
      type: "activate_draft",
      session_id: sessionId,
      id,
      cmid: args.row?.cmid ?? m.id,
      ...(activation.front ? { front: true } : {}),
      ...wrapUpActivateFields(sessions, sessionId, activation, draftWrapUpSupported),
    };
  }
  if ((m.name === "sendQueued" || m.name === "forceQueued") && args.id !== undefined) {
    return {
      type: m.name === "sendQueued" ? "request_send_queued" : "force_push_queued",
      session_id: sessionId,
      id: args.id,
      cmid: m.id,
      ...(typeof args.issuedAtMs === "number" ? { issued_ms: args.issuedAtMs } : {}),
    };
  }
  if (m.name === "returnQueuedToDraft" && args.id !== undefined) {
    return { type: "queued_to_draft", session_id: sessionId, id: args.id };
  }
  const row = args.row;
  if (row === undefined) {
    return { type: "activate_draft", session_id: sessionId, id: args.id ?? m.id };
  }
  return {
    type: "submit",
    session_id: sessionId,
    text: row.text,
    content: contentOf(row.text, row.attachments),
    cmid: m.id,
    ...(m.name === "forceQueue" && { force: true }),
    ...(m.name === "frontQueue" && { front: true }),
    ...wrapUpSubmitFields(sessions, sessionId, m.name, row),
  };
}
