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
  changes: CodeChange[];
  truncated: boolean;
}

export interface CodeDocument {
  apiVersion: 1;
  path: string;
  revision?: string;
  text: string;
  truncated: boolean;
}

async function codeFetch<T>(
  url: string,
  signal?: AbortSignal,
): Promise<T> {
  const response = await fetch(url, signal ? { signal } : undefined);
  if (!response.ok) {
    throw new Error(`Code API ${response.status}`);
  }
  const body = await response.json() as T & { apiVersion?: number };
  if (body.apiVersion !== 1) {
    throw new Error("Unsupported Code API version");
  }
  return body;
}

export function fetchCodeChanges(
  sessionId: string,
  signal?: AbortSignal,
): Promise<CodeChanges> {
  return codeFetch(
    `/api/code/sessions/${encodeURIComponent(sessionId)}/changes`,
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
