/** Actual Desktop tool inspector layout and image output in a real browser. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { BrowserProductTheme } from "./browserProductTheme";
import { ToolDetailsBrowser } from "./Transcript";
import type { RenderItem } from "./derive";
import { toolRuns } from "./tools/runs";

// 1x1 opaque PNG.
const PNG =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function readImage(key: string, image: Record<string, string>): RenderItem {
  return {
    key,
    kind: "tool",
    id: key,
    toolKind: "read",
    toolName: "Read",
    status: "completed",
    title: `Read /tmp/${key}.png`,
    rawInput: { file_path: `/tmp/${key}.png` },
    content: [{
      type: "content",
      content: { type: "image", mimeType: "image/png", ...image },
    }],
  };
}

export async function runToolInspectorBrowserConformance(): Promise<string[]> {
  const items: RenderItem[] = [];
  for (let index = 0; index < 40; index++) {
    items.push({
      key: `m${index}`,
      kind: "message",
      role: "assistant",
      chunks: [{ type: "text", text: `step ${index}` }],
    });
    const command =
      `cd /home/draven/.local/state/cowboy-machine/worktrees/sess-1791179743143 && rg -n pattern ${index}`;
    items.push({
      key: `t${index}`,
      kind: "tool",
      id: `t${index}`,
      toolKind: "execute",
      toolName: "Bash",
      status: "completed",
      title: command,
      rawInput: {
        command,
        description:
          "Inspect the focus state before Normal-mode Space reaches the leader group and the pending-input owner",
      },
      content: [{ type: "content", content: { type: "text", text: "ok" } }],
    });
  }
  items.push(readImage("inline", { data: PNG }));
  items.push(readImage("missing", { url: "/api/artifacts/missing.png" }));
  const runs = toolRuns(items);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const show = async (selectedKey: string): Promise<void> => {
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <ToolDetailsBrowser
            items={items}
            runs={runs}
            selectedKey={selectedKey}
            desktop
            provider="claude"
            onSelect={() => {}}
            onClose={() => {}}
            onLocate={() => {}}
            historyComplete={false}
          />
        </BrowserProductTheme>,
      )
    );
    // Details mount after the inspector's 320ms skeleton.
    await wait(700);
  };
  try {
    await show("t35");
    const nav = document.querySelector<HTMLElement>(
      "nav[aria-label='Tool run history']",
    );
    check(nav, "missing tool run history");
    check(
      nav.scrollWidth <= nav.clientWidth,
      `history overflows sideways: ${nav.scrollWidth} > ${nav.clientWidth}`,
    );
    check(nav.scrollLeft === 0, "history panned sideways to the selected run");

    await show("inline");
    const image = document.querySelector<HTMLImageElement>(
      "[data-tool-output-images] img",
    );
    check(image, "image Read did not render its image");
    for (let attempt = 0; attempt < 20 && !image.complete; attempt++) {
      await wait(50);
    }
    check(image.naturalWidth === 1, "image Read did not load its bytes");

    await show("missing");
    for (
      let attempt = 0;
      attempt < 40 && document.querySelector("[data-tool-output-images]");
      attempt++
    ) await wait(50);
    check(
      !document.querySelector("[data-tool-output-images]"),
      "an unloadable image stayed on screen",
    );
    check(
      document.body.textContent?.includes("No output"),
      "an unloadable image lost the text fallback",
    );
    return [
      "long history descriptions ellipsize without panning the rail",
      "an image Read shows its image",
      "an unloadable image Read falls back to the text presentation",
    ];
  } finally {
    flushSync(() => root.unmount());
    container.remove();
  }
}
