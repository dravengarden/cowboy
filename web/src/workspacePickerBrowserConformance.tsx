import { StrictMode, useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { WorkspacePicker } from "./WorkspacePicker";

export async function runWorkspacePickerBrowserConformance(): Promise<
  string[]
> {
  localStorage.removeItem("cowboy.workspaceHierarchy");
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
  function Harness() {
    const [value, setValue] = useState("other-id");
    return (
      <WorkspacePicker
        entries={entries}
        value={value}
        onChange={(id) => {
          selections.push(id);
          setValue(id);
        }}
      />
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
      .find((element) => element.textContent?.trim() === text);
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
    await closed();
    check(
      selections[0] === "parent-id",
      "Clicking a project with children selects its own opaque identity",
    );
    check(
      !document.querySelector('[role="menu"]'),
      "Parent selection closes the picker",
    );
    click('[role="combobox"]');
    await settle();
    click('[role="menuitem"]', "hawk");
    await settle();
    const browse = document.querySelector<HTMLElement>(
      '[aria-label="Browse subdirectories of hawk/columbus"]',
    );
    check(browse, "A selectable parent has a separate browse action");
    const parentBounds = item("columbus").getBoundingClientRect();
    const browseBounds = browse.getBoundingClientRect();
    check(
      browseBounds.width >= 44 && browseBounds.height >= 44 &&
        browseBounds.left >= parentBounds.right &&
        Math.abs(browseBounds.top - parentBounds.top) < 1,
      `Browse action has its own same-row touch target: ${
        JSON.stringify({ parentBounds, browseBounds })
      }`,
    );
    flushSync(() => browse.click());
    await settle();
    check(
      selections.slice().length === 1,
      "Browsing children must not change the selected project",
    );
    click('[role="menuitem"]', "cowboy");
    await closed();
    check(
      selections[1] === "child-id",
      "Child project selects its own identity",
    );
    click('[role="combobox"]');
    await settle();
    click('[role="menuitem"]', "hawk");
    await settle();
    flushSync(() => {
      const parent = item("columbus");
      parent.focus();
      parent.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
      );
    });
    await settle();
    check(
      selections.length === 2 && item("cowboy"),
      "Right arrow browses without selecting",
    );
    flushSync(() =>
      item("cowboy").dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }),
      )
    );
    await settle();
    flushSync(() => {
      const parent = item("columbus");
      parent.focus();
      parent.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
    });
    await closed();
    check(
      selections[2] === "parent-id",
      "Enter selects the parent after keyboard browsing",
    );
    click('[role="combobox"]');
    await settle();
    click('input[type="checkbox"]');
    await settle();
    click('[role="menuitem"]', "hawk/columbus/cowboy");
    await closed();
    check(selections[3] === "child-id", "Flat mode selects the full path");
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
    return [
      "default hierarchy",
      "parent name selects itself",
      "separate 44px browse action preserves selection",
      "child selects itself",
      "keyboard browse and parent selection",
      "flat full paths",
      "saved preference",
    ];
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    localStorage.removeItem("cowboy.workspaceHierarchy");
  }
}
