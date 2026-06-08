// Wire protocol — mirrors src/core.rs (Outbound/Envelope/Event) and the
// inbound commands parsed in src/server.rs. The ACP pass-through `update`
// payloads are typed loosely; their discriminant is `sessionUpdate`.

export type Status =
  | "starting"
  | "running"
  | "busy"
  | "exited"
  | "crashed"
  // Restored from a daemon restart that happened mid-turn — the last turn never
  // finished (src/core.rs Hub::restore). A dead/resumable state like exited.
  | "interrupted";

// Which surface opened the session — for the sidebar badge.
// Mirrors src/core.rs `SessionOrigin`. Older daemons that predate this field
// will omit it; treat absent as "api".
export type SessionOrigin = "api" | "web" | "zed";

// Human label for the surface that opened a session. Absent origin (older
// daemons) reads as "API", matching the daemon's `SessionOrigin` default.
export function originLabel(o: SessionOrigin | undefined): string {
  switch (o ?? "api") {
    case "zed":
      return "Zed";
    case "web":
      return "Web";
    default:
      return "API";
  }
}

export interface SessionMeta {
  id: string;
  provider: string;
  cwd: string;
  title: string;
  status: Status;
  origin?: SessionOrigin;
  /** Per-session auto-resume OVERRIDE: `null`/absent = inherit the global
   *  default (`settings['session.autoResume.default']`); `true`/`false` = force.
   *  Effective = override ?? default (see store `effectiveAutoResume`). */
  auto_resume?: boolean | null;
}

// A serialized ACP SessionUpdate. Internally tagged on `sessionUpdate`.
export interface AcpUpdate {
  sessionUpdate: string;
  content?: ContentBlock;
  // tool_call / tool_call_update
  toolCallId?: string;
  title?: string;
  kind?: string;
  status?: string;
  // plan
  entries?: PlanEntry[];
  // available_commands_update
  availableCommands?: AvailableCommand[];
  // current_mode_update
  currentModeId?: string;
  [key: string]: unknown;
}

export interface ContentBlock {
  type: string;
  text?: string;
  [key: string]: unknown;
}

export interface PlanEntry {
  content: string;
  priority?: string;
  status?: string;
}

export interface AvailableCommand {
  name: string;
  description: string;
}

export interface PermissionOption {
  optionId: string;
  name: string;
  kind: string;
}

// Event = the `kind`-tagged part of an Envelope.
export type Event =
  | { kind: "update"; update: AcpUpdate }
  | {
      kind: "permission_request";
      request_id: string;
      tool_call: unknown;
      options: PermissionOption[];
    }
  | { kind: "permission_resolved"; request_id: string; option_id: string | null }
  | { kind: "lifecycle"; status: Status; detail: string | null }
  | { kind: "turn_end"; stop_reason: string };

export type Envelope = { session_id: string; seq: number; cmid?: string } & Event;

// A single ACP config option the agent advertises for a session. Shape is
// stable across mode / model / effort because claude-agent-acp routes them
// all through one configOptions array. `currentValue` and option values are
// usually strings, but the protocol allows booleans too — leave wide.
export interface ConfigOption {
  id: string;
  name: string;
  description?: string;
  category?: string;
  type?: string;
  currentValue: string | boolean;
  options: {
    value: string | boolean;
    name: string;
    description?: string;
  }[];
}

// A staged message on the wire — a queued prompt or a draft. `content` is the
// ACP content-block array (empty for plain text); `text` is kept for display /
// re-edit. Mirrors src/core.rs `QueuedMessage`. The store converts this to/from
// the UI shape (which carries reconstructed `Attachment`s instead of raw blocks).
export interface WireQueued {
  id: string;
  text: string;
  content: ContentBlock[];
  /** Client message id, round-tripped so the originating client can reconcile
   *  its optimistic row by id (never text). Absent for bridge/API-staged items. */
  cmid?: string;
}

export type Outbound =
  | { type: "sessions"; sessions: SessionMeta[] }
  // The RECENT log tail (last SNAPSHOT_TAIL events). `reached_start` = these are
  // the whole log (nothing older to page to). Older history is fetched on demand
  // over HTTP — see loadOlder + GET /api/history/:id/:page.
  | { type: "snapshot"; session_id: string; events: Envelope[]; reached_start: boolean }
  | { type: "event"; envelope: Envelope }
  | { type: "config_options"; session_id: string; options: ConfigOption[] }
  // Generic optimistic-sync snapshot patch (@shared-utils/sync): the absolute
  // `value` of one `state` ("title" map / "order" array / "queue:<sid>" object)
  // at `version`, plus the mutation ids newly confirmed. `resync` = a
  // connect/reconnect snapshot the client adopts as ground truth regardless of
  // version (the daemon's clock resets on restart). The store folds it into that
  // state's sync client. See store.ts `sync_patch`. (Queue + drafts flow here as
  // state "queue:<session_id>" — no dedicated `queues` message anymore.)
  | { type: "sync_patch"; state: string; version: number; value: unknown; confirmed: string[]; resync?: boolean }
  // Global key-value settings (auto-resume default flag + continuation template).
  // Sent on connect + re-broadcast on every edit. See store.ts `settings`.
  | { type: "settings"; settings: Record<string, unknown> }
  | { type: "error"; session_id?: string; message: string };

export type Inbound =
  | { type: "new_session"; provider: string; cwd?: string }
  | {
      type: "prompt";
      session_id: string;
      text?: string;
      content?: ContentBlock[];
    }
  | { type: "cancel"; session_id: string }
  | {
      type: "permission";
      session_id: string;
      request_id: string;
      option_id?: string;
    }
  | { type: "delete_session"; session_id: string }
  | { type: "rename_session"; session_id: string; title: string }
  // Generic optimistic-sync mutation (@shared-utils/sync): `state` selects the
  // synced value ("title"/"order"/…), `name`+`args` are the mutator + args, `id`
  // is the client-minted mutation id (idempotent retry). The daemon arbiter
  // echoes a `sync_patch`. Supersedes rename_session/reorder_sessions for the web.
  | { type: "sync"; state: string; id: string; name: string; args: unknown }
  | {
      // Mode / model / effort change — same wire shape, server routes the
      // right ext_method downstream. See src/acp.rs SetConfigOption.
      type: "set_config_option";
      session_id: string;
      config_id: string;
      value: string | boolean;
    }
  // Client opened/selected a session — revive its agent if it died with a
  // daemon restart, without sending a turn. Idempotent. See src/core.rs.
  | { type: "open_session"; session_id: string }
  // --- Server-authoritative queue + drafts (synced across terminals) --------
  // The Web UI sends these; the daemon owns the per-session queue/drafts and the
  // drain, so every terminal sees identical state. (The bridge keeps using
  // `prompt` for a direct, un-queued dispatch.) `content` is the ACP block array
  // (empty ⇒ plain text in `text`). See src/core.rs Inbound.
  // `force` = long-press send: jump to the front of the queue + interrupt the
  // running turn so it runs next (no-op on an idle session).
  | { type: "submit"; session_id: string; text?: string; content?: ContentBlock[]; cmid?: string; force?: boolean }
  | { type: "remove_queued"; session_id: string; id: string }
  | {
      type: "edit_queued";
      session_id: string;
      id: string;
      text?: string;
      content?: ContentBlock[];
    }
  | { type: "clear_queue"; session_id: string }
  | { type: "request_send_queued"; session_id: string; id: string }
  | { type: "force_push_queued"; session_id: string; id: string }
  | { type: "queued_to_draft"; session_id: string; id: string }
  | { type: "set_queue_editing"; session_id: string; id: string | null }
  | { type: "add_draft"; session_id: string; text?: string; content?: ContentBlock[]; cmid?: string }
  | {
      type: "edit_draft";
      session_id: string;
      id: string;
      text?: string;
      content?: ContentBlock[];
    }
  | { type: "remove_draft"; session_id: string; id: string }
  | { type: "clear_drafts"; session_id: string }
  | { type: "activate_draft"; session_id: string; id: string }
  | { type: "activate_all_drafts"; session_id: string }
  // Move a draft to another session's drafts (the "wrong session" fix).
  | { type: "move_draft"; session_id: string; id: string; to_session: string }
  // Drag-to-arrange (server-authoritative, synced). `order` is the full list of
  // ids in the new order; omitted ids keep their relative order at the end.
  | { type: "reorder_sessions"; order: string[] }
  | { type: "reorder_queue"; session_id: string; order: string[] }
  | { type: "reorder_drafts"; session_id: string; order: string[] }
  // Auto-resume (tasks/active/session-auto-resume): per-session override
  // (`value: null` = inherit the global default) + a global setting upsert.
  | { type: "set_session_auto_resume"; session_id: string; value: boolean | null }
  | { type: "set_setting"; key: string; value: unknown };

export const PROVIDERS = ["claude-code", "codex"] as const;
