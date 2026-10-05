import { Typography } from "@mui/material";
import {
  ChatBubbleOutline,
  DescriptionOutlined,
  FolderOutlined,
} from "@mui/icons-material";
import { useMemo } from "react";
import { SegmentedTabs } from "./SegmentedTabs";
import { WorkspacePicker } from "./WorkspacePicker";
import { draftFolderTree } from "./documents/model";
import { useDraftLibrary } from "./documents/store";
import { sessionDirectoryChoices } from "./sessionDirectoryChoices";

export type CreateVariant = "session" | "draft" | "folder";

export function CreateVariantPicker({ value, disabled, onChange }: {
  value: CreateVariant;
  disabled: boolean;
  onChange: (value: CreateVariant) => void;
}): React.JSX.Element {
  return (
    <SegmentedTabs
      value={value}
      onChange={onChange}
      aria-label="Create type"
      disabled={disabled}
      options={[
        {
          value: "session",
          label: "Session",
          icon: <ChatBubbleOutline />,
          id: "create-session-tab",
          controls: "create-variant-panel",
        },
        {
          value: "draft",
          label: "Draft",
          icon: <DescriptionOutlined />,
          id: "create-draft-tab",
          controls: "create-variant-panel",
        },
        {
          value: "folder",
          label: "Folder",
          icon: <FolderOutlined />,
          id: "create-folder-tab",
          controls: "create-variant-panel",
        },
      ]}
    />
  );
}

export function DraftCreationDirectory({ value, onChange }: {
  value: string;
  onChange: (value: string) => void;
}): React.JSX.Element {
  const library = useDraftLibrary();
  const choices = useMemo(
    () => sessionDirectoryChoices(draftFolderTree(library.entries)),
    [library.entries],
  );
  return (
    <>
      <WorkspacePicker
        label="Draft directory (optional)"
        clearable
        hierarchyPreferenceKey="cowboy.draftDirectoryHierarchy"
        entries={choices}
        value={value}
        onChange={onChange}
      />
      <Typography variant="caption" color="text.secondary">
        Leave empty for the top level. Drafts are independent documents; no
        project, computer or AI installation is needed.
      </Typography>
    </>
  );
}
