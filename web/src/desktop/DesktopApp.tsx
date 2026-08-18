import { Box } from "@mui/material";
import { useEffect, useMemo, useState } from "react";
import { App } from "../App";
import { useProductAuth } from "../auth/ProductAuthGate";
import { ProductPasskeysPanel } from "../auth/ProductPasskeysPanel";
import { ProductTokensPanel } from "../auth/ProductTokensPanel";
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
  const [tokensOpen, setTokensOpen] = useState(false);
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
  const tokensCommand = useMemo<DesktopCommand>(() => ({
    id: "account.tokens",
    title: "API tokens",
    description: "Create or revoke personal access tokens",
    group: "Account",
    run: () => setTokensOpen(true),
  }), []);
  const passkeysCommand = useMemo<DesktopCommand>(() => ({
    id: "account.passkeys",
    title: "Passkeys",
    description: "Add a Passkey or turn off the 15-minute viewing lock",
    group: "Account",
    run: () => setPasskeysOpen(true),
  }), []);
  useDesktopCommand(signOutCommand);
  useDesktopCommand(tokensCommand);
  useDesktopCommand(passkeysCommand);
  return (
    <>
    <DesktopModal
      open={tokensOpen}
      onClose={() => setTokensOpen(false)}
      title="API tokens"
      description={`Tokens for ${me.account}. Use COWBOY_USER_TOKEN with serve-acp.`}
      width={520}
    >
      <Box sx={{ px: 2.25, py: 2 }}>
        <ProductTokensPanel />
      </Box>
    </DesktopModal>
    <DesktopModal
      open={passkeysOpen}
      onClose={() => setPasskeysOpen(false)}
      title="Passkeys"
      description="Password login stays first. A Passkey is optional, then locks this view after 15 minutes idle."
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
