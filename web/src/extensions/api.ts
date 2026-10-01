/** The resource protocol is intentionally independent of Plugin identity. */
import {
  decodeRemoteReview,
  type RemoteReviewPage,
} from "../mobile/review/remoteReviewModel.ts";
export interface ExtensionIdentity {
  pluginId: string;
  pluginVersion: string;
  generationDigest: string;
}
export interface ExtensionView {
  id: string;
  label: string;
  filters: { value: string; label: string }[];
  review?: "pull_request" | null;
}
export interface WorkspaceExtension {
  identity: ExtensionIdentity;
  label: string;
  description: string;
  views: ExtensionView[];
  available: boolean;
}
export interface RepositoryRemote {
  name: string;
  host: string;
  owner: string;
  repository: string;
}
export interface Resource {
  id: string;
  title: string;
  url: string | null;
  body: string | null;
  bodyTruncated: boolean;
  state: string | null;
  updatedAt: string | null;
  metadata: { label: string; value: string }[];
}
export type ExtensionResponse =
  | {
    type: "inventory";
    extensions: WorkspaceExtension[];
    remotes: RepositoryRemote[];
  }
  | { type: "page"; items: Resource[]; nextPage: number | null }
  | { type: "detail"; item: Resource }
  | { type: "review"; review: RemoteReviewPage }
  | { type: "unavailable"; code: keyof typeof failures };

const failures = {
  review_changed:
    "This PR changed while loading. Refresh to read its new version.",
  review_unavailable:
    "The PR preview is unavailable or exceeds the preview limit. Retry or open it on GitHub.",
  machine_unavailable:
    "The workspace’s Machine is unavailable. Reconnect it and refresh.",
  extension_changed:
    "This extension changed. Refresh Extensions to load the installed version.",
  dependency_unavailable:
    "A required Plugin is unavailable. Check this Machine’s installed extensions.",
  repository_unavailable:
    "The repository remote is unavailable. Refresh or choose another remote.",
  connection_unavailable:
    "The CLI connection is unavailable. Check the existing CLI login on this workspace’s Machine.",
  request_failed:
    "The resource could not be read. Check repository access or retry after the service’s rate limit resets.",
  busy: "This Machine is handling other extension requests. Try again shortly.",
  invalid_request:
    "This resource request is no longer valid. Refresh Extensions.",
};

function invalid(): never {
  throw new Error(
    "Incompatible extension response. Refresh Cowboy and try again.",
  );
}
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) invalid();
  return value as Record<string, unknown>;
}
function text(value: unknown, max = 1024): string {
  if (typeof value !== "string" || value.length > max) invalid();
  return value;
}
function nullableText(value: unknown, max = 1024): string | null {
  return value === null ? null : text(value, max);
}
function array<T>(value: unknown, max: number, decode: (v: unknown) => T): T[] {
  if (!Array.isArray(value) || value.length > max) invalid();
  return value.map(decode);
}
function resource(input: unknown): Resource {
  const v = record(input);
  if (typeof v.bodyTruncated !== "boolean") invalid();
  const url = nullableText(v.url, 2048);
  if (url !== null) {
    const parsed = new URL(url);
    if (parsed.protocol !== "https:" || parsed.username || parsed.password) {
      invalid();
    }
  }
  return {
    id: text(v.id, 24),
    title: text(v.title),
    url,
    body: nullableText(v.body, 65536),
    bodyTruncated: v.bodyTruncated,
    state: nullableText(v.state, 80),
    updatedAt: nullableText(v.updatedAt, 64),
    metadata: array(v.metadata, 12, (m) => {
      const row = record(m);
      return { label: text(row.label, 60), value: text(row.value, 256) };
    }),
  };
}
export function decodeExtensionResponse(input: unknown): ExtensionResponse {
  const v = record(input);
  switch (v.type) {
    case "review":
      return { type: "review", review: decodeRemoteReview(v.review) };
    case "unavailable": {
      const code = text(v.code);
      if (!Object.hasOwn(failures, code)) invalid();
      return { type: "unavailable", code: code as keyof typeof failures };
    }
    case "inventory":
      return {
        type: "inventory",
        extensions: array(v.extensions, 128, (item) => {
          const row = record(item), identity = record(row.identity);
          if (typeof row.available !== "boolean") invalid();
          return {
            identity: {
              pluginId: text(identity.pluginId, 128),
              pluginVersion: text(identity.pluginVersion, 128),
              generationDigest: text(identity.generationDigest, 71),
            },
            label: text(row.label, 80),
            description: text(row.description, 512),
            available: row.available,
            views: array(row.views, 16, (item) => {
              const view = record(item);
              if (view.review != null && view.review !== "pull_request") {
                invalid();
              }
              return {
                id: text(view.id, 128),
                label: text(view.label, 80),
                review: view.review === "pull_request" ? "pull_request" : null,
                filters: array(view.filters, 8, (item) => {
                  const filter = record(item);
                  return {
                    value: text(filter.value, 64),
                    label: text(filter.label, 60),
                  };
                }),
              };
            }),
          };
        }),
        remotes: array(v.remotes, 64, (item) => {
          const row = record(item);
          return {
            name: text(row.name, 100),
            host: text(row.host, 253),
            owner: text(row.owner, 100),
            repository: text(row.repository, 100),
          };
        }),
      };
    case "page": {
      if (
        v.nextPage !== null &&
        (typeof v.nextPage !== "number" || !Number.isInteger(v.nextPage) ||
          v.nextPage < 2 || v.nextPage > 1000)
      ) invalid();
      return {
        type: "page",
        items: array(v.items, 50, resource),
        nextPage: v.nextPage,
      };
    }
    case "detail":
      return { type: "detail", item: resource(v.item) };
    default:
      return invalid();
  }
}

export async function extensionRequest(
  context: string,
  query: URLSearchParams | null,
  signal: AbortSignal,
): Promise<Exclude<ExtensionResponse, { type: "unavailable" }>> {
  const response = await fetch(
    `/api/code/sessions/${encodeURIComponent(context)}/extensions${
      query ? `/resources?${query}` : ""
    }`,
    {
      credentials: "same-origin",
      cache: "no-store",
      signal,
      headers: { accept: "application/json" },
    },
  );
  if (
    !response.ok ||
    !response.headers.get("content-type")?.includes("application/json")
  ) {
    await response.body?.cancel();
    throw new Error(
      "The workspace is unavailable. Reconnect and refresh Extensions.",
    );
  }
  const value = decodeExtensionResponse(await response.json());
  if (value.type === "unavailable") throw new Error(failures[value.code]);
  return value;
}

export function resourceQuery(
  identity: ExtensionIdentity,
  remote: string,
  view: string,
  filter: string,
  page: number,
  item?: string,
): URLSearchParams {
  return new URLSearchParams({
    ...identity,
    remote,
    view,
    page: String(page),
    ...(filter ? { filter } : {}),
    ...(item ? { item } : {}),
  });
}
