import { Box, Divider, ListItemText, Menu, MenuItem } from "@mui/material";
import { Close, PushPin } from "@mui/icons-material";
import { alpha, type Theme } from "@mui/material/styles";
import {
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { ShortcutKeycap } from "../ShortcutKeycap";
import {
  type DesktopCommand,
  useDesktopCommands,
} from "./commands/DesktopCommandProvider";
import { LeaderKeycap } from "./commands/DesktopKeycap";
import {
  DESKTOP_TAB_GROUP,
  DESKTOP_TAB_KEYS,
  DESKTOP_WORKSPACE_KEYS,
  desktopLeaderLabel,
  desktopLeaderSequence,
} from "./commands/workspaceShortcuts";
import {
  type DesktopTabCloseScope,
  desktopTabAfterClose,
  desktopTabs,
  desktopTabsToClose,
  desktopTabStep,
  keepDesktopTab,
  moveDesktopTab,
  reopenDesktopTab,
  toggleDesktopTabPin,
  updateDesktopTabs,
  withoutDesktopTabs,
} from "./workspaceTabs";

const TAB_DRAG_TYPE = "application/x-cowboy-workspace-tab";

/** A tab resolved against the live Sessions and Drafts. */
export interface DesktopTabItem {
  readonly key: string;
  readonly title: string;
  readonly kind: "session" | "draft";
  readonly pinned: boolean;
  readonly preview: boolean;
  /** Status dot and Provider icon, or the Draft glyph. */
  readonly leading: ReactNode;
}

interface TabMenu {
  readonly key: string;
  readonly left: number;
  readonly top: number;
}

/**
 * The Desktop tab strip (FOCUS.md "Tabs"). It owns every tab command: the
 * pointer (click, middle click, double click, drag, context menu) and the
 * leader (`␣[` `␣]`, the `␣B` group) run the same functions.
 */
export function DesktopTabStrip({
  items,
  currentKey,
  visits,
  exists,
  onOpen,
  onRename,
  sx,
}: {
  items: readonly DesktopTabItem[];
  currentKey: string | null;
  /** Most recently visited keys first: where closing the current tab goes. */
  visits: readonly string[];
  exists: (key: string) => boolean;
  onOpen: (key: string) => void;
  /** Rename a Session; a Draft's name is its title. */
  onRename: (key: string) => void;
  sx?: object;
}): React.JSX.Element {
  const registry = useDesktopCommands();
  // A pending approval takes the digits (FOCUS.md "Approvals"); the tabs
  // then show none, so every digit on screen does what it says.
  const digits = !registry.commands.some((command) =>
    command.id.startsWith("approval.choice.")
  );
  const [menu, setMenu] = useState<TabMenu | null>(null);
  const [drop, setDrop] = useState<{ key: string; after: boolean } | null>(
    null,
  );
  const strip = useRef<HTMLDivElement | null>(null);
  const live = useRef({ items, currentKey, visits, exists, onOpen, onRename });
  live.current = { items, currentKey, visits, exists, onOpen, onRename };

  const actions = useMemo(() => {
    const order = (): string[] => live.current.items.map((item) => item.key);
    const close = (key: string | null, scope: DesktopTabCloseScope): void => {
      const { currentKey: current, visits: visited, onOpen: open } =
        live.current;
      if (!key) return;
      const { tabs } = desktopTabs();
      let closing = desktopTabsToClose(tabs, key, scope);
      if (current && closing.includes(current)) {
        const next = desktopTabAfterClose(tabs, current, closing, visited);
        // The item on screen always has a tab: the last one stays.
        if (next === null) closing = closing.filter((k) => k !== current);
        else open(next);
      }
      updateDesktopTabs((state) => withoutDesktopTabs(state, closing));
    };
    return {
      close,
      step: (delta: number): void => {
        const { currentKey: current, onOpen: open } = live.current;
        const keys = order();
        const at = keys.indexOf(current ?? "");
        const next = desktopTabStep(
          keys.map((key) => ({ key, pinned: false, preview: false })),
          at === -1 ? null : current,
          delta,
        );
        if (next && next !== current) open(next);
      },
      openAt: (index: number): void => {
        const key = order()[index];
        if (key && key !== live.current.currentKey) live.current.onOpen(key);
      },
      pin: (key: string | null): void => {
        if (key) updateDesktopTabs((s) => toggleDesktopTabPin(s, key));
      },
      keep: (key: string | null): void => {
        if (key) updateDesktopTabs((s) => keepDesktopTab(s, key));
      },
      move: (key: string | null, delta: number): void => {
        if (!key) return;
        updateDesktopTabs((s) => {
          const at = s.tabs.findIndex((tab) => tab.key === key);
          return at === -1 ? s : moveDesktopTab(s, key, at + delta);
        });
      },
      reopen: (): void => {
        const { exists: live_exists, currentKey: current, onOpen: open } =
          live.current;
        const result = reopenDesktopTab(desktopTabs(), live_exists, current);
        updateDesktopTabs(() => result.state);
        if (result.key) open(result.key);
      },
      canReopen: (): boolean => {
        const state = desktopTabs();
        return state.closed.some((key) =>
          live.current.exists(key) &&
          !state.tabs.some((tab) => tab.key === key)
        );
      },
    };
  }, []);

  useEffect(() => {
    const current = (): string | null => live.current.currentKey;
    const tab = (key = current()) =>
      live.current.items.find((item) => item.key === key);
    const count = (): number => live.current.items.length;
    const group = (key: string): string[] =>
      desktopLeaderSequence(`${DESKTOP_TAB_GROUP}${key}`);
    const base = { group: "Tabs", leaderAnywhere: true } as const;
    const commands: DesktopCommand[] = [
      {
        ...base,
        id: "tabs.previous",
        title: "Previous Tab",
        sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.previousTab),
        allowInEditor: true,
        when: () => count() > 1,
        run: () => actions.step(-1),
      },
      {
        ...base,
        id: "tabs.next",
        title: "Next Tab",
        sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.nextTab),
        allowInEditor: true,
        when: () => count() > 1,
        run: () => actions.step(1),
      },
      {
        ...base,
        id: "tabs.close",
        title: "Close Tab",
        description: "The Session or Draft itself is kept",
        sequence: group(DESKTOP_TAB_KEYS.close),
        when: () => count() > 1,
        run: () => actions.close(current(), "one"),
      },
      {
        ...base,
        id: "tabs.closeOthers",
        title: "Close Other Tabs",
        description: "Pinned tabs stay",
        sequence: group(DESKTOP_TAB_KEYS.closeOthers),
        when: () => count() > 1,
        run: () => actions.close(current(), "others"),
      },
      {
        ...base,
        id: "tabs.closeRight",
        title: "Close Tabs to the Right",
        sequence: group(DESKTOP_TAB_KEYS.closeRight),
        when: () => count() > 1,
        run: () => actions.close(current(), "right"),
      },
      {
        ...base,
        id: "tabs.closeLeft",
        title: "Close Tabs to the Left",
        sequence: group(DESKTOP_TAB_KEYS.closeLeft),
        when: () => count() > 1,
        run: () => actions.close(current(), "left"),
      },
      {
        ...base,
        id: "tabs.closeAll",
        title: "Close All Tabs",
        description: "Pinned tabs and the current item stay",
        sequence: group(DESKTOP_TAB_KEYS.closeAll),
        when: () => count() > 1,
        run: () => actions.close(current(), "all"),
      },
      {
        ...base,
        id: "tabs.pin",
        title: "Pin or Unpin Tab",
        sequence: group(DESKTOP_TAB_KEYS.pin),
        when: () => tab() !== undefined,
        run: () => actions.pin(current()),
      },
      {
        ...base,
        id: "tabs.keep",
        title: "Keep Tab Open",
        description: "Turn the preview tab into a permanent one",
        sequence: group(DESKTOP_TAB_KEYS.keep),
        when: () => tab()?.preview === true,
        run: () => actions.keep(current()),
      },
      {
        ...base,
        id: "tabs.reopen",
        title: "Reopen Closed Tab",
        sequence: group(DESKTOP_TAB_KEYS.reopen),
        when: actions.canReopen,
        run: actions.reopen,
      },
      {
        ...base,
        id: "tabs.moveLeft",
        title: "Move Tab Left",
        sequence: group(DESKTOP_TAB_KEYS.moveLeft),
        when: () => count() > 1,
        run: () => actions.move(current(), -1),
      },
      {
        ...base,
        id: "tabs.moveRight",
        title: "Move Tab Right",
        sequence: group(DESKTOP_TAB_KEYS.moveRight),
        when: () => count() > 1,
        run: () => actions.move(current(), 1),
      },
      ...Array.from({ length: 9 }, (_, index): DesktopCommand => ({
        ...base,
        id: `tabs.open.${String(index + 1)}`,
        title: `Open Tab ${String(index + 1)}`,
        sequence: desktopLeaderSequence(String(index + 1)),
        fallback: true,
        when: () => index < count(),
        run: () => actions.openAt(index),
      })),
    ];
    const release = commands.map((command) => registry.register(command));
    return () => {
      for (const off of release) off();
    };
  }, [actions, registry.register]);

  // Typing in the item makes its preview tab permanent, as editing a file
  // does in VS Code.
  useEffect(() => {
    const keep = (event: Event): void => {
      const { currentKey: current, items: tabs } = live.current;
      if (!current || !tabs.some((tab) => tab.key === current && tab.preview)) {
        return;
      }
      const target = event.target instanceof Element ? event.target : null;
      if (
        target?.closest("[data-desktop-pane='prompt'], [data-workspace-document]")
      ) actions.keep(current);
    };
    document.addEventListener("beforeinput", keep, true);
    document.addEventListener("input", keep, true);
    return () => {
      document.removeEventListener("beforeinput", keep, true);
      document.removeEventListener("input", keep, true);
    };
  }, [actions]);

  useLayoutEffect(() => {
    strip.current?.querySelector<HTMLElement>("[aria-selected='true']")
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [currentKey, items.length]);

  // The menu keeps its subject while it animates closed.
  const shown = useRef<TabMenu | null>(null);
  if (menu) shown.current = menu;
  const menuKey = shown.current?.key ?? "";
  const menuItem = items.find((item) => item.key === menuKey);
  const menuIndex = items.findIndex((item) => item.key === menuKey);
  const unpinnedBeside = (from: number, to: number): boolean =>
    items.slice(from, to).some((item) => !item.pinned);
  const run = (action: () => void) => (): void => {
    setMenu(null);
    action();
  };
  const slot = (key: string): React.JSX.Element | null =>
    menuKey === currentKey
      ? (
        <ShortcutKeycap
          keyLabel={desktopLeaderLabel(`${DESKTOP_TAB_GROUP}${key}`)}
          variant="global"
          sx={{ ml: "1.5rem" }}
        />
      )
      : null;

  return (
    <Box
      ref={strip}
      role="tablist"
      aria-label="Open Sessions and Drafts"
      data-desktop-tabs
      // Every tab is reached by its digit slot or the group slot at the end.
      data-desktop-shortcut-owner
      onWheel={(event): void => {
        if (event.deltaY !== 0 && event.deltaX === 0) {
          event.currentTarget.scrollLeft += event.deltaY;
        }
      }}
      sx={{
        display: "flex",
        alignItems: "stretch",
        flexShrink: 0,
        height: "2.375rem",
        minWidth: 0,
        overflowX: "auto",
        overflowY: "hidden",
        scrollbarWidth: "none",
        "&::-webkit-scrollbar": { display: "none" },
        // A recessed rail: the selected tab rises out of it into the page.
        borderBottom: 1,
        borderColor: "divider",
        bgcolor: (t: Theme) =>
          alpha(t.palette.common.black, t.palette.mode === "dark" ? 0.28 : 0.045),
        ...sx,
      }}
    >
      {items.map((item, index) => {
        const selected = item.key === currentKey;
        return (
          <Box
            key={item.key}
            role="tab"
            aria-selected={selected}
            tabIndex={-1}
            data-desktop-tab={item.key}
            data-desktop-tab-state={item.pinned
              ? "pinned"
              : item.preview
              ? "preview"
              : "kept"}
            title={item.title}
            draggable
            onClick={(): void => {
              if (!selected) onOpen(item.key);
            }}
            onDoubleClick={(): void => actions.keep(item.key)}
            onAuxClick={(event): void => {
              if (event.button !== 1) return;
              event.preventDefault();
              actions.close(item.key, "one");
            }}
            onMouseDown={(event): void => {
              // Middle click closes; it must not start autoscroll.
              if (event.button === 1) event.preventDefault();
            }}
            onContextMenu={(event): void => {
              event.preventDefault();
              setMenu({ key: item.key, left: event.clientX, top: event.clientY });
            }}
            onDragStart={(event): void => {
              event.dataTransfer.setData(TAB_DRAG_TYPE, item.key);
              event.dataTransfer.effectAllowed = "move";
            }}
            onDragOver={(event): void => {
              if (!event.dataTransfer.types.includes(TAB_DRAG_TYPE)) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
              const box = event.currentTarget.getBoundingClientRect();
              const after = event.clientX > box.left + box.width / 2;
              if (drop?.key !== item.key || drop.after !== after) {
                setDrop({ key: item.key, after });
              }
            }}
            onDragLeave={(): void => {
              if (drop?.key === item.key) setDrop(null);
            }}
            onDragEnd={(): void => setDrop(null)}
            onDrop={(event): void => {
              const moved = event.dataTransfer.getData(TAB_DRAG_TYPE);
              const after = drop?.key === item.key && drop.after;
              setDrop(null);
              if (!moved || moved === item.key) return;
              event.preventDefault();
              updateDesktopTabs((state) => {
                const from = state.tabs.findIndex((tab) => tab.key === moved);
                const over = state.tabs.findIndex((tab) => tab.key === item.key);
                if (from === -1 || over === -1) return state;
                const to = (after ? over + 1 : over) - (from < over ? 1 : 0);
                return moveDesktopTab(state, moved, to);
              });
            }}
            sx={{
              position: "relative",
              display: "flex",
              alignItems: "center",
              gap: "0.375rem",
              flex: item.pinned ? "0 0 auto" : "0 1 13rem",
              minWidth: item.pinned ? 0 : "7.5rem",
              maxWidth: item.pinned ? "9rem" : "13rem",
              pl: "0.625rem",
              pr: "0.375rem",
              borderRight: 1,
              borderColor: "divider",
              cursor: "pointer",
              userSelect: "none",
              fontSize: "0.8125rem",
              color: selected ? "text.primary" : "text.secondary",
              transition: "background-color 120ms ease, color 120ms ease",
              "&:hover": { bgcolor: "action.hover", color: "text.primary" },
              // The selected tab is the page's own surface: it covers the
              // rail's bottom edge and carries the accent on top.
              ...(selected && {
                bgcolor: "background.default",
                mb: "-1px",
                fontWeight: 500,
                boxShadow: (t: Theme) =>
                  `inset 0 2px 0 ${t.palette.primary.main}`,
                "&:hover": { bgcolor: "background.default" },
              }),
              "&:hover [data-desktop-tab-close], &[aria-selected='true'] [data-desktop-tab-close]":
                { opacity: 1 },
              ...(drop?.key === item.key && {
                "&::after": {
                  content: '""',
                  position: "absolute",
                  top: "0.25rem",
                  bottom: "0.25rem",
                  width: "2px",
                  bgcolor: "primary.main",
                  ...(drop.after ? { right: "-1px" } : { left: "-1px" }),
                },
              }),
            }}
          >
            {index < 9 && digits && (
              <LeaderKeycap leaderKey={String(index + 1)} />
            )}
            <Box
              component="span"
              sx={{
                display: "inline-flex",
                alignItems: "center",
                gap: "0.25rem",
                flexShrink: 0,
                // The Provider or Draft glyph only: the status dot keeps
                // its own, smaller size.
                "& [data-desktop-tab-icon]": { fontSize: "1rem" },
              }}
            >
              {item.leading}
            </Box>
            <Box
              component="span"
              sx={{
                flex: 1,
                minWidth: 0,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                fontStyle: item.preview ? "italic" : "normal",
              }}
            >
              {item.title}
            </Box>
            <Box
              component="button"
              type="button"
              data-desktop-tab-close
              aria-label={item.pinned
                ? `Unpin ${item.title}`
                : `Close ${item.title}`}
              tabIndex={-1}
              onClick={(event): void => {
                event.stopPropagation();
                if (item.pinned) actions.pin(item.key);
                else actions.close(item.key, "one");
              }}
              onDoubleClick={(event): void => event.stopPropagation()}
              sx={{
                display: "inline-flex",
                alignItems: "center",
                justifyContent: "center",
                flexShrink: 0,
                width: "1.25rem",
                height: "1.25rem",
                p: 0,
                border: 0,
                borderRadius: "0.25rem",
                bgcolor: "transparent",
                color: "inherit",
                cursor: "pointer",
                // A pinned tab always shows its pin; a close button appears
                // on the selected and the hovered tab.
                opacity: item.pinned ? 0.7 : 0,
                "&:hover": { bgcolor: "action.selected", opacity: 1 },
                "& .MuiSvgIcon-root": { fontSize: "0.875rem" },
              }}
            >
              {item.pinned ? <PushPin /> : <Close />}
            </Box>
          </Box>
        );
      })}
      <Box
        component="span"
        title="Tab actions: D close, O others, P pin, U reopen. ␣[ and ␣] step"
        sx={{
          display: "inline-flex",
          alignItems: "center",
          flexShrink: 0,
          px: "0.625rem",
        }}
      >
        <LeaderKeycap leaderKey={DESKTOP_TAB_GROUP} />
      </Box>
      <Menu
        open={menu !== null && menuItem !== undefined}
        onClose={(): void => setMenu(null)}
        anchorReference="anchorPosition"
        {...(menu ? { anchorPosition: { left: menu.left, top: menu.top } } : {})}
        slotProps={{ list: { dense: true } }}
      >
        <MenuItem
          disabled={items.length < 2}
          onClick={run(() => actions.close(menuKey, "one"))}
        >
          <ListItemText>Close</ListItemText>
          {slot(DESKTOP_TAB_KEYS.close)}
        </MenuItem>
        <MenuItem
          disabled={!unpinnedBeside(0, menuIndex) &&
            !unpinnedBeside(menuIndex + 1, items.length)}
          onClick={run(() => actions.close(menuKey, "others"))}
        >
          <ListItemText>Close Others</ListItemText>
          {slot(DESKTOP_TAB_KEYS.closeOthers)}
        </MenuItem>
        <MenuItem
          disabled={!unpinnedBeside(menuIndex + 1, items.length)}
          onClick={run(() => actions.close(menuKey, "right"))}
        >
          <ListItemText>Close to the Right</ListItemText>
          {slot(DESKTOP_TAB_KEYS.closeRight)}
        </MenuItem>
        <MenuItem
          disabled={!unpinnedBeside(0, menuIndex)}
          onClick={run(() => actions.close(menuKey, "left"))}
        >
          <ListItemText>Close to the Left</ListItemText>
          {slot(DESKTOP_TAB_KEYS.closeLeft)}
        </MenuItem>
        <MenuItem
          disabled={!unpinnedBeside(0, items.length) || items.length < 2}
          onClick={run(() => actions.close(menuKey, "all"))}
        >
          <ListItemText>Close All</ListItemText>
          {slot(DESKTOP_TAB_KEYS.closeAll)}
        </MenuItem>
        <Divider />
        <MenuItem onClick={run(() => actions.pin(menuKey))}>
          <ListItemText>{menuItem?.pinned ? "Unpin" : "Pin"}</ListItemText>
          {slot(DESKTOP_TAB_KEYS.pin)}
        </MenuItem>
        {menuItem?.preview && (
          <MenuItem onClick={run(() => actions.keep(menuKey))}>
            <ListItemText>Keep Open</ListItemText>
            {slot(DESKTOP_TAB_KEYS.keep)}
          </MenuItem>
        )}
        <MenuItem disabled={!actions.canReopen()} onClick={run(actions.reopen)}>
          <ListItemText>Reopen Closed Tab</ListItemText>
          <ShortcutKeycap
            keyLabel={desktopLeaderLabel(
              `${DESKTOP_TAB_GROUP}${DESKTOP_TAB_KEYS.reopen}`,
            )}
            variant="global"
            sx={{ ml: "1.5rem" }}
          />
        </MenuItem>
        {menuItem?.kind === "session" && <Divider />}
        {menuItem?.kind === "session" && (
          <MenuItem onClick={run(() => onRename(menuKey))}>
            <ListItemText>Rename…</ListItemText>
          </MenuItem>
        )}
      </Menu>
    </Box>
  );
}
