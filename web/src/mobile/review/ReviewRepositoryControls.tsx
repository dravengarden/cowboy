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
        borderTop: 1,
        borderColor: "divider",
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
          gap: 0,
          gridAutoColumns: "max-content",
          "&& [data-segmented-tab]": {
            minHeight: 44,
            px: 0.5,
            borderRadius: 1,
            fontSize: "0.8125rem",
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
      {section === "changes" && (
        <Box ref={onControls} sx={{ display: "contents" }} />
      )}
    </Stack>
  );
}
