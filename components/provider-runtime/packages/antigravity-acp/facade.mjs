// The Cowboy tool facade of an Antigravity account unit (design "Phase 4:
// split execution" and amendments R1-5, R1-10, R1-16, R2-16, R3-17).
//
// `agy` runs on OVH; every workspace tool runs on the session's development
// Machine through the one execution `Connection` the proxy owns. Paths are
// canonicalized on the target; reads reach any file the target user can
// read, as Claude's and Codex's do, and writes are confined to the
// workspace root. That confinement is a guardrail against mistakes, not an
// isolation boundary:
// `RunCommand` already gives the model a shell on the same Machine, and a
// path can change between the check and its use (design "Revision 5
// simplification"). A mutation
// whose reply is lost after it may have been admitted is never retried or
// resolved by observation: the tool reports `outcome unknown` and the proxy
// ends the turn (the worker's trusted ledger records `uncertain_effect`).
// A process the executor itself reports forgotten has ended (evidence rule
// E1, docs/uncertain-effect-recovery.md); that answered request is not a
// lost one.
import { createHash, randomUUID } from "node:crypto";
import { posix } from "node:path";
import { fileURLToPath } from "node:url";
import {
  APPLY,
  decodeText,
  ENV_POLICY,
  forgotten,
  notSent,
  imageType,
  INLINE_APPLY,
  inside,
  LIMITS,
  lost,
  missing,
  Retained,
  remoteMessage,
  sha256,
  splice,
  TOOLS,
  ToolError,
  UncertainEffect,
  unknownOutcome,
  unreachable,
  url,
} from "./facade-tools.mjs";
import { MANAGED_FACADE_TOOLS } from "./managed.mjs";
import {
  formatPostEditNote,
  lfSha256,
  PostEditDedupe,
  postEditDedupeKey,
} from "./post-edit-notes.mjs";
import { planUpdate } from "./presentation.mjs";

/** The worker endpoint's own request recording an uncertain effect the
 * facade detected (`src/worker_execution/ledger.rs`); never forwarded. */
export const UNCERTAIN_REPORT = "cowboy/uncertainEffect";
/** Thrown ONLY when `beforeSend` refuses a request positively before `socket.send()`. */
class PreDispatchRefusal extends ToolError {}

/** Fixed Cowboy-authored advisory note appended once per root turn on wrap-up hint. */
export const WRAP_UP_NOTE =
  "[Cowboy note: the owner asked you to wrap up your current step briefly so they can follow up next.]";

/** The process id prefix of readiness probes (diagnostic only: the worker
 * authorizes the probe from its own state, never from this id). */
export const READINESS_PREFIX = "cowboy-agy-readiness-";

/** Maximum byte length of one JSON apply request fed to `APPLY_STDIN`. */
const MAX_STDIN_APPLY_BYTES = 5 * 1024 * 1024;
/**
 * Bounded, stdin-fed large-edit wrapper: confirms the execution helper exists
 * before reading stdin, reads exactly `len + 1` bytes terminated by a newline
 * into memory, and only then creates and owns a private 0600 scratch file that
 * is unconditionally removed on exit or signal. Neither pre-write fencing nor
 * exit-127 helper unavailability can leave a payload file behind.
 */
const APPLY_STDIN = [
  "set -eu",
  '[ -n "${COWBOY_EXECUTION_FILE_HELPER:-}" ] && [ -f "$COWBOY_EXECUTION_FILE_HELPER" ] && [ -x "$COWBOY_EXECUTION_FILE_HELPER" ] || exit 127',
  'len="$1"',
  "case \"$len\" in ''|*[!0-9]*) exit 2 ;; esac",
  `[ "$len" -ge 1 ] && [ "$len" -le ${MAX_STDIN_APPLY_BYTES} ] || exit 2`,
  "my_uid=$(id -u) || exit 2",
  "case \"$my_uid\" in ''|*[!0-9]*) exit 2 ;; esac",
  'h="${HOME:-}"',
  'while [ "$h" != "/" ] && [ "${h%/}" != "$h" ]; do h="${h%/}"; done',
  'case "$h" in /*) [ "$h" != "/" ] || exit 2 ;; *) exit 2 ;; esac',
  "check_ancestors() {",
  '  cur="$1"',
  '  child_uid=""',
  "  while :; do",
  '    [ -d "$cur" ] || exit 2',
  '    [ ! -L "$cur" ] || exit 2',
  '    line=$(LC_ALL=C ls -ldn -- "$cur" 2>/dev/null) || exit 2',
  "    IFS=' ' read -r perms _ uid _ <<EOF",
  "$line",
  "EOF",
  '    case "$uid" in',
  '      0|"$my_uid") ;;',
  "      *) exit 2 ;;",
  "    esac",
  '    case "$perms" in',
  "      d????-??-?*) ;;",
  "      d????????t*|d????????T*)",
  '        [ -n "$child_uid" ] || exit 2',
  '        case "$child_uid" in',
  '          0|"$my_uid") ;;',
  "          *) exit 2 ;;",
  "        esac",
  "        ;;",
  "      *) exit 2 ;;",
  "    esac",
  '    [ "$cur" = "/" ] && break',
  '    child_uid="$uid"',
  "    parent=${cur%/*}",
  "    cur=${parent:-/}",
  "  done",
  "}",
  "check_owner_dir() {",
  '  [ -d "$1" ] || exit 2',
  '  [ ! -L "$1" ] || exit 2',
  '  [ -O "$1" ] || exit 2',
  '  line=$(LC_ALL=C ls -ldn -- "$1" 2>/dev/null) || exit 2',
  "  IFS=' ' read -r perms _ uid _ <<EOF",
  "$line",
  "EOF",
  '  [ "$uid" = "$my_uid" ] || exit 2',
  '  case "$perms" in d????-??-?*) ;; *) exit 2 ;; esac',
  "}",
  "check_private_dir() {",
  '  check_owner_dir "$1"',
  '  chmod 00700 -- "$1" || exit 2',
  '  line=$(LC_ALL=C ls -ldn -- "$1" 2>/dev/null) || exit 2',
  "  IFS=' ' read -r perms _ uid _ <<EOF",
  "$line",
  "EOF",
  '  [ "$uid" = "$my_uid" ] || exit 2',
  '  case "$perms" in drwx------*) ;; *) exit 2 ;; esac',
  "}",
  'raw=$(LC_ALL=C head -c "$((len + 1))"; printf x)',
  "nl='\n'",
  'case "$raw" in *"${nl}x") ;; *) exit 2 ;; esac',
  "req=${raw%?}",
  "req=${req%?}",
  "unset raw",
  '(LC_ALL=C; [ "${#req}" -eq "$len" ]) || exit 2',
  "orig_umask=$(umask)",
  "umask 077",
  'check_owner_dir "$h"',
  'h_real=$(cd -P -- "$h" && pwd -P && printf x) || exit 2',
  "h_real=${h_real%?}",
  "h_real=${h_real%?}",
  '[ -n "$h_real" ] || exit 2',
  'check_ancestors "$h_real"',
  'c="$h_real/.cache"',
  'if [ ! -e "$c" ] && [ ! -L "$c" ]; then mkdir -- "$c" 2>/dev/null || true; fi',
  'check_owner_dir "$c"',
  'cb="$c/cowboy"',
  'if [ ! -e "$cb" ] && [ ! -L "$cb" ]; then mkdir -- "$cb" 2>/dev/null || true; fi',
  'if [ -L "$cb" ]; then',
  '  [ -d "$cb" ] || exit 2',
  '  cb_real=$(cd -P -- "$cb" && pwd -P && printf x) || exit 2',
  "  cb_real=${cb_real%?}",
  "  cb_real=${cb_real%?}",
  '  [ -n "$cb_real" ] || exit 2',
  '  check_owner_dir "$cb_real"',
  '  check_ancestors "$cb_real"',
  "else",
  '  check_owner_dir "$cb"',
  '  cb_real="$cb"',
  "fi",
  'fb="$cb_real/file-buffers"',
  'if [ ! -e "$fb" ] && [ ! -L "$fb" ]; then mkdir -- "$fb" 2>/dev/null || true; fi',
  'check_private_dir "$fb"',
  'd="$fb/antigravity"',
  'if [ ! -e "$d" ] && [ ! -L "$d" ]; then mkdir -- "$d" 2>/dev/null || true; fi',
  'check_private_dir "$d"',
  "LC_ALL=C find \"$d\" -maxdepth 1 -type f -name 'agy-apply-[0-9A-Za-z][0-9A-Za-z][0-9A-Za-z][0-9A-Za-z][0-9A-Za-z][0-9A-Za-z][0-9A-Za-z][0-9A-Za-z]' -mmin +60 -exec rm -f -- {} + 2>/dev/null || true",
  'tmp=""',
  'cleanup() { [ -n "$tmp" ] && rm -f -- "$tmp" || true; }',
  "trap cleanup EXIT",
  "trap 'cleanup; trap - HUP; kill -HUP $$' HUP",
  "trap 'cleanup; trap - INT; kill -INT $$' INT",
  "trap 'cleanup; trap - TERM; kill -TERM $$' TERM",
  'tmp=$(mktemp "$d/agy-apply-XXXXXXXX") || exit 2',
  '[ -f "$tmp" ] || exit 2',
  '[ ! -L "$tmp" ] || exit 2',
  '[ -O "$tmp" ] || exit 2',
  `printf '%s' "$req" > "$tmp" || exit 2`,
  "unset req",
  'umask "$orig_umask"',
  '"$COWBOY_EXECUTION_FILE_HELPER" apply --request-file "$tmp"',
].join("\n");
/** A publish settled by the file's bytes alone (E1, see `publishOnce`). */
const VERIFIED =
  "the requested content is verified on the target; the write helper's own report was lost";

/** `resolveOnce`: a missing component appeared while it resolved. */
const APPEARED = Symbol("appeared");

/** Shells whose `-c` script is POSIX shell syntax. */
const POSIX_SHELLS = new Set(["sh", "bash", "zsh", "dash", "ksh", "mksh"]);
/** `value` as one single-quoted POSIX shell word. */
const posixLiteral = (value) => `'${value.replaceAll("'", "'\\''")}'`;
/** `value` as one single-quoted fish word. */
const fishLiteral = (value) => `'${value.replaceAll("\\", "\\\\").replaceAll("'", "\\'")}'`;

export {
  inside,
  LIMITS,
  Retained,
  TOOL_NAMES,
  ToolError,
  UncertainEffect,
} from "./facade-tools.mjs";

export class Facade {
  /**
   * @param connection the proxy's execution `Connection`
   * @param root the canonical workspace root on the target
   * @param shell the target user's shell
   * @param onUncertain called with (sessionId, method) when a turn must end
   * @param onLost called with (sessionId, method) at the moment an effect is
   *   lost, before anything is awaited, so a subagent's parent turn and its
   *   other children are fenced at once
   * @param onResult called with (call, result) once a tool call is answered,
   *   before `agy` receives the answer (`call` is `{ sessionId, name, args }`,
   *   one object per call, so a presentation can correlate it)
   * @param onProgress called with (call, text) while `RunCommand` runs: the
   *   output retained so far, at most every `LIMITS.progressMs`
   * @param managed a managed read-only child: only `MANAGED_FACADE_TOOLS`
   *   exist and no extension is ever attached
   */
  constructor(
    connection,
    {
      root,
      shell,
      onUncertain = () => {},
      onLost = () => {},
      onResult = () => {},
      onProgress = () => {},
      onAdmit = () => {},
      authorize = () => ({ allowed: true }),
      managed = false,
      stopConfirmMs = LIMITS.stopConfirmMs,
      publishTimeoutMs = LIMITS.publishTimeoutMs,
      diagnostics = undefined,
      canaryTrace = false,
    },
  ) {
    this.connection = connection;
    this.managed = managed;
    this.stopConfirmMs = stopConfirmMs;
    this.publishTimeoutMs = publishTimeoutMs;
    this.diagnostics = diagnostics;
    this.canaryTrace = Boolean(canaryTrace);
    this.canaryNonce =
      typeof canaryNonce === "string" && canaryNonce.length > 0 ? canaryNonce : undefined;
    this.startWaitMs = 10_000;
    this.root = root;
    this.shell = shell;
    this.onUncertain = onUncertain;
    this.onLost = onLost;
    this.onResult = onResult;
    this.onProgress = onProgress;
    this.onAdmit = onAdmit;
    this.authorize = authorize;
    this.sessions = new Map();
    /** Per canonical path, how often a publish of it became uncertain. */
    this.invalidations = new Map();
    /** Per canonical path serialization locks for WriteFile/EditFile. */
    this.fileLocks = new Map();
    /** Per canonical path count of in-flight writes. */
    this.activeWrites = new Map();
    /** Per canonical path monotonic write dispatch generation. */
    this.writeDispatchGen = new Map();
    /** Per canonical path monotonic committed publish revision. */
    this.committedRevision = new Map();
    /** Per canonical path latest observation generation + raw SHA-256 digest (H-Proxy-3). */
    this.fileObservation = new Map();
    /** Tokenized deduplication of delivered post-edit diagnostic notes. */
    this.postEditDedupe = new PostEditDedupe();
    /** Active `endTurn` promises kept visible to `settled()` even after turn detach. */
    this.activeCleanups = new Set();
    /** Active root `endTurn` command-stop deadlines for shutdown drain budgeting. */
    this.rootCleanupDeadlines = new Set();
    /** Open MCP relay streams (`relayId -> sessionOf`). */
    this.relays = new Map();
    this.nextRelayId = 1;
    /** The session's extension tools (`extensions.mjs`), once started. */
    this.extensions = undefined;
    /**
     * The proxy's subagent host (`subagents.mjs`): `start(sessionId, {prompt,
     * description})`, `wait(sessionId, id, ms, signal)` and `stop(sessionId,
     * id)`. Absent until the proxy attaches (and in a managed child).
     */
    this.subagents = undefined;
  }

  /** Canonicalize the binding's workspace root and build the facade. */
  static async create(connection, options = {}) {
    const workspace = connection.descriptor.binding.workspace.cwd;
    const canonical = await connection.call("fs/canonicalize", { path: url(workspace) });
    const root = fileURLToPath(canonical.path);
    const shell = connection.info?.shell?.path;
    if (typeof shell !== "string" || !posix.isAbsolute(shell)) {
      throw new Error("Execution environment has no shell");
    }
    return new Facade(connection, { root, shell, ...options });
  }

  /** Attach or replace the dispatch authorizer (e.g. `AcpProxy`). */
  setAuthorizer(authorize) {
    this.authorize = typeof authorize === "function" ? authorize : () => ({ allowed: true });
  }

  /** Attach or replace the admission hook capturing call turn lifetime for UI binding. */
  setAdmitter(onAdmit) {
    this.onAdmit = typeof onAdmit === "function" ? onAdmit : () => {};
  }

  tools() {
    if (this.managed) {
      // The keeper refuses writes anyway; a managed child is never offered
      // them, and RunCommand's description names no StopCommand.
      return structuredClone(TOOLS)
        .filter((tool) => MANAGED_FACADE_TOOLS.includes(tool.name))
        .map((tool) =>
          tool.name === "RunCommand"
            ? {
                ...tool,
                description:
                  "Run a read-only shell command without network in the workspace root (commands cannot change files). Waits up to wait_seconds (default: until it ends); a command still running returns its command_id, which RunCommand continues. Output keeps at most 256 KiB (head and tail).",
              }
            : tool,
        );
    }
    return [...structuredClone(TOOLS), ...(this.extensions?.tools() ?? [])];
  }

  session(sessionId) {
    let session = this.sessions.get(sessionId);
    if (!session) {
      session = {
        turn: undefined,
        quiescedInflight: new Set(),
        observed: new Map(),
        commands: new Map(),
        parent: undefined,
        // The parent's turn that was open when the child started.
        parentTurn: undefined,
        // Relay sockets that were open across an ambiguous turn end; cleared only on verified close.
        ambiguousRelays: new Set(),
      };
      this.sessions.set(sessionId, session);
    }
    return session;
  }

  /** `childId` is a subagent of `parentId`: its lost effects end the parent's turn too. */
  link(childId, parentId) {
    const session = this.session(childId);
    session.parent = parentId;
    session.parentTurn = this.session(parentId).turn;
  }

  /**
   * A settled subagent's session is forgotten (its turn already ended): a
   * command whose termination is not yet confirmed keeps it tracked.
   */
  forgetSession(sessionId) {
    const session = this.sessions.get(sessionId);
    if (
      session &&
      !session.turn &&
      session.commands.size === 0 &&
      session.quiescedInflight.size === 0
    )
      this.sessions.delete(sessionId);
  }

  registerRelay(sessionOf) {
    const relayId = this.nextRelayId++;
    this.relays.set(relayId, sessionOf);
    return relayId;
  }

  unregisterRelay(relayId) {
    this.relays.delete(relayId);
    for (const session of this.sessions.values()) {
      session.ambiguousRelays.delete(relayId);
    }
  }

  markAmbiguousRelays(sessionId) {
    const session = this.session(sessionId);
    if (session.parent !== undefined) return;
    for (const [relayId, sessionOf] of this.relays) {
      let bound;
      try {
        bound = sessionOf();
      } catch {
        bound = undefined;
      }
      if (bound === sessionId || bound === undefined) {
        session.ambiguousRelays.add(relayId);
      }
    }
  }

  hasOpenCommands(sessionId) {
    return (this.sessions.get(sessionId)?.commands.size ?? 0) > 0;
  }

  beginCall(sessionId) {
    const turn = sessionId === undefined ? undefined : this.sessions.get(sessionId)?.turn;
    if (turn) turn.openCalls += 1;
    return turn;
  }

  endCall(turn) {
    if (turn && turn.openCalls > 0) turn.openCalls -= 1;
  }

  detachTurn(sessionId, { cancelled = false, errored = false } = {}) {
    const session = this.sessions.get(sessionId);
    if (!session) return;
    const hadUnsettled =
      (session.turn?.inflight.size ?? 0) > 0 || (session.turn?.openCalls ?? 0) > 0;
    if (session.parent === undefined && (cancelled || errored || hadUnsettled)) {
      this.markAmbiguousRelays(sessionId);
    }
    if (session.turn) {
      session.turn.ended = true;
      session.turn = undefined;
    }
  }

  beginTurn(sessionId, { wrapUp = null, turnNonce = undefined } = {}) {
    this.session(sessionId).turn = {
      sessionId,
      cancelled: false,
      ended: false,
      uncertain: undefined,
      wrapUp,
      turnNonce,
      inflight: new Set(),
      openCalls: 0,
      // Aborted when the turn ends or is cancelled: a call waiting on a
      // subagent returns at once instead of holding the end of the turn.
      abort: new AbortController(),
    };
  }

  /**
   * The turn ended or was cancelled. On every terminal path the turn is
   * fenced first (no new effect is admitted, also by a tool call still
   * running), then its admitted mutations are awaited and its commands
   * stopped. A command whose termination is not confirmed stays tracked.
   */
  async endTurn(sessionId, { cancelled = false, errored = false } = {}) {
    const session = this.session(sessionId);
    const turn = session.turn;
    const isRoot = session.parent === undefined;
    const hadUnsettled = (turn?.inflight.size ?? 0) > 0 || (turn?.openCalls ?? 0) > 0;
    if (isRoot && (cancelled || errored || hadUnsettled)) {
      this.markAmbiguousRelays(sessionId);
    }
    if (turn) {
      turn.cancelled ||= cancelled;
      if (turn.wrapUp) turn.wrapUp.pending = false;
      if (!cancelled) {
        turn.ended = true;
        session.turn = undefined;
      }
      turn.abort?.abort();
    }
    const confirmMs = isRoot ? this.stopConfirmMs : 5000;
    const startWaitMs = isRoot ? this.startWaitMs : 5000;
    const reportMarginMs = 5000;
    // Combined budget aligns with Controller EXECUTION_FORCE_CANCEL_GRACE (75s =
    // 10s start registration + 60s Stop confirmation + 5s report flush margin).
    const deadlineEntry = isRoot
      ? { deadline: Date.now() + startWaitMs + confirmMs + reportMarginMs }
      : undefined;
    if (deadlineEntry) this.rootCleanupDeadlines.add(deadlineEntry);
    const cleanup = (async () => {
      try {
        // Bounded start/publish registration phase (joins detached quiesce work too).
        const startDeadline = Date.now() + startWaitMs;
        let inflightTimedOut = false;
        const awaited = new Set();
        while (true) {
          const pending = [...(turn?.inflight ?? []), ...session.quiescedInflight].filter(
            (work) => !awaited.has(work),
          );
          if (pending.length === 0) break;
          const remaining = startDeadline - Date.now();
          if (remaining <= 0) {
            inflightTimedOut = true;
            break;
          }
          let timer;
          const timedOut = new Promise((resolve) => {
            timer = setTimeout(() => resolve("timeout"), remaining);
          });
          let outcome;
          try {
            outcome = await Promise.race([
              Promise.allSettled(pending).then(() => "done"),
              timedOut,
            ]);
          } finally {
            clearTimeout(timer);
          }
          if (outcome === "timeout") {
            inflightTimedOut = true;
            break;
          }
          for (const work of pending) awaited.add(work);
        }
        // Run start-timeout uncertainty reporting CONCURRENTLY with stopping
        // known commands under a strict 3s request timeout so reporting can
        // never starve termination or wait 185s on a wedged transport.
        const startReport =
          isRoot && inflightTimedOut
            ? this.reportUncertainBounded(sessionId, turn, "process/start", 3000)
            : Promise.resolve();
        const commands = [...session.commands.values()];
        if (deadlineEntry) {
          if (commands.length > 0) {
            deadlineEntry.deadline = Date.now() + confirmMs + reportMarginMs;
          } else {
            this.rootCleanupDeadlines.delete(deadlineEntry);
          }
        }
        let anyUnconfirmed = false;
        await Promise.all([
          startReport,
          ...commands.map(async (command) => {
            clearTimeout(command.timer);
            command.cleaning = (command.cleaning ?? 0) + 1;
            let closed;
            try {
              closed = await this.terminate(command, confirmMs).catch(() => false);
            } finally {
              command.cleaning -= 1;
            }
            if (closed) this.forget(session, command);
            else {
              anyUnconfirmed = true;
              if (
                !this.closing &&
                command.cleaning === 0 &&
                session.commands.get(command.processId) === command
              ) {
                clearTimeout(command.timer);
                command.timer = setTimeout(() => this.expire(session, command), 5000);
                command.timer.unref?.();
              }
            }
          }),
        ]);
        if (isRoot && anyUnconfirmed) {
          await this.reportUncertainBounded(sessionId, turn, "process/terminate", 3000);
        }
      } finally {
        if (deadlineEntry) this.rootCleanupDeadlines.delete(deadlineEntry);
      }
    })();
    this.activeCleanups.add(cleanup);
    try {
      await cleanup;
    } finally {
      this.activeCleanups.delete(cleanup);
    }
  }

  /**
   * The proxy is exiting: no new target work starts. Every command's expiry
   * timer stops, so the connection's in-flight requests can drain before it
   * closes (design L1c); a process left running is the keeper's to reap.
   */
  quiesce() {
    this.closing = true;
    this.diagnostics?.close?.();
    for (const session of this.sessions.values()) {
      if (session.turn) {
        session.turn.ended = true;
        session.turn.cancelled = true;
        if (session.turn.wrapUp) session.turn.wrapUp.pending = false;
        // Retain already-admitted inflight work (e.g. file publish / command
        // start) in BOTH session.quiescedInflight (so buffered endTurn awaits it)
        // and activeCleanups (so settled() waits for it before connection close).
        for (const work of session.turn.inflight) {
          session.quiescedInflight.add(work);
          this.activeCleanups.add(work);
          const done = () => {
            session.quiescedInflight.delete(work);
            this.activeCleanups.delete(work);
          };
          work.then(done, done);
        }
        session.turn.abort?.abort();
        session.turn = undefined;
      }
      for (const command of session.commands.values()) clearTimeout(command.timer);
    }
  }

  /** Remaining root Stop confirmation budget (ms) for shutdown drain, including report margin. */
  remainingRootStopMs() {
    let maxDeadline = 0;
    for (const entry of this.rootCleanupDeadlines) {
      if (entry.deadline > maxDeadline) maxDeadline = entry.deadline;
    }
    if (maxDeadline === 0) return 0;
    return Math.max(
      0,
      Math.min(this.startWaitMs + this.stopConfirmMs + 10_000, maxDeadline - Date.now()),
    );
  }

  /** Report uncertainty with a strict bound and local turn/proxy fallback so
   *  a refused or timed-out report still parks the turn on the trusted worker. */
  async reportUncertainBounded(sessionId, turn, method, timeoutMs = 3000) {
    if (turn) this.fenceLost(turn, method);
    this.onUncertain(sessionId, method);
    try {
      await Promise.race([
        this.connection.call(UNCERTAIN_REPORT, { method }, { timeoutMs }),
        new Promise((resolve) => setTimeout(resolve, timeoutMs)),
      ]);
    } catch {
      // Proxy already received `onUncertain` and `turn.uncertain` fallback.
    }
  }

  /** Record uncertainty explicitly before closing the execution connection if
   *  shutdown drain timed out while admitted starts or cleanups were still open. */
  async reportUnsettledOnTimeout(timeoutMs = 2000) {
    const unsettledSessions = [...this.sessions.entries()].filter(
      ([, s]) =>
        (s.turn?.inflight.size ?? 0) > 0 || s.quiescedInflight.size > 0 || s.commands.size > 0,
    );
    if (this.activeCleanups.size === 0 && unsettledSessions.length === 0) return;
    for (const [sid, session] of unsettledSessions) {
      if (session.turn) this.fenceLost(session.turn, "process/start");
      this.onUncertain(sid, "process/start");
    }
    try {
      await Promise.race([
        this.connection.call(UNCERTAIN_REPORT, { method: "process/start" }, { timeoutMs }),
        new Promise((resolve) => setTimeout(resolve, timeoutMs)),
      ]);
    } catch {
      // Local turn uncertainty already latched for settleExit.
    }
  }

  /** Explicitly mark one relay ambiguous (e.g. failed response handoff) until socket close. */
  markRelayAmbiguous(sessionId, relayId) {
    if (sessionId === undefined || relayId === undefined) return;
    const session = this.sessions.get(sessionId);
    if (!session || session.parent !== undefined) return;
    if (this.relays.has(relayId)) {
      session.ambiguousRelays.add(relayId);
    }
  }

  /**
   * Wait at most `ms` until every session's admitted turn work (a publish
   * through its verification and report included) settled; `false` when the
   * bound ended the wait.
   */
  async settled(ms) {
    const floorDeadline = Date.now() + Math.max(0, ms);
    const awaited = new Set();
    while (true) {
      const work = [
        ...[...this.sessions.values()].flatMap((session) => [
          ...(session.turn?.inflight ?? []),
          ...session.quiescedInflight,
        ]),
        ...this.activeCleanups,
      ].filter((item) => !awaited.has(item));
      if (work.length === 0) return true;
      const allDone = Promise.allSettled(work).then(() => "done");
      while (true) {
        const targetDeadline = Math.max(floorDeadline, Date.now() + this.remainingRootStopMs());
        const remaining = targetDeadline - Date.now();
        if (remaining <= 0) return false;
        let timer;
        const expired = new Promise((resolve) => {
          timer = setTimeout(() => resolve("timeout"), remaining);
        });
        let outcome;
        try {
          outcome = await Promise.race([allDone, expired]);
        } finally {
          clearTimeout(timer);
        }
        if (outcome === "done") {
          for (const item of work) awaited.add(item);
          break;
        }
        // Re-check root cleanup deadlines in case a late command start registered
        // while waiting and extended its confirmation + report window.
        if (this.remainingRootStopMs() <= 0) return false;
      }
    }
  }

  /** Stop tracking a command whose process is gone. */
  forget(session, command) {
    clearTimeout(command.timer);
    session.commands.delete(command.processId);
  }

  /** Convert canonical absolute target path into a validated workspace-relative POSIX path (R3-M15). */
  relativeWorkspacePath(file) {
    if (typeof file !== "string" || !inside(this.root, file)) return null;
    const rel = posix.relative(this.root, file);
    if (
      !rel ||
      rel === "." ||
      rel === ".." ||
      rel.startsWith("/") ||
      rel.startsWith("../") ||
      rel.includes("\0") ||
      rel.includes("//") ||
      rel.split("/").some((part) => part === "" || part === "." || part === "..")
    ) {
      return null;
    }
    return rel;
  }

  /** Serialize concurrent WriteFile/EditFile operations on the same canonical file. */
  async withFileLock(file, fn) {
    const prev = this.fileLocks.get(file) ?? Promise.resolve();
    let release;
    const gate = new Promise((resolve) => {
      release = resolve;
    });
    const next = prev.catch(() => {}).then(() => gate);
    this.fileLocks.set(file, next);
    await prev.catch(() => {});
    try {
      return await fn();
    } finally {
      release();
      if (this.fileLocks.get(file) === next) this.fileLocks.delete(file);
    }
  }

  /** Record a file observation generation + raw SHA-256 across reads, conflicts, and publishes (H-Proxy-3). */
  recordFileObservation(file, rawDigest, { forceAdvance = false } = {}) {
    const prev = this.fileObservation.get(file);
    if (
      !forceAdvance &&
      prev !== undefined &&
      typeof rawDigest === "string" &&
      prev.rawDigest === rawDigest
    ) {
      return prev;
    }
    const gen = (prev?.gen ?? 0) + 1;
    const entry = { gen, rawDigest };
    this.fileObservation.set(file, entry);
    return entry;
  }

  /** Synchronous pre-effect authorization guard run immediately before socket.send() (R3-H3, H-Proxy-4). */
  assertAuthorizedBeforeEffect(session, turn, decision, signal, fileCheck = undefined) {
    if (
      this.closing ||
      signal?.aborted ||
      turn.abort?.signal?.aborted ||
      session.turn !== turn ||
      turn.cancelled ||
      turn.ended ||
      turn.uncertain !== undefined ||
      (typeof decision?.confirm === "function" && !decision.confirm())
    ) {
      throw new PreDispatchRefusal(
        "The tool request was cancelled because the session's mode or turn changed.",
      );
    }
    if (fileCheck) {
      const { file, path, queuedObserved, queuedInvalidationGen } = fileCheck;
      if ((this.invalidations.get(file) ?? 0) !== queuedInvalidationGen) {
        throw new PreDispatchRefusal(
          `${path} became uncertain since it was read; read it again first.`,
        );
      }
      if (session.observed.get(file) !== queuedObserved) {
        throw new PreDispatchRefusal(`${path} changed since it was read; read it again first.`);
      }
    }
  }

  /** Pull post-edit diagnostics over the separate `/diagnostics` channel when admissible. */
  async pullPostEditNote(session, turn, postEditMeta, { signal = undefined } = {}) {
    if (
      this.managed ||
      !postEditMeta ||
      !session ||
      session.parent !== undefined ||
      !turn ||
      session.turn !== turn ||
      turn.cancelled ||
      turn.ended ||
      turn.uncertain !== undefined ||
      !turn.turnNonce ||
      signal?.aborted ||
      turn.abort?.signal?.aborted ||
      !this.diagnostics
    ) {
      return null;
    }
    const { file, relPath, expectedLfSha256, rawDigest, rev, obsGen, writeGen, invalidationGen } =
      postEditMeta;
    const signals = [turn.abort?.signal, signal].filter(Boolean);
    const combinedSignal = signals.length > 1 ? AbortSignal.any(signals) : signals[0];
    const isFresh = () => {
      const obs = this.fileObservation.get(file);
      return (
        !this.closing &&
        !combinedSignal?.aborted &&
        session.turn === turn &&
        !turn.cancelled &&
        !turn.ended &&
        turn.uncertain === undefined &&
        (this.activeWrites.get(file) ?? 0) === 0 &&
        this.committedRevision.get(file) === rev &&
        (this.writeDispatchGen.get(file) ?? 0) === writeGen &&
        (this.invalidations.get(file) ?? 0) === invalidationGen &&
        obs?.gen === obsGen &&
        obs?.rawDigest === rawDigest
      );
    };

    if (!isFresh()) return null;
    const canaryInspectId = this.canaryTrace
      ? `canary-${randomUUID().replaceAll("-", "")}`
      : undefined;
    const makeCanaryMeta = (expectedNoteDigest = null) =>
      canaryInspectId ? { inspectId: canaryInspectId, expectedNoteDigest } : undefined;

    const reply = await this.diagnostics.postEdit({
      path: relPath,
      expectedLfSha256,
      turnNonce: turn.turnNonce,
      timeoutMs: 4000,
      canaryInspectId,
      signal: combinedSignal,
    });
    if (!isFresh() || reply?.state !== "projected") {
      const canaryMeta = makeCanaryMeta(null);
      return canaryMeta ? { note: null, dedupeKey: null, isFresh, canaryMeta } : null;
    }
    const formatted = formatPostEditNote({
      payload: reply.payload,
      matchesExpected: reply.matchesExpected,
      expectedPath: relPath,
    });
    if (!formatted) {
      const canaryMeta = makeCanaryMeta(null);
      return canaryMeta ? { note: null, dedupeKey: null, isFresh, canaryMeta } : null;
    }
    const dedupeKey = postEditDedupeKey(turn.sessionId, file, expectedLfSha256, formatted.digest);
    return {
      note: formatted.note,
      dedupeKey,
      isFresh,
      canaryMeta: makeCanaryMeta(formatted.digest),
    };
  }

  appendTextNote(result, note) {
    if (!note || !Array.isArray(result?.content)) return result;
    const firstTextIndex = result.content.findIndex(
      (block) => block?.type === "text" && typeof block.text === "string",
    );
    const content =
      firstTextIndex >= 0
        ? result.content.map((block, index) =>
            index === firstTextIndex ? { ...block, text: `${block.text}\n\n${note}` } : block,
          )
        : [...result.content, { type: "text", text: note }];
    return { ...result, content };
  }

  /** One MCP `tools/call`, answered as an MCP tool result. */
  async call(sessionId, name, args, relayId = undefined, { signal = undefined } = {}) {
    const session = sessionId === undefined ? undefined : this.sessions.get(sessionId);
    const callTurn = session?.turn;
    if (callTurn) callTurn.openCalls += 1;
    const call = { sessionId, name, args };
    let rawResult;
    try {
      rawResult = await this.answer(call, { signal });
    } finally {
      if (callTurn && callTurn.openCalls > 0) callTurn.openCalls -= 1;
    }
    const postEditMeta = rawResult?._postEditMeta;
    const cleanResult =
      rawResult && Object.hasOwn(rawResult, "_postEditMeta")
        ? { content: rawResult.content, ...(rawResult.isError ? { isError: true } : {}) }
        : rawResult;
    // Presentation only: it never changes the answer `agy` receives.
    try {
      this.onResult(call, cleanResult);
    } catch {
      // A presentation failure must not fail the tool.
    }
    let result = cleanResult;
    if (!result?.isError && postEditMeta) {
      const pulled = await this.pullPostEditNote(session, callTurn, postEditMeta, { signal });
      if (pulled?.note && pulled.dedupeKey && pulled.isFresh()) {
        const token = this.postEditDedupe.reserve(pulled.dedupeKey);
        if (token !== null) {
          result = this.appendTextNote(result, pulled.note);
          this.postEditDedupe.commit(token);
        }
      }
    }
    return this.maybeAttachWrapUpNote(session, callTurn, name, result, relayId);
  }

  /**
   * `serveMcp` entry point: evaluates `isFresh()` synchronously inside `commitReply`'s
   * `!entry.committed` latch (R2-M13) and commits dedupe tokens only when the relay
   * write succeeds.
   */
  async callForMcp(sessionId, name, args, relayId, { signal, commitReply }) {
    const session = sessionId === undefined ? undefined : this.sessions.get(sessionId);
    const callTurn = session?.turn;
    if (callTurn) callTurn.openCalls += 1;
    const call = { sessionId, name, args };
    let rawResult;
    try {
      rawResult = await this.answer(call, { signal });
    } finally {
      if (callTurn && callTurn.openCalls > 0) callTurn.openCalls -= 1;
    }
    const postEditMeta = rawResult?._postEditMeta;
    const cleanResult =
      rawResult && Object.hasOwn(rawResult, "_postEditMeta")
        ? { content: rawResult.content, ...(rawResult.isError ? { isError: true } : {}) }
        : rawResult;
    try {
      this.onResult(call, cleanResult);
    } catch {
      // Presentation only.
    }
    let pulled = null;
    if (!cleanResult?.isError && postEditMeta) {
      pulled = await this.pullPostEditNote(session, callTurn, postEditMeta, { signal });
    }
    let noteAttached = false;
    let dedupeToken = null;
    const delivered = await commitReply((entry) => {
      let finalResult = cleanResult;
      if (pulled?.note && pulled.dedupeKey && !entry.cancelled && pulled.isFresh()) {
        // M-Proxy-5: Reserve synchronously inside the !entry.committed handoff latch.
        dedupeToken = this.postEditDedupe.reserve(pulled.dedupeKey);
        if (dedupeToken !== null) {
          finalResult = this.appendTextNote(finalResult, pulled.note);
          noteAttached = true;
        }
      }
      finalResult = this.maybeAttachWrapUpNote(session, callTurn, name, finalResult, relayId);
      const envelope = { result: finalResult };
      if (this.canaryTrace && pulled?.canaryMeta) {
        envelope._cowboyCanary = { ...pulled.canaryMeta, noteAttached };
      }
      return envelope;
    });
    if (dedupeToken !== null) {
      if (delivered && noteAttached) this.postEditDedupe.commit(dedupeToken);
      else this.postEditDedupe.release(dedupeToken);
    }
    return delivered;
  }

  /**
   * Non-mutating attachment of `WRAP_UP_NOTE` strictly once per root turn when
   * no ambiguous relay exists, after `onResult` presentation has observed the
   * clean result. Attaches to the first text block (including `[text, image]`
   * `ReadFile` results), appending a text block only if none exists.
   */
  maybeAttachWrapUpNote(session, callTurn, name, result, relayId) {
    if (
      this.managed ||
      session === undefined ||
      session.parent !== undefined ||
      session.ambiguousRelays.size > 0 ||
      (relayId !== undefined && session.ambiguousRelays.has(relayId)) ||
      callTurn === undefined ||
      session.turn !== callTurn ||
      callTurn.cancelled ||
      callTurn.ended ||
      callTurn.uncertain !== undefined ||
      callTurn.wrapUp?.pending !== true ||
      callTurn.wrapUp.delivered !== false ||
      name === "UpdatePlan" ||
      result?.isError ||
      !Array.isArray(result?.content)
    ) {
      return result;
    }
    callTurn.wrapUp.pending = false;
    callTurn.wrapUp.delivered = true;
    const firstTextIndex = result.content.findIndex(
      (block) => block?.type === "text" && typeof block.text === "string",
    );
    const content =
      firstTextIndex >= 0
        ? result.content.map((block, index) =>
            index === firstTextIndex
              ? {
                  ...block,
                  text: `${block.text}

${WRAP_UP_NOTE}`,
                }
              : block,
          )
        : [...result.content, { type: "text", text: WRAP_UP_NOTE }];
    return { ...result, content };
  }

  async answer(call, { signal = undefined } = {}) {
    if (this.closing) throw new ToolError("Antigravity proxy is shutting down");
    return this.answerOnce(call, { signal });
  }

  async answerOnce(call, { signal = undefined } = {}) {
    const { sessionId, name } = call;
    let { args } = call;
    const failure = (text) => ({ content: [{ type: "text", text }], isError: true });
    // Reject non-string tool names before any handler or policy lookup so JS
    // property coercion (e.g. ["WriteFile"]) cannot bypass plan checks.
    if (typeof name !== "string" || name.length === 0) {
      return failure(`Unknown tool ${String(name)}`);
    }
    const session = sessionId === undefined ? undefined : this.session(sessionId);
    const turn = session?.turn;
    if (!turn) return failure("No Cowboy turn is active, so workspace tools are unavailable.");
    if (turn.uncertain) {
      return failure(`The turn ended: an earlier ${turn.uncertain} has an unknown outcome.`);
    }
    if (turn.cancelled || turn.ended) return failure("The turn was cancelled.");
    if (signal?.aborted) return failure("The tool request was cancelled.");
    if (args === null || typeof args !== "object" || Array.isArray(args)) args = {};
    // Freeze a snapshot of the exact execution arguments before authorization so
    // the owner approves the exact payload that executes.
    const execArgs = structuredClone(args);
    if (this.managed && !MANAGED_FACADE_TOOLS.includes(name)) {
      return failure(`${String(name)} is not available in a read-only Cowboy managed call.`);
    }
    const handlers = {
      ReadFile: () => this.readFile(session, execArgs),
      ListDirectory: () => this.listDirectory(execArgs),
      SearchFiles: () => this.searchFiles(turn, execArgs),
      WriteFile: (decision) => this.writeFile(session, turn, execArgs, { decision, signal }),
      EditFile: (decision) => this.editFile(session, turn, execArgs, { decision, signal }),
      RunCommand: () =>
        this.runCommand(session, turn, execArgs, (text) => {
          try {
            this.onProgress(call, text);
          } catch {
            // Presentation only.
          }
        }),
      StopCommand: () => this.stopCommand(session, execArgs),
      Subagent: () => this.subagent(sessionId, turn, execArgs),
      StopSubagent: () => this.stopSubagent(sessionId, execArgs),
      // Presentation only (the proxy shows it as the session's plan); it
      // never reaches the target.
      UpdatePlan: async () => {
        if (!planUpdate(execArgs)) {
          throw new ToolError(
            "plan must list 1 to 50 steps, each with a step text and a status of pending, in_progress or completed",
          );
        }
        return "Plan updated.";
      },
    };
    // Cowboy MCP, the folder's bindings and memory answer for themselves.
    const extension = !Object.hasOwn(handlers, name) && this.extensions?.has(name);
    if (!Object.hasOwn(handlers, name) && !extension)
      return failure(`Unknown tool ${String(name)}`);
    // Capture call turn lifetime synchronously at admission for UI binding.
    try {
      this.onAdmit(call);
    } catch {
      // Presentation only.
    }
    // Enforce confirmed mode before any tool handler, extension RPC, or
    // inflight effect registration. In plan / unknown / switching modes,
    // RunCommand and extensions await single-use owner approval bound directly
    // to this exact invocation and `execArgs`.
    const decision = await this.authorize(call, { extension, args: execArgs, signal });
    if (decision?.allowed !== true) {
      return failure(decision?.reason ?? `${String(name)} is not available in the current mode.`);
    }
    // Re-check shutdown, request abort, turn, and owner mode generation
    // synchronously immediately before registering inflight work or executing.
    if (this.closing) throw new ToolError("Antigravity proxy is shutting down");
    if (signal?.aborted) return failure("The tool request was cancelled.");
    if (session.turn !== turn) {
      return failure("No Cowboy turn is active, so workspace tools are unavailable.");
    }
    if (turn.uncertain) {
      return failure(`The turn ended: an earlier ${turn.uncertain} has an unknown outcome.`);
    }
    if (turn.cancelled || turn.ended) return failure("The turn was cancelled.");
    if (typeof decision.confirm === "function" && !decision.confirm()) {
      return failure(`${name} was cancelled because the session's mode or turn changed.`);
    }

    const execute = async () => {
      try {
        if (extension) {
          return await this.extensions.call(this.connection, name, execArgs);
        }
        // A handler answers text, or (an image ReadFile) its own content blocks.
        const answer = await handlers[name](decision);
        return typeof answer === "string" ? { content: [{ type: "text", text: answer }] } : answer;
      } catch (error) {
        if (error instanceof UncertainEffect) {
          this.fenceLost(turn, error.method);
          // The worker's trusted ledger records it before this tool answers
          // (so before the turn can end), also when every transport call was
          // answered (a timed-out publish): the session parks until the owner
          // acknowledges it. A lost report still ends the turn here.
          await this.connection.call(UNCERTAIN_REPORT, { method: error.method }).catch(() => {});
          this.onUncertain(sessionId, error.method);
          return failure(
            `outcome unknown: the connection to the workspace Machine was lost after ${error.method} may have been applied. Cowboy ends this turn; do not retry.`,
          );
        }
        if (error instanceof ToolError) return failure(error.message);
        if (lost(error))
          return failure("The workspace Machine is unreachable; nothing was changed.");
        return failure(`The workspace Machine refused the request: ${remoteMessage(error)}`);
      }
    };
    const trackedSelf =
      Boolean(turn.inflight) && (name === "WriteFile" || name === "EditFile" || Boolean(extension));
    const parentInflight = session.parentTurn?.inflight;
    if (!trackedSelf && !parentInflight) {
      return execute();
    }
    // Track admitted mutations, extensions, and child executions on the owning
    // turn(s) only after authorization succeeds, staying active through any
    // UncertainEffect report.
    const running = execute();
    const tracked = running.catch(() => {});
    if (trackedSelf) turn.inflight.add(tracked);
    if (parentInflight) parentInflight.add(tracked);
    try {
      return await running;
    } finally {
      if (trackedSelf) turn.inflight.delete(tracked);
      if (parentInflight) parentInflight.delete(tracked);
    }
  }

  async rpc(method, params) {
    try {
      return await this.connection.call(method, params);
    } catch (error) {
      if (missing(error))
        throw new ToolError(`No such file or directory: ${fileURLToPath(params.path)}`);
      throw error;
    }
  }

  /**
   * An effect of `turn` has an unknown outcome: fence the turn, the parent
   * turn that was open when a child's effect was lost, and (through
   * `onLost`) its other children, before anything is awaited. Repeating it
   * changes nothing.
   */
  fenceLost(turn, method) {
    turn.uncertain ??= method;
    if (turn.sessionId === undefined) return;
    const session = this.sessions.get(turn.sessionId);
    if (session?.parentTurn) session.parentTurn.uncertain ??= method;
    try {
      this.onLost(turn.sessionId, method);
    } catch {
      // The turn's own fence still ends it.
    }
  }

  /**
   * One request that may change the target. It is admitted only while the
   * turn is open; an unknown outcome fences the turn at once, so a
   * concurrent tool call of the same turn admits nothing after it.
   */
  async mutate(turn, method, params, { beforeSend, timeoutMs } = {}) {
    if (turn.cancelled || turn.ended || turn.uncertain) {
      throw new ToolError("The turn was cancelled.");
    }
    const callOpts =
      beforeSend || timeoutMs !== undefined
        ? {
            ...(beforeSend ? { beforeSend } : {}),
            ...(timeoutMs !== undefined ? { timeoutMs } : {}),
          }
        : undefined;
    const call = this.connection.call(method, params, callOpts);
    turn.inflight?.add(call);
    try {
      return await call;
    } catch (error) {
      if (unknownOutcome(error)) {
        turn.uncertain ??= method;
        throw new UncertainEffect(method);
      }
      throw error;
    } finally {
      turn.inflight?.delete(call);
    }
  }

  /**
   * A canonical path, relative paths from the workspace root; missing tails
   * allowed. It must be inside the root unless `outside` (a read).
   */
  async resolve(path, { allowMissing = false, outside = false } = {}) {
    if (path === undefined) path = ".";
    if (typeof path !== "string" || !path || path.length > 4096 || path.includes("\0")) {
      throw new ToolError("Invalid path");
    }
    // A component that appears while this resolves is resolved again.
    for (let attempt = 0; ; attempt++) {
      const resolved = await this.resolveOnce(path, allowMissing);
      if (resolved === APPEARED) {
        if (attempt < 2) continue;
        throw new ToolError(`${path} changed while it was resolved; try again`);
      }
      if (!outside && !inside(this.root, resolved)) {
        throw new ToolError(`${path} is outside the workspace root ${this.root}`);
      }
      return resolved;
    }
  }

  /** One pass of `resolve`: the path, or `APPEARED`. */
  async resolveOnce(path, allowMissing) {
    let existing = posix.resolve(this.root, path);
    const rest = [];
    let canonical;
    for (;;) {
      try {
        canonical = fileURLToPath(
          (await this.connection.call("fs/canonicalize", { path: url(existing) })).path,
        );
        break;
      } catch (error) {
        if (!missing(error)) throw error;
        if (!allowMissing || existing === "/") {
          throw new ToolError(`No such file or directory: ${path}`);
        }
        rest.unshift(posix.basename(existing));
        existing = posix.dirname(existing);
      }
    }
    if (rest.length && (await this.refuseDangling(canonical, rest[0], path))) return APPEARED;
    return posix.join(canonical, ...rest);
  }

  /**
   * Refuse a write through a dangling symlink: the first missing component
   * of a path, under its existing canonical parent, may be a symlink whose
   * target cannot be canonicalized, so nothing shows it stays inside the
   * root and creating through it could write anywhere. It is refused only
   * once identified: no-follow metadata reporting a symlink, or (when the
   * metadata follows links and reports nothing) its name in the parent's
   * listing. A write needs positive evidence of absence: a parent that
   * cannot be listed refuses it. Returns `true` when the component
   * now exists as a file or directory (it appeared meanwhile): the caller
   * resolves again, so containment is checked on its canonical path.
   */
  async refuseDangling(parent, name, path) {
    const component = posix.join(parent, name);
    const refuse = () => {
      throw new ToolError(
        `${path} passes through ${component}, a symlink whose target does not exist; refused`,
      );
    };
    let metadata;
    try {
      metadata = await this.connection.call("fs/getMetadata", { path: url(component) });
    } catch (error) {
      if (!missing(error)) throw error;
    }
    if (metadata?.isSymlink) refuse();
    if (metadata) return true;
    // Metadata that follows links reports a dangling one as missing too, so
    // absence is proven only by the parent's listing.
    let listing;
    try {
      listing = await this.connection.call("fs/readDirectory", { path: url(parent) });
    } catch (error) {
      if (lost(error)) throw error;
      throw new ToolError(
        `${path} cannot be created: ${parent} cannot be listed to confirm ${name} is absent (it may be a dangling symlink); refused`,
      );
    }
    // Compared case-insensitively: on a case-folding filesystem a dangling
    // link listed with other casing would resolve to the requested name.
    const folded = name.toLowerCase();
    if (
      (listing?.entries ?? []).some(
        (entry) => typeof entry?.fileName === "string" && entry.fileName.toLowerCase() === folded,
      )
    ) {
      refuse();
    }
    return false;
  }

  /**
   * The whole content of a file, or undefined when it does not exist. A
   * caller that has just read its `metadata` passes it, saving a round trip.
   */
  async current(file, metadata, { beforeSend } = {}) {
    const callOpts = beforeSend ? { beforeSend } : undefined;
    try {
      metadata ??= await this.connection.call("fs/getMetadata", { path: url(file) }, callOpts);
    } catch (error) {
      if (missing(error)) {
        this.recordFileObservation(file, null);
        return undefined;
      }
      throw error;
    }
    if (!metadata.isFile) throw new ToolError(`Not a regular file: ${file}`);
    if (metadata.size > LIMITS.wholeFile) throw new ToolError("The file is larger than 8 MiB");
    let read;
    try {
      read = await this.connection.call("fs/readFile", { path: url(file) }, callOpts);
    } catch (error) {
      if (missing(error)) {
        this.recordFileObservation(file, null);
        return undefined;
      }
      throw error;
    }
    const buf = Buffer.from(read.dataBase64, "base64");
    this.recordFileObservation(file, sha256(buf));
    return buf;
  }

  recordMissingPath(session, path, resolvedFile = undefined) {
    const candidates = new Set();
    if (typeof resolvedFile === "string" && resolvedFile.length > 0) candidates.add(resolvedFile);
    if (typeof path === "string" && path.length > 0) {
      const normalized = posix.isAbsolute(path)
        ? posix.normalize(path)
        : this.root
          ? posix.resolve(this.root, path)
          : undefined;
      if (normalized) candidates.add(normalized);
    }
    for (const candidate of candidates) {
      if (this.fileObservation.has(candidate) || session?.observed?.has(candidate)) {
        this.recordFileObservation(candidate, null);
        session?.observed?.delete(candidate);
      }
    }
  }

  async readFile(session, { path, offset = 0, length: asked }) {
    // Only an omitted length defaults; `null` stays invalid.
    const length = asked === undefined ? LIMITS.readBytes : asked;
    if (!Number.isInteger(offset) || offset < 0) throw new ToolError("Invalid offset");
    if (!Number.isInteger(length) || length < 1 || length > LIMITS.readBytes) {
      throw new ToolError(`length must be between 1 and ${LIMITS.readBytes}`);
    }
    let file;
    try {
      file = await this.resolve(path, { outside: true });
    } catch (error) {
      if (missing(error) || /No such file/i.test(String(error?.message ?? ""))) {
        this.recordMissingPath(session, path);
      }
      throw error;
    }
    let metadata;
    try {
      metadata = await this.rpc("fs/getMetadata", { path: url(file) });
    } catch (error) {
      if (missing(error) || /No such file/i.test(String(error?.message ?? ""))) {
        this.recordMissingPath(session, path, file);
      }
      throw error;
    }
    if (!metadata.isFile) throw new ToolError(`Not a regular file: ${path}`);
    let size = metadata.size;
    let bytes;
    if (size <= LIMITS.wholeFile) {
      const data = await this.current(file, metadata);
      if (data === undefined) {
        this.recordMissingPath(session, path, file);
        throw new ToolError(`No such file: ${path}`);
      }
      const digest = sha256(data);
      session.observed.set(file, digest);
      this.recordFileObservation(file, digest);
      size = data.length;
      // A whole image the model can see, as Claude's Read shows one, for an
      // unpaged read; its type follows the name asked for (a symlink's
      // target may be named otherwise), its bytes the canonical file.
      const mimeType =
        offset === 0 && asked === undefined && size <= LIMITS.imageBytes
          ? imageType(path, data)
          : undefined;
      if (mimeType) {
        return {
          content: [
            { type: "text", text: `Image ${file} (${mimeType}, ${size} bytes)` },
            { type: "image", data: data.toString("base64"), mimeType },
          ],
        };
      }
      bytes = data.subarray(offset, offset + length);
    } else {
      bytes = await this.readBlock(file, offset, length);
      this.recordFileObservation(file, undefined);
    }
    const end = offset + bytes.length;
    const body = bytes.includes(0) ? `[binary content, ${bytes.length} bytes]` : bytes.toString();
    const more =
      offset > 0 || end < size
        ? `\n[cowboy: bytes ${offset}-${end} of ${size}${end < size ? `; continue with offset=${end}` : ""}]`
        : "";
    return body + more;
  }

  async readBlock(file, offset, length) {
    const handleId = `cowboy-${randomUUID()}`;
    await this.rpc("fs/open", { handleId, path: url(file) });
    try {
      const block = await this.connection.call("fs/readBlock", { handleId, offset, len: length });
      return Buffer.from(block.chunk, "base64");
    } finally {
      await this.connection.call("fs/close", { handleId }).catch(() => {});
    }
  }

  async listDirectory({ path }) {
    const directory = await this.resolve(path, { outside: true });
    const listing = await this.rpc("fs/readDirectory", { path: url(directory) });
    const entries = (listing.entries ?? [])
      .filter((entry) => typeof entry?.fileName === "string")
      .map((entry) => (entry.isDirectory ? `${entry.fileName}/` : entry.fileName))
      .sort();
    const shown = entries.slice(0, LIMITS.listEntries);
    const omitted = entries.length - shown.length;
    return (
      `${directory}:\n${shown.join("\n")}` +
      (omitted ? `\n[cowboy: truncated; ${omitted} more entries]` : "")
    );
  }

  async searchFiles(turn, { pattern, path, glob, ignore_case: ignoreCase }) {
    // Only a glob: list the matching file paths (find by name), as Claude's
    // Glob does. A pattern searches contents.
    const names = pattern === undefined;
    if (!names && (typeof pattern !== "string" || !pattern || pattern.length > 4096)) {
      throw new ToolError("Invalid pattern");
    }
    if (glob !== undefined && (typeof glob !== "string" || !glob || glob.length > 1024)) {
      throw new ToolError("Invalid glob");
    }
    if (names && glob === undefined) throw new ToolError("Give pattern, glob or both");
    const target = await this.resolve(path, { outside: true });
    if (names) {
      // ripgrep lists an explicitly named file whatever the glob says, so a
      // name search needs a directory.
      const metadata = await this.rpc("fs/getMetadata", { path: url(target) });
      if (!metadata?.isDirectory) {
        throw new ToolError("To find files by name, give a directory as path");
      }
    }
    // Inside the root, matches are named relative to it (the command's cwd).
    const searched = inside(this.root, target) ? posix.relative(this.root, target) || "." : target;
    const argv = names
      ? ["rg", "--files", "--color=never"]
      : ["rg", "--line-number", "--no-heading", "--with-filename", "--color=never"];
    if (!names) argv.push("--max-columns=1000", "--max-columns-preview");
    if (ignoreCase === true) argv.push(names ? "--glob-case-insensitive" : "--ignore-case");
    if (glob !== undefined) argv.push("--glob", glob);
    // The root is the command's cwd: listed without a path, names carry no "./".
    if (names) argv.push(...(searched === "." ? [] : ["--", searched]));
    else argv.push("-e", pattern, "--", searched);
    const result = await this.runBounded(turn, argv, {
      timeoutMs: LIMITS.searchTimeoutMs,
      maxBytes: LIMITS.searchBytes,
      maxLines: LIMITS.searchMatches,
    });
    if (result.forgotten) {
      throw new ToolError(
        "The search ended while the connection to the workspace Machine was interrupted and its output is unavailable; run it again.",
      );
    }
    if (result.exitCode === 1 && !result.stdout) return "No matches.";
    if (result.exitCode === 127) throw new ToolError("ripgrep (rg) is not installed on the target");
    const lines = result.stdout.split("\n").filter(Boolean);
    const shown = lines.slice(0, LIMITS.searchMatches).join("\n");
    const notes = [];
    if (result.truncated || lines.length > LIMITS.searchMatches) notes.push("results truncated");
    if (result.timedOut) notes.push("search stopped after 30 s");
    if (result.exitCode > 1 && result.stderr) notes.push(result.stderr.trim().slice(0, 2000));
    return shown + (notes.length ? `\n[cowboy: ${notes.join("; ")}]` : "");
  }

  async writeFile(session, turn, { path, content }, { decision, signal } = {}) {
    if (typeof content !== "string") throw new ToolError("content must be a string");
    const bytes = Buffer.from(content);
    if (bytes.length > LIMITS.writeBytes) throw new ToolError("content is larger than 3 MiB");
    const file = await this.resolve(path, { allowMissing: true });
    // H-Proxy-1: Capture observed digest and invalidation generation BEFORE queuing
    // on `fileLocks` so a second queued write can never silently overwrite the first.
    const queuedObserved = session.observed.get(file);
    const queuedInvalidationGen = this.invalidations.get(file) ?? 0;
    const fileCheck = { file, path, queuedObserved, queuedInvalidationGen };
    const beforeSend = () =>
      this.assertAuthorizedBeforeEffect(session, turn, decision, signal, fileCheck);
    return await this.withFileLock(file, async () => {
      beforeSend();
      const current = await this.current(file, undefined, { beforeSend });
      if (current && queuedObserved === undefined) {
        this.recordFileObservation(file, sha256(current));
        throw new ToolError(`${path} exists; read it with ReadFile before replacing it.`);
      }
      const base = current ? sha256(current) : "absent";
      if (queuedObserved !== undefined && queuedObserved !== base) {
        this.recordFileObservation(file, current ? base : undefined);
        throw new ToolError(`${path} changed since it was read; read it again first.`);
      }
      const published = await this.publish(session, turn, file, current, bytes, { beforeSend });
      const text = published.verified
        ? `${current ? "Replaced" : "Created"} ${file}: ${VERIFIED}`
        : `${current ? "Replaced" : "Created"} ${file} (${bytes.length} bytes)`;
      return { content: [{ type: "text", text }], _postEditMeta: published.postEdit };
    });
  }

  async editFile(session, turn, args, { decision, signal } = {}) {
    const { path, old_string: before, new_string: after, replace_all: all = false } = args;
    if (typeof before !== "string" || !before || typeof after !== "string" || before === after) {
      throw new ToolError("old_string must be non-empty and differ from new_string");
    }
    const file = await this.resolve(path);
    const queuedObserved = session.observed.get(file);
    const queuedInvalidationGen = this.invalidations.get(file) ?? 0;
    const fileCheck = { file, path, queuedObserved, queuedInvalidationGen };
    const beforeSend = () =>
      this.assertAuthorizedBeforeEffect(session, turn, decision, signal, fileCheck);
    return await this.withFileLock(file, async () => {
      beforeSend();
      const current = await this.current(file, undefined, { beforeSend });
      if (!current) {
        this.recordFileObservation(file, undefined);
        throw new ToolError(`No such file: ${path}`);
      }
      const base = sha256(current);
      if (queuedObserved === undefined) {
        this.recordFileObservation(file, base);
        throw new ToolError(`Read ${path} with ReadFile before editing it.`);
      }
      if (queuedObserved !== base) {
        this.recordFileObservation(file, base);
        throw new ToolError(`${path} changed since it was read; read it again.`);
      }
      // Editing as text must never rewrite bytes that are not valid UTF-8.
      const text = decodeText(current);
      if (text === undefined) throw new ToolError(`${path} is not valid UTF-8 text; edit refused`);
      let count = 0;
      for (let at = text.indexOf(before); at >= 0; at = text.indexOf(before, at + before.length)) {
        count++;
      }
      if (count === 0) throw new ToolError("old_string was not found");
      if (count > 1 && all !== true) {
        throw new ToolError(`old_string occurs ${count} times; make it unique or set replace_all`);
      }
      // The result's size is known before anything is allocated for it.
      const replaced = all === true ? count : 1;
      const size =
        current.length + replaced * (Buffer.byteLength(after) - Buffer.byteLength(before));
      if (size > LIMITS.writeBytes) throw new ToolError("the result is larger than 3 MiB");
      const next = Buffer.from(
        all === true ? text.replaceAll(before, () => after) : text.replace(before, () => after),
      );
      const published = await this.publish(session, turn, file, current, next, { beforeSend });
      const summary = published.verified
        ? `Edited ${file}: ${VERIFIED}`
        : `Edited ${file} (${replaced} replacement${replaced === 1 ? "" : "s"})`;
      return { content: [{ type: "text", text: summary }], _postEditMeta: published.postEdit };
    });
  }

  /**
   * Publish `bytes` to `file` only while it still holds `base` (its bytes,
   * or `undefined` for a new file), through the execution helper's
   * conditional `apply`: the original stays in place until the replacement
   * is, and a failure leaves the file unchanged or exactly at the result.
   */
  async publish(session, turn, file, base, bytes, { beforeSend } = {}) {
    const generation = this.invalidations.get(file) ?? 0;
    const writeGen = (this.writeDispatchGen.get(file) ?? 0) + 1;
    this.writeDispatchGen.set(file, writeGen);
    this.activeWrites.set(file, (this.activeWrites.get(file) ?? 0) + 1);
    try {
      const verified = await this.publishOnce(turn, file, base, bytes, { beforeSend });
      // Checked and written in one step: a publish of the file that became
      // uncertain meanwhile is never undone by this one.
      let postEdit = null;
      if ((this.invalidations.get(file) ?? 0) === generation) {
        const rawDigest = sha256(bytes);
        session.observed.set(file, rawDigest);
        const obs = this.recordFileObservation(file, rawDigest, { forceAdvance: true });
        const rev = (this.committedRevision.get(file) ?? 0) + 1;
        this.committedRevision.set(file, rev);
        const relPath = this.relativeWorkspacePath(file);
        const expectedLfSha256 = lfSha256(bytes);
        if (relPath !== null && expectedLfSha256 !== null) {
          postEdit = {
            file,
            relPath,
            expectedLfSha256,
            rawDigest,
            rev,
            obsGen: obs.gen,
            writeGen,
            invalidationGen: generation,
          };
        }
      }
      return { verified, postEdit };
    } catch (error) {
      // Its outcome is unknown: no session of this facade (the parent or a
      // subagent) may edit the file before reading it again.
      if (error instanceof UncertainEffect) {
        this.invalidations.set(file, (this.invalidations.get(file) ?? 0) + 1);
        this.committedRevision.set(file, (this.committedRevision.get(file) ?? 0) + 1);
        this.recordFileObservation(file, undefined);
        for (const each of this.sessions.values()) each.observed.delete(file);
      }
      throw error;
    } finally {
      const left = (this.activeWrites.get(file) ?? 1) - 1;
      if (left <= 0) this.activeWrites.delete(file);
      else this.activeWrites.set(file, left);
    }
  }

  /** `publish`; `true` when only the file's bytes, not the helper's own
   * report, show the result. */
  async publishOnce(turn, file, base, bytes, { beforeSend } = {}) {
    const expect = sha256(bytes);
    const request = JSON.stringify({
      schema: 1,
      path: file,
      base: base === undefined ? "absent" : sha256(base),
      expect,
      splices: splice(base ?? Buffer.alloc(0), bytes),
      staleTempSeconds: 3600,
    });
    const requestBytes = Buffer.from(request, "utf8");
    let argv;
    let stdinPayload;
    if (requestBytes.length <= INLINE_APPLY) {
      argv = ["/bin/sh", "-c", APPLY, "sh", "--request", requestBytes.toString("base64")];
    } else {
      if (requestBytes.length > MAX_STDIN_APPLY_BYTES) {
        throw new ToolError("Edit request exceeds helper limit");
      }
      argv = ["/bin/sh", "-c", APPLY_STDIN, "sh", String(requestBytes.length)];
      stdinPayload = requestBytes;
    }
    const result = await this.runBounded(turn, argv, {
      timeoutMs: this.publishTimeoutMs,
      maxBytes: 64 * 1024,
      lostIsUncertain: true,
      pipeStdin: stdinPayload !== undefined,
      stdinPayload,
      beforeSend,
    });
    let outcome;
    try {
      outcome = JSON.parse(result.stdout);
    } catch {
      outcome = undefined;
    }
    if (result.truncated) {
      // Output beyond the helper's bound: its report cannot be trusted.
      throw new UncertainEffect("file publish");
    }
    if (outcome?.status === "applied") return false;
    if (outcome?.status === "conflict") {
      this.recordFileObservation(file, undefined);
      throw new ToolError("The file changed on the target since it was read; read it again.");
    }
    if (result.stdinTimedOutBeforeDispatch) {
      // Stdin was never dispatched and the helper was confirmed stopped:
      // nothing was written, so fail cleanly without parking the session.
      throw new ToolError(
        "Timed out before delivering the edit request to the workspace helper; nothing was written.",
      );
    }
    if (result.timedOut) {
      // The helper may still publish: its outcome is unknown.
      throw new UncertainEffect("file publish");
    }
    if (result.forgotten) {
      // The helper ended unseen (E1) and its report is lost; it can never
      // publish now. Only the requested bytes on the target settle it.
      const now = await this.current(file).catch(() => null);
      if (now && sha256(now) === expect) return true;
      throw new UncertainEffect("file publish");
    }
    if (result.exitCode === 127) {
      // No helper ran (the executor exports none): nothing was written and
      // APPLY_STDIN never staged a persistent payload file.
      throw new ToolError(
        "The workspace Machine's execution helper is unavailable; nothing was written.",
      );
    }
    // A helper error leaves the file unchanged or exactly at the result;
    // read it again to tell which.
    const now = await this.current(file).catch(() => null);
    if (now === null) throw new UncertainEffect("file publish");
    if (now !== undefined && sha256(now) === expect) return false;
    throw new ToolError(
      `Writing ${file} failed: ${String(outcome?.error ?? result.stderr.trim()).slice(0, 500)}`,
    );
  }

  /**
   * Start `argv` on the target. A `readiness` probe (the projection's Git
   * state) runs outside any turn: the worker's ledger admits and tracks it
   * by its id prefix, one at a time.
   */
  async start(turn, argv, { readiness = false, pipeStdin = false, beforeSend } = {}) {
    const processId = `${readiness ? READINESS_PREFIX : "cowboy-agy-"}${randomUUID()}`;
    const result = await this.mutate(
      turn,
      "process/start",
      {
        processId,
        argv,
        cwd: url(this.root),
        env: {},
        tty: false,
        pipeStdin: Boolean(pipeStdin),
        arg0: null,
        envPolicy: ENV_POLICY,
      },
      { beforeSend },
    );
    if (result?.processId !== processId) throw new ToolError("Target process identity changed");
    return { processId, afterSeq: null, exitCode: null, closed: false };
  }

  /**
   * One `process/read`; returns the chunks and updates the cursor. Reads of
   * one process run one at a time, so a stop's drain and a waiting reader
   * never share a cursor, and a command's chunks land in its retained
   * output whichever reader took them.
   */
  read(process, waitMs, options) {
    const read = (process.reading ?? Promise.resolve()).then(() =>
      this.readOnce(process, waitMs, options),
    );
    process.reading = read.catch(() => {});
    return read;
  }

  async readOnce(process, waitMs, { lostIsUncertain = true, deadline } = {}) {
    // Checked when this read's turn in the chain comes: a reader ahead of
    // it may have confirmed the close or used the caller's remaining time.
    if (process.closed) return [];
    if (deadline !== undefined) {
      const remaining = deadline - Date.now();
      if (remaining <= 0) return [];
      waitMs = Math.min(waitMs, remaining);
    }
    let result;
    try {
      result = await this.connection.call("process/read", {
        processId: process.processId,
        afterSeq: process.afterSeq,
        maxBytes: 65536,
        waitMs,
      });
    } catch (error) {
      if (forgotten(error, process.processId)) {
        // E1: the executor forgot it, so it ended; its exit status and any
        // unread output are lost. Nothing more is sent for it.
        process.closed = true;
        process.forgotten = true;
        process.wake?.();
        return [];
      }
      // A started process whose state cannot be read, because the reply was
      // lost or the connection could not be opened again, has an unknown
      // outcome: a quiet reader waits with no read in flight, so a close
      // reaches it as a reconnect failure rather than a lost reply.
      if (lostIsUncertain && (unknownOutcome(error) || unreachable(error))) {
        throw new UncertainEffect("process/read");
      }
      throw error;
    }
    const chunks = [];
    for (const chunk of result.chunks ?? []) {
      if (process.afterSeq !== null && chunk.seq <= process.afterSeq) {
        throw new ToolError("Repeated process output");
      }
      process.afterSeq = chunk.seq;
      const bytes = Buffer.from(chunk.chunk, "base64");
      process.output?.push(bytes);
      chunks.push({ stream: chunk.stream, bytes });
    }
    if (result.exited) process.exitCode = result.exitCode;
    if (result.closed) {
      process.closed = true;
      // Another reader (StopCommand, expiry, cleanup) confirmed the close:
      // a quiet reader waiting on this process ends its wait.
      process.wake?.();
    }
    return chunks;
  }

  /**
   * Read `process` once within `remaining` ms: a one-second read while it
   * produces output, and after `LIMITS.quietReads` empty reads in a row a
   * read that waits for a notification naming the process instead. A quiet
   * wait ends one second before the deadline, so a one-second read still
   * sees an exit whose notification was lost.
   */
  async poll(process, remaining, options) {
    const listeners = this.connection.listeners;
    const deadline = Date.now() + remaining;
    options = { ...options, deadline };
    const chunks =
      (process.quiet ?? 0) >= LIMITS.quietReads && listeners && remaining > 2000
        ? await this.notifiedRead(
            process,
            Math.min(LIMITS.quietWaitMs, remaining - 1000),
            deadline,
            options,
          )
        : await this.read(process, Math.max(1, Math.min(1000, remaining)), options);
    process.quiet = chunks.length || process.closed ? 0 : (process.quiet ?? 0) + 1;
    return chunks;
  }

  /**
   * Read without waiting; with nothing new, wait up to `waitMs` for an
   * executor notification naming the process (or the connection's close,
   * or another reader confirming its close) and read once more. It listens
   * before reading, so output arriving in between still ends the wait, and
   * no read is in flight while it waits. A wait that ends with no
   * notification returns nothing: the caller reads first on its next poll
   * if its deadline leaves time, so a lost notification costs one read per
   * `waitMs` and a reader never starts a read past its deadline.
   */
  async notifiedRead(process, waitMs, deadline, options) {
    const listeners = this.connection.listeners;
    let listener;
    let timer;
    const notified = new Promise((resolve) => {
      listener = (frame) => {
        if (frame.method === "closed" || frame.params?.processId === process.processId) {
          resolve(true);
        }
      };
      process.wake = () => resolve(true);
      timer = setTimeout(() => resolve(false), waitMs);
      listeners.add(listener);
    });
    try {
      const chunks = await this.read(process, 0, options);
      if (chunks.length || process.closed) return chunks;
      if (!(await notified) || process.closed || Date.now() >= deadline) return [];
      return await this.read(process, 0, options);
    } finally {
      clearTimeout(timer);
      listeners.delete(listener);
      process.wake = undefined;
    }
  }

  /** Terminate `process`; `true` only once the target confirmed it closed
   * within `confirmMs`. */
  async terminate(process, confirmMs = 5000) {
    if (process.closed) return true;
    // One stop per process: after a stop that may have been delivered (its
    // reply lost included), later attempts only observe, so a process
    // handling the first signal never receives a second.
    // One monotonic budget covers the stop's own reply, waiting behind
    // another reader, reconnects and the confirming reads: when it ends the
    // close is unconfirmed (`false`), and a request still in flight finishes
    // on its own.
    const deadline = Date.now() + confirmMs;
    // One stop per process, shared by every caller: a later caller joins the
    // stop in flight (so nothing that settles this process leaves its reply
    // outstanding) and never sends a second signal to a process handling the
    // first. Only a stop refused before it was sent (no connection, or no
    // request slot) is sent again later; a reply lost after sending keeps it.
    if (!process.stopping) {
      process.stopping = this.connection
        .call("process/terminate", { processId: process.processId })
        .catch((error) => {
          // Callers joined on this stop see its failure; a later caller only
          // observes (the stop may have been delivered) or, if it was never
          // sent, sends it.
          process.stopping = notSent(error) ? undefined : Promise.resolve();
          throw error;
        });
      process.stopping.catch(() => {});
    }
    let stopTimer;
    const stopExpired = new Promise((resolve) => {
      stopTimer = setTimeout(() => resolve(false), Math.max(0, deadline - Date.now()));
    });
    try {
      if ((await Promise.race([process.stopping.then(() => true), stopExpired])) === false) {
        return process.closed;
      }
    } finally {
      clearTimeout(stopTimer);
    }
    while (!process.closed) {
      const remaining = deadline - Date.now();
      if (remaining <= 0) break;
      let timer;
      const expired = new Promise((resolve) => {
        timer = setTimeout(resolve, remaining);
      });
      try {
        await Promise.race([
          // Not sent at all once the deadline passed while it waited in
          // the reader chain.
          this.read(process, Math.min(500, remaining), { lostIsUncertain: false, deadline }),
          expired,
        ]);
      } finally {
        clearTimeout(timer);
      }
    }
    return process.closed;
  }

  /** Run a process to its end within bounds; stdout and stderr apart. */
  async runBounded(turn, argv, options) {
    // The turn's end waits for the process to finish (a publish helper is an
    // admitted effect like any request), within the call's own timeout.
    const run = this.runUntracked(turn, argv, options);
    const tracked = turn?.inflight ? run.catch(() => {}) : undefined;
    if (tracked) turn.inflight.add(tracked);
    try {
      return await run;
    } finally {
      if (tracked) turn.inflight.delete(tracked);
    }
  }

  async runUntracked(
    turn,
    argv,
    {
      timeoutMs,
      maxBytes,
      maxLines,
      lostIsUncertain = true,
      readiness = false,
      pipeStdin = false,
      stdinPayload = undefined,
      beforeSend,
    },
  ) {
    const process = await this.start(turn, argv, {
      readiness,
      pipeStdin: Boolean(pipeStdin || stdinPayload !== undefined),
      beforeSend,
    });
    const out = { stdout: [], stderr: [], size: 0, lines: 0 };
    const deadline = Date.now() + timeoutMs;
    let truncated = false;
    let timedOut = false;
    let stdinTimedOutBeforeDispatch = false;
    let failure;
    try {
      let stdinDelivered = stdinPayload === undefined;
      if (stdinPayload !== undefined) {
        const chunk = Buffer.concat([stdinPayload, Buffer.from("\n")]).toString("base64");
        const writeId = randomUUID();
        const deadlineSentinel = Symbol("stdin-write-deadline");
        for (;;) {
          const remaining = deadline - Date.now();
          if (remaining <= 0) {
            stdinTimedOutBeforeDispatch = true;
            break;
          }
          let dispatched = false;
          let preSendExpired = false;
          let writeClosed = false;
          const writeBeforeSend = () => {
            if (typeof beforeSend === "function") beforeSend();
            if (writeClosed || Date.now() >= deadline) {
              preSendExpired = true;
              throw new ToolError("Timed out before delivering apply request to workspace helper.");
            }
            dispatched = true;
          };
          let reply;
          let timer;
          try {
            const writeCall = this.mutate(
              turn,
              "process/write",
              { processId: process.processId, chunk, writeId },
              { beforeSend: writeBeforeSend, timeoutMs: Math.max(1, Math.ceil(remaining)) },
            );
            writeCall.catch(() => {});
            const expired = new Promise((resolve) => {
              timer = setTimeout(
                () => resolve(deadlineSentinel),
                Math.max(1, Math.ceil(remaining)),
              );
            });
            const raced = await Promise.race([writeCall, expired]);
            if (raced === deadlineSentinel) {
              writeClosed = true;
              if (dispatched) {
                turn.uncertain ??= "process/write";
                throw new UncertainEffect("process/write");
              }
              stdinTimedOutBeforeDispatch = true;
              break;
            }
            reply = raced;
          } catch (error) {
            if (preSendExpired && !dispatched) {
              writeClosed = true;
              stdinTimedOutBeforeDispatch = true;
              break;
            }
            if (!dispatched && notSent(error)) {
              writeClosed = true;
              throw new ToolError(
                "Execution unavailable before delivering the edit request to the workspace helper; nothing was written.",
              );
            }
            if (dispatched && !notSent(error) && error?.message === "Target request timed out") {
              turn.uncertain ??= "process/write";
              throw new UncertainEffect("process/write");
            }
            // If the target executor itself reports the process already closed
            // (e.g. exit 127 when the helper is absent), poll to observe its
            // exit status rather than masking it. Local fence refusals and
            // unknown outcomes always fail closed.
            if (
              error?.remote !== undefined &&
              error.remote.code !== -32000 &&
              !unknownOutcome(error)
            ) {
              break;
            }
            throw error;
          } finally {
            clearTimeout(timer);
          }
          if (reply?.status === "accepted") {
            stdinDelivered = true;
            break;
          }
          if (reply?.status === "starting") {
            const pause = Math.min(100, deadline - Date.now());
            if (pause <= 0) {
              stdinTimedOutBeforeDispatch = true;
              break;
            }
            await new Promise((resolve) => setTimeout(resolve, pause));
            continue;
          }
          break;
        }
      }
      const pollDeadline = stdinDelivered ? deadline : Math.min(deadline, Date.now() + 5000);
      while (!process.closed && !timedOut && !stdinTimedOutBeforeDispatch) {
        const remaining = pollDeadline - Date.now();
        if (remaining <= 0) {
          if (stdinDelivered) {
            timedOut = true;
            break;
          }
          throw new ToolError("Failed to deliver apply request to workspace helper.");
        }
        for (const { stream, bytes } of await this.poll(process, remaining, {
          lostIsUncertain,
        })) {
          const room = Math.max(0, maxBytes - out.size);
          const kept = bytes.subarray(0, room);
          out.size += kept.length;
          (stream === "stderr" ? out.stderr : out.stdout).push(kept);
          if (stream !== "stderr") out.lines += kept.toString().split("\n").length - 1;
          if (kept.length < bytes.length || (maxLines && out.lines > maxLines)) truncated = true;
        }
        if (truncated) break;
      }
    } catch (error) {
      failure = error;
      // Fenced before cleanup is awaited: a concurrent call of the turn
      // admits no mutation while this helper's outcome is unknown.
      if (error instanceof UncertainEffect && turn) this.fenceLost(turn, error.method);
    }
    // Only the ordinary poll's E1 settles an outcome; during cleanup below it
    // merely confirms the close and never replaces a timeout or failure.
    const ended =
      failure === undefined &&
      !timedOut &&
      !stdinTimedOutBeforeDispatch &&
      !truncated &&
      process.forgotten === true;
    // Whatever ended the loop, the process is not left running unconfirmed: a
    // helper that may still publish is an effect of unknown outcome.
    if (!process.closed) {
      // A slow-exiting process tree is observed for a bounded while before
      // its outcome counts as unknown.
      const closed = await this.terminate(process, LIMITS.stopConfirmMs).catch(() => false);
      if (!closed && !readiness) throw new UncertainEffect("process/terminate");
    }
    if (failure !== undefined) throw failure;
    return {
      exitCode: process.exitCode,
      stdout: Buffer.concat(out.stdout).toString(),
      stderr: Buffer.concat(out.stderr).toString(),
      truncated,
      timedOut,
      stdinTimedOutBeforeDispatch,
      // The process ended unseen (E1): its output may be incomplete.
      forgotten: ended,
    };
  }

  async runCommand(session, turn, args, progress = () => {}) {
    const { command, command_id: id, timeout_seconds: timeout, wait_seconds: wait } = args;
    // Validated before anything starts: a bad argument starts no process.
    if (
      wait !== undefined &&
      (!Number.isInteger(wait) || wait < 0 || wait > LIMITS.commandMaxSeconds)
    ) {
      throw new ToolError(`wait_seconds must be between 0 and ${LIMITS.commandMaxSeconds}`);
    }
    let running;
    if (id !== undefined) {
      if (command !== undefined) throw new ToolError("Give either command or command_id");
      running = session.commands.get(id);
      if (!running) throw new ToolError(`No running command ${String(id)}`);
    } else {
      if (typeof command !== "string" || !command.trim() || command.length > 65536) {
        throw new ToolError("Invalid command");
      }
      const seconds = timeout ?? LIMITS.commandDefaultSeconds;
      if (!Number.isInteger(seconds) || seconds < 1 || seconds > LIMITS.commandMaxSeconds) {
        throw new ToolError(`timeout_seconds must be between 1 and ${LIMITS.commandMaxSeconds}`);
      }
      // From its start to its registration the command is part of the turn:
      // the turn's end waits for it, so its cleanup always sees the handle.
      const registering = (async () => {
        const started = await this.start(turn, this.commandArgv(command));
        started.deadline = Date.now() + seconds * 1000;
        // Every reader's chunks land here (see `read`).
        started.output = new Retained(LIMITS.commandOutput);
        session.commands.set(started.processId, started);
        // The deadline holds even when nobody reads the command again.
        started.timer = setTimeout(() => this.expire(session, started), seconds * 1000);
        started.timer.unref?.();
        return started;
      })();
      const tracked = registering.catch(() => {});
      turn.inflight?.add(tracked);
      try {
        running = await registering;
      } finally {
        turn.inflight?.delete(tracked);
      }
    }
    if (running.busy) throw new ToolError("Another call is reading this command");
    running.busy = true;
    try {
      const until = Math.min(running.deadline, Date.now() + (wait ?? Infinity) * 1000);
      // Output so far is shown while the command runs: a bounded tail, at
      // most every `progressMs` and only when it changed. No extra request
      // reaches the target; the reads are the ones the command needs anyway.
      let shown = Date.now();
      let changed = false;
      while (!running.closed && Date.now() < until) {
        for (const { bytes } of await this.poll(running, until - Date.now())) {
          changed ||= bytes.length > 0;
        }
        if (changed && !running.closed && Date.now() - shown >= LIMITS.progressMs) {
          shown = Date.now();
          changed = false;
          progress(running.output.peek(LIMITS.progressBytes));
        }
      }
      let status;
      if (running.closed) {
        status = `Exit code: ${running.exitCode ?? "unknown"}`;
        if (running.forgotten) {
          status +=
            " (the command's process ended while the connection to the workspace Machine was interrupted; its remaining output is unavailable)";
        }
      } else if (Date.now() >= running.deadline) {
        let stopped;
        try {
          stopped = await this.terminate(running);
        } catch (error) {
          if (unknownOutcome(error)) throw new UncertainEffect("process/terminate");
          throw error;
        }
        status = stopped
          ? "Timed out; the command was stopped."
          : `Timed out; stopping it is not yet confirmed (command_id: ${running.processId}).`;
      } else {
        // Each continuation costs the agent a whole turn; say how long one
        // may wait so a long gate is read in few, long waits.
        const left = Math.max(1, Math.ceil((running.deadline - Date.now()) / 1000));
        status =
          `Still running (command_id: ${running.processId}). ` +
          `Continue it with wait_seconds ${Math.min(left, LIMITS.commandMaxSeconds)} to wait for the rest of its ${left} s.`;
      }
      if (running.closed) this.forget(session, running);
      return `${status}\n${running.output.take()}`;
    } finally {
      running.busy = false;
    }
  }

  /**
   * The argv of one RunCommand. A login shell, so /etc/profile and the
   * user's profile set PATH (Nix profiles, user tools) as in the user's own
   * terminal and as Claude's login-shell snapshot does; a profile that
   * changes directory is undone before the command runs. A managed
   * (read-only) call, and a shell whose syntax is unknown, keeps a plain
   * `-c`. Internal helpers stay `/bin/sh`.
   */
  commandArgv(command) {
    const name = posix.basename(this.shell);
    if (this.managed) return [this.shell, "-c", command];
    // The `cd` is part of the script, not an `eval` around it: the shell
    // still execs the command's last simple command, so a stop reaches it.
    // Only builtins run before the command (a profile's `cd` function or
    // alias cannot replace them, and a minimal login PATH cannot break
    // them), and the physical directory is checked against the canonical
    // root before the command runs. A sentinel after `pwd -P` keeps the
    // path exact (a command substitution would strip trailing newlines).
    if (name === "fish") {
      return [
        this.shell,
        "-l",
        "-c",
        `builtin cd ${fishLiteral(this.root)}; or exit 1; builtin test (begin; builtin pwd -P; builtin printf x; end | builtin string collect) = ${fishLiteral(`${this.root}\nx`)}; or exit 1\n${command}`,
      ];
    }
    if (POSIX_SHELLS.has(name)) {
      // `builtin` in bash and zsh (zsh's `command` skips builtins); POSIX
      // `command` elsewhere, which finds a builtin before PATH. The leading
      // backslash keeps an alias from expanding.
      const b = name === "bash" || name === "zsh" ? "\\builtin" : "\\command";
      return [
        this.shell,
        "-l",
        "-c",
        `${b} cd -- ${posixLiteral(this.root)} || exit 1; ${b} test "$(${b} pwd -P; ${b} printf x)" = ${posixLiteral(`${this.root}\nx`)} || exit 1\n${command}`,
      ];
    }
    return [this.shell, "-c", command];
  }

  /**
   * A command's deadline passed: stop it. The timer is independent of
   * readers: while one is busy it is armed again, so the deadline holds even
   * when that reader returns before it.
   */
  async expire(session, running) {
    // The turn's cleanup owns the stop while it runs and re-arms the timer
    // itself when the close stays unconfirmed.
    if (
      this.closing ||
      running.closed ||
      running.cleaning > 0 ||
      session.commands.get(running.processId) !== running
    ) {
      return;
    }
    if (running.busy) {
      running.timer = setTimeout(() => this.expire(session, running), 1000);
      running.timer.unref?.();
      return;
    }
    running.busy = true;
    try {
      if (await this.terminate(running).catch(() => false)) this.forget(session, running);
      else if (!this.closing && !(running.cleaning > 0)) {
        // Not confirmed yet: try again; the handle stays until it is.
        running.timer = setTimeout(() => this.expire(session, running), 5000);
        running.timer.unref?.();
      }
    } finally {
      running.busy = false;
    }
  }

  async stopCommand(session, { command_id: id }) {
    const running = typeof id === "string" ? session.commands.get(id) : undefined;
    if (!running) throw new ToolError(`No running command ${String(id)}`);
    let closed;
    try {
      closed = await this.terminate(running);
    } catch (error) {
      if (unknownOutcome(error)) throw new UncertainEffect("process/terminate");
      throw error;
    }
    // The handle is kept until the target confirmed the process closed.
    if (!closed) return `Stopping ${id} is not yet confirmed; call StopCommand again.`;
    this.forget(session, running);
    return `Stopped ${id}.\n${running.output.take()}`;
  }

  /**
   * Start a subagent, or keep waiting for one, and return its report once it
   * ends. The turn stays open while this waits (the subagent's own tool calls
   * need it), so a wait never exceeds `LIMITS.subagentWaitSeconds`; a longer
   * task returns its id to wait for again.
   */
  async subagent(sessionId, turn, args) {
    const { prompt, description, subagent_id: id, wait_seconds: wait } = args;
    const host = this.subagents;
    if (host === undefined) throw new ToolError("Subagents are not available in this session.");
    if (
      wait !== undefined &&
      (!Number.isInteger(wait) || wait < 0 || wait > LIMITS.subagentWaitSeconds)
    ) {
      throw new ToolError(`wait_seconds must be between 0 and ${LIMITS.subagentWaitSeconds}`);
    }
    let childId = id;
    if (id !== undefined) {
      if (prompt !== undefined || description !== undefined) {
        throw new ToolError("Give either prompt or subagent_id");
      }
      if (typeof id !== "string" || !id) throw new ToolError("Invalid subagent_id");
    } else {
      if (typeof prompt !== "string" || !prompt.trim()) {
        throw new ToolError("prompt (or the subagent_id of a running subagent) is required");
      }
      if (Buffer.byteLength(prompt) > LIMITS.subagentPromptBytes) {
        throw new ToolError(`prompt is larger than ${LIMITS.subagentPromptBytes} bytes`);
      }
      if (description !== undefined && typeof description !== "string") {
        throw new ToolError("description must be text");
      }
      childId = await host.start(sessionId, { prompt, description });
    }
    const outcome = await host.wait(
      sessionId,
      childId,
      (wait ?? LIMITS.subagentWaitSeconds) * 1000,
      turn.abort?.signal,
    );
    return renderSubagent(childId, outcome);
  }

  async stopSubagent(sessionId, { subagent_id: id }) {
    const host = this.subagents;
    if (host === undefined) throw new ToolError("Subagents are not available in this session.");
    if (typeof id !== "string" || !id) throw new ToolError("Invalid subagent_id");
    return renderSubagent(id, await host.stop(sessionId, id));
  }
}

/**
 * What a subagent call tells the model: its report, or how to keep waiting.
 * A failed one is a tool error, so the model does not take its text for a
 * report.
 */
export function renderSubagent(id, outcome) {
  switch (outcome?.state) {
    case "running":
      return `Subagent ${id} is still running. Call Subagent with subagent_id "${id}" (and no prompt) to keep waiting, or StopSubagent to stop it. It is stopped when your turn ends.`;
    case "completed":
      return `Subagent ${id} finished.\n\n${outcome.report || "(it reported nothing)"}`;
    case "cancelled":
      return `Subagent ${id} was stopped${outcome.report ? `. What it had said:\n\n${outcome.report}` : "."}`;
    default:
      throw new ToolError(`Subagent ${id} failed: ${outcome?.error ?? "unknown error"}`);
  }
}

/**
 * Serve MCP (newline-delimited JSON-RPC) on one relay stream. `sessionOf`
 * maps the stream's channel to its ACP session when a tool is called.
 */
const RELAY_WRITE_TIMEOUT_MS = 10_000;

export function serveMcp(stream, facade, sessionOf) {
  const relayId = facade.registerRelay(sessionOf);
  const activeCalls = new Map();
  let streamEnded = false;
  const abortAllCalls = () => {
    for (const entry of activeCalls.values()) {
      entry.cancelled = true;
      entry.controller.abort();
    }
  };
  stream.once("end", () => {
    streamEnded = true;
    abortAllCalls();
  });
  stream.once("close", () => {
    streamEnded = true;
    abortAllCalls();
    facade.unregisterRelay(relayId);
  });
  stream.on("error", () => {
    streamEnded = true;
    abortAllCalls();
  });
  let buffered = "";
  stream.setEncoding("utf8");
  const send = (message) =>
    new Promise((resolve) => {
      if (stream.destroyed || stream.writableEnded || stream.writable === false) {
        return resolve(false);
      }
      let settled = false;
      const done = (ok) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        stream.off("close", onClose);
        stream.off("error", onError);
        resolve(ok);
      };
      const onClose = () => done(false);
      const onError = () => done(false);
      // R2-M15: Bound stalled relay writes so an unread pipe cannot hold ownership forever.
      const timer = setTimeout(() => {
        try {
          stream.destroy();
        } catch {
          // Ignore destroy errors.
        }
        done(false);
      }, RELAY_WRITE_TIMEOUT_MS);
      stream.once("close", onClose);
      stream.once("error", onError);
      try {
        stream.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`, (err) => done(!err));
      } catch {
        done(false);
      }
    });

  // R3-H7 & R2-H14: Every ID-bearing reply (including initialize, ping, tools/list,
  // tools/call, duplicate -32600, -32601, and -32603) flows through one per-callKey
  // ownership map with a synchronous `committed` latch and shared `sendPromise`.
  const commitFor = (entry, id, buildMessage) => {
    if (entry.committed || entry.suppressed) return Promise.resolve(false);
    entry.committed = true;
    let body;
    try {
      body = buildMessage(entry);
    } catch {
      body = { error: { code: -32603, message: "Internal error" } };
    }
    entry.sendPromise = send({ id, ...body });
    return entry.sendPromise;
  };

  const handle = async (message) => {
    const { id, method, params } = message;
    if (id === undefined || id === null) {
      if (
        (method === "notifications/cancelled" ||
          method === "$/cancel_request" ||
          method === "$/cancelRequest") &&
        params?.requestId !== undefined
      ) {
        const key = JSON.stringify(params.requestId);
        const active = activeCalls.get(key);
        if (active) {
          // R3-H7: Never delete activeCalls[key] on cancellation; mark cancelled & abort only.
          active.cancelled = true;
          active.controller.abort();
        }
      }
      return;
    }

    const callKey = JSON.stringify(id);
    if (activeCalls.has(callKey)) {
      const existing = activeCalls.get(callKey);
      existing.cancelled = true;
      existing.controller.abort();
      if (!existing.committed) {
        existing.committed = true;
        existing.suppressed = true;
        existing.sendPromise = send({
          id,
          error: { code: -32600, message: "Duplicate active MCP request ID" },
        });
      }
      return;
    }

    const entry = {
      controller: new AbortController(),
      cancelled: false,
      suppressed: false,
      committed: false,
      sendPromise: null,
    };
    activeCalls.set(callKey, entry);

    try {
      if (method === "initialize") {
        await commitFor(entry, id, () => ({
          result: {
            protocolVersion:
              typeof params?.protocolVersion === "string" ? params.protocolVersion : "2024-11-05",
            capabilities: { tools: {} },
            serverInfo: { name: "cowboy", version: "1.0.0" },
          },
        }));
        return;
      }
      if (method === "ping") {
        await commitFor(entry, id, () => ({ result: {} }));
        return;
      }
      if (method === "tools/list") {
        await commitFor(entry, id, () => ({ result: { tools: facade.tools() } }));
        return;
      }
      if (method === "tools/call") {
        if (streamEnded || stream.destroyed || stream.writableEnded) {
          return;
        }
        if (facade.closing) {
          await commitFor(entry, id, () => ({
            result: {
              isError: true,
              content: [{ type: "text", text: "Antigravity proxy is shutting down" }],
            },
          }));
          return;
        }
        const sid = sessionOf();
        const trackedTurn = facade.beginCall(sid);
        let delivered = false;
        try {
          try {
            delivered = await facade.callForMcp(sid, params?.name, params?.arguments, relayId, {
              signal: entry.controller.signal,
              commitReply: (buildMessage) => commitFor(entry, id, buildMessage),
            });
          } catch {
            delivered = await commitFor(entry, id, () => ({
              error: { code: -32603, message: "Internal error" },
            }));
          }
        } finally {
          if (!delivered) {
            facade.markRelayAmbiguous(sid, relayId);
          }
          facade.endCall(trackedTurn);
        }
        return;
      }
      await commitFor(entry, id, () => ({
        error: { code: -32601, message: "Method not found" },
      }));
    } catch {
      await commitFor(entry, id, () => ({
        error: { code: -32603, message: "Internal error" },
      }));
    } finally {
      if (entry.sendPromise) await entry.sendPromise.catch(() => {});
      if (activeCalls.get(callKey) === entry) activeCalls.delete(callKey);
    }
  };
  stream.on("data", (chunk) => {
    buffered += chunk;
    if (buffered.length > 16 * 1024 * 1024) return stream.destroy();
    let newline;
    while ((newline = buffered.indexOf("\n")) >= 0) {
      const line = buffered.slice(0, newline);
      buffered = buffered.slice(newline + 1);
      if (!line.trim()) continue;
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        stream.destroy();
        return;
      }
      handle(message).catch(() => {});
    }
  });
}
