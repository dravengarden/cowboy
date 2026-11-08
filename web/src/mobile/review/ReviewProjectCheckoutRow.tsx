import { ChevronRight, FolderOpenOutlined } from "@mui/icons-material";
import { Box, ListItemButton, Typography } from "@mui/material";
import type { ReviewRegisteredWorkspace } from "../../review/reviewContextModel";

export function ReviewProjectCheckoutRow({
  workspace,
  selected,
  onOpen,
}: {
  workspace: ReviewRegisteredWorkspace;
  selected: boolean;
  onOpen: () => void;
}): React.JSX.Element {
  return (
    <ListItemButton
      data-review-project-checkout
      selected={selected}
      onClick={onOpen}
      sx={{ minHeight: 48, py: 0.75, px: 1.25, borderRadius: 1.5 }}
    >
      <FolderOpenOutlined color="primary" sx={{ mr: 1.25 }} />
      <Box sx={{ flex: 1, minWidth: 0 }}>
        <Typography variant="body2" fontWeight={700} noWrap>
          Project code
        </Typography>
        <Typography variant="caption" color="text.secondary" noWrap>
          Registered project checkout
        </Typography>
        <Typography variant="caption" color="text.disabled" sx={{ display: "block" }} noWrap>
          {workspace.canonicalPath}
        </Typography>
      </Box>
      <ChevronRight color="disabled" fontSize="small" />
    </ListItemButton>
  );
}
