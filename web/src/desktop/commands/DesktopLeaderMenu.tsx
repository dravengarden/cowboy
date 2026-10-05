import { useEffect, useState } from "react";
import { alpha, Box, ButtonBase, Stack, Typography } from "@mui/material";
import { ShortcutKeycap } from "../../ShortcutKeycap";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import {
  type DesktopCommand,
  desktopCommandInScope,
  desktopLeaderGroupAvailable,
  desktopLeaderGroupCommands,
  useDesktopCommands,
} from "./DesktopCommandProvider";
import { useDesktopHints, useDesktopLeaderOptional } from "./leaderContext";
import { activateHint } from "./hintTargets";
import {
  DESKTOP_SESSION_JUMP_EVENT,
  useSessionJumpTargets,
} from "./sessionJump";
import {
  DESKTOP_LEADER_GLYPH,
  DESKTOP_LEADER_GROUPS,
  desktopLeaderGroupKey,
  desktopLeaderKey,
  desktopLeaderLabel,
} from "./workspaceShortcuts";

/** which-key waits this long before painting, so a fluent `␣N` never flashes
 *  a panel; the on-screen slots light at once regardless. */
export const DESKTOP_LEADER_MENU_DELAY_MS = 180;

interface LeaderEntry {
  key: string;
  command: DesktopCommand;
  enabled: boolean;
}

function leaderSortKey(key: string): string {
  // Letters first in alphabetical order, then punctuation, Space last.
  if (key === " ") return "~~";
  return /^[a-z]$/.test(key) ? `a${key}` : `b${key}`;
}

/**
 * The which-key panel (FOCUS.md "Leader"). It is a live legend of the armed
 * leader: every entry is a key that runs now in the current focus, grouped
 * by command group with the focused surface's own actions first. Entries are
 * also clickable, so the panel doubles as a pointer menu.
 */
export function DesktopLeaderMenu(): React.JSX.Element | null {
  const leader = useDesktopLeaderOptional();
  const registry = useDesktopCommands();
  const workspace = useDesktopWorkspace();
  const sessions = useSessionJumpTargets();
  const hints = useDesktopHints();
  const armed = leader?.armed === true;
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    if (!armed) {
      setVisible(false);
      return undefined;
    }
    const timer = globalThis.setTimeout(
      () => setVisible(true),
      DESKTOP_LEADER_MENU_DELAY_MS,
    );
    return () => globalThis.clearTimeout(timer);
  }, [armed]);
  if (!leader || !armed || !visible) return null;

  const run = (command: DesktopCommand): void => {
    leader.close();
    command.run();
  };
  const sessionsLayer = leader.layer === "sessions";
  const modalLayer = leader.layer === "modal";
  const groupKey = leader.layer.startsWith("group:")
    ? leader.layer.slice("group:".length)
    : null;
  const entries: LeaderEntry[] = [];
  if (groupKey !== null) {
    for (
      const command of desktopLeaderGroupCommands(
        registry.commands,
        groupKey,
        workspace.focusedPane,
      )
    ) {
      const path = desktopLeaderGroupKey(command)!;
      // Listed under the group name, not "Here": the group is the scope.
      const { regions: _regions, ...unscoped } = command;
      entries.push({
        key: path.key,
        command: { ...unscoped, group: DESKTOP_LEADER_GROUPS[groupKey] ?? command.group },
        enabled: command.when?.() !== false,
      });
    }
  } else if (!sessionsLayer && !modalLayer) {
    // Groups are one entry each; their commands appear one layer down. An
    // available group takes its key over the root command of the same key.
    for (const [key, name] of Object.entries(DESKTOP_LEADER_GROUPS)) {
      if (
        !desktopLeaderGroupAvailable(
          registry.commands,
          key,
          workspace.focusedPane,
        )
      ) continue;
      entries.push({
        key,
        command: {
          id: `group.${key}`,
          title: `${name} …`,
          group: "Groups",
          run: () => leader.open(`group:${key}`),
        },
        enabled: true,
      });
    }
    const seen = new Set<string>(entries.map((entry) => entry.key));
    for (const command of registry.commands) {
      const key = desktopLeaderKey(command);
      if (key === null || seen.has(key)) continue;
      if (
        !desktopCommandInScope(
          command,
          workspace.focusedPane,
          workspace.focusedRegion,
        )
      ) continue;
      seen.add(key);
      entries.push({ key, command, enabled: command.when?.() !== false });
    }
  }
  const scoped = entries.filter((entry) => entry.command.regions);
  const groups = new Map<string, LeaderEntry[]>();
  if (scoped.length > 0) groups.set("Here", scoped);
  for (const entry of entries) {
    if (entry.command.regions) continue;
    const group = groups.get(entry.command.group) ?? [];
    group.push(entry);
    groups.set(entry.command.group, group);
  }
  for (const group of groups.values()) {
    group.sort((left, right) =>
      leaderSortKey(left.key).localeCompare(leaderSortKey(right.key))
    );
  }

  return (
    <Box
      data-desktop-leader-menu={leader.layer}
      role="group"
      aria-label={sessionsLayer ? "Switch session" : modalLayer ? "Dialog keys" : "Leader keys"}
      sx={{
        position: "fixed",
        right: 16,
        bottom: 44,
        zIndex: (theme) => theme.zIndex.modal + 2,
        width: sessionsLayer || modalLayer
          ? "min(26rem, calc(100vw - 32px))"
          : "min(72rem, calc(100vw - 32px))",
        maxHeight: "min(50vh, 30rem)",
        overflow: "auto",
        p: 1.25,
        borderRadius: 2,
        border: 1,
        borderColor: "divider",
        bgcolor: (theme) => alpha(theme.palette.background.paper, 0.96),
        backdropFilter: "blur(16px)",
        boxShadow: 8,
        animation: "cowboyLeaderMenuIn 120ms ease-out",
        "@keyframes cowboyLeaderMenuIn": {
          from: { opacity: 0, transform: "translateY(6px)" },
          to: { opacity: 1, transform: "none" },
        },
        "@media (prefers-reduced-motion: reduce)": { animation: "none" },
      }}
    >
      <Stack direction="row" alignItems="center" spacing={1} sx={{ mb: 1, px: 0.5 }}>
        <ShortcutKeycap
          keyLabel={sessionsLayer
            ? desktopLeaderLabel(" ")
            : groupKey !== null
            ? desktopLeaderLabel(groupKey)
            : DESKTOP_LEADER_GLYPH}
          availability="active"
          accent
        />
        <Typography variant="subtitle2" fontWeight={750}>
          {sessionsLayer
            ? "Switch session"
            : modalLayer
            ? "This dialog"
            : groupKey !== null
            ? DESKTOP_LEADER_GROUPS[groupKey] ?? "Group"
            : "Leader"}
        </Typography>
        <Box sx={{ flex: 1 }} />
        <Typography variant="caption" color="text.secondary">
          {sessionsLayer || groupKey !== null ? "⌫ back · Esc close" : "Esc close"}
        </Typography>
      </Stack>
      {modalLayer
        ? (
          <Box
            sx={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fill, minmax(11rem, 1fr))",
              columnGap: 1,
            }}
          >
            {hints.map((hint) => (
              <ButtonBase
                key={`${hint.label}:${hint.name}`}
                data-modal-leader-entry={hint.label}
                onClick={() => {
                  leader.close();
                  activateHint(hint.element);
                }}
                sx={entrySx(false)}
              >
                <ShortcutKeycap keyLabel={hint.label} availability="active" accent />
                <Typography variant="body2" noWrap sx={{ flex: 1, minWidth: 0, textAlign: "left" }}>
                  {hint.name}
                </Typography>
              </ButtonBase>
            ))}
          </Box>
        )
        : sessionsLayer
        ? (
          <Stack spacing={0.25}>
            {sessions.length === 0 && (
              <Typography variant="body2" color="text.secondary" sx={{ px: 1, py: 0.5 }}>
                No sessions yet.
              </Typography>
            )}
            {sessions.map((target) => (
              <ButtonBase
                key={target.id}
                data-session-jump-entry={target.label}
                onClick={() => {
                  leader.close();
                  document.querySelector<HTMLElement>(
                    "[data-desktop-region='sessions.list'] ul",
                  )?.dispatchEvent(
                    new CustomEvent(DESKTOP_SESSION_JUMP_EVENT, {
                      cancelable: true,
                      detail: { label: target.label },
                    }),
                  );
                }}
                sx={entrySx(target.current)}
              >
                <ShortcutKeycap keyLabel={target.label} availability="active" accent />
                <Box sx={{ minWidth: 0, flex: 1, textAlign: "left" }}>
                  <Typography variant="body2" noWrap fontWeight={target.current ? 700 : 500}>
                    {target.title}
                  </Typography>
                  {target.detail && (
                    <Typography variant="caption" color="text.secondary" noWrap component="div">
                      {target.detail}
                    </Typography>
                  )}
                </Box>
                {target.current && (
                  <Typography variant="caption" color="primary.main">open</Typography>
                )}
              </ButtonBase>
            ))}
          </Stack>
        )
        : (
          <Box
            sx={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fill, minmax(13rem, 1fr))",
              columnGap: 2,
              rowGap: 0.75,
            }}
          >
            {[...groups.entries()].map(([group, list]) => (
              <Box key={group}>
                <Typography
                  variant="overline"
                  color={group === "Here" ? "primary.main" : "text.secondary"}
                  sx={{ px: 0.5, lineHeight: 1.6, display: "block" }}
                >
                  {group}
                </Typography>
                {list.map((entry) => (
                  <ButtonBase
                    key={entry.key}
                    data-leader-entry={entry.key}
                    disabled={!entry.enabled}
                    onClick={() => run(entry.command)}
                    title={entry.command.description}
                    sx={entrySx(false)}
                  >
                    <ShortcutKeycap
                      keyLabel={entry.key === " " ? DESKTOP_LEADER_GLYPH : entry.key.toUpperCase()}
                      availability={entry.enabled ? "active" : "inactive"}
                      accent={entry.enabled}
                    />
                    <Typography
                      variant="body2"
                      noWrap
                      color={entry.enabled ? "text.primary" : "text.disabled"}
                      sx={{ flex: 1, minWidth: 0, textAlign: "left" }}
                    >
                      {entry.command.title}
                    </Typography>
                  </ButtonBase>
                ))}
              </Box>
            ))}
          </Box>
        )}
    </Box>
  );
}

function entrySx(current: boolean): object {
  return {
    width: "100%",
    display: "flex",
    alignItems: "center",
    gap: 1,
    px: 0.75,
    py: 0.2,
    borderRadius: 1,
    justifyContent: "flex-start",
    bgcolor: current ? "action.selected" : "transparent",
    "&:hover": { bgcolor: "action.hover" },
  };
}
