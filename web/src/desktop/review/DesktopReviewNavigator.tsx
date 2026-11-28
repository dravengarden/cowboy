import { alpha, Box, InputBase, Tab, Tabs, Tooltip } from "@mui/material";
import { useDesktopLeaderOptional } from "../commands/leaderContext";
import { useEffect, useRef } from "react";
import { desktopSize } from "../../surface/desktopSize";
import { LeaderKeycap } from "../commands/DesktopKeycap";
import type { GitReviewEntry } from "../../review/gitReviewModel";
import { ReviewChangesList } from "./ReviewChangesList";
import { ReviewFilesList } from "./ReviewFilesList";
import { ReviewContextMenu } from "./ReviewContextMenu";
import { ReviewHistoryList } from "./ReviewHistoryList";
import { ReviewWorktreesList } from "./ReviewWorktreesList";
import type { DesktopReviewContext } from "./useDesktopReviewContext";
import { ReviewPullList } from "./ReviewPullList";
import type { ReviewFinding } from "./reviewFindingsModel";
import type { RemoteReviewBinding, RemoteReviewFile } from "../../review/remoteReviewModel";
import type { DesktopReviewChanges } from "./useDesktopReviewChanges";

export type ReviewNavigatorTab = "changes" | "files" | "history" | "worktrees" | "pulls";
export const REVIEW_NAVIGATOR_TABS: readonly {
  value: ReviewNavigatorTab;
  label: string;
  fullLabel: string;
  leaderKey: string;
}[] = [
  { value: "changes", label: "Changes", fullLabel: "Changes", leaderKey: "VC" },
  { value: "files", label: "Files", fullLabel: "Files", leaderKey: "VF" },
  { value: "history", label: "History", fullLabel: "History", leaderKey: "VH" },
  { value: "worktrees", label: "Trees", fullLabel: "Worktrees", leaderKey: "VW" },
  { value: "pulls", label: "PRs", fullLabel: "Pull requests", leaderKey: "VG" },
];
export const REVIEW_NAVIGATOR_REGION = "review.navigator";
export const REVIEW_FILTER_FOCUS_EVENT = "cowboy:review-filter-focus";

/**
 * The Navigator: Changes and Files as MUI tabs over one Cowboy list. Its rows
 * are `data-desktop-item`s, so J/K, gg/G, Ctrl-D/U, zz and `'` labels come
 * from the shared list grammar; `L`/`H`/`[`/`]`/`/` are its own commands.
 */
/** The Navigator's tabs; each names its leader key (focus stays on rows). */
function NavigatorTabs({ tab, onTab }: { tab: ReviewNavigatorTab; onTab: (tab: ReviewNavigatorTab) => void }): React.JSX.Element {
  const leader = useDesktopLeaderOptional();
  const showLeader = leader?.armed === true && (leader.layer === "root" || leader.layer === "group:v");
  return (
    <Tabs
      value={tab}
      variant="fullWidth"
      onChange={(_event, next: ReviewNavigatorTab) => onTab(next)}
      aria-label="Review navigator"
      sx={{
        minHeight: desktopSize(32),
        px: 0.75,
        py: 0.4,
        borderBottom: 1,
        borderColor: "divider",
        flexShrink: 0,
        "& .MuiTabs-flexContainer": { gap: 0.35, alignItems: "center" },
        "& .MuiTabs-indicator": { display: "none" },
      }}
    >
      {REVIEW_NAVIGATOR_TABS.map(({ value, label, fullLabel, leaderKey }) => (
        <Tab
          key={value}
          value={value}
          tabIndex={-1}
          aria-label={`${fullLabel} (Space ${leaderKey})`}
          data-review-navigator-tab={value}
          data-review-navigator-armed={showLeader ? "true" : undefined}
          disableRipple
          label={
            <Tooltip title={`${fullLabel} (␣${leaderKey})`} enterDelay={400}>
              <span>{showLeader ? <LeaderKeycap leaderKey={leaderKey} /> : label}</span>
            </Tooltip>
          }
          sx={{
            minHeight: desktopSize(24),
            minWidth: 0,
            flex: "1 1 0%",
            px: showLeader ? 0.25 : 0.5,
            py: 0,
            gap: 0.25,
            borderRadius: "6px",
            textTransform: "none",
            fontSize: "0.73rem",
            fontWeight: 550,
            color: "text.secondary",
            whiteSpace: "nowrap",
            transition: "background-color 120ms ease, color 120ms ease",
            "&:hover": { bgcolor: "action.hover", color: "text.primary" },
            "&.Mui-selected": {
              color: "text.primary",
              fontWeight: 650,
              bgcolor: (theme) => alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.18 : 0.11),
            },
            "& .MuiTab-icon": { m: 0 },
          }}
        />
      ))}
    </Tabs>
  );
}

export function DesktopReviewNavigator({
  context,
  contextRequest,
  onReviewContext,
  openCommitPath,
  onOpenCommit,
  openPullPath,
  onOpenPull,
  sessionId,
  agentSessionId,
  tab,
  onTab,
  filter,
  onFilter,
  changes,
  reviewed,
  progress,
  openEntryKey,
  openSourcePath,
  unreadOnly,
  onUnreadOnly,
  streamMode,
  onToggleStream,
  compareRequest,
  onOpenEntry,
  onOpenSource,
  dataRevision,
  findings,
}: {
  findings: readonly ReviewFinding[];
  context: DesktopReviewContext;
  contextRequest: number;
  onReviewContext: (value: string) => void;
  openCommitPath: { oid: string; path: string } | undefined;
  onOpenCommit: (oid: string, path: string) => void;
  openPullPath: string | undefined;
  onOpenPull: (binding: RemoteReviewBinding, revision: string, file: RemoteReviewFile) => void;
  sessionId: string;
  agentSessionId: string | undefined;
  tab: ReviewNavigatorTab;
  onTab: (tab: ReviewNavigatorTab) => void;
  filter: string;
  onFilter: (filter: string) => void;
  changes: DesktopReviewChanges;
  reviewed: ReadonlySet<string>;
  progress: Readonly<Record<string, string>>;
  openEntryKey: string | undefined;
  openSourcePath: string | undefined;
  unreadOnly: boolean;
  onUnreadOnly: (value: boolean) => void;
  streamMode: boolean;
  onToggleStream: () => void;
  compareRequest: number;
  onOpenEntry: (entry: GitReviewEntry) => void;
  onOpenSource: (path: string) => void;
  dataRevision: number;
}): React.JSX.Element {
  const field = useRef<HTMLInputElement>(null);
  const region = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const element = region.current;
    const focus = (): void => field.current?.focus();
    element?.addEventListener(REVIEW_FILTER_FOCUS_EVENT, focus);
    return () => element?.removeEventListener(REVIEW_FILTER_FOCUS_EVENT, focus);
  }, []);
  const toList = (): void => {
    region.current?.querySelector<HTMLElement>("[data-desktop-item]")?.focus();
  };
  return (
    <Box
      ref={region}
      data-desktop-region={REVIEW_NAVIGATOR_REGION}
      data-review-navigator
      tabIndex={-1}
      sx={{ display: "flex", flexDirection: "column", minHeight: 0, height: "100%", overflow: "hidden", outline: "none" }}
    >
      <ReviewContextMenu context={context} openRequest={contextRequest} onChoose={onReviewContext} />
      <NavigatorTabs tab={tab} onTab={onTab} />
      {(tab === "changes" || tab === "files") && <InputBase
        inputRef={field}
        value={filter}
        onChange={(event) => onFilter(event.target.value)}
        placeholder={tab === "files" ? "Find a file  /" : "Filter changes  /"}
        inputProps={{
          "aria-label": tab === "files" ? "Find a file" : "Filter changes",
          "data-desktop-vim": "off",
          "data-review-filter": tab,
        }}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) return;
          if (event.key === "Escape" || event.key === "ArrowDown" || event.key === "Enter") {
            event.preventDefault();
            event.stopPropagation();
            if (event.key === "Escape" && filter) onFilter("");
            toList();
          }
        }}
        sx={{ mx: 1, my: 0.5, px: 1, fontSize: "0.78rem", borderRadius: 1, bgcolor: "action.hover", flexShrink: 0 }}
      />}
      {tab === "history" && context.target && (
        <ReviewHistoryList
          sessionId={sessionId}
          dataRevision={dataRevision}
          openCommitPath={openCommitPath}
          onOpen={onOpenCommit}
        />
      )}
      {tab === "pulls" && context.target && (
        <ReviewPullList context={context} openPath={openPullPath} onOpen={onOpenPull} />
      )}
      {tab === "worktrees" && context.target && (
        <ReviewWorktreesList target={context.target} onReview={onReviewContext} />
      )}
      {tab === "changes"
        ? (
          <ReviewChangesList
            sessionId={sessionId}
            agentSessionId={agentSessionId}
            changes={changes}
            reviewed={reviewed}
            progress={progress}
            openKey={openEntryKey}
            unreadOnly={unreadOnly}
            onUnreadOnly={onUnreadOnly}
            streamMode={streamMode}
            onToggleStream={onToggleStream}
            filter={filter}
            compareRequest={compareRequest}
            onOpen={onOpenEntry}
            findings={findings}
          />
        )
        : tab === "files" && (
          <ReviewFilesList
            sessionId={sessionId}
            filter={filter}
            openPath={openSourcePath}
            dataRevision={dataRevision}
            onOpen={onOpenSource}
          />
        )}
    </Box>
  );
}
