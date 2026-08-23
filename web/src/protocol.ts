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

// Which surface opened the session — for the sidebar badge. The wire values are
// legacy implementation names: "web" also covers the PWA/native shell, while
// "api" covers ACP clients such as Zed as well as direct API callers.
// Mirrors src/core.rs `SessionOrigin`. Older daemons that predate this field
// will omit it; treat absent as "api". (Legacy "zed" rows from the retired
// bridge now read back as "api".)
export type SessionOrigin = "api" | "web";

// User-facing source names describe the actual ownership boundary rather than
// the transport used internally. Absent origin (older daemons) is external,
// matching the daemon's `SessionOrigin` default.
export function originLabel(o: SessionOrigin | undefined): string {
  switch (o ?? "api") {
    case "web":
      return "Cowboy";
    default:
      return "External";
  }
}

export interface SessionMeta {
  id: string;
  provider: string;
  /** Exact immutable Provider generation selected for this session. */
  provider_version?: string;
  provider_generation_digest?: string;
  provider_auth_generation?: number;
  /** Stable machine placement. Missing on older daemons means local. */
  machine_id?: string;
  /** Stable selected workspace identity; cwd may point at its isolated worktree. */
  workspace_id?: string;
  workspace_name?: string;
  workspace_source_path?: string;
  cwd: string;
  title: string;
  status: Status;
  origin?: SessionOrigin;
  /** True for a machine-driven, view-only system session: the UI hides its
   *  composer and shows a "System" badge. Persisted. */
  system?: boolean;
  /** User-set MANUAL PAUSE of the queue drain (the ⏸ toggle). While true queued
   *  messages don't auto-advance (even after the turn ends), but the running turn
   *  isn't interrupted. Released by the user to resume. Transient, never
   *  persisted (resets to false on a daemon restart). */
  paused?: boolean;
  /** Context-window usage the agent reports over ACP `usage_update`:
   *  `context_used` tokens of a `context_size`-token window (drives the composer's
   *  "context X% full" ring). `0`/`0` (or absent) = not reported yet. Transient. */
  context_used?: number;
  context_size?: number;
  /** Full latest ACP usage update. `raw` retains optional standard cost and
   * provider `_meta` rate-limit fields for the Info → Usage panel. */
  usage?: {
    used: number;
    size: number;
    raw: Record<string, unknown> | null;
    observed_at_ms: number;
  };
  /** Soonest fire time (epoch ms) across this session's SCHEDULED DRAFTS, or
   *  absent if none. Drives the session-row clock badge. Transient. */
  next_schedule_ms?: number;
  /** Product account that created this session. Absent is the pre-auth shared
   *  pool (legacy rows and unauthenticated creates). */
  owner_user_id?: string;
  /** Display username for `owner_user_id`. Absent when the session is unowned
   *  or the username is not yet joined. */
  owner_username?: string;
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

export function isPureTerminalOutputDelta(update: AcpUpdate): boolean {
  if (update.sessionUpdate !== "tool_call_update") return false;
  if (
    !Object.keys(update).every((key) =>
      key === "sessionUpdate" || key === "toolCallId" || key === "_meta"
    )
  ) return false;
  const meta = update._meta;
  if (typeof meta !== "object" || meta === null || Array.isArray(meta)) return false;
  if (!Object.keys(meta).every((key) => key === "terminal_output_delta")) return false;
  const terminal = (meta as Record<string, unknown>).terminal_output_delta;
  return typeof terminal === "object" && terminal !== null && !Array.isArray(terminal) &&
    typeof (terminal as Record<string, unknown>).data === "string";
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
  /** Present only on a DRAFT with a future fire time — the server auto-activates
   *  it then. Absent on plain drafts / queued items. Mirrors src/core.rs. */
  schedule?: DraftSchedule;
}

/** Where a fired scheduled draft lands in the send-queue. `back` = tail (runs
 *  after everything queued; default); `front` = head (runs before other queued
 *  prompts). BOTH wait for a running turn to finish and BOTH respect a paused
 *  queue — neither bypasses the ⏸ hold. */
export type Delivery = "back" | "front";

/** A draft's future auto-send instruction. Mirrors src/core.rs `DraftSchedule`. */
export interface DraftSchedule {
  /** Absolute epoch-ms fire time. */
  fire_at_ms: number;
  delivery: Delivery;
}

export type Outbound =
  | { type: "sessions"; sessions: SessionMeta[] }
  // End of the deterministic connect snapshot. Browser clients need no action;
  // stdio bridges use it to avoid racing session/list against bootstrap.
  | { type: "bootstrap_complete" }
  // App-level heartbeat (see src/core.rs Outbound::Ping). Carries nothing; its
  // ARRIVAL is the signal — the client tracks the last-message time to detect a
  // half-open socket that never fires `onclose` and reconnect.
  | { type: "ping" }
  // The RECENT log tail (last SNAPSHOT_TAIL events). `reached_start` = these are
  // the whole log (nothing older to page to). Older history is fetched on demand
  // over HTTP — see loadOlder + GET /api/history/:id?before_seq=….
  | { type: "snapshot"; session_id: string; events: Envelope[]; reached_start: boolean }
  | { type: "event"; envelope: Envelope }
  | { type: "config_options"; session_id: string; options: ConfigOption[] }
  // Generic optimistic-sync snapshot patch (state-sync): the absolute
  // `value` of one `state` ("title" map / "order" array / "queue:<sid>" object)
  // at `version`, plus the mutation ids newly confirmed. `resync` = a
  // connect/reconnect snapshot the client adopts as ground truth regardless of
  // version (the daemon's clock resets on restart). The store folds it into that
  // state's sync client. See store.ts `sync_patch`. (Queue + drafts flow here as
  // state "queue:<session_id>" — no dedicated `queues` message anymore.)
  | { type: "sync_patch"; state: string; version: number; value: unknown; confirmed: string[]; resync?: boolean }
  // Compatibility tombstone for clients cached before automatic resume was
  // retired. Current clients ignore the empty snapshot.
  | { type: "settings"; settings: Record<string, unknown> }
  | { type: "error"; session_id?: string; message: string };

/** Focused-session hydration returned by the HTTP bootstrap route. Every item
 * uses the normal Outbound reducer so HTTP and live WebSocket overlap dedupes. */
export interface SessionBootstrapResponse {
  messages: Outbound[];
}

export type Inbound =
  | { type: "new_session"; provider: string; cwd?: string }
  | {
      type: "prompt";
      session_id: string;
      text?: string;
      content?: ContentBlock[];
    }
  | { type: "cancel"; session_id: string }
  // ACP bridge-only: remove exactly one queued prompt by its correlation id.
  | { type: "cancel_submitted"; session_id: string; cmid: string }
  | {
      type: "permission";
      session_id: string;
      request_id: string;
      option_id?: string;
    }
  | { type: "delete_session"; session_id: string }
  | { type: "rename_session"; session_id: string; title: string }
  // Generic optimistic-sync mutation (state-sync): `state` selects the
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
  // "Clear conversation": the daemon respawns the agent with a fresh session/new
  // (dropping its context) and drops a `context_cleared` timeline marker. Over
  // ACP, clearing is the client's job — no agent exposes a `clear` command — so
  // this is a session reset, not a slash command. See src/core.rs Inbound.
  | { type: "reset_session"; session_id: string }
  // --- Server-authoritative queue + drafts (synced across terminals) --------
  // The Web UI sends these; the daemon owns the per-session queue/drafts and the
  // drain, so every terminal sees identical state. (The bridge keeps using
  // `prompt` for a direct, un-queued dispatch.) `content` is the ACP block array
  // (empty ⇒ plain text in `text`). See src/core.rs Inbound.
  // `force` = long-press send: jump to the front of the queue + interrupt the
  // running turn so it runs next (no-op on an idle session). `front` = jump to
  // the front of the queue WITHOUT interrupting (runs next after the current
  // turn, ahead of the rest of the queue); no-op on an idle/empty-queue session.
  | {
    type: "submit";
    session_id: string;
    text?: string;
    content?: ContentBlock[];
    cmid?: string;
    force?: boolean;
    front?: boolean;
  }
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
  // Attach/replace a future fire time on a draft (creates it if id/cmid match
  // nothing). The server auto-activates it at fire_at_ms — fires even offline.
  | {
      type: "schedule_draft";
      session_id: string;
      id?: string;
      cmid?: string;
      text?: string;
      content?: ContentBlock[];
      fire_at_ms: number;
      delivery?: Delivery;
    }
  // Strip the schedule off a draft (it stays a plain parked draft).
  | { type: "unschedule_draft"; session_id: string; id: string }
  // Move a draft to another session's drafts (the "wrong session" fix).
  | { type: "move_draft"; session_id: string; id: string; to_session: string }
  // Drag-to-arrange (server-authoritative, synced). `order` is the full list of
  // ids in the new order; omitted ids keep their relative order at the end.
  | { type: "reorder_sessions"; order: string[] }
  | { type: "reorder_queue"; session_id: string; order: string[] }
  | { type: "reorder_drafts"; session_id: string; order: string[] }
  | { type: "set_paused"; session_id: string; paused: boolean }
  // Overlay action for errored turns.
  | { type: "retry_turn"; session_id: string }
  // Deprecated rollout tombstones. Current clients never send these; current
  // controllers accept them as no-ops for stale service-worker clients.
  | {
    type: "set_session_auto_resume";
    session_id: string;
    value: boolean | null;
  }
  | { type: "resume_turn"; session_id: string }
  | { type: "set_setting"; key: string; value: unknown };

// End of Inbound wire union.
