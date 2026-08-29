import { Box, CircularProgress, Stack, Typography } from "@mui/material";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { flushSync } from "react-dom";
import { useStoreSelector } from "../store";
import { MachineSetupPage } from "./MachineSetupPage";
import {
  needsMachineSetup,
  type SetupMachine,
} from "./machineReady";

export function MachineSetupGate({
  children,
}: {
  children: ReactNode;
}): React.JSX.Element {
  const pushedMachines = useStoreSelector((snapshot) => snapshot.machines);
  const pushedMachinesLoaded = useStoreSelector((snapshot) =>
    snapshot.machinesLoaded
  );
  const [presented, setPresented] = useState<{
    loaded: boolean;
    machines: readonly SetupMachine[];
  }>({ loaded: false, machines: [] });
  const presentedRef = useRef(presented);

  useEffect(() => {
    if (!pushedMachinesLoaded) return;
    const previous = presentedRef.current;
    const next = { loaded: true, machines: pushedMachines };
    const transitionToSetup = previous.loaded &&
      !needsMachineSetup(previous.machines) && needsMachineSetup(next.machines);
    const commit = (): void => {
      presentedRef.current = next;
      setPresented(next);
    };
    if (transitionToSetup && "startViewTransition" in document) {
      try {
        const transition = document.startViewTransition(() => flushSync(commit));
        void transition.finished.catch(() => undefined);
        return;
      } catch {
        // A browser may expose the API while another transition owns it.
        // The setup gate remains authoritative and falls back immediately.
      }
    }
    commit();
  }, [pushedMachines, pushedMachinesLoaded]);

  if (!presented.loaded) {
    return (
      <Box
        sx={{
          minHeight: "100%",
          display: "grid",
          placeItems: "center",
          bgcolor: "background.default",
          color: "text.secondary",
        }}
      >
        <Stack spacing={2} alignItems="center">
          <CircularProgress size={28} color="inherit" />
          <Typography sx={{ fontSize: 14, letterSpacing: "0.06em", opacity: 0.75 }}>
            cowboy
          </Typography>
        </Stack>
      </Box>
    );
  }
  if (needsMachineSetup(presented.machines)) {
    return (
      <Box
        data-machine-setup-gate
        sx={{
          minHeight: "100dvh",
          animation: "machine-setup-enter 180ms ease-out both",
          "@keyframes machine-setup-enter": {
            from: { opacity: 0 },
            to: { opacity: 1 },
          },
          "@media (prefers-reduced-motion: reduce)": { animation: "none" },
        }}
      >
        <MachineSetupPage />
      </Box>
    );
  }
  return <>{children}</>;
}
