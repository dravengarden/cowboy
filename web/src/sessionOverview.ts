import { originLabel, type SessionMeta, type Status } from "./protocol";
import { providerName } from "./providerPresentation";
import { fireLabel, fireRel } from "./scheduleTime";
import { sessionProjectLabel } from "./sessionProject";

/** `/api/sessions/:id/info` — flattened `SessionMeta` plus live in-memory counts. */
export interface SessionInfoPayload extends SessionMeta {
  event_count: number;
  queue_count: number;
  drafts_count: number;
}

export interface SessionOverviewRow {
  label: string;
  value: string;
}

export interface SessionOverviewSection {
  key: "identity" | "workspace" | "activity" | "ids";
  rows: SessionOverviewRow[];
}

/** Prefer the live WS snapshot for status/context; keep HTTP counts. */
export function mergeSessionOverview(
  info: SessionInfoPayload,
  live?: SessionMeta | null,
): SessionInfoPayload {
  if (!live || live.id !== info.id) return info;
  return {
    ...info,
    ...live,
    event_count: info.event_count,
    queue_count: info.queue_count,
    drafts_count: info.drafts_count,
  };
}

/**
 * Dense read-only rows for the session-list info sheet / desktop overview.
 * Lifecycle words match the sidebar status-dot labels in App.tsx.
 */
export function sessionOverviewSections(
  info: SessionInfoPayload,
  now: number = Date.now(),
): SessionOverviewSection[] {
  const identity: SessionOverviewRow[] = [
    { label: "Title", value: info.title },
    { label: "Status", value: sessionOverviewStatus(info) },
  ];
  if (info.system) identity.push({ label: "Kind", value: "System" });
  identity.push(
    { label: "Provider", value: sessionOverviewProvider(info) },
    { label: "Machine", value: info.machine_id?.trim() || "local" },
    { label: "Origin", value: originLabel(info.origin) },
  );

  const workspace: SessionOverviewRow[] = [
    { label: "Project", value: sessionProjectLabel(info) ?? "Not recorded" },
    ...sessionOverviewPaths(info),
  ];

  const activity: SessionOverviewRow[] = [
    { label: "Context", value: sessionOverviewContext(info) },
    { label: "Events", value: info.event_count.toLocaleString("en-US") },
    { label: "Queued", value: String(info.queue_count) },
    { label: "Drafts", value: String(info.drafts_count) },
  ];
  const nextDraft = sessionOverviewNextDraft(info, now);
  if (nextDraft) activity.push({ label: "Next draft", value: nextDraft });

  return [
    { key: "identity", rows: identity },
    { key: "workspace", rows: workspace },
    { key: "activity", rows: activity },
    { key: "ids", rows: [{ label: "Session ID", value: info.id }] },
  ];
}

export function sessionOverviewStatus(
  info: Pick<SessionMeta, "status" | "paused">,
): string {
  const parts = [lifecycleStatus(info.status)];
  if (info.paused) parts.push("Queue paused");
  return parts.join(" · ");
}

function lifecycleStatus(status: Status): string {
  switch (status) {
    case "running":
      return "Live";
    case "busy":
      return "Running…";
    case "starting":
      return "Starting…";
    case "exited":
      return "Dormant";
    case "interrupted":
      return "Interrupted";
    case "crashed":
      return "Crashed";
  }
}

function sessionOverviewProvider(info: SessionMeta): string {
  const name = providerName(
    info.provider,
    info.provider_version,
    info.provider_generation_digest,
  );
  const version = info.provider_version?.trim();
  return version ? `${name} · ${version}` : name;
}

function sessionOverviewPaths(info: SessionMeta): SessionOverviewRow[] {
  const checkout = info.workspace_source_path?.trim();
  if (checkout && checkout !== info.cwd) {
    return [
      { label: "Checkout", value: checkout },
      { label: "Worktree", value: info.cwd },
    ];
  }
  return [{ label: "Directory", value: info.cwd }];
}

function sessionOverviewContext(info: SessionMeta): string {
  const used = info.context_used ?? 0;
  const size = info.context_size ?? 0;
  if (!(size > 0)) return "Not reported";
  const percent = Math.min(100, Math.max(0, Math.round((used / size) * 100)));
  return `${used.toLocaleString("en-US")} / ${
    size.toLocaleString("en-US")
  } · ${percent}%`;
}

function sessionOverviewNextDraft(
  info: SessionMeta,
  now: number,
): string | null {
  const at = info.next_schedule_ms;
  if (typeof at !== "number" || !Number.isFinite(at)) return null;
  return `${fireLabel(at, now)} · ${fireRel(at, now)}`;
}
