// Trusted-input fixture for the Desktop Review workbench
// (tools/cdp-review-acceptance.ts): the real DesktopWorkspace, command layer
// and workbench over the review-context fixture's socket and Code API.
import { useEffect, useState } from "react";
import { SurfaceContext } from "../../surface/SurfaceProfile";
import { subscribeDraftAppend } from "../../draftStore";
import { renderReviewContextPreview } from "../../reviewContextBrowserConformance";
import { DesktopCommandProvider } from "../commands/DesktopCommandProvider";
import { DesktopWorkspace } from "../DesktopWorkspace";
import { DesktopWorkspaceProvider, useDesktopWorkspace } from "../DesktopWorkspaceController";

function Prompt(): React.JSX.Element {
  const [text, setText] = useState("");
  useEffect(() => subscribeDraftAppend("worktree", (value) => setText((previous) => previous + value)), []);
  return (
    <div data-desktop-region="prompt.composer" style={{ flex: 1, display: "flex" }}>
      <textarea
        aria-label="Prompt"
        data-fixture-prompt
        data-desktop-focus-default
        value={text}
        onChange={(event) => setText(event.target.value)}
        style={{ flex: 1 }}
      />
    </div>
  );
}

function Probe(): null {
  (globalThis as unknown as Record<string, unknown>).__workspace = useDesktopWorkspace();
  return null;
}

export function mountDesktopReviewWorkbench(): void {
  (globalThis as { __fixturePullRequests?: boolean }).__fixturePullRequests = true;
  localStorage.removeItem("cowboy.desktop.workView.v1");
  localStorage.removeItem("cowboy.desktop.reviewContext.v1");
  localStorage.removeItem("cowboy.desktop.reviewLayout.v1");
  for (const key of Object.keys(localStorage)) {
    if (key.startsWith("cowboy.desktop.reviewTabs.v1:")) localStorage.removeItem(key);
  }
  const fixture = renderReviewContextPreview(true, false, true, true, false, false, () => (
    <SurfaceContext.Provider value={{ kind: "desktop", input: "keyboard-pointer", touchCapable: false, finePointer: true, hover: true }}>
      <DesktopWorkspaceProvider>
        <DesktopCommandProvider>
          <Probe />
          <div style={{ height: "100dvh", display: "flex" }}>
            <DesktopWorkspace
              promptWidth={320}
              resizing={false}
              onResizeStart={() => {}}
              prompt={<Prompt />}
              conversation={<div data-fixture-transcript>Transcript</div>}
              sessionId="worktree"
              projection="history"
              onProjectionChange={() => {}}
            />
          </div>
        </DesktopCommandProvider>
      </DesktopWorkspaceProvider>
    </SurfaceContext.Provider>
  ));
  (globalThis as unknown as Record<string, unknown>).__reviewFixture = fixture;
}
