import { Box, Stack } from "@mui/material";
import {
  displayShortcutKey,
  ShortcutKeycap,
  type ShortcutKeycapAvailability,
} from "../../ShortcutKeycap";
import { useDesktopLeaderOptional } from "./leaderContext";
import { leaderShortcutAvailability } from "./shortcutAvailability";
import {
  DESKTOP_WORKSPACE_PREFIX,
  desktopLeaderLabel,
} from "./workspaceShortcuts";

/**
 * The leader slot primitive: one keycap holding the leader glyph and its key
 * (`␣N`). Interactive slots pass `scopeAvailable`; the keycap then lights up
 * on its own while the leader is armed. Reference slots (palette, help) pass
 * an explicit `availability` and never light.
 */
export function LeaderKeycap({
  leaderKey,
  scopeAvailable,
  availability,
  quiet = true,
}: {
  leaderKey: string;
  scopeAvailable?: boolean;
  availability?: ShortcutKeycapAvailability;
  quiet?: boolean;
}): React.JSX.Element {
  const leader = useDesktopLeaderOptional();
  // A grouped slot (`␣TR`) lights at the root, where its group key is next,
  // and inside its own group layer.
  const armed = leader?.armed === true && (leader.layer === "root" ||
    (leaderKey.length > 1 &&
      leader.layer === `group:${leaderKey[0]!.toLowerCase()}`));
  const state = availability ??
    leaderShortcutAvailability(scopeAvailable ?? true, armed);
  return (
    <ShortcutKeycap
      keyLabel={desktopLeaderLabel(leaderKey)}
      variant={quiet ? "global" : "default"}
      accent={state === "active"}
      availability={state}
    />
  );
}

export function DesktopKeycap({
  keyLabel,
  accent = false,
  quiet = false,
  availability = "available",
}: {
  keyLabel: string;
  accent?: boolean;
  quiet?: boolean;
  availability?: ShortcutKeycapAvailability;
}): React.JSX.Element {
  return (
    <ShortcutKeycap
      keyLabel={keyLabel}
      variant={quiet ? "global" : "default"}
      accent={accent}
      availability={availability}
    />
  );
}

/** `Mod+K` becomes `⌘K` on macOS and `Ctrl+K` where a modifier is a word. */
function compactStroke(stroke: string): string {
  const keys = stroke.split("+").filter(Boolean).map(displayShortcutKey);
  return keys.join(keys.every((key) => key.length === 1) ? "" : "+");
}

export function DesktopShortcut(
  { shortcut, quiet = false, compact = false, availability = "available" }: {
    shortcut: string;
    quiet?: boolean;
    /** Toolbar form: one keycap per stroke (`⌘K` `,`) and no arrow. */
    compact?: boolean;
    availability?: ShortcutKeycapAvailability;
  },
): React.JSX.Element {
  const strokes = shortcut.split(" → ").filter((stroke) => stroke.length > 0);
  if (strokes.length >= 2 && strokes[0] === DESKTOP_WORKSPACE_PREFIX) {
    return (
      <Stack direction="row" alignItems="center" aria-label={shortcut}>
        <LeaderKeycap
          leaderKey={strokes.slice(1).join("")}
          quiet={quiet || compact}
          {...(availability === "available"
            ? { scopeAvailable: true }
            : { availability })}
        />
      </Stack>
    );
  }
  if (compact) {
    return (
      <Stack
        direction="row"
        spacing={0.2}
        alignItems="center"
        aria-label={shortcut}
      >
        {strokes.map((stroke, strokeIndex) => (
          <DesktopKeycap
            key={`${stroke}-${String(strokeIndex)}`}
            keyLabel={compactStroke(stroke)}
            quiet={quiet}
            availability={availability}
          />
        ))}
      </Stack>
    );
  }
  return (
    <Stack
      direction="row"
      spacing={0.35}
      alignItems="center"
      aria-label={shortcut}
    >
      {strokes.map((stroke, strokeIndex) => (
        <Stack
          key={`${stroke}-${String(strokeIndex)}`}
          direction="row"
          spacing={0.35}
          alignItems="center"
        >
          {strokeIndex > 0 && (
            <Box component="span" aria-hidden sx={{ fontSize: "0.625rem", color: "text.disabled" }}>
              →
            </Box>
          )}
          {stroke.split("+").filter(Boolean).map((key, keyIndex) => (
            <DesktopKeycap
              key={`${key}-${String(keyIndex)}`}
              keyLabel={key}
              quiet={quiet}
              availability={availability}
            />
          ))}
        </Stack>
      ))}
    </Stack>
  );
}
