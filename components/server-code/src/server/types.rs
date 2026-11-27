//! Code plane request/response DTOs and workspace types.

use super::*;

/// One selectable working directory for the New Session dialog's dropdown.
#[derive(Debug, Serialize)]
pub struct Workspace {
    /// Sent to the daemon as `cwd` (absolute paths are honoured as-is).
    pub value: String,
    /// Short display name shown in the dropdown.
    pub label: String,
    /// Secondary line — the resolved absolute path or a description.
    pub help: String,
    /// Columbus registry id; absent for host-level roots.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Durable central work items projected onto this project.
    pub active_work_items: Vec<WorkspaceWorkItem>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WorkspaceWorkItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub projects: Vec<String>,
    #[serde(default)]
    pub recipe: String,
    #[serde(default)]
    pub blocked: bool,
}

/// Query string for `GET /api/sessions/{id}/files` — the composer's `@` picker.
#[derive(Debug, Deserialize)]
pub struct FileSearchQuery {
    /// Fuzzy query; empty returns the "most useful" files (shallow + recent).
    #[serde(default)]
    pub q: String,
    #[serde(default = "default_file_limit")]
    pub limit: usize,
}

#[derive(Debug, Serialize)]
pub struct FileSearchResponse {
    pub files: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct FileTreeQuery {
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_file_tree_limit")]
    pub limit: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTreeEntry {
    pub name: String,
    pub path: String,
    pub kind: &'static str,
    pub ignored: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTreeResponse {
    pub api_version: u8,
    pub path: String,
    pub revision: String,
    pub entries: Vec<FileTreeEntry>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeChangeResponse {
    pub path: String,
    pub old_path: Option<String>,
    pub status: &'static str,
    pub staged: bool,
    pub unstaged: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additions: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletions: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeChangesResponse {
    pub api_version: u8,
    pub head: Option<String>,
    pub revision: String,
    pub changes: Vec<CodeChangeResponse>,
    pub truncated: bool,
    pub comparison: Option<String>,
    pub default_comparison: Option<String>,
    pub comparisons: Vec<crate::code_review::GitComparison>,
    pub committed_comparisons: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched_at_ms: Option<u64>,
    /// Review can fetch the compared branch here; Machine adapters cannot yet.
    pub fetch_available: bool,
}

#[derive(Debug, Deserialize)]
pub struct CodeFetchRequest {
    pub comparison: String,
}

#[derive(Debug, Deserialize)]
pub struct CodeChangesQuery {
    pub comparison: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeRepositoryResponse {
    pub api_version: u8,
    pub commits: Vec<crate::code_review::GitCommitSummary>,
    pub history_truncated: bool,
    pub worktrees: Vec<crate::code_review::GitWorktreeSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<crate::code_review::GitChangeTarget>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeCommitResponse {
    pub api_version: u8,
    #[serde(flatten)]
    pub commit: crate::code_review::GitCommitDetail,
}

#[derive(Debug, Deserialize)]
pub struct CodeRepositoryQuery {
    pub after: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodeCommitQuery {
    pub oid: String,
}

#[derive(Debug, Deserialize)]
pub struct CodeCommitDiffQuery {
    pub oid: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeManifestResponse {
    pub api_version: u8,
    pub provider: String,
    pub revision: String,
    pub head: Option<String>,
    pub project: String,
    pub branch: Option<String>,
    pub worktree: Option<String>,
    pub change_count: usize,
    pub language: CodeLanguageCapabilities,
    pub buffer_mode: code_buffers::ReviewMode,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeLanguageCapabilities {
    pub provider: &'static str,
    pub state: &'static str,
    pub diagnostics: bool,
    pub inlay_hints: bool,
    pub semantic_tokens: bool,
    pub hover: bool,
    pub navigation: bool,
    pub outline: bool,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeBufferVersion {
    pub replica_id: u32,
    pub timestamp: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeInlayHint {
    pub offset: u64,
    pub label: String,
    pub kind: Option<String>,
    pub padding_left: bool,
    pub padding_right: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeLanguageResponse {
    pub api_version: u8,
    pub path: String,
    pub version: Vec<CodeBufferVersion>,
    pub diagnostics: Vec<CodeDiagnostic>,
    pub inlay_hints: Vec<CodeInlayHint>,
    pub semantic_tokens: Vec<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeHoverBlock {
    pub text: String,
    pub language: Option<String>,
    pub markdown: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeHoverResponse {
    pub api_version: u8,
    pub path: String,
    pub contents: Vec<CodeHoverBlock>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeLocation {
    pub path: String,
    pub start: CodePoint,
    pub end: CodePoint,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeNavigationResponse {
    pub api_version: u8,
    pub path: String,
    pub locations: Vec<CodeLocation>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeDocumentSymbol {
    pub name: String,
    pub kind: i32,
    pub start: CodePoint,
    pub end: CodePoint,
    pub selection_start: CodePoint,
    pub selection_end: CodePoint,
    pub children: Vec<CodeDocumentSymbol>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineResponse {
    pub api_version: u8,
    pub path: String,
    pub symbols: Vec<CodeDocumentSymbol>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeDiffQuery {
    pub path: Option<String>,
    pub cursor: Option<String>,
    #[serde(default = "default_code_context")]
    pub context: usize,
    #[serde(default = "default_true")]
    pub show_whitespace: bool,
    #[serde(default)]
    pub scope: CodeDiffScope,
    pub comparison: Option<String>,
    #[serde(default)]
    pub change_fingerprint: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum CodeDiffScope {
    #[default]
    Combined,
    Staged,
    Unstaged,
}

pub fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeDiffResponse {
    pub api_version: u8,
    pub path: String,
    pub revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_fingerprint: Option<String>,
    pub text: String,
    pub added: usize,
    pub removed: usize,
    pub truncated: bool,
    pub next_cursor: Option<String>,
    pub limited: bool,
}

#[derive(Debug, Deserialize)]
pub struct CodeFileQuery {
    pub path: String,
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeBufferLeaseRequest {
    pub path: String,
    pub lease_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeBufferLeaseResponse {
    pub api_version: u8,
    pub path: String,
    pub leases: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeFileResponse {
    pub api_version: u8,
    pub path: String,
    pub revision: String,
    pub text: String,
    pub size: u64,
    pub truncated: bool,
    pub next_cursor: Option<String>,
    pub limited: bool,
}

pub const WORKSPACE_CODE_CONTEXT_PREFIX: &str = "workspace::";

pub struct ResolvedCodeContext {
    pub cwd: String,
    pub scope: CodeReadScope,
}

// Legacy resource calls retain their separate original-connection operation
// boundary. Buffered language reads use code_reads' product and route binding.
pub struct SessionCodeContext {
    pub machine_id: String,
    pub cwd: String,
    pub scope: crate::core::SessionCodeScope,
}

#[derive(Debug, Deserialize)]
pub struct CodeLanguageQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct CodeHoverQuery {
    pub path: String,
    pub row: u32,
    pub column: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeNavigationKind {
    Definition,
    Declaration,
    TypeDefinition,
    Implementation,
    References,
}

#[derive(Debug, Deserialize)]
pub struct CodeNavigationQuery {
    pub path: String,
    pub row: u32,
    pub column: u32,
    pub kind: CodeNavigationKind,
}

#[derive(Debug, Deserialize)]
pub struct ReconcileProjectSessionsRequest {
    pub project: String,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct ReconcileProjectSessionsResponse {
    pub session_ids: Vec<String>,
    pub native_thread_ids: HashMap<String, String>,
}
