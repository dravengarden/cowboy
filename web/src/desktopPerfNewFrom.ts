/** Replace or fork in the Desktop perf fixture: the Machine, the HTTP answers
 * its form reads, and the Sessions `f` action measured end to end. */
import type { MachineSummary, SessionMeta } from "./protocol";
import type { ProviderCatalogEntry } from "@cowboy/provider-ui";
import { closeSessionSettings, requestNewFrom } from "./sessionSettingsOpen";
import { requestPickSession } from "./sessionPickRequest";

type Until = (predicate: () => boolean, label: string, ms?: number) => Promise<void>;

const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

export function newFromMachine(entry: ProviderCatalogEntry, busy: number): MachineSummary {
  return {
    id: "hawk",
    display_name: "Hawk",
    platform: "linux",
    architecture: "x86_64",
    status: "online",
    components: [],
    capacity: { max_sessions: 20, draining: false },
    active_sessions: busy,
    local: true,
    connected: true,
    schedulable: true,
    workspaces: Array.from({ length: 12 }, (_, i) => ({
      id: `project-${i}`,
      display_name: `project-${i}`,
      canonical_path: `/fixture/project-${i}`,
    })),
    plugins: [{
      plugin_id: entry.provider_id,
      plugin_kind: "agent_provider",
      plugin_version: entry.provider_version,
      generation_digest: entry.artifact_digest,
      contract_fingerprint: entry.contract_fingerprint,
      state: "active",
      materialization_state: "current",
      replica_state: "current",
      active_session_leases: 0,
    }],
  } as MachineSummary;
}

const ANSWERS: Record<string, unknown> = {
  "/api/project-policies": {
    schema: 1,
    revision: "fixture",
    default_runtime_machine_id: "hawk",
    machines: {},
  },
  "/api/project-placements": {
    machine_id: "hawk",
    default_runtime_machine_id: "hawk",
    placements: [{ runtime_machine_id: "hawk", provider: "example", mode: "local" }],
  },
  "/api/provider-accounts": {
    registry: {
      revision: 1,
      accounts: [{ id: "husky", provider_id: "example", label: "Husky", enabled: true }],
      defaults: {},
    },
    session_admission: "enabled",
    session_providers: ["example"],
  },
  "/api/provider-accounts/usage": { accounts: [] },
};

/** The form's reads, each after a short network delay; undefined otherwise. */
export async function newFromFetch(path: string): Promise<Response | undefined> {
  const context = /^\/api\/sessions\/[^/]+\/context$/.test(path);
  if (!context && !Object.hasOwn(ANSWERS, path)) return undefined;
  await delay(30);
  return Response.json(context
    ? {
      revision: 1,
      applied_revision: 1,
      sources: [],
      next_agent: null,
      source_sessions: [],
      delivery: null,
      max_sources: 8,
    }
    : ANSWERS[path]);
}

/** Open Replace or fork on `count` idle Hawk sessions as the Sessions `f`
 *  action does. `openMs` ends when the form shows its installation and
 *  account (and holds the cursor); `menuMs` is opening the Subscription menu. */
export async function openNewFrom(
  ids: readonly string[],
  sessions: readonly SessionMeta[],
  until: Until,
): Promise<{ openMs: number; menuMs: number }[]> {
  const timings: { openMs: number; menuMs: number }[] = [];
  const editor = () => document.querySelector<HTMLElement>("[data-new-session-editor]");
  const describe = (error: unknown) => {
    throw new Error(
      `${String(error)}: ${editor()?.textContent?.slice(0, 400) ?? "no form"}; focus on ${
        document.activeElement?.outerHTML.slice(0, 120)
      }`,
    );
  };
  for (const id of ids) {
    const began = performance.now();
    requestPickSession(id, sessions.find((session) => session.id === id));
    requestNewFrom(id);
    await until(
      () =>
        Boolean(editor()?.textContent?.includes("Hawk")) &&
        !editor()!.textContent!.includes("Loading subscriptions") &&
        editor()!.contains(document.activeElement),
      "Replace or fork form with the cursor",
    ).catch(describe);
    const openMs = performance.now() - began;
    await delay(300);
    const pressed = performance.now();
    document.querySelector<HTMLElement>("[data-session-subscription] [role=combobox]")!
      .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    await until(() => document.querySelector("[role=listbox]") !== null, "subscription menu")
      .catch(describe);
    await new Promise((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0)));
    timings.push({ openMs, menuMs: performance.now() - pressed });
    document.querySelector<HTMLElement>("[role=listbox]")!.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
    );
    await delay(300);
    closeSessionSettings(id);
    await until(() => document.querySelector("[data-run-config-panel]") === null, "settings closed");
    await delay(300);
  }
  return timings;
}
