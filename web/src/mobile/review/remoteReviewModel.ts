export interface RemoteReviewBinding {
  pluginId: string;
  view: string;
  host: string;
  owner: string;
  repository: string;
  repositoryId: string;
  number: string;
}

export interface RemoteReviewFile {
  path: string;
  oldPath: string | null;
  status: string;
  additions: number;
  deletions: number;
  patch: string | null;
  limited: boolean;
}

export interface RemoteReviewPage {
  repositoryId: string;
  number: string;
  title: string;
  url: string;
  state: string;
  head: string;
  base: string;
  revision: string;
  totalFiles: number;
  files: RemoteReviewFile[];
  nextPage: number | null;
  limited: boolean;
}

export function pullNumber(
  input: string,
  remote: { host: string; owner: string; repository: string },
): string {
  const text = input.trim();
  if (/^[1-9][0-9]{0,15}$/.test(text)) return text;
  const url = new URL(text);
  const prefix = `/${remote.owner}/${remote.repository}/pull/`;
  if (
    url.protocol !== "https:" || url.hostname !== remote.host || url.port ||
    url.username || url.password ||
    !url.pathname.toLowerCase().startsWith(prefix.toLowerCase())
  ) {
    throw new Error("Choose the repository matching this pull request URL.");
  }
  const number = url.pathname.slice(prefix.length).replace(/\/$/, "");
  if (!/^[1-9][0-9]{0,15}$/.test(number)) {
    throw new Error("Enter a PR number or its main GitHub URL.");
  }
  return number;
}

export function decodeRemoteReview(input: unknown): RemoteReviewPage {
  function object(v: unknown): Record<string, unknown> {
    if (!v || typeof v !== "object" || Array.isArray(v)) {
      throw new Error("Invalid PR response");
    }
    return v as Record<string, unknown>;
  }
  function text(v: unknown, max = 4096): string {
    if (typeof v !== "string" || v.length > max) {
      throw new Error("Invalid PR text");
    }
    return v;
  }
  function count(v: unknown): number {
    if (typeof v !== "number" || !Number.isSafeInteger(v) || v < 0) {
      throw new Error("Invalid PR count");
    }
    return v;
  }
  function flag(v: unknown): boolean {
    if (typeof v !== "boolean") throw new Error("Invalid PR flag");
    return v;
  }
  const v = object(input);
  const url = new URL(text(v.url, 2048));
  if (url.protocol !== "https:" || url.username || url.password || url.port) {
    throw new Error("Invalid PR URL");
  }
  if (!Array.isArray(v.files) || v.files.length > 20) {
    throw new Error("Invalid PR files");
  }
  const head = text(v.head, 64),
    base = text(v.base, 64),
    revision = text(v.revision, 64);
  if (
    ![head, base].every((oid) =>
      /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/i.test(oid)
    ) || !/^[a-f0-9]{64}$/.test(revision)
  ) throw new Error("Invalid PR revision");
  const nextPage = v.nextPage === null ? null : count(v.nextPage);
  if (nextPage !== null && (nextPage < 2 || nextPage > 150)) {
    throw new Error("Invalid PR page");
  }
  return {
    repositoryId: text(v.repositoryId, 24),
    number: text(v.number, 24),
    title: text(v.title, 1000),
    url: url.href,
    state: text(v.state, 16),
    head,
    base,
    revision,
    totalFiles: count(v.totalFiles),
    nextPage,
    limited: flag(v.limited),
    files: v.files.map((input) => {
      const f = object(input);
      return {
        path: text(f.path),
        oldPath: f.oldPath === null ? null : text(f.oldPath),
        status: text(f.status, 32),
        additions: count(f.additions),
        deletions: count(f.deletions),
        patch: f.patch === null ? null : text(f.patch, 256 * 1024),
        limited: flag(f.limited),
      };
    }),
  };
}

export function sameReview(
  left: RemoteReviewPage,
  right: RemoteReviewPage,
): boolean {
  return left.repositoryId === right.repositoryId &&
    left.number === right.number &&
    left.revision === right.revision && left.totalFiles === right.totalFiles;
}
