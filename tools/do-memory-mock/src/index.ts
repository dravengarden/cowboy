/**
 * Extreme Durable Object memory mock for Cowboy's compact control plane.
 *
 * Layers (do not collapse these numbers):
 *   storage  — SQLite payload bytes, not loaded into the isolate
 *   focused  — JS heap for busy/focused hot tails + one shared live frame
 *   all-hot  — JS heap if every session hot tail is materialized
 *   all-rows — JS heap if archive history is also parsed (the anti-pattern)
 *
 * POST /seed accepts the Hub fixture. Archive rows are synthesized inside
 * SQLite so the HTTP body does not carry megabytes of padding.
 */

export interface Env {
  HUB: DurableObjectNamespace;
}

const ISOLATE_BUDGET = 128 * 1024 * 1024;
const TARGET_BUDGET = 100 * 1024 * 1024;
const IDLE_TAIL_BUDGET = 512 * 1024;
const BUSY_TAIL_BUDGET = 1024 * 1024;

type SeedSession = {
  id: string;
  status: string;
  focused?: boolean;
  hotTail: unknown[];
  hotTailBytes?: number;
};

type SeedFixture = {
  sessions?: SeedSession[];
  liveFrames?: string[];
  focusedSessionIds?: string[];
  rawFanoutWouldHaveBeen?: number;
  archiveRows?: number;
  archivePayloadBytes?: number;
};

function jsonBytes(value: unknown): number {
  return new TextEncoder().encode(JSON.stringify(value)).length;
}

export class CowboyHubMock {
  constructor(
    private readonly ctx: DurableObjectState,
    _env: Env,
  ) {}

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    this.ensureSchema();
    try {
      if (url.pathname === "/reset" && request.method !== "GET") {
        this.reset();
        return Response.json({ ok: true });
      }
      if (url.pathname === "/seed" && request.method === "POST") {
        const fixture = (await request.json()) as SeedFixture;
        this.reset();
        this.seedFromFixture(fixture);
        const mode = url.searchParams.get("mode") ?? "focused";
        return Response.json(this.report(mode));
      }
      if (this.sessionCount() === 0) {
        return Response.json({ error: "empty; POST /seed first" }, { status: 409 });
      }
      return Response.json(this.report(url.searchParams.get("mode") ?? "focused"));
    } catch (error) {
      return Response.json(
        {
          error: error instanceof Error ? error.message : String(error),
          isolateBudgetBytes: ISOLATE_BUDGET,
        },
        { status: 500 },
      );
    }
  }

  private ensureSchema(): void {
    this.ctx.storage.sql.exec(`
      CREATE TABLE IF NOT EXISTS sessions (
        id TEXT PRIMARY KEY,
        status TEXT NOT NULL,
        focused INTEGER NOT NULL DEFAULT 0
      );
    `);
    this.ctx.storage.sql.exec(`
      CREATE TABLE IF NOT EXISTS events (
        session_id TEXT NOT NULL,
        seq INTEGER NOT NULL,
        payload TEXT NOT NULL,
        hot INTEGER NOT NULL DEFAULT 1,
        PRIMARY KEY (session_id, seq)
      );
    `);
    this.ctx.storage.sql.exec(`
      CREATE TABLE IF NOT EXISTS live_frames (
        idx INTEGER PRIMARY KEY,
        json TEXT NOT NULL
      );
    `);
    this.ctx.storage.sql.exec(`
      CREATE TABLE IF NOT EXISTS meta (
        key TEXT PRIMARY KEY,
        value INTEGER NOT NULL
      );
    `);
  }

  private reset(): void {
    this.ctx.storage.sql.exec("DELETE FROM events");
    this.ctx.storage.sql.exec("DELETE FROM sessions");
    this.ctx.storage.sql.exec("DELETE FROM live_frames");
    this.ctx.storage.sql.exec("DELETE FROM meta");
  }

  private sessionCount(): number {
    return Number(
      this.ctx.storage.sql.exec("SELECT count(*) AS n FROM sessions").one().n,
    );
  }

  private seedFromFixture(fixture: SeedFixture): void {
    const focusedIds = new Set(fixture.focusedSessionIds ?? []);
    for (const session of fixture.sessions ?? []) {
      const focused = session.focused === true || focusedIds.has(session.id) ? 1 : 0;
      this.ctx.storage.sql.exec(
        "INSERT INTO sessions (id, status, focused) VALUES (?, ?, ?)",
        session.id,
        session.status,
        focused,
      );
      for (const [index, envelope] of session.hotTail.entries()) {
        const payload = JSON.stringify(envelope);
        const seq =
          typeof envelope === "object" &&
          envelope !== null &&
          "seq" in envelope &&
          typeof envelope.seq === "number"
            ? envelope.seq
            : index;
        this.ctx.storage.sql.exec(
          "INSERT OR REPLACE INTO events (session_id, seq, payload, hot) VALUES (?, ?, ?, 1)",
          session.id,
          seq,
          payload,
        );
      }
    }
    for (const [index, json] of (fixture.liveFrames ?? []).entries()) {
      this.ctx.storage.sql.exec(
        "INSERT INTO live_frames (idx, json) VALUES (?, ?)",
        index,
        json,
      );
    }
    this.ctx.storage.sql.exec(
      "INSERT INTO meta (key, value) VALUES (?, ?)",
      "rawFanoutWouldHaveBeen",
      fixture.rawFanoutWouldHaveBeen ?? 0,
    );
    const archiveRows = Math.max(0, fixture.archiveRows ?? 0);
    const archiveBytes = Math.min(4096, Math.max(32, fixture.archivePayloadBytes ?? 512));
    const pad = "y".repeat(archiveBytes);
    const archiveSession =
      (fixture.sessions ?? []).map((session) => session.id)[0] ?? "session-0";
    const startSeq = 1_000_000;
    for (let index = 0; index < archiveRows; index += 1) {
      this.ctx.storage.sql.exec(
        "INSERT OR REPLACE INTO events (session_id, seq, payload, hot) VALUES (?, ?, ?, 0)",
        archiveSession,
        startSeq + index,
        JSON.stringify({ seq: startSeq + index, pad }),
      );
    }
    this.ctx.storage.sql.exec(
      "INSERT INTO meta (key, value) VALUES (?, ?)",
      "archiveRows",
      archiveRows,
    );
  }

  private report(mode: string): Record<string, unknown> {
    const started = Date.now();
    const sessionRows = this.ctx.storage.sql
      .exec("SELECT id, status, focused FROM sessions ORDER BY id")
      .toArray();
    const liveRows = this.ctx.storage.sql
      .exec("SELECT json FROM live_frames ORDER BY idx")
      .toArray()
      .map((row) => String(row.json));
    const counts = this.ctx.storage.sql
      .exec(
        `SELECT
           count(*) AS rows,
           coalesce(sum(length(payload)), 0) AS bytes,
           coalesce(sum(CASE WHEN hot = 1 THEN 1 ELSE 0 END), 0) AS hot_rows,
           coalesce(sum(CASE WHEN hot = 1 THEN length(payload) ELSE 0 END), 0) AS hot_bytes,
           coalesce(sum(CASE WHEN hot = 0 THEN 1 ELSE 0 END), 0) AS archive_rows,
           coalesce(sum(CASE WHEN hot = 0 THEN length(payload) ELSE 0 END), 0) AS archive_bytes
         FROM events`,
      )
      .one();
    const liveJson = liveRows[0] ?? "";
    const compactLiveBytes = jsonBytes(liveJson);
    const sharedFrame =
      liveRows.length <= 1 || liveRows.every((frame) => frame === liveJson);
    const metaBytes = jsonBytes(
      sessionRows.map((row) => ({
        id: row.id,
        status: row.status,
        focused: Number(row.focused) === 1,
      })),
    );
    const focusedIds = sessionRows
      .filter((row) => Number(row.focused) === 1)
      .map((row) => String(row.id));

    let materializedBytes = 0;
    let materializedRows = 0;
    let droppedRawOutput = !liveJson.includes('"rawOutput"');
    if (mode !== "storage") {
      const sql =
        mode === "all-rows"
          ? "SELECT payload FROM events"
          : mode === "all-hot"
            ? "SELECT payload FROM events WHERE hot = 1"
            : `SELECT payload FROM events WHERE hot = 1 AND session_id IN (${
                focusedIds.length === 0
                  ? "NULL"
                  : focusedIds.map(() => "?").join(",")
              })`;
      const rows =
        mode === "focused" && focusedIds.length > 0
          ? this.ctx.storage.sql.exec(sql, ...focusedIds).toArray()
          : this.ctx.storage.sql.exec(sql).toArray();
      for (const row of rows) {
        const payload = String(row.payload);
        materializedBytes += jsonBytes(JSON.parse(payload));
        materializedRows += 1;
        if (payload.includes('"rawOutput"')) {
          droppedRawOutput = false;
        }
      }
    }

    const isolateHeapBytes =
      mode === "storage"
        ? metaBytes + compactLiveBytes
        : metaBytes + compactLiveBytes + materializedBytes;
    const rawFanoutWouldHaveBeen = Number(
      this.ctx.storage.sql
        .exec("SELECT value FROM meta WHERE key = ?", "rawFanoutWouldHaveBeen")
        .toArray()[0]?.value ?? 0,
    );

    return {
      source: "cowboy-hub-extreme",
      mode,
      sessions: sessionRows.length,
      focusedSessions: focusedIds.length,
      terminals: liveRows.length,
      durableEvents: Number(counts.rows),
      hotRows: Number(counts.hot_rows),
      archiveRows: Number(counts.archive_rows),
      sqliteHotBytes: Number(counts.hot_bytes),
      sqliteArchiveBytes: Number(counts.archive_bytes),
      sqliteTotalBytes: Number(counts.bytes),
      materializedRows,
      materializedBytes,
      compactLiveBytes,
      sharedFrame,
      sessionMetaBytes: metaBytes,
      isolateHeapBytes,
      rawFanoutWouldHaveBeen,
      idleTailBudgetBytes: IDLE_TAIL_BUDGET,
      busyTailBudgetBytes: BUSY_TAIL_BUDGET,
      isolateBudgetBytes: ISOLATE_BUDGET,
      targetBudgetBytes: TARGET_BUDGET,
      fitsTarget: isolateHeapBytes < TARGET_BUDGET,
      fitsIsolate: isolateHeapBytes < ISOLATE_BUDGET,
      droppedRawOutput,
      liveImageExternalized: liveJson.includes("/api/artifacts/"),
      elapsedMs: Date.now() - started,
    };
  }
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const id = env.HUB.idFromName("cowboy-controller");
    const stub = env.HUB.get(id);
    return stub.fetch(request);
  },
};
