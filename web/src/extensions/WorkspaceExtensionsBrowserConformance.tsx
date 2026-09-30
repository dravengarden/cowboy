/** Actual resource navigation with synthetic HTTP, no account or remote repo. */
import { StrictMode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { createTheme, ThemeProvider } from "@mui/material";
import { SurfaceProvider } from "../surface/SurfaceProfile.tsx";
import WorkspaceExtensions from "./WorkspaceExtensions.tsx";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function until(description: string, predicate: () => boolean) {
  for (let n = 0; n < 400; n++) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`Extension navigation timed out: ${description}`);
}
const response = (value: unknown) =>
  new Response(JSON.stringify(value), {
    headers: { "content-type": "application/json" },
  });
const item = (title: string) => ({
  id: "12",
  title,
  url: "https://example.test/owner/repo/issues/12",
  body: "Description",
  bodyTruncated: false,
  state: "open",
  updatedAt: null,
  metadata: [],
});

export async function runWorkspaceExtensionsBrowserConformance(): Promise<
  string[]
> {
  const originalFetch = globalThis.fetch;
  const pending: Array<(value: Response) => void> = [];
  const requests: URL[] = [];
  globalThis.fetch = (input, init) => {
    check(init?.cache === "no-store", "repository reads must bypass caches");
    const url = new URL(String(input), location.href);
    requests.push(url);
    if (url.pathname.endsWith("/extensions")) {
      return Promise.resolve(response({
        type: "inventory",
        remotes: [{
          name: "origin",
          host: "example.test",
          owner: "owner",
          repository: "repo",
        }],
        extensions: [{
          identity: {
            pluginId: "fixture-resources",
            pluginVersion: "1.0.0",
            generationDigest: `sha256:${"1".repeat(64)}`,
          },
          label: "Fixture resources",
          description: "Resources supplied by any extension",
          available: true,
          views: [{ id: "work", label: "Work", filters: [] }],
        }],
      }));
    }
    if (url.searchParams.has("item")) {
      return Promise.resolve(
        response({ type: "detail", item: item("Item detail") }),
      );
    }
    if (url.searchParams.get("page") === "2") {
      return new Promise((resolve) => pending.push(resolve));
    }
    return Promise.resolve(
      response({
        type: "page",
        items: [item(
          url.pathname.includes("second")
            ? "Second workspace"
            : "First workspace",
        )],
        nextPage: 2,
      }),
    );
  };
  const container = document.createElement("div");
  container.style.cssText =
    "width:1100px;height:700px;display:flex;flex-direction:column";
  document.body.append(container);
  const root = createRoot(container);
  const theme = createTheme();
  const render = (context: string) =>
    flushSync(() =>
      root.render(
        <StrictMode>
          <ThemeProvider theme={theme}>
            <SurfaceProvider>
              <WorkspaceExtensions key={context} context={context} />
            </SurfaceProvider>
          </ThemeProvider>
        </StrictMode>,
      )
    );
  function click(label: string) {
    const target = [
      ...container.querySelectorAll<HTMLElement>("button,[role=button]"),
    ].find((button) =>
      button.getAttribute("aria-label") === label ||
      button.textContent?.includes(label)
    );
    check(target, `missing ${label}`);
    check(
      !(target instanceof HTMLButtonElement && target.disabled) &&
        target.getAttribute("aria-disabled") !== "true",
      `disabled ${label}`,
    );
    target.click();
  }
  try {
    render("first");
    await until(
      "first workspace extensions",
      () => container.textContent?.includes("Fixture resources") ?? false,
    );
    click("Fixture resources");
    await until(
      "first workspace items",
      () => container.textContent?.includes("First workspace") ?? false,
    );
    check(
      requests.some((r) =>
        r.searchParams.get("pluginId") === "fixture-resources" &&
        r.searchParams.has("generationDigest")
      ),
      "generic renderer must bind the exact extension",
    );
    click("First workspace");
    await until(
      "resource detail",
      () => container.textContent?.includes("Item detail") ?? false,
    );
    const link = container.querySelector<HTMLAnchorElement>(
      'a[aria-label="Open original resource"]',
    );
    check(
      link?.target === "_blank" && link.rel.includes("noopener"),
      "detail source link must be explicit",
    );
    click("Back to list");
    await until(
      "next page available after returning to the list",
      () =>
        container.querySelector<HTMLButtonElement>('[aria-label="Next page"]')
          ?.disabled === false,
    );
    click("Next page");
    await until("pending page request", () => pending.length > 0);
    render("second");
    await until(
      "second workspace extensions",
      () => container.textContent?.includes("Fixture resources") ?? false,
    );
    click("Fixture resources");
    await until(
      "second workspace items",
      () => container.textContent?.includes("Second workspace") ?? false,
    );
    pending.forEach((resolve) =>
      resolve(
        response({
          type: "page",
          items: [item("STALE PRIVATE RESOURCE")],
          nextPage: null,
        }),
      )
    );
    await new Promise<void>((resolve) => setTimeout(resolve, 50));
    check(
      !container.textContent?.includes("STALE PRIVATE RESOURCE"),
      "late response escaped its original context",
    );
    return [
      "generic exact-release resource navigation",
      "readable detail and explicit source link",
      "late private response discarded across workspace change",
    ];
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    globalThis.fetch = originalFetch;
  }
}
