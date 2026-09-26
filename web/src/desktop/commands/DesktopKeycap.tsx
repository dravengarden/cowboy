import { Box, Stack } from "@mui/material";
import {
  displayShortcutKey,
  ShortcutKeycap,
  type ShortcutKeycapAvailability,
} from "../../ShortcutKeycap";

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
  const strokes = shortcut.split(" → ").filter(Boolean);
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
