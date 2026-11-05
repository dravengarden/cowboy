// The Run configuration workbench keymap (FOCUS.md "Run configuration").
//
// The panel is split into panes (Agent, Context, Tools). Inside a pane the
// fields run top to bottom: a `[data-config-row]` (a Provider option or the
// recommended presets) is one field whose choices `H/L` apply at once; every
// other control joins a row read from geometry, as in the shared modal
// grammar, where `H/L` set the value of what the cursor is on (a switch off
// or on, a disclosure closed or open) before moving beside it.

import { type RefObject, useEffect } from "react";
import {
  enterModalNormal,
  focusModalStop,
  isModalTextField,
  modalRows,
  modalStops,
} from "./commands/modalNavigation";
import { desktopImeOwnsKey } from "./commands/imeShortcut";
import { regionMotionKey } from "./commands/regionNavigation";
import { workspaceCommandKey } from "./commands/workspaceCommandKey";
import {
  nextRunConfigChoiceIndex,
  type RunConfigKeyAction,
  runConfigKeyAction,
  runConfigMotionAction,
} from "./runConfigKeyboard";

/** Dispatched on a `[data-config-row]` whose control is a Select: the row
 *  steps its own value and cancels the event. `detail.delta` is -1 or 1. */
export const RUN_CONFIG_STEP_EVENT = "cowboy:run-config-step";

const PANE = "[data-config-pane]";

/** Regions inside the panel that keep their own keys once focused. */
export const RUN_CONFIG_OWN_KEYS = [
  "[role='tree']",
  "[role='listbox']",
  "[role='menu']",
  ".cm-editor",
  "[data-desktop-keys='own']",
].join(", ");
const ROW = "[data-config-row]";

interface Field {
  /** The config row, or the first control of a geometric row. */
  readonly element: HTMLElement;
  readonly stops: readonly HTMLElement[];
  readonly config: boolean;
}

function usable(element: HTMLElement): boolean {
  return !element.matches(":disabled, [aria-disabled='true']");
}

function shown(element: HTMLElement): boolean {
  return element.getClientRects().length > 0;
}

function isSelected(choice: HTMLElement): boolean {
  return choice.classList.contains("Mui-selected") ||
    choice.getAttribute("aria-selected") === "true" ||
    choice.getAttribute("aria-pressed") === "true";
}

export function configChoices(row: HTMLElement): HTMLElement[] {
  return [...row.querySelectorAll<HTMLElement>("[data-config-choice]")]
    .filter(usable);
}

function selectedChoice(choices: readonly HTMLElement[]): number {
  return choices.findIndex(isSelected);
}

/** Focus a config row on its selected choice. */
export function focusConfigRow(row: HTMLElement): boolean {
  const choices = configChoices(row);
  const choice = choices[Math.max(0, selectedChoice(choices))];
  if (!choice) return false;
  choice.focus();
  return true;
}

export function runConfigPanes(panel: HTMLElement): HTMLElement[] {
  return [...panel.querySelectorAll<HTMLElement>(PANE)].filter(shown);
}

/** A pane's fields top to bottom. */
export function paneFields(pane: HTMLElement): Field[] {
  type Item = { element: HTMLElement; config: boolean };
  const items: Item[] = [
    ...[...pane.querySelectorAll<HTMLElement>(ROW)]
      .filter((row) => shown(row) && configChoices(row).length > 0)
      .map((element) => ({ element, config: true })),
    ...modalStops(pane)
      .filter((stop) => stop.closest(ROW) === null)
      .map((element) => ({ element, config: false })),
    // A tree's rows are not buttons; the tree is one stop at its roving row.
    ...[...pane.querySelectorAll<HTMLElement>("[role='tree']")]
      .map((tree) =>
        tree.querySelector<HTMLElement>("[role='treeitem'][tabindex='0']") ??
          tree.querySelector<HTMLElement>("[role='treeitem']")
      )
      .filter((row): row is HTMLElement => row !== null && shown(row))
      .filter((row) => !modalStops(pane).includes(row))
      .map((element) => ({ element, config: false })),
  ];
  const box = ({ element }: Item): DOMRect =>
    (isModalTextField(element)
      ? element.closest<HTMLElement>(
        ".MuiFormControl-root, .MuiInputBase-root",
      ) ??
        element
      : element).getBoundingClientRect();
  return modalRows(items, box).flatMap((row): Field[] => {
    const loose = row.filter((item) => !item.config).map((item) =>
      item.element
    );
    return [
      ...row.filter((item) => item.config).map((item) => ({
        element: item.element,
        stops: configChoices(item.element),
        config: true,
      })),
      ...(loose.length > 0
        ? [{ element: loose[0]!, stops: loose, config: false }]
        : []),
    ];
  });
}

function fieldHolds(field: Field, active: Element | null): boolean {
  if (!active) return false;
  if (field.config) return field.element.contains(active);
  const region = active.closest(RUN_CONFIG_OWN_KEYS);
  return field.stops.some((stop) =>
    stop === active || stop.contains(active) || active.contains(stop) ||
    (region !== null && region.contains(stop))
  );
}

function centreX(element: HTMLElement): number {
  const rect = element.getBoundingClientRect();
  return (rect.left + rect.right) / 2;
}

function focusField(field: Field, from: HTMLElement | null): void {
  if (field.config) {
    focusConfigRow(field.element);
    return;
  }
  const x = from ? centreX(from) : null;
  const stop = x === null
    ? field.stops[0]!
    : field.stops.reduce((best, next) =>
      Math.abs(centreX(next) - x) < Math.abs(centreX(best) - x) ? next : best
    );
  focusModalStop(stop);
}

/** Where each pane's cursor was when the user left it. */
const paneCursor = new WeakMap<HTMLElement, HTMLElement>();

function enterPane(pane: HTMLElement): void {
  const remembered = paneCursor.get(pane);
  if (
    remembered?.isConnected && pane.contains(remembered) &&
    shown(remembered) && usable(remembered)
  ) {
    remembered.focus();
    return;
  }
  const first = paneFields(pane)[0];
  if (first) focusField(first, null);
  else pane.focus({ preventScroll: true });
}

function switchState(element: HTMLElement): boolean | null {
  if (element instanceof HTMLInputElement && element.type === "checkbox") {
    return element.checked;
  }
  if (element.matches("[role='switch'], [role='checkbox']")) {
    return element.getAttribute("aria-checked") === "true";
  }
  return null;
}

/** `H/L` on a geometric row: set what the cursor is on, else move beside. */
function changeLoose(
  field: Field,
  active: HTMLElement | null,
  delta: -1 | 1,
): void {
  const current =
    field.stops.find((stop) =>
      active !== null && (stop === active || stop.contains(active))
    ) ?? field.stops[0]!;
  const want = delta > 0;
  const checked = switchState(current);
  if (checked !== null && checked !== want) {
    current.click();
    return;
  }
  const expanded = current.getAttribute("aria-expanded");
  if (expanded !== null && (expanded === "true") !== want) {
    current.click();
    return;
  }
  if (current.matches("[role='combobox']") && want) {
    openSelect(current);
    return;
  }
  const index = field.stops.indexOf(current);
  const next = field.stops[
    Math.max(0, Math.min(field.stops.length - 1, index + delta))
  ]!;
  focusModalStop(next);
}

function openSelect(display: HTMLElement): void {
  // MUI Select opens from its display's primary mousedown, not click.
  display.focus();
  display.dispatchEvent(
    new MouseEvent("mousedown", {
      bubbles: true,
      cancelable: true,
      button: 0,
    }),
  );
}

/** Step a config row: apply the next choice, or let a Select step itself. */
function changeConfig(row: HTMLElement, delta: -1 | 1, wrap: boolean): boolean {
  const choices = configChoices(row);
  if (choices.length === 0) return false;
  const select = choices.find((choice) =>
    choice.hasAttribute("data-config-select")
  );
  if (select) {
    select.focus();
    const handled = !row.dispatchEvent(
      new CustomEvent(RUN_CONFIG_STEP_EVENT, {
        cancelable: true,
        detail: { delta, wrap },
      }),
    );
    if (!handled) openSelect(select);
    return true;
  }
  const active = document.activeElement instanceof HTMLElement
    ? document.activeElement
    : null;
  const focused = active ? choices.indexOf(active) : -1;
  const current = focused >= 0 ? focused : selectedChoice(choices);
  const next = nextRunConfigChoiceIndex(choices.length, current, delta, wrap);
  const choice = choices[next];
  if (!choice) return false;
  choice.focus();
  if (next !== current || !isSelected(choice)) choice.click();
  return true;
}

let goPending = false;
let goTimer: ReturnType<typeof setTimeout> | undefined;

/** The last control the cursor was on. A switch that is disabled while its
 *  change saves drops DOM focus to the body; the cursor stays here. */
let lastCursor: HTMLElement | null = null;

/** Follow the cursor inside the panel. The panel is read per event: a
 *  dialog portal mounts its content after the opening effect runs. */
export function trackRunConfigCursor(
  panel: () => HTMLElement | null,
): () => void {
  const track = (event: FocusEvent): void => {
    if (
      event.target instanceof HTMLElement &&
      panel()?.contains(event.target) === true
    ) {
      lastCursor = event.target;
    }
  };
  // When the control under the cursor is disabled for a moment (a switch
  // while its change saves) focus falls to the dialog frame; put it back as
  // soon as the control is usable again, unless the user moved meanwhile.
  let restore = 0;
  const lost = (event: FocusEvent): void => {
    const root = panel();
    if (!root || !(event.target instanceof HTMLElement)) return;
    if (!root.contains(event.target)) return;
    const next = event.relatedTarget;
    if (next instanceof Node && !(next as Node).contains(root)) return;
    cancelAnimationFrame(restore);
    let frames = 0;
    const retry = (): void => {
      const active = document.activeElement;
      if (active !== null && !active.contains(root)) return;
      const cursor = lastCursorIn(root);
      if (cursor && usable(cursor)) {
        cursor.focus({ preventScroll: true });
        return;
      }
      if (++frames < 120) restore = requestAnimationFrame(retry);
    };
    restore = requestAnimationFrame(retry);
  };
  document.addEventListener("focusin", track, true);
  document.addEventListener("focusout", lost, true);
  return (): void => {
    cancelAnimationFrame(restore);
    document.removeEventListener("focusin", track, true);
    document.removeEventListener("focusout", lost, true);
    lastCursor = null;
  };
}

/** The last cursor, or its replacement when a save re-rendered it. */
function lastCursorIn(panel: HTMLElement): HTMLElement | null {
  if (!lastCursor) return null;
  if (lastCursor.isConnected && panel.contains(lastCursor)) return lastCursor;
  const label = lastCursor.getAttribute("aria-label");
  return label
    ? panel.querySelector<HTMLElement>(`[aria-label="${CSS.escape(label)}"]`)
    : null;
}

function cursorIn(panel: HTMLElement): HTMLElement | null {
  const active = document.activeElement;
  if (active instanceof HTMLElement && panel.contains(active)) return active;
  // Focus that fell back to the body or the dialog's own frame.
  if (active !== null && !active.contains(panel)) return null;
  const cursor = lastCursorIn(panel);
  if (cursor && usable(cursor)) cursor.focus({ preventScroll: true });
  return cursor;
}

/** Run one resolved key on the panel. Returns true when it was consumed. */
export function runConfigKey(
  panel: HTMLElement,
  action: RunConfigKeyAction,
): boolean {
  if (action.type !== "go") goPending = false;
  const active = cursorIn(panel);
  if (action.type === "preset") {
    const preset = panel.querySelector<HTMLElement>(
      `[data-config-preset="${String(action.index)}"]`,
    );
    if (!preset || !usable(preset)) return false;
    preset.focus();
    preset.click();
    return true;
  }
  if (action.type === "direct") {
    const row = panel.querySelector<HTMLElement>(
      `[data-config-shortcut="${action.shortcut}"]`,
    );
    if (!row) return false;
    const select = row.querySelector<HTMLElement>("[data-config-select]");
    if (select && usable(select)) {
      openSelect(select);
      return true;
    }
    return changeConfig(row, 1, true);
  }
  const panes = runConfigPanes(panel);
  if (panes.length === 0) return false;
  const pane = active?.closest<HTMLElement>(PANE) ?? panes[0]!;
  if (action.type === "pane") {
    const index = panes.indexOf(pane);
    const target = action.wrap
      ? panes[(index + action.delta + panes.length) % panes.length]!
      : panes[Math.max(0, Math.min(panes.length - 1, index + action.delta))]!;
    if (target === pane && active) return true;
    if (active) paneCursor.set(pane, active);
    enterPane(target);
    return true;
  }
  if (action.type === "go") {
    if (!goPending) {
      goPending = true;
      globalThis.clearTimeout(goTimer);
      goTimer = globalThis.setTimeout(() => {
        goPending = false;
      }, 1000);
      return true;
    }
    goPending = false;
  }
  const fields = paneFields(pane);
  if (fields.length === 0) return false;
  if (action.type === "go" || action.type === "edge") {
    const edge = action.type === "go" ? "first" : action.edge;
    focusField(edge === "first" ? fields[0]! : fields.at(-1)!, active);
    return true;
  }
  const index = fields.findIndex((field) => fieldHolds(field, active));
  if (action.type === "field") {
    const next = index < 0
      ? (action.delta > 0 ? fields[0]! : fields.at(-1)!)
      : fields[Math.max(0, Math.min(fields.length - 1, index + action.delta))]!;
    focusField(next, active);
    return true;
  }
  const field = fields[index] ?? fields[0]!;
  if (field.config) return changeConfig(field.element, action.delta, false);
  changeLoose(field, index < 0 ? null : active, action.delta);
  return true;
}

/** The panel's keymap while the dialog is open: the cursor starts on the
 *  first field and keys resolve through `runConfigKey`. */
export function useRunConfigKeyboard(
  open: boolean,
  panelRef: RefObject<HTMLDivElement | null>,
): void {
  useEffect(() => {
    if (!open) return undefined;
    // Open on the first field, the presets when they exist. Provider options
    // and presets can arrive a moment after the dialog, so keep following
    // the first field until the user moves.
    let settled = false;
    let tries = 0;
    let focusFirst = 0;
    const placeCursor = (): void => {
      const firstRow = panelRef.current?.querySelector<HTMLElement>(
        "[data-config-row]",
      );
      if (firstRow && !firstRow.contains(document.activeElement)) {
        focusConfigRow(firstRow);
      }
      tries += 1;
      if (!settled && tries < 90) {
        focusFirst = requestAnimationFrame(placeCursor);
      }
    };
    focusFirst = requestAnimationFrame(placeCursor);
    const settle = (): void => {
      settled = true;
    };
    globalThis.addEventListener("pointerdown", settle, true);
    const untrack = trackRunConfigCursor(() => panelRef.current);
    const onKeyDown = (event: KeyboardEvent): void => {
      settled = true;
      if (desktopImeOwnsKey(event)) return;
      const panel = panelRef.current;
      if (!panel) return;
      // A picker, menu or nested dialog above the panel keeps its own keys.
      if (document.querySelector("[role='listbox']") !== null) return;
      const target = event.target instanceof HTMLElement ? event.target : null;
      // The dialog's frame (or the body) holds focus when the control under
      // the cursor was disabled; the panel still owns the key.
      if (target && !panel.contains(target) && !target.contains(panel)) {
        return;
      }
      // A region with its own keymap (the session tree, its search) keeps
      // every key but the pane motions that leave it.
      const region = target?.closest<HTMLElement>(RUN_CONFIG_OWN_KEYS);
      if (
        target && region && panel.contains(region) &&
        // The Normal cursor on a text field sits on its box, not inside.
        !target.hasAttribute("data-desktop-field-cursor")
      ) {
        const regionMotion = regionMotionKey(event);
        const key = workspaceCommandKey(event);
        const leave = regionMotion !== null
          ? runConfigMotionAction(regionMotion)
          : !isModalTextField(target) && !event.metaKey && !event.ctrlKey &&
              !event.altKey && (key === "[" || key === "]")
          ? runConfigKeyAction(key)
          : null;
        if (leave === null || !runConfigKey(panel, leave)) return;
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      // Insert: a text field owns every key; Esc leaves it for Normal.
      if (target && isModalTextField(target)) {
        const escape = (event.key === "Escape" && !event.ctrlKey &&
          !event.metaKey && !event.altKey && !event.shiftKey) ||
          (event.ctrlKey && event.code === "BracketLeft");
        if (!escape) return;
        enterModalNormal(target);
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      if (
        event.code === "Enter" && !event.metaKey && !event.ctrlKey &&
        !event.altKey && target instanceof HTMLInputElement &&
        target.type === "checkbox"
      ) {
        target.click();
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      const motion = regionMotionKey(event);
      if (motion === null && (event.metaKey || event.ctrlKey || event.altKey)) {
        return;
      }
      const action = motion !== null
        ? runConfigMotionAction(motion)
        : runConfigKeyAction(workspaceCommandKey(event));
      if (action === null || !runConfigKey(panel, action)) return;
      event.preventDefault();
      event.stopPropagation();
    };
    globalThis.addEventListener("keydown", onKeyDown, true);
    return (): void => {
      cancelAnimationFrame(focusFirst);
      globalThis.removeEventListener("pointerdown", settle, true);
      untrack();
      globalThis.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open, panelRef]);
}
