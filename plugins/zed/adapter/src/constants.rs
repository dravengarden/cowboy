//! Wire identity and bounded native response sizes.
pub(super) const ADAPTER_VERSION: u8 = 1;
pub(super) const ZED_VERSION: &str = "1.13.0";
pub(super) const ZED_REVISION: &str = "aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45";
pub(super) const MAX_DIAGNOSTICS: usize = 1_000;
pub(super) const MAX_INLAY_HINTS: usize = 2_000;
pub(super) const MAX_SEMANTIC_TOKEN_WORDS: usize = 50_000;
pub(super) const MAX_DOCUMENT_SYMBOLS: usize = 2_000;
