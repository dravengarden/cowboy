/** Actual Desktop tool inspector layout and image output in a real browser. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { ToolDetailsBrowser } from "./Transcript";
import { derive, type RenderItem } from "./derive";
import { toolRuns } from "./tools/runs";
import { ToolBody } from "./tools/registry";

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

export async function runToolInspectorBrowserConformance(
  keepFixture = false,
): Promise<string[]> {
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
  const mcp = (
    key: string,
    server: string,
    tool: string,
    args: Record<string, unknown>,
    rawOutput?: unknown,
  ): RenderItem => {
    const item = derive([{
      session_id: "fixture",
      seq: 1,
      kind: "update",
      update: {
        sessionUpdate: "tool_call",
        toolCallId: key,
        title: `mcp.${server}.${tool}`,
        kind: "search",
        status: "completed",
        rawInput: { server, tool, arguments: args },
        ...(rawOutput === undefined ? {} : { rawOutput }),
      },
    }])[0]!;
    return { ...item, key };
  };
  items.push(mcp("help", "cowboy", "cowboy_read", { help: "calls" }, {
    result: {
      content: [{
        type: "text",
        text: JSON.stringify({
          api: "cowboy-ts-v1",
          types: "interface Calls { list(): Promise<Call[]>; }",
        }),
      }],
    },
    error: null,
  }));
  items.push(
    mcp("program", "cowboy", "cowboy_read", {
      code: "return await cowboy.session.info();",
      language: "ts",
    }, { status: "busy", queued: 0 }),
  );
  items.push(
    mcp("action", "cowboy", "cowboy_execute", {
      action: "review.start",
      arguments: { title: "Review tool UX" },
      operation: "review-1",
    }, { receipt: { status: "accepted", operation: "review-1" } }),
  );
  items.push(
    mcp("memory", "matrix", "memory_search", { query: "tool UX" }, {
      content: [],
      structuredContent: {
        hits: [{
          title: "Tool result UX",
          snippet: "Preserve structured results",
          ref: { id: "memory-1", revision: 3 },
        }],
        truncated: false,
      },
    }),
  );
  items.push(mcp("absent", "matrix", "memory_get", { id: "historical" }));
  items.push(mcp("record", "matrix", "memory_get", { id: "memory-1" }, {
    structuredContent: {
      id: "memory-1",
      revision: 3,
      title: "Tool result UX",
      body: "Readable **memory body**",
      sources: [{ quote: "Supporting evidence" }],
    },
  }));
  items.push(
    mcp("error", "other-server", "lookup", { query: "test" }, {
      result: null,
      error: { message: "Service unavailable" },
    }),
  );
  items.push(mcp("empty", "cowboy", "cowboy_status", {}, { content: [] }));
  items.push(mcp("external-hits", "other-server", "search", { query: "file" }, {
    structuredContent: { hits: [{ title: "File search hit", snippet: "A file" }] },
  }));
  items.push(mcp("external-record", "other-server", "get", { id: "document" }, {
    structuredContent: {
      title: "External document", body: "Document body", id: "document", revision: 3,
    },
  }));
  const runs = toolRuns(items);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const show = async (selectedKey: string, desktop = true): Promise<void> => {
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <CssBaseline />
          <ToolDetailsBrowser
            items={items}
            runs={runs}
            selectedKey={selectedKey}
            desktop={desktop}
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
    await show("help");
    check(
      document.body.textContent?.includes("API reference"),
      "MCP help lost its typed reference",
    );
    check(
      document.body.textContent?.includes("Calls"),
      "MCP opaque help result disappeared",
    );
    check(
      document.querySelector("[aria-label='Tool detail view']"),
      "MCP search kind incorrectly forced Raw",
    );
    const raw = [...document.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "Raw");
    check(raw, "missing raw escape hatch");
    raw.click();
    await wait(150);
    check(
      document.body.textContent?.includes("tool_result"),
      "Raw omitted original opaque envelope",
    );
    const formatted = [
      ...document.querySelectorAll<HTMLButtonElement>("button"),
    ].find((button) => button.textContent === "Formatted");
    formatted?.click();
    await wait(150);
    check(
      document.body.textContent?.includes("API reference"),
      "Formatted result did not return",
    );

    await show("program");
    check(
      document.body.textContent?.includes("Program") &&
        document.body.textContent.includes("cowboy.session.info"),
      "CodeAct program not exposed",
    );
    await show("action");
    check(
      document.body.textContent?.includes("review.start") &&
        document.body.textContent.includes("accepted"),
      "operation/receipt missing",
    );
    await show("memory");
    check(
      document.querySelector("[data-memory-results]")?.textContent?.includes(
        "Preserve structured results",
      ),
      "structured-only memory results missing",
    );
    await show("record");
    check(
      document.querySelector("[data-memory-record] strong")?.textContent ===
        "memory body",
      "memory body lost Markdown rendering",
    );
    const evidence = document.querySelector<HTMLDetailsElement>(
      "[data-memory-record] details",
    );
    check(
      evidence && !evidence.open &&
        evidence.textContent?.includes("Supporting evidence"),
      "memory evidence not inspectable",
    );
    await show("absent");
    check(
      document.body.textContent?.includes(
        "No result was recorded for this call.",
      ),
      "missing history result conflated with empty return",
    );
    await show("empty");
    check(
      document.body.textContent?.includes("Tool returned an empty result."),
      "explicit empty result not explained",
    );
    await show("error");
    check(
      document.body.textContent?.includes("Service unavailable") &&
        document.body.textContent.includes("Tool reported an error"),
      "transport error hidden",
    );
    for (const [key, text] of [
      ["external-hits", "File search hit"],
      ["external-record", "External document"],
    ]) {
      await show(key!);
      check(
        document.body.textContent?.includes(text!) &&
          !document.querySelector("[data-memory-results], [data-memory-record]"),
        "another MCP server's data was mislabeled as memory",
      );
    }

    // Exercise the shared result/argument frame at phone, tablet and desktop
    // pane widths, including long operation IDs and structured values. The
    // actual product theme owns typography, touch targets and code wrapping.
    for (const width of [320, 390, 768, 1100]) {
      const fixture = document.createElement("div");
      fixture.style.width = `${width}px`;
      container.append(fixture);
      const fixtureRoot = createRoot(fixture);
      flushSync(() =>
        fixtureRoot.render(
          <BrowserProductTheme>
            <ToolBody
              ctx={{
                provider: "future-provider",
                toolName: "mcp__cowboy__cowboy_execute",
                kind: "execute",
                title: "MCP",
                rawInput: {
                  action: "preferences.update",
                  operation: "x".repeat(200),
                  arguments: { description: "long field ".repeat(30) },
                },
                content: {
                  structuredContent: { value: "multilingual 中文 ".repeat(40) },
                },
                running: false,
                status: "completed",
              }}
            />
          </BrowserProductTheme>,
        )
      );
      await wait(200);
      const overflow = [...fixture.querySelectorAll<HTMLElement>("*")].filter((
        node,
      ) =>
        node.getBoundingClientRect().right >
          fixture.getBoundingClientRect().right + 1
      ).slice(0, 4).map((node) =>
        `${node.tagName}.${node.className}:${node.getBoundingClientRect().width}`
      ).join("; ");
      check(
        fixture.scrollWidth <= width + 1,
        `MCP frame overflows ${width}px: ${fixture.scrollWidth}; ${overflow}`,
      );
      flushSync(() => fixtureRoot.unmount());
      fixture.remove();
    }
    if (keepFixture) await show("memory", window.innerWidth >= 1000);
    return [
      "long history descriptions ellipsize without panning the rail",
      "an image Read shows its image",
      "an unloadable image Read falls back to the text presentation",
      "MCP help and typed results retain Formatted and Raw views",
      "CodeAct programs and action receipts are readable",
      "structured-only memory hits render as cards",
      "memory bodies and evidence retain readable and inspectable views",
      "historical missing and explicit empty results are distinct",
      "transport errors remain visible",
      "other MCP servers retain their own search and document semantics",
      "phone, tablet and desktop MCP frames stay within their pane",
    ];
  } finally {
    if (!keepFixture) {
      flushSync(() => root.unmount());
      container.remove();
    }
  }
}
