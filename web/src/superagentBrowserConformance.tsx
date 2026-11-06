/** Real participation controls, isolated HTTP; no live sessions or models. */
import type { ReactNode } from "react";
import { SessionSuperagentSettings } from "./SuperagentSettings";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import type { SuperagentConfig } from "./superagentPreferences";

async function until<T>(read: () => T | undefined | false, label: string): Promise<T> {
  for (let n = 0; n < 100; n++) {
    const value = read();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 30));
  }
  throw new Error(label);
}

function field(label: string): HTMLElement | undefined {
  const node = [...document.querySelectorAll<HTMLLabelElement>("label")].find(
    (node) => node.textContent === label,
  );
  return node ? (document.getElementById(node.htmlFor) ?? undefined) : undefined;
}

async function choose(label: string, value: string): Promise<void> {
  const input = await until(() => {
    const node = field(label);
    return node?.getAttribute("aria-disabled") !== "true" ? node : undefined;
  }, `${label} unavailable`);
  input.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  (
    await until(
      () =>
        [...document.querySelectorAll<HTMLElement>("[role=option]")].find(
          (node) => node.textContent?.trim() === value,
        ),
      `${value} option unavailable`,
    )
  ).click();
}

export function superagentBrowserFixture() {
  let config: SuperagentConfig = {
    sources: [],
    projects: {
      cowboy: {
        profile: "project.cowboy",
        coordinator: "coordinator",
        mode: "observe",
        preset: "astra-max",
        max_followups: 2,
        learning: false,
      },
    },
    sessions: {},
  };
  let revision = 1;
  let loseNextResponse = false;
  const writes: {
    operation: string;
    expected_revision: number;
    changes: { configuration: SuperagentConfig };
  }[] = [];
  const receipts = new Map<string, unknown>();
  const preferences = () => ({
    scope: "superagent",
    revision,
    values: { configuration: config },
  });
  return {
    fetch(url: string, init?: RequestInit): Promise<Response> | undefined {
      if (url === "/api/preferences/superagent") {
        return Promise.resolve(
          Response.json({
            dataset: {
              dataset_id: "calls-fixture",
              user_id: "call-account-fixture",
            },
            preferences: preferences(),
          }),
        );
      }
      if (url === "/api/preferences" && init?.method === "PUT") {
        const input = JSON.parse(String(init.body));
        if (input.scope !== "superagent") return undefined;
        writes.push(input);
        const existing = receipts.get(input.operation);
        if (existing) return Promise.resolve(Response.json(existing));
        if (input.expected_revision !== revision) {
          return Promise.resolve(
            Response.json(
              {
                error: { code: "revision_conflict" },
                current: preferences(),
              },
              { status: 409 },
            ),
          );
        }
        config = input.changes.configuration;
        revision++;
        const result = { status: "committed", preferences: preferences() };
        receipts.set(input.operation, result);
        if (loseNextResponse) {
          loseNextResponse = false;
          return Promise.reject(new TypeError("Fixture lost the response after commit"));
        }
        return Promise.resolve(Response.json(result));
      }
      if (url === "/api/superagent/activity") {
        return Promise.resolve(Response.json({ activity: [] }));
      }
      return undefined;
    },
    async run(render: (node: ReactNode) => unknown): Promise<string[]> {
      render(<SessionSuperagentSettings sessionId="supervised-worker" />);
      await until(
        () => field("Project")?.textContent?.includes("Not enrolled"),
        "default must not enroll",
      );
      await choose("Project", "cowboy");
      await until(
        () => config.sessions["supervised-worker"]?.mode === "off",
        "enrollment must start Off",
      );
      const scope = await until(
        () => field("Task scope") as HTMLTextAreaElement | undefined,
        "task scope missing",
      );
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(
        scope,
        "Finish the existing Cowboy task",
      );
      scope.dispatchEvent(new Event("input", { bubbles: true }));
      await choose("Participation", "Coordinate");
      await until(
        () => config.sessions["supervised-worker"]?.mode === "coordinate",
        "coordinate save missing",
      );
      if (config.sessions["supervised-worker"]?.mandate !== "Finish the existing Cowboy task") {
        throw new Error("mode change lost the edited task scope");
      }
      config.sessions["supervised-worker"]!.mandate = "Updated scope from another device";
      revision++;
      window.dispatchEvent(
        new CustomEvent(PREFERENCES_CHANGED, { detail: { superagent: revision } }),
      );
      await until(
        () =>
          (field("Task scope") as HTMLTextAreaElement | undefined)?.value ===
          "Updated scope from another device",
        "saved draft must not mask a newer task scope",
      );
      render(<></>);
      render(<SessionSuperagentSettings sessionId="supervised-worker" />);
      await until(
        () => field("Participation")?.textContent?.includes("Coordinate"),
        "remount lost participation",
      );
      await choose("Participation", "Inherit project (observe)");
      await until(
        () => config.sessions["supervised-worker"]?.mode === null,
        "reset must store inheritance",
      );
      loseNextResponse = true;
      await choose("Participation", "Off");
      const retry = await until(
        () =>
          [...document.querySelectorAll<HTMLButtonElement>("button")].find(
            (node) => node.textContent === "Retry",
          ),
        "unknown write must expose retry",
      );
      const original = writes.at(-1)!;
      retry.click();
      await until(() => writes.at(-1) !== original, "retry not submitted");
      if (
        writes.at(-1)?.operation !== original.operation ||
        writes.at(-1)?.expected_revision !== original.expected_revision
      ) {
        throw new Error("uncertain write retried with a new identity");
      }
      await until(
        () => field("Participation")?.textContent?.includes("Off"),
        "receipt did not recover committed value",
      );
      revision++;
      config = {
        ...config,
        projects: {
          cowboy: { ...config.projects.cowboy!, mode: "coordinate" },
        },
      };
      await choose("Participation", "Observe");
      await until(
        () => document.body.textContent?.includes("Settings changed on another device"),
        "stale edit must surface conflict",
      );
      await until(
        () => field("Participation")?.textContent?.includes("Off"),
        "conflict must reload current preferences",
      );
      if (config.sessions["supervised-worker"]?.mode !== "off") {
        throw new Error("stale edit overwrote server state");
      }
      return [
        "superagent-explicit-enrollment-inheritance-persistence",
        "superagent-unknown-write-same-operation-and-conflict",
      ];
    },
  };
}
