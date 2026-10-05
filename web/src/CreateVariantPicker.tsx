import { Tab, Tabs, Typography } from "@mui/material";
import { useMemo } from "react";
import { WorkspacePicker } from "./WorkspacePicker";
import { draftFolderTree } from "./documents/model";
import { useDraftLibrary } from "./documents/store";
import { sessionDirectoryChoices } from "./sessionDirectoryChoices";

export type CreateVariant = "session" | "draft";

export function CreateVariantPicker({ value, disabled, onChange }: {
  value: CreateVariant;
  disabled: boolean;
  onChange: (value: CreateVariant) => void;
}): React.JSX.Element {
  return (
    <Tabs
      value={value}
      onChange={(_, next: CreateVariant) => onChange(next)}
      variant="fullWidth"
      aria-label="Create type"
    >
      <Tab
        id="create-session-tab"
        aria-controls="create-variant-panel"
        value="session"
        label="Session"
        disabled={disabled}
      />
      <Tab
        id="create-draft-tab"
        aria-controls="create-variant-panel"
        value="draft"
        label="Draft"
        disabled={disabled}
      />
    </Tabs>
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
