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
  const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 40));
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
      "Entering folders must not select a workspace",
    );
    click('[role="menuitem"]', "Select this directory");
    await settle();
    check(
      selections[0] === "parent-id",
      "Selectable parent keeps its opaque identity",
    );
    click('[role="combobox"]');
    await settle();
    click('input[type="checkbox"]');
    await settle();
    click('[role="menuitem"]', "hawk/columbus/cowboy");
    await settle();
    check(selections[1] === "child-id", "Flat mode selects the full path");
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
      "drilldown without selection",
      "selectable parent",
      "flat full paths",
      "saved preference",
    ];
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    localStorage.removeItem("cowboy.workspaceHierarchy");
  }
}
