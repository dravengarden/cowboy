import { Button } from "@mui/material";
import { DescriptionOutlined } from "@mui/icons-material";
import { openDrafts } from "./navigation";

export function DraftsButton(): React.JSX.Element {
  return (
    <Button
      color="inherit"
      startIcon={<DescriptionOutlined />}
      onClick={() => openDrafts()}
      sx={{
        justifyContent: "flex-start",
        mx: 1,
        my: 0.5,
        minHeight: "2.75rem",
        textTransform: "none",
      }}
    >
      Drafts
    </Button>
  );
}
