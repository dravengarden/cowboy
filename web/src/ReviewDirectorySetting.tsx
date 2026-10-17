import { useState } from "react";
import {
  Alert,
  Button,
  ListItemButton,
  ListItemText,
  Stack,
  Typography,
} from "@mui/material";
import {
  ArrowBack,
  ChevronRight,
  FolderOpenOutlined,
} from "@mui/icons-material";
import { useActiveWorkspaceBinding } from "./controlPlane";
import {
  mutateMobileReview,
  useMobileReviewState,
  useStoreSelector,
} from "./store";
import {
  inheritedReviewDirectory,
  resolveReviewDirectory,
  reviewDirectoryLabel,
} from "./reviewDirectoryDefault";
import { ReviewDirectoryPicker } from "./ReviewDirectoryPicker";

export function ReviewDirectorySetting(
  { onUseDefault, currentDirectory, onEditingChange }: {
    onUseDefault?: () => void;
    currentDirectory?: string | undefined;
    onEditingChange?: (editing: boolean) => void;
  } = {},
): React.JSX.Element | null {
  const bound = useActiveWorkspaceBinding();
  const sessions = useStoreSelector((s) => s.sessions);
  const machines = useStoreSelector((s) => s.machines);
  const tree = useStoreSelector((s) => s.sessionFolders);
  const own = useMobileReviewState(bound?.sessionId).directory;
  const [editing, setEditing] = useState(false);
  const session = sessions.find((s) => s.id === bound?.sessionId);
  if (!session) return null;
  const effective = inheritedReviewDirectory(session, tree, own);
  const resolved = resolveReviewDirectory(
    effective.value,
    session,
    sessions,
    machines,
  );
  const edit = (value: boolean) => {
    setEditing(value);
    onEditingChange?.(value);
  };
  const defaultIdentity = resolved.context?.sessionId.startsWith("workspace::")
    ? resolved.context.sessionId
    : resolved.context
    ? `session::${resolved.context.sessionId}`
    : undefined;
  const save = (directory: string | null | undefined) => {
    mutateMobileReview(
      session.id,
      "setDirectory",
      directory === undefined ? { inherit: true } : { directory },
    );
    edit(false);
  };
  return (
    <Stack
      spacing={1}
      data-session-review-directory
      sx={{ minHeight: 0, flex: editing ? 1 : undefined }}
    >
      {editing
        ? (
          <>
            <Button
              startIcon={<ArrowBack />}
              onClick={() => edit(false)}
              sx={{ alignSelf: "flex-start", textTransform: "none" }}
            >
              {onUseDefault ? "Context" : "Default directory"}
            </Button>
            <ReviewDirectoryPicker
              value={own}
              inherited={effective.inherited}
              machines={machines}
              sessions={sessions}
              onChange={save}
            />
          </>
        )
        : (
          <>
            <ListItemButton
              data-edit-review-directory
              onClick={() => edit(true)}
              sx={{ borderRadius: 1.5, px: 1.25, minHeight: 56, gap: 1 }}
            >
              <FolderOpenOutlined fontSize="small" color="primary" />
              <ListItemText
                primary="Zed default directory"
                secondary={`${
                  effective.value == null
                    ? "Automatic"
                    : reviewDirectoryLabel(effective.value, machines, sessions)
                }${
                  effective.source === "Automatic"
                    ? " · Session worktree, then project"
                    : ` · ${effective.source}`
                }`}
                slotProps={{
                  primary: { variant: "body2", fontWeight: 650 },
                  secondary: { sx: { overflowWrap: "anywhere" } },
                }}
              />
              <ChevronRight fontSize="small" />
            </ListItemButton>
            {resolved.context && (
              <Typography
                variant="caption"
                color="text.secondary"
                noWrap
                title={resolved.context.cwd}
                sx={{ px: 1.25 }}
              >
                {resolved.context.cwd}
              </Typography>
            )}
            {resolved.error && (
              <Alert severity="warning">{resolved.error}</Alert>
            )}
            {!onUseDefault && (
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ px: 1.25 }}
              >
                Used when opening this session. Your current file and the
                Agent’s working directory stay in place.
              </Typography>
            )}
            <Stack
              direction="row"
              spacing={1}
              flexWrap="wrap"
              sx={{
                "& .MuiButton-root": { textTransform: "none", minHeight: 44 },
              }}
            >
              {onUseDefault && (
                <Button
                  data-use-review-default
                  onClick={onUseDefault}
                  disabled={!resolved.context}
                >
                  Open default
                </Button>
              )}
              {currentDirectory && defaultIdentity !== currentDirectory && (
                <Button onClick={() => save(currentDirectory)}>
                  Save current
                </Button>
              )}
              {own !== undefined && (
                <Button onClick={() => save(undefined)}>
                  Restore inheritance
                </Button>
              )}
            </Stack>
          </>
        )}
    </Stack>
  );
}
