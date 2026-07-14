import type { Envelope } from "./protocol";
import { derive } from "./derive";

/**
 * Translate a visual-row budget into the event suffix needed to preserve it.
 * ACP histories are not one-event-per-row: a single tool card can be followed
 * by many update envelopes, while message chunks coalesce into one bubble. A
 * raw event cap can therefore collapse 170 visible rows to five and make the
 * viewport auto-loader immediately fetch them again.
 */
export function retainedEventCountForRows(
  timeline: Envelope[],
  minimumEvents: number,
  minimumRows: number,
): number {
  if (timeline.length <= minimumEvents) return timeline.length;
  const items = derive(timeline);
  const firstRetained = items[Math.max(0, items.length - minimumRows)];
  if (!firstRetained) return minimumEvents;
  const firstIndex = timeline.findIndex((event) => String(event.seq) === firstRetained.key);
  if (firstIndex < 0) return minimumEvents;
  return Math.max(minimumEvents, timeline.length - firstIndex);
}

/**
 * Keep the recent render window plus the older events that still define live
 * session UI state. Those checkpoints are tiny and do not render transcript
 * rows, but dropping them makes Plan / slash commands / permissions disappear
 * until scrollback happens to page the event back in.
 */
export function retainTimelineState(
  timeline: readonly Envelope[],
  retain: number,
): { events: Envelope[]; recentStartSeq: number | null } {
  if (timeline.length <= retain) {
    return { events: [...timeline], recentStartSeq: timeline[0]?.seq ?? null };
  }

  const recentStart = timeline.length - retain;
  const keep = new Set<number>();
  for (let index = recentStart; index < timeline.length; index += 1) keep.add(index);

  let latestPlan = -1;
  let latestCommands = -1;
  const pendingPermissions = new Map<string, number>();
  for (let index = 0; index < timeline.length; index += 1) {
    const event = timeline[index];
    if (!event) continue;
    if (event.kind === "update") {
      if (event.update.sessionUpdate === "plan") latestPlan = index;
      if (event.update.sessionUpdate === "available_commands_update") latestCommands = index;
    } else if (event.kind === "permission_request") {
      pendingPermissions.set(event.request_id, index);
    } else if (event.kind === "permission_resolved") {
      pendingPermissions.delete(event.request_id);
    }
  }

  if (latestPlan >= 0 && latestPlan < recentStart) {
    keep.add(latestPlan);
    // latestPlan also derives whether a later user turn superseded the plan.
    // Preserve one such marker when it too fell outside the recent window.
    for (let index = recentStart - 1; index > latestPlan; index -= 1) {
      const event = timeline[index];
      if (event?.kind === "update" && event.update.sessionUpdate === "user_message_chunk") {
        keep.add(index);
        break;
      }
    }
  }
  if (latestCommands >= 0 && latestCommands < recentStart) keep.add(latestCommands);
  for (const index of pendingPermissions.values()) {
    if (index < recentStart) keep.add(index);
  }

  return {
    events: [...keep].sort((a, b) => a - b).map((index) => timeline[index]!),
    recentStartSeq: timeline[recentStart]?.seq ?? null,
  };
}
