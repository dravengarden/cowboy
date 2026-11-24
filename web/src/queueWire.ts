import { blocksToAttachments } from "./attachments";
import type { WireQueued } from "./protocol";
import type { QueuedMessage } from "./store";

// Convert a server `WireQueued` (raw ACP content blocks) into the UI shape, with
// attachments reconstructed for display / re-edit.
export function fromWire(list: WireQueued[]): QueuedMessage[] {
  return list.map((m) => ({
    id: m.id,
    text: m.text,
    attachments: blocksToAttachments(m.content, m.text),
    // Carried so an optimistic row can reconcile against its confirmed twin by
    // id. `?? undefined` keeps `exactOptionalPropertyTypes` happy.
    ...(m.cmid !== undefined && { cmid: m.cmid }),
    ...(m.schedule !== undefined && { schedule: m.schedule }),
  }));
}
