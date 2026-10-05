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

/**
 * A held Desktop surface for trusted-input acceptance over CDP
 * (tools/cdp-keyboard-acceptance.ts): the real command provider and host
 * (leader, which-key, hint layer), a Vim-enabled shared CodeMirror composer
 * with its toolbar, a labelled list region and the real Create dialog. No
 * account, store connection or product endpoint.
 */
export function mountDesktopKeyboardAcceptance(): void {
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
