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
  | { kind: "thought"; text: string }
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
);

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

function pushChunk(item: { chunks: ContentChunk[] }, chunk: ContentChunk): void {
  const last = item.chunks[item.chunks.length - 1];
  if (last && last.type === "text" && chunk.type === "text") {
    last.text += chunk.text;
  } else {
    item.chunks.push(chunk);
  }
}

export function derive(timeline: Envelope[]): RenderItem[] {
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
              last.text += text;
            } else {
              items.push({ kind: "thought", text, key: String(env.seq) });
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
              rawInput: u["rawInput"] ?? u["input"],
              content: u["content"],
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
              if (u["rawInput"] !== undefined) existing.rawInput = u["rawInput"];
              if (u["content"] !== undefined) existing.content = u["content"];
            }
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
          items.push({ kind: "lifecycle", status: env.status, detail: env.detail, key: String(env.seq) });
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
  return items.filter((it) => it.kind !== "thought" || it.text.trim() !== "");
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
    key: entries.map((e) => e.content).join(" "),
    supersededByUserTurn: superseded,
  };
}

function titleOfToolCall(tc: unknown): string {
  if (tc && typeof tc === "object" && "title" in tc) {
    const t = (tc as { title?: unknown }).title;
    if (typeof t === "string") return t;
  }
  return "Permission requested";
}
