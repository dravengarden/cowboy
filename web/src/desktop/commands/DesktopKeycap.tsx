import { Stack } from "@mui/material";
import { ShortcutKeycap } from "../../ShortcutKeycap";

export function DesktopKeycap({
  keyLabel,
  accent = false,
  quiet = false,
}: {
  keyLabel: string;
  accent?: boolean;
  quiet?: boolean;
}): React.JSX.Element {
  return (
    <ShortcutKeycap
      keyLabel={keyLabel}
      variant={quiet ? "global" : "default"}
      accent={accent}
    />
  );
}

export function DesktopShortcut(
  { shortcut, quiet = false }: { shortcut: string; quiet?: boolean },
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
        <DesktopKeycap key={`${key}-${String(index)}`} keyLabel={key} quiet={quiet} />
      ))}
    </Stack>
  );
}
