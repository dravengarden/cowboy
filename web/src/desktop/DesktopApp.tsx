import { Box } from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import { App } from "../App";
import { useProductAuth } from "../auth/ProductAuthGate";
import { ProductPasskeysPanel } from "../auth/ProductPasskeysPanel";
import { ProductDevicesPanel } from "../auth/ProductDevicesPanel";
import type { Mode as ThemeMode } from "../theme";
import {
  type DesktopCommand,
  DesktopCommandProvider,
  useDesktopCommand,
} from "./commands/DesktopCommandProvider";
import { DesktopModal } from "./DesktopModal";
import { DesktopWorkspaceProvider } from "./DesktopWorkspaceController";
import { installDesktopNativeEscapeGuard } from "./desktopNativeEscapeGuard";

function DesktopAccountCommands(): React.JSX.Element {
  const { me, signOut } = useProductAuth();
  if (me.auth_enabled === false) return <></>;
  return <EnabledDesktopAccountCommands me={me} signOut={signOut} />;
}

function EnabledDesktopAccountCommands({
  me,
  signOut,
}: Pick<ReturnType<typeof useProductAuth>, "me" | "signOut">): React.JSX.Element {
  const [devicesOpen, setDevicesOpen] = useState(false);
  const [passkeysOpen, setPasskeysOpen] = useState(false);
  const signOutCommand = useMemo<DesktopCommand>(() => ({
    id: "account.signOut",
    title: "Sign out",
    description: `Sign out ${me.account}`,
    group: "Account",
    run: () => {
      void signOut();
    },
  }), [me.account, signOut]);
  const devicesCommand = useMemo<DesktopCommand>(() => ({
    id: "account.devices",
    title: "Authorized clients",
    description: "Review or revoke signed-in Cowboy clients",
    group: "Account",
    run: () => setDevicesOpen(true),
  }), []);
  const passkeysCommand = useMemo<DesktopCommand>(() => ({
    id: "account.passkeys",
    title: "Passkeys",
    description: "Add a Passkey or configure periodic verification",
    group: "Account",
    run: () => setPasskeysOpen(true),
  }), []);
  useDesktopCommand(signOutCommand);
  useDesktopCommand(devicesCommand);
  useDesktopCommand(passkeysCommand);
  return (
    <>
    <DesktopModal
      open={devicesOpen}
      onClose={() => setDevicesOpen(false)}
      title="Authorized clients"
      description={`CLI and ACP clients signed in as ${me.account}. Browser sessions and Passkeys are managed separately.`}
      width={520}
    >
      <Box sx={{ px: 2.25, py: 2 }}>
        <ProductDevicesPanel />
      </Box>
    </DesktopModal>
    <DesktopModal
      open={passkeysOpen}
      onClose={() => setPasskeysOpen(false)}
      title="Passkeys"
      description="Password login stays first. A Passkey can periodically lock this view while agents keep running."
      width={520}
    >
      <Box sx={{ px: 2.25, py: 2 }}>
        <ProductPasskeysPanel />
      </Box>
    </DesktopModal>
    </>
  );
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
        <DesktopAccountCommands />
        <App
          themeMode={themeMode}
          onSetThemeMode={onSetThemeMode}
          surface="desktop"
        />
      </DesktopCommandProvider>
    </DesktopWorkspaceProvider>
  );
}
