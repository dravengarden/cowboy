export type IdbFailureCode =
  | "unavailable"
  | "open_failed"
  | "open_blocked"
  | "open_timeout"
  | "schema_mismatch"
  | "transaction_failed"
  | "transaction_aborted"
  | "request_failed"
  | "key_limit_exceeded"
  | "close_failed"
  | "snapshot_invalid"
  | "outbox_conflict"
  | "outbox_loading"
  | "record_mode_conflict";

/** Closed, content-free diagnostics: never include keys, values or native text. */
export class IdbPersistenceError extends Error {
  constructor(readonly code: IdbFailureCode) {
    super(`IndexedDB persistence: ${code}`);
    this.name = "IdbPersistenceError";
  }
}
