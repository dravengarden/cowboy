import { StrictMode, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { ThemeProvider } from "@mui/material";
import { useThemeMode } from "./theme";
import { WorkspacePicker } from "./WorkspacePicker";

export async function runWorkspacePickerBrowserConformance(): Promise<
  string[]
> {
  localStorage.removeItem("cowboy.workspaceHierarchy");
  localStorage.removeItem("cowboy.projectHierarchy");
  const container = document.createElement("div");
  container.style.width = "360px";
  container.style.display = "flex";
  container.style.flexDirection = "column";
  document.body.append(container);
  const root = createRoot(container);
  const selections: string[] = [];
  const entries = [
    { value: "parent-id", label: "hawk/columbus", help: "Parent root" },
    { value: "child-id", label: "hawk/columbus/cowboy", help: "Project root" },
    { value: "other-id", label: "falcon/suger", help: "Other root" },
  ];
  function Harness({ label = "Working directory" }: { label?: string }) {
    const [value, setValue] = useState("other-id");
    const { theme } = useThemeMode();
    return (
      <ThemeProvider theme={theme}>
        <WorkspacePicker
          label={label}
          entries={entries}
          value={value}
          onChange={(id) => {
            selections.push(id);
            setValue(id);
          }}
        />
      </ThemeProvider>
    );
  }
  // Popover's automatic Grow duration depends on its measured content height.
  const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 400));
  const closed = async (): Promise<void> => {
    for (let attempt = 0; attempt < 80; attempt++) {
      if (!document.querySelector('[role="menu"]')) return;
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    throw new Error("Selection did not close the picker");
  };
  function item(text: string): HTMLElement {
    const target = [
      ...document.querySelectorAll<HTMLElement>('[role="menuitem"]'),
    ]
      .find((element) =>
        element.textContent?.trim() === text ||
        element.querySelector(".MuiTypography-root")?.textContent?.trim() ===
          text
      );
    if (!target) throw new Error(`Missing menu item: ${text}`);
    return target;
  }
  function click(selector: string, text?: string): void {
    const target = [...document.querySelectorAll<HTMLElement>(selector)]
      .find((element) =>
        text === undefined || element.textContent?.includes(text)
      );
    if (!target) throw new Error(`Missing ${selector}: ${text}`);
    flushSync(() => target.click());
  }
  function check(value: unknown, message: string): asserts value {
    if (!value) throw new Error(message);
  }
  try {
    flushSync(() =>
      root.render(
        <StrictMode>
          <Harness />
        </StrictMode>,
      )
    );
    click('[role="combobox"]');
    await settle();
    check(
      document.querySelector<HTMLInputElement>('input[type="checkbox"]')
        ?.checked,
      "Grouping defaults on",
    );
    click('[role="menuitem"]', "hawk");
    await settle();
    click('[role="menuitem"]', "columbus");
    await settle();
    check(
      selections.length === 0,
      "Browsing registered parents preserves the selected project",
    );
    check(
      document.querySelector('[role="menu"]') && item("cowboy"),
      "Parent expands and leaves picker open",
    );
    const parent = document.querySelector<HTMLElement>(
      '[data-current-directory="true"]',
    );
    check(
      parent?.textContent?.includes("Use this directory"),
      "Current parent has an explicit caption",
    );
    check(
      parseFloat(getComputedStyle(parent!).borderTopWidth) > 0,
      "Current parent has a distinct outlined surface",
    );
    check(
      parent!.getBoundingClientRect().bottom <
        item("cowboy").getBoundingClientRect().top,
      "Current parent is separated from children",
    );
    flushSync(() => parent!.click());
    await closed();
    check(
      selections.at(-1) === "parent-id",
      "Explicit parent selection closes the picker",
    );
    click('[role="combobox"]');
    await settle();
    click('[role="menuitem"]', "hawk");
    await settle();
    click('[role="menuitem"]', "columbus");
    await settle();
    click('[role="menuitem"]', "cowboy");
    await closed();
    check(
      selections.at(-1) === "child-id",
      "Child selection closes the picker",
    );
    click('[role="combobox"]');
    await settle();
    click('[role="menuitem"]', "hawk");
    await settle();
    const count = selections.length;
    flushSync(() => {
      const row = item("columbus");
      row.focus();
      row.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    });
    await settle();
    check(
      document.querySelector('[role="menu"]') && item("cowboy") &&
        selections.length === count,
      "Keyboard Enter expands without changing selection",
    );
    flushSync(() =>
      item("cowboy").dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }),
      )
    );
    await settle();
    flushSync(() =>
      item("columbus").dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
      )
    );
    await settle();
    check(
      item("cowboy") && selections.length === count,
      "Right arrow follows parent expansion semantics",
    );
    check(selections.length === count, "Parent reentry preserves selection");
    click('[role="menuitem"]', "cowboy");
    await closed();
    click('[role="combobox"]');
    await settle();
    click('input[type="checkbox"]');
    await settle();
    click('[role="menuitem"]', "hawk/columbus/cowboy");
    await closed();
    check(selections.at(-1) === "child-id", "Flat mode selects the full path");
    click('[role="combobox"]');
    await settle();
    check(
      !document.querySelector<HTMLInputElement>('input[type="checkbox"]')
        ?.checked,
      "Preference survives reopening",
    );
    check(
      localStorage.getItem("cowboy.workspaceHierarchy") === "false",
      "Preference is saved",
    );
    // An older local directory picker must not flatten the unified project
    // picker. Both local and remote projects use the same component and tree.
    click('[role="menuitem"]', "hawk/columbus/cowboy");
    await closed();
    flushSync(() => root.render(<Harness key="project" label="Project" />));
    click('[role="combobox"]');
    await settle();
    check(
      document.querySelector<HTMLInputElement>('input[type="checkbox"]')
        ?.checked,
      "Projects default to hierarchy despite the old flat directory preference",
    );
    check(
      document.querySelectorAll('[role="menuitem"]').length === 2 &&
        item("hawk") && item("falcon"),
      "Project root shows Machines instead of all projects",
    );
    const search = document.querySelector<HTMLInputElement>(
      'input:not([type="checkbox"]):not([role="combobox"])',
    )!;
    flushSync(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!
        .set!.call(search, "Project root");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await settle();
    check(
      document.querySelectorAll('[role="menuitem"]').length === 1 &&
        item("hawk/columbus/cowboy"),
      "Search finds nested projects by source path without browsing each Machine",
    );
    click('[role="menuitem"]', "hawk/columbus/cowboy");
    await closed();
    check(
      selections.at(-1) === "child-id",
      "Search preserves project identity",
    );
    return [
      "default hierarchy",
      "parent browsing preserves selection",
      "current parent has a distinct outlined surface",
      "explicit parent and child choices close the picker",
      "keyboard browsing preserves selection",
      "flat full paths",
      "saved preference",
      "project hierarchy independent of old directory preference",
      "cross-Machine project search by source path",
    ];
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    localStorage.removeItem("cowboy.workspaceHierarchy");
    localStorage.removeItem("cowboy.projectHierarchy");
  }
}
