import { Box, Stack } from "@mui/material";
import {
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

export function DesktopShortcut(
  { shortcut, quiet = false, availability = "available" }: {
    shortcut: string;
    quiet?: boolean;
    availability?: ShortcutKeycapAvailability;
  },
): React.JSX.Element {
  const strokes = shortcut.split(" → ").filter(Boolean);
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
