import { useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceContext } from "./surface/SurfaceProfile";
import { classifySurface } from "./surface/profile";
import { FolderDefaultsView } from "./FolderDefaultsSheet";
import { DirectorySessionsView } from "./DirectorySessionsSheet";
import type { MachineSummary, SessionMeta } from "./protocol";
import type { SessionFoldersValue } from "./sessionFolders";

const tree: SessionFoldersValue = {
  folders: [
    {
      id: "parent",
      name: "Projects",
      parent: null,
      position: 0,
      project: null,
      defaults: { provider: "claude-code" },
    },
    { id: "root", name: "Suger", parent: "parent", position: 0, project: null },
    { id: "child", name: "Review", parent: "root", position: 0, project: null },
  ],
  placement: { a: "root", b: "root", nested: "child", late: "root" },
};
const machine = {
  id: "hawk",
  display_name: "Hawk",
  local: true,
  connected: true,
  platform: "linux",
  architecture: "x86_64",
  status: "online",
  schedulable: true,
  components: [],
  plugins: [],
  capacity: { max_sessions: 8, draining: false },
  active_sessions: 0,
  workspaces: [
    {
      id: "columbus",
      display_name: "columbus",
      canonical_path: "/work/columbus",
    },
    ...["argus", "books", "cardea", "stormbird"].map((id) => ({
      id,
      display_name: `columbus/${id}`,
      canonical_path: `/work/columbus/projects/${id}`,
    })),
    { id: "suger", display_name: "suger", canonical_path: "/work/suger" },
    { id: "cowboy", display_name: "cowboy", canonical_path: "/work/cowboy" },
  ],
} as MachineSummary;
const session = (id: string): SessionMeta => ({
  id,
  title: ({
    a: "Review billing changes",
    b: "Fix authentication flow",
    nested: "Verify release",
    late: "New session",
  } as Record<string, string>)[id] ?? id,
  provider: "claude-code",
  status: "running",
  cwd: "/work/suger",
});
const tick = () => new Promise((resolve) => setTimeout(resolve, 60));
function check(value: unknown, message: string): void {
  if (!value) throw new Error(message);
}
const button = (text: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find((element) =>
    element.textContent?.trim().startsWith(text)
  );
const click = async (element: HTMLElement | null | undefined) => {
  check(element, "expected visible control");
  flushSync(() => element!.click());
  await tick();
};

export function renderDirectoryUxPreview(
  mode: "defaults" | "projects" | "sessions",
  mobile = false,
) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const profile = classifySurface({
    finePointer: !mobile,
    hover: !mobile,
    tabletWidth: false,
    touchCapable: mobile,
    desktopHost: !mobile,
  });
  const calls: string[] = [];
  const backgroundFailures: number[] = [];
  let gate: Promise<void> | undefined;
  let release: (() => void) | undefined;
  let fail = true;
  let addSession = () => {};
  function Fixture() {
    const [folders, setFolders] = useState(tree);
    const [sessions, setSessions] = useState([
      session("a"),
      session("b"),
      session("nested"),
    ]);
    addSession = () => setSessions((current) => [...current, session("late")]);
    return (
      <SurfaceContext.Provider value={profile}>
        <BrowserProductTheme>
          <CssBaseline />
          {mode === "sessions"
            ? (
              <DirectorySessionsView
                id="root"
                tree={folders}
                sessions={sessions}
                deleting={new Set()}
                onClose={() => {}}
                removeSession={async (id) => {
                  calls.push(id);
                  if (gate) await gate;
                  if (id === "b" && fail) throw new Error("Machine offline");
                  setSessions((current) => current.filter((s) => s.id !== id));
                }}
                onBackgroundFailure={(count) => backgroundFailures.push(count)}
              />
            )
            : (
              <FolderDefaultsView
                id="root"
                tree={folders}
                machines={[machine]}
                catalog={null}
                onClose={() => {}}
                onSetDefault={(id, key, value) =>
                  setFolders((current) => ({
                    ...current,
                    folders: current.folders.map((folder) => {
                      if (folder.id !== id) return folder;
                      const defaults = { ...folder.defaults };
                      if (value === undefined) delete defaults[key];
                      else defaults[key] = value;
                      return { ...folder, defaults };
                    }),
                  }))}
              />
            )}
        </BrowserProductTheme>
      </SurfaceContext.Provider>
    );
  }
  flushSync(() => root.render(<Fixture />));
  return {
    calls,
    backgroundFailures,
    pause: () => {
      gate = new Promise<void>((resolve) => {
        release = resolve;
      });
    },
    resume: () => release?.(),
    addSession: () => flushSync(addSession),
    allowRetry: () => {
      fail = false;
    },
    close: () => {
      flushSync(() => root.unmount());
      container.remove();
    },
  };
}

export async function runDirectoryUxBrowserChecks(): Promise<void> {
  for (const mobile of [false, true]) {
    const defaults = renderDirectoryUxPreview("projects", mobile);
    try {
      await tick();
      await click(document.querySelector('[data-default-field="project"]'));
      check(
        document.querySelectorAll('[role="dialog"]').length === 1,
        "project choice must use one sheet",
      );
      check(
        document.querySelector('[data-default-value="argus"]'),
        "registered child projects are visible in hierarchy",
      );
      await click(document.querySelector('[aria-label="Collapse columbus"]'));
      check(
        !document.querySelector('[data-default-value="argus"]'),
        "collapsing parent hides its children",
      );
      const input = document.querySelector<HTMLInputElement>(
        'input[aria-label="Search defaults"]',
      )!;
      flushSync(() => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!
          .set!.call(input, "argus");
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await tick();
      await click(document.querySelector('[data-default-value="argus"]'));
      check(
        document.querySelector('[data-default-field="project"]')?.textContent
          ?.includes("argus"),
        "project selection returns to overview and saves",
      );
      check(
        document.body.textContent?.includes("Inherited from Projects"),
        "overview preserves inheritance provenance",
      );
      await click(document.querySelector('[data-default-field="project"]'));
      await click(document.querySelector('[data-default-mode="clear"]'));
      check(
        document.querySelector('[data-default-field="project"]')?.textContent
          ?.includes("Choose when creating"),
        "explicit clear is distinct from inheritance",
      );
    } finally {
      defaults.close();
    }
    const batch = renderDirectoryUxPreview("sessions", mobile);
    try {
      await tick();
      check(
        document.querySelectorAll('[role="menuitemcheckbox"]').length === 2,
        "bulk selection initially excludes subdirectories",
      );
      await click(button("Select all in directory (2)"));
      batch.addSession();
      await tick();
      check(
        button("Delete (2)"),
        "new sessions never join an existing selection",
      );
      await click(button("Delete (2)"));
      check(
        batch.calls.length === 0,
        "opening confirmation sends no destructive request",
      );
      await click(button("Cancel"));
      check(
        batch.calls.length === 0,
        "cancelling sends no destructive request",
      );
      await click(button("Delete (2)"));
      await click(button("Delete 2 sessions"));
      await tick();
      check(
        batch.calls.length === 2 && batch.calls.includes("a") &&
          batch.calls.includes("b"),
        "only confirmed identities are deleted",
      );
      check(
        document.body.textContent?.includes("Machine offline"),
        "failed item retains actionable error",
      );
      check(
        button("Delete (1)"),
        "only retained failure stays selected after success disappears",
      );
      batch.allowRetry();
      await click(button("Delete (1)"));
      await click(button("Delete 1 sessions"));
      check(
        batch.calls.join(",") === "b,a,b",
        "retry never resends a successful deletion",
      );
    } finally {
      batch.close();
    }
    const detached = renderDirectoryUxPreview("sessions", mobile);
    await tick();
    detached.pause();
    await click(button("Select all in directory (2)"));
    await click(button("Delete (2)"));
    await click(button("Delete 2 sessions"));
    detached.close();
    detached.resume();
    await tick();
    check(
      detached.backgroundFailures.join(",") === "1",
      "closing a running batch still reports its failures globally",
    );
  }
}
