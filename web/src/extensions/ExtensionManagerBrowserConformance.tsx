/** Real generic management UI with fixture inventory and HTTP effects only. */
import { StrictMode } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { createTheme, ThemeProvider } from "@mui/material";
import { SurfaceProvider } from "../surface/SurfaceProfile.tsx";
import type { MachineSummary } from "../protocol.ts";
import type { PluginRelease } from "../admin/adminApi.ts";
import { lifecycleFixture } from "../pluginLifecycle.fixture.ts";
import { ExtensionManagerView } from "./ExtensionManager.tsx";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 50));
async function until(label: string, predicate: () => boolean) {
  for (let n = 0; n < 300; n++) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`Extension manager timed out: ${label}`);
}
const machine = (id: string, sdk = "1.9.0"): MachineSummary => ({
  id,
  display_name: id,
  platform: "linux",
  architecture: "x86_64",
  status: "online",
  local: false,
  connected: true,
  schedulable: true,
  workspaces: [],
  components: [],
  plugins: [],
  capacity: { max_sessions: 10, draining: false },
  active_sessions: 0,
  plugin_contracts: {
    plugin_sdk_version: sdk,
    min_manifest_schema: 1,
    max_manifest_schema: 1,
    min_package_schema: 1,
    max_package_schema: 1,
    min_release_schema: 1,
    max_release_schema: 3,
    min_agent_provider_schema: 1,
    max_agent_provider_schema: 1,
    min_authentication_provider_schema: 1,
    max_authentication_provider_schema: 1,
    min_code_intelligence_schema: 1,
    max_code_intelligence_schema: 1,
    min_host_bundle_schema: 1,
    max_host_bundle_schema: 1,
    min_host_schema: 1,
    max_host_schema: 1,
  },
});
const release = (
  version: string,
  digit: string,
  sdk = "1.9.0",
): PluginRelease => ({
  plugin_id: "fixture-tools",
  plugin_version: version,
  plugin_kind: "workspace_extension",
  package_digest: `sha256:${digit.repeat(64)}`,
  artifact_digest: `sha256:${digit.repeat(64)}`,
  release_state: "ready",
  publisher: "fixture",
  supported_platforms: [{ os: "linux", architecture: "x86_64" }],
  compatibility_requirements: {
    plugin_sdk_version: sdk,
    manifest_schema: 1,
    package_schema: 1,
    release_schema: 3,
    plugin_kind: "workspace_extension",
    payload_schema: 1,
  },
});

export async function runExtensionManagerBrowserConformance(): Promise<
  string[]
> {
  const originalFetch = globalThis.fetch;
  const tests: string[] = [];
  const future = release("2.0.0", "2", "1.10.0");
  const compatible = release("1.0.0", "1");
  let catalog = [future];
  let catalogFails = false;
  const effects: Array<
    { path: string; body: unknown; resolve: (response: Response) => void }
  > = [];
  const historyReads: string[] = [];
  globalThis.fetch = (input, init) => {
    const url = new URL(String(input), location.href);
    if (url.pathname === "/api/plugins") {
      return Promise.resolve(
        catalogFails
          ? new Response("private diagnostic", { status: 503 })
          : Response.json({ plugins: catalog }),
      );
    }
    if (url.pathname.endsWith("/lifecycle-history")) {
      check((init?.method ?? "GET") === "GET", "history dispatched an effect");
      historyReads.push(url.pathname);
      return Promise.resolve(
        Response.json(
          lifecycleFixture(url.pathname.split("/")[3], "fixture-tools"),
        ),
      );
    }
    check(init?.method === "POST", "unexpected management read");
    return new Promise((resolve) =>
      effects.push({
        path: url.pathname,
        body: JSON.parse(String(init.body)),
        resolve,
      })
    );
  };
  const container = document.createElement("div");
  container.style.cssText = "width:420px;min-height:600px";
  document.body.append(container);
  const root = createRoot(container);
  const theme = createTheme();
  const render = (
    key: string,
    machines: MachineSummary[] = [machine("fixture")],
  ) =>
    flushSync(() =>
      root.render(
        <StrictMode>
          <ThemeProvider theme={theme}>
            <SurfaceProvider>
              <ExtensionManagerView key={key} machines={machines} />
            </SurfaceProvider>
          </ThemeProvider>
        </StrictMode>,
      )
    );
  const text = () => container.textContent ?? "";
  const button = (label: string) => {
    const node = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find((candidate) =>
        candidate.getAttribute("aria-label") === label ||
        candidate.textContent?.trim() === label
      );
    check(node, `missing ${label}`);
    return node;
  };
  const click = (label: string) => {
    const node = button(label);
    check(!node.disabled, `disabled ${label}`);
    flushSync(() => node.click());
  };
  const choose = async (index: number, value: string) => {
    await until(
      "selector ready",
      () =>
        container.querySelectorAll('[role="combobox"]')[index]?.getAttribute(
          "aria-disabled",
        ) !== "true",
    );
    const select = container.querySelectorAll('[role="combobox"]')[index];
    check(select, "missing selector");
    flushSync(() =>
      select.dispatchEvent(
        new MouseEvent("mousedown", { bubbles: true, button: 0 }),
      )
    );
    await until(
      "selected option available",
      () =>
        [...document.querySelectorAll('[role="option"]')].some((node) =>
          node.getAttribute("data-value") === value
        ),
    );
    const option = [
      ...document.querySelectorAll<HTMLElement>('[role="option"]'),
    ].find((node) => node.getAttribute("data-value") === value);
    check(option, `missing option ${value}`);
    flushSync(() => option.click());
    await settle();
  };
  try {
    render("compatibility");
    await until("catalog", () => text().includes("2.0.0"));
    check(
      button("Install").disabled,
      "extension manager enabled a release requiring a newer Plugin SDK",
    );
    check(
      text().includes("1.10.0"),
      "compatibility failure omitted the required SDK",
    );
    tests.push(
      "extension installation uses the shared complete Plugin compatibility contract",
    );

    catalogFails = true;
    render("recovery");
    await until(
      "catalog failure",
      () => text().includes("Extension catalog unavailable"),
    );
    check(
      !text().includes("No extensions are available"),
      "catalog failure was presented as an empty catalog",
    );
    catalogFails = false;
    catalog = [future, compatible];
    click("Refresh extension catalog");
    await until("recovered catalog", () => text().includes("1.0.0"));
    check(
      !text().includes("Extension catalog unavailable"),
      "successful refresh retained the old catalog error",
    );
    check(
      !button("Install").disabled,
      "latest compatible release was not selected by default",
    );
    tests.push(
      "catalog recovery clears its error and selects the newest compatible release",
    );

    render("selection", [machine("fixture"), machine("newer", "1.10.0")]);
    await until("selection catalog", () => text().includes("1.0.0"));
    await choose(1, future.artifact_digest!);
    check(
      button("Install").disabled,
      "explicit incompatible release was not blocked",
    );
    await choose(0, "newer");
    check(
      !button("Install").disabled && text().includes("2.0.0"),
      "new Machine retained another Machine's compatibility selection",
    );
    await choose(1, compatible.artifact_digest!);
    await choose(0, "fixture");
    check(
      button("Install").disabled && text().includes("2.0.0"),
      "manual version choice was not scoped to its Machine",
    );
    tests.push(
      "manual version choices remain scoped to each Machine and keep incompatible choices visibly blocked",
    );

    render("delayed", []);
    await settle();
    render("delayed", [machine("fixture")]);
    await until("delayed inventory", () => text().includes("1.0.0"));
    check(
      !button("Install").disabled,
      "Machine arriving after mount left management without a target",
    );
    render("delayed", [machine("other"), machine("fixture")]);
    check(
      container.querySelector('[role="combobox"]')?.textContent?.includes(
        "fixture",
      ),
      "Machine inventory reorder silently changed the selected target",
    );
    tests.push("late Machine inventory supplies a usable initial target");

    const install = button("Install");
    install.click();
    install.click();
    await settle();
    check(
      effects.length === 1,
      "one pending installation dispatched duplicate requests",
    );
    const attempt = effects[0]!;
    check(
      attempt.path === "/api/machines/fixture/plugins/fixture-tools",
      "installation used the wrong Machine",
    );
    const body = attempt.body as Record<string, unknown>;
    check(
      body.version === "1.0.0" && body.digest === compatible.artifact_digest &&
        typeof body.operation_id === "string",
      "installation lost its exact selected release or operation identity",
    );
    attempt.resolve(new Response("private diagnostic", { status: 503 }));
    await until(
      "failed installation",
      () => text().includes("operation could not be completed"),
    );
    click("Refresh extension catalog");
    await settle();
    check(
      text().includes("operation could not be completed"),
      "catalog refresh concealed an unresolved installation failure",
    );
    check(effects.length === 1, "catalog refresh retried a mutation");
    tests.push(
      "duplicate clicks share one pending action and catalog refresh preserves unresolved operation feedback",
    );

    click("Plugin operation history");
    await until(
      "operation history",
      () => text().includes("operation-shared-fixture"),
    );
    check(
      historyReads.length === 1 && effects.length === 1,
      "opening history repeated an installation",
    );
    click("Refresh evidence");
    await until("refreshed evidence", () => historyReads.length === 2);
    check(effects.length === 1, "reading receipts dispatched a mutation");
    tests.push(
      "extensions expose the shared read-only operation history without replaying effects",
    );

    const installed = machine("fixture", "1.10.0");
    installed.plugins = [{
      plugin_id: "fixture-tools",
      plugin_version: "1.0.0",
      plugin_kind: "workspace_extension",
      generation_digest: compatible.artifact_digest,
      contract_fingerprint: compatible.package_digest,
      state: "active",
      active_session_leases: 0,
      replica_state: "absent",
      materialization_state: "not_installed",
    }];
    render("installed", [installed]);
    await until(
      "installed release",
      () => text().includes("1.0.0 · Current installation"),
    );
    check(
      button("Installed").disabled,
      "opening management silently selected an upgrade",
    );
    await until(
      "available update",
      () => text().includes("Update available: 2.0.0"),
    );
    await choose(1, future.artifact_digest!);
    check(
      !button("Change version").disabled,
      "explicit compatible upgrade unavailable",
    );
    catalog = [compatible];
    click("Refresh extension catalog");
    await until(
      "removed selected release",
      () => text().includes("Selected release unavailable"),
    );
    check(
      button("Change version").disabled,
      "catalog refresh replaced an explicit digest with another release",
    );
    await choose(1, compatible.artifact_digest!);
    catalog = [];
    click("Refresh extension catalog");
    await until(
      "installed release absent from catalog",
      () => text().includes("This exact release is no longer available"),
    );
    check(
      button("Installed").disabled && !button("Uninstall").disabled,
      "catalog absence hid the actual installation",
    );
    tests.push(
      "installed and explicitly selected digests survive catalog changes without silent upgrades or fallback",
    );

    catalog = [compatible];
    const invalid = machine("fixture");
    invalid.plugins = [{ plugin_id: "fixture-tools", state: "invalid" }];
    render("inventory", [invalid]);
    await until(
      "invalid inventory",
      () => text().includes("Installation status unavailable"),
    );
    check(
      button("Install").disabled && !text().includes("Not installed"),
      "invalid inventory became an absent installation",
    );
    render("inventory", [{ ...machine("fixture"), status: "offline" }]);
    check(
      button("Install").disabled && text().includes("is offline"),
      "offline Machine lacked a reason for disabled installation",
    );
    render("inventory", [machine("fixture")]);
    const searchInput = container.querySelector<HTMLInputElement>(
      'input:not([aria-hidden="true"])',
    );
    check(searchInput, "missing extension filter");
    flushSync(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!
        .call(searchInput, "missing-extension");
      searchInput.dispatchEvent(new Event("input", { bubbles: true }));
    });
    check(
      text().includes("No extensions match your search"),
      "empty search left a blank manager",
    );
    tests.push(
      "unknown inventory, offline Machines and empty search retain explicit actionable states",
    );
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    globalThis.fetch = originalFetch;
  }
  return tests;
}
