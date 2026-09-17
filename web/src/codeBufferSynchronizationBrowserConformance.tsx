/** Real React/MUI/StrictMode and core continuations; only synthetic HTTP. */
import { StrictMode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { createTheme, ThemeProvider } from "@mui/material";
import { SurfaceProvider } from "./surface/SurfaceProfile.tsx";
import { CodeBufferSynchronizationPanel } from "./CodeBufferSynchronizationPanel.tsx";
import type { CodeBufferSynchronizations } from "./codeBuffers/synchronizationProjection.ts";
import { fixture, opened, wire } from "./codeBuffers/fixture.ts";
import {
  appliedState,
  content,
  preparedSync,
  SYNC_ID,
  syncWire,
} from "./codeBuffers/synchronizationFixture.ts";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function until(predicate: () => boolean, message = "fixture timed out") {
  for (let n = 0; n < 200; n++) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  check(predicate(), message);
}
function button(label: string, scope: ParentNode = document) {
  const found = [...scope.querySelectorAll("button")].find((node) =>
    node.textContent === label
  );
  check(found instanceof HTMLButtonElement, `missing ${label}`);
  return found;
}
function hasButton(label: string, scope: ParentNode) {
  return [...scope.querySelectorAll("button")].some((node) =>
    node.textContent === label
  );
}
function mount(source: CodeBufferSynchronizations) {
  const container = document.createElement("div");
  container.style.width = "360px";
  document.body.append(container);
  const root = createRoot(container);
  flushSync(() =>
    root.render(
      <StrictMode>
        <ThemeProvider theme={createTheme()}>
          <SurfaceProvider>
            <CodeBufferSynchronizationPanel source={source} />
          </SurfaceProvider>
        </ThemeProvider>
      </StrictMode>,
    )
  );
  return {
    container,
    text: () => container.textContent ?? "",
    rows: () => container.querySelectorAll('[data-code-sync="row"]'),
    expand: () => {
      const disclosure = container.querySelector(".MuiAccordionSummary-root");
      check(disclosure instanceof HTMLElement, "missing disclosure");
      flushSync(() => disclosure.click());
    },
    unmount: () => {
      flushSync(() => root.unmount());
      container.remove();
    },
  };
}
async function dialog() {
  await until(() => !!document.querySelector('[role="dialog"]'));
}
async function dismissed() {
  await until(() => !document.querySelector('[role="dialog"]'));
}

export async function runCodeBufferSynchronizationBrowserConformance(): Promise<
  string[]
> {
  const tests: string[] = [];
  const passive = await preparedSync();
  let view = mount(passive.source);
  try {
    view.expand();
    check(view.text().includes("Prepared only"), "preparation looks applied");
    check(!view.text().includes(SYNC_ID), "operation identifier leaked to UI");
    check(passive.calls.length === 3, "render or disclosure started work");
    view.unmount();
    view = mount(passive.source);
    check(passive.calls.length === 3, "StrictMode remount started work");
    tests.push(
      "mount, disclosure and StrictMode remount observe local evidence without preparing or applying",
    );
  } finally {
    view.unmount();
    passive.context.abort();
  }

  const apply = await preparedSync();
  view = mount(apply.source);
  try {
    view.expand();
    button("Review refresh…", view.container).click();
    await dialog();
    check(
      document.body.textContent?.includes("cannot automatically undo"),
      "missing effect boundary",
    );
    button("Cancel").click();
    await dismissed();
    check(apply.calls.length === 3, "cancel applied");
    button("Review refresh…", view.container).click();
    await dialog();
    const confirm = button("Refresh native buffer");
    confirm.click();
    confirm.click();
    await until(() => apply.calls.length === 4);
    check(
      apply.calls[3]!.url.endsWith(SYNC_ID) &&
        apply.calls[3]!.init.method === "PUT",
      "confirmation replaced original operation",
    );
    view.unmount();
    view = mount(apply.source);
    view.expand();
    check(!apply.calls[3]!.init.signal!.aborted, "unmount cancelled Apply");
    check(
      !hasButton("Review refresh…", view.container),
      "in-flight Apply offered replay",
    );
    apply.reply(3, syncWire(appliedState));
    await until(() => view.text().includes("Refresh confirmed"));
    check(
      Number(apply.calls.length) === 4,
      "late completion or remount repeated Apply",
    );
    tests.push(
      "cancel is inert; duplicate confirmation and remount preserve exactly one original Apply and its late outcome",
    );
  } finally {
    view.unmount();
    apply.context.abort();
  }

  const unknown = await preparedSync();
  const applying = unknown.operation.confirm(
    unknown.operation.preview("apply"),
  );
  unknown.calls[3]!.result.reject(new Error("private fixture detail"));
  await applying.catch(() => undefined);
  view = mount(unknown.source);
  try {
    view.expand();
    check(view.text().includes("Outcome unknown"), "lost Apply hidden");
    check(
      !view.text().includes("private fixture"),
      "private exception displayed",
    );
    check(
      !hasButton("Review refresh…", view.container) &&
        !hasButton("Retire operation…", view.container),
      "uncertainty offered effect or disposal",
    );
    button("Check synchronization", view.container).click();
    await until(() => unknown.calls.length === 5);
    view.unmount();
    view = mount(unknown.source);
    view.expand();
    check(!unknown.calls[4]!.init.signal!.aborted, "unmount aborted Query");
    unknown.reply(4, {}, 404);
    await until(() =>
      !button("Check synchronization", view.container).disabled
    );
    check(
      view.rows().length === 1 && view.text().includes("Outcome unknown"),
      "404 fabricated completion",
    );
    button("Check synchronization", view.container).click();
    await until(() => unknown.calls.length === 6);
    unknown.reply(5, syncWire(appliedState));
    await until(() => hasButton("Retire operation…", view.container));
    button("Retire operation…", view.container).click();
    await dialog();
    button("Retire synchronization").click();
    await until(() => unknown.calls.length === 7);
    unknown.calls[6]!.result.reject(new Error("lost retirement"));
    await until(() => view.text().includes("Retirement not confirmed"));
    check(
      !hasButton("Retire operation…", view.container),
      "ambiguous retirement offered retry",
    );
    button("Check synchronization", view.container).click();
    await until(() => unknown.calls.length === 8);
    unknown.reply(7, syncWire({ kind: "retired" }));
    await until(() => view.container.children.length === 0);
    check(
      unknown.calls.filter(({ init }) => init.method === "DELETE").length === 1,
      "retirement repeated",
    );
    tests.push(
      "lost Apply, 404 and lost retirement remain visible and query-only; only exact terminal evidence removes ownership",
    );
  } finally {
    view.unmount();
    unknown.context.abort();
  }

  const ended = await preparedSync();
  view = mount(ended.source);
  try {
    view.expand();
    button("Review refresh…", view.container).click();
    await dialog();
    const confirm = button("Refresh native buffer");
    ended.context.abort();
    confirm.click(); // same stack, before React redacts
    await dismissed();
    await until(() => view.text().includes("Original access ended"));
    check(ended.calls.length === 3, "ended authority accepted a stale click");
    check(
      !document.body.textContent?.includes("src/main.rs") &&
        !view.text().includes(ended.row.content!.sha256),
      "ended identity details remain visible",
    );
    check(
      view.rows().length === 1 &&
        view.container.querySelectorAll("button").length === 1,
      "ended owner lost or still actionable",
    );
    tests.push(
      "same-stack identity end fences confirmation and redacts private path/content without dropping unresolved ownership",
    );
  } finally {
    view.unmount();
    ended.context.abort();
  }

  const stale = await preparedSync();
  view = mount(stale.source);
  try {
    view.expand();
    button("Review refresh…", view.container).click();
    await dialog();
    const query = stale.operation.observe();
    stale.reply(3, syncWire());
    await query;
    await until(() => button("Refresh native buffer").disabled);
    button("Refresh native buffer").click();
    check(stale.calls.length === 4, "observation renewed old preview");
    check(
      document.body.textContent?.includes("state changed"),
      "stale confirmation unexplained",
    );
    button("Cancel").click();
    await dismissed();
    button("Review refresh…", view.container).click();
    await dialog();
    const confirm = button("Refresh native buffer");
    const closed = stale.owner.close();
    confirm.click();
    await closed;
    check(
      stale.calls.length === 4,
      "closing consumer allowed an old Apply confirmation",
    );
    tests.push(
      "intervening queries and consumer close invalidate old confirmation even if the prepared identity is unchanged",
    );
  } finally {
    view.unmount();
    stale.context.abort();
  }

  const late = await opened();
  const preparing = late.owner.prepareSynchronization(await content());
  const closed = late.owner.close();
  late.reply(2, syncWire());
  await preparing;
  await closed;
  view = mount(late.registry.synchronizations);
  try {
    view.expand();
    check(
      !hasButton("Review refresh…", view.container),
      "closed view's late preparation can Apply",
    );
    button("Retire operation…", view.container).click();
    await dialog();
    const confirm = button("Retire synchronization");
    confirm.click();
    confirm.click();
    await until(() => late.calls.length === 4);
    view.unmount();
    view = mount(late.registry.synchronizations);
    late.reply(3, syncWire({ kind: "retired" }));
    await until(() => view.container.children.length === 0);
    check(
      late.calls.length === 4 && late.registry.retained().length === 1,
      "retirement silently released/reopened a buffer",
    );
    tests.push(
      "a late preparation after close permits retirement only; unmount preserves it and retirement never automatically releases the buffer",
    );
  } finally {
    view.unmount();
    late.context.abort();
  }

  const paged = fixture();
  await paged.owner.close();
  const captured = await content();
  for (let n = 0; n < 12; n++) {
    const owner = paged.registry.reserve({
      sessionId: "paging",
      path: `${"<unsafe-label>-long-name-".repeat(40)}${n}.rs`,
    });
    const id = `${"a".repeat(32)}-${(n + 1).toString(16).padStart(16, "0")}`;
    const prepare = owner.prepare();
    paged.reply(paged.calls.length - 1, wire("prepared", id));
    await prepare;
    const open = owner.open();
    paged.reply(paged.calls.length - 1, wire("open", id));
    await open;
    const sync = owner.prepareSynchronization(captured);
    paged.reply(paged.calls.length - 1, {
      ...syncWire(),
      resourceId: id,
      operationId: `sync-${id}`,
    });
    await sync;
  }
  view = mount(paged.registry.synchronizations);
  try {
    view.expand();
    check(view.rows().length === 5, "unbounded synchronization list");
    check(
      view.container.scrollWidth <= view.container.clientWidth + 1,
      "long text overflows 360px",
    );
    check(
      view.container.querySelector("unsafe-label") === null,
      "target rendered as markup",
    );
    flushSync(() => button("Next", view.container).click());
    check(
      view.rows().length === 5 && view.text().includes("6–10 of 12"),
      "wrong second page",
    );
    flushSync(() => button("Next", view.container).click());
    check(
      view.rows().length === 2 && view.text().includes("11–12 of 12"),
      "wrong last page",
    );
    check(paged.calls.length === 36, "paging emitted HTTP");
    tests.push(
      "twelve long-path operations remain five-per-page at 360px; labels are escaped and pagination sends nothing",
    );
  } finally {
    view.unmount();
    paged.context.abort();
  }

  const replacement = await preparedSync();
  view = mount(replacement.source);
  try {
    view.expand();
    button("Review refresh…", view.container).click();
    await dialog();
    const retirement = replacement.operation.confirm(
      replacement.operation.preview("retire"),
    );
    replacement.reply(3, syncWire({ kind: "retired" }));
    await retirement;
    const observe = replacement.owner.observe();
    replacement.reply(4, wire("open"));
    await observe;
    const prepare = replacement.owner.prepareSynchronization(
      replacement.captured,
    );
    replacement.reply(5, {
      ...syncWire(),
      operationId: `sync-${"b".repeat(32)}-0000000000000002`,
    });
    await prepare;
    // The old dialog is removed, not retargeted when the only row disappears.
    await until(() => view.text().includes("Synchronization 2"));
    check(replacement.calls.length === 6, "replacement renewed a stale Apply");
    const buttons = [...document.querySelectorAll("button")].filter((node) =>
      node.textContent === "Refresh native buffer"
    );
    check(
      buttons.every((node) => node.disabled),
      "old dialog can apply replacement",
    );
    tests.push(
      "retired handles and open confirmation cannot adopt a new operation on the same buffer",
    );
  } finally {
    view.unmount();
    replacement.context.abort();
  }
  return tests;
}
