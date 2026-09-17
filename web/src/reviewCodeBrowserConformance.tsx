/** Actual Review hook, Outline and status surface. Synthetic HTTP, no account. */
import { StrictMode, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { createTheme, ThemeProvider } from "@mui/material";
import { SurfaceProvider } from "./surface/SurfaceProfile.tsx";
import { fixture, ID, readWire, wire } from "./codeBuffers/fixture.ts";
import {
  captureContent,
  type CapturedContent,
  capturedIdentity,
} from "./codeBuffers/content.ts";
import { BufferClientError } from "./codeBuffers/protocol.ts";
import {
  appSettingsFromEvent,
  OPEN_APP_SETTINGS_EVENT,
} from "./appSettings.ts";
import { ReviewCodeStatus } from "./mobile/review/ReviewCodeStatus.tsx";
import { ReviewOutline } from "./mobile/review/ReviewOutline.tsx";
import {
  type OwnedReviewIntelligence,
  reviewDisplayText,
  useOwnedReviewBuffer,
} from "./mobile/review/useOwnedReviewBuffer.ts";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function until(predicate: () => boolean, label: string) {
  for (let n = 0; n < 300; n++) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`Review fixture timed out: ${label}`);
}
function contentWire(
  content: CapturedContent,
  kind: "language" | "symbols" = "language",
  mismatch = false,
) {
  return {
    ...readWire(kind),
    result: {
      kind: "content",
      content: capturedIdentity(content),
      result: mismatch
        ? { kind: "mismatch" }
        : { kind: "observed", observation: readWire(kind).result },
    },
  };
}

export async function runReviewCodeBrowserConformance(): Promise<string[]> {
  const tests: string[] = [];
  const f = fixture();
  const count = () => f.calls.length;
  await f.owner.close();
  const source = { ready: () => Promise.resolve(f.registry) };
  const container = document.createElement("div");
  container.style.width = "360px";
  document.body.append(container);
  const root = createRoot(container);
  let latest: OwnedReviewIntelligence | undefined;
  function Consumer({ text, complete }: { text: string; complete: boolean }) {
    const [outline, setOutline] = useState(false);
    const display = reviewDisplayText(text);
    const intelligence = useOwnedReviewBuffer(
      "fixture",
      "a.txt",
      true,
      complete ? display : undefined,
      source,
    );
    latest = intelligence;
    return (
      <>
        <pre data-review-text>{display}</pre>
        <output>{intelligence.language?.path ?? "none"}</output>
        <ReviewCodeStatus intelligence={intelligence} />
        <button onClick={() => setOutline(true)}>Outline</button>
        <ReviewOutline
          open={outline}
          onClose={() => setOutline(false)}
          sessionId="fixture"
          path="a.txt"
          onSelect={() => undefined}
          owned={{
            identity: intelligence.identity,
            read: intelligence.outline,
          }}
        />
      </>
    );
  }
  const render = (text: string, complete = true) =>
    flushSync(() =>
      root.render(
        <StrictMode>
          <ThemeProvider theme={createTheme()}>
            <SurfaceProvider>
              <Consumer text={text} complete={complete} />
            </SurfaceProvider>
          </ThemeProvider>
        </StrictMode>,
      )
    );
  const calls = async (count: number) => {
    await until(() => f.calls.length >= count, `request ${count}`);
    check(
      f.calls.length === count,
      `unexpected request count ${f.calls.length}/${count}`,
    );
  };
  const status = () =>
    container.querySelector("[data-review-code-status]")?.getAttribute(
      "data-review-code-status",
    );
  const click = (label: string) => {
    const button = [...document.querySelectorAll("button")].find((button) =>
      button.textContent === label
    );
    check(button, `missing button ${label}`);
    flushSync(() => button.click());
  };
  const initial = await captureContent("a🙂z\n");
  const changed = await captureContent("different\n");
  try {
    render("a🙂z\r\n");
    await calls(1);
    check(
      f.registry.retained().length === 1,
      "StrictMode orphaned a buffer before discovery",
    );
    f.reply(0, wire("prepared"));
    await calls(2);
    f.reply(1, wire("open"));
    await calls(3);
    const body = JSON.parse(String(f.calls[2]!.init.body));
    check(
      body.kind === "content" &&
        body.content.sha256 === capturedIdentity(initial).sha256,
      "read did not hash displayed LF text",
    );
    check(
      container.querySelector("pre")?.textContent === initial.text,
      "display and hash differ",
    );
    f.reply(2, contentWire(initial));
    await until(() => status() === "ready", "matched content");
    tests.push(
      "actual Review StrictMode effects allocate one owner after discovery and render/hash identical normalized LF text",
    );

    check(latest, "missing hook");
    const hover = latest.hover(0, 3, new AbortController().signal);
    await calls(4);
    click("Outline");
    // Outline queues behind hover instead of borrowing a second native owner.
    await new Promise<void>((resolve) => setTimeout(resolve, 30));
    check(count() === 4, "Outline overlapped native borrow");
    f.reply(3, {
      ...readWire("language"),
      result: {
        kind: "content",
        content: capturedIdentity(initial),
        result: {
          kind: "hover",
          contents: [{ text: "exact hover", language: null, markdown: false }],
        },
      },
    });
    check(
      (await hover).contents[0]?.text === "exact hover",
      "hover lost its content",
    );
    await calls(5);
    check(
      JSON.parse(String(f.calls[4]!.init.body)).query.kind === "symbols",
      "Outline used legacy API",
    );
    f.reply(4, contentWire(initial, "symbols"));
    await until(
      () =>
        document.body.textContent?.includes("No symbols in this file") === true,
      "actual Outline",
    );
    tests.push(
      "actual Outline and hover share one serialized content-bound owner without a legacy request",
    );

    // Close the real Outline sheet before changing its source snapshot.
    const close = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Close"]',
    );
    check(close, "missing Outline close");
    flushSync(() => close.click());
    const stale = latest.hover(0, 3, new AbortController().signal).then(
      () => "accepted",
      (error: unknown) =>
        error instanceof BufferClientError ? error.kind : "wrong error",
    );
    await calls(6);
    render(changed.text);
    check(
      container.querySelector("output")?.textContent === "none",
      "old language survived replacement render",
    );
    check(
      await stale === "cancelled",
      "old text observer survived replacement",
    );
    f.reply(5, {
      ...readWire("language"),
      result: {
        kind: "content",
        content: capturedIdentity(initial),
        result: {
          kind: "hover",
          contents: [{
            text: "stale private result",
            language: null,
            markdown: false,
          }],
        },
      },
    });
    await calls(7);
    check(
      f.calls[6]!.url === `/api/code/buffers/${ID}/read`,
      "replacement reopened path",
    );
    f.reply(6, contentWire(changed));
    await until(() => status() === "ready", "replacement match");
    check(
      !container.textContent?.includes("stale private result"),
      "late hover painted",
    );
    tests.push(
      "displayed text replacement cancels old hover, hides old annotations synchronously and drains before reading the same owner",
    );

    render(changed.text, false);
    check(
      container.querySelector("output")?.textContent === "none",
      "partial content retained annotations",
    );
    await until(() => status() === "incomplete", "partial document");
    check(count() === 7, "partial content made a read");
    const partial = await latest.outline(new AbortController().signal).then(
      () => "accepted",
      () => "refused",
    );
    check(
      partial === "refused" && count() === 7,
      "partial Outline escaped to legacy",
    );
    render(changed.text);
    await calls(8);
    f.reply(7, contentWire(changed, "language", true));
    await until(() => status() === "mismatch", "mismatch");
    tests.push(
      "partial files cannot request positions or retain language results; mismatch stays explicit without native reload or reopen",
    );

    click("Reload from disk…");
    await calls(9);
    check(
      f.calls[8]!.init.method === "POST" &&
        f.calls[8]!.url.endsWith(`/${ID}/synchronizations`),
      "refresh skipped preparation",
    );
    f.reply(8, {
      apiVersion: 1,
      operationId: `sync-${ID}`,
      resourceId: ID,
      purpose: "refresh_from_disk",
      content: capturedIdentity(changed),
      state: { kind: "prepared" },
      pending: false,
    });
    await until(() => status() === "synchronization", "refresh preparation");
    check(
      count() === 9 &&
        container.textContent?.includes("Settings → About"),
      "refresh applied without confirmation",
    );
    let requested: unknown;
    const settings = (event: Event) => {
      requested = appSettingsFromEvent(event);
    };
    globalThis.addEventListener(OPEN_APP_SETTINGS_EVENT, settings, {
      once: true,
    });
    try {
      click("Confirm in Settings");
    } finally {
      globalThis.removeEventListener(OPEN_APP_SETTINGS_EVENT, settings);
    }
    check(
      JSON.stringify(requested) ===
          JSON.stringify({ tab: "info", section: "code" }) && count() === 9,
      "settings link admitted an effect or wrong route",
    );
    tests.push(
      "Review refresh only prepares the original content; Apply and retirement remain separately confirmed core actions",
    );

    const owner = f.registry.retained()[0]!;
    const operation = owner.synchronization();
    check(operation, "missing retained synchronization");
    const preview = operation.preview("apply");
    render(initial.text);
    await until(() => owner.view().closing, "content abandonment fence");
    check(
      !operation.isCurrent(preview),
      "old Settings confirmation remained current",
    );
    await until(() => status() === "unavailable", "ended source consumer");
    check(
      count() === 9,
      "abandonment applied, retired, released or reopened automatically",
    );
    check(
      f.registry.retained().length === 1 && owner.view().synchronizing,
      "uncertain cleanup forgotten",
    );
    tests.push(
      "changing text after preparation fences stale Apply immediately and retains the original synchronization for explicit cleanup",
    );
  } finally {
    f.context.abort();
    flushSync(() => root.unmount());
    container.remove();
  }
  return tests;
}
