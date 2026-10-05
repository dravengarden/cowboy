import { useEffect, useState } from "react";
import { Box } from "@mui/material";
import { Kbd } from "../Kbd";
import { isModalTextField } from "./commands/modalNavigation";
import type { DesktopShortcutGroup } from "./DesktopShortcutBar";

export type DesktopModalMode = "insert" | "normal";

/** Insert while a text field of the page owns focus, else Normal. */
export function useDesktopModalMode(): DesktopModalMode {
  const read = (): DesktopModalMode =>
    isModalTextField(globalThis.document?.activeElement ?? null)
      ? "insert"
      : "normal";
  const [mode, setMode] = useState<DesktopModalMode>(read);
  useEffect(() => {
    const update = (): void => setMode(read());
    globalThis.addEventListener("focusin", update);
    globalThis.addEventListener("focusout", update);
    return () => {
      globalThis.removeEventListener("focusin", update);
      globalThis.removeEventListener("focusout", update);
    };
  }, []);
  return mode;
}

/** The modal grammar's keys for the current mode (FOCUS.md "Modals"), as
 *  footer groups. `tabs` is the number of top tabs, 0 when there are none. */
export function desktopModalShortcutGroups(
  mode: DesktopModalMode,
  tabs = 0,
): DesktopShortcutGroup[] {
  if (mode === "insert") {
    return [{ slots: [{ shortcut: "Esc", label: "Normal" }] }];
  }
  return [
    ...(tabs > 1
      ? [{
        label: "Tabs",
        slots: [
          { shortcut: `1…${String(Math.min(tabs, 9))}`, label: "Pick" },
          { shortcut: "H/L", label: "Step" },
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
        { shortcut: "I", label: "Edit" },
        { shortcut: "Enter", label: "Activate" },
        { shortcut: "Esc", label: "Close" },
      ],
    },
  ];
}

/** The same keys as one quiet line, for dialogs without a shortcut bar. */
export function DesktopModalKeyHint(
  { mode, tabs = 0 }: { mode: DesktopModalMode; tabs?: number },
): React.JSX.Element {
  const slots = desktopModalShortcutGroups(mode, tabs).flatMap((group) =>
    group.slots
  );
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
