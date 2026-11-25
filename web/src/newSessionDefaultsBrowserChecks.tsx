import { useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceContext } from "./surface/SurfaceProfile";
import { classifySurface } from "./surface/profile";
import { SessionPresetPicker } from "./SessionPresetPicker";
import {
  newSessionPreset,
  resolveCreateSessionPreset,
  type SessionPreset,
} from "./newSessionPreset";
import {
  SessionSubscription,
  useSessionSubscription,
} from "./SessionSubscription";
import { WorkspacePicker } from "./WorkspacePicker";
import { sessionDirectoryChoices } from "./sessionDirectoryChoices";

/** Actual creation fields: defaults must be visible and selected in their menus. */
export async function runNewSessionDefaultsBrowserChecks(): Promise<void> {
  const originalFetch = globalThis.fetch;
  const presets: SessionPreset[] = [
    {
      id: "other",
      name: "Other",
      detail: "Alternative",
      is_default: false,
      values: {},
    },
    {
      id: "opus",
      name: "Opus",
      detail: "Recommended",
      is_default: true,
      values: {},
    },
  ];
  const check = (ok: unknown, message: string): void => { if (!ok) throw new Error(message); };
  const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 50));
  globalThis.fetch = (() =>
    Promise.resolve(Response.json({
      registry: {
        accounts: [{
          id: "work",
          provider_id: "claude-code",
          label: "Work",
          enabled: true,
        }, {
          id: "lynx",
          provider_id: "claude-code",
          label: "Lynx",
          enabled: true,
        }, {
          id: "off",
          provider_id: "claude-code",
          label: "Off",
          enabled: false,
        }],
        defaults: { "claude-code": "work" },
      },
      session_admission: "enabled",
      session_providers: ["claude-code"],
    }))) as typeof fetch;
  try {
    for (const mobile of [false, true]) {
      const container = document.createElement("div");
      document.body.append(container);
      const root = createRoot(container);
      let submitted = "";
      function Fields() {
        const [override, setOverride] = useState<string>();
        const value = newSessionPreset(
          "claude-code",
          presets,
          undefined,
          override,
        );
        const subscription = useSessionSubscription(true, "claude-code");
        return (
          <>
            <SessionPresetPicker
              provider="claude-code"
              presets={presets}
              value={value}
              onChange={setOverride}
              helperText="Applied before the first prompt"
            />
            <WorkspacePicker
              label="Directory (optional)"
              entries={sessionDirectoryChoices(
                { folders: [], placement: {} },
                true,
              )}
              value=""
              onChange={() => {}}
            />
            <SessionSubscription
              state={subscription}
              disabled={false}
              desktop={!mobile}
            />
            <button
              onClick={() => {
                submitted = JSON.stringify({
                  preset: value,
                  account: subscription.value,
                });
              }}
            >
              Submit defaults
            </button>
          </>
        );
      }
      const profile = classifySurface({
        finePointer: !mobile,
        hover: !mobile,
        tabletWidth: false,
        touchCapable: mobile,
        desktopHost: !mobile,
      });
      try {
        flushSync(() =>
          root.render(
            <SurfaceContext.Provider value={profile}>
              <BrowserProductTheme>
                <Fields />
              </BrowserProductTheme>
            </SurfaceContext.Provider>,
          )
        );
        for (
          let i = 0;
          i < 40 &&
          !container.querySelector("[data-session-subscription]")?.textContent
            ?.includes("Work · Default");
          i++
        ) await tick();
        const field = () =>
          container.querySelector<HTMLElement>(
            '.MuiSelect-select[role="combobox"]',
          )!;
        check(
          field().textContent?.includes("Opus · Recommended"),
          "default preset is displayed immediately",
        );
        check(
          container.querySelector<HTMLInputElement>('input[role="combobox"]')
            ?.value === "Global",
          "Global is visibly selected",
        );
        check(
          container.querySelector("[data-session-subscription]")?.textContent
            ?.includes("Work · Default"),
          "default subscription is visibly selected",
        );
        flushSync(() => container.querySelector("button")!.click());
        check(
          submitted ===
            JSON.stringify({ preset: "claude-code/opus", account: "work" }),
          "shown defaults submit without reselection",
        );
        flushSync(() =>
          field().dispatchEvent(
            new MouseEvent("mousedown", { button: 0, bubbles: true }),
          )
        );
        await tick();
        const options = [
          ...document.querySelectorAll<HTMLElement>('[role="option"]'),
        ];
        check(
          options.find((option) =>
            option.getAttribute("aria-selected") === "true"
          )?.textContent?.includes("Opus"),
          "default menu item is selected",
        );
        flushSync(() =>
          options.find((option) => option.textContent === "Agent default")!
            .click()
        );
        await tick();
        check(
          field().textContent === "Agent default",
          "explicit Agent default is visible instead of blank",
        );
      } finally {
        flushSync(() => root.unmount());
        container.remove();
      }
    }
    // A directory's latest session account precedes the registry default;
    // an unusable one falls back instead of blocking creation.
    for (
      const [preferred, expected] of [["lynx", "Lynx"], [
        "off",
        "Work · Default",
      ]]
    ) {
      const container = document.createElement("div");
      document.body.append(container);
      const root = createRoot(container);
      let state: ReturnType<typeof useSessionSubscription> | undefined;
      function Hinted() {
        const subscription = useSessionSubscription(
          true,
          "claude-code",
          preferred,
        );
        state = subscription;
        return (
          <SessionSubscription state={subscription} disabled={false} desktop />
        );
      }
      try {
        flushSync(() =>
          root.render(
            <BrowserProductTheme>
              <Hinted />
            </BrowserProductTheme>,
          )
        );
        for (let i = 0; i < 40 && !state?.data; i++) await tick();
        await tick();
        check(
          container.querySelector("[data-session-subscription]")?.textContent
            ?.includes(expected!),
          "preferred " + preferred + " shows " + expected,
        );
        check(
          state?.hinted === (preferred === "lynx"),
          "preferred " + preferred + " reports its source",
        );
      } finally {
        flushSync(() => root.unmount());
        container.remove();
      }
    }
    checkCrossProviderPresetSwitch(presets, check);
  } finally {
    globalThis.fetch = originalFetch;
  }
}

function checkCrossProviderPresetSwitch(
  presets: readonly SessionPreset[],
  check: (ok: unknown, message: string) => void,
): void {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const agyPresets: SessionPreset[] = [
    {
      id: "gemini-neon-high",
      name: "Gemini 4 Neon · High",
      detail: "Strongest",
      is_default: true,
      values: {},
    },
  ];
  const claudePresets: SessionPreset[] = [
    {
      id: "sonnet-xhigh",
      name: "Sonnet · XHigh",
      detail: "",
      is_default: false,
      values: {},
    },
    ...presets,
  ];
  const folderDefaults = {
    provider: "claude-code",
    preset: "claude-code/sonnet-xhigh",
  };
  let submitted = "";
  function SwitchAgent() {
    const [provider, setProvider] = useState("claude-code");
    const [chosen, setChosen] = useState(false);
    const [override, setOverride] = useState<string>();
    const activePresets = provider === "antigravity" ? agyPresets : claudePresets;
    const state = resolveCreateSessionPreset(
      provider,
      activePresets,
      folderDefaults,
      chosen,
      override,
    );
    return (
      <>
        <button
          data-switch-agy
          onClick={() => {
            setProvider("antigravity");
            setChosen(true);
            setOverride(undefined);
          }}
        >
          Switch to Antigravity
        </button>
        <button
          data-switch-claude
          onClick={() => {
            setProvider("claude-code");
            setChosen(true);
            setOverride(undefined);
          }}
        >
          Switch to Claude
        </button>
        <SessionPresetPicker
          provider={provider}
          presets={activePresets}
          value={state.selected}
          onChange={setOverride}
          helperText={state.override === undefined && state.inherited
            ? "Inherited from Suger"
            : "Applied before the first prompt"}
        />
        <button data-submit onClick={() => (submitted = state.selected)}>
          Submit
        </button>
      </>
    );
  }
  try {
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <SwitchAgent />
        </BrowserProductTheme>,
      )
    );
    const field = () =>
      container.querySelector<HTMLElement>('.MuiSelect-select[role="combobox"]')!;
    check(
      field().textContent?.includes("Sonnet · XHigh"),
      "directory preset selected initially for Claude",
    );
    check(
      container.textContent?.includes("Inherited from Suger"),
      "inherited helper shown for matching provider",
    );
    flushSync(() =>
      container.querySelector<HTMLButtonElement>("[data-switch-agy]")!.click()
    );
    check(
      field().textContent?.includes("Gemini 4 Neon · High · Recommended"),
      "switching to Antigravity selects Antigravity recommended preset",
    );
    check(
      !container.textContent?.includes("Unavailable or incompatible"),
      "switching to Antigravity does not show incompatible warning",
    );
    check(
      !container.textContent?.includes("Inherited from Suger"),
      "switched agent does not claim preset was inherited from Suger",
    );
    flushSync(() =>
      container.querySelector<HTMLButtonElement>("[data-submit]")!.click()
    );
    check(
      submitted === "antigravity/gemini-neon-high",
      "submits Antigravity recommended preset",
    );
    flushSync(() =>
      container.querySelector<HTMLButtonElement>("[data-switch-claude]")!.click()
    );
    check(
      field().textContent?.includes("Sonnet · XHigh"),
      "switching back to Claude restores Suger inherited preset",
    );
  } finally {
    flushSync(() => root.unmount());
    container.remove();
  }
}
