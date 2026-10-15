//! Agent API: Cowboy's own capabilities exposed to the agent of a session as
//! a token-frugal MCP surface (see `docs/agent-api.md`).
//!
//! The agent reaches it through the parent-scoped managed-call gateway that
//! every ordinary session already has (`COWBOY_CALL_CONTEXT`), so it adds no
//! credential: the Controller re-derives authority from the live parent worker
//! for every action. `cowboy mcp` (see [`mcp`]) is a stdio MCP server with no
//! background traffic; one `cowboy_read` program composes many reads inside a
//! contained runtime beside the Controller's state and returns only its result.

#[cfg(unix)]
pub mod mcp;

/// Model-facing API identity; bumped on any incompatible SDK change.
pub const API_VERSION: &str = "cowboy-ts-v1";
/// Program source, as Matrix CodeAct.
pub const MAX_SOURCE_BYTES: usize = 32 * 1024;
/// A program's wall-clock budget when it names none; at most `MAX_WAIT_MS`.
pub const DEFAULT_TIMEOUT_MS: u64 = 20_000;
/// Runtime startup headroom added to the gateway exchange.
pub const STARTUP_MARGIN_MS: u64 = 5_000;
/// The JSON a program returns to the model, and its console log.
pub const MAX_RESULT_BYTES: usize = 16 * 1024;
pub const MAX_LOG_BYTES: usize = 4 * 1024;
/// API calls one program may make.
pub const MAX_CALLS: usize = 16;
/// One API reply delivered into the program, and all broker traffic.
pub const MAX_REPLY_BYTES: usize = 256 * 1024;
pub const MAX_BROKER_BYTES: usize = 1024 * 1024;

/// Every operation a read program may call, in SDK order. The Controller's
/// dispatcher and [`SDK`] are checked against this list.
pub const READ_OPERATIONS: &[&str] = &[
    "session.info",
    "session.queue",
    "session.drafts",
    "session.events",
    "calls.capabilities",
    "calls.inspect",
    "calls.wait",
    "calls.result",
    "calls.observe",
    "sessions.list",
    "sessions.info",
    "documents.list",
    "documents.read",
];

/// The complete strict declaration, served as `cowboy://sdk/v1`.
pub const SDK: &str = r#"// Cowboy agent API cowboy-ts-v1. cowboy_read runs CODE as
// `async ({cowboy}) => { CODE }` (format "module": export default async function({cowboy})).
// Return JSON (<=16 KiB). Each await is one of <=16 API calls; no network or files.
// A failed call throws Error with .code (not_found, permission_denied, unavailable, ...).
export type Seq = number;
export type SessionView = {
  id: string; title: string; provider: string;
  status: "starting" | "running" | "busy" | "exited" | "crashed" | "interrupted" | string;
  machine: string; workspace: string | null; cwd: string; paused: boolean;
  queued: number; drafts: number; events: number; background_tasks: number;
  context: { used: number; size: number } | null;
};
export type Message = { id: string; text: string; attachments: number; scheduled_at_ms?: number };
export type EventKind = "user" | "agent" | "tool" | "plan" | "permission" | "turn_end" | "lifecycle";
export type EventQuery = {
  after?: Seq; before?: Seq;   // after: oldest first; otherwise the newest before `before`
  limit?: number;              // items, default 20, max 100
  chars?: number;              // text per item, default 400, max 4000
  kinds?: EventKind[];         // default all but "lifecycle"
};
export type EventItem = {
  seq: Seq; to: Seq; kind: EventKind; text?: string; truncated?: true;
  tool?: { id: string; title?: string; status?: string; kind?: string };
};
export type EventPage = { items: EventItem[]; reached_start: boolean; more: boolean };
export type CallView = {
  state: string; call_id: string; request_id: string;
  resolved: { provider: string; machine_id: string; workspace_id: string; child_session_id?: string | null };
  labels?: Record<string, string>; cancel_requested: boolean;
  error: { code: string } | null; result?: unknown;
};
export type CallCapabilities = {
  enabled: boolean; default: string;
  providers: { id: string; available: boolean; reason: string | null; presets: unknown[] }[];
  purposes: string[]; limits: Record<string, number>;
};
export type DocumentMeta = {
  id: string; kind: "document" | "folder"; title: string; parent_id: string | null;
  revision: number; body_revision: number; updated_at_ms: number;
};
export type Document = DocumentMeta & { body: string; attachments: number };
export interface Cowboy {
  session: {                       // this session
    info(): Promise<SessionView>;
    queue(): Promise<Message[]>;   // prompts waiting for the current turn
    drafts(): Promise<Message[]>;  // parked composer drafts
    events(q?: EventQuery): Promise<EventPage>; // coalesced transcript, retained window
  };
  calls: {                         // managed calls this session started
    capabilities(): Promise<CallCapabilities>;
    inspect(call_id: string): Promise<CallView>;
    wait(call_id: string, timeout_ms?: number): Promise<CallView>; // bounded by the program
    result(call_id: string): Promise<CallView>;
    observe(request_id: string): Promise<CallView>;
  };
  sessions: {                      // sessions with this session's owner
    list(q?: { status?: string; limit?: number }): Promise<SessionView[]>;
    info(id: string): Promise<SessionView>;
  };
  documents: {                     // the owner's Draft documents
    list(): Promise<DocumentMeta[]>;
    read(id: string): Promise<Document>;
  };
}
"#;

pub const STATUS_DESCRIPTION: &str = "This Cowboy session in one call: status, queue/draft counts, placement, and whether managed agent calls are enabled. No arguments.";

pub const READ_DESCRIPTION: &str = r#"Read Cowboy state with one TS/JS program: compose many reads beside Cowboy and return only what you need. CODE is the body of `async ({cowboy}) => { CODE }`; return JSON (<=16 KiB). Limits: 32 KiB source, 16 API calls, timeout_ms default 20000 (max 60000), no network or files. Full types: resource cowboy://sdk/v1.
cowboy.session.{info(),queue(),drafts(),events({after?,before?,limit?,chars?,kinds?})}
cowboy.calls.{capabilities(),inspect(id),wait(id,ms?),result(id),observe(request_id)}
cowboy.sessions.{list({status?,limit?}),info(id)} cowboy.documents.{list(),read(id)}
Errors throw with .code. Example: const v=await cowboy.calls.wait(id,50000); return v.state==="completed"?(await cowboy.calls.result(id)).result:v.state;"#;

/// Short server instructions, sent once at MCP initialization.
pub const INSTRUCTIONS: &str = "Cowboy session tools. Prefer one cowboy_read program over several calls; cowboy_status needs no program.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operation_is_declared_in_the_sdk_and_the_description() {
        for operation in READ_OPERATIONS {
            let (namespace, method) = operation.split_once('.').unwrap();
            assert!(SDK.contains(&format!("{namespace}: {{")), "{namespace}");
            assert!(SDK.contains(&format!("    {method}(")), "{operation}");
            assert!(READ_DESCRIPTION.contains(method), "{operation}");
        }
    }

    #[test]
    fn the_resident_surface_stays_small() {
        // Tool descriptions are paid on every turn; the SDK only when read.
        assert!(READ_DESCRIPTION.len() + STATUS_DESCRIPTION.len() + INSTRUCTIONS.len() < 1500);
        assert!(SDK.len() < 6000);
    }
}
