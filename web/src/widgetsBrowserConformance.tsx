/** Inline widgets rendered by the real Markdown renderer in the product theme
 *  at phone, tablet and Desktop widths (docs/widgets.md §13). */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CssBaseline } from "@mui/material";
import { approval, comparison, statusTable } from "../../contracts/widget-v1.examples.ts";
import { BrowserProductTheme } from "./browserProductTheme";
import { Markdown } from "./Markdown";
import { SurfaceContext, type SurfaceProfile } from "./surface/SurfaceProfile";
import { DesktopWorkspaceProvider } from "./desktop/DesktopWorkspaceController";
import { DesktopCommandProvider } from "./desktop/commands/DesktopCommandProvider";

function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}

const fence = (value: unknown): string =>
  `Report:\n\n\`\`\`cowboy-widget\n${JSON.stringify(value, null, 2)}\n\`\`\`\n`;

async function waitFor<T>(probe: () => T | null | undefined, label: string): Promise<T> {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    const found = probe();
    if (found) return found;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error(`timed out: ${label}`);
}

export async function runWidgetsBrowserConformance(): Promise<string[]> {
  const passed: string[] = [];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = (
    width: number,
    text: string,
    streaming = false,
    surface: SurfaceProfile | null = null,
  ): void =>
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <CssBaseline />
          <SurfaceContext.Provider value={surface}>
            <Box data-width={width} sx={{ width, p: 1 }}>
              <Markdown text={text} streaming={streaming} />
            </Box>
          </SurfaceContext.Provider>
        </BrowserProductTheme>,
      ),
    );

  for (const width of [320, 375, 768, 1100]) {
    render(width, fence(statusTable));
    const widget = await waitFor(
      () => container.querySelector<HTMLElement>("[data-cowboy-widget]"),
      `widget at ${width}`,
    );
    const table = await waitFor(
      () => container.querySelector<HTMLElement>("[data-widget-table]"),
      `table at ${width}`,
    );
    await new Promise((resolve) => setTimeout(resolve, 100));
    const mode = table.getAttribute("data-widget-table");
    check(
      width < 560 ? mode === "cards" : mode === "table",
      `four columns at ${width}px render as ${mode}`,
    );
    check(
      widget.scrollWidth <= widget.clientWidth + 1,
      `no horizontal overflow at ${width}px (${widget.scrollWidth} > ${widget.clientWidth})`,
    );
    passed.push(`status table ${width}px`);
  }

  render(320, fence(comparison));
  const compact = await waitFor(
    () => container.querySelector<HTMLElement>("[data-widget-table]"),
    "three-column table",
  );
  check(
    compact.getAttribute("data-widget-table") === "table",
    "three columns stay a table on a phone",
  );
  passed.push("three columns stay a table");

  render(768, '```cowboy-widget\n{"schema":"cowboy.widget/1","title":"Half', true);
  await new Promise((resolve) => setTimeout(resolve, 100));
  check(!container.querySelector("[data-cowboy-widget]"), "an open fence never renders a widget");
  check(container.textContent?.includes("Half"), "an open fence streams as code");
  passed.push("streaming fence shows source");

  render(
    768,
    '```cowboy-widget\n{"schema":"cowboy.widget/1","title":"x","root":{"type":"table","columns":["a"],"rows":[[1,2]]}}\n```\n',
  );
  await waitFor(() => container.querySelector("[data-widget-invalid]"), "invalid notice");
  check(
    !container.querySelector("[data-cowboy-widget]"),
    "an invalid document never renders a widget",
  );
  check(container.textContent?.includes('"columns"'), "an invalid document keeps its source");
  passed.push("invalid fence falls back to source");

  passed.push(...(await inspectorChecks(render, container)));
  root.unmount();
  passed.push(await desktopKeyChecks(container));

  container.remove();
  return passed;
}

const DESKTOP: SurfaceProfile = {
  kind: "desktop",
  input: "keyboard-pointer",
  touchCapable: false,
  finePointer: true,
  hover: true,
};

function press(key: string): void {
  (document.activeElement ?? document.body).dispatchEvent(
    new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }),
  );
}

/** The real Desktop dispatcher: Tab selects the widget row, `l` opens the
 *  inspector, Esc closes it and returns focus to the row. */
async function desktopKeyChecks(container: HTMLElement): Promise<string> {
  const root = createRoot(container);
  flushSync(() =>
    root.render(
      <SurfaceContext.Provider value={DESKTOP}>
        <BrowserProductTheme>
          <CssBaseline />
          <DesktopWorkspaceProvider>
            <DesktopCommandProvider>
              <Box data-desktop-pane="conversation" sx={{ width: 900, height: 700 }}>
                <Box
                  data-desktop-region="conversation.transcript"
                  data-desktop-navigation="scroll"
                  data-desktop-focus-default
                  tabIndex={-1}
                  sx={{ height: 1, overflow: "auto", p: 1 }}
                >
                  <Markdown text={fence(longWidget)} />
                </Box>
              </Box>
            </DesktopCommandProvider>
          </DesktopWorkspaceProvider>
        </BrowserProductTheme>
      </SurfaceContext.Provider>,
    ),
  );
  try {
    const toggle = await waitFor(
      () => container.querySelector<HTMLElement>('[data-desktop-widget-toggle="widget"]'),
      "widget row toggle",
    );
    container.querySelector<HTMLElement>("[data-desktop-region]")?.focus();
    await new Promise((resolve) => setTimeout(resolve, 60));
    press("Tab");
    check(document.activeElement === toggle, "Tab selects the widget row");
    press("l");
    await waitFor(() => document.querySelector("[data-widget-inspector]"), "l opens the inspector");
    check(toggle.getAttribute("aria-expanded") === "true", "the row reports its open inspector");
    const dialog = document.querySelector<HTMLElement>("[data-widget-inspector]")!;
    const visited: string[] = [];
    for (let step = 0; step < 6 && !document.activeElement?.closest(".MuiTableSortLabel-root"); step += 1) {
      press("j");
      await new Promise((resolve) => setTimeout(resolve, 30));
      visited.push(document.activeElement?.textContent?.slice(0, 20) ?? "");
    }
    const header = document.activeElement?.closest("th");
    check(
      header && dialog.contains(header),
      `j reaches a sort header inside the inspector (${visited.join(" | ")})`,
    );
    press("Enter");
    await new Promise((resolve) => setTimeout(resolve, 30));
    check(header.getAttribute("aria-sort") === "ascending", "Enter sorts by the focused header");
    press("Escape");
    await waitFor(() => !document.querySelector("[data-widget-inspector]"), "Esc closes");
    await waitFor(() => document.activeElement === toggle, "focus returns to the row");
    check(toggle.getAttribute("aria-expanded") === "false", "the row reports a closed inspector");
    return "Desktop keys select, open and leave a widget";
  } finally {
    root.unmount();
  }
}

const longWidget = {
  schema: "cowboy.widget/1",
  title: "Long report",
  root: {
    type: "stack",
    children: [
      {
        type: "table",
        sort: true,
        columns: ["Name", { label: "Size", format: "bytes", align: "end" }],
        rows: Array.from({ length: 120 }, (_, i) => [`item-${String(i)}`, ((i * 37) % 997) + 1]),
      },
      { type: "list", items: Array.from({ length: 60 }, (_, i) => ({ text: `entry ${String(i)}` })) },
    ],
  },
};

function firstSize(scope: ParentNode): string {
  return scope.querySelector("tbody tr td:nth-child(2)")?.textContent ?? "";
}

/** Preview bounds, header sort, and the inspector's paging and focus return. */
async function inspectorChecks(
  render: (width: number, text: string, streaming?: boolean, surface?: SurfaceProfile) => void,
  container: HTMLElement,
): Promise<string[]> {
  render(1100, fence(longWidget));
  const table = await waitFor(
    () => container.querySelector<HTMLElement>("[data-widget-table]"),
    "long table",
  );
  check(table.querySelectorAll("tbody tr").length === 20, "the preview draws 20 rows");
  check(table.textContent?.includes("100 more rows"), "the preview counts hidden rows");
  check(container.textContent?.includes("10 more items"), "the preview bounds a long list");
  const sizeHeader = table.querySelector<HTMLElement>("th:nth-child(2)");
  const sortButton = sizeHeader?.querySelector<HTMLElement>(".MuiTableSortLabel-root");
  check(sizeHeader && sortButton, "a sortable header has a sort control");
  flushSync(() => sortButton.click());
  check(sizeHeader.getAttribute("aria-sort") === "ascending", "first activation sorts ascending");
  check(firstSize(table) === "1 B", `ascending starts at the smallest size (${firstSize(table)})`);
  flushSync(() => sortButton.click());
  check(sizeHeader.getAttribute("aria-sort") === "descending", "second activation sorts descending");
  const largest = Math.max(...longWidget.root.children[0]!.rows!.map((row) => Number(row[1])));
  check(
    firstSize(table) === `${String(largest)} B`,
    `descending starts at the largest size (${firstSize(table)})`,
  );
  const passed = ["preview bounds rows and items", "header sort cycles by value"];

  const open = container.querySelector<HTMLElement>("[data-widget-open]");
  check(open, "the widget header has Open");
  open.focus();
  open.click();
  const inspector = await waitFor(
    () => document.querySelector<HTMLElement>("[data-widget-inspector]"),
    "inspector",
  );
  check(inspector.querySelectorAll("tbody tr").length === 50, "the inspector pages 50 rows");
  check(
    inspector.querySelector("th:nth-child(2)")?.getAttribute("aria-sort") === "descending" &&
      firstSize(inspector) === `${String(largest)} B`,
    "the inspector keeps the row's sort",
  );
  check(inspector.querySelectorAll("li").length === 60, "the inspector draws every list item");
  check(
    inspector.querySelector(".MuiTablePagination-root"),
    "a table longer than a page has pagination",
  );
  passed.push("inspector pages the whole widget");
  document.activeElement?.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
  );
  await waitFor(() => !document.querySelector("[data-widget-inspector]"), "inspector closes");
  await waitFor(() => document.activeElement === open, "focus returns to Open");
  passed.push("Escape closes the inspector and restores focus");

  const phone: SurfaceProfile = {
    kind: "mobile",
    input: "touch",
    touchCapable: true,
    finePointer: false,
    hover: false,
  };
  render(375, fence(longWidget), false, phone);
  const phoneOpen = await waitFor(
    () => container.querySelector<HTMLElement>("[data-widget-open]"),
    "phone Open",
  );
  check(phoneOpen.getBoundingClientRect().width >= 44, "Open keeps a 44 px touch target");
  phoneOpen.click();
  const sheet = await waitFor(
    () => document.querySelector<HTMLElement>("[data-widget-inspector]"),
    "phone inspector",
  );
  check(sheet.closest("[data-detent-sheet]"), "a phone opens the inspector as a sheet");
  render(375, "done");
  await waitFor(() => !document.querySelector("[data-widget-inspector]"), "sheet unmounts");
  passed.push("phone inspector is a sheet");
  return passed;
}

/** Leaves the three contract examples mounted for screenshots. */
export function showWidgetGallery(): void {
  const container = document.createElement("div");
  document.body.append(container);
  flushSync(() =>
    createRoot(container).render(
      <BrowserProductTheme>
        <CssBaseline />
        <Box sx={{ p: 1.5, maxWidth: 860 }}>
          <Markdown text={[statusTable, comparison, approval].map(fence).join("\n")} />
        </Box>
      </BrowserProductTheme>,
    ),
  );
}

/** Leaves the long sortable widget mounted (optionally as a phone) for screenshots. */
export function showLongWidget(kind: "desktop" | "mobile" = "desktop"): void {
  const container = document.createElement("div");
  document.body.append(container);
  const surface: SurfaceProfile | null = kind === "mobile"
    ? { kind, input: "touch", touchCapable: true, finePointer: false, hover: false }
    : null;
  flushSync(() =>
    createRoot(container).render(
      <BrowserProductTheme>
        <CssBaseline />
        <SurfaceContext.Provider value={surface}>
          <Box sx={{ p: 1.5, maxWidth: 860 }}>
            <Markdown text={fence(longWidget)} />
          </Box>
        </SurfaceContext.Provider>
      </BrowserProductTheme>,
    ),
  );
}
