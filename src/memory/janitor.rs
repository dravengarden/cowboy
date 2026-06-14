//! The in-process memory janitor: a faithful Rust fold of mnemosyne's
//! waker+janitor, but reading the judge's reply IN-PROCESS instead of over MCP.
//!
//! The janitor is a cowboy `system` session (a coding-agent CLI). On each
//! coalesced batch (or on the tidy timer) cowboy:
//!   1. builds a reconcile/tidy prompt rendering the candidates + the top
//!      similar existing memories (so the judge can dedup);
//!   2. wakes the session via `supervisor.send(.., AgentCommand::Prompt(..))`;
//!   3. waits for the turn to finish by polling `hub.snapshot`, then reads the
//!      assistant reply IN-PROCESS (no MCP, no tools, no admin socket);
//!   4. parses a fenced ```json array of resolve-ops out of the reply;
//!   5. materializes them via `apply::resolve` (one git commit), then rebuilds
//!      the keyword index.
//!
//! Parse failure → re-prompt ONCE ("emit ONLY the json block"), then give up
//! (the batch's effect is left unwritten and logged). The driver NEVER panics.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Deserialize;

use crate::core::{Envelope, Event, Hub, Status};
use crate::supervisor::Supervisor;

use super::apply::{self, ResolveOp};
use super::index::Index;
use super::queue::Mutation;
use super::store::{Memory, MemoryType, Store, Tier};
use super::tier::project_tier;

/// How long to wait for the janitor's turn to finish before giving up on a
/// batch (it still leaves the batch unwritten + logs). mnemosyne's HTTP waker
/// used a 30s POST timeout, but that only covered the POST; here we wait for the
/// whole turn (the model actually thinking + replying), so it is generous.
const TURN_TIMEOUT: Duration = Duration::from_secs(180);

/// Poll cadence for the turn-end watch.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Everything the janitor loop needs to drive one reconcile/tidy pass. Cheap to
/// clone (all handles are `Arc`/clone-cheap), so the spawned tasks each hold one.
#[derive(Clone)]
pub struct Janitor {
    pub hub: Hub,
    pub supervisor: Arc<Supervisor>,
    pub store: Store,
    pub session_id: String,
    /// The recall/dedup index. Shared so reconcile passes see each other's
    /// writes; rebuilt after every `apply::resolve` that changed the store.
    pub index: Arc<Mutex<Index>>,
}

/// The tier a mutation's slug routes to (machine for the empty slug). Mirrors
/// `apply`'s private `tier_from_slug` (kept local to avoid widening that API).
fn tier_for_slug(slug: &str) -> Tier {
    if slug.is_empty() {
        Tier::machine()
    } else {
        project_tier(slug)
    }
}

/// One rendered candidate line block (op/tier/name/type/description/body), as
/// mnemosyne's `waker.Wake` renders it, PLUS the top similar existing memories
/// for that candidate so the judge can dedup without any tool call.
fn render_candidate(i: usize, m: &Mutation, index: &Index) -> String {
    let mut sb = String::new();
    let tier = tier_for_slug(&m.slug);
    sb.push_str(&format!(
        "{}. op={} tier={} name={:?} type={}\n   description: {}\n",
        i + 1,
        m.op.as_str(),
        tier,
        m.memory.name,
        m.memory.mem_type,
        m.memory.description,
    ));
    let body = m.memory.body.trim();
    if !body.is_empty() {
        sb.push_str(&format!("   body: {body}\n"));
    }
    // Dedup pre-filter: the single best overlap + the top-5 description matches.
    // The judge uses these to decide write-new vs merge/archive/move — no tool.
    if let Some(best) = index.best_duplicate(&m.memory) {
        sb.push_str(&format!(
            "   closest existing: name={:?} tier={} (score {}) — {}\n",
            best.name, best.tier, best.score, best.description,
        ));
    }
    let similar = index.query(&m.memory.description, 5);
    if !similar.is_empty() {
        sb.push_str("   similar existing:\n");
        for h in &similar {
            sb.push_str(&format!(
                "     - name={:?} tier={} (score {}) — {}\n",
                h.name, h.tier, h.score, h.description,
            ));
        }
    }
    sb.push('\n');
    sb
}

/// The strict OUTPUT contract appended to both the reconcile and tidy prompts.
/// The judge MUST reply with ONLY a fenced json block; this is what cowboy
/// parses in-process.
const OUTPUT_INSTRUCTION: &str = "\
Reply with ONLY a fenced ```json block: an array of resolve ops, each \
{\"kind\": \"write\"|\"archive\"|\"move\", \"tier\": \"...\", \"from\": \"...\", \
\"name\": \"...\", \"memory\": {\"name\",\"description\",\"type\",\"body\"}} — \
kind=write needs tier+memory; archive needs from+name; move needs from+tier+name. \
An empty array DISCARDS all candidates — use only for pure noise. A `tier`/`from` is a store-relative tier string: \
\"machine\", \"archive\", or \"projects/<slug>/memory\". `type` is one of \
user|feedback|project|reference. Emit NOTHING outside the ```json fence.";

/// Build the reconcile prompt for a coalesced batch: an instruction header, each
/// rendered candidate (+ its similar existing memories), then the strict output
/// contract.
#[must_use]
pub fn build_reconcile_prompt(batch: &[Mutation], index: &Index) -> String {
    let mut sb = String::new();
    sb.push_str(&format!(
        "You are the memory janitor. Below are {} memory CANDIDATE(s) an agent \
         proposed. They are NOT in the store yet — a candidate exists ONLY if \
         you materialize it with a `write` op. For EACH candidate you MUST emit \
         exactly one op:\n\
         - No existing memory covers it (the COMMON case) → emit `kind=write` \
         with its tier + memory verbatim. This is the default; do it.\n\
         - It overlaps an existing memory (see `closest`/`similar` below) → emit \
         `kind=write` to the merged/updated memory, reusing the EXISTING name + \
         tier; add a `kind=archive` for any now-redundant duplicate.\n\
         - It belongs in a different tier than proposed → `kind=write` to the \
         right tier (and `archive`/`move` the misplaced one if it already exists).\n\
         Emitting `[]` DISCARDS every candidate — do that ONLY if every candidate \
         is pure noise with no lasting value. When in doubt, WRITE. Do NOT use \
         any tools; decide solely from the information below. Do NOT propose new \
         candidates (that re-enqueues).\n\nCandidates:\n\n",
        batch.len()
    ));
    for (i, m) in batch.iter().enumerate() {
        sb.push_str(&render_candidate(i, m, index));
    }
    sb.push_str(OUTPUT_INSTRUCTION);
    sb
}

/// Build the tidy prompt: a conservative scheduled maintenance pass (no
/// candidates). Mirrors mnemosyne's `waker.Tidy`, plus the strict output
/// contract so the reply is machine-readable in-process.
#[must_use]
pub fn build_tidy_prompt(index: &Index) -> String {
    let mut sb = String::new();
    sb.push_str(
        "Run a scheduled tidy pass over the memory store: survey the existing \
         memories below, rotate/condense episodic notes, and soft-archive \
         clearly-stale memories (emit a `kind=archive` op — never hard-delete). \
         Conservative — when unsure, leave it. Do NOT propose new \
         candidates.\n\nExisting memories (a sample):\n\n",
    );
    // Surface a sample of what exists so the judge has something to survey. The
    // empty query returns nothing from the keyword index, so list the entries.
    for h in index.query("memory the and a project user feedback reference", 0).iter().take(40) {
        sb.push_str(&format!(
            "- name={:?} tier={} — {}\n",
            h.name, h.tier, h.description
        ));
    }
    sb.push('\n');
    sb.push_str(OUTPUT_INSTRUCTION);
    sb
}

// ---- the reply DTO (judge JSON → apply::ResolveOp) --------------------------

/// A single resolve op as the janitor emits it in its ```json reply. This is the
/// wire shape; `into_resolve_op` converts it to the typed `apply::ResolveOp`.
#[derive(Debug, Deserialize)]
struct ResolveOpDto {
    kind: String,
    #[serde(default)]
    tier: Option<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    memory: Option<MemoryDto>,
}

/// The memory payload of a `write` op.
#[derive(Debug, Deserialize)]
struct MemoryDto {
    name: String,
    description: String,
    #[serde(rename = "type")]
    mem_type: String,
    #[serde(default)]
    body: String,
}

impl MemoryDto {
    fn into_memory(self) -> Result<Memory, String> {
        let mem_type = MemoryType::parse(&self.mem_type)
            .ok_or_else(|| format!("unknown memory.type {:?}", self.mem_type))?;
        Ok(Memory {
            name: self.name,
            description: self.description,
            mem_type,
            body: self.body,
        })
    }
}

impl ResolveOpDto {
    fn into_resolve_op(self) -> Result<ResolveOp, String> {
        match self.kind.as_str() {
            "write" => {
                let tier = self
                    .tier
                    .ok_or("write op missing `tier`")?;
                let memory = self
                    .memory
                    .ok_or("write op missing `memory`")?
                    .into_memory()?;
                Ok(ResolveOp::write(Tier(tier), memory))
            }
            "archive" => {
                let from = self.from.ok_or("archive op missing `from`")?;
                let name = self.name.ok_or("archive op missing `name`")?;
                Ok(ResolveOp::archive(Tier(from), name))
            }
            "move" => {
                let from = self.from.ok_or("move op missing `from`")?;
                let tier = self.tier.ok_or("move op missing `tier`")?;
                let name = self.name.ok_or("move op missing `name`")?;
                Ok(ResolveOp::move_op(Tier(from), Tier(tier), name))
            }
            other => Err(format!("unknown resolve kind {other:?}")),
        }
    }
}

/// Extract the FIRST fenced ```json block from a reply and parse it into typed
/// resolve-ops. Tolerates a bare ``` fence (no `json` lang tag) and leading
/// prose. Returns a human error on a missing fence / bad JSON / bad op shape.
///
/// # Errors
/// When no fenced block is found, the JSON is malformed, or an op is invalid.
pub fn parse_resolve_ops(reply: &str) -> Result<Vec<ResolveOp>, String> {
    let json = extract_json_block(reply).ok_or("no ```json block in reply")?;
    let dtos: Vec<ResolveOpDto> =
        serde_json::from_str(&json).map_err(|e| format!("json parse: {e}"))?;
    let mut ops = Vec::with_capacity(dtos.len());
    for d in dtos {
        ops.push(d.into_resolve_op()?);
    }
    Ok(ops)
}

/// Find the inner text of the first fenced code block. Prefers a ```json fence
/// but accepts a plain ``` fence too (the judge may drop the lang tag).
fn extract_json_block(reply: &str) -> Option<String> {
    // Scan for an opening fence, capture to the next closing ```.
    let bytes_search = |needle: &str| -> Option<usize> { reply.find(needle) };
    let open = bytes_search("```json").map(|i| (i, "```json".len())).or_else(|| {
        // Plain fence: only accept it if the body actually looks like a JSON
        // array (starts with `[` after trimming), so we don't grab a random
        // code block.
        bytes_search("```").map(|i| (i, "```".len()))
    })?;
    let after = &reply[open.0 + open.1..];
    // Skip an optional trailing language tag / newline right after the fence.
    let after = after.strip_prefix('\n').unwrap_or(after);
    let close = after.find("```")?;
    let inner = after[..close].trim();
    // Strip a leftover `json` lang line if the plain-fence branch grabbed one.
    let inner = inner.strip_prefix("json").map_or(inner, str::trim_start);
    if inner.trim_start().starts_with('[') {
        Some(inner.to_string())
    } else {
        None
    }
}

// ---- the driver -------------------------------------------------------------

impl Janitor {
    /// Reconcile one coalesced batch: build the prompt, wake the session, wait
    /// for the turn, read the reply, parse + apply the resolve-ops. On a parse
    /// failure, re-prompt ONCE with a stricter instruction, then give up
    /// (leaving the batch's effect unwritten + logged). Never panics.
    pub async fn run_janitor(&self, batch: Vec<Mutation>) {
        if batch.is_empty() {
            return;
        }
        let prompt = {
            let index = self.index.lock();
            build_reconcile_prompt(&batch, &index)
        };
        self.drive("reconcile", prompt).await;
    }

    /// A scheduled tidy pass (no candidates). Same machinery as `run_janitor`
    /// but with the conservative survey/soft-archive prompt.
    pub async fn tidy(&self) {
        let prompt = {
            let index = self.index.lock();
            build_tidy_prompt(&index)
        };
        self.drive("tidy", prompt).await;
    }

    /// Shared driver: send `prompt`, await the turn, read + parse the reply,
    /// apply the ops. `label` tags the logs (`reconcile`/`tidy`). Re-prompts
    /// ONCE on a parse failure.
    async fn drive(&self, label: &str, prompt: String) {
        match self.wake_and_read(prompt).await {
            Ok(reply) => match parse_resolve_ops(&reply) {
                Ok(ops) => self.apply_ops(label, ops),
                Err(e) => {
                    tracing::warn!(
                        label,
                        error = %e,
                        "janitor reply did not parse; re-prompting once"
                    );
                    self.reprompt_once(label).await;
                }
            },
            Err(e) => {
                tracing::warn!(label, error = %e, "janitor turn failed (batch left unwritten)");
            }
        }
    }

    /// The single stricter retry: ask for ONLY the json block, parse + apply, or
    /// give up.
    async fn reprompt_once(&self, label: &str) {
        let prompt = format!(
            "Your previous reply could not be parsed. Emit ONLY a fenced \
             ```json block — an array of resolve ops (possibly empty `[]`), and \
             NOTHING else.\n\n{OUTPUT_INSTRUCTION}"
        );
        match self.wake_and_read(prompt).await {
            Ok(reply) => match parse_resolve_ops(&reply) {
                Ok(ops) => self.apply_ops(label, ops),
                Err(e) => tracing::warn!(
                    label,
                    error = %e,
                    "janitor re-prompt still unparseable; giving up (unwritten)"
                ),
            },
            Err(e) => tracing::warn!(label, error = %e, "janitor re-prompt turn failed; giving up"),
        }
    }

    /// Apply parsed resolve-ops (one git commit), then rebuild the index. An
    /// empty op list is a clean no-op.
    fn apply_ops(&self, label: &str, ops: Vec<ResolveOp>) {
        if ops.is_empty() {
            tracing::info!(label, "janitor: empty op list (no change)");
            return;
        }
        let n = ops.len();
        match apply::resolve(&self.store, &ops) {
            Ok(commit) => {
                tracing::info!(label, ops = n, commit = ?commit, "janitor applied resolve ops");
                // Rebuild the recall/dedup index so the next pass sees the new
                // store state (the cheap, no-model rebuild — store list + tokenize).
                match Index::from_store(&self.store) {
                    Ok(ix) => *self.index.lock() = ix,
                    Err(e) => tracing::warn!(label, error = %e, "index rebuild after apply failed"),
                }
            }
            Err(e) => tracing::warn!(label, error = %e, "janitor apply::resolve failed"),
        }
    }

    /// Wake the janitor session with `prompt`, wait for the turn to finish, and
    /// return the assistant reply text read IN-PROCESS from the Hub.
    ///
    /// Turn-end detection: capture the session's max event seq BEFORE sending;
    /// after sending, poll `hub.snapshot` until a `TurnEnd` (or a terminal
    /// lifecycle) lands with a seq strictly greater than that mark, or the
    /// timeout elapses. The reply is the concatenation of the
    /// `agent_message_chunk` text in envelopes after the mark.
    async fn wake_and_read(&self, prompt: String) -> Result<String, String> {
        let mark = self.max_seq();
        let blocks = vec![agent_client_protocol::schema::ContentBlock::from(prompt)];
        self.supervisor
            .send(&self.session_id, crate::acp::AgentCommand::Prompt(blocks, None))
            .map_err(|e| format!("send prompt: {e}"))?;

        let deadline = tokio::time::Instant::now() + TURN_TIMEOUT;
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;
            let Some((log, _reached_start)) = self.hub.snapshot(&self.session_id) else {
                return Err("session vanished while waiting for turn".to_string());
            };
            // A TurnEnd after the mark = the turn we issued finished.
            let turn_ended = log
                .iter()
                .any(|e| e.seq > mark && matches!(e.event, Event::TurnEnd { .. }));
            // A terminal lifecycle (exited/crashed) also ends our wait — the
            // agent died; treat whatever it managed to emit as the reply.
            let terminal = log.iter().any(|e| {
                e.seq > mark
                    && matches!(
                        &e.event,
                        Event::Lifecycle {
                            status: Status::Exited | Status::Crashed,
                            ..
                        }
                    )
            });
            if turn_ended || terminal {
                let reply = reply_text_after(&log, mark);
                if reply.trim().is_empty() {
                    return Err("turn ended with an empty assistant reply".to_string());
                }
                return Ok(reply);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "turn did not finish within {}s",
                    TURN_TIMEOUT.as_secs()
                ));
            }
        }
    }

    /// The highest event seq currently in the session's snapshot, or 0 if the
    /// session has no events yet (or vanished — caller re-checks on poll).
    fn max_seq(&self) -> u64 {
        self.hub
            .snapshot(&self.session_id)
            .map(|(log, _)| log.iter().map(|e| e.seq).max().unwrap_or(0))
            .unwrap_or(0)
    }
}

/// Concatenate the `agent_message_chunk` text from envelopes with seq > `mark` —
/// the assistant reply produced by the turn we just issued. Walks the same
/// `Event::Update`/`sessionUpdate`/`content.text` shape as the Hub's private
/// `last_turn_texts`, scoped to events after the mark so a prior turn's output
/// is never mixed in.
fn reply_text_after(log: &[Envelope], mark: u64) -> String {
    let mut out = String::new();
    for env in log.iter().filter(|e| e.seq > mark) {
        let Event::Update { update } = &env.event else {
            continue;
        };
        let kind = update.get("sessionUpdate").and_then(serde_json::Value::as_str);
        if kind != Some("agent_message_chunk") {
            continue;
        }
        if let Some(text) = update
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(serde_json::Value::as_str)
        {
            out.push_str(text);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::apply::ResolveKind;

    #[test]
    fn parse_write_archive_move() {
        let reply = "Here are my decisions.\n\n```json\n[\n  {\"kind\": \"write\", \
            \"tier\": \"machine\", \"memory\": {\"name\": \"omega-deploy\", \
            \"description\": \"omega proxy deploy\", \"type\": \"project\", \
            \"body\": \"the body\"}},\n  {\"kind\": \"archive\", \"from\": \
            \"machine\", \"name\": \"stale-note\"},\n  {\"kind\": \"move\", \
            \"from\": \"projects/-home-draven-columbus/memory\", \"tier\": \
            \"machine\", \"name\": \"x\"}\n]\n```\nDone.";
        let ops = parse_resolve_ops(reply).expect("should parse");
        assert_eq!(ops.len(), 3);

        assert_eq!(ops[0].kind, ResolveKind::Write);
        assert_eq!(ops[0].tier.as_str(), "machine");
        assert_eq!(ops[0].memory.name, "omega-deploy");
        assert_eq!(ops[0].memory.mem_type, MemoryType::Project);
        assert_eq!(ops[0].memory.body, "the body");

        assert_eq!(ops[1].kind, ResolveKind::Archive);
        assert_eq!(ops[1].from.as_str(), "machine");
        assert_eq!(ops[1].name, "stale-note");

        assert_eq!(ops[2].kind, ResolveKind::Move);
        assert_eq!(ops[2].from.as_str(), "projects/-home-draven-columbus/memory");
        assert_eq!(ops[2].tier.as_str(), "machine");
        assert_eq!(ops[2].name, "x");
    }

    #[test]
    fn parse_empty_array() {
        let ops = parse_resolve_ops("```json\n[]\n```").expect("empty array parses");
        assert!(ops.is_empty());
    }

    #[test]
    fn parse_plain_fence_no_lang() {
        let ops = parse_resolve_ops("```\n[{\"kind\":\"archive\",\"from\":\"machine\",\"name\":\"a\"}]\n```")
            .expect("plain fence parses");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].kind, ResolveKind::Archive);
    }

    #[test]
    fn parse_missing_fence_errs() {
        assert!(parse_resolve_ops("no fence here, just prose").is_err());
    }

    #[test]
    fn parse_write_missing_memory_errs() {
        let r = "```json\n[{\"kind\":\"write\",\"tier\":\"machine\"}]\n```";
        assert!(parse_resolve_ops(r).is_err());
    }

    #[test]
    fn parse_unknown_kind_errs() {
        let r = "```json\n[{\"kind\":\"frobnicate\"}]\n```";
        assert!(parse_resolve_ops(r).is_err());
    }

    #[test]
    fn reply_text_scopes_to_mark() {
        let env = |seq: u64, kind: &str, text: &str| Envelope {
            session_id: "s".to_string(),
            seq,
            event: Event::Update {
                update: serde_json::json!({
                    "sessionUpdate": kind,
                    "content": {"type": "text", "text": text},
                }),
            },
            cmid: None,
        };
        let log = vec![
            env(1, "agent_message_chunk", "OLD turn output"),
            env(2, "user_message_chunk", "the prompt"),
            env(3, "agent_message_chunk", "new "),
            env(4, "agent_message_chunk", "reply"),
        ];
        assert_eq!(reply_text_after(&log, 2), "new reply");
    }
}
