import { Box, CircularProgress, Stack, Typography } from "@mui/material";
import { useEffect, useState, type ReactNode } from "react";
import { flushSync } from "react-dom";
import { MachineSetupPage } from "./MachineSetupPage";
import {
  fetchSetupMachines,
  MACHINE_SETUP_REFRESH_EVENT,
  needsMachineSetup,
  type SetupMachine,
} from "./machineReady";

export function MachineSetupGate({
  children,
}: {
  children: ReactNode;
}): React.JSX.Element {
  const [machines, setMachines] = useState<SetupMachine[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    const commit = (next: SetupMachine[], transitionToSetup: boolean): void => {
      if (cancelled) return;
      if (
        transitionToSetup && needsMachineSetup(next) &&
        "startViewTransition" in document
      ) {
        try {
          const transition = document.startViewTransition(() => {
            flushSync(() => setMachines(next));
          });
          void transition.finished.catch(() => undefined);
          return;
        } catch {
          // A browser may expose the API while another transition owns it.
          // The setup gate remains authoritative and falls back immediately.
        }
      }
      setMachines(next);
    };
    const load = (transitionToSetup = false): void => {
      void fetchSetupMachines()
        .then((next) => {
          commit(next, transitionToSetup);
        })
        .catch(() => {
          commit([], transitionToSetup);
        });
    };
    load();
    const timer = globalThis.setInterval(load, 3000);
    const refreshAfterMutation = (): void => load(true);
    globalThis.addEventListener(MACHINE_SETUP_REFRESH_EVENT, refreshAfterMutation);
    return () => {
      cancelled = true;
      globalThis.clearInterval(timer);
      globalThis.removeEventListener(MACHINE_SETUP_REFRESH_EVENT, refreshAfterMutation);
    };
  }, []);

  if (machines == null) {
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
  if (needsMachineSetup(machines)) {
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
