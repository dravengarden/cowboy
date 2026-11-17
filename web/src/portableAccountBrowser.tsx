import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SubscriptionApiKey } from "./SubscriptionApiKey";
import type { PortableCredential } from "./subscriptionAccounts";

/** A named DeepSeek account card: the key is posted once, cleared and never shown. */
export async function portableAccountBrowser(observe?: (surface: string, stage: string) => Promise<void>): Promise<string[]> {
  const originalFetch = globalThis.fetch;
  const bodies: { url: string; body: unknown }[] = [];
  let reject = true;
  let changes = 0;
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    bodies.push({ url: String(input), body: JSON.parse(String(init?.body)) });
    return reject
      ? Response.json({ error: { code: "invalid_secret" } }, { status: 422 })
      : Response.json({ status: "committed" }, { status: 201 });
  }) as typeof fetch;
  const container = document.createElement("div");
  container.style.cssText = "width:360px;max-width:100%;padding:12px;box-sizing:border-box";
  document.body.append(container);
  const root = createRoot(container);
  const results: string[] = [];
  const check = (condition: unknown, message: string) => { if (!condition) throw new Error(message); results.push(message); };
  const waitFor = async (condition: () => boolean) => {
    for (let attempt = 0; attempt < 100; ++attempt) {
      if (condition()) return;
      await new Promise((resolve) => setTimeout(resolve, 30));
    }
    throw new Error(`Portable account fixture timed out: ${container.textContent}`);
  };
  const render = (credential: PortableCredential | undefined) => flushSync(() => root.render(<BrowserProductTheme><CssBaseline />
    <SubscriptionApiKey account="7c1f0e8e-4b9b-4d55-9a2a-5f0f3c1f7a10" method={{ id: "api-key", label: "DeepSeek API key" }}
      credential={credential} disabled={false} expanded onChange={() => { ++changes; }} />
  </BrowserProductTheme>));
  const input = () => container.querySelector<HTMLInputElement>("input[data-subscription-api-key]")!;
  const save = () => [...container.querySelectorAll("button")].find((node) => /key$/.test(node.textContent ?? ""))!;
  const type = (value: string) => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input(), value);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  };
  try {
    render(undefined);
    check(input()?.type === "password" && container.textContent!.includes("No API key"), "an unkeyed account offers a masked key field");
    check(save().disabled, "an empty key cannot be submitted");
    type("sk-browser-secret");
    await waitFor(() => !save().disabled);
    save().click();
    await waitFor(() => container.textContent!.includes("without line breaks"));
    check(!container.textContent!.includes("sk-browser-secret") && changes === 0, "a refused key is not echoed or reported as saved");
    reject = false;
    save().click();
    await waitFor(() => changes === 1 && input().value === "");
    check(bodies.length === 2 && bodies[1]!.url === "/api/provider-accounts/7c1f0e8e-4b9b-4d55-9a2a-5f0f3c1f7a10/credentials" &&
      JSON.stringify(bodies[1]!.body) === JSON.stringify({ method: "api-key", secret: "sk-browser-secret" }),
    "the key is posted once to the account's credential route and then cleared");
    render({ account_id: "7c1f0e8e-4b9b-4d55-9a2a-5f0f3c1f7a10", provider_id: "claude-deepseek", authentication_state: "ready",
      auth_generation: 1, distribution_state: "current", updated_at_ms: 1 });
    check(container.textContent!.includes("API key saved") && save().textContent === "Replace key", "a saved key is shown as state only");
    check(container.scrollWidth <= container.clientWidth + 1, "the key card fits a narrow settings panel");
    await observe?.("mobile", "portable-account");
  } finally {
    root.unmount();
    container.remove();
    globalThis.fetch = originalFetch;
  }
  return results;
}
