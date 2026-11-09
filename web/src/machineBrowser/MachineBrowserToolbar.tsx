/** Browser-style controls of the Machine browser panel: Close, Back, Forward
 * and Reload (key chords on the attached session), the touch keyboard and
 * the "More" menu with the lifecycle and clipboard commands. Mobile places
 * them, under a one-line identity caption, in a bottom toolbar like a phone
 * browser; Desktop keeps its top row. */
import {
  ArrowBackRounded,
  ArrowForwardRounded,
  CloseRounded,
  KeyboardHideOutlined,
  KeyboardOutlined,
  MoreHorizRounded,
  RefreshRounded,
} from "@mui/icons-material";
import {
  Box,
  Divider,
  IconButton,
  ListItemText,
  Menu,
  MenuItem,
  Stack,
  Tooltip,
} from "@mui/material";
import { type ReactNode, useState } from "react";
import type { BrowserChord } from "./browserKeys";
import type { BrowserChromeState } from "./browserChrome";

export interface BrowserMenuItem {
  key: string;
  label: string;
  disabled?: boolean;
  destructive?: boolean;
  /** Starts a new group with a divider above it. */
  group?: boolean;
  onSelect(): void;
}

const NAVIGATION: readonly { chord: BrowserChord; label: string; icon: ReactNode }[] = [
  { chord: "back", label: "Back", icon: <ArrowBackRounded /> },
  { chord: "forward", label: "Forward", icon: <ArrowForwardRounded /> },
  { chord: "reload", label: "Reload", icon: <RefreshRounded /> },
];

function targetSize(touch: boolean): number {
  return touch ? 46 : 34;
}

export function BrowserNavigationButtons({
  chrome,
  touch,
  onChord,
}: {
  chrome: BrowserChromeState;
  touch: boolean;
  onChord: (chord: BrowserChord) => void;
}): React.JSX.Element {
  const size = targetSize(touch);
  return (
    <>
      {NAVIGATION.map(({ chord, label, icon }) => (
        <Tooltip key={chord} title={touch ? "" : label}>
          <span>
            <IconButton
              aria-label={label}
              disabled={!chrome.navigation}
              onClick={() => onChord(chord)}
              sx={{ width: size, height: size }}
            >
              {icon}
            </IconButton>
          </span>
        </Tooltip>
      ))}
    </>
  );
}

export function BrowserMenu({
  items,
  touch,
  upward,
}: {
  items: readonly BrowserMenuItem[];
  touch: boolean;
  /** Open above the button (a bottom toolbar). */
  upward: boolean;
}): React.JSX.Element {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null);
  const size = targetSize(touch);
  const close = (): void => setAnchor(null);
  return (
    <>
      <Tooltip title={touch ? "" : "More"}>
        <IconButton
          aria-label="More browser actions"
          aria-haspopup="menu"
          aria-expanded={anchor ? "true" : undefined}
          onClick={(event) => setAnchor(event.currentTarget)}
          sx={{ width: size, height: size }}
        >
          <MoreHorizRounded />
        </IconButton>
      </Tooltip>
      <Menu
        anchorEl={anchor}
        open={anchor !== null}
        onClose={close}
        anchorOrigin={{ vertical: upward ? "top" : "bottom", horizontal: "right" }}
        transformOrigin={{ vertical: upward ? "bottom" : "top", horizontal: "right" }}
        slotProps={{ list: { dense: !touch, "aria-label": "Browser actions" } }}
      >
        {items.flatMap((item, index) => [
          ...(item.group && index > 0 ? [<Divider key={`${item.key}-divider`} />] : []),
          <MenuItem
            key={item.key}
            disabled={item.disabled}
            onClick={() => {
              close();
              item.onSelect();
            }}
            sx={{
              minHeight: touch ? 44 : 32,
              whiteSpace: "normal",
              color: item.destructive ? "error.main" : undefined,
            }}
          >
            <ListItemText>{item.label}</ListItemText>
          </MenuItem>,
        ])}
      </Menu>
    </>
  );
}

/** Height of the phone toolbar's targets (CSS px). */
const MOBILE_TARGET = 46;

/** The phone browser's bottom area: the identity caption (16 px line) over
 * one row of six equal 46 px targets, above the home indicator and the
 * software keyboard. The phone has no top row. */
export function MobileBrowserToolbar({
  chrome,
  keyboardOpen,
  identity,
  onChord,
  onKeyboard,
  onClose,
  menu,
}: {
  chrome: BrowserChromeState;
  keyboardOpen: boolean;
  /** Account, Machine and state on one ellipsized line. */
  identity: ReactNode;
  onChord: (chord: BrowserChord) => void;
  onKeyboard: () => void;
  onClose: () => void;
  menu: ReactNode;
}): React.JSX.Element {
  return (
    <Box
      component="nav"
      aria-label="Browser toolbar"
      data-machine-browser-toolbar
      sx={{
        flexShrink: 0,
        borderTop: 1,
        borderColor: "divider",
        bgcolor: "background.paper",
        pt: "2px",
        pb: "max(4px, calc(env(safe-area-inset-bottom, 0px) - var(--kb-inset, 0px)))",
        pl: "max(8px, env(safe-area-inset-left, 0px))",
        pr: "max(8px, env(safe-area-inset-right, 0px))",
      }}
    >
      <Box sx={{ px: 1 }}>{identity}</Box>
      <Stack direction="row" justifyContent="space-around" alignItems="center">
        <IconButton
          aria-label="Close browser"
          onClick={onClose}
          sx={{ width: MOBILE_TARGET, height: MOBILE_TARGET }}
        >
          <CloseRounded />
        </IconButton>
        <BrowserNavigationButtons chrome={chrome} touch onChord={onChord} />
        <IconButton
          aria-label={keyboardOpen ? "Hide keyboard" : "Show keyboard"}
          aria-pressed={keyboardOpen}
          disabled={!chrome.keyboard}
          // Keep focus in the keyboard overlay so a tap toggles, not reopens.
          onPointerDown={(event) => event.preventDefault()}
          onMouseDown={(event) => event.preventDefault()}
          onClick={onKeyboard}
          sx={{ width: MOBILE_TARGET, height: MOBILE_TARGET }}
        >
          {keyboardOpen ? <KeyboardHideOutlined /> : <KeyboardOutlined />}
        </IconButton>
        {menu}
      </Stack>
    </Box>
  );
}
