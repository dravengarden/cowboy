/** Production selection hook and picker, synthetic HTTP in an isolated browser. */
import { StrictMode, useLayoutEffect } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import type { MachineSummary } from "./protocol";
import { useProjectPlacement } from "./useProjectPlacement";
import { WorkspacePicker } from "./WorkspacePicker";
import { managementEntryFixture } from "./providerManagement.fixture";
import { resetProviderCatalog } from "./providerCatalogRegistry";
import { projectMachineOccupancy } from "./machineState";
import { AiInstallationPicker } from "./AiInstallationPicker";
import { SessionMachineBadge } from "./SessionMachineBadge";
import { useReliableTouchTap } from "./useReliableTouchTap";

export async function runProjectPlacementBrowserConformance(): Promise<
  string[]
> {
  const originalFetch = globalThis.fetch;
  const entry = managementEntryFixture("codex");
  entry.manifest.display.name = "Codex";
  entry.publisher = entry.manifest.publisher;
  const claude = managementEntryFixture("claude-code");
  claude.manifest.display.name = "Claude Code";
  claude.publisher = claude.manifest.publisher;
  const entries = [entry, claude];
  const machines = ["hawk", "falcon", "ovh"].map((id) => ({
    id,
    display_name: id,
    platform: "linux",
    architecture: "x86_64",
    status: "online",
    components: [],
    capacity: { max_sessions: 8, draining: false },
    active_sessions: 0,
    local: id === "hawk",
    connected: true,
    schedulable: id !== "ovh",
    workspaces: id === "ovh" ? [] : [{
      id: "stable-id",
      display_name: "columbus/cowboy",
      canonical_path: "/unrelated directory ' 中文",
    }],
    plugins: id !== "falcon"
      ? (id === "ovh" ? entries : [entry]).map((entry) => ({
        plugin_id: entry.provider_id,
        plugin_kind: "agent_provider",
        plugin_version: entry.provider_version,
        generation_digest: entry.artifact_digest,
        contract_fingerprint: entry.contract_fingerprint,
        state: "active",
        materialization_state: "current",
        replica_state: "current",
        active_session_leases: 0,
      }))
      : [],
  } as MachineSummary));
  let pendingFalcon: ((value: Response) => void) | undefined;
  const ready = (machine: string) => ({
    machine_id: machine,
    default_runtime_machine_id: "ovh",
    placements: [
      ...entries.map((entry) => ({
        runtime_machine_id: "ovh",
        provider: entry.provider_id,
        mode: "remote",
      })),
      ...(machine === "hawk"
        ? [{ runtime_machine_id: "hawk", provider: "codex", mode: "local" }]
        : []),
    ],
  });
  globalThis.fetch = ((input: RequestInfo | URL) => {
    const url = String(input);
    if (url === "/api/plugins") {
      return Promise.resolve(
        Response.json({
          providers: entries,
          authentications: [],
          authentication_executors: [],
        }),
      );
    }
    if (url === "/api/project-policies") {
      return Promise.resolve(
        Response.json({
          schema: 1,
          revision: "r1",
          default_runtime_machine_id: "ovh",
          machines: {
            ovh: {
              agent_mode: "remote",
              hosts_projects: false,
              remote_targets: ["hawk", "falcon"],
            },
          },
        }),
      );
    }
    if (url.endsWith("machine_id=hawk")) {
      return Promise.resolve(Response.json(ready("hawk")));
    }
    if (url.endsWith("machine_id=falcon")) {
      return new Promise<Response>((resolve) => {
        pendingFalcon = resolve;
      });
    }
    throw new Error(`Unexpected fixture request: ${url}`);
  }) as typeof fetch;
  resetProviderCatalog();
  const container = document.createElement("div");
  container.style.width = "360px";
  container.style.display = "flex";
  container.style.flexDirection = "column";
  container.style.gap = "16px";
  document.body.append(container);
  const root = createRoot(container);
  let current: ReturnType<typeof useProjectPlacement> | undefined;
  function Harness({ inventory }: { inventory: MachineSummary[] }) {
    // Exercise the same session-derived projection that store.ts applies to
    // HTTP and WebSocket inventories before the New Session picker reads them.
    const placement = useProjectPlacement(
      true,
      projectMachineOccupancy(inventory, []),
    );
    useLayoutEffect(() => {
      current = placement;
    });
    return (
      <>
        <WorkspacePicker
          label="Project"
          entries={placement.projects}
          value={placement.project?.value ?? ""}
          onChange={placement.selectProject}
        />
        <AiInstallationPicker
          installations={placement.installations}
          value={placement.installation?.value ?? ""}
          onChange={placement.selectInstallation}
          helperText="Remote · AI on OVH · Files and commands on Hawk"
        />
      </>
    );
  }
  const render = (inventory = machines): void =>
    flushSync(() =>
      root.render(
        <StrictMode>
          <Harness inventory={inventory} />
        </StrictMode>,
      )
    );
  const wait = async (
    predicate: () => unknown,
    label: string,
  ): Promise<void> => {
    for (let attempt = 0; attempt < 160; attempt++) {
      if (predicate()) return;
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    throw new Error(`${label}: ${current?.error ?? "timeout"}`);
  };
  const check = (value: unknown, label: string): void => {
    if (!value) throw new Error(label);
  };
  try {
    render();
    await wait(() => current?.ready, "initial readiness");
    check(
      current?.machineId === "hawk" && current.runtimeMachineId === "ovh" &&
        current.separate,
      "AI and project Machines must be independent",
    );
    check(
      current?.projects.length === 2 && current.installations.length === 3,
      "runtime needs no mirror project",
    );
    const field = () =>
      container.querySelector<HTMLElement>(
        '.MuiSelect-select[role="combobox"]',
      )!;
    const checkAlignment = (): void => {
      const icon = field().querySelector<SVGElement>('[role="img"]')!;
      const name = field().querySelector<HTMLElement>("[title]")!;
      check(icon && name, "selected installation has an icon and label");
      const a = icon.getBoundingClientRect();
      const b = name.getBoundingClientRect();
      const input = container.querySelector('[role="combobox"]')!.closest(
        ".MuiInputBase-root",
      )!.getBoundingClientRect();
      check(
        a.right < b.left &&
          Math.abs((a.top + a.bottom) - (b.top + b.bottom)) < 2,
        "selected icon and label share one horizontal row",
      );
      check(
        Math.abs(
          field().closest(".MuiInputBase-root")!.getBoundingClientRect()
            .height - input.height,
        ) < 2,
        "closed AI field is as compact as the project field",
      );
      check(
        b.right <= field().getBoundingClientRect().right,
        "selected text stays inside the field",
      );
    };
    checkAlignment();
    for (const width of [320, 375, 720]) {
      container.style.width = `${width}px`;
      render(
        machines.map((m) =>
          m.id === "ovh"
            ? { ...m, display_name: `OVH ${"long-machine-name".repeat(10)}` }
            : m
        ),
      );
      checkAlignment();
      const name = field().querySelector<HTMLElement>("[title]")!;
      check(
        name.scrollWidth > name.clientWidth &&
          getComputedStyle(name).textOverflow === "ellipsis",
        "long selected names truncate within mobile and desktop widths",
      );
    }
    container.style.width = "360px";
    render();
    flushSync(() =>
      field().dispatchEvent(
        new MouseEvent("mousedown", { button: 0, bubbles: true }),
      )
    );
    await wait(
      () => document.querySelector('[role="option"]'),
      "AI menu opens",
    );
    const claudeOption = [
      ...document.querySelectorAll<HTMLElement>('[role="option"]'),
    ]
      .find((option) => option.textContent?.includes("Claude Code · ovh"))!;
    check(
      claudeOption?.textContent?.includes("Remote"),
      "AI menu retains mode detail",
    );
    flushSync(() => claudeOption.click());
    await wait(
      () => !document.querySelector('[role="listbox"]'),
      "AI menu closes",
    );
    checkAlignment();
    check(
      current?.ready && current.runtimeMachineId === "ovh" &&
        current.separate &&
        current.installation?.provider === "claude-code",
      "Claude remains selectable on an AI Machine without projects",
    );
    check(
      container.querySelector("label")?.textContent === "Project",
      "project presentation",
    );
    flushSync(() =>
      current!.selectProject(JSON.stringify(["falcon", "stable-id"]))
    );
    await wait(
      () => pendingFalcon && current?.machineId === "falcon",
      "switch to Falcon",
    );
    check(
      !current?.ready && current?.runtimeMachineId === "",
      "old readiness must not cross project selection",
    );
    const lateFalcon = pendingFalcon!;
    flushSync(() =>
      current!.selectProject(JSON.stringify(["hawk", "stable-id"]))
    );
    await wait(() => current?.ready, "return to Hawk");
    lateFalcon(Response.json(ready("falcon")));
    await new Promise((resolve) => setTimeout(resolve, 50));
    check(
      current?.machineId === "hawk" && current.ready,
      "late target response must be ignored",
    );
    pendingFalcon = undefined;
    flushSync(() =>
      current!.selectProject(JSON.stringify(["falcon", "stable-id"]))
    );
    await wait(() => pendingFalcon, "Falcon readiness request");
    pendingFalcon!(new Response("target unavailable", { status: 503 }));
    await wait(() => current?.error, "target error");
    check(
      !current?.ready && current?.machineId === "falcon",
      "target error cannot fall back to Hawk or OVH",
    );
    render(
      machines.map((machine) => ({
        ...machine,
        workspaces: [...machine.workspaces],
      })),
    );
    check(
      current?.machineId === "falcon",
      "inventory refresh preserves project selection",
    );
    render(
      machines.map((m) => m.id === "ovh" ? { ...m, connected: false } : m),
    );
    flushSync(() =>
      current!.selectProject(JSON.stringify(["hawk", "stable-id"]))
    );
    await wait(
      () => current?.installations.some((i) => i.runtime_machine_id === "hawk"),
      "available local alternative",
    );
    check(
      !current?.ready && !current?.installation,
      "unavailable preferred runtime cannot silently select local AI",
    );
    flushSync(() =>
      current!.selectInstallation(JSON.stringify(["hawk", "codex"]))
    );
    check(
      current?.ready && !current.separate,
      "explicit local installation selection remains available",
    );
    let picked = 0;
    let inspected = 0;
    function SessionRow({ cwd = "/runtime" }: { cwd?: string }) {
      const tap = useReliableTouchTap<HTMLDivElement>(() => picked++);
      return (
        <div role="button" {...tap}>
          <SessionMachineBadge
            session={{
              id: "remote",
              provider: "claude-code",
              machine_id: "ovh",
              cwd,
              title: "Claude",
              status: "running",
              execution_binding: {
                schema: 1,
                runtime: { machine_id: "ovh", cwd: "/runtime" },
                environment: { machine_id: "hawk", protocol: 1 },
                workspace: { cwd: "/target" },
              },
            }}
            onInfo={() => inspected++}
          />
        </div>
      );
    }
    flushSync(() => root.render(<SessionRow />));
    const badge = container.querySelector<HTMLElement>(".MuiChip-root")!;
    check(
      badge.textContent === "OVH → Hawk",
      "remote badge labels both Machines",
    );
    for (const type of ["pointerdown", "pointerup"]) {
      badge.dispatchEvent(
        new PointerEvent(type, {
          bubbles: true,
          pointerId: 1,
          pointerType: "touch",
          isPrimary: true,
          clientX: 10,
          clientY: 10,
        }),
      );
    }
    badge.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 1 }));
    check(
      inspected === 1 && picked === 0,
      "touch opens details once without selecting the row",
    );
    badge.focus();
    badge.dispatchEvent(
      new KeyboardEvent("keydown", { bubbles: true, key: " " }),
    );
    badge.dispatchEvent(
      new KeyboardEvent("keyup", { bubbles: true, key: " " }),
    );
    check(
      inspected === 2 && picked === 0,
      "keyboard click opens details without selecting the row",
    );
    check(
      getComputedStyle(badge).boxShadow === "none",
      "badge remains paint-only on the swipe surface",
    );
    flushSync(() => root.render(<SessionRow cwd="/changed-runtime" />));
    const unavailableBadge = container.querySelector<HTMLElement>(
      ".MuiChip-root",
    )!;
    check(
      unavailableBadge.textContent === "OVH → Hawk" &&
        unavailableBadge.getAttribute("aria-label")?.includes("unavailable") &&
        unavailableBadge.querySelector("svg") !== null,
      "unavailable executor retains the target badge with a visible warning",
    );
    return [
      "project before installed AI",
      "runtime without mirror directories",
      "Codex and Claude selectable after live occupancy projection",
      "compact selected AI icon and label share one row",
      "long AI labels fit mobile and desktop fields",
      "cross-target readiness isolation",
      "late reply ignored",
      "failure without local fallback",
      "selection survives inventory refresh",
      "unavailable OVH requires explicit alternative selection",
      "remote session badge identifies target and isolates touch activation",
    ];
  } finally {
    flushSync(() => root.unmount());
    resetProviderCatalog();
    container.remove();
    globalThis.fetch = originalFetch;
  }
}
