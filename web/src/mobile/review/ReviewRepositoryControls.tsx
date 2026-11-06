import { Box, Stack, Tab, Tabs } from "@mui/material";

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
  return (
    <Stack
      data-review-repository-controls
      direction="row"
      alignItems="center"
      sx={{
        flexShrink: 0,
        flexWrap: "wrap",
        columnGap: 0.25,
        px: 0.75,
        py: 0.25,
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
          borderRadius: 1,
          bgcolor: "action.hover",
          "& .MuiTabs-indicator": { display: "none" },
          "& .MuiTabs-list": { flexWrap: "wrap" },
          "& .MuiTab-root": {
            minHeight: 48,
            minWidth: 48,
            px: 0.5,
            borderRadius: 1,
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
