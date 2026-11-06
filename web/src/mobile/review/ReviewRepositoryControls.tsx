import { Box, Stack } from "@mui/material";
import { SegmentedTabs } from "../../SegmentedTabs";

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
        columnGap: 0.5,
        px: 0.75,
        py: 0.25,
        bgcolor: "background.default",
      }}
    >
      <SegmentedTabs
        rootProps={{ "data-mobile-repository-tabs": "" }}
        aria-label="Repository views"
        fullWidth={false}
        sx={{
          flex: "0 0 auto",
          minWidth: 0,
          position: "relative",
          zIndex: 0,
          gap: 0,
          gridAutoColumns: "max-content",
          "--segmented-selected-fill": "transparent",
          "&::before": {
            content: '""',
            position: "absolute",
            inset: 0,
            my: "auto",
            height: "2rem",
            borderRadius: "0.5rem",
            bgcolor: "action.hover",
            zIndex: -1,
            pointerEvents: "none",
          },
          "&& [data-segmented-tab]": {
            minHeight: 44,
            minWidth: 44,
            px: 0,
            borderRadius: "0.5rem",
            fontSize: "0.8125rem",
            "& .segmented-tab-label": {
              px: "0.25rem",
              py: "0.375rem",
              borderRadius: "0.375rem",
            },
            "&[data-selected='true'] .segmented-tab-label": {
              bgcolor: "action.selected",
              fontWeight: 600,
            },
          },
        }}
        value={section}
        onChange={onSection}
        options={[
          { value: "changes", label: "Changes" },
          { value: "history", label: "History" },
          { value: "worktrees", label: "Worktrees" },
        ]}
      />
      {section === "changes" && <Box ref={onControls} sx={{ display: "contents" }} />}
    </Stack>
  );
}
