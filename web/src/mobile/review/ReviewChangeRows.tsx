import {
  Add,
  CallSplit,
  CheckCircleOutline,
  ChevronRight,
  DeleteOutline,
  DescriptionOutlined,
  ErrorOutline,
  ExpandMore,
  FolderOutlined,
  VisibilityOutlined,
} from "@mui/icons-material";
import {
  Box,
  IconButton,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Stack,
  Typography,
} from "@mui/material";
import type { CodeChangeStatus } from "../../review/codeApi";
import type { GitReviewEntry } from "../../review/gitReviewModel";
import type { GitChangeTreeNode } from "../../review/gitChangeTree";
import { reviewEntryKey } from "../../review/diffNavigationModel";
import { isMarkdownReviewPath } from "./reviewMarkdown";

const statusLabel: Record<CodeChangeStatus, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  untracked: "U",
  conflicted: "!",
};

function ChangeIcon(
  { status }: { status: CodeChangeStatus },
): React.JSX.Element {
  if (status === "added" || status === "untracked") return <Add />;
  if (status === "deleted") return <DeleteOutline />;
  if (status === "renamed") return <CallSplit />;
  if (status === "conflicted") return <ErrorOutline />;
  return <DescriptionOutlined />;
}

export function GitTreeRows({
  nodes,
  depth,
  collapsed,
  onToggle,
  onOpenDiff,
  queue,
  reviewed,
  onPreview,
}: {
  nodes: GitChangeTreeNode[];
  depth: number;
  collapsed: ReadonlySet<string>;
  onToggle: (path: string) => void;
  onOpenDiff: (entry: GitReviewEntry, queue: GitReviewEntry[]) => void;
  queue: GitReviewEntry[];
  reviewed: ReadonlySet<string>;
  onPreview?: ((path: string) => void) | undefined;
}): React.JSX.Element {
  return (
    <>
      {nodes.map((node) => {
        if (node.kind === "directory") {
          const isCollapsed = collapsed.has(node.path);
          return (
            <Box key={`directory:${node.path}`}>
              <ListItemButton
                onClick={() => onToggle(node.path)}
                sx={{
                  minHeight: 34,
                  py: 0,
                  pl: 0.75 + depth * 1.5,
                  pr: 1,
                }}
              >
                <ListItemIcon sx={{ minWidth: 26, color: "text.secondary" }}>
                  {isCollapsed
                    ? <ChevronRight fontSize="small" />
                    : <ExpandMore fontSize="small" />}
                </ListItemIcon>
                <FolderOutlined
                  fontSize="small"
                  sx={{ mr: 0.75, color: "text.secondary" }}
                />
                <ListItemText
                  primary={node.name}
                  primaryTypographyProps={{
                    noWrap: true,
                    fontFamily: "var(--cowboy-font-mono)",
                    fontSize: "0.8125rem",
                  }}
                  sx={{ my: 0 }}
                />
              </ListItemButton>
              {!isCollapsed && (
                <GitTreeRows
                  nodes={node.children}
                  depth={depth + 1}
                  collapsed={collapsed}
                  onToggle={onToggle}
                  onOpenDiff={onOpenDiff}
                  queue={queue}
                  reviewed={reviewed}
                  onPreview={onPreview}
                />
              )}
            </Box>
          );
        }
        const entry = node.entry;
        if (!entry) return null;
        const isReviewed = reviewed.has(
          reviewEntryKey(
            entry.change.path,
            entry.scope,
            entry.comparison,
          ),
        );
        return (
          <Stack
            direction="row"
            alignItems="center"
            key={`${entry.scope}:${entry.change.path}`}
          >
            <ListItemButton
              key={`${entry.scope}:${entry.change.path}`}
              onClick={() => onOpenDiff(entry, queue)}
              sx={{
                minHeight: 38,
                py: 0,
                pl: 0.75 + depth * 1.5,
                pr: 1.25,
              }}
            >
              <ListItemIcon
                sx={{
                  minWidth: 28,
                  color: isReviewed ? "success.main" : "text.secondary",
                  "& svg": { fontSize: "1.125rem" },
                }}
              >
                {isReviewed
                  ? <CheckCircleOutline fontSize="small" />
                  : <ChangeIcon status={entry.change.status} />}
              </ListItemIcon>
              <ListItemText
                primary={node.name}
                primaryTypographyProps={{
                  noWrap: true,
                  fontFamily: "var(--cowboy-font-mono)",
                  fontSize: "0.8125rem",
                }}
                sx={{ my: 0 }}
              />
              {entry.change.additions !== undefined && (
                <Typography
                  aria-label={`${entry.change.additions} added, ${
                    entry.change.deletions ?? 0
                  } removed`}
                  variant="caption"
                  noWrap
                  sx={{
                    flexShrink: 0,
                    ml: 1,
                    fontFamily: "var(--cowboy-font-mono)",
                  }}
                >
                  <Box component="span" sx={{ color: "success.main" }}>
                    {`+${entry.change.additions}`}
                  </Box>{" "}
                  <Box component="span" sx={{ color: "error.main" }}>
                    {`−${entry.change.deletions ?? 0}`}
                  </Box>
                </Typography>
              )}
              <Typography
                aria-label={entry.change.status}
                variant="caption"
                fontWeight={700}
                color={entry.change.status === "conflicted"
                  ? "error.main"
                  : "text.secondary"}
                sx={{ ml: 1, fontFamily: "var(--cowboy-font-mono)" }}
              >
                {statusLabel[entry.change.status]}
              </Typography>
            </ListItemButton>
            {onPreview && isMarkdownReviewPath(entry.change.path) &&
              entry.change.status !== "deleted" && (
              <IconButton
                aria-label={`Preview current file ${entry.change.path}`}
                onClick={() => onPreview(entry.change.path)}
              >
                <VisibilityOutlined fontSize="small" />
              </IconButton>
            )}
          </Stack>
        );
      })}
    </>
  );
}
