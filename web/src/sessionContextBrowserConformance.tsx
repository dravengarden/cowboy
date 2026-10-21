/** Product components with isolated HTTP/socket fixtures; no Provider calls. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceContext } from "./surface/SurfaceProfile";
import { classifySurface } from "./surface/profile";
import { SessionContextSettings } from "./SessionContextSettings";
import { announceContext, type SessionContext } from "./sessionContext";
import { shouldShowSessionErrorSnackbar } from "./sessionErrorNotice";
import { frontPrompt, submitPrompt, useStore } from "./store";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import { managementEntryFixture } from "./providerManagement.fixture";
import { resetProviderCatalog } from "./providerCatalogRegistry";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import type { MachineSummary, SessionMeta } from "./protocol";

export async function runSessionContextBrowserConformance(
  observe?: (stage: string) => Promise<void>,
): Promise<string[]> {
  const realFetch = globalThis.fetch;
  const realSocket = globalThis.WebSocket;
  const checks: string[] = [];
  const entry = managementEntryFixture("claude-code");
  entry.publisher = entry.manifest.publisher;
  const sessions: SessionMeta[] = [
    "Current",
    "API decisions",
    "Mobile findings",
  ].map((title, index) => ({
    id: `context-fixture-${index}`,
    title,
    provider: "claude-code",
    provider_account_id: index === 1 ? "mink" : "default",
    machine_id: "hawk",
    cwd: `/worktrees/${index}`,
    workspace_id: "project",
    workspace_name: "Cowboy",
    status: "exited",
    owner_user_id: "context-fixture",
  }));
  const machine = {
    id: "hawk",
    display_name: "Hawk",
    platform: "linux",
    architecture: "x86_64",
    status: "online",
    components: [],
    capacity: { max_sessions: 20, draining: false },
    active_sessions: 0,
    local: true,
    connected: true,
    schedulable: true,
    workspaces: [{
      id: "project",
      display_name: "Cowboy",
      canonical_path: "/project",
    }],
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
  let context: SessionContext;
  let sendFailure: number | null = null;
  const writes: { url: string; body: Record<string, unknown> }[] = [];
  class Socket {
    static active: Socket[] = [];
    static OPEN = 1;
    static CONNECTING = 0;
    static CLOSED = 3;
    readyState = 1;
    protocol = "cowboy-sync-v1";
    onopen: (() => void) | null = null;
    onmessage: ((event: { data: string }) => void) | null = null;
    onclose: (() => void) | null = null;
    onerror = null;
    constructor() {
      Socket.active.push(this);
      setTimeout(() => {
        this.onopen?.();
        this.publish({ type: "sessions", sessions });
        this.publish({ type: "machines", machines: [machine] });
        this.publish({ type: "bootstrap_complete" });
      }, 10);
    }
    publish(value: unknown) {
      this.onmessage?.({ data: JSON.stringify(value) });
    }
    send() {}
    close() {
      this.readyState = 3;
    }
  }
  globalThis.WebSocket = Socket as unknown as typeof WebSocket;
  bindProductSyncPrincipal("context-fixture");
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    if (!url.startsWith("/api/")) return realFetch(input, init);
    if (url === "/api/sync/dataset") {
      return Response.json({
        schema: "dravengarden.cowboy.product-sync-dataset/v1",
        dataset_id: `dataset-${"c".repeat(64)}`,
        user_id: "context-fixture",
        database_version: 2,
        outbox_contract: "atomic-delta-v1",
      });
    }
    if (url.endsWith("/context/send")) {
      const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
      writes.push({ url, body });
      if (sendFailure) {
        return new Response("Payload too large", { status: sendFailure });
      }
      context.revision++;
      context.applied_revision = context.revision;
      context.next_agent = null;
      context.sources = [];
      context.delivery = {
        operation: String(body.operation),
        session_id: "fresh-destination",
        state: "accepted",
      };
      return Response.json(context.delivery);
    }
    if (url.endsWith("/context")) {
      if (init?.method === "PUT") {
        const body = JSON.parse(String(init.body)) as Record<string, unknown>;
        writes.push({ url, body });
        if (body.revision !== context.revision) {
          return Response.json({ error: "Context changed" }, { status: 409 });
        }
        context = {
          ...context,
          sources: body.sources as string[],
          next_agent: body.next_agent as SessionContext["next_agent"],
          revision: context.revision + 1,
          delivery: null,
        };
      }
      return Response.json(context);
    }
    if (url === "/api/plugins") {
      return Response.json({
        providers: [entry],
        authentications: [],
        authentication_executors: [],
      });
    }
    if (url === "/api/project-policies") {
      return Response.json({
        schema: 1,
        revision: "fixture",
        default_runtime_machine_id: "hawk",
        machines: {},
      });
    }
    if (url.startsWith("/api/project-placements")) {
      return Response.json({
        machine_id: "hawk",
        default_runtime_machine_id: "hawk",
        placements: [{
          runtime_machine_id: "hawk",
          provider: "claude-code",
          mode: "local",
        }],
      });
    }
    if (url === "/api/provider-accounts") {
      return Response.json({
        dataset: {
          dataset_id: `dataset-${"c".repeat(64)}`,
          user_id: "context-fixture",
        },
        display_preferences: {
          scope: "subscriptions",
          revision: 1,
          values: { legacy_labels: { "claude-code": "Claude · Lynx" } },
        },
        registry: {
          revision: 1,
          accounts: [{
            id: "mink",
            provider_id: "claude-code",
            label: "Claude · Mink",
            enabled: true,
          }],
          defaults: {},
        },
        session_admission: "enabled",
        session_providers: ["claude-code"],
      });
    }
    if (url.endsWith("/bootstrap")) return Response.json({ messages: [] });
    return new Response("fixture unavailable", { status: 404 });
  }) as typeof fetch;
  resetProviderCatalog();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const check = (ok: unknown, name: string) => {
    if (!ok) throw new Error(name);
    checks.push(name);
  };
  const until = async (ready: () => boolean) => {
    for (let attempt = 0; attempt < 150; attempt++) {
      if (ready()) return;
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    throw new Error(`Context fixture timeout: ${container.textContent}`);
  };
  const button = (text: string) =>
    [...container.querySelectorAll<HTMLButtonElement>("button")].find((
      button,
    ) => button.textContent?.includes(text))!;
  const openChoices = async () => {
    const input = container.querySelector<HTMLInputElement>(
      'input[role="combobox"]',
    )!;
    input.focus();
    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }),
    );
    await until(() => document.querySelector('[role="listbox"]') !== null);
  };
  const option = (text: string) =>
    [...document.querySelectorAll<HTMLElement>('[role="option"]')].find((
      option,
    ) => option.textContent?.includes(text))!;
  function Fixture({ desktop }: { desktop: boolean }) {
    const state = useStore();
    return (
      <SurfaceContext.Provider
        value={classifySurface({
          finePointer: desktop,
          hover: desktop,
          tabletWidth: false,
          touchCapable: !desktop,
          desktopHost: desktop,
        })}
      >
        <BrowserProductTheme>
          <CssBaseline />
          <output
            hidden
            data-context-error
            data-visible-at-destination={shouldShowSessionErrorSnackbar(
              state.lastError,
              context.delivery?.session_id,
              0,
            )}
          >
            {state.lastError?.message}
          </output>
          <SessionContextSettings session={sessions[0]!} />
        </BrowserProductTheme>
      </SurfaceContext.Provider>
    );
  }
  try {
    for (const desktop of [false, true]) {
      const mode = desktop ? "desktop" : "mobile";
      sessions.forEach((session, index) => {
        session.id = `context-fixture-${mode}-${index}`;
      });
      context = {
        revision: 0,
        applied_revision: 0,
        sources: [],
        next_agent: null,
        source_sessions: [],
        delivery: null,
        max_sources: 32,
      };
      writes.length = 0;
      for (const socket of Socket.active) {
        socket.publish({ type: "sessions", sessions });
      }
      container.style.cssText = `width:${
        desktop ? 680 : 360
      }px;max-width:100%;padding:16px;box-sizing:border-box;margin:auto`;
      flushSync(() => root.render(<Fixture key={mode} desktop={desktop} />));
      button("Context & next agent").click();
      await until(() =>
        Boolean(container.querySelector("[data-context-editor]"))
      );
      check(
        writes.length === 0,
        `${mode}: opening context starts no agent or writes`,
      );
      await openChoices();
      option("API decisions").click();
      await openChoices();
      option("Mobile findings").click();
      document.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
      await until(() =>
        container.querySelectorAll(".MuiChip-root").length === 2
      );
      check(
        container.querySelectorAll(".MuiChip-root").length === 2,
        `${mode}: independent sources can be selected together`,
      );
      check(
        !container.textContent?.includes("credentials"),
        `${mode}: context UI exposes no credential controls`,
      );
      button("Use on next message").click();
      await until(() =>
        context.revision === 1 &&
        !container.querySelector("[data-context-editor]")
      );
      check(
        writes.length === 1 && writes[0]!.url.endsWith("/context"),
        `${mode}: saving links only writes a preference`,
      );
      button("Context & next agent").click();
      await until(() =>
        Boolean(container.querySelector("[data-context-editor]"))
      );
      container.querySelector<HTMLInputElement>('input[type="checkbox"]')!
        .click();
      await until(() =>
        container.querySelectorAll(".MuiChip-root").length === 3
      );
      check(
        container.querySelectorAll(".MuiChip-root").length === 3,
        `${mode}: fresh agent defaults to linking the current session`,
      );
      await until(() => !button("Use on next message").disabled);
      container.querySelector<HTMLElement>(
        '[data-session-subscription] [role="combobox"]',
      )!
        .dispatchEvent(
          new MouseEvent("mousedown", { bubbles: true, button: 0 }),
        );
      await until(() => Boolean(option("Claude · Mink")));
      option("Claude · Mink").click();
      await observe?.(`${mode}-selection`);
      button("Use on next message").click();
      await until(() =>
        context.revision === 2 &&
        !container.querySelector("[data-context-editor]")
      );
      check(
        context.next_agent?.provider === "claude-code" && writes.length === 2,
        `${mode}: agent choice remains staged until send`,
      );
      check(
        context.next_agent?.provider_account_id === "mink" &&
          sessions[0]!.provider_account_id === "default",
        `${mode}: changing the next account preserves the current binding`,
      );
      await frontPrompt(sessions[0]!.id, "", []);
      check(
        writes.length === 2,
        `${mode}: an empty composer never creates a context send`,
      );
      sendFailure = 413;
      await submitPrompt(sessions[0]!.id, "Too large").catch(() => undefined);
      await until(() =>
        Boolean(
          container.querySelector("[data-context-error]")?.textContent
            ?.includes("too large"),
        )
      );
      check(
        context.revision === 2,
        `${mode}: rejected sends publish a visible error without consuming context`,
      );
      sendFailure = null;
      await submitPrompt(sessions[0]!.id, "Continue the selected work");
      check(
        writes.some((write) =>
          write.url.endsWith("/send") && Boolean(write.body.operation)
        ),
        `${mode}: first send uses the staged destination operation`,
      );
      check(
        writes.filter((write) => write.url.endsWith("/send")).length === 2 &&
          sessions[0]!.status === "exited",
        `${mode}: sending preserves the original agent`,
      );
      context = {
        ...context,
        delivery: { ...context.delivery!, operation: `unseen-${mode}` },
      };
      announceContext(sessions[0]!.id);
      await submitPrompt(sessions[0]!.id, "Potential duplicate").catch(() =>
        undefined
      );
      await until(() =>
        Boolean(
          container.querySelector("[data-context-error]")?.textContent
            ?.includes("previous message"),
        )
      );
      check(
        container.querySelector("[data-context-error]")?.getAttribute(
              "data-visible-at-destination",
            ) === "true" &&
          writes.filter((write) => write.url.endsWith("/send")).length === 2,
        `${mode}: lost-reply navigation explains the result on the destination without resending`,
      );
      button("Context & next agent").click();
      await until(() =>
        Boolean(container.querySelector("[data-context-editor]"))
      );
      context = { ...context, revision: context.revision + 1 };
      window.dispatchEvent(
        new CustomEvent(PREFERENCES_CHANGED, {
          detail: { [`session_context:${sessions[0]!.id}`]: 30 },
        }),
      );
      await until(() =>
        Boolean(container.textContent?.includes("changed on another device"))
      );
      check(
        button("Use on next message").disabled,
        `${mode}: concurrent edits do not overwrite an open form`,
      );
      await observe?.(`${mode}-conflict`);
      check(
        container.scrollWidth <= container.clientWidth + 1,
        `${mode}: controls fit the available width`,
      );
      flushSync(() => root.render(null));
    }
  } finally {
    root.unmount();
    container.remove();
    globalThis.fetch = realFetch;
    globalThis.WebSocket = realSocket;
  }
  return checks;
}
