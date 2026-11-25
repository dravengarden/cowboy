import { desktopSize } from "../../surface/desktopSize";
import {
  alpha,
  Box,
  Divider,
  InputBase,
  Stack,
  Typography,
} from "@mui/material";
import { Keyboard, Search } from "@mui/icons-material";
import { useMemo, useRef, useState } from "react";
import { DESKTOP_INSET_RADIUS } from "../DesktopEmbeddedControl";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import { DesktopKeycap, DesktopShortcut } from "./DesktopKeycap";
import { useDesktopCommands } from "./DesktopCommandProvider";
import { desktopImeOwnsKey } from "./imeShortcut";
import { workspaceCommandKey } from "./workspaceCommandKey";
import {
  DESKTOP_APPROVAL_SHORTCUT,
  DESKTOP_FOCUS_CALLS_SHORTCUT,
  DESKTOP_FOCUS_PLAN_SHORTCUT,
  DESKTOP_FOCUS_PROMPT_SHORTCUT,
  DESKTOP_RESIZE_SELECT_SHORTCUT,
  DESKTOP_DOCUMENT_KEYS,
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_KEYS,
  desktopLeaderGroupSequence,
  desktopWorkspaceSequence,
} from "./workspaceShortcuts";
import { DesktopModal } from "../DesktopModal";

interface ShortcutRow {
  keys: string[];
  title: string;
  description?: string;
}

const NAVIGATION: ShortcutRow[] = [
  { keys: [DESKTOP_SHORTCUTS.focusSessions], title: "Focus Sessions / Sidebar" },
  {
    keys: [DESKTOP_SHORTCUTS.sessionsFold],
    title: "Fold Sessions",
    description:
      "The fold button beside Create: focus the current session's folder path, expand every folder, or scroll back to the current row",
  },
  {
    keys: [
      desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.sessionsNewFolder),
      desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.sessionsMove),
      desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.sessionsOrganize),
    ],
    title: "New folder / Move pick / Organize by project",
    description: "Move pick darkens the page and letters every folder; press one to file the focused row (else the current item) there, Esc to leave. M on a row starts it too",
  },
  {
    keys: [DESKTOP_FOCUS_PROMPT_SHORTCUT],
    title: "Focus Message the Agent",
    description: "Keeps its Vim mode and caret",
  },
  {
    keys: [DESKTOP_SHORTCUTS.insertPrompt],
    title: "Write to the agent: the Prompt in Insert at its caret",
  },
  {
    keys: [DESKTOP_SHORTCUTS.compact, DESKTOP_SHORTCUTS.clear],
    title: "Compact / Clear the conversation",
    description:
      "While the Prompt owns focus, its editor included; both confirm first",
  },
  { keys: [DESKTOP_FOCUS_PLAN_SHORTCUT], title: "Focus Plan" },
  { keys: [DESKTOP_FOCUS_CALLS_SHORTCUT], title: "Open the running call, or open or close Calls" },
  { keys: [DESKTOP_SHORTCUTS.focusConversation], title: "Focus Conversation Log" },
  { keys: [DESKTOP_SHORTCUTS.focusTopbar], title: "Focus Top Bar" },
  {
    keys: [DESKTOP_SHORTCUTS.newSession],
    title: "Create a new Session in the current directory",
    description: "The Sessions cursor row's directory, otherwise the open session's. The form shows it and can change it",
  },
  { keys: [DESKTOP_SHORTCUTS.settings], title: "Open Settings" },
  { keys: [DESKTOP_SHORTCUTS.extensions], title: "Extensions" },
  {
    keys: [DESKTOP_SHORTCUTS.toggleSourceMode],
    title: "Toggle composer Source mode",
    description:
      "Edit the literal markdown instead of the live preview. Obsidian's Mod+E is reserved by Chrome and macOS, so Source mode uses the workspace prefix",
  },
  // The Top bar group (FOCUS.md "Leader"): `␣T` then the key runs the
  // control from any focus; the bare letter works while the bar owns focus.
  ...([
    ["R", "Open Run Configuration"],
    ["U", "Open Usage Limits"],
    ["A", "Verify Product Session (while it is requested)"],
    ["N", "Update Provider (while a newer release waits)"],
    ["L", "Reload Session Runtime"],
  ] as const).map(([key, title], index): ShortcutRow => ({
    keys: [desktopLeaderGroupSequence(DESKTOP_WORKSPACE_KEYS.focusTopbar, key), key],
    title: `${title} in Top Bar`,
    ...(index === 0
      ? {
        description:
          "The Top bar group runs from any focus; the bare letter works while the Top Bar owns focus",
      }
      : {}),
  })),
  {
    keys: [DESKTOP_SHORTCUTS.focusQueue, DESKTOP_SHORTCUTS.focusDrafts],
    title: "Queue / Drafts inside Prompt",
    description:
      `␣Q lands on your just-queued prompt (press I to edit in the Prompt canvas). ␣D opens Drafts even when empty (press N for a new draft). ${DESKTOP_FOCUS_PROMPT_SHORTCUT} returns to the active buffer; ${DESKTOP_SHORTCUTS.insertPrompt} returns to New Prompt in Insert`,
  },
  {
    keys: ["Ctrl+H/J/K/L"],
    title: "Move to the region left / below / above / right",
    description: "By on-screen position, as LazyVim's window keys. Works from Vim Normal, Insert, and every list, reader, bar, and dialog; only a pending Vim command keeps its next key",
  },
  { keys: [DESKTOP_SHORTCUTS.cycleRegion], title: "Cycle visible workspace regions" },
  {
    keys: [
      desktopWorkspaceSequence([
        DESKTOP_WORKSPACE_KEYS.toggleSessions,
        DESKTOP_WORKSPACE_KEYS.togglePrompt,
        DESKTOP_WORKSPACE_KEYS.toggleConversation,
      ].join(" ")),
    ],
    title: "Collapse / expand Sessions, Prompt, Conversation",
    description:
      "The three keys sit left to right like the panes. Prompt and Conversation never both collapse: hiding the last one swaps them. Jumping to a collapsed pane expands it; dragging a splitter past a pane's minimum collapses it",
  },
  {
    keys: [DESKTOP_RESIZE_SELECT_SHORTCUT],
    title: "Select the nearest layout resize bar",
    description:
      "Enter Resize mode without moving the bar. H/L then resizes, Shift+H/L uses a larger step, Tab selects the next visible bar, and Esc or Enter finishes",
  },
  { keys: ["J/K"], title: "Move through items in list regions" },
  { keys: ["Shift+J/K"], title: "Reorder the focused item when supported" },
  { keys: ["O"], title: "Toggle continuous Order mode for list reordering" },
  {
    keys: ["gg"],
    title: "First item",
    description: "A pending g takes the next key: any other key cancels it and does nothing else",
  },
  { keys: ["Ctrl+D/U", "Ctrl+F/B"], title: "Half page / page down and up in a list, cursor and view together" },
  { keys: ["zz", "zt", "zb"], title: "Scroll the cursor row to the centre / top / bottom" },
  { keys: ["za", "zM", "zR"], title: "Sessions: fold button, close every folder, open every folder" },
  { keys: ["A"], title: "Sessions: new session in the cursor row's directory" },
  {
    keys: ["R", "M", "X", ","],
    title: "Sessions: rename, move, move to Trash, open settings of the cursor row",
    description: "The same letters as the row's actions modal (S). An idle session goes to Trash with Undo; a working one or a folder asks first. On a folder `,` opens its directory defaults",
  },
  { keys: ["Shift+G"], title: "Last item" },
  {
    keys: ["'"],
    title: "Label every visible row of the focused list, then press a label to jump",
    description: "Works in Sessions, the collapsed Sessions rail, Queue and Drafts. Labels start on the home row and disappear after the jump",
  },
  {
    keys: ["Space"],
    title: "Leader: which-key for every command in the current focus",
    description: "Cmd/Alt+K opens the same leader from a text field. Inside a dialog it labels every control of that dialog",
  },
  { keys: ["Enter"], title: "Open or activate the focused item" },
  { keys: ["I"], title: "Edit the focused item" },
  { keys: ["Esc"], title: "Leave the current Vim mode or close the active layer" },
];

const DISCOVERY: ShortcutRow[] = [
  { keys: [DESKTOP_SHORTCUTS.shortcuts], title: "Open this shortcut guide" },
  { keys: [DESKTOP_SHORTCUTS.commands], title: "Open Command Palette" },
];

const DIRECT_ACTIONS: ShortcutRow[] = [
  {
    keys: ["Mod+Enter"],
    title: "Send or deliver prompt, or Use Draft in Session…",
    description:
      "In New Prompt or Session Draft: send or open delivery picker. In a Queue edit: save and open delivery picker (Steer / Queue first) without blind-sending. In an independent Draft: choose a Session to open it in its Prompt canvas",
  },
  {
    keys: [DESKTOP_SHORTCUTS.saveDraft],
    title: "Save without sending",
    description:
      "In New Prompt: save as a Session draft. In a Queue edit: save changes, close editor, and release hold. In a Session Draft or independent Draft: save in place",
  },
  { keys: ["Alt+Enter"], title: "Choose delivery mode (Steer / Queue first / Send & pause)" },
  {
    keys: ["↑ / K", "[q / ]q", "ZZ / ZQ"],
    title: "Prompt workbench editing motions",
    description:
      "↑ (or Vim Normal k) on an empty New Prompt recalls the just-queued message into Insert. [q / ]q walk send order across Queue items and New Prompt. ZZ saves and closes; ZQ discards changes",
  },
  { keys: [DESKTOP_SHORTCUTS.stop], title: "Stop the current turn" },
  {
    keys: [DESKTOP_SHORTCUTS.reconnect],
    title: "Reconnect now",
    description: "Only while the status line shows a Retry control",
  },
  {
    keys: ["Mod+R"],
    title: "Reload into a ready new version",
    description: "While the update bar shows Reload (or Retry, Cancel). Otherwise the browser reloads as usual",
  },
  {
    keys: [DESKTOP_SHORTCUTS.switchSession],
    title: "Switch session: every session gets a letter label, press it to open",
  },
  {
    keys: [DESKTOP_SHORTCUTS.alternateSession],
    title: "Return to the previous Session or Draft (Alt-Tab)",
  },
  {
    keys: [DESKTOP_SHORTCUTS.recentSessions],
    title: "Recent: the Sessions and Drafts you opened, newest first",
    description: "1–9 open a row at once; J/K move, L or Enter open, H or Esc close. It opens on the previous item",
  },
  {
    keys: [DESKTOP_SHORTCUTS.undoHistory],
    title: "Undo history: changes from the last 24 hours you can still undo",
  },
  {
    keys: [DESKTOP_SHORTCUTS.rename],
    title: "Rename the current Session",
  },
  {
    keys: [
      DESKTOP_SHORTCUTS.previousTab,
      DESKTOP_SHORTCUTS.nextTab,
      desktopWorkspaceSequence("1"),
    ],
    title: "Previous / next tab; a digit opens that tab",
    description: "While an approval waits, the digits answer it instead",
  },
  {
    keys: [desktopWorkspaceSequence("B")],
    title: "Tabs: D close, O others, R right, L left, A all",
    description: "P pin, K keep the preview tab, U reopen the last closed, < > move",
  },
    {
      keys: [
        desktopWorkspaceSequence(DESKTOP_DOCUMENT_KEYS.title),
        desktopWorkspaceSequence(DESKTOP_DOCUMENT_KEYS.copy),
        desktopWorkspaceSequence(DESKTOP_DOCUMENT_KEYS.history),
        desktopWorkspaceSequence(DESKTOP_DOCUMENT_KEYS.export),
        desktopWorkspaceSequence(DESKTOP_DOCUMENT_KEYS.readableWidth),
      ],
      title: "On a Draft: rename title, copy to a Session, history, export, readable width",
      description:
        "␣R focuses the title (matching Session Rename); Mod+Enter copies to a Session and opens it in its Prompt canvas; H is Schedule in a Session and History in a Draft",
    },
];

const CONVERSATION: ShortcutRow[] = [
  { keys: ["J/K"], title: "Scroll down / up by one reading line" },
  { keys: ["Ctrl+D/U"], title: "Scroll down / up by half a page" },
  { keys: ["Ctrl+F/B"], title: "Scroll down / up by a page" },
  { keys: ["gg"], title: "Jump to the oldest loaded message" },
  { keys: ["Shift+G"], title: "Jump to latest and enable Following" },
  { keys: ["F"], title: "Toggle automatic Following" },
  { keys: ["V"], title: "Switch the Conversation between History and Explore" },
  { keys: ["Tab / Shift+Tab"], title: "Select the next / previous expandable widget" },
  { keys: ["H/L"], title: "Close / open the selected widget" },
  { keys: ["Enter"], title: "Toggle the selected widget" },
  {
    keys: ["P"],
    title: "Explore: open or close the Page Index",
    description: "J/K move, L or Enter opens, / searches, H or Esc closes",
  },
  { keys: ["[", "]"], title: "Explore: previous / next page" },
  { keys: ["N"], title: "Explore: new page, focusing the Prompt editor" },
  {
    keys: [DESKTOP_APPROVAL_SHORTCUT],
    title: "Answer a pending approval with its first choice (1–9 pick the others)",
    description: "Choices are numbered in the Provider's order; works from any focus",
  },
];

const CODE_REVIEW: ShortcutRow[] = [
  {
    keys: [desktopWorkspaceSequence(DESKTOP_WORKSPACE_KEYS.review)],
    title: "Open Code Review (or open the Review group once shown)",
    description: "While Review is shown, ␣VV focuses it",
  },
  {
    keys: [
      desktopLeaderGroupSequence("V", "C"),
      desktopLeaderGroupSequence("V", "F"),
      desktopLeaderGroupSequence("V", "H"),
      desktopLeaderGroupSequence("V", "W"),
      desktopLeaderGroupSequence("V", "G"),
    ],
    title: "Navigator: Changes / Files / History / Worktrees / Pull request",
    description: "Inside Navigator: [ / ] switch tabs, / filters, L or Enter opens, H collapses",
  },
  {
    keys: [
      desktopLeaderGroupSequence("V", "X"),
      desktopLeaderGroupSequence("V", "A"),
      desktopLeaderGroupSequence("V", "U"),
    ],
    title: "Choose context / Compare against / Only unreviewed files",
  },
  {
    keys: [
      desktopLeaderGroupSequence("V", "M"),
      desktopLeaderGroupSequence("V", "D"),
      desktopLeaderGroupSequence("V", "Z"),
    ],
    title: "Mark reviewed / Unified or split diff / Whole file or patch context",
  },
  {
    keys: [
      desktopLeaderGroupSequence("V", "["),
      desktopLeaderGroupSequence("V", "]"),
      desktopLeaderGroupSequence("V", "O"),
      desktopLeaderGroupSequence("V", "K"),
    ],
    title: "Fold Navigator / Fold Inspector / Outline / Run project check",
  },
  {
    keys: [
      desktopLeaderGroupSequence("V", "Q"),
      desktopLeaderGroupSequence("V", "P"),
      "gt / gT",
    ],
    title: "Close file tab / Keep preview tab / Next or previous file tab",
  },
  {
    keys: [
      desktopLeaderGroupSequence("V", "S"),
      desktopLeaderGroupSequence("V", "E"),
      desktopLeaderGroupSequence("V", "R"),
      "Enter",
    ],
    title: "Send to Prompt / Explain in a fork / Review in a fork",
  },
  {
    keys: ["]c / [c", "]f / [f", "]u / [u", "]d / [d"],
    title: "Document: next / previous hunk, file, unreviewed file, or problem",
  },
  {
    keys: ["gd / gD / gy / gI / gr", "K", "gO"],
    title: "Document: language navigation, symbol hover, and outline",
  },
];

function filterShortcutRows(
  sectionTitle: string,
  rows: readonly ShortcutRow[],
  tokens: readonly string[],
): ShortcutRow[] {
  if (tokens.length === 0) return [...rows];
  const section = sectionTitle.toLowerCase();
  if (tokens.every((token) => section.includes(token))) return [...rows];
  return rows.filter((row) => {
    const haystack = `${section} ${row.keys.join(" ")} ${row.title} ${row.description ?? ""}`.toLowerCase();
    return tokens.every((token) => haystack.includes(token));
  });
}

function KeySequence({ keys }: { keys: string[] }): React.JSX.Element {
  return (
    <Stack
      data-shortcut-reference
      direction="row"
      spacing={0.4}
      alignItems="center"
      flexWrap="wrap"
      useFlexGap
    >
      {keys.map((key, index) =>
        key.includes("+")
          ? <DesktopShortcut key={`${key}-${String(index)}`} shortcut={key} />
          : <DesktopKeycap key={`${key}-${String(index)}`} keyLabel={key} />
      )}
    </Stack>
  );
}

function ShortcutSection({
  title,
  rows,
  active = false,
}: {
  title: string;
  rows: ShortcutRow[];
  active?: boolean;
}): React.JSX.Element {
  return (
    <Box
      sx={{
        border: 1,
        borderColor: (theme) => alpha(
          active ? theme.palette.primary.main : theme.palette.divider,
          active ? 0.34 : 0.72,
        ),
        borderRadius: 2,
        bgcolor: (theme) => alpha(
          active ? theme.palette.primary.main : theme.palette.background.default,
          active ? 0.055 : 0.28,
        ),
        overflow: "hidden",
      }}
    >
      <Typography
        variant="overline"
        color={active ? "primary.main" : "text.secondary"}
        sx={{ display: "block", px: 1.5, py: 1, fontWeight: 750, lineHeight: 1 }}
      >
        {title}
      </Typography>
      <Divider />
      {rows.map((row, index) => (
        <Box
          key={`${row.title}-${String(index)}`}
          sx={{
            minHeight: desktopSize(46),
            px: 1.5,
            py: 0.8,
            display: "grid",
            gridTemplateColumns: "minmax(118px, auto) 1fr",
            alignItems: "center",
            gap: 1.5,
            borderTop: index === 0 ? 0 : 1,
            borderColor: "divider",
          }}
        >
          <KeySequence keys={row.keys} />
          <Box sx={{ minWidth: 0 }}>
            <Typography variant="body2" fontWeight={650}>{row.title}</Typography>
            {row.description && (
              <Typography variant="caption" color="text.secondary">
                {row.description}
              </Typography>
            )}
          </Box>
        </Box>
      ))}
    </Box>
  );
}

export function DesktopShortcutsDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}): React.JSX.Element | null {
  if (!open) return null;
  return <DesktopShortcutsDialogContent onClose={onClose} />;
}

function ShortcutsFilterBar({
  inputRef,
  query,
  onChange,
}: {
  inputRef: React.RefObject<HTMLInputElement | null>;
  query: string;
  onChange: (value: string) => void;
}): React.JSX.Element {
  return (
    <Box sx={{ px: 1.5, pt: 1.25 }}>
      <Box
        sx={{
          minHeight: desktopSize(38),
          px: 1.25,
          display: "flex",
          alignItems: "center",
          gap: 1,
          border: 1,
          borderColor: (theme) => alpha(theme.palette.divider, 0.9),
          borderRadius: `${DESKTOP_INSET_RADIUS}px`,
          bgcolor: (theme) => alpha(theme.palette.background.paper, 0.52),
          "&:focus-within": {
            borderColor: "primary.main",
            bgcolor: "background.paper",
          },
        }}
      >
        <Search aria-hidden sx={{ fontSize: "1.05rem", color: "text.secondary" }} />
        <InputBase
          inputRef={inputRef}
          fullWidth
          value={query}
          onChange={(event): void => onChange(event.target.value)}
          onKeyDown={(event): void => {
            if (desktopImeOwnsKey(event.nativeEvent)) return;
            if (event.key === "Escape" && query.length > 0) {
              event.preventDefault();
              event.stopPropagation();
              onChange("");
            }
          }}
          placeholder="Filter shortcuts… (press / to focus)"
          inputProps={{
            "aria-label": "Filter keyboard shortcuts",
            "data-desktop-escape": "close",
            "data-desktop-vim": "off",
          }}
          sx={{ fontSize: "0.85rem" }}
        />
      </Box>
    </Box>
  );
}

function handleShortcutsDialogKeyDown(
  event: React.KeyboardEvent<HTMLDivElement>,
  searchRef: React.RefObject<HTMLInputElement | null>,
  bodyRef: React.RefObject<HTMLDivElement | null>,
  pendingGo: React.MutableRefObject<number>,
): void {
  if (document.activeElement === searchRef.current) return;
  const native = event.nativeEvent;
  if (desktopImeOwnsKey(native) || event.metaKey || event.altKey) return;
  const key = workspaceCommandKey(native);
  const scroller = bodyRef.current?.parentElement ?? null;
  if (event.ctrlKey && !event.shiftKey) {
    const lower = key.toLowerCase();
    if (scroller && ["d", "u", "f", "b"].includes(lower)) {
      const dir = lower === "d" || lower === "f" ? 1 : -1;
      const share = lower === "d" || lower === "u" ? 0.5 : 0.9;
      scroller.scrollBy({ top: dir * scroller.clientHeight * share });
      event.preventDefault();
      event.stopPropagation();
    }
    pendingGo.current = 0;
    return;
  }
  if (event.ctrlKey) return;
  if (key === "/") {
    pendingGo.current = 0;
    event.preventDefault();
    event.stopPropagation();
    searchRef.current?.focus();
    return;
  }
  if (!scroller) return;
  if (key === "g" && !event.shiftKey) {
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat) return;
    const now = Date.now();
    if (pendingGo.current && now - pendingGo.current <= 1200) {
      pendingGo.current = 0;
      scroller.scrollTo({ top: 0 });
    } else {
      pendingGo.current = now;
    }
    return;
  }
  pendingGo.current = 0;
  if (key === "G") {
    scroller.scrollTo({ top: scroller.scrollHeight });
    event.preventDefault();
    event.stopPropagation();
  } else if (key === "j" || key === "k") {
    scroller.scrollBy({ top: key === "j" ? 64 : -64 });
    event.preventDefault();
    event.stopPropagation();
  }
}

function DesktopShortcutsDialogContent({
  onClose,
}: {
  onClose: () => void;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const registry = useDesktopCommands();
  const [query, setQuery] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const pendingGo = useRef(0);
  const paneCommands = useMemo<ShortcutRow[]>(() =>
    registry.commands
      .filter((command) =>
        (command.contexts || command.regions) &&
        (!command.contexts || command.contexts.includes(workspace.focusedPane)) &&
        (!command.regions ||
          (!!workspace.focusedRegion && command.regions.includes(workspace.focusedRegion))) &&
        Boolean(command.shortcut || command.sequence?.length)
      )
      .map((command) => ({
        keys: [command.shortcut ?? command.sequence!.join(" → ")],
        title: command.title,
        ...(command.description ? { description: command.description } : {}),
      })), [registry.commands, workspace.focusedPane, workspace.focusedRegion]);

  const tokens = useMemo(
    () => query.trim().toLowerCase().split(/\s+/).filter(Boolean),
    [query],
  );
  const sections = useMemo(() => {
    const all: { title: string; rows: ShortcutRow[]; active?: boolean }[] = [
      { title: "Workspace navigation", rows: NAVIGATION, active: true },
      { title: "Discovery and commands", rows: DISCOVERY },
      { title: "Direct actions", rows: DIRECT_ACTIONS },
      // Reference material stays readable from every focus; it lights
      // up while the reader owns the keys.
      {
        title: "Conversation reading",
        rows: CONVERSATION,
        active: workspace.focusedRegion === "conversation.transcript",
      },
      {
        title: "Code Review",
        rows: CODE_REVIEW,
        active: workspace.focusedRegion?.startsWith("review.") === true,
      },
      ...(paneCommands.length > 0
        ? [{
          title: `${workspace.focusedRegion ?? workspace.focusedPane} commands`,
          rows: paneCommands,
          active: true,
        }]
        : []),
    ];
    return all
      .map((section) => ({
        ...section,
        rows: filterShortcutRows(section.title, section.rows, tokens),
      }))
      .filter((section) => section.rows.length > 0);
  }, [paneCommands, tokens, workspace.focusedPane, workspace.focusedRegion]);

  return (
    <DesktopModal
      open
      onClose={onClose}
      title="Keyboard shortcuts"
      description={`Vim-first navigation · ${workspace.focusedRegion ?? workspace.focusedPane}`}
      icon={<Keyboard color="primary" />}
      width={920}
      onShortcutKeyDown={(event): void =>
        handleShortcutsDialogKeyDown(event, searchRef, bodyRef, pendingGo)}
      shortcutGroups={[
        {
          slots: [{
            shortcut: DESKTOP_SHORTCUTS.shortcuts,
            label: "Guide",
            availability: "active",
          }],
        },
        {
          label: "Scroll",
          slots: [
            { shortcut: "J/K", label: "Line" },
            { shortcut: "Ctrl+D/U", label: "Half" },
            { shortcut: "gg/G", label: "Top · Bottom" },
          ],
        },
        { slots: [{ shortcut: "/", label: "Search" }, { shortcut: "Esc", label: "Close" }] },
      ]}
    >
      <Box ref={bodyRef} data-desktop-modal-keys="own">
      <ShortcutsFilterBar inputRef={searchRef} query={query} onChange={setQuery} />
      <Box
        sx={{
          p: 1.5,
          display: "grid",
          gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
          gap: 1.25,
          "@media (max-width: 760px)": { gridTemplateColumns: "1fr" },
        }}
      >
        {sections.length === 0 && (
          <Typography
            color="text.secondary"
            sx={{ gridColumn: "1 / -1", px: 1.5, py: 3, textAlign: "center" }}
          >
            No shortcuts match “{query.trim()}”.
          </Typography>
        )}
        {sections.map((section) => (
          <ShortcutSection
            key={section.title}
            title={section.title}
            rows={section.rows}
            active={section.active === true}
          />
        ))}
      </Box>
      </Box>
    </DesktopModal>
  );
}
