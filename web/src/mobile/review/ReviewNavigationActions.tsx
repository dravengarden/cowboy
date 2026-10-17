import { Close, Refresh, SettingsOutlined } from "@mui/icons-material";
import { Box, CircularProgress, IconButton } from "@mui/material";
import {
  type MobileSheetAction,
  MobileSheetActionGroup,
} from "@cowboy/app-shell";
import { createPortal } from "react-dom";
import { openAppSettings } from "../../appSettings";
import { HintTooltip } from "../../HintTooltip";
import { useNetworkActionState } from "../../NetworkActionFeedback";

/** The navigator owns its actions even when they live in the page's bottom
 * bar. Moving between overview and sidebar never duplicates the navigator or
 * registers callbacks that could outlive their workspace. */
export function ReviewNavigationActions({
  embedded,
  container,
  label,
  actions = [],
  refreshLabel,
  onRefresh,
  disabled,
  closeLabel,
  onClose,
}: {
  embedded: boolean;
  container?: HTMLElement | null | undefined;
  label: string;
  actions?: readonly MobileSheetAction[];
  refreshLabel: string;
  onRefresh: () => Promise<void> | void;
  disabled: boolean;
  closeLabel: string;
  onClose: () => void;
}): React.JSX.Element | null {
  const refresh = useNetworkActionState();
  const items: MobileSheetAction[] = [
    ...actions,
    {
      key: "refresh",
      label: refreshLabel,
      icon: refresh.progress
        ? <CircularProgress size={20} color="inherit" aria-label="Refreshing" />
        : <Refresh fontSize="small" />,
      onPress: () => void refresh.run(onRefresh),
      disabled: disabled || refresh.pending,
    },
    {
      key: "settings",
      label: "Review settings",
      icon: <SettingsOutlined fontSize="small" />,
      onPress: () => openAppSettings({ section: "code" }),
    },
  ];
  if (embedded) {
    if (!container) return null;
    return createPortal(
      <Box
        role="group"
        aria-label={label}
        data-review-navigation-actions="inline"
        onClick={(event) => event.stopPropagation()}
        sx={{ display: "flex", alignItems: "center", gap: 0.5 }}
      >
        {items.filter((action) => action.visible !== false).map((action) => (
          <HintTooltip key={action.key} title={action.label}>
            <span>
              <IconButton
                aria-label={action.label}
                aria-busy={action.key === "refresh" && refresh.pending ||
                  undefined}
                disabled={action.disabled}
                onClick={action.onPress}
                sx={{ width: 44, height: 44, color: "text.secondary" }}
              >
                {action.icon}
              </IconButton>
            </span>
          </HintTooltip>
        ))}
      </Box>,
      container,
    );
  }
  return (
    <Box
      component="nav"
      aria-label={label}
      data-review-navigation-actions="sidebar"
      sx={{
        position: "absolute",
        zIndex: 3,
        left: 0,
        right: 0,
        bottom: "max(env(safe-area-inset-bottom, 0px), 12px)",
        display: "flex",
        justifyContent: "flex-start",
        px: 0.75,
        pointerEvents: "none",
        "& > [data-mobile-sheet-footer-shield]": {
          width: "auto",
          flex: "0 0 auto",
        },
      }}
    >
      <MobileSheetActionGroup
        actions={[...items, {
          key: "close",
          label: closeLabel,
          icon: <Close fontSize="small" />,
          onPress: onClose,
        }]}
      />
    </Box>
  );
}
