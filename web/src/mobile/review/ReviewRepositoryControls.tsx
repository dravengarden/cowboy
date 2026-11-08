import { Box, Stack, Tab, Tabs } from "@mui/material";
import { useSurfaceProfile } from "../../surface/SurfaceProfile";

export type RepositorySection = "changes" | "history" | "worktrees";

export function ReviewRepositoryControls({
  section,
  onSection,
  onControls,
}: {
  section: RepositorySection;
  onSection: (section: RepositorySection) => void;
  onControls: (container: HTMLDivElement | null) => void;
}): React.JSX.Element {
  const mobile = useSurfaceProfile().kind !== "desktop";
  return (
    <Stack
      data-review-repository-controls
      direction="row"
      alignItems="center"
      sx={{
        flexShrink: 0,
        flexWrap: "wrap",
        columnGap: mobile ? 1 : 0.25,
        rowGap: mobile ? 0.75 : 0,
        px: mobile ? 1 : 0.75,
        py: mobile ? 1 : 0.25,
        bgcolor: "background.default",
      }}
    >
      <Tabs
        data-mobile-repository-tabs
        aria-label="Repository views"
        selectionFollowsFocus
        sx={{
          flex: "0 0 auto",
          minWidth: 0,
          maxWidth: "100%",
          minHeight: 48,
          borderRadius: mobile ? "24px" : 1,
          bgcolor: "action.hover",
          "& .MuiTabs-indicator": { display: "none" },
          "& .MuiTabs-list": { flexWrap: "wrap", gap: mobile ? 0.25 : 0 },
          "& .MuiTab-root": {
            minHeight: 48,
            minWidth: mobile ? 56 : 48,
            px: mobile ? 1.5 : 0.5,
            borderRadius: mobile ? "999px" : 1,
            fontSize: "0.8125rem",
            textTransform: "none",
            "&.Mui-selected": {
              bgcolor: "action.selected",
            },
          },
        }}
        value={section}
        onChange={(_, value: RepositorySection) => onSection(value)}
      >
        <Tab data-segmented-tab="changes" value="changes" label="Changes" />
        <Tab data-segmented-tab="history" value="history" label="History" />
        <Tab data-segmented-tab="worktrees" value="worktrees" label="Worktrees" />
      </Tabs>
      {section === "changes" && <Box ref={onControls} sx={{ display: "contents" }} />}
    </Stack>
  );
}
