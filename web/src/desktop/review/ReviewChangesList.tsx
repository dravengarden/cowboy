import {
  CheckCircle,
  ExpandMore,
  Refresh,
} from "@mui/icons-material";
import {
  Box,
  Button,
  Divider,
  IconButton,
  List,
  ListItemButton,
  ListItemText,
  ListSubheader,
  Menu,
  MenuItem,
  ToggleButton,
  Tooltip,
  Typography,
} from "@mui/material";
import { useEffect, useMemo, useRef, useState } from "react";
import { desktopSize } from "../../surface/desktopSize";
import { LeaderKeycap } from "../commands/DesktopKeycap";
import { DESKTOP_SURFACE_RADIUS } from "../DesktopEmbeddedControl";
import type { CodeChange } from "../../review/codeApi";
import { reviewEntryKey } from "../../review/diffNavigationModel";
import type { GitReviewEntry } from "../../review/gitReviewModel";
import {
  reviewCompareBranches,
  reviewCompareLabel,
  reviewCompareScenarios,
} from "../../review/reviewCompareModel";
import type { DesktopReviewChanges } from "./useDesktopReviewChanges";
import { findingBadge, type FindingSeverity, type ReviewFinding } from "./reviewFindingsModel";

const STATUS: Record<CodeChange["status"], { letter: string; color: string }> = {
  modified: { letter: "M", color: "warning.main" },
  added: { letter: "A", color: "success.main" },
  untracked: { letter: "U", color: "success.main" },
  deleted: { letter: "D", color: "error.main" },
  renamed: { letter: "R", color: "info.main" },
  conflicted: { letter: "C", color: "error.main" },
};

/** Rows mounted at first; more are added on demand so a 1,000-file change
 *  does not mount 1,000 rows (13-code-review "Git review"). */
const WINDOW = 200;

export function entryKey(entry: GitReviewEntry): string {
  return reviewEntryKey(entry.change.path, entry.scope, entry.comparison);
}

const BADGE_COLOR = { critical: "error.main", high: "error.main", medium: "warning.main", low: "info.main" } as const;

function ChangeRow({
  entry,
  open,
  reviewed,
  onOpen,
  badge,
}: {
  entry: GitReviewEntry;
  open: boolean;
  reviewed: boolean;
  onOpen: (entry: GitReviewEntry) => void;
  /** Current AI review findings in this file: a read-out, not a control. */
  badge: { count: number; severity: FindingSeverity } | undefined;
}): React.JSX.Element {
  const { change } = entry;
  const slash = change.path.lastIndexOf("/");
  const name = change.path.slice(slash + 1);
  const directory = slash > 0 ? change.path.slice(0, slash) : "";
  const status = STATUS[change.status];
  return (
    <ListItemButton
      data-desktop-item={entryKey(entry)}
      data-review-change={change.path}
      data-review-reviewed={reviewed ? "true" : undefined}
      selected={open}
      onClick={() => onOpen(entry)}
      title={change.oldPath ? `${change.oldPath} → ${change.path}` : change.path}
      sx={{
        py: 0.25,
        px: 1,
        gap: 1,
        minHeight: desktopSize(30),
        borderRadius: `${String(DESKTOP_SURFACE_RADIUS)}px`,
        opacity: reviewed ? 0.62 : 1,
      }}
    >
      <Typography
        component="span"
        sx={{ width: "1.1em", flexShrink: 0, fontFamily: "monospace", fontWeight: 700, color: status.color, fontSize: "0.78rem" }}
        aria-label={change.status}
      >
        {status.letter}
      </Typography>
      <ListItemText
        primary={name}
        secondary={directory || undefined}
        slotProps={{
          primary: { noWrap: true, sx: { fontSize: "0.8rem", fontWeight: open ? 650 : 500 } },
          secondary: { noWrap: true, sx: { fontSize: "0.7rem", direction: "rtl", textAlign: "left" } },
        }}
        sx={{ my: 0, minWidth: 0 }}
      />
      {(change.additions !== undefined || change.deletions !== undefined) && (
        <Typography component="span" sx={{ flexShrink: 0, fontFamily: "monospace", fontSize: "0.7rem" }}>
          <Box component="span" sx={{ color: "success.main" }}>+{change.additions ?? 0}</Box>
          {" "}
          <Box component="span" sx={{ color: "error.main" }}>−{change.deletions ?? 0}</Box>
        </Typography>
      )}
      {badge && (
        <Box
          component="span"
          data-review-finding-badge={badge.count}
          aria-label={`${String(badge.count)} finding${badge.count === 1 ? "" : "s"}, highest ${badge.severity}`}
          sx={{ flexShrink: 0, minWidth: "1.3em", px: 0.5, borderRadius: 1, fontSize: "0.66rem", fontWeight: 700, textAlign: "center", color: "background.paper", bgcolor: BADGE_COLOR[badge.severity] }}
        >
          {badge.count}
        </Box>
      )}
      {reviewed && <CheckCircle aria-label="Reviewed" sx={{ fontSize: "1rem", color: "success.main", flexShrink: 0 }} />}
    </ListItemButton>
  );
}

function ComparisonMenu({
  changes,
  openRequest,
}: {
  changes: DesktopReviewChanges;
  openRequest: number;
}): React.JSX.Element {
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  // A request opens the menu once: remounting (a new context, a tab
  // switch) must not replay the last one.
  const handled = useRef(openRequest);
  useEffect(() => {
    if (openRequest === handled.current) return;
    handled.current = openRequest;
    setOpen(true);
  }, [openRequest]);
  const result = changes.result;
  const comparisons = result?.comparisons ?? [];
  const scenarios = useMemo(
    () => reviewCompareScenarios(undefined, comparisons, result?.defaultComparison, result?.committedComparisons === true),
    [comparisons, result?.committedComparisons, result?.defaultComparison],
  );
  const branches = useMemo(() => reviewCompareBranches(comparisons, scenarios, "").slice(0, 30), [comparisons, scenarios]);
  const choose = (comparison: string | null): void => {
    setOpen(false);
    if (comparison !== changes.comparison) changes.setComparison(comparison);
  };
  return (
    <>
      <Button
        ref={anchor}
        size="small"
        color="inherit"
        data-review-compare
        title={reviewCompareLabel(changes.comparison, scenarios, comparisons)}
        endIcon={<ExpandMore />}
        onClick={() => setOpen(true)}
        sx={{ minWidth: 0, textTransform: "none", fontSize: "0.75rem", justifyContent: "flex-start", overflow: "hidden" }}
      >
        <Box component="span" sx={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {reviewCompareLabel(changes.comparison, scenarios, comparisons)}
        </Box>
      </Button>
      <LeaderKeycap leaderKey="VA" />
      <Menu anchorEl={anchor.current} open={open} onClose={() => setOpen(false)}>
        {scenarios.map((scenario) => (
          <MenuItem
            key={scenario.key}
            selected={scenario.comparison === changes.comparison}
            onClick={() => choose(scenario.comparison)}
          >
            <ListItemText primary={scenario.title} secondary={scenario.detail} />
          </MenuItem>
        ))}
        {branches.length > 0 && <Divider />}
        {branches.map((branch) => (
          <MenuItem
            key={branch.reference}
            selected={branch.reference === changes.comparison}
            onClick={() => choose(branch.reference)}
          >
            <ListItemText primary={branch.label} secondary="Branch, including uncommitted work" />
          </MenuItem>
        ))}
      </Menu>
    </>
  );
}

/** Changes: the comparison, the unread filter and the change list. */
export function ReviewChangesList({
  changes,
  reviewed,
  openKey,
  unreadOnly,
  onUnreadOnly,
  filter,
  compareRequest,
  onOpen,
  findings,
}: {
  findings: readonly ReviewFinding[];
  changes: DesktopReviewChanges;
  reviewed: ReadonlySet<string>;
  openKey: string | undefined;
  unreadOnly: boolean;
  onUnreadOnly: (value: boolean) => void;
  filter: string;
  compareRequest: number;
  onOpen: (entry: GitReviewEntry) => void;
}): React.JSX.Element {
  const [visible, setVisible] = useState(WINDOW);
  const needle = filter.trim().toLowerCase();
  const sections = useMemo(() => changes.sections.map((section) => ({
    ...section,
    entries: section.entries.filter((entry) =>
      (!unreadOnly || !reviewed.has(entryKey(entry))) &&
      (!needle || entry.change.path.toLowerCase().includes(needle))
    ),
  })).filter((section) => section.entries.length > 0), [changes.sections, needle, reviewed, unreadOnly]);
  const total = changes.queue.length;
  const done = changes.queue.filter((entry) => reviewed.has(entryKey(entry))).length;
  let budget = visible;
  const listed = sections.reduce((count, section) => count + section.entries.length, 0);
  return (
    <Box sx={{ display: "flex", flexDirection: "column", minHeight: 0, flex: 1 }}>
      <Box sx={{ display: "flex", alignItems: "center", gap: 0.5, px: 1, py: 0.5, minWidth: 0 }}>
        <ComparisonMenu changes={changes} openRequest={compareRequest} />
        <Box sx={{ flex: 1 }} />
        <Tooltip title="Show only files not reviewed yet">
          <ToggleButton
            value="unread"
            size="small"
            selected={unreadOnly}
            onChange={() => onUnreadOnly(!unreadOnly)}
            data-review-unread-toggle
            sx={{ py: 0, px: 0.75, gap: 0.5, textTransform: "none", fontSize: "0.72rem", height: desktopSize(26), border: 0 }}
          >
            Unread <LeaderKeycap leaderKey="VU" />
          </ToggleButton>
        </Tooltip>
        <Tooltip title="Refresh">
          <IconButton size="small" aria-label="Refresh changes" onClick={changes.refresh}>
            <Refresh fontSize="small" />
          </IconButton>
        </Tooltip>
      </Box>
      <Typography sx={{ px: 1.5, pb: 0.5, fontSize: "0.7rem", color: "text.secondary" }} data-review-progress>
        {changes.error
          ? "Changes could not be read."
          : changes.result
          ? `${String(done)} / ${String(total)} reviewed${changes.result.truncated ? " · list cut short" : ""}`
          : "Reading changes…"}
      </Typography>
      <List
        dense
        disablePadding
        sx={{ flex: 1, minHeight: 0, overflowY: "auto", px: 0.5, pb: 1 }}
      >
        {changes.result && listed === 0 && (
          <Typography sx={{ p: 2, fontSize: "0.8rem", color: "text.secondary" }}>
            {total === 0 ? "No changes in this comparison." : "Nothing matches."}
          </Typography>
        )}
        {sections.map((section) => {
          const shown = section.entries.slice(0, Math.max(0, budget));
          budget -= shown.length;
          if (shown.length === 0) return null;
          return (
            <li key={section.kind}>
              <ul style={{ padding: 0 }}>
                {sections.length > 1 && (
                  <ListSubheader sx={{ lineHeight: desktopSize(26), fontSize: "0.68rem", bgcolor: "background.paper" }}>
                    {section.label} · {section.entries.length}
                  </ListSubheader>
                )}
                {shown.map((entry) => (
                  <ChangeRow
                    key={entryKey(entry)}
                    entry={entry}
                    open={entryKey(entry) === openKey}
                    reviewed={reviewed.has(entryKey(entry))}
                    onOpen={onOpen}
                    badge={findingBadge(findings, entry.change.path)}
                  />
                ))}
              </ul>
            </li>
          );
        })}
        {listed > visible && (
          <Button size="small" onClick={() => setVisible((count) => count + WINDOW)} sx={{ m: 1 }}>
            Show {Math.min(WINDOW, listed - visible)} more
          </Button>
        )}
      </List>
    </Box>
  );
}
