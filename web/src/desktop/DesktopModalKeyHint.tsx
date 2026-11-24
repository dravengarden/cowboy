import { useEffect, useState } from "react";
import { Box } from "@mui/material";
import { Kbd } from "../Kbd";
import { useVimSetting } from "../vimSetting";
import { isModalTextField } from "./commands/modalNavigation";
import type { DesktopShortcutGroup } from "./DesktopShortcutBar";
import { isInputVimNormal, subscribeInputVimMode } from "./vim/inputVim";

export type DesktopModalMode = "insert" | "normal";

export interface DesktopModalShortcutOptions {
  readonly tabs?: number;
  readonly hasTextField?: boolean;
  readonly vim?: boolean;
  readonly includeLeader?: boolean;
}

/** Insert while a writable text field of the page owns focus (not in Vim Normal), else Normal. */
export function useDesktopModalMode(): DesktopModalMode {
  const read = (): DesktopModalMode => {
    const active = globalThis.document?.activeElement ?? null;
    return isModalTextField(active) && !isInputVimNormal(active)
      ? "insert"
      : "normal";
  };
  const [mode, setMode] = useState<DesktopModalMode>(read);
  useEffect(() => {
    const update = (): void => setMode(read());
    globalThis.addEventListener("focusin", update);
    globalThis.addEventListener("focusout", update);
    const unsubscribeVim = subscribeInputVimMode(update);
    return () => {
      globalThis.removeEventListener("focusin", update);
      globalThis.removeEventListener("focusout", update);
      unsubscribeVim();
    };
  }, []);
  return mode;
}

/** The modal grammar's keys for the current mode (FOCUS.md "Modals"), as
 *  footer groups. Accepts either a tab count or an options object. */
export function desktopModalShortcutGroups(
  mode: DesktopModalMode,
  tabsOrOptions: number | DesktopModalShortcutOptions = 0,
): DesktopShortcutGroup[] {
  if (mode === "insert") {
    return [{ slots: [{ shortcut: "Esc", label: "Normal" }] }];
  }
  const options: DesktopModalShortcutOptions = typeof tabsOrOptions === "number"
    ? { tabs: tabsOrOptions }
    : tabsOrOptions;
  const tabs = options.tabs ?? 0;
  const hasTextField = options.hasTextField ?? true;
  const vim = options.vim ?? false;
  const includeLeader = options.includeLeader ?? false;
  return [
    ...(tabs > 1
      ? [{
        label: "Tabs",
        slots: [
          { shortcut: `1…${String(Math.min(tabs, 9))}`, label: "Pick" },
          { shortcut: vim ? "[/]" : "H/L", label: "Step" },
        ],
      }]
      : []),
    {
      label: "Move",
      slots: [
        { shortcut: "J/K", label: "Row" },
        ...(tabs > 1 ? [] : [{ shortcut: "H/L", label: "Across" }]),
      ],
    },
    {
      slots: [
        ...(hasTextField ? [{ shortcut: "I", label: "Edit" }] : []),
        { shortcut: "Enter", label: "Activate" },
        ...(includeLeader ? [{ shortcut: "␣", label: "Labels" }] : []),
        { shortcut: "Esc", label: "Close" },
      ],
    },
  ];
}

/** The same keys as one quiet line, for dialogs without a shortcut bar. */
export function DesktopModalKeyHint(
  { mode, tabs = 0, hasTextField = true }: {
    mode: DesktopModalMode;
    tabs?: number;
    hasTextField?: boolean;
  },
): React.JSX.Element {
  const vim = useVimSetting();
  // One line, so switching mode never shifts the form: Enter is implied.
  const slots = desktopModalShortcutGroups(mode, { tabs, hasTextField, vim })
    .flatMap((group) => group.slots)
    .filter((slot) => slot.shortcut !== "Enter");
  return (
    <>
      {slots.map((slot, index) => (
        <Box
          key={`${slot.shortcut}-${slot.label ?? ""}`}
          component="span"
          sx={{ display: "inline-flex", alignItems: "center", mr: index === slots.length - 1 ? 0 : 1 }}
        >
          <Kbd keys={slot.shortcut} variant="context" />
          <Box component="span" sx={{ ml: 0.5 }}>{slot.label?.toLowerCase()}</Box>
        </Box>
      ))}
    </>
  );
}
