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

export type RenderItem =
  | { kind: "message"; role: "assistant" | "user"; chunks: ContentChunk[] }
  | { kind: "thought"; text: string }
  | {
      kind: "tool";
      id: string;
      title: string;
      toolKind: string;
      status: string;
      /// Raw input JSON (tool args), preserved for the expanded card.
      rawInput?: unknown;
      /// Tool result content (text / files / diff) as the upstream sent it.
      content?: unknown;
    }
  | { kind: "plan"; entries: PlanEntry[] }
  | {
      kind: "permission";
      requestId: string;
      title: string;
      options: PermissionOption[];
      resolved: boolean;
      chosen: string | null;
    }
  | { kind: "lifecycle"; status: Status; detail: string | null };

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
        source?: { type?: string; data?: string; url?: string; media_type?: string };
      };
  if (!c) return null;
  if (c.type === "text") {
    return { type: "text", text: c.text ?? "" };
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
  let planIndex = -1;
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
              items.push({ kind: "message", role, chunks: [chunk] });
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
              items.push({ kind: "thought", text });
              cursor = { kind: "thought", role: "assistant" };
            }
            break;
          }
          case "tool_call": {
            const id = u.toolCallId ?? "";
            items.push({
              kind: "tool",
              id,
              title: u.title ?? id,
              toolKind: u.kind ?? "other",
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
              if (u["rawInput"] !== undefined) existing.rawInput = u["rawInput"];
              if (u["content"] !== undefined) existing.content = u["content"];
            }
            break;
          }
          case "plan": {
            const entries = u.entries ?? [];
            if (planIndex >= 0) {
              const p = items[planIndex];
              if (p && p.kind === "plan") p.entries = entries;
            } else {
              items.push({ kind: "plan", entries });
              planIndex = items.length - 1;
            }
            break;
          }
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
          items.push({ kind: "lifecycle", status: env.status, detail: env.detail });
        }
        break;
      }
      case "turn_end":
        break;
    }
  }
  return items;
}

function titleOfToolCall(tc: unknown): string {
  if (tc && typeof tc === "object" && "title" in tc) {
    const t = (tc as { title?: unknown }).title;
    if (typeof t === "string") return t;
  }
  return "Permission requested";
}
