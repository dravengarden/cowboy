import { Box, Typography } from "@mui/material";
import {
  ChatBubbleOutline,
  DescriptionOutlined,
  FolderOutlined,
} from "@mui/icons-material";
import { useMemo } from "react";
import { type SegmentedTabChangeSource, SegmentedTabs } from "./SegmentedTabs";
import { Kbd } from "./Kbd";
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
  { value, disabled, onChange, keyboard = false, digitsAvailable = false }: {
    value: CreateVariant;
    disabled: boolean;
    onChange: (value: CreateVariant, source: SegmentedTabChangeSource) => void;
    /** Desktop: the modal grammar's tab keys (FOCUS.md "Modals"): `1`–`3`
     *  and `H/L`, each tab showing its digit. */
    keyboard?: boolean;
    /** The dialog is in Normal, where digits pick a tab. */
    digitsAvailable?: boolean;
  },
): React.JSX.Element {
  return (
    <SegmentedTabs
      value={value}
      onChange={onChange}
      aria-label="Create type"
      disabled={disabled}
      vimKeys={keyboard}
      options={CREATE_VARIANTS.map(({ value, label, icon }, index) => ({
        value,
        icon,
        label: keyboard
          ? (
            <Box component="span" sx={{ display: "inline-flex", alignItems: "center" }}>
              {label}
              <Kbd
                keys={String(index + 1)}
                availability={digitsAvailable ? "available" : "inactive"}
              />
            </Box>
          )
          : label,
        ...(keyboard ? { keyShortcuts: String(index + 1) } : {}),
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
