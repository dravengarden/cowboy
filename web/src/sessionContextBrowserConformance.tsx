/** Product components with isolated HTTP/socket fixtures; no Provider calls. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CssBaseline, Divider, Typography } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceContext } from "./surface/SurfaceProfile";
import { classifySurface } from "./surface/profile";
import { SessionContextSettings } from "./SessionContextSettings";
import { SessionContextCue, SessionContinuedFrom, SessionHandedOffDock, SessionProvenance } from "./SessionContextCue";
import {
  CLOSE_SESSION_SETTINGS_EVENT,
  COMPOSER_FOCUS_EVENT,
  requestNewFrom,
} from "./sessionSettingsOpen";
import { Sheet } from "./Sheet";
import { announceContext, type SessionContext } from "./sessionContext";
import { shouldShowSessionErrorSnackbar } from "./sessionErrorNotice";
import { frontPrompt, submitPrompt, useStore } from "./store";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import { managementEntryFixture } from "./providerManagement.fixture";
import { runCreateDefaultsBrowserChecks } from "./createDefaultsBrowserChecks";
import { resetProviderCatalog } from "./providerCatalogRegistry";
import type { MachineSummary, SessionMeta } from "./protocol";

export async function runSessionContextBrowserConformance(
  observe?: (stage: string) => Promise<void>,
): Promise<string[]> {
  const realFetch = globalThis.fetch;
  const realSocket = globalThis.WebSocket;
  const checks: string[] = [];
  const duplicateKeys: string[] = [];
  const entry = managementEntryFixture("claude-code");
  entry.publisher = entry.manifest.publisher;
  const sessions: SessionMeta[] = [
    "Current",
    "API decisions",
    "Mobile findings",
    ...Array.from({ length: 36 }, (_, index) => `History ${index + 1}`),
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
  let parentDismissals = 0;
  const writes: { url: string; body: Record<string, unknown> }[] = [];
  const contextReads: string[] = [];
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
    if (url.endsWith("/hibernate")) {
      writes.push({ url, body: {} });
      return new Response(null, { status: 204 });
    }
    if (url.endsWith("/fork") || url.endsWith("/duplicate")) {
      const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
      writes.push({ url, body });
      duplicateKeys.push(new Headers(init?.headers).get("idempotency-key") ?? "");
      return Response.json({
        session_id: url.endsWith("/fork") ? "fork-copy" : "linked-copy",
        session: null,
      }, { status: 201 });
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
      context.pending_links = [];
      context.delivery = {
        operation: String(body.operation),
        session_id: "fresh-destination",
        state: "accepted",
      };
      return Response.json(context.delivery);
    }
    if (url.endsWith("/context")) {
      contextReads.push(url);
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
          pending_links: body.sources as string[],
          applied_revision: !body.next_agent
            ? context.revision + 1
            : context.applied_revision,
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
    throw new Error(`Context fixture timeout after "${checks.at(-1)}": ${container.textContent}`);
  };
  const button = (text: string) =>
    [...container.querySelectorAll<HTMLButtonElement>("button")].find((
      button,
    ) => button.textContent?.includes(text))!;
  // The commit label "Switch" is also a prefix of the section toggle.
  const continueRun = () =>
    container.querySelector<HTMLButtonElement>("[data-new-session-continue-run]")!;
  const exactButton = (text: string) =>
    [...container.querySelectorAll<HTMLButtonElement>("button")].find((
      button,
    ) => button.textContent?.trim() === text)!;
  // Both surfaces open a picker: touch a sheet list, Desktop the Sessions
  // folder tree in a modal.
  const openChoices = async (_desktop: boolean) => {
    button("Select sessions").click();
    await until(() =>
      document.querySelector("[data-context-choices]") !== null
    );
  };
  const option = (text: string) =>
    [
      ...document.querySelectorAll<HTMLElement>(
        '[role="option"], [role="treeitem"][data-session-destination-session]',
      ),
    ].find((option) => option.textContent?.includes(text))!;
  const closeChoices = async (_desktop: boolean) => {
    [...document.querySelectorAll<HTMLButtonElement>(
      '[role="dialog"] button',
    )]
      .find((button) => button.textContent === "Done")!.click();
    await until(() => document.querySelector("[data-context-choices]") === null);
  };
  const searchChoices = async (query: string) => {
    const input = document.querySelector<HTMLInputElement>(
      '[aria-label="Search context sessions"]',
    )!;
    input.focus();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!
      .call(input, query);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    await new Promise((resolve) => setTimeout(resolve, 20));
  };
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
          <SessionContinuedFrom sessionId={sessions[3]!.id} />
          <SessionProvenance sessionId={sessions[3]!.id} />
          <SessionContinuedFrom sessionId={sessions[2]!.id} />
          <SessionHandedOffDock sessionId={sessions[1]!.id} desktop={desktop} />
          <SessionHandedOffDock sessionId={sessions[2]!.id} desktop={desktop} />
          <SessionContextCue sessionId={sessions[0]!.id} desktop={desktop} />
          <SessionContextCue sessionId={sessions[0]!.id} desktop={desktop} placement="end" />
          {desktop ? (
            <SessionContextSettings session={sessions[0]!} />
          ) : (
            <Sheet
              open
              cover
              forceSheet
              title="Session settings"
              onClose={() => {
                parentDismissals++;
              }}
            >
              <Box sx={{ py: 2 }}>
                <SessionContextSettings session={sessions[0]!} />
                <Divider sx={{ my: 2 }} />
                <Typography variant="overline">Session</Typography>
                <Typography>{sessions[0]!.title}</Typography>
              </Box>
            </Sheet>
          )}
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
      // Another tab's handoff: only the synced mark links sessions[1] here.
      sessions[3]!.continued_from = sessions[1]!.id;
      context = {
        revision: 0,
        applied_revision: 0,
        sources: [],
        next_agent: null,
        source_sessions: [],
        links: [{ id: "deleted-link", available: false, title: "Old work" }],
        delivery: null,
        max_sources: 32,
      };
      writes.length = 0;
      parentDismissals = 0;
      for (const socket of Socket.active) {
        socket.publish({ type: "sessions", sessions });
      }
      container.style.cssText = `width:${
        desktop ? 680 : 360
      }px;max-width:100%;padding:16px;box-sizing:border-box;margin:auto`;
      flushSync(() => root.render(<Fixture key={mode} desktop={desktop} />));
      await until(() => Boolean(button("Continue or fork")));
      await until(() =>
        contextReads.some((url) => url.includes(`/${sessions[3]!.id}/context`))
      );
      await until(() => Boolean(container.querySelector('[data-session-link="deleted"]')));
      check(
        container.querySelector('[data-session-link="deleted"]')?.textContent
          ?.includes("Old work") === true &&
          container.querySelector<HTMLButtonElement>('[data-session-link="deleted"]')!.disabled,
        `${mode}: a deleted link stays named and cannot be opened`,
      );
      check(
        Boolean(container.querySelector('[data-session-link="open"]')?.textContent
          ?.includes("Continued from")),
        `${mode}: links show how each session relates to this one`,
      );
      await until(() => Boolean(exactButton("Fork")));
      check(
        container.querySelectorAll("[data-session-handed-off]").length === 1 &&
          Boolean(exactButton("Move to Trash")),
        `${mode}: only a session that was continued is history, and it can be cleared away`,
      );
      exactButton("Fork").click();
      await until(() => writes.some((write) => write.url.endsWith("/duplicate")));
      check(
        writes.some((write) =>
          write.url.endsWith(`/${sessions[1]!.id}/duplicate`) &&
          write.body.provider_account_id === "mink"
        ),
        `${mode}: forking keeps this agent's subscription in a new session`,
      );
      await until(() => !button("Forking"));
      writes.length = 0;
      check(
        !contextReads.some((url) => url.includes(`/${sessions[2]!.id}/context`)),
        `${mode}: a reloaded handoff is read from its synced mark, an ordinary session is not`,
      );
      check(
        writes.length === 0,
        `${mode}: opening settings starts no agent or writes`,
      );
      check(
        !button("Save changes") && !button("Discard"),
        `${mode}: links have no save step`,
      );
      await openChoices(desktop);
      if (desktop) {
        check(
          document.querySelector(
            "[data-context-choices] [role=\"tree\"][aria-multiselectable=\"true\"]",
          ) !== null &&
            document.activeElement?.getAttribute("aria-label") ===
              "Search context sessions",
          "desktop: sources are chosen from the Sessions folder tree, search first",
        );
      }
      if (!desktop) {
        const panel = document.querySelector<HTMLElement>(
          "[data-context-choices]",
        )!;
        check(
          document.activeElement === panel && !container.contains(panel),
          "mobile: nested picker escapes the settings scroller without opening an input",
        );
        await until(() =>
          !panel.closest("[data-obsidian-sheet]")!.hasAttribute(
            "data-detent-moving",
          )
        );
        await observe?.("mobile-picker");
        const searchTop =
          panel.querySelector("input")!.getBoundingClientRect().top;
        await searchChoices("Mink");
        await until(() =>
          document.querySelectorAll('[role="option"]').length === 1
        );
        check(
          Boolean(option("API decisions")),
          "mobile: search finds the account as well as the session title",
        );
        check(
          Math.abs(
            panel.querySelector("input")!.getBoundingClientRect().top -
              searchTop,
          ) < 1,
          "mobile: filtering results does not move the search field",
        );
        await searchChoices("no matching context");
        check(
          Boolean(
            document.querySelector("[data-context-choices]")?.textContent
              ?.includes("No matching sessions"),
          ),
          "mobile: empty search has a visible result",
        );
        await searchChoices("");
        const input = panel.querySelector("input")!;
        input.dispatchEvent(
          new KeyboardEvent("keydown", {
            key: "Escape",
            isComposing: true,
            bubbles: true,
          }),
        );
        check(
          document.activeElement === input && panel.isConnected,
          "mobile: composition keys never dismiss the search or picker",
        );
        input.dispatchEvent(
          new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
        );
        check(
          document.activeElement === panel && panel.isConnected,
          "mobile: Escape leaves search before dismissing the picker",
        );
        input.focus();
        const oldInset = document.documentElement.style.getPropertyValue(
          "--kb-inset",
        );
        document.documentElement.style.setProperty("--kb-inset", "280px");
        try {
          const body = panel.parentElement!;
          const search = panel.querySelector("input")!;
          const dialog = panel.closest('[role="dialog"]')!;
          const done = [...dialog.querySelectorAll("button")].find((button) =>
            button.textContent === "Done"
          )!;
          await until(() => !dialog.hasAttribute("data-detent-moving"));
          body.scrollTop = body.scrollHeight;
          await new Promise((resolve) => requestAnimationFrame(resolve));
          check(
            body.scrollTop > 0 &&
              search.getBoundingClientRect().top >=
                body.getBoundingClientRect().top - 1,
            "mobile: long results scroll while search remains visible",
          );
          check(
            done.getBoundingClientRect().bottom <=
                window.innerHeight - 280 + 1 &&
              done.getBoundingClientRect().top >= 0,
            "mobile: Done stays above the simulated software keyboard",
          );
          await observe?.("mobile-keyboard");
          body.scrollTop = 0;
        } finally {
          if (oldInset) {
            document.documentElement.style.setProperty("--kb-inset", oldInset);
          } else document.documentElement.style.removeProperty("--kb-inset");
        }
        const order = () =>
          [...document.querySelectorAll('[role="option"]')].map((row) =>
            row.textContent
          ).join("\n");
        const initialOrder = order();
        option("History 17").click();
        await until(() =>
          option("History 17").getAttribute("aria-selected") === "true"
        );
        check(
          order() === initialOrder,
          "mobile: selecting a later row does not move results under the pointer",
        );
        option("History 17").click();
        await until(() =>
          option("History 17").getAttribute("aria-selected") === "false"
        );
        const done = [
          ...panel.closest('[role="dialog"]')!.querySelectorAll("button"),
        ].find((button) => button.textContent === "Done")!;
        done.focus();
        done.dispatchEvent(
          new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
        );
        await until(() => !document.querySelector("[data-context-choices]"));
        await new Promise((resolve) => setTimeout(resolve, 350));
        check(
          parentDismissals === 0 &&
            document.activeElement === button("Select sessions"),
          "mobile: Escape on the footer closes only the picker and restores the trigger",
        );
        await openChoices(desktop);
      }
      option("API decisions").click();
      await until(() =>
        option("API decisions")?.getAttribute("aria-selected") === "true"
      );
      option("Mobile findings").click();
      await until(() =>
        option("Mobile findings")?.getAttribute("aria-selected") === "true"
      );
      await closeChoices(desktop);
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
      await observe?.(`${mode}-links`);
      await until(() => context.sources.length === 2);
      check(
        writes.every((write) => write.url.endsWith("/context")) &&
          context.revision === context.applied_revision,
        `${mode}: each selection applies at once and starts nothing`,
      );
      await until(() =>
        Boolean(document.querySelector('[data-session-context-cue="pending_links"]'))
      );
      check(true, `${mode}: the composer says the next prompt reads them`);
      const beforeAgent = writes.length;
      button("Continue or fork").click();
      await until(() => Boolean(continueRun()) && !continueRun().disabled);
      check(
        !container.querySelector("[data-context-advanced]") &&
          container.querySelector("[data-new-session-continue]")?.textContent
              ?.includes("fresh conversation") === true,
        `${mode}: an identical target can still start a fresh conversation, with no extra fields`,
      );
      check(
        container.querySelector("[data-new-session-fork]")?.textContent
          ?.includes("uncommitted changes stay here") === true,
        `${mode}: the form says what happens to files`,
      );
      button("More options").click();
      await until(() =>
        Boolean(container.querySelector("[data-context-advanced]"))
      );
      check(
        container.querySelector("[data-context-advanced]")?.textContent
          ?.includes("Inherit current model & reasoning") === true,
        `${mode}: advanced preset defaults to current model and reasoning`,
      );
      button("Fewer options").click();
      container.querySelector<HTMLElement>(
        '[data-new-session-editor] [data-session-subscription] [role="combobox"]',
      )!
        .dispatchEvent(
          new MouseEvent("mousedown", { bubbles: true, button: 0 }),
        );
      await until(() => Boolean(option("Claude · Mink")));
      option("Claude · Mink").click();
      await until(() =>
        container.querySelector("[data-new-session-continue]")?.textContent
          ?.includes("moves the work") === true
      );
      check(true, `${mode}: another subscription explains that the work moves`);
      await observe?.(`${mode}-selection`);
      continueRun().click();
      await until(() =>
        Boolean(context.next_agent) &&
        !container.querySelector("[data-new-session-editor]")
      );
      check(
        context.next_agent?.provider === "claude-code" &&
          writes.length === beforeAgent + 1,
        `${mode}: agent choice remains staged until send`,
      );
      check(
        context.sources.length === 3 && context.sources[0] === sessions[0]!.id,
        `${mode}: continuing links the current session ahead of selected ones`,
      );
      check(
        context.next_agent?.provider_account_id === "mink" &&
          context.next_agent.inherit_settings === true &&
          sessions[0]!.provider_account_id === "default",
        `${mode}: changing the next account preserves the current binding`,
      );
      await until(() =>
        Boolean(
          document.querySelector('[data-session-context-cue="pending_agent"]')
            ?.textContent?.includes("Claude · Mink"),
        )
      );
      check(
        Boolean(
          document.querySelector('[data-session-context-cue="pending_agent"]')
            ?.textContent?.includes("Claude · Mink"),
        ),
        `${mode}: the composer cue names the pending agent's subscription`,
      );
      await until(() =>
        Boolean(container.querySelector("[data-session-switch-back]"))
      );
      check(
        !button("Save changes"),
        `${mode}: loading a saved agent is not an unsaved edit`,
      );
      await observe?.(`${mode}-pending`);
      await frontPrompt(sessions[0]!.id, "", []);
      check(
        writes.length === beforeAgent + 1,
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
        Boolean(context.next_agent),
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
      await until(() =>
        Boolean(
          document.querySelector('[data-session-context-cue="continued_in"]'),
        )
      );
      check(
        Boolean(
          document.querySelector(
            '[data-session-context-cue="continued_in"][data-session-provenance="end"]',
          ),
        ) &&
          !document.querySelector(
            '[data-composer-stack-slot="context"][data-session-context-cue^="continued"]',
          ),
        `${mode}: the original session links to where it continued from Settings, not the Composer`,
      );
      // An idle original agent can be hibernated straight from the cue.
      const idle = { ...sessions[0]!, status: "running" as const };
      Socket.active.at(-1)?.publish({
        type: "sessions",
        sessions: [idle, ...sessions.slice(1)],
      });
      Socket.active.at(-1)?.publish({
        type: "machines",
        machines: [{ ...machine, capabilities: { hibernation: true } }],
        resync: true,
      });
      await until(() =>
        Boolean(document.querySelector("[data-context-cue-hibernate]"))
      );
      document.querySelector<HTMLButtonElement>("[data-context-cue-hibernate]")!
        .click();
      await until(() =>
        writes.some((write) => write.url.endsWith("/hibernate"))
      );
      check(
        writes.filter((write) => write.url.endsWith("/hibernate")).length === 1,
        `${mode}: the original idle agent can be hibernated from the cue`,
      );
      Socket.active.at(-1)?.publish({ type: "sessions", sessions });
      Socket.active.at(-1)?.publish({
        type: "machines",
        machines: [machine],
        resync: true,
      });
      await until(() =>
        !document.querySelector("[data-context-cue-hibernate]")
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
      await openChoices(desktop);
      option("Mobile findings").click();
      await until(() => context.sources.length === 1);
      await until(() =>
        option("Mobile findings")?.getAttribute("aria-selected") === "true"
      );
      option("Mobile findings").click();
      await until(() => context.sources.length === 0);
      await until(() =>
        option("Mobile findings")?.getAttribute("aria-selected") === "false"
      );
      check(
        !context.next_agent && context.revision === context.applied_revision,
        `${mode}: removing a selection undoes it at once`,
      );
      option("API decisions").click();
      await until(() => context.sources.length === 1);
      await closeChoices(desktop);
      const surfaceEvents: string[] = [];
      const recordSurface = (event: Event) => surfaceEvents.push(event.type);
      document.addEventListener(CLOSE_SESSION_SETTINGS_EVENT, recordSurface);
      document.addEventListener(COMPOSER_FOCUS_EVENT, recordSurface);
      requestNewFrom(sessions[0]!.id);
      await until(() => Boolean(continueRun()) && !continueRun().disabled);
      check(
        Boolean(container.querySelector("[data-new-session-editor]")),
        `${mode}: the session menu opens the new session form`,
      );
      continueRun().click();
      await until(() => surfaceEvents.includes(COMPOSER_FOCUS_EVENT));
      check(
        surfaceEvents.includes(CLOSE_SESSION_SETTINGS_EVENT) &&
          surfaceEvents.includes(COMPOSER_FOCUS_EVENT) &&
          Boolean(context.next_agent),
        `${mode}: staging a new agent closes settings and returns to the composer`,
      );
      document.removeEventListener(CLOSE_SESSION_SETTINGS_EVENT, recordSurface);
      document.removeEventListener(COMPOSER_FOCUS_EVENT, recordSurface);
      await until(() =>
        Boolean(container.querySelector("[data-session-switch-back]"))
      );
      container.querySelector<HTMLButtonElement>("[data-session-switch-back]")!
        .click();
      await until(() => !context.next_agent);
      check(
        context.sources.length === 1 &&
          !context.sources.includes(sessions[0]!.id),
        `${mode}: Back cancels the new agent and keeps independent links`,
      );
      check(
        context.revision === context.applied_revision,
        `${mode}: links left after Back wait without holding anything`,
      );
      flushSync(() => root.render(null));
      const workspace = sessions[0]!.workspace_id;
      delete sessions[0]!.workspace_id;
      flushSync(() => root.render(<Fixture desktop={desktop} />));
      await until(() =>
        Boolean(button("Continue or fork")) &&
        !button("Continue or fork").disabled
      );
      button("Continue or fork").click();
      await until(() => Boolean(button("Choose project")));
      check(
        continueRun().disabled &&
          container.textContent?.includes(
              "Choose a project in More options",
            ) === true,
        `${mode}: missing inherited project explains why Continue is disabled`,
      );
      button("Choose project").click();
      await until(() =>
        Boolean(container.querySelector("[data-context-advanced]"))
      );
      check(
        Boolean(container.querySelector("[data-context-advanced]")),
        `${mode}: missing project action exposes the field`,
      );
      flushSync(() => root.render(null));
      if (workspace !== undefined) sessions[0]!.workspace_id = workspace;
    }
    // One form, two outcomes: a native fork on the same agent, a linked fork
    // on another subscription; neither touches this session's pending context.
    const forkMode = () =>
      container.querySelector("[data-new-session-fork]")?.getAttribute("data-new-session-fork");
    const forkRun = () =>
      container.querySelector<HTMLButtonElement>("[data-new-session-fork-run]")!;
    const openForm = async () => {
      await until(() =>
        Boolean(button("Continue or fork")) &&
        !button("Continue or fork").disabled
      );
      button("Continue or fork").click();
      await until(() => Boolean(forkMode()) && !forkRun().disabled);
    };
    flushSync(() => root.render(<Fixture desktop />));
    await openForm();
    check(
      forkMode() === "linked",
      "fork: without native fork on the Machine the fork is a linked new agent",
    );
    Socket.active.at(-1)?.publish({
      type: "machines",
      machines: [{ ...machine, capabilities: { hibernation: true, native_fork: true } }],
      resync: true,
    });
    await until(() => forkMode() === "fork");
    check(true, "fork: the same agent on a fork-capable Machine forks natively");
    const beforeFork = writes.length;
    forkRun().click();
    await until(() => writes.length > beforeFork);
    const forked = writes.at(-1)!;
    check(
      forked.url.endsWith("/fork") && forked.body.config === undefined,
      "fork: a native fork carries no configuration",
    );
    check(
      !writes.slice(beforeFork).some((write) => write.url.endsWith("/context")),
      "fork: the source's context and agent stay unchanged",
    );
    flushSync(() => root.render(null));
    flushSync(() => root.render(<Fixture desktop />));
    await openForm();
    button("More options").click();
    await until(() => Boolean(container.querySelector("[data-context-advanced]")));
    container.querySelector<HTMLInputElement>('[data-context-advanced] input[type="checkbox"]')!
      .click();
    await until(() => forkMode() === "linked");
    check(true, "fork: clearing inheritance is honored by a linked fork, not ignored");
    flushSync(() => root.render(null));
    flushSync(() => root.render(<Fixture desktop />));
    await openForm();
    container.querySelector<HTMLElement>("[data-new-session-editor] [data-session-subscription] [role=\"combobox\"]")!
      .dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    await until(() => Boolean(option("Claude · Mink")));
    option("Claude · Mink").click();
    await until(() => forkMode() === "linked");
    const beforeCopy = writes.length;
    forkRun().click();
    await until(() => writes.length > beforeCopy);
    const copied = writes.at(-1)!;
    check(
      copied.url.endsWith("/duplicate") && copied.body.provider_account_id === "mink" &&
        (duplicateKeys.at(-1) ?? "").startsWith("duplicate-"),
      "fork: another subscription creates a linked fork with an idempotency key",
    );
    flushSync(() => root.render(null));
    Socket.active.at(-1)?.publish({ type: "machines", machines: [machine], resync: true });
    await runCreateDefaultsBrowserChecks(sessions[1]!, "Claude · Mink");
  } finally {
    root.unmount();
    container.remove();
    globalThis.fetch = realFetch;
    globalThis.WebSocket = realSocket;
  }
  return checks;
}
