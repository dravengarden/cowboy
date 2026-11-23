import { useEffect, useRef } from "react";
import { Button, Stack } from "@mui/material";
import { Kbd } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";

/** Replace or fork's two actions. Keyboard first on Desktop: from anywhere
 *  in the enclosing form, Mod+Enter replaces (or forks when Replace is not
 *  offered) and Mod+Shift+Enter forks. */
export function NewSessionActions({
  desktop,
  forkOnly,
  allowFork,
  canReplace,
  canFork,
  busy,
  onReplace,
  onFork,
}: {
  desktop: boolean;
  forkOnly: boolean;
  allowFork: boolean;
  canReplace: boolean;
  canFork: boolean;
  busy: boolean;
  onReplace: () => Promise<void>;
  onFork: () => Promise<void>;
}): React.JSX.Element {
  const own = useRef<HTMLDivElement>(null);
  const latest = useRef({ canReplace, canFork, onReplace, onFork, forkOnly });
  latest.current = { canReplace, canFork, onReplace, onFork, forkOnly };
  useEffect(() => {
    if (!desktop) return undefined;
    const onKey = (event: KeyboardEvent): void => {
      if (event.key !== "Enter" || event.isComposing || event.repeat || event.altKey) return;
      if (!(event.metaKey || event.ctrlKey)) return;
      const form = own.current?.closest("[data-new-session-editor]");
      if (!form || !(event.target instanceof Node) || !form.contains(event.target)) return;
      const current = latest.current;
      const fork = event.shiftKey || current.forkOnly;
      event.preventDefault();
      event.stopPropagation();
      if (fork && current.canFork) void current.onFork();
      else if (!fork && current.canReplace) void current.onReplace();
    };
    globalThis.addEventListener("keydown", onKey, true);
    return () => globalThis.removeEventListener("keydown", onKey, true);
  }, [desktop]);
  return (
    <Stack ref={own} direction="row" spacing={1} useFlexGap flexWrap="wrap" alignItems="center">
      {!forkOnly && (
        <Button
          variant="contained"
          data-new-session-continue-run
          disabled={!canReplace}
          onClick={() => void onReplace()}
          sx={{ gap: 0.75, ...(desktop && { textTransform: "none" }) }}
        >
          {busy ? "Saving…" : "Replace this session"}
          <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} availability={canReplace ? "available" : "inactive"} />
        </Button>
      )}
      {allowFork && (
        <Button
          variant={forkOnly ? "contained" : "outlined"}
          data-new-session-fork-run
          disabled={!canFork}
          onClick={() => void onFork()}
          sx={{ gap: 0.75, ...(desktop && { textTransform: "none" }) }}
        >
          Fork to new session
          <Kbd
            keys={forkOnly ? `${MOD_LABEL}${ENTER_LABEL}` : `${MOD_LABEL}⇧${ENTER_LABEL}`}
            availability={canFork ? "available" : "inactive"}
          />
        </Button>
      )}
    </Stack>
  );
}
