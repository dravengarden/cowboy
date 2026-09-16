/** Actual core owner + React/MUI/StrictMode, synthetic HTTP and fresh profile. */
import { StrictMode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { createTheme, ThemeProvider } from "@mui/material";
import { CodeBufferCleanupPanel } from "./CodeBufferCleanupPanel.tsx";
import { SurfaceProvider } from "./surface/SurfaceProfile.tsx";
import { fixture, ID, opened, OTHER, wire } from "./codeBuffers/fixture.ts";
import type { CodeBufferCleanup } from "./codeBuffers/cleanup.ts";

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
function mount(source: CodeBufferCleanup) {
  const container = document.createElement("div");
  container.style.width = "360px";
  document.body.append(container);
  const root = createRoot(container);
  const theme = createTheme({
    components: { MuiButton: { styleOverrides: { root: { minHeight: 44 } } } },
  });
  flushSync(() =>
    root.render(
      <StrictMode>
        <ThemeProvider theme={theme}>
          <SurfaceProvider>
            <CodeBufferCleanupPanel source={source} />
          </SurfaceProvider>
        </ThemeProvider>
      </StrictMode>,
    )
  );
  return {
    container,
    text: () => container.textContent ?? "",
    rows: () => container.querySelectorAll('[data-code-cleanup="row"]'),
    expand: () => {
      const button = container.querySelector(".MuiAccordionSummary-root");
      check(button instanceof HTMLElement, "missing disclosure");
      flushSync(() => button.click());
    },
    unmount: () => {
      flushSync(() => root.unmount());
      container.remove();
    },
  };
}
function button(label: string, root: ParentNode = document) {
  const candidate = [...root.querySelectorAll("button")].find((item) =>
    item.textContent === label
  );
  check(candidate instanceof HTMLButtonElement, `missing button ${label}`);
  return candidate;
}
async function awaitingCleanup() {
  const f = await opened();
  const close = f.owner.close();
  await until(() => f.calls.length === 3);
  f.reply(2, wire("open", ID, true), 202);
  await close;
  return f;
}

export async function runCodeBufferCleanupBrowserConformance(): Promise<
  string[]
> {
  const tests: string[] = [];
  const local = fixture();
  let view = mount(local.registry.cleanup);
  try {
    view.expand();
    check(view.text().includes("1 active"), "active owner not counted");
    check(view.rows().length === 0, "active owner exposed cleanup");
    check(local.calls.length === 0, "mount or expand sent a request");
    view.unmount();
    view = mount(local.registry.cleanup);
    check(local.calls.length === 0, "StrictMode remount sent a request");
    await local.owner.close();
    await until(() => view.container.children.length === 0);
    tests.push(
      "mount, disclosure and StrictMode remount are local; active views cannot be closed from Settings",
    );
  } finally {
    view.unmount();
    local.context.abort();
  }

  const pending = await awaitingCleanup();
  view = mount(pending.registry.cleanup);
  try {
    view.expand();
    check(view.text().includes("cleanup was not queued"), "202 hidden");
    check(pending.calls.length === 3, "render retried cleanup");
    button("Continue cleanup…", view.container).click();
    await until(() => !!document.querySelector('[role="dialog"]'));
    check(pending.calls.length === 3, "preview released a resource");
    button("Cancel").click();
    await until(() => !document.querySelector('[role="dialog"]'));
    check(pending.calls.length === 3, "cancel sent a request");
    button("Continue cleanup…", view.container).click();
    await until(() => !!document.querySelector('[role="dialog"]'));
    const confirm = button("Continue cleanup");
    confirm.click();
    confirm.click(); // same stack, before React can remove the confirmation
    await until(() => pending.calls.length === 4);
    check(
      pending.calls[3]!.init.method === "GET",
      "cleanup skipped observation",
    );
    pending.reply(3, wire("open"));
    await until(() => pending.calls.length === 5);
    check(pending.calls[4]!.init.method === "DELETE", "missing exact release");
    check(
      pending.calls[4]!.url.endsWith(ID),
      "release replaced original owner",
    );
    // Admitted work remains owned when Settings goes away.
    view.unmount();
    view = mount(pending.registry.cleanup);
    check(
      Number(pending.calls.length) === 5,
      "remount retried pending release",
    );
    pending.reply(4, wire("released"));
    await until(() => pending.registry.retained().length === 0);
    await until(() => view.container.children.length === 0);
    tests.push(
      "pending cleanup requires confirmation; cancel and duplicate confirmation cannot queue or repeat a release",
    );
    tests.push(
      "Settings unmount during release preserves the original continuation and remount observes its terminal result",
    );
  } finally {
    view.unmount();
    pending.context.abort();
  }

  const lost = await opened();
  const close = lost.owner.close();
  await until(() => lost.calls.length === 3);
  lost.calls[2]!.result.reject(new Error("private-fixture-transport-detail"));
  await close;
  view = mount(lost.registry.cleanup);
  try {
    view.expand();
    check(
      view.text().includes("status checks only"),
      "uncertain release hidden",
    );
    check(!view.text().includes("private-fixture"), "exception detail leaked");
    check(
      ![...view.container.querySelectorAll("button")].some((item) =>
        item.textContent === "Continue cleanup…"
      ),
      "ambiguous release offered retry",
    );
    button("Check status", view.container).click();
    await until(() => lost.calls.length === 4);
    view.unmount();
    view = mount(lost.registry.cleanup);
    view.expand();
    check(
      !lost.calls[3]!.init.signal!.aborted,
      "unmount cancelled core observation",
    );
    lost.reply(3, wire("open"));
    await until(() => view.text().includes("status checks only"));
    check(lost.calls.length === 4, "late open evidence retried release");
    button("Check status", view.container).click();
    await until(() => lost.calls.length === 5);
    lost.reply(4, {}, 404);
    await until(() => !button("Check status", view.container).disabled);
    check(view.rows().length === 1, "404 fabricated successful cleanup");
    check(!view.text().includes("private-fixture"), "private error displayed");
    tests.push(
      "lost release stays inspect-only across remount, late open evidence and 404; no native retry or false success",
    );
  } finally {
    view.unmount();
    lost.context.abort();
  }

  const revoked = await awaitingCleanup();
  view = mount(revoked.registry.cleanup);
  try {
    view.expand();
    button("Continue cleanup…", view.container).click();
    await until(() => !!document.querySelector('[role="dialog"]'));
    const confirm = button("Continue cleanup");
    revoked.context.abort();
    confirm.click(); // stale rendered confirmation in the same JS stack
    await until(() => !document.querySelector('[role="dialog"]'));
    await until(() => view.text().includes("Original access ended"));
    check(
      !document.body.textContent?.includes("src/main.rs"),
      "ended principal path remains visible",
    );
    check(
      revoked.calls.length === 3,
      "old confirmation borrowed replacement authority",
    );
    check(
      view.rows().length === 1,
      "revocation discarded unresolved ownership",
    );
    tests.push(
      "identity end fences stale confirmation synchronously, hides private paths and keeps unresolved ownership visible",
    );
  } finally {
    view.unmount();
    revoked.context.abort();
  }

  const paged = fixture();
  await paged.owner.close();
  const retained = [];
  for (let n = 0; n < 12; n++) {
    const owner = paged.registry.reserve({
      sessionId: "paging",
      path: `${"very-long-file-name-".repeat(30)}-${n}.rs`,
    });
    const id = `${"a".repeat(32)}-${(n + 1).toString(16).padStart(16, "0")}`;
    const prepare = owner.prepare();
    paged.reply(paged.calls.length - 1, wire("prepared", id));
    await prepare;
    const closing = owner.close();
    await until(() => paged.calls.length === (n + 1) * 2);
    paged.reply(paged.calls.length - 1, wire("prepared", id, true), 202);
    await closing;
    retained.push({ owner, id });
  }
  view = mount(paged.registry.cleanup);
  try {
    view.expand();
    check(view.rows().length === 5, "unbounded cleanup list");
    check(
      view.container.scrollWidth <= view.container.clientWidth + 1,
      "long paths overflow mobile width",
    );
    flushSync(() => button("Next", view.container).click());
    check(
      view.rows().length === 5 && view.text().includes("6–10 of 12"),
      "second page wrong",
    );
    flushSync(() => button("Next", view.container).click());
    check(
      view.rows().length === 2 && view.text().includes("11–12 of 12"),
      "last page wrong",
    );
    check(paged.calls.length === 24, "paging queried native resources");
    for (const entry of retained.slice(5)) {
      const observation = entry.owner.observe();
      paged.reply(paged.calls.length - 1, wire("released", entry.id));
      await observation;
    }
    await until(() =>
      view.rows().length === 5 && !view.text().includes("Next")
    );
    check(Number(paged.calls.length) === 31, "page clamping sent requests");
    tests.push(
      "360px long-path list renders at most five rows, pages locally and clamps safely after terminal removals",
    );
  } finally {
    view.unmount();
    paged.context.abort();
  }

  const changed = await awaitingCleanup();
  view = mount(changed.registry.cleanup);
  try {
    view.expand();
    button("Continue cleanup…", view.container).click();
    await until(() => !!document.querySelector('[role="dialog"]'));
    const status = changed.owner.observe();
    changed.reply(3, wire("released"));
    await status;
    const replacement = changed.registry.reserve({
      sessionId: "replacement",
      path: "other.rs",
    });
    const prepare = replacement.prepare();
    changed.reply(4, wire("prepared", OTHER));
    await prepare;
    await until(() => button("Continue cleanup").disabled);
    check(
      changed.calls.length === 5,
      "stale confirmation released replacement",
    );
    tests.push(
      "a removed original owner invalidates an open confirmation without adopting a replacement buffer",
    );
  } finally {
    view.unmount();
    changed.context.abort();
  }
  return tests;
}
