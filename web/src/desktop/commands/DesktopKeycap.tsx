import { Stack } from "@mui/material";
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
  const keys = shortcut.split("+").filter(Boolean);
  return (
    <Stack
      direction="row"
      spacing={0.35}
      alignItems="center"
      aria-label={shortcut}
    >
      {keys.map((key, index) => (
        <DesktopKeycap
          key={`${key}-${String(index)}`}
          keyLabel={key}
          quiet={quiet}
          availability={availability}
        />
      ))}
    </Stack>
  );
}
