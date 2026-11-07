//! Machine-local filesystem observation, not acquisition or delivery authority.
//! A complete event window never proves that an analyzer consumed those inputs.

mod journal;
pub use journal::{Change, Coverage, Cursor, Gap, Journal, Page, PageState, Scan};
#[cfg(target_os = "linux")]
pub mod linux;
