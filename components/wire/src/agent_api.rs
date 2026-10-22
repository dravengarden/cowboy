//! Agent API limits that appear in wire contracts.

/// Program source, as Matrix CodeAct.
pub const MAX_SOURCE_BYTES: usize = 32 * 1024;
/// A program's wall-clock budget when it names none; at most `MAX_WAIT_MS`.
pub const DEFAULT_TIMEOUT_MS: u64 = 20_000;
/// Runtime startup headroom added to the gateway exchange.
pub const STARTUP_MARGIN_MS: u64 = 5_000;
