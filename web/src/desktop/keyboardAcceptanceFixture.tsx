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
