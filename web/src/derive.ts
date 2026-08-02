// Fold a session's flat event log into an ordered list of render items:
// coalesced messages/thoughts, tool calls (status updated in place), the
// latest plan, and unresolved permission requests. This is the multi-agent
// transcript model — no code editor / file tree / git, just the conversation.

import type { AcpUpdate, Envelope, PermissionOption, PlanEntry, Status } from "./protocol";

/// A renderable slice of a message — text and images can interleave inside
/// one message item (e.g. user pastes "describe this:" + image + " thanks").
export type ContentChunk =
  | { type: "text"; text: string }
  | { type: "image"; src: string; alt?: string };

// Every item carries a STABLE `key` — the seq of the envelope that first
// created it (coalesced message/thought items keep their first chunk's seq).
// The transcript keys rows by this so prepending older history (which shifts
// every array index) doesn't re-mount/jump the visible rows; index keys can't.
export type RenderItem = { key: string } & (
  | { kind: "message"; role: "assistant" | "user"; chunks: ContentChunk[]; autoResumed?: boolean }
  | { kind: "thought"; sections: string[] }
  | {
      kind: "tool";
      id: string;
      title: string;
      toolKind: string;
      /// Upstream tool NAME (`_meta.<provider>.toolName`, e.g. "Bash" / "Edit" /
      /// codex "shell"), or "" when the agent didn't tag one. Drives the
      /// per-tool renderer in tools/registry; `toolKind` is the ACP fallback.
      toolName: string;
      status: string;
      /// Raw input JSON (tool args), preserved for the expanded card.
      rawInput?: unknown;
      /// Tool result content (text / files / diff) as the upstream sent it.
      content?: unknown;
    }
  | {
      kind: "permission";
      requestId: string;
      title: string;
      options: PermissionOption[];
      resolved: boolean;
      chosen: string | null;
    }
  | { kind: "lifecycle"; status: Status; detail: string | null }
  // "Clear conversation" divider: the agent was reset to a fresh context here.
  // Everything ABOVE is transcript-only (the agent no longer remembers it).
  | { kind: "cleared"; at: number }
);

// Store timelines are immutable arrays. Transcript and Composer often derive
// the same array in one render, so share the fold by identity.
const DERIVE_CACHE = new WeakMap<Envelope[], RenderItem[]>();
const TIMELINE_PARENT = new WeakMap<Envelope[], Envelope[]>();

/** Record immutable timeline ancestry so a new derivation can preserve the
 * identity of unchanged render rows. The store calls this whenever it creates
 * a successor array; WeakMap keys keep released history collectible. */
export function linkTimeline(next: Envelope[], previous: Envelope[]): Envelope[] {
  if (next !== previous) {
    // Point straight at the latest derivable ancestor instead of chaining every
    // per-token array. Subscriber notifications are frame-batched, so several
    // intermediate arrays may never render; a chain would retain all of them.
    const ancestor = DERIVE_CACHE.has(previous)
      ? previous
      : TIMELINE_PARENT.get(previous);
    if (ancestor) TIMELINE_PARENT.set(next, ancestor);
  }
  return next;
}

function sameChunks(a: ContentChunk[], b: ContentChunk[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((chunk, index) => {
    const other = b[index];
    if (!other || chunk.type !== other.type) return false;
    return chunk.type === "text"
      ? other.type === "text" && chunk.text === other.text
      : other.type === "image" && chunk.src === other.src && chunk.alt === other.alt;
  });
}

function sameStrings(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((value, index) => value === b[index]);
}

/** Cheap semantic equality for structural sharing. Large tool payloads stay
 * reference-compared: unchanged envelopes retain their nested object identity,
 * while an actual tool update supplies a new object and must re-render. */
function sameRenderItem(a: RenderItem, b: RenderItem): boolean {
  if (a.kind !== b.kind || a.key !== b.key) return false;
  switch (a.kind) {
    case "message":
      return b.kind === "message" && a.role === b.role &&
        a.autoResumed === b.autoResumed && sameChunks(a.chunks, b.chunks);
    case "thought":
      return b.kind === "thought" && sameStrings(a.sections, b.sections);
    case "tool":
      return b.kind === "tool" && a.id === b.id && a.title === b.title &&
        a.toolKind === b.toolKind && a.toolName === b.toolName &&
        a.status === b.status && Object.is(a.rawInput, b.rawInput) &&
        Object.is(a.content, b.content);
    case "permission":
      return b.kind === "permission" && a.requestId === b.requestId &&
        a.title === b.title && a.resolved === b.resolved &&
        a.chosen === b.chosen && Object.is(a.options, b.options);
    case "lifecycle":
      return b.kind === "lifecycle" && a.status === b.status && a.detail === b.detail;
    case "cleared":
      return b.kind === "cleared" && a.at === b.at;
  }
}

function shareUnchangedRows(timeline: Envelope[], items: RenderItem[]): RenderItem[] {
  const parent = TIMELINE_PARENT.get(timeline);
  TIMELINE_PARENT.delete(timeline);
  const previous = parent ? DERIVE_CACHE.get(parent) : undefined;
  if (!previous?.length || !items.length) return items;
  const byKey = new Map(previous.map((item) => [item.key, item]));
  let changed = false;
  const shared = items.map((item) => {
    const prior = byKey.get(item.key);
    if (prior && sameRenderItem(prior, item)) {
      changed = true;
      return prior;
    }
    return item;
  });
  return changed ? shared : items;
}

/// Convert an ACP content block into a renderable chunk (or null if we don't
/// support that type yet).
function chunkOf(update: AcpUpdate): ContentChunk | null {
  const c = update.content as
    | undefined
    | {
        type?: string;
        text?: string;
        data?: string;
        url?: string;
        mimeType?: string;
        media_type?: string;
        name?: string;
        uri?: string;
        source?: { type?: string; data?: string; url?: string; media_type?: string };
        resource?: { uri?: string; text?: string; blob?: string; mimeType?: string };
      };
  if (!c) return null;
  if (c.type === "text") {
    return { type: "text", text: c.text ?? "" };
  }
  if (c.type === "resource" || c.type === "resource_link") {
    // A file the user attached (embedded resource or link). The agent reads the
    // bytes; in the transcript we just echo a paperclip + the file name so the
    // user sees what they sent. Derive a readable name from the uri.
    const uri = c.resource?.uri ?? c.uri ?? "";
    const name = c.name ?? decodeURIComponent(uri.split("/").pop() ?? uri) ?? "attachment";
    return { type: "text", text: `📎 ${name}` };
  }
  if (c.type === "image") {
    // ACP image content blocks have a few shapes; cover both flat
    // {data, mimeType, url} and nested {source: {data|url, media_type}}.
    const src = c.source ?? c;
    if (src.url) {
      return { type: "image", src: src.url };
    }
    if (src.data) {
      const mt = src.media_type ?? c.mimeType ?? "image/png";
      return { type: "image", src: `data:${mt};base64,${src.data}` };
    }
  }
  return null;
}

function textOf(update: AcpUpdate): string {
  const ch = chunkOf(update);
  return ch?.type === "text" ? ch.text : "";
}

// Codex uses empty HTML comments as separators inside reasoning summaries. Turn
// those provider-internal markers into structure before Markdown sees them, so
// the transcript can render distinct thinking steps without exposing raw HTML.
// Non-empty comments remain part of the text, as do comments in normal messages.
function thoughtSectionsOf(text: string): string[] {
  return text.split(/<!--\s*-->/g);
}

function pushThoughtText(item: { sections: string[] }, incoming: string): void {
  // Reparse the unfinished tail together with the next chunk. ACP may split an
  // HTML separator at any byte boundary (`<!--` then ` -->`), and parsing each
  // notification independently would leak that partial marker into the UI.
  const tail = item.sections.pop() ?? "";
  item.sections.push(...thoughtSectionsOf(tail + incoming));
}

/// The upstream tool name lives under `_meta.<provider>.toolName` (e.g.
/// `_meta.claudeCode.toolName = "Bash"`). The provider key varies, so scan the
/// one-level `_meta` object for any `{ toolName }`. "" when absent.
function toolNameOf(update: AcpUpdate): string {
  const meta = update["_meta"];
  if (meta && typeof meta === "object") {
    for (const v of Object.values(meta as Record<string, unknown>)) {
      if (v && typeof v === "object") {
        const tn = (v as { toolName?: unknown }).toolName;
        if (typeof tn === "string") return tn;
      }
    }
  }
  return "";
}

// Codex MCP read calls may expose their useful payload through the generic
// `locations` + `rawOutput.formatted_output` fields instead of ACP's optional
// `rawInput` + `content` pair. Normalize both shapes once so the transcript can
// always offer the same details surface.
function toolInputOf(update: AcpUpdate): unknown {
  const explicit = update["rawInput"] ?? update["input"];
  if (explicit !== undefined) return explicit;
  const locations = update["locations"];
  if (!Array.isArray(locations) || locations.length === 0) return undefined;
  const first = locations[0];
  if (first && typeof first === "object") {
    const path = (first as { path?: unknown }).path;
    if (typeof path === "string") return { path, locations };
  }
  return { locations };
}

function toolContentOf(update: AcpUpdate): unknown {
  if (update["content"] !== undefined) return update["content"];
  const rawOutput = update["rawOutput"];
  if (!rawOutput || typeof rawOutput !== "object") return undefined;
  const formatted = (rawOutput as { formatted_output?: unknown }).formatted_output;
  return typeof formatted === "string"
    ? [{ type: "raw_output", text: formatted }]
    : undefined;
}

function pushChunk(item: { chunks: ContentChunk[] }, chunk: ContentChunk): void {
  const last = item.chunks[item.chunks.length - 1];
  if (last && last.type === "text" && chunk.type === "text") {
    last.text += chunk.text;
  } else {
    item.chunks.push(chunk);
  }
}

export function derive(timeline: Envelope[]): RenderItem[] {
  const cached = DERIVE_CACHE.get(timeline);
  if (cached) return cached;
  const items: RenderItem[] = [];
  const toolIndex = new Map<string, number>();
  const permIndex = new Map<string, number>();

  // Coalesce consecutive chunks of the same role/thought into one item.
  let cursor: { kind: "message" | "thought"; role: "assistant" | "user" } | null = null;

  for (const env of timeline) {
    if (env.kind !== "update") cursor = null;

    switch (env.kind) {
      case "update": {
        const u = env.update;
        switch (u.sessionUpdate) {
          case "agent_message_chunk":
          case "user_message_chunk": {
            const role = u.sessionUpdate === "user_message_chunk" ? "user" : "assistant";
            const chunk = chunkOf(u);
            if (!chunk) break;
            const last = items[items.length - 1];
            if (cursor?.kind === "message" && cursor.role === role && last?.kind === "message") {
              pushChunk(last, chunk);
            } else {
              // An auto-resume continuation echo carries `autoResumed` (set by the
              // daemon, src/acp.rs) so the UI marks it as a resumed turn rather
              // than letting it read as a user message the human typed.
              const autoResumed = u["autoResumed"] === true;
              items.push({ kind: "message", role, chunks: [chunk], key: String(env.seq), autoResumed });
              cursor = { kind: "message", role };
            }
            break;
          }
          case "agent_thought_chunk": {
            const text = textOf(u);
            const last = items[items.length - 1];
            if (cursor?.kind === "thought" && last?.kind === "thought") {
              pushThoughtText(last, text);
            } else {
              items.push({ kind: "thought", sections: thoughtSectionsOf(text), key: String(env.seq) });
              cursor = { kind: "thought", role: "assistant" };
            }
            break;
          }
          case "tool_call": {
            const id = u.toolCallId ?? "";
            items.push({
              kind: "tool",
              key: String(env.seq),
              id,
              title: u.title ?? id,
              toolKind: u.kind ?? "other",
              toolName: toolNameOf(u),
              status: u.status ?? "pending",
              rawInput: toolInputOf(u),
              content: toolContentOf(u),
            });
            toolIndex.set(id, items.length - 1);
            break;
          }
          case "tool_call_update": {
            const id = u.toolCallId ?? "";
            const idx = toolIndex.get(id);
            const existing = idx === undefined ? undefined : items[idx];
            if (existing && existing.kind === "tool") {
              if (u.status) existing.status = u.status;
              if (u.title) existing.title = u.title;
              if (!existing.toolName) existing.toolName = toolNameOf(u);
              const input = toolInputOf(u);
              const content = toolContentOf(u);
              if (input !== undefined) existing.rawInput = input;
              if (content !== undefined) existing.content = content;
            }
            break;
          }
          case "context_cleared": {
            // The "Clear conversation" reset (src/core.rs mark_context_cleared):
            // render a divider so the user sees where the agent's memory was cut.
            const at = u["at"];
            items.push({
              kind: "cleared",
              at: typeof at === "number" ? at : 0,
              key: String(env.seq),
            });
            break;
          }
          // `plan` is NOT rendered inline — it's surfaced by the pinned PlanDock
          // above the composer (see latestPlan). It updates latest-wins anyway,
          // so an inline card would just be a duplicate that scrolls away.
          default:
            // available_commands_update / current_mode_update: metadata, not
            // rendered in the transcript for v1.
            break;
        }
        break;
      }
      case "permission_request": {
        items.push({
          kind: "permission",
          key: String(env.seq),
          requestId: env.request_id,
          title: titleOfToolCall(env.tool_call),
          options: env.options,
          resolved: false,
          chosen: null,
        });
        permIndex.set(env.request_id, items.length - 1);
        break;
      }
      case "permission_resolved": {
        const idx = permIndex.get(env.request_id);
        const p = idx === undefined ? undefined : items[idx];
        if (p && p.kind === "permission") {
          p.resolved = true;
          p.chosen = env.option_id;
        }
        break;
      }
      case "lifecycle": {
        // Only surface notable transitions (crash/exit); running/busy show in
        // the header.
        if (env.status === "crashed" || env.status === "exited") {
          const previous = items.at(-1);
          if (
            previous?.kind === "lifecycle" && previous.status === env.status &&
            (previous.detail === env.detail || previous.detail === null || env.detail === null)
          ) {
            if (env.detail !== null) previous.detail = env.detail;
            previous.key = String(env.seq);
          } else {
            items.push({ kind: "lifecycle", status: env.status, detail: env.detail, key: String(env.seq) });
          }
        }
        break;
      }
      case "turn_end": {
        // The turn is over, so nothing from it is still "in flight". Some agents
        // end a turn without emitting the final tool_call_update (or it races the
        // stop), leaving a tool stuck on `pending`/`in_progress`. Settle those to
        // a terminal state here so (a) the card stops showing a live chip after
        // the turn, and (b) the "agent is working" indicator — which keys off any
        // in-flight tool to survive status races — doesn't spin forever on a
        // replayed/abandoned turn. `completed` is the likeliest truth for a tool
        // the agent moved past; a genuinely failed one would have reported it.
        for (const it of items) {
          if (it.kind === "tool" && (it.status === "pending" || it.status === "in_progress")) {
            it.status = "completed";
          }
        }
        break;
      }
    }
  }
  // Drop thought items that never accrued any text. An `agent_thought_chunk`
  // with empty/unsupported content (some agents emit a thinking-block *marker*
  // with no text on a trivial turn) would otherwise leave a `text: ""` thought
  // that ItemView renders as a perpetual loading spinner — it kept "thinking"
  // forever, even after the turn ended, because that render path isn't gated on
  // session status. The transient "thinking, no text yet" state is covered by
  // the trailing indicator instead, so an empty thought carries nothing.
  const filtered = items.filter((it) =>
    it.kind !== "thought" || it.sections.some((section) => section.trim() !== "")
  );
  const result = shareUnchangedRows(timeline, filtered);
  DERIVE_CACHE.set(timeline, result);
  return result;
}

/// The session's current plan, or null if it has none.
export interface CurrentPlan {
  entries: PlanEntry[];
  /// A stable id for this plan: the list of step contents. Unchanged as the
  /// agent flips statuses (in_progress → completed), different when it starts a
  /// genuinely new plan — so manual-dismiss tracking + React identity survive
  /// progress updates but reset for a new task.
  key: string;
  /// True once a user prompt has been sent AFTER this plan was emitted. ACP has
  /// no "plan done/clear" signal, so this ordering fact (every prompt echoes a
  /// `user_message_chunk` — see src/acp.rs) is how we know the plan belongs to a
  /// finished, past exchange rather than the current one.
  supersededByUserTurn: boolean;
}

/// The session's current plan (latest ACP `plan` update's entries) + the signals
/// the dock needs to decide whether it's still relevant. A cheap single pass so
/// the pinned PlanDock can read it without deriving the whole transcript. Null
/// when there's no plan / an empty one.
export function latestPlan(timeline: Envelope[]): CurrentPlan | null {
  let entries: PlanEntry[] | null = null;
  let superseded = false;
  for (const env of timeline) {
    if (env.kind === "update" && env.update.sessionUpdate === "plan") {
      entries = env.update.entries ?? [];
      superseded = false; // a fresh plan: reset the "new turn after it" flag
    } else if (
      entries &&
      env.kind === "update" &&
      env.update.sessionUpdate === "user_message_chunk"
    ) {
      superseded = true; // a user prompt landed after the latest plan
    }
  }
  if (!entries || entries.length === 0) return null;
  return {
    entries,
    key: entries.map((e) => e.content).join("\0"),
    supersededByUserTurn: superseded,
  };
}

/// The latest UNRESOLVED tool-permission request, or null. A cheap single pass
/// (mirrors latestPlan) so the composer's sticky PermissionOverlay can read it
/// WITHOUT deriving the whole transcript — and, by living next to the turn-status
/// overlay, the two coordinate (a pending permission outranks the status pill)
/// instead of overlapping. A request stays pending until its matching
/// `permission_resolved` removes it.
export interface PendingPermission {
  requestId: string;
  title: string;
  options: PermissionOption[];
}
export function latestPendingPermission(timeline: Envelope[]): PendingPermission | null {
  const pending = new Map<string, PendingPermission>();
  for (const env of timeline) {
    if (env.kind === "permission_request") {
      pending.set(env.request_id, {
        requestId: env.request_id,
        title: titleOfToolCall(env.tool_call),
        options: env.options,
      });
    } else if (env.kind === "permission_resolved") {
      pending.delete(env.request_id);
    }
  }
  // Map preserves insertion order → the last entry is the most recent request.
  let last: PendingPermission | null = null;
  for (const p of pending.values()) last = p;
  return last;
}

/// Claude Code's auto-compaction notice: while it condenses history it streams a
/// standalone assistant message whose entire text is this literal, then continues
/// the turn under a fresh message. Shared with the Transcript's CompactingWidget.
export const COMPACTING_NOTICE = "Compacting...";

const COMPACTION_COMPLETION_NOTICES = new Set([
  "context compacted.",
  "context compacted to fit the model's context window.",
]);

function messageText(item: Extract<RenderItem, { kind: "message" }>): string | null {
  if (item.chunks.length === 0 || item.chunks.some((chunk) => chunk.type !== "text")) {
    return null;
  }
  return item.chunks
    .map((chunk) => chunk.type === "text" ? chunk.text : "")
    .join("")
    .trim();
}

export function isCompactionCommandText(text: string): boolean {
  const command = text.trim().toLowerCase();
  return command === "/compact" || command === "/compress" || command === "/summarize";
}

export function isCompactionCompletionText(text: string): boolean {
  // Codex ACP currently wraps this notification in Markdown emphasis. Older
  // replayed sessions and other providers can send the same notice as plain
  // text, so compare the semantic content rather than the decoration.
  const normalized = text.trim().replace(/^\*+|\*+$/g, "").trim().toLowerCase();
  return COMPACTION_COMPLETION_NOTICES.has(normalized);
}

export function latestCompactionCompletionSeq(timeline: Envelope[]): number {
  const items = derive(timeline);
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (item?.kind !== "message" || item.role !== "assistant") continue;
    const text = messageText(item);
    if (text && isCompactionCompletionText(text)) return Number(item.key);
  }
  return 0;
}

export function isCompactionCompletionTail(timeline: Envelope[]): boolean {
  const last = derive(timeline).at(-1);
  if (last?.kind !== "message" || last.role !== "assistant") return false;
  const text = messageText(last);
  return text !== null && isCompactionCompletionText(text);
}

/// True while a compaction is running RIGHT NOW — the `COMPACTING_NOTICE` message
/// is the live tail of the turn. Once the agent continues (a new item follows) or
/// the turn ends, the tail changes and this goes false. The composer uses it to
/// disable + spin its Compact button so you can't fire a second /compact mid-run
/// (mirrors the Transcript's active CompactingWidget). Pair with `busy` at the
/// call site — a persisted notice sitting as the last item of an IDLE, ended turn
/// is scrollback, not an in-flight compaction.
export function isCompactingTail(timeline: Envelope[]): boolean {
  const items = derive(timeline);
  const last = items[items.length - 1];
  if (last?.kind !== "message") return false;
  const text = messageText(last);
  if (text === null) return false;
  return last.role === "assistant"
    ? text === COMPACTING_NOTICE
    : isCompactionCommandText(text);
}

function titleOfToolCall(tc: unknown): string {
  if (tc && typeof tc === "object" && "title" in tc) {
    const t = (tc as { title?: unknown }).title;
    if (typeof t === "string") return t;
  }
  return "Permission requested";
}
