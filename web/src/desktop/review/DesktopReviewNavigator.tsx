import { Box, InputBase, Tab, Tabs } from "@mui/material";
import { useEffect, useRef } from "react";
import { desktopSize } from "../../surface/desktopSize";
import { LeaderKeycap } from "../commands/DesktopKeycap";
import type { GitReviewEntry } from "../../mobile/review/gitReviewModel";
import { ReviewChangesList } from "./ReviewChangesList";
import { ReviewFilesList } from "./ReviewFilesList";
import type { DesktopReviewChanges } from "./useDesktopReviewChanges";

export type ReviewNavigatorTab = "changes" | "files";
export const REVIEW_NAVIGATOR_REGION = "review.navigator";
export const REVIEW_FILTER_FOCUS_EVENT = "cowboy:review-filter-focus";

/**
 * The Navigator: Changes and Files as MUI tabs over one Cowboy list. Its rows
 * are `data-desktop-item`s, so J/K, gg/G, Ctrl-D/U, zz and `'` labels come
 * from the shared list grammar; `L`/`H`/`[`/`]`/`/` are its own commands.
 */
export function DesktopReviewNavigator({
  sessionId,
  tab,
  onTab,
  filter,
  onFilter,
  changes,
  reviewed,
  openEntryKey,
  openSourcePath,
  unreadOnly,
  onUnreadOnly,
  compareRequest,
  onOpenEntry,
  onOpenSource,
  dataRevision,
}: {
  sessionId: string;
  tab: ReviewNavigatorTab;
  onTab: (tab: ReviewNavigatorTab) => void;
  filter: string;
  onFilter: (filter: string) => void;
  changes: DesktopReviewChanges;
  reviewed: ReadonlySet<string>;
  openEntryKey: string | undefined;
  openSourcePath: string | undefined;
  unreadOnly: boolean;
  onUnreadOnly: (value: boolean) => void;
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
      sx={{ display: "flex", flexDirection: "column", minHeight: 0, height: "100%", outline: "none" }}
    >
      <Tabs
        value={tab}
        onChange={(_event, next: ReviewNavigatorTab) => onTab(next)}
        aria-label="Review navigator"
        sx={{ minHeight: desktopSize(32), px: 0.5, borderBottom: 1, borderColor: "divider", flexShrink: 0 }}
      >
        {(["changes", "files"] as const).map((value) => (
          <Tab
            key={value}
            value={value}
            tabIndex={-1}
            data-review-navigator-tab={value}
            disableRipple
            label={value === "changes" ? "Changes" : "Files"}
            icon={<LeaderKeycap leaderKey={value === "changes" ? "VC" : "VF"} />}
            iconPosition="end"
            sx={{ minHeight: desktopSize(32), minWidth: 0, px: 1, py: 0, gap: 0.5, textTransform: "none", fontSize: "0.75rem", "& .MuiTab-icon": { m: 0 } }}
          />
        ))}
      </Tabs>
      <InputBase
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
      />
      {tab === "changes"
        ? (
          <ReviewChangesList
            changes={changes}
            reviewed={reviewed}
            openKey={openEntryKey}
            unreadOnly={unreadOnly}
            onUnreadOnly={onUnreadOnly}
            filter={filter}
            compareRequest={compareRequest}
            onOpen={onOpenEntry}
          />
        )
        : (
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
