import { alpha, Box, Stack, Tooltip, Typography } from "@mui/material";
import type { ShortcutKeycapAvailability } from "../ShortcutKeycap";
import { DesktopShortcut } from "./commands/DesktopKeycap";

export interface DesktopShortcutSlot {
  shortcut: string;
  label?: string;
  title?: string;
  availability?: ShortcutKeycapAvailability;
}

export interface DesktopShortcutGroup {
  label?: string;
  slots: readonly DesktopShortcutSlot[];
}

/**
 * Canonical footer for a keyboard-owned Desktop surface. It is both a legend
 * and a live state display: every keycap receives the same inactive / available
 * / active grammar as its embedded counterpart.
 */
export function DesktopShortcutBar({
  groups,
  sticky = false,
}: {
  groups: readonly DesktopShortcutGroup[];
  sticky?: boolean;
}): React.JSX.Element {
  return (
    <Box
      component="footer"
      data-desktop-shortcut-bar
      sx={{
        ...(sticky && { position: "sticky", bottom: 0, zIndex: 5 }),
        minHeight: 42,
        px: 2,
        py: 0.8,
        borderTop: 1,
        borderColor: "divider",
        bgcolor: (theme) => alpha(theme.palette.background.paper, 0.94),
        backdropFilter: "blur(16px)",
        overflowX: "auto",
        overflowY: "hidden",
        scrollbarWidth: "thin",
        "&::-webkit-scrollbar": { height: 3 },
        "&::-webkit-scrollbar-thumb": {
          bgcolor: "action.disabled",
          borderRadius: 99,
        },
      }}
    >
      <Stack
        direction="row"
        alignItems="center"
        justifyContent="space-between"
        spacing={2.25}
        sx={{ width: "max-content", minWidth: "100%" }}
      >
        {groups.map((group, groupIndex) => (
          <Stack
            key={`${group.label ?? "shortcuts"}-${String(groupIndex)}`}
            direction="row"
            alignItems="center"
            spacing={0.8}
            sx={{ flexShrink: 0 }}
          >
            {group.label && (
              <Typography
                variant="overline"
                color="text.secondary"
                sx={{ opacity: 0.68, mr: 0.15, lineHeight: 1 }}
              >
                {group.label}
              </Typography>
            )}
            {group.slots.map((slot, slotIndex) => {
              const body = (
                <Stack
                  component="span"
                  data-desktop-shortcut-slot
                  data-desktop-shortcut-state={slot.availability ?? "available"}
                  direction="row"
                  alignItems="center"
                  spacing={0.45}
                  sx={{ color: "text.secondary", whiteSpace: "nowrap" }}
                >
                  <DesktopShortcut
                    shortcut={slot.shortcut}
                    quiet
                    {...(slot.availability
                      ? { availability: slot.availability }
                      : {})}
                  />
                  {slot.label && (
                    <Typography component="span" variant="caption">
                      {slot.label}
                    </Typography>
                  )}
                </Stack>
              );
              return slot.title
                ? (
                  <Tooltip
                    key={`${slot.shortcut}-${String(slotIndex)}`}
                    title={slot.title}
                    enterDelay={400}
                  >
                    {body}
                  </Tooltip>
                )
                : (
                  <Box key={`${slot.shortcut}-${String(slotIndex)}`} component="span">
                    {body}
                  </Box>
                );
            })}
          </Stack>
        ))}
      </Stack>
    </Box>
  );
}
