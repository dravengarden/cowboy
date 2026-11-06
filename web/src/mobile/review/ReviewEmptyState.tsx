import { CheckRounded, CommitOutlined, FolderOutlined, History } from "@mui/icons-material";
import { Box, Button, List, ListItemButton, Stack, Typography } from "@mui/material";
import { useSurfaceProfile } from "../../surface/SurfaceProfile";
import type { GitChangeTarget, GitCommitSummary } from "./codeApi";
import { relativeDate, shortOid } from "./ReviewRepositoryRows";

export interface EmptyReviewContext {
  machineLabel?: string | undefined;
  projectPath?: string | undefined;
  commits: readonly GitCommitSummary[];
  onHistory: () => void;
  onCommit: (commit: GitCommitSummary) => void;
}

/** A quiet repository still has useful context. Reuse the snapshot already
 * loaded by Review; opening this overview never requests additional data. */
export function ReviewEmptyState({
  comparisonLabel,
  compared,
  documentsOnly,
  target,
  head,
  repository,
  onBrowseFiles,
  linkedSession,
}: {
  comparisonLabel: string;
  compared: boolean;
  documentsOnly: boolean;
  target?: GitChangeTarget | undefined;
  head?: string | undefined;
  repository?: EmptyReviewContext | undefined;
  onBrowseFiles?: (() => void) | undefined;
  linkedSession?: { title: string; onOpen: () => void } | undefined;
}): React.JSX.Element {
  const desktop = useSurfaceProfile().kind === "desktop";
  return (
    <Box
      data-review-empty-state
      sx={{
        minHeight: "100%",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        px: 0.75,
        py: 3,
      }}
    >
      <Stack spacing={2} sx={{ width: "100%", maxWidth: desktop ? 720 : "none", minWidth: 0 }}>
        <Stack alignItems="center" spacing={0.75} sx={{ textAlign: "center" }}>
          <Box
            sx={{
              width: 40,
              height: 40,
              borderRadius: "50%",
              bgcolor: "action.hover",
              display: "grid",
              placeItems: "center",
              color: "success.main",
            }}
          >
            <CheckRounded sx={{ fontSize: "1.5rem" }} />
          </Box>
          <Typography sx={{ fontSize: "1.125rem", fontWeight: 600 }}>
            {documentsOnly
              ? "No Markdown changes in this comparison"
              : compared
                ? "No changes to review"
                : "Working tree is clean"}
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ fontSize: "0.875rem" }}>
            {documentsOnly
              ? "This view includes Markdown files only."
              : compared
                ? "The selected comparison has no file differences."
                : "There are no uncommitted file changes."}
          </Typography>
        </Stack>
        <EmptyFacts
          comparisonLabel={comparisonLabel}
          target={target}
          head={head}
          repository={repository}
        />
        {repository && <RecentActivity repository={repository} />}
        {linkedSession && (
          <Box sx={{ px: 1 }}>
            <Typography
              variant="body2"
              color="text.secondary"
            >{`This session's worktree doesn't include the changes in ${linkedSession.title}.`}</Typography>
            <Button
              data-review-linked-session
              onClick={linkedSession.onOpen}
              sx={{ minHeight: 44, textTransform: "none", fontSize: "0.875rem" }}
            >
              Review linked session
            </Button>
          </Box>
        )}
        <Stack direction="row" sx={{ flexWrap: "wrap", justifyContent: "center", gap: 1 }}>
          {onBrowseFiles && (
            <Button
              startIcon={<FolderOutlined />}
              onClick={onBrowseFiles}
              sx={{
                minHeight: 44,
                px: 1.5,
                borderRadius: 1,
                textTransform: "none",
                fontSize: "0.875rem",
                bgcolor: "action.selected",
              }}
            >
              Browse files
            </Button>
          )}
          {repository && (
            <Button
              aria-label="View repository history"
              startIcon={<History />}
              onClick={repository.onHistory}
              sx={{
                minHeight: 44,
                px: 1.5,
                textTransform: "none",
                color: "text.secondary",
                fontSize: "0.875rem",
              }}
            >
              View history
            </Button>
          )}
        </Stack>
      </Stack>
    </Box>
  );
}

function RecentActivity({
  repository,
}: {
  repository: EmptyReviewContext;
}): React.JSX.Element | null {
  if (!repository.commits.length) return null;
  return (
    <Box>
      <Typography variant="caption" color="text.secondary" sx={{ px: 1, fontSize: "0.75rem" }}>
        Recent activity
      </Typography>
      <List disablePadding aria-label="Recent repository activity">
        {repository.commits.slice(0, 3).map((commit) => (
          <ListItemButton
            key={commit.oid}
            onClick={() => repository.onCommit(commit)}
            sx={{ minHeight: 44, px: 1, py: 0.5, gap: 1, borderRadius: 1 }}
          >
            <CommitOutlined
              sx={{
                fontSize: "1.25rem",
                color: "text.secondary",
                flexShrink: 0,
              }}
            />
            <Box sx={{ minWidth: 0, flex: 1 }}>
              <Typography noWrap sx={{ fontSize: "0.875rem" }}>
                {commit.subject}
              </Typography>
              <Typography
                variant="caption"
                color="text.secondary"
                noWrap
                sx={{ display: "block", fontSize: "0.75rem" }}
              >
                {commit.author} · {relativeDate(commit.authoredAt)} · {shortOid(commit.oid)}
              </Typography>
            </Box>
          </ListItemButton>
        ))}
      </List>
    </Box>
  );
}

function EmptyFacts({
  comparisonLabel,
  target,
  head,
  repository,
}: {
  comparisonLabel: string;
  target: GitChangeTarget | undefined;
  head: string | undefined;
  repository: EmptyReviewContext | undefined;
}): React.JSX.Element {
  const base = target?.base;
  const relation =
    base &&
    [
      base.ahead ? `${base.ahead} commits ahead` : "",
      base.behind ? `${base.behind} commits behind` : "",
    ]
      .filter(Boolean)
      .join(" · ");
  const facts = [
    ["Branch", target?.branch],
    ["Comparison", comparisonLabel],
    [
      "Working tree",
      target
        ? target.changeCount > 0
          ? `${target.changeCount} uncommitted files`
          : "Clean"
        : undefined,
    ],
    ["Base branch", base ? `${base.label} · ${relation || "Up to date"}` : undefined],
    ["HEAD", head ? shortOid(head) : undefined],
    ["Machine", repository?.machineLabel],
    ["Location", repository?.projectPath],
  ];
  return (
    <Box
      component="dl"
      data-review-empty-facts
      sx={{
        m: 0,
        p: 1.5,
        display: "grid",
        gridTemplateColumns: "max-content minmax(0, 1fr)",
        columnGap: 1.5,
        rowGap: 0.75,
        bgcolor: "background.paper",
        borderRadius: "8px",
      }}
    >
      {facts
        .filter(([, value]) => value)
        .map(([label, value]) => (
          <Box key={label} sx={{ display: "contents" }}>
            <Typography
              component="dt"
              color="text.secondary"
              sx={{ fontSize: "0.75rem", lineHeight: 1.5 }}
            >
              {label}
            </Typography>
            <Typography
              component="dd"
              sx={{
                m: 0,
                overflowWrap: "anywhere",
                fontSize: "0.75rem",
                lineHeight: 1.5,
              }}
            >
              {value}
            </Typography>
          </Box>
        ))}
    </Box>
  );
}
