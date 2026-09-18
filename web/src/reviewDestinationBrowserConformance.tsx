/** Actual Review hook, navigation, recovery and CodeMirror; synthetic HTTP only. */
import { StrictMode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { createTheme, ThemeProvider } from "@mui/material";
import { SurfaceProvider } from "./surface/SurfaceProfile.tsx";
import { fixture, OTHER, readWire, wire } from "./codeBuffers/fixture.ts";
import { navigationWire } from "./codeBuffers/navigationFixture.ts";
import { golden, targetText } from "./codeBuffers/destinationFixture.ts";
import { CodeBufferNavigationPanel } from "./CodeBufferNavigationPanel.tsx";
import { ReviewNavigation } from "./mobile/review/ReviewNavigation.tsx";
import { ReviewCodeStatus } from "./mobile/review/ReviewCodeStatus.tsx";
import { useOwnedReviewBuffer } from "./mobile/review/useOwnedReviewBuffer.ts";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function until(predicate: () => boolean, label: string) {
  for (let n = 0; n < 300; n++) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`destination consumer timed out: ${label}`);
}
function button(label: string, scope: ParentNode = document) {
  const found = [...scope.querySelectorAll("button")].find((node) =>
    node.textContent === label
  );
  check(found, `missing button: ${label}`);
  return found;
}
function click(label: string, scope: ParentNode = document) {
  const found = button(label, scope);
  check(!found.disabled, `disabled button: ${label}`);
  flushSync(() => found.click());
}
function enabled(label: string) {
  return [...document.querySelectorAll("button")].some((node) =>
    node.textContent === label && !node.disabled
  );
}
async function mount(locations = golden.locations) {
  const f = fixture();
  await f.owner.close();
  const source = { ready: () => Promise.resolve(f.registry) };
  const container = document.createElement("div");
  container.style.width = "360px";
  document.body.append(container);
  const root = createRoot(container);
  function Consumer({ text }: { text: string }) {
    const owned = useOwnedReviewBuffer("fixture", "a.rs", true, text, source);
    return (
      <>
        <ReviewCodeStatus intelligence={owned} />
        <output data-source-language>{owned.language?.path ?? "none"}</output>
        {owned.identity
          ? (
            <ReviewNavigation
              intelligence={owned}
              point={golden.position}
              source={f.registry.cleanup}
            />
          )
          : null}
      </>
    );
  }
  const render = (show = true, text = "abc") =>
    flushSync(() =>
      root.render(
        <StrictMode>
          <ThemeProvider theme={createTheme()}>
            <SurfaceProvider>
              {show && <Consumer text={text} />}
              <CodeBufferNavigationPanel source={f.registry.navigations} />
            </SurfaceProvider>
          </ThemeProvider>
        </StrictMode>,
      )
    );
  const calls = async (n: number) => {
    await until(() => f.calls.length >= n, `request ${n}`);
    check(f.calls.length === n, `extra requests ${f.calls.length}/${n}`);
  };
  render();
  await calls(1);
  f.reply(0, wire("prepared"));
  await calls(2);
  f.reply(1, wire("open"));
  await calls(3);
  f.reply(2, {
    ...readWire("language"),
    result: {
      kind: "content",
      content: golden.content,
      result: { kind: "observed", observation: readWire("language").result },
    },
  });
  await until(() => enabled("Prepare Definition"), "source capture");
  const prepare = async () => {
    click("Prepare Definition");
    // Synthetic double-click must still have exactly one original intent.
    flushSync(() => button("Prepare Definition").click());
    await calls(4);
    f.reply(3, navigationWire());
    await until(
      () =>
        !!container.querySelector("[data-review-navigation]")?.textContent
          ?.includes("Navigation · prepared"),
      "prepared navigation",
    );
    await until(() => enabled("Acquire targets"), "Execute action");
  };
  const acquire = async () => {
    await prepare();
    click("Acquire targets");
    await calls(5);
    f.reply(4, { ...navigationWire("retained"), locations });
    await until(
      () => enabled("Read src/target.rs:1"),
      "target choice",
    );
  };
  const read = async (mismatch = false) => {
    await acquire();
    click("Read src/target.rs:1");
    await calls(6);
    f.reply(5, golden);
    await calls(7);
    f.reply(6, wire("open", OTHER));
    await calls(8);
    const value = targetText();
    f.reply(
      7,
      mismatch
        ? {
          ...value,
          result: { ...value.result, result: { kind: "mismatch" } },
        }
        : value,
    );
    await until(() => !button("Check target").disabled, "target read settled");
  };
  return {
    ...f,
    container,
    waitCalls: calls,
    render,
    prepare,
    acquire,
    read,
    unmount() {
      f.context.abort();
      flushSync(() => root.unmount());
      container.remove();
    },
  };
}

export async function runReviewDestinationBrowserConformance(): Promise<
  string[]
> {
  const tests: string[] = [];
  const f = await mount();
  try {
    check(
      Number(f.calls.length) === 3 && f.registry.retained().length === 1,
      "StrictMode or mount acquired navigation",
    );
    await f.read();
    check(
      f.calls.filter((call) =>
        call.url.endsWith("/navigations") && call.init.method === "POST"
      ).length === 1,
      "Prepare replay",
    );
    check(
      JSON.parse(String(f.calls[3]!.init.body)).position.column === 1,
      "source coordinate changed",
    );
    tests.push(
      "actual Review hook binds complete source capture; StrictMode/mount do not navigate and double Prepare admits once",
    );
    await until(
      () => f.container.querySelector(".cm-content")?.textContent === "abc",
      "actual CodeMirror native text",
    );
    await until(
      () =>
        f.container.querySelector(".cowboy-navigation-target")?.textContent ===
          "abc",
      "validated UTF-16 range",
    );
    check(
      f.calls[7]!.url === `/api/code/buffers/${OTHER}/read`,
      "target used path fallback",
    );
    tests.push(
      "explicit Execute and original-index handoff display verified complete native text and exact range in real CodeMirror",
    );
    click("Release navigation", f.container);
    await f.waitCalls(9);
    f.reply(8, { ...golden, state: "released" });
    await until(
      () => f.container.textContent?.includes("Navigation · released") === true,
      "group release",
    );
    check(
      f.container.querySelector(".cm-content")?.textContent === "abc",
      "group release closed independent view",
    );
    check(f.calls.length === 9, "release started child cleanup");
    tests.push(
      "group release leaves the opened destination snapshot and owner independent",
    );
    click("Close target", f.container);
    await f.waitCalls(10);
    check(
      !f.container.querySelector(".cm-content"),
      "closed target still displayed",
    );
    check(
      f.calls[9]!.init.method === "DELETE" && f.calls[9]!.url.endsWith(OTHER),
      "wrong child cleanup",
    );
    f.reply(9, wire("released", OTHER));
    await until(() => f.registry.retained().length === 1, "child release");
    tests.push(
      "Close target removes text immediately and releases only its original ordinary owner",
    );
    check(
      f.container.querySelector("[data-review-code-status]")?.getAttribute(
        "data-review-code-status",
      ) === "navigation",
      "source incorrectly remained ready after navigation",
    );
    check(
      f.container.querySelector("[data-source-language]")?.textContent ===
        "none",
      "old source annotations retained",
    );
    click("Check", f.container);
    await f.waitCalls(11);
    check(f.calls[10]!.init.method === "GET", "source Check reopened source");
    f.reply(10, wire("open"));
    await f.waitCalls(12);
    f.reply(11, {
      ...readWire("language"),
      result: {
        kind: "content",
        content: golden.content,
        result: { kind: "observed", observation: readWire("language").result },
      },
    });
    await until(
      () =>
        f.container.querySelector("[data-review-code-status]")?.getAttribute(
          "data-review-code-status",
        ) === "ready",
      "explicit source reconciliation",
    );
    tests.push(
      "navigation hides old source annotations until explicit Check reobserves the same source and complete content without Open replay",
    );
  } finally {
    f.unmount();
  }

  const late = await mount();
  try {
    await late.acquire();
    click("Read src/target.rs:1");
    await late.waitCalls(6);
    click("Close target");
    late.reply(5, golden);
    await until(
      () =>
        late.registry.retained().some((owner) =>
          owner.view().resourceId === OTHER
        ),
      "late child identity",
    );
    await new Promise<void>((resolve) => setTimeout(resolve, 30));
    check(
      late.calls.length === 6 && !late.container.querySelector(".cm-content"),
      "late handoff opened/disclosed target",
    );
    check(
      late.registry.cleanup.get().rows.length === 1,
      "late child cleanup lost",
    );
    tests.push(
      "closing while handoff is pending retains late child evidence but never Opens or displays it",
    );
  } finally {
    late.unmount();
  }

  const mismatch = await mount();
  try {
    await mismatch.read(true);
    check(
      mismatch.container.textContent?.includes("Target text changed"),
      "mismatch looked successful",
    );
    check(
      !mismatch.container.querySelector(".cm-content") &&
        mismatch.calls.length === 8,
      "mismatch rendered or fell back",
    );
    tests.push(
      "mismatched target text shows no editor, stale location, retry or legacy path request",
    );
  } finally {
    mismatch.unmount();
  }

  const ended = await mount();
  try {
    await ended.read();
    await until(
      () => !!ended.container.querySelector(".cm-content"),
      "reader before context end",
    );
    ended.context.abort();
    await until(
      () => !ended.container.querySelector(".cm-content"),
      "context redaction",
    );
    check(
      !ended.container.textContent?.includes("src/target.rs"),
      "target label leaked after access ended",
    );
    check(ended.calls.length === 8, "context end issued remote work");
    tests.push(
      "core access loss redacts destination text and labels without new transport or account adoption",
    );
  } finally {
    ended.unmount();
  }

  const recovery = await mount();
  try {
    await recovery.prepare();
    // Changing equal-length displayed source ends the original positional view.
    recovery.render(true, "xyz");
    await until(
      () => recovery.registry.retained()[0]!.view().closing,
      "source capture replacement",
    );
    check(
      recovery.registry.retained()[0]!.navigation()?.view().canExecute ===
        false,
      "old source retained Execute",
    );
    recovery.render(false);
    const disclosure = recovery.container.querySelector(
      ".MuiAccordionSummary-root",
    );
    check(disclosure instanceof HTMLElement, "missing recovery projection");
    flushSync(() => disclosure.click());
    check(recovery.calls.length === 4, "mount/expand recovery performed work");
    await until(
      () => enabled("Release navigation…"),
      "original source work drained",
    );
    click("Release navigation…", recovery.container);
    await until(
      () => !!document.querySelector('[role="dialog"]'),
      "release confirmation",
    );
    click("Cancel", document.querySelector('[role="dialog"]')!);
    check(recovery.calls.length === 4, "cancel released navigation");
    await until(
      () => !document.querySelector('[role="dialog"]'),
      "cancel dismissed",
    );
    click("Release navigation…", recovery.container);
    await until(
      () => !!document.querySelector('[role="dialog"]'),
      "new confirmation",
    );
    click("Release navigation", document.querySelector('[role="dialog"]')!);
    await recovery.waitCalls(5);
    recovery.calls[4]!.result.reject(new Error("lost release"));
    await until(
      () =>
        recovery.container.textContent?.includes("Release unconfirmed") ===
          true,
      "unknown release",
    );
    check(
      button("Release navigation…").disabled,
      "lost release rearmed DELETE",
    );
    click("Check navigation status");
    await recovery.waitCalls(6);
    recovery.reply(5, { ...navigationWire(), state: "released" });
    await until(
      () => !recovery.container.querySelector('[data-code-navigation="panel"]'),
      "terminal group retired",
    );
    check(
      recovery.calls.filter((call) => call.init.method === "DELETE").length ===
        1,
      "release replayed",
    );
    tests.push(
      "source replacement revokes Execute; passive Settings recovery confirms/cancels and queries lost Release without replay",
    );
  } finally {
    recovery.unmount();
  }
  const paged = await mount(Array.from({ length: 6 }, (_, index) => ({
    ...golden.locations[0]!,
    path: index === 0 ? "src/target.rs" : `src/target${index}.rs`,
  })));
  try {
    await paged.acquire();
    check(
      paged.container.querySelectorAll("[data-review-target-choice]").length ===
        5,
      "unbounded target list",
    );
    click("Next targets");
    check(
      paged.container.querySelectorAll("[data-review-target-choice]").length ===
        1,
      "target page boundary",
    );
    check(
      paged.container.textContent?.includes("src/target5.rs"),
      "wrong original target page",
    );
    click("Previous targets");
    check(paged.calls.length === 5, "paging acquired a destination");
    tests.push(
      "target choices page five original tokens at a time without transport or capacity allocation",
    );
  } finally {
    paged.unmount();
  }
  return tests;
}
