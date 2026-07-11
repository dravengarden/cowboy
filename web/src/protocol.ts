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
// will omit it; treat absent as "api". (Legacy "zed" rows from the retired
// bridge now read back as "api".)
export type SessionOrigin = "api" | "web";

// Human label for the surface that opened a session. Absent origin (older
// daemons) reads as "API", matching the daemon's `SessionOrigin` default.
export function originLabel(o: SessionOrigin | undefined): string {
  switch (o ?? "api") {
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
  /** True for the machine-driven VIEW-ONLY system session (the mnemosyne memory
   *  janitor): the UI hides its composer and shows a "System" badge. Persisted. */
  system?: boolean;
  /** Per-session auto-resume OVERRIDE: `null`/absent = inherit the global
   *  default (`settings['session.autoResume.default']`); `true`/`false` = force.
   *  Effective = override ?? default (see store `effectiveAutoResume`). */
  auto_resume?: boolean | null;
  /** True when the confirm-detect skill judged the agent's last turn as awaiting
   *  the user (a question/confirmation): the queue is held and the awaiting
   *  widget shows. Transient (never persisted) — resets to false on restart. */
  awaiting_user?: boolean;
  /** True when the last turn was judged as having COMPLETED the task (green "done"
   *  overlay). Transient, never persisted. */
  done?: boolean;
  /** True while the async confirm-detect L2 judge is in flight for the last turn
   *  (between the provisional hold and the verdict). Drives the pill's "Judging…"
   *  loading state. Transient, never persisted. */
  judging?: boolean;
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
  /** Soonest fire time (epoch ms) across this session's SCHEDULED DRAFTS, or
   *  absent if none. Drives the session-row clock badge. Transient. */
  next_schedule_ms?: number;
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
  // App-level heartbeat (see src/core.rs Outbound::Ping). Carries nothing; its
  // ARRIVAL is the signal — the client tracks the last-message time to detect a
  // half-open socket that never fires `onclose` and reconnect.
  | { type: "ping" }
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
  // Inference-provider configs (model + key_set — NEVER the key). Connect + edits.
  | { type: "inference_config"; providers: InferenceProviderView[] }
  // The static skill registry (prompt + extract), sent once on connect.
  | { type: "skills"; skills: SkillView[] }
  // The confirm-detect judge's full result for a turn (verdict + raw I/O for the
  // overlay's "raw data" expand). Latest-per-session.
  | ({ type: "judge_result" } & JudgeResult)
  // A session's confirm-detect judge-run HISTORY (newest first), capped. Backs the
  // inspector widget (long-press the turn-status pill). Sent per session on
  // connect + re-broadcast on every new run / per-item delete / clear.
  | { type: "judge_history"; session_id: string; runs: JudgeRun[] }
  // Result of a dev probe (Info sheet "Test"): text + cache token counts, or error.
  | {
    type: "inference_probe_result";
    provider: string;
    ok: boolean;
    text: string;
    cache_hit: number;
    cache_miss: number;
    error?: string;
  }
  | { type: "error"; session_id?: string; message: string };

/** One inference provider's config as the daemon exposes it — `key_set` says
 *  whether an API key exists, but the key itself never reaches the client. */
export interface InferenceProviderView {
  provider: string;
  model: string;
  params: unknown;
  key_set: boolean;
  /** Selectable models (id + human label) the daemon offers for this provider —
   *  the model dropdown renders from this, never hardcoding ids (Step 18). */
  models: { id: string; label: string }[];
}

/** A registered skill as the daemon exposes it — the prompt template + extraction
 *  rule are rendered verbatim in the Info sheet so they're inspectable. */
export interface SkillView {
  id: string;
  title: string;
  description: string;
  prompt_template: string;
  extract: string;
}

/** One confirm-detect judge run — the verdict + the observability detail the
 *  overlay's "raw data" expand surfaces. */
export interface JudgeResult {
  session_id: string;
  layer: string; // "L1" | "L2"
  awaiting_user: boolean;
  done: boolean;
  confidence: number;
  reason: string;
  model: string;
  input: string;
  output: string;
  cache_hit: number;
  cache_miss: number;
  latency_ms: number;
}

/** One persisted judge run — the `JudgeResult` detail (minus `session_id`, which
 *  rides on the `JudgeHistory` wrapper) plus the durable `id` (delete key) and
 *  `at` (unix-ms). Backs the inspector widget's history list. */
export interface JudgeRun extends Omit<JudgeResult, "session_id"> {
  id: string;
  at: number;
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
  // Inspector widget: delete one judge run from a session's history, or clear all.
  | { type: "remove_judge_run"; session_id: string; id: string }
  | { type: "clear_judge_runs"; session_id: string }
  // Auto-resume (tasks/active/session-auto-resume): per-session override
  // (`value: null` = inherit the global default) + a global setting upsert.
  | { type: "set_session_auto_resume"; session_id: string; value: boolean | null }
  // Confirm-detect: clear/set the "awaiting user" hold (the awaiting widget's
  // dismiss / Send). `awaiting: false` = "not a question" → drain the held queue.
  | { type: "set_awaiting"; session_id: string; awaiting: boolean }
  | { type: "set_paused"; session_id: string; paused: boolean }
  // Overlay actions for interrupted / errored turns.
  | { type: "resume_turn"; session_id: string }
  | { type: "retry_turn"; session_id: string }
  | { type: "set_setting"; key: string; value: unknown }
  // Inference provider config (model + params) + API key (separate; key never
  // echoed back). See store.ts `inferenceConfig`.
  | { type: "set_inference_config"; provider: string; model: string; params?: unknown }
  | { type: "set_inference_secret"; provider: string; api_key: string }
  | { type: "inference_probe"; provider: string; prompt?: string };

export const PROVIDERS = ["claude-code", "codex", "gemini"] as const;
