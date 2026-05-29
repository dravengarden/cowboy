// Wire protocol — mirrors src/core.rs (Outbound/Envelope/Event) and the
// inbound commands parsed in src/server.rs. The ACP pass-through `update`
// payloads are typed loosely; their discriminant is `sessionUpdate`.

export type Status = "starting" | "running" | "busy" | "exited" | "crashed";

// Which surface opened the session — for the sidebar badge.
// Mirrors src/core.rs `SessionOrigin`. Older daemons that predate this field
// will omit it; treat absent as "api".
export type SessionOrigin = "api" | "web" | "zed";

export interface SessionMeta {
  id: string;
  provider: string;
  cwd: string;
  title: string;
  status: Status;
  origin?: SessionOrigin;
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

export type Envelope = { session_id: string; seq: number } & Event;

export type Outbound =
  | { type: "sessions"; sessions: SessionMeta[] }
  | { type: "snapshot"; session_id: string; events: Envelope[] }
  | { type: "event"; envelope: Envelope }
  | { type: "error"; message: string };

export type Inbound =
  | { type: "new_session"; provider: string; cwd?: string }
  | { type: "prompt"; session_id: string; text: string }
  | { type: "cancel"; session_id: string }
  | {
      type: "permission";
      session_id: string;
      request_id: string;
      option_id?: string;
    }
  | { type: "delete_session"; session_id: string };

export const PROVIDERS = ["claude-code", "codex"] as const;
