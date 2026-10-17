/** Synthetic account/HTTP only; exercises the real preference owner and editor. */
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Button, CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { AboutLayoutEditor } from "./AboutLayoutEditor";
import { type AboutPreferences, PREFERENCES_CHANGED, useAboutPreferences } from "./aboutPreferences";
import { bindProductSyncPrincipal } from "./productSyncIdentity";

export async function runAboutLayoutBrowserConformance(keepFixture = false): Promise<string[]> {
  bindProductSyncPrincipal("layout-fixture");
  const realFetch = globalThis.fetch;
  const container = document.createElement("div");
  container.style.cssText = "width:360px;max-width:100%;padding:12px;box-sizing:border-box";
  document.body.append(container);
  const root = createRoot(container);
  const tests: string[] = [];
  const data: AboutPreferences = {
    preferences: { scope: "about", revision: 0, values: { usage_order: [], machine_order: [] } },
    choices: { usage_order: [{ id: "anthropic", name: "Anthropic" }, { id: "openai", name: "OpenAI" }], machine_order: [{ id: "hawk", name: "Hawk" }, { id: "ovh", name: "OVH" }] },
    dataset: { dataset_id: "fixture-dataset", user_id: "layout-fixture" },
  };
  const writes: string[] = [];
  let failure = false;
  let conflict = false;
  globalThis.fetch = (async (_input: RequestInfo | URL, init?: RequestInit) => {
    if (init?.method !== "PUT") return Response.json(data);
    const body = String(init.body);
    writes.push(body);
    if (failure) throw new Error("Connection lost");
    if (conflict) {
      conflict = false;
      data.preferences.revision += 1;
      return Response.json({ error: { code: "revision_conflict" }, current: data.preferences }, { status: 409 });
    }
    const input = JSON.parse(body);
    if (input.expected_revision !== data.preferences.revision) throw new Error("stale revision");
    data.preferences = { ...data.preferences, revision: data.preferences.revision + 1, values: { ...data.preferences.values, ...input.changes } };
    return Response.json({ status: "committed", preferences: data.preferences });
  }) as typeof fetch;
  function Fixture({ desktop = false }: { desktop?: boolean }) {
    const layout = useAboutPreferences(true);
    const [message, setMessage] = useState("");
    if (!layout.data) return <div>Loading</div>;
    return <BrowserProductTheme><CssBaseline />
      <AboutLayoutEditor desktop={desktop} choices={layout.data.choices} values={{ ...layout.data.preferences.values, ...layout.pending?.changes }} disabled={layout.saving || !!layout.pending} onChange={(changes) => { void layout.save(changes).then(() => setMessage("saved")); }} />
      <div role="status">{layout.error ?? message}</div>
      {layout.pending && <Button onClick={() => { void layout.retry(); }}>Retry</Button>}
    </BrowserProductTheme>;
  }
  const until = async (predicate: () => boolean) => {
    for (let n = 0; n < 150; n++) { if (predicate()) return; await new Promise((resolve) => setTimeout(resolve, 20)); }
    throw new Error(`layout fixture timed out: ${container.textContent}`);
  };
  const check = (ok: boolean, label: string) => { if (!ok) throw new Error(label); tests.push(label); };
  const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent === label)!;
  const move = async (name: string, label: string) => {
    await until(() => document.querySelector('[role="menu"]') === null);
    container.querySelector<HTMLButtonElement>(`[aria-label="Move ${name}"]`)!.click();
    await until(() => document.querySelector('[role="menu"]') !== null);
    [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find((item) => item.textContent === label)!.click();
    await until(() => document.querySelector('[role="menu"]') === null);
  };
  try {
    flushSync(() => root.render(<Fixture />));
    await until(() => container.querySelector('[aria-label="Move OpenAI"]') !== null);
    check(container.scrollWidth <= container.clientWidth, "phone editor fits without horizontal overflow");
    container.querySelector<HTMLButtonElement>('[aria-label="Move OpenAI"]')!.focus();
    check(document.activeElement?.getAttribute("aria-label") === "Move OpenAI", "move controls support native keyboard focus");
    await move("OpenAI", "Move to top");
    await until(() => data.preferences.revision === 1 && !container.querySelector<HTMLButtonElement>('[aria-label="Move OpenAI"]')!.disabled);
    check(data.preferences.values.usage_order[0] === "openai", "provider reorder persists through shared preference writer");
    failure = true;
    await move("OVH", "Move to top");
    await until(() => !!button("Retry"));
    check(container.textContent?.includes("Connection lost") === true, "uncertain write retains proposal and shows error");
    const uncertain = writes.at(-1);
    failure = false;
    button("Retry").click();
    await until(() => data.preferences.revision === 2 && !button("Retry"));
    check(writes.at(-1) === uncertain, "retry reuses exact operation and input");
    conflict = true;
    await move("Hawk", "Move to top");
    await until(() => container.textContent?.includes("Layout changed elsewhere") === true);
    check(data.preferences.values.machine_order[0] === "ovh", "conflict preserves accepted remote order");
    button("Retry").click();
    await until(() => data.preferences.revision === 4 && !button("Retry"));
    check(data.preferences.values.machine_order[0] === "hawk", "explicit retry applies retained proposal on fresh revision");
    data.preferences = { ...data.preferences, revision: 5, values: { usage_order: ["anthropic", "openai"], machine_order: ["ovh", "hawk"] } };
    window.dispatchEvent(new Event(PREFERENCES_CHANGED));
    await until(() => container.querySelector('[aria-label^="Move "]')?.getAttribute("aria-label") === "Move Anthropic");
    check(true, "AI or another device invalidation refreshes the open editor");
    container.style.width = "960px";
    flushSync(() => root.render(<Fixture desktop />));
    check(container.scrollWidth <= container.clientWidth, "desktop editor fits with two compact columns");
    button("Restore default order").click();
    await until(() => data.preferences.revision === 6);
    check(data.preferences.values.usage_order.length === 0 && data.preferences.values.machine_order.length === 0, "restore defaults updates both lists atomically");
    return tests;
  } finally {
    if (!keepFixture) { root.unmount(); container.remove(); }
    globalThis.fetch = realFetch;
  }
}
