import { createHash } from "node:crypto";
import { appendFileSync, readFileSync } from "node:fs";
import { join } from "node:path";

// Spent identities cannot be evicted: that would allow an old mutation to run
// again. Spill them into 256 private local buckets instead. Only a bounded hot
// cache and the bridge's outstanding results remain in memory. The containing
// directory belongs to this bridge incarnation and is removed on close.
export class SpentCalls {
  cache = new Map();
  buckets = new Map();
  failed = false;

  constructor(directory) {
    this.directory = directory;
  }

  claim(id, kind) {
    try {
      if (!/^[a-zA-Z0-9_-]{1,256}$/.test(id) || !["admitted", "cancelled"].includes(kind)) {
        throw new Error("Invalid admission identity");
      }
      const previous = this.get(id);
      if (previous !== undefined) return previous;
      // Synchronous local admission is intentional: no cancellation or second
      // request may pass this check before the identity is recorded. No target
      // effect starts if storage fails. Each record starts with a delimiter so
      // a partial failed append cannot obscure a later successful record.
      const path = this.path(id);
      const record = `\n${id}\t${kind}\n`;
      const bucket = this.buckets.get(path);
      appendFileSync(path, record, { mode: 0o600, flag: bucket === undefined ? "wx" : "a" });
      const hash = bucket?.hash ?? createHash("sha256");
      hash.update(record);
      this.buckets.set(path, { size: (bucket?.size ?? 0) + record.length, hash });
      this.remember(id, kind);
      return undefined;
    } catch {
      this.failed = true;
      throw Object.assign(new Error("Bridge admission storage unavailable"), {
        code: "bridge_storage_unavailable",
      });
    }
  }

  get(id) {
    try {
      if (this.failed) throw new Error("Admission storage previously failed");
      if (this.cache.has(id)) return this.cache.get(id);
      const path = this.path(id);
      const bucket = this.buckets.get(path);
      if (bucket === undefined) return undefined;
      const text = readFileSync(path, "utf8");
      if (
        text.length !== bucket.size ||
        createHash("sha256").update(text).digest("hex") !== bucket.hash.copy().digest("hex")
      ) {
        throw new Error("Admission bucket changed");
      }
      const marker = `\n${id}\t`;
      const offset = text.indexOf(marker);
      if (offset < 0) return undefined;
      const start = offset + marker.length;
      const kind = text.slice(start, text.indexOf("\n", start));
      if (!["admitted", "cancelled"].includes(kind)) throw new Error("Invalid spent record");
      this.remember(id, kind);
      return kind;
    } catch {
      this.failed = true;
      throw Object.assign(new Error("Bridge admission storage unavailable"), {
        code: "bridge_storage_unavailable",
      });
    }
  }

  path(id) {
    return join(
      this.directory,
      "spent-" + createHash("sha256").update(id).digest("hex").slice(0, 2),
    );
  }

  remember(id, kind) {
    this.cache.set(id, kind);
    if (this.cache.size > 4096) this.cache.delete(this.cache.keys().next().value);
  }
}
