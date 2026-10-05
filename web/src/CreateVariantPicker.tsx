import { Typography } from "@mui/material";
import {
  ChatBubbleOutline,
  DescriptionOutlined,
  FolderOutlined,
} from "@mui/icons-material";
import { useMemo } from "react";
import { type SegmentedTabChangeSource, SegmentedTabs } from "./SegmentedTabs";
import { WorkspacePicker } from "./WorkspacePicker";
import { useStoreSelector } from "./store";
import { sessionDirectoryChoices } from "./sessionDirectoryChoices";

export type CreateVariant = "session" | "draft" | "folder";

const CREATE_VARIANTS: readonly {
  value: CreateVariant;
  label: string;
  icon: React.JSX.Element;
}[] = [
  { value: "session", label: "Session", icon: <ChatBubbleOutline /> },
  { value: "draft", label: "Draft", icon: <DescriptionOutlined /> },
  { value: "folder", label: "Folder", icon: <FolderOutlined /> },
];

export function createVariantTabId(variant: CreateVariant): string {
  return `create-${variant}-tab`;
}

export function CreateVariantPicker(
  { value, disabled, onChange, keyboard = false }: {
    value: CreateVariant;
    disabled: boolean;
    onChange: (value: CreateVariant, source: SegmentedTabChangeSource) => void;
    /** Desktop: Vim tablist grammar (h/l). Direct picks come from the
     *  dialog's leader labels, so the tabs carry no digits. */
    keyboard?: boolean;
  },
): React.JSX.Element {
  return (
    <SegmentedTabs
      value={value}
      onChange={onChange}
      aria-label="Create type"
      disabled={disabled}
      vimKeys={keyboard}
      options={CREATE_VARIANTS.map(({ value, label, icon }) => ({
        value,
        icon,
        label,
        id: createVariantTabId(value),
        controls: "create-variant-panel",
        ariaLabel: label,
      }))}
    />
  );
}

export function DraftCreationDirectory({ value, onChange }: {
  value: string;
  onChange: (value: string) => void;
}): React.JSX.Element {
  const folders = useStoreSelector((snapshot) => snapshot.sessionFolders);
  const choices = useMemo(
    () => sessionDirectoryChoices(folders),
    [folders],
  );
  return (
    <>
      <WorkspacePicker
        label="Directory (optional)"
        clearable
        hierarchyPreferenceKey="cowboy.sessionDirectoryHierarchy"
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
