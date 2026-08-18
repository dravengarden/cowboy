import { useEffect, useMemo } from "react";
import { App } from "../App";
import { useProductAuth } from "../auth/ProductAuthGate";
import type { Mode as ThemeMode } from "../theme";
import {
  type DesktopCommand,
  DesktopCommandProvider,
  useDesktopCommand,
} from "./commands/DesktopCommandProvider";
import { DesktopWorkspaceProvider } from "./DesktopWorkspaceController";
import { installDesktopNativeEscapeGuard } from "./desktopNativeEscapeGuard";

function DesktopProductSignOutCommand(): null {
  const { me, signOut } = useProductAuth();
  const command = useMemo<DesktopCommand>(() => ({
    id: "account.signOut",
    title: "Sign out",
    description: `Sign out ${me.account}`,
    group: "Account",
    run: () => {
      void signOut();
    },
  }), [me.account, signOut]);
  useDesktopCommand(command);
  return null;
}

export function DesktopApp({
  themeMode,
  onSetThemeMode,
}: {
  themeMode: ThemeMode;
  onSetThemeMode: (mode: ThemeMode) => void;
}): React.JSX.Element {
  useEffect(() => {
    (globalThis as typeof globalThis & {
      __cowboyDesktopBootReady?: () => void;
    }).__cowboyDesktopBootReady?.();
  }, []);
  useEffect(() => installDesktopNativeEscapeGuard(), []);
  return (
    <DesktopWorkspaceProvider>
      <DesktopCommandProvider>
        <DesktopProductSignOutCommand />
        <App
          themeMode={themeMode}
          onSetThemeMode={onSetThemeMode}
          surface="desktop"
        />
      </DesktopCommandProvider>
    </DesktopWorkspaceProvider>
  );
}
