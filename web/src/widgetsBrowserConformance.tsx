/** Inline widgets rendered by the real Markdown renderer in the product theme
 *  at phone, tablet and Desktop widths (docs/widgets.md §13). */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CssBaseline } from "@mui/material";
import { approval, comparison, statusTable } from "../../contracts/widget-v1.examples.ts";
import { BrowserProductTheme } from "./browserProductTheme";
import { Markdown } from "./Markdown";

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
  const render = (width: number, text: string, streaming = false): void =>
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <CssBaseline />
          <Box data-width={width} sx={{ width, p: 1 }}>
            <Markdown text={text} streaming={streaming} />
          </Box>
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

  root.unmount();
  container.remove();
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
