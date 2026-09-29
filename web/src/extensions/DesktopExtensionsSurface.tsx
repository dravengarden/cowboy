import { type ReactNode, useMemo } from "react";
import { useDesktopCommand } from "../desktop/commands/DesktopCommandProvider";
import { DesktopModal } from "../desktop/DesktopModal";

export default function DesktopExtensionsSurface(
  { open, onOpen, onClose, children }: {
    open: boolean;
    onOpen: () => void;
    onClose: () => void;
    children: ReactNode;
  },
): React.JSX.Element {
  const command = useMemo(
    () => ({
      id: "workspace.extensions",
      title: "Open workspace extensions",
      description: "Browse repository resources from installed extensions",
      group: "Workspace",
      run: onOpen,
    }),
    [onOpen],
  );
  useDesktopCommand(command);
  return (
    <DesktopModal open={open} onClose={onClose} title="Extensions" width={1120}>
      {children}
    </DesktopModal>
  );
}
