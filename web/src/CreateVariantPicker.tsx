import { Stack, Typography } from "@mui/material";
import {
  ChatBubbleOutline,
  DescriptionOutlined,
  FolderOutlined,
} from "@mui/icons-material";
import { useMemo } from "react";
import { Kbd } from "./Kbd";
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
  { value, disabled, onChange, keyboard = false, keysAvailable = false }: {
    value: CreateVariant;
    disabled: boolean;
    onChange: (value: CreateVariant, source: SegmentedTabChangeSource) => void;
    /** Desktop: Vim tablist grammar plus visible digit slots. */
    keyboard?: boolean;
    /** The tablist owns keyboard focus, so its slots execute now. */
    keysAvailable?: boolean;
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
        id: createVariantTabId(value),
        controls: "create-variant-panel",
        ariaLabel: label,
        ...(keyboard
          ? {
            keyShortcuts: String(index + 1),
            label: (
              <Stack
                component="span"
                direction="row"
                spacing={0.75}
                alignItems="center"
                justifyContent="center"
              >
                <span>{label}</span>
                <Kbd
                  keys={String(index + 1)}
                  variant="context"
                  availability={keysAvailable && !disabled
                    ? "available"
                    : "inactive"}
                />
              </Stack>
            ),
          }
          : { label }),
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
