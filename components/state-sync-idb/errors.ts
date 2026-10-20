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
  readonly nativeErrorName: string;
  constructor(readonly code: IdbFailureCode, nativeError?: unknown) {
    super(`IndexedDB persistence: ${code}`);
    this.name = "IdbPersistenceError";
    // Keep only known browser error names, never native messages or record data.
    const name = nativeError instanceof Error ? nativeError.name : "";
    this.nativeErrorName = [
        "AbortError",
        "UnknownError",
        "InvalidStateError",
        "NotReadableError",
        "QuotaExceededError",
        "DataCloneError",
        "DataError",
        "TransactionInactiveError",
        "NotFoundError",
        "SecurityError",
        "VersionError",
        "ConstraintError",
      ].includes(name)
      ? name
      : "";
  }
}
