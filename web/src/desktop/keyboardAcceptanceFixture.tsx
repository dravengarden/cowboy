import { createRef, StrictMode, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { Box, ButtonBase, CssBaseline, Typography } from "@mui/material";
import { BrowserProductTheme } from "../browserProductTheme";
import { SurfaceProvider } from "../surface/SurfaceProfile";
import type { ComposerEditorHandle } from "../ComposerEditor";
import { PlatformComposerEditor } from "../composer/PlatformComposerEditor";
import { CreateDialog } from "../App";
import { DesktopComposerToolbar } from "./DesktopComposerToolbar";
import { DesktopPermissionDock } from "./DesktopApprovalDock";
import type { PendingPermission } from "../derive";
import {
  DesktopWorkspaceProvider,
  useDesktopWorkspace,
} from "./DesktopWorkspaceController";
import { DesktopCommandProvider } from "./commands/DesktopCommandProvider";
import { DesktopCommandHost } from "./commands/DesktopCommandHost";
import { WorkspaceDraftPane } from "../documents/WorkspaceDraft";
import { DesktopTabStrip } from "./DesktopTabStrip";
import { DescriptionOutlined } from "@mui/icons-material";
import { StatusDot } from "../SessionStatusDot";
import {
  updateDesktopTabs,
  useDesktopTabs,
  withDesktopTab,
} from "./workspaceTabs";
import type { DraftFlush } from "../documents/DraftEditor";
import { draftRepository } from "../documents/store";
import { bindProductSyncPrincipal } from "../productSyncIdentity";
import {
  type DraftDocument,
  draftMetadata,
  type DraftMutationArgs,
  projectDraft,
} from "../documents/model";

/**
 * A held Desktop surface for trusted-input acceptance over CDP
 * (tools/cdp-keyboard-acceptance.ts): the real command provider and host
 * (leader, which-key, hint layer), a Vim-enabled shared CodeMirror composer
 * with its toolbar, a labelled list region and the real Create dialog. No
 * account, store connection or product endpoint.
 */
function desktopPointer(): void {
  // Real pointer/hover in a visible Chrome; headless engines report none.
  const media = globalThis.matchMedia;
  globalThis.matchMedia = (query) => {
    const pointer = query.includes("(pointer: fine)") ||
      query.includes("(hover: hover)");
    if (!pointer && !query.includes("(any-pointer: coarse)")) {
      return media.call(globalThis, query);
    }
    return {
      matches: pointer,
      media: query,
      onchange: null,
      addListener() {},
      removeListener() {},
      addEventListener() {},
      removeEventListener() {},
      dispatchEvent: () => true,
    };
  };
}

export function mountDesktopKeyboardAcceptance(): void {
  desktopPointer();
  const nativeFetch = globalThis.fetch;
  globalThis.fetch = (input, init) => String(input) === "/api/plugins"
    ? Promise.resolve(Response.json({ providers: [], authentications: [], authentication_executors: [] }))
    : nativeFetch(input, init);
  const container = document.createElement("div");
  container.style.cssText = "height:100vh;display:flex";
  document.body.append(container);
  const editorRef = createRef<ComposerEditorHandle>();
  const sendButtonRef = createRef<HTMLButtonElement>();
  const calls: string[] = [];
  const probe = globalThis as unknown as Record<string, unknown>;
  probe.__acceptance = { calls, editor: editorRef };
  function Surface(): React.JSX.Element {
    const workspace = useDesktopWorkspace();
    const [creating, setCreating] = useState(false);
    probe.__workspace = workspace;
    return (
      <>
        <DesktopCommandHost
          onNewSession={() => setCreating(true)}
          onOpenSettings={() => calls.push("settings")}
        />
        <Box
          data-desktop-pane="sessions"
          data-desktop-region="sessions.list"
          tabIndex={-1}
          sx={{ width: 260, borderRight: 1, borderColor: "divider", p: 1 }}
        >
          <Typography variant="overline">Sessions</Typography>
          <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
            {["Alpha", "Beta", "Gamma", "Delta", "Epsilon"].map((title) => (
              <li key={title}>
                <ButtonBase
                  data-desktop-item={title.toLowerCase()}
                  onClick={() => calls.push(`open:${title}`)}
                  sx={{ width: "100%", justifyContent: "flex-start", px: 4, py: 1.25 }}
                >
                  {title}
                </ButtonBase>
              </li>
            ))}
          </ul>
        </Box>
        <Box
          data-desktop-pane="prompt"
          sx={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}
        >
          <Box
            data-desktop-region="prompt.composer"
            tabIndex={-1}
            sx={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}
          >
            <PlatformComposerEditor
              ref={editorRef}
              value=""
              onChange={() => {}}
              onSubmit={() => {
                calls.push("send");
                return true;
              }}
              sessionId="keyboard-acceptance"
              commands={() => []}
              vim
              fill
              borderless
              endInset={0}
            />
            <DesktopComposerToolbar
              sessionId="keyboard-acceptance"
              status="running"
              editorRef={editorRef}
              sendButtonRef={sendButtonRef}
              canInsert
              sendable
              canJumpFront
              canForce
              pending={false}
              progress={false}
              sendLabel="Send"
              sendDescription="Send message"
              unavailableReason="Write a message first"
              overlayOpen={false}
              onAttach={() => calls.push("attach")}
              onSaveDraft={() => calls.push("draft")}
              onSchedule={() => calls.push("schedule")}
              onJumpFront={() => calls.push("next")}
              onForce={() => calls.push("force")}
              onSubmit={() => calls.push("send")}
            />
          </Box>
        </Box>
        <CreateDialog
          open={creating}
          onClose={() => setCreating(false)}
          onCreated={() => calls.push("created")}
        />
      </>
    );
  }
  createRoot(container).render(
    <StrictMode>
      <SurfaceProvider>
        <BrowserProductTheme>
          <CssBaseline />
          <DesktopWorkspaceProvider>
            <DesktopCommandProvider>
              <Surface />
            </DesktopCommandProvider>
          </DesktopWorkspaceProvider>
        </BrowserProductTheme>
      </SurfaceProvider>
    </StrictMode>,
  );
}

/**
 * The Conversation pane's approval dock beside a Vim composer: two waiting
 * tool permissions, answered by `␣1`…`␣9` from the reader and from Insert.
 * No socket exists, so an answer stays "not connected" until the runner
 * resolves the request, as `permission_resolved` would.
 */
export function mountApprovalKeyboardAcceptance(): void {
  desktopPointer();
  const container = document.createElement("div");
  container.style.cssText = "height:100vh;display:flex";
  document.body.append(container);
  const editorRef = createRef<ComposerEditorHandle>();
  const probe = globalThis as unknown as Record<string, unknown>;
  const request = (id: string, title: string): PendingPermission => ({
    seq: 0,
    requestId: id,
    requestKind: "permission",
    title,
    elicitation: null,
    options: [
      { optionId: "once", name: "Yes", kind: "allow_once" },
      {
        optionId: "always",
        name: "Yes, and don't ask again for scripts/mp",
        kind: "allow_always",
      },
      { optionId: "reject", name: "No", kind: "reject_once" },
    ],
  });
  function Surface(): React.JSX.Element {
    const workspace = useDesktopWorkspace();
    const [waiting, setWaiting] = useState<PendingPermission[]>([
      request(
        "first",
        "cd /home/draven/suger && W=/srv/storage/fast0/agent/worktrees/brazil-2p/marketplace-service; scripts/mp check $W 2>&1 | tail -5 && scripts/mp exec $W bash -c 'make swaggo-marketplace >/tmp/swag.log 2>&1; echo swaggo rc=$?; tail -2 /tmp/swag.log' 2>&1 | tail -3",
      ),
      request("second", "git status --short"),
      request("third", "ls -la"),
    ]);
    probe.__workspace = workspace;
    probe.__approval = {
      editor: editorRef,
      resolve: () => setWaiting((current) => current.slice(1)),
    };
    const item = waiting[0];
    return (
      <>
        <DesktopCommandHost onNewSession={() => {}} onOpenSettings={() => {}} />
        <Box
          data-desktop-pane="prompt"
          sx={{
            width: 420,
            display: "flex",
            flexDirection: "column",
            borderRight: 1,
            borderColor: "divider",
          }}
        >
          <Box
            data-desktop-region="prompt.composer"
            tabIndex={-1}
            sx={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}
          >
            <PlatformComposerEditor
              ref={editorRef}
              value=""
              onChange={() => {}}
              onSubmit={() => true}
              sessionId="keyboard-acceptance"
              commands={() => []}
              vim
              fill
              borderless
              endInset={0}
            />
          </Box>
        </Box>
        <Box
          data-desktop-pane="conversation"
          sx={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}
        >
          <Box
            data-desktop-region="conversation.transcript"
            data-desktop-navigation="scroll"
            data-desktop-focus-default
            tabIndex={-1}
            sx={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}
          >
            <Box
              data-approval-transcript
              sx={{ flex: 1, minHeight: 0, overflow: "auto", p: 2 }}
            >
              {Array.from({ length: 60 }, (_, index) => (
                <Typography key={index} variant="body2">
                  Transcript line {index + 1}
                </Typography>
              ))}
            </Box>
            {item && (
              <DesktopPermissionDock
                key={item.requestId}
                item={item}
                waiting={waiting.length}
                sessionId="keyboard-acceptance"
              />
            )}
          </Box>
        </Box>
      </>
    );
  }
  createRoot(container).render(
    <StrictMode>
      <SurfaceProvider>
        <BrowserProductTheme>
          <CssBaseline />
          <DesktopWorkspaceProvider>
            <DesktopCommandProvider>
              <Surface />
            </DesktopCommandProvider>
          </DesktopWorkspaceProvider>
        </BrowserProductTheme>
      </SurfaceProvider>
    </StrictMode>,
  );
}

/**
 * The real Desktop Draft page (WorkspaceDraftPane → DraftEditor, shared
 * editor, document toolbar and `␣D` group) on a local-first document; no
 * server is reachable, so the draft stays on this device.
 */
export async function mountDraftKeyboardAcceptance(): Promise<void> {
  desktopPointer();
  if (!bindProductSyncPrincipal("keyboard-acceptance")) {
    throw new Error("acceptance principal not bound");
  }
  // A tiny in-page Drafts server: the product dataset and draft mutations,
  // enough for the real local-first repository to save and synchronize.
  const server = new Map<string, DraftDocument>();
  const nativeFetch = globalThis.fetch;
  globalThis.fetch = (input, init) => {
    const path = String(input);
    if (!path.startsWith("/api/")) return nativeFetch(input, init);
    if (path === "/api/sync/dataset") {
      return Promise.resolve(Response.json({
        schema: "dravengarden.cowboy.product-sync-dataset/v1",
        dataset_id: `dataset-${"b".repeat(64)}`,
        user_id: "keyboard-acceptance",
        database_version: 2,
        outbox_contract: "atomic-delta-v1",
      }));
    }
    if (path === "/api/drafts") {
      return Promise.resolve(
        Response.json({ entries: [...server.values()].map(draftMetadata) }),
      );
    }
    if (path === "/api/drafts/mutations") {
      const args = JSON.parse(String(init?.body)) as DraftMutationArgs;
      const next = projectDraft(server.get(args.document_id) ?? null, {
        ...args,
        authored_at_ms: Date.now(),
      })!;
      server.set(next.id, next);
      return Promise.resolve(Response.json(next));
    }
    if (path.endsWith("/history")) return Promise.resolve(Response.json([]));
    const row = server.get(path.split("/").at(-1)!);
    return Promise.resolve(
      row ? Response.json(row) : Response.json({}, { status: 404 }),
    );
  };
  const id = await draftRepository().create("Acceptance draft", null);
  const container = document.createElement("div");
  container.style.cssText = "height:100vh;display:flex";
  document.body.append(container);
  const actions: string[] = [];
  const probe = globalThis as unknown as Record<string, unknown>;
  probe.__acceptance = { actions, id };
  const beforeLeave = { current: (async () => {}) as DraftFlush };
  function Surface(): React.JSX.Element {
    probe.__workspace = useDesktopWorkspace();
    return (
      <>
        <DesktopCommandHost
          onNewSession={() => actions.push("new")}
          onOpenSettings={() => actions.push("settings")}
          draftOpen
        />
        <Box
          data-desktop-pane="prompt"
          sx={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}
        >
          <WorkspaceDraftPane
            id={id}
            beforeLeave={beforeLeave}
            onAction={(action) => actions.push(action.action)}
          />
        </Box>
      </>
    );
  }
  createRoot(container).render(
    <StrictMode>
      <SurfaceProvider>
        <BrowserProductTheme>
          <CssBaseline />
          <DesktopWorkspaceProvider>
            <DesktopCommandProvider>
              <Surface />
            </DesktopCommandProvider>
          </DesktopWorkspaceProvider>
        </BrowserProductTheme>
      </SurfaceProvider>
    </StrictMode>,
  );
}

/**
 * The real tab strip and its commands over six fictitious items. The list on
 * the left opens an item the way the Sessions sidebar does (browsing), so the
 * preview tab, the leader keys and the context menu run as in the App.
 */
export function mountTabsKeyboardAcceptance(): void {
  desktopPointer();
  const container = document.createElement("div");
  container.style.cssText = "height:100vh;display:flex";
  document.body.append(container);
  const titles = ["Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Notes"];
  const keyOf = (title: string): string =>
    title === "Notes" ? "draft:notes" : title.toLowerCase();
  const probe = globalThis as unknown as Record<string, unknown>;
  const renamed: string[] = [];
  probe.__acceptance = { renamed };
  function Surface(): React.JSX.Element {
    probe.__workspace = useDesktopWorkspace();
    const tabs = useDesktopTabs();
    const [current, setCurrent] = useState("alpha");
    const [visits, setVisits] = useState<string[]>(["alpha"]);
    const from = useRef<string | null>(null);
    useEffect(() => {
      const previous = from.current;
      from.current = current;
      updateDesktopTabs((state) => withDesktopTab(state, current, previous));
      setVisits((keys) => [current, ...keys.filter((key) => key !== current)]);
    }, [current]);
    return (
      <>
        <DesktopCommandHost
          onNewSession={() => {}}
          onOpenSettings={() => {}}
        />
        <Box
          data-desktop-pane="sessions"
          data-desktop-region="sessions.list"
          tabIndex={-1}
          sx={{ width: 220, borderRight: 1, borderColor: "divider", p: 1 }}
        >
          <ul style={{ listStyle: "none", margin: 0, padding: 0 }}>
            {titles.map((title) => (
              <li key={title}>
                <ButtonBase
                  data-desktop-item={keyOf(title)}
                  data-acceptance-open={keyOf(title)}
                  onClick={() => setCurrent(keyOf(title))}
                  sx={{ width: "100%", justifyContent: "flex-start", px: 2, py: 1 }}
                >
                  {title}
                </ButtonBase>
              </li>
            ))}
          </ul>
        </Box>
        <Box
          data-desktop-pane="prompt"
          sx={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}
        >
          <DesktopTabStrip
            items={tabs.tabs.map((tab) => ({
              ...tab,
              kind: tab.key.startsWith("draft:") ? "draft" : "session",
              title: titles.find((title) => keyOf(title) === tab.key) ?? tab.key,
              // The App's own leading content: a status dot and a glyph.
              leading: (
                <>
                  <StatusDot status="running" />
                  <DescriptionOutlined data-desktop-tab-icon />
                </>
              ),
            }))}
            currentKey={current}
            visits={visits}
            exists={(key) => titles.some((title) => keyOf(title) === key)}
            onOpen={setCurrent}
            onRename={(key) => renamed.push(key)}
          />
          <Box
            data-desktop-region="prompt.composer"
            data-acceptance-body
            tabIndex={-1}
            sx={{ flex: 1, p: 2, outline: "none" }}
          >
            <Typography data-acceptance-current>{current}</Typography>
            <textarea data-acceptance-input aria-label="Prompt" />
          </Box>
        </Box>
      </>
    );
  }
  createRoot(container).render(
    <StrictMode>
      <SurfaceProvider>
        <BrowserProductTheme>
          <CssBaseline />
          <DesktopWorkspaceProvider>
            <DesktopCommandProvider>
              <Surface />
            </DesktopCommandProvider>
          </DesktopWorkspaceProvider>
        </BrowserProductTheme>
      </SurfaceProvider>
    </StrictMode>,
  );
}

/**
 * The Run configuration workbench over the real store, Top Bar controls and
 * command host, with an isolated socket and API: six presets, the usual
 * Provider options, linkable sessions and callable agents. Every
 * `set_config_option` is acknowledged and recorded in `__acceptance.sent`.
 */
export async function mountRunConfigKeyboardAcceptance(): Promise<void> {
  desktopPointer();
  const { DesktopTopBarControls } = await import("./DesktopTopBarControls");
  const { openSession } = await import("../store");
  const { openSessionSettings } = await import("../sessionSettingsOpen");
  const { managementEntryFixture } = await import(
    "../providerManagement.fixture"
  );
  const { loadProviderCatalog, resetProviderCatalog } = await import(
    "../providerCatalogRegistry"
  );
  const entry = managementEntryFixture("example");
  entry.publisher = entry.manifest.publisher;
  entry.manifest.authentication.required = false;
  entry.manifest.authentication.methods = [];
  const presetModels = [
    ["sonnet", "Sonnet"],
    ["opus", "Opus"],
    ["fable", "Fable"],
  ] as const;
  entry.manifest.configuration.presets = presetModels.flatMap((
    [model, name],
  ) =>
    (["high", "max"] as const).map((effort) => ({
      id: `${model}-${effort}`,
      name: `${name} · ${effort === "high" ? "High" : "Max"}`,
      detail: `${name} 5.5 · ${effort} reasoning`,
      is_default: model === "opus" && effort === "high",
      values: { model, reasoning_effort: effort },
    }))
  );
  const session = {
    id: "run",
    title: "Run configuration",
    provider: "example",
    cwd: "/fixture",
    status: "running" as const,
    provider_version: entry.provider_version,
    provider_generation_digest: entry.artifact_digest!,
  };
  const others = ["Mink", "Lynx", "Otter"].map((name, index) => ({
    ...session,
    id: `other-${String(index)}`,
    title: `cowboy improve · ${name}`,
    status: "exited" as const,
  }));
  let options = [{
    id: "mode",
    name: "Mode",
    currentValue: "bypass",
    options: [
      ["manual", "Manual"],
      ["accept", "Accept edits"],
      ["plan", "Plan"],
      ["auto", "Auto"],
      ["bypass", "Bypass permissions"],
    ].map(([value, name]) => ({ value: value!, name: name! })),
  }, {
    id: "model",
    name: "Model",
    category: "model",
    currentValue: "opus",
    options: presetModels.map(([value, name]) => ({
      value,
      name: `${name} 5.5`,
    })),
  }, {
    id: "reasoning_effort",
    name: "Reasoning effort",
    currentValue: "high",
    options: ["low", "medium", "high", "max"].map((value) => ({
      value,
      name: value[0]!.toUpperCase() + value.slice(1),
    })),
  }, {
    id: "fast",
    name: "Fast mode",
    currentValue: "off",
    options: [{ value: "on", name: "On" }, { value: "off", name: "Off" }],
  }];
  const sent: string[] = [];
  const requests: string[] = [];
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
      setTimeout(() => {
        this.onopen?.();
        this.publish({ type: "sessions", sessions: [session, ...others] });
        this.publish({ type: "bootstrap_complete" });
        this.publish({
          type: "config_options",
          session_id: session.id,
          options,
        });
      }, 20);
    }
    publish(message: unknown): void {
      this.onmessage?.({ data: JSON.stringify(message) });
    }
    send(data: string): void {
      const command = JSON.parse(data) as {
        type: string;
        config_id?: string;
        value?: string;
      };
      if (command.type !== "set_config_option") return;
      sent.push(`${command.config_id!}=${String(command.value)}`);
      options = options.map((option) =>
        option.id === command.config_id
          ? { ...option, currentValue: command.value! }
          : option
      );
      this.publish({
        type: "config_options",
        session_id: session.id,
        options,
      });
    }
    close(): void {
      this.readyState = 3;
    }
  }
  globalThis.WebSocket = FixtureSocket as unknown as typeof WebSocket;
  bindProductSyncPrincipal("run-config-fixture");
  let tools = {
    schema: 1 as const,
    memory: { tools: true, recall: true },
    calls: {
      enabled: true,
      targets: [{ agent: "codex" }],
      default: "auto",
      max_concurrent: 4,
      max_per_session: 64,
      images: true,
      self_images: false,
    },
  };
  const nativeFetch = globalThis.fetch;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input);
    if (!path.startsWith("/api/")) return nativeFetch(input, init);
    requests.push(`${init?.method ?? "GET"} ${path}`);
    if (path === "/api/sync/dataset") {
      return Response.json({
        schema: "dravengarden.cowboy.product-sync-dataset/v1",
        dataset_id: `dataset-${"b".repeat(64)}`,
        user_id: "run-config-fixture",
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
            events: [],
            reached_start: true,
          },
          { type: "config_options", session_id: session.id, options },
        ],
      });
    }
    if (path === "/api/sessions/run/tools") {
      if (init?.method === "PUT") {
        const override = JSON.parse(String(init.body)) as {
          memory?: typeof tools.memory;
          calls?: Partial<typeof tools.calls>;
        };
        tools = {
          ...tools,
          memory: { ...tools.memory, ...override.memory },
          calls: { ...tools.calls, ...override.calls },
        };
        sent.push(`tools=${JSON.stringify(override)}`);
      }
      return Response.json({
        session_id: session.id,
        agent: "claude",
        defaults: tools,
        defaults_customized: false,
        override: { schema: 1 },
        effective: tools,
        catalog: {
          call_targets: [{
            agent: "codex",
            presets: [{
              id: "deep",
              name: "Deep review",
              detail: "Highest reasoning",
              is_default: true,
            }],
            options: [],
          }],
        },
      });
    }
    if (path === "/api/sessions/run/context") {
      return Response.json({
        revision: 1,
        applied_revision: 1,
        sources: ["other-0"],
        next_agent: null,
        source_sessions: [{ id: "other-0", available: true }],
        pending_links: ["other-0"],
        delivery: null,
        max_sources: 8,
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
  container.style.cssText = "height:100vh;display:flex;flex-direction:column";
  document.body.append(container);
  const probe = globalThis as unknown as Record<string, unknown>;
  probe.__acceptance = { sent, requests };
  function Surface(): React.JSX.Element {
    probe.__workspace = useDesktopWorkspace();
    return (
      <>
        <DesktopCommandHost onNewSession={() => {}} onOpenSettings={() => {}} />
        <Box data-desktop-region="prompt.composer" tabIndex={-1} sx={{ p: 1 }}>
          <DesktopTopBarControls sessionId={session.id} status="running" />
        </Box>
      </>
    );
  }
  createRoot(container).render(
    <StrictMode>
      <SurfaceProvider>
        <BrowserProductTheme>
          <CssBaseline />
          <DesktopWorkspaceProvider>
            <DesktopCommandProvider>
              <Surface />
            </DesktopCommandProvider>
          </DesktopWorkspaceProvider>
        </BrowserProductTheme>
      </SurfaceProvider>
    </StrictMode>,
  );
  openSession(session.id);
  for (let i = 0; i < 200; i++) {
    if (document.querySelector("[data-desktop-topbar-action]")) break;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  openSessionSettings("agent", session.id);
}
