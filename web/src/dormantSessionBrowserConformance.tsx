/** Real session controls and store; isolated transport, no deployed sessions. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { DesktopCommandProvider } from "./desktop/commands/DesktopCommandProvider";
import { DesktopWorkspaceProvider } from "./desktop/DesktopWorkspaceController";
import { DesktopTopBarControls } from "./desktop/DesktopTopBarControls";
import { SessionControls } from "./Composer";
import { Transcript } from "./Transcript";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import { managementEntryFixture } from "./providerManagement.fixture";
import {
  loadProviderCatalog,
  resetProviderCatalog,
} from "./providerCatalogRegistry";
import { openSession, useStore } from "./store";
import { openSessionSettings } from "./sessionSettingsOpen";
import { type SessionToolsOverride, withMemoryKey } from "./agentTools";
import type { ConfigOption, Envelope, SessionMeta } from "./protocol";

export async function runDormantSessionBrowserConformance(
  keepFixture = false,
): Promise<string[]> {
  const originalFetch = globalThis.fetch;
  const originalSocket = globalThis.WebSocket;
  const entry = managementEntryFixture("example");
  entry.publisher = entry.manifest.publisher;
  entry.manifest.authentication.required = false;
  entry.manifest.authentication.methods = [];
  entry.manifest.configuration.presets = [
    {
      id: "default",
      name: "Standard",
      detail: "Model default",
      is_default: true,
      values: { model: "small" },
    },
    {
      id: "large",
      name: "Large",
      detail: "Larger model",
      is_default: false,
      values: { model: "large" },
    },
  ];
  const session: SessionMeta = {
    id: "sleeping",
    title: "Ready to configure",
    provider: "example",
    cwd: "/fixture",
    status: "exited",
    provider_version: entry.provider_version,
    provider_generation_digest: entry.artifact_digest!,
  };
  let options: ConfigOption[] = [];
  let timeline: Envelope[] = [];
  let toolsOverride: SessionToolsOverride = { schema: 1 };
  let holdToolsWrite = false;
  let releaseToolsWrite: (() => void) | undefined;
  let socket: FixtureSocket | undefined;
  const commands: string[] = [];
  const requests: string[] = [];
  const results: string[] = [];
  const check = (ok: unknown, message: string): void => {
    if (!ok) throw new Error(message);
    results.push(message);
  };
  const until = async (
    predicate: () => boolean,
    message: string,
  ): Promise<void> => {
    for (let i = 0; i < 150; i++) {
      if (predicate()) return;
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    throw new Error(
      `${message}; socket=${Boolean(socket)}; requests=${
        JSON.stringify(requests)
      }; commands=${commands.join(",")}; body=${
        document.body.textContent?.slice(0, 300)
      }`,
    );
  };
  class FixtureSocket {
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
      socket = this;
      setTimeout(() => {
        this.onopen?.();
        this.publish({ type: "sessions", sessions: [session] });
        this.publish({ type: "bootstrap_complete" });
      }, 20);
    }
    publish(message: unknown): void {
      this.onmessage?.({ data: JSON.stringify(message) });
    }
    send(data: string): void {
      const command = JSON.parse(data);
      commands.push(command.type);
      if (command.type === "set_config_option") {
        options = options.map((option) =>
          option.id === command.config_id
            ? { ...option, currentValue: command.value }
            : option
        );
        this.publish({
          type: "config_options",
          session_id: session.id,
          options,
        });
      }
      if (command.type === "reset_session") {
        timeline = [{
          seq: 1,
          kind: "update",
          session_id: session.id,
          update: { sessionUpdate: "context_cleared" },
        }];
        this.publish({
          type: "snapshot",
          session_id: session.id,
          events: timeline,
          reached_start: true,
        });
      }
    }
    close(): void {
      this.readyState = 3;
    }
  }
  globalThis.WebSocket = FixtureSocket as unknown as typeof WebSocket;
  bindProductSyncPrincipal("dormant-fixture");
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input);
    if (!path.startsWith("/api/")) return originalFetch(input, init);
    requests.push(`${init?.method ?? "GET"} ${path}`);
    if (path === "/api/sync/dataset") {
      return Response.json({
        schema: "dravengarden.cowboy.product-sync-dataset/v1",
        dataset_id: `dataset-${"a".repeat(64)}`,
        user_id: "dormant-fixture",
        database_version: 2,
        outbox_contract: "atomic-delta-v1",
      });
    }
    if (path === "/api/plugins") {
      return Response.json({
        providers: [entry],
        authentications: [],
        authentication_executors: [],
      });
    }
    if (path.endsWith("/bootstrap")) {
      return Response.json({
        messages: [
          {
            type: "snapshot",
            session_id: session.id,
            events: timeline,
            reached_start: true,
          },
          { type: "config_options", session_id: session.id, options },
        ],
      });
    }
    if (path === "/api/sessions/sleeping/tools") {
      if (init?.method === "PUT" && holdToolsWrite) {
        await new Promise<void>((resolve) => {
          releaseToolsWrite = resolve;
        });
      }
      if (init?.method === "PUT") {
        toolsOverride = withMemoryKey(JSON.parse(String(init.body)));
      }
      const defaults = {
        schema: 1,
        memory: { tools: true, recall: true },
        calls: {
          enabled: false,
          targets: [],
          default: "auto",
          max_concurrent: 2,
          max_per_session: 32,
        },
      };
      return Response.json({
        session_id: session.id,
        agent: session.provider,
        defaults,
        defaults_customized: false,
        override: toolsOverride,
        effective: {
          ...defaults,
          memory: { ...defaults.memory, ...toolsOverride.memory },
          calls: { ...defaults.calls, ...toolsOverride.calls },
        },
        catalog: { call_targets: [] },
      });
    }
    if (path === "/api/usage") {
      return Response.json({ providers: [], observed_at_ms: Date.now() });
    }
    return new Response("Unavailable in isolated fixture", { status: 404 });
  }) as typeof fetch;
  resetProviderCatalog();
  await loadProviderCatalog();
  const container = document.createElement("div");
  container.style.cssText =
    "height:700px;max-height:100vh;width:390px;max-width:100%;margin:auto";
  document.body.append(container);
  const root = createRoot(container);
  let desktop = false;
  function Fixture(): React.JSX.Element {
    const state = useStore();
    return (
      <SurfaceProvider>
        <BrowserProductTheme>
          <CssBaseline />
          <DesktopWorkspaceProvider>
            <DesktopCommandProvider>
              <Box
                sx={{
                  height: "100%",
                  display: "flex",
                  flexDirection: "column",
                }}
              >
                <Transcript
                  sessionId={session.id}
                  provider={session.provider}
                  cwd={session.cwd}
                  status="exited"
                  timeline={state.timelines.get(session.id) ?? []}
                  loading={false}
                  connected={state.connected}
                  desktopNavigation={desktop}
                />
                {desktop
                  ? (
                    <DesktopTopBarControls
                      sessionId={session.id}
                      status="exited"
                    />
                  )
                  : <SessionControls sessionId={session.id} status="exited" />}
              </Box>
            </DesktopCommandProvider>
          </DesktopWorkspaceProvider>
        </BrowserProductTheme>
      </SurfaceProvider>
    );
  }
  const click = (selector: string): void => {
    const target = document.querySelector<HTMLElement>(selector);
    if (!target) throw new Error(`Missing ${selector}`);
    target.click();
  };
  try {
    flushSync(() => root.render(<Fixture />));
    await until(
      () => container.querySelector("button[aria-label='options']") !== null,
      "settings trigger missing",
    );
    openSession(session.id);
    check(
      !container.querySelector<HTMLButtonElement>(
        "button[aria-label='options']",
      )!.disabled,
      "settings stay available before any Provider options exist",
    );
    await until(
      () =>
        container.querySelector("[data-conversation-empty-state='ready']") !==
          null,
      "new session empty state missing",
    );
    check(
      !container.textContent?.includes("send a message to wake"),
      "new sleeping session is a usable empty workspace",
    );
    click("[data-conversation-empty-settings]");
    await until(
      () => document.body.textContent?.includes("Session actions") === true,
      "empty state failed to open settings",
    );
    check(
      !commands.some((type) =>
        ["prompt", "reset_session", "set_config_option"].includes(type)
      ) &&
        !requests.some((request) => request.includes("/reload")),
      "opening settings performs no execution or mutation",
    );
    await until(
      () =>
        document.querySelectorAll("input[aria-label='Memory tools']").length ===
          2,
      "both tools surfaces missing",
    );
    holdToolsWrite = true;
    [...document.querySelectorAll<HTMLInputElement>(
      "input[aria-label='Memory tools']",
    )].at(-1)!.click();
    await until(
      () =>
        releaseToolsWrite !== undefined &&
        [...document.querySelectorAll<HTMLInputElement>(
          "input[aria-label='Memory tools']",
        )].every((input) => input.disabled),
      "a pending write did not lock both tools editors",
    );
    flushSync(() => root.render(<Fixture key="pending-tools" />));
    openSessionSettings("agent", session.id);
    await until(() => {
      const fields = [
        ...document.querySelectorAll<HTMLInputElement>(
          "input[aria-label='Memory tools']",
        ),
      ];
      return fields.length === 2 && fields.every((input) => input.disabled);
    }, "newly opened tools editors must inherit the in-flight lock");
    check(
      requests.filter((request) =>
        request === "PUT /api/sessions/sleeping/tools"
      ).length === 1,
      "a slow save locks all tools editors, including newly opened settings",
    );
    holdToolsWrite = false;
    releaseToolsWrite!();
    await until(
      () =>
        [...document.querySelectorAll<HTMLInputElement>(
          "input[aria-label='Memory tools']",
        )].every((input) => !input.checked && !input.disabled),
      "tools surfaces did not receive the saved settings",
    );
    check(
      !commands.includes("prompt"),
      "tools changes in the settings sheet update the start page without activation",
    );
    // Simulate the Controller's workerless signed configuration projection.
    options = [{
      id: "model",
      name: "Model",
      category: "model",
      currentValue: "small",
      options: [{ value: "small", name: "Small" }, {
        value: "large",
        name: "Large",
      }],
    }];
    socket!.publish({
      type: "config_options",
      session_id: session.id,
      options,
    });
    await until(
      () => document.querySelector("[data-run-config-preset='large']") !== null,
      "prelaunch presets missing",
    );
    click("[data-run-config-preset='large']");
    await until(
      () =>
        document.querySelector("[data-run-config-preset='large']")
          ?.getAttribute("aria-pressed") === "true",
      "selection was not acknowledged",
    );
    check(
      commands.includes("set_config_option") && !commands.includes("prompt"),
      "preset selection saves without sending a message",
    );
    click("[aria-label='Expand session actions']");
    await until(
      () =>
        document.querySelector(
          "[aria-label='clear conversation from session settings']",
        ) !== null,
      "Clear unavailable",
    );
    click("[aria-label='clear conversation from session settings']");
    await until(
      () => document.body.textContent?.includes("Clear conversation?") === true,
      "Clear confirmation missing",
    );
    const confirmation = [...document.querySelectorAll("[role='dialog']")].at(
      -1,
    );
    const confirm = [
      ...(confirmation?.querySelectorAll<HTMLButtonElement>("button") ?? []),
    ]
      .find((button) => button.textContent?.startsWith("Clear"));
    if (!confirm) throw new Error("Clear confirm unavailable");
    confirm.click();
    await until(() => commands.includes("reset_session"), "Clear was not sent");
    check(
      !commands.includes("prompt") &&
        !requests.some((request) => request.includes("/reload")),
      "Clear uses the reset path without prompting or reloading",
    );
    // The start page exposes every advertised option, including future Plugin
    // options and booleans, without opening the settings sheet or waking up.
    options = [...options, {
      id: "permission_mode",
      name: "Permissions",
      currentValue: "ask",
      options: [{ value: "ask", name: "Ask before changes" }, {
        value: "auto",
        name: "Allow workspace changes",
      }],
    }, {
      id: "reasoning_effort",
      name: "Reasoning",
      currentValue: "medium",
      options: [{ value: "medium", name: "Medium" }, {
        value: "high",
        name: "High",
      }],
    }, {
      id: "fast",
      name: "Fast mode",
      currentValue: false,
      options: [{ value: false, name: "Off" }, { value: true, name: "On" }],
    }, {
      id: "future_plugin_option",
      name: "Planning",
      currentValue: "adaptive",
      options: [{ value: "adaptive", name: "Adaptive" }, {
        value: "always",
        name: "Always plan",
      }],
    }];
    socket!.publish({
      type: "config_options",
      session_id: session.id,
      options,
    });
    flushSync(() => root.render(<Fixture key="inline-settings" />));
    await until(
      () =>
        container.querySelectorAll("select[data-start-option]").length === 5,
      "inline options missing",
    );
    check(
      container.querySelector(
        "[data-session-start-settings] [data-session-tools]",
      ) !== null,
      "tools are available directly on the start page",
    );
    const choose = async (selector: string, value: string): Promise<void> => {
      await until(
        () =>
          container.querySelector<HTMLSelectElement>(selector)?.disabled ===
            false,
        "inline selector unavailable",
      );
      const control = container.querySelector<HTMLSelectElement>(selector)!;
      control.value = value;
      control.dispatchEvent(new Event("change", { bubbles: true }));
      await until(
        () =>
          container.querySelector<HTMLSelectElement>(selector)?.value ===
            value &&
          !container.querySelector<HTMLSelectElement>(selector)?.disabled,
        "inline selection not acknowledged",
      );
    };
    await choose("select[data-start-option='fast']", "true");
    check(
      options.find((item) => item.id === "fast")?.currentValue === true,
      "inline boolean options keep their wire type",
    );
    await choose("select[data-start-option='future_plugin_option']", "always");
    check(
      options.find((item) => item.id === "future_plugin_option")
        ?.currentValue === "always",
      "new Plugin options work without a hard-coded UI list",
    );
    await choose("select[data-start-preset]", "default");
    check(
      options.find((item) => item.id === "model")?.currentValue === "small" &&
        !commands.includes("prompt"),
      "inline presets save acknowledged configuration without activation",
    );
    const option = container.querySelector<HTMLSelectElement>(
      "select[data-start-option='model']",
    )!;
    option.focus();
    const key = new KeyboardEvent("keydown", {
      key: " ",
      bubbles: true,
      cancelable: true,
    });
    option.dispatchEvent(key);
    check(
      !key.defaultPrevented && document.activeElement === option,
      "configuration keys stay with the field instead of focusing the composer",
    );
    container.style.width = "320px";
    await new Promise((resolve) => requestAnimationFrame(resolve));
    const panel = container.querySelector<HTMLElement>(
      "[data-session-start-settings]",
    )!;
    check(
      panel.scrollWidth <= panel.clientWidth + 1,
      "all start controls fit a narrow phone without horizontal scrolling",
    );
    container.style.width = "390px";
    desktop = true;
    flushSync(() => root.render(<Fixture key="desktop" />));
    openSessionSettings("agent", "another-session");
    await new Promise((resolve) => setTimeout(resolve, 60));
    check(
      document.querySelector("[role='dialog']") === null,
      "a settings action cannot open another pane",
    );
    openSessionSettings("agent", session.id);
    await until(
      () =>
        document.body.textContent?.includes(
          "Changes are saved for your next message",
        ) === true,
      "Desktop dormant configuration did not open",
    );
    check(
      document.body.textContent?.includes("Larger model"),
      "Desktop can configure the sleeping session too",
    );
    if (keepFixture) {
      desktop = false;
      flushSync(() => root.render(<Fixture key="mobile-preview" />));
      await until(
        () => document.querySelector("[role='dialog']") === null,
        "preview modal close",
      );
    }
    return results;
  } finally {
    if (!keepFixture) {
      root.unmount();
      container.remove();
      globalThis.fetch = originalFetch;
      globalThis.WebSocket = originalSocket;
      resetProviderCatalog();
    }
  }
}
