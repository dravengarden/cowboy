export type CodeChangeStatus =
  | "modified"
  | "added"
  | "deleted"
  | "renamed"
  | "untracked"
  | "conflicted";

export interface CodeChange {
  path: string;
  oldPath?: string;
  status: CodeChangeStatus;
  staged: boolean;
  unstaged: boolean;
}

export type CodeDiffScope = "combined" | "staged" | "unstaged";

export interface CodeChanges {
  apiVersion: 1;
  head?: string;
  revision: string;
  changes: CodeChange[];
  truncated: boolean;
}

export interface CodeTreeEntry {
  name: string;
  path: string;
  kind: "directory" | "file";
}

export interface CodeTreePage {
  apiVersion: 1;
  path: string;
  revision: string;
  entries: CodeTreeEntry[];
  truncated: boolean;
}

export interface CodeManifest {
  apiVersion: 1;
  provider: string;
  revision: string;
  head?: string;
  changeCount: number;
  language: CodeLanguageCapabilities;
}

export interface CodeLanguageCapabilities {
  provider: string;
  state: "restricted" | "warming" | "ready" | "unavailable" | "failed";
  diagnostics: boolean;
  inlayHints: boolean;
  semanticTokens: boolean;
}

export interface CodeSearchResults {
  apiVersion: 1;
  files: string[];
}

export interface CodeDocument {
  apiVersion: 1;
  path: string;
  revision?: string;
  text: string;
  truncated: boolean;
  nextCursor?: string;
  limited?: boolean;
}

export interface CodeBufferLease {
  apiVersion: 1;
  path: string;
  leases: number;
}

export interface CodeLanguagePoint {
  row: number;
  column: number;
}

export interface CodeDiagnostic {
  start: CodeLanguagePoint;
  end: CodeLanguagePoint;
  severity: number;
  source?: string;
  message: string;
}

export interface CodeInlayHint {
  offset: number;
  label: string;
  kind?: string;
  paddingLeft: boolean;
  paddingRight: boolean;
}

export interface CodeLanguage {
  apiVersion: 1;
  path: string;
  version: { replicaId: number; timestamp: number }[];
  diagnostics: CodeDiagnostic[];
  inlayHints: CodeInlayHint[];
  semanticTokens: number[];
}

export class CodeApiError extends Error {
  constructor(public readonly status: number) {
    super(`Code API ${status}`);
  }
}

async function codeFetch<T>(
  url: string,
  signal?: AbortSignal,
  cache?: RequestCache,
): Promise<T> {
  const response = await fetch(url, {
    ...(signal ? { signal } : {}),
    ...(cache ? { cache } : {}),
  });
  if (!response.ok) {
    throw new CodeApiError(response.status);
  }
  const body = await response.json() as T & { apiVersion?: number };
  if (body.apiVersion !== 1) {
    throw new Error("Unsupported Code API version");
  }
  return body;
}

async function codeBufferLease(
  sessionId: string,
  path: string,
  leaseId: string,
  method: "PUT" | "DELETE",
): Promise<CodeBufferLease> {
  const response = await fetch(
    `/api/code/sessions/${encodeURIComponent(sessionId)}/buffer`,
    {
      method,
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ path, leaseId }),
      keepalive: method === "DELETE",
    },
  );
  if (!response.ok) {
    throw new CodeApiError(response.status);
  }
  const body = await response.json() as CodeBufferLease;
  if (body.apiVersion !== 1) {
    throw new Error("Unsupported Code API version");
  }
  return body;
}

export function openCodeBuffer(
  sessionId: string,
  path: string,
  leaseId: string,
): Promise<CodeBufferLease> {
  return codeBufferLease(sessionId, path, leaseId, "PUT");
}

export function closeCodeBuffer(
  sessionId: string,
  path: string,
  leaseId: string,
): Promise<CodeBufferLease> {
  return codeBufferLease(sessionId, path, leaseId, "DELETE");
}

export function fetchCodeLanguage(
  sessionId: string,
  path: string,
  signal?: AbortSignal,
): Promise<CodeLanguage> {
  const query = new URLSearchParams({ path });
  return codeFetch(
    `/api/code/sessions/${encodeURIComponent(sessionId)}/language?${query}`,
    signal,
    "no-store",
  );
}

export function fetchCodeDiffPage(
  sessionId: string,
  cursor: string,
  signal?: AbortSignal,
): Promise<CodeDocument & { added: number; removed: number }> {
  const query = new URLSearchParams({ cursor });
  return codeFetch(
    `/api/code/sessions/${encodeURIComponent(sessionId)}/diff?${query}`,
    signal,
  );
}

export function fetchCodeChanges(
  sessionId: string,
  signal?: AbortSignal,
): Promise<CodeChanges> {
  return codeFetch(
    `/api/code/sessions/${encodeURIComponent(sessionId)}/changes`,
    signal,
    "no-store",
  );
}

export function fetchCodeManifest(
  sessionId: string,
  signal?: AbortSignal,
): Promise<CodeManifest> {
  return codeFetch(
    `/api/code/sessions/${encodeURIComponent(sessionId)}/manifest`,
    signal,
    "no-store",
  );
}

export function fetchCodeTree(
  sessionId: string,
  path: string,
  signal?: AbortSignal,
  refresh = false,
): Promise<CodeTreePage> {
  const query = path ? `?${new URLSearchParams({ path })}` : "";
  return codeFetch(
    `/api/code/sessions/${encodeURIComponent(sessionId)}/tree${query}`,
    signal,
    refresh ? "reload" : "default",
  );
}

export function fetchCodeSearch(
  sessionId: string,
  query: string,
  signal?: AbortSignal,
): Promise<CodeSearchResults> {
  const params = new URLSearchParams({ q: query, limit: "50" });
  return codeFetch(
    `/api/code/sessions/${encodeURIComponent(sessionId)}/search?${params}`,
    signal,
  );
}

export function fetchCodeDiff(
  sessionId: string,
  path: string,
  context: number,
  showWhitespace: boolean,
  scope: CodeDiffScope,
  signal?: AbortSignal,
): Promise<CodeDocument & { added: number; removed: number }> {
  const query = new URLSearchParams({
    path,
    context: String(context < 0 ? 100 : context),
    showWhitespace: String(showWhitespace),
    scope,
  });
  return codeFetch(
    `/api/code/sessions/${encodeURIComponent(sessionId)}/diff?${query}`,
    signal,
  );
}

export function fetchCodeFile(
  sessionId: string,
  path: string,
  signal?: AbortSignal,
): Promise<CodeDocument & { size: number }> {
  const query = new URLSearchParams({ path });
  return codeFetch(
    `/api/code/sessions/${encodeURIComponent(sessionId)}/file?${query}`,
    signal,
  );
}

export function fetchCodeFilePage(
  sessionId: string,
  path: string,
  cursor: string,
  signal?: AbortSignal,
): Promise<CodeDocument & { size: number }> {
  const query = new URLSearchParams({ path, cursor });
  return codeFetch(
    `/api/code/sessions/${encodeURIComponent(sessionId)}/file?${query}`,
    signal,
  );
}
