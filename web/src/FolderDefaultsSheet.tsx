import { useState } from "react";
import { Button } from "@mui/material";
import { setSessionFolderDefault, useStoreSelector } from "./store";
import { FOLDER_DEFAULT_FIELDS, type FolderDefaultKey } from "./folderDefaults";
import { sessionFolderById, type SessionFoldersValue } from "./sessionFolders";
import type { MachineSummary } from "./protocol";
import { useProviderCatalog } from "./providerCatalog";
import { InAppRoot } from "./SessionFolderUi";
import { DirectoryWorkbench } from "./DirectoryWorkbench";
import { FolderDefaultsEditor } from "./FolderDefaultsEditor";

export function FolderDefaultsSheet(
  { id, onClose }: { id: string; onClose: () => void },
): React.JSX.Element {
  const tree = useStoreSelector((state) => state.sessionFolders);
  const machines = useStoreSelector((state) => state.machines);
  const { catalog, error } = useProviderCatalog();
  return (
    <FolderDefaultsView
      id={id}
      onClose={onClose}
      tree={tree}
      machines={machines}
      catalog={catalog}
      error={error}
      onSetDefault={setSessionFolderDefault}
    />
  );
}

export function FolderDefaultsView({ id, onClose, ...props }: {
  id: string;
  onClose: () => void;
  tree: SessionFoldersValue;
  machines: readonly MachineSummary[];
  catalog: ReturnType<typeof useProviderCatalog>["catalog"];
  error?: string | null;
  onSetDefault: typeof setSessionFolderDefault;
}): React.JSX.Element {
  const [field, setField] = useState<FolderDefaultKey | null>(null);
  const folder = sessionFolderById(props.tree, id);
  return (
    <InAppRoot>
      <DirectoryWorkbench
        title={field
          ? FOLDER_DEFAULT_FIELDS[field]
          : `${folder?.name ?? "Directory"} defaults`}
        onClose={field ? () => setField(null) : onClose}
        actions={<Button onClick={onClose}>Done</Button>}
      >
        <FolderDefaultsEditor
          {...props}
          id={id}
          field={field}
          onFieldChange={setField}
          fill
        />
      </DirectoryWorkbench>
    </InAppRoot>
  );
}
