import { useEffect, useRef } from "react";
import { LeaderKeycap } from "./commands/DesktopKeycap";
import { useOptionalDesktopCommands } from "./commands/DesktopCommandProvider";
import { desktopLeaderSequence } from "./commands/workspaceShortcuts";

/** A blocking approval offers at most this many numbered choices. */
export const DESKTOP_APPROVAL_CHOICE_LIMIT = 9;

export interface DesktopApprovalChoice {
  /** The provider's own wording; which-key and the palette show it. */
  label: string;
  run: () => void;
}

/** The leader path of the nth choice (`␣1`). */
export function desktopApprovalLeaderKey(index: number): string | null {
  return index >= 0 && index < DESKTOP_APPROVAL_CHOICE_LIMIT
    ? String(index + 1)
    : null;
}

/**
 * The keyboard half of a pending approval (FOCUS.md "Approvals"): the choices
 * are numbered in the order the Provider gave them and `␣1`…`␣9` answers from
 * any focus, the Composer's Insert mode included. The commands exist only
 * while the approval is on screen and carry the choice's own label.
 */
export function DesktopApprovalCommands({
  choices,
  available,
}: {
  choices: readonly DesktopApprovalChoice[];
  /** False while an answer is in flight: the keys then do nothing. */
  available: boolean;
}): null {
  // Isolated fixtures mount the surface without a command host.
  const register = useOptionalDesktopCommands()?.register;
  const latest = useRef(choices);
  latest.current = choices;
  const open = useRef(available);
  open.current = available;
  const labels = choices.slice(0, DESKTOP_APPROVAL_CHOICE_LIMIT)
    .map((choice) => choice.label);
  // Re-register only when the wording changes, not on every render.
  const signature = JSON.stringify(labels);
  useEffect(() => {
    if (!register) return undefined;
    const names = JSON.parse(signature) as string[];
    const releases = names.map((label, index) =>
      register({
        id: `approval.choice.${String(index + 1)}`,
        title: label,
        description: "Answer the pending approval",
        group: "Approval",
        sequence: desktopLeaderSequence(String(index + 1)),
        contextual: true,
        when: () => open.current,
        disabledReason: "Your answer is being sent",
        run: () => latest.current[index]?.run(),
      })
    );
    return () => {
      for (const release of releases) release();
    };
  }, [register, signature]);
  return null;
}

/** The slot drawn on the nth choice; inactive while an answer is in flight. */
export function DesktopApprovalKeycap({
  index,
  available,
}: {
  index: number;
  available: boolean;
}): React.JSX.Element | null {
  const key = desktopApprovalLeaderKey(index);
  return key === null
    ? null
    : <LeaderKeycap leaderKey={key} scopeAvailable={available} />;
}
