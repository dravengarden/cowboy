import { createRef, StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { Box, ButtonBase, CssBaseline, Typography } from "@mui/material";
import { BrowserProductTheme } from "../browserProductTheme";
import { SurfaceProvider } from "../surface/SurfaceProfile";
import type { ComposerEditorHandle } from "../ComposerEditor";
import { PlatformComposerEditor } from "../composer/PlatformComposerEditor";
import { CreateDialog } from "../App";
import { DesktopComposerToolbar } from "./DesktopComposerToolbar";
import {
  DesktopWorkspaceProvider,
  useDesktopWorkspace,
} from "./DesktopWorkspaceController";
import { DesktopCommandProvider } from "./commands/DesktopCommandProvider";
import { DesktopCommandHost } from "./commands/DesktopCommandHost";
import { WorkspaceDraftPane } from "../documents/WorkspaceDraft";
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
