import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { Box } from "@mui/material";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import { isTextEditingTarget } from "./commands/shortcut";

const HINT_KEYS = "asdfghjkl";
const TARGET_SELECTOR = [
  "a[href]",
  "button:not(:disabled)",
  "[role='button']:not([aria-disabled='true'])",
  "[role='tab']:not([aria-disabled='true'])",
  "[role='menuitem']:not([aria-disabled='true'])",
  "input:not(:disabled)",
  "select:not(:disabled)",
  "textarea:not(:disabled)",
].join(",");

interface HintTarget {
  label: string;
  element: HTMLElement;
  x: number;
  y: number;
}

function labelFor(index: number, count: number): string {
  if (count <= HINT_KEYS.length) return HINT_KEYS[index] ?? "";
  const width = Math.ceil(Math.log(count) / Math.log(HINT_KEYS.length));
  let value = index;
  let label = "";
  for (let digit = 0; digit < width; digit += 1) {
    label = (HINT_KEYS[value % HINT_KEYS.length] ?? "") + label;
    value = Math.floor(value / HINT_KEYS.length);
  }
  return label;
}

function visibleTargets(): HintTarget[] {
  const elements = [...document.querySelectorAll<HTMLElement>(TARGET_SELECTOR)]
    .filter((element) => {
      if (element.closest("[aria-hidden='true'], [inert]")) return false;
      const rect = element.getBoundingClientRect();
      const style = globalThis.getComputedStyle(element);
      return rect.width > 0 && rect.height > 0 && rect.bottom > 0 && rect.right > 0 &&
        rect.top < globalThis.innerHeight && rect.left < globalThis.innerWidth &&
        style.visibility !== "hidden" && style.display !== "none" && Number(style.opacity) > 0.05;
    });
  return elements.map((element, index) => {
    const rect = element.getBoundingClientRect();
    return {
      label: labelFor(index, elements.length),
      element,
      x: Math.max(4, Math.min(globalThis.innerWidth - 34, rect.left + Math.min(14, rect.width / 2))),
      y: Math.max(4, Math.min(globalThis.innerHeight - 24, rect.top + Math.min(10, rect.height / 2))),
    };
  });
}

export function DesktopHintOverlay(): React.JSX.Element | null {
  const workspace = useDesktopWorkspace();
  const [targets, setTargets] = useState<HintTarget[]>([]);
  const [typed, setTyped] = useState("");
  const active = workspace.mode === "hint";

  useEffect(() => {
    const openHints = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.repeat || event.metaKey || event.ctrlKey || event.altKey ||
        event.shiftKey || event.key.toLowerCase() !== "f" || isTextEditingTarget(event.target)) return;
      event.preventDefault();
      event.stopPropagation();
      setTyped("");
      setTargets(visibleTargets());
      workspace.setMode("hint");
    };
    globalThis.addEventListener("keydown", openHints, true);
    return () => globalThis.removeEventListener("keydown", openHints, true);
  }, [workspace]);

  useEffect(() => {
    if (!active) return undefined;
    const refresh = (): void => setTargets(visibleTargets());
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        workspace.setMode("normal");
        return;
      }
      if (event.key === "Backspace") {
        event.preventDefault();
        setTyped((value) => value.slice(0, -1));
        return;
      }
      const key = event.key.toLowerCase();
      if (!HINT_KEYS.includes(key)) return;
      event.preventDefault();
      event.stopPropagation();
      const next = typed + key;
      const matches = targets.filter((target) => target.label.startsWith(next));
      const exact = matches.find((target) => target.label === next);
      if (exact) {
        workspace.setMode("normal");
        exact.element.focus({ preventScroll: true });
        exact.element.click();
      } else if (matches.length > 0) {
        setTyped(next);
      } else {
        setTyped("");
      }
    };
    globalThis.addEventListener("keydown", onKeyDown, true);
    globalThis.addEventListener("resize", refresh);
    globalThis.addEventListener("scroll", refresh, true);
    return () => {
      globalThis.removeEventListener("keydown", onKeyDown, true);
      globalThis.removeEventListener("resize", refresh);
      globalThis.removeEventListener("scroll", refresh, true);
    };
  }, [active, targets, typed, workspace]);

  const visible = useMemo(
    () => targets.filter((target) => target.label.startsWith(typed)),
    [targets, typed],
  );
  if (!active) return null;
  return createPortal(
    <Box aria-label="Keyboard target hints" sx={{ position: "fixed", inset: 0, zIndex: 20000, pointerEvents: "none", userSelect: "none" }}>
      {visible.map((target) => (
        <Box
          key={`${target.label}-${target.x}-${target.y}`}
          sx={{
            position: "fixed",
            left: target.x,
            top: target.y,
            minWidth: 24,
            height: 20,
            px: 0.5,
            display: "grid",
            placeItems: "center",
            borderRadius: 0.75,
            border: "1px solid rgba(55,34,0,.55)",
            bgcolor: "#ffd75f",
            color: "#241800",
            boxShadow: "0 2px 8px rgba(0,0,0,.28)",
            fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
            fontSize: 11,
            fontWeight: 900,
            lineHeight: 1,
            textTransform: "uppercase",
          }}
        >
          <span>
            <span style={{ opacity: 0.38 }}>{target.label.slice(0, typed.length)}</span>
            {target.label.slice(typed.length)}
          </span>
        </Box>
      ))}
    </Box>,
    document.body,
  );
}
