import {
  type MouseEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  Box,
  Button,
  type ButtonProps,
  CircularProgress,
  IconButton,
  type IconButtonProps,
} from "@mui/material";
import { notify } from "./store";
import {
  NETWORK_PRESS_MIN_MS,
  NETWORK_PROGRESS_DELAY_MS,
  NETWORK_PROGRESS_MIN_MS,
} from "./networkActionPolicy";

export interface NetworkActionState {
  pending: boolean;
  progress: boolean;
  run: (action: () => Promise<void> | void) => Promise<boolean>;
}

const wait = (milliseconds: number): Promise<void> =>
  new Promise((resolve) => globalThis.setTimeout(resolve, milliseconds));

/**
 * Shared async-control timing grammar.
 *
 * A click becomes visibly inert immediately, but the progress glyph is delayed
 * so a normal round trip remains a crisp press instead of a spinner flash. Once
 * progress is painted it stays long enough to be perceived. The action promise
 * must resolve from authoritative state/acknowledgement, not an animation timer.
 */
export function useNetworkActionState(): NetworkActionState {
  const [pending, setPending] = useState(false);
  const [progress, setProgress] = useState(false);
  const mounted = useRef(true);
  const running = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const run = useCallback(async (action: () => Promise<void> | void): Promise<boolean> => {
    if (running.current) return false;
    running.current = true;
    setPending(true);
    setProgress(false);

    let progressAt = 0;
    const timer = globalThis.setTimeout(() => {
      progressAt = performance.now();
      if (mounted.current) setProgress(true);
    }, NETWORK_PROGRESS_DELAY_MS);

    let succeeded = false;
    try {
      await Promise.all([Promise.resolve().then(action), wait(NETWORK_PRESS_MIN_MS)]);
      succeeded = true;
    } catch (error) {
      notify(error instanceof Error ? error.message : "The action could not be completed");
    } finally {
      globalThis.clearTimeout(timer);
      if (progressAt > 0) {
        const remaining = NETWORK_PROGRESS_MIN_MS - (performance.now() - progressAt);
        if (remaining > 0) await wait(remaining);
      }
      running.current = false;
      if (mounted.current) {
        setProgress(false);
        setPending(false);
      }
    }
    return succeeded;
  }, []);

  return { pending, progress, run };
}

function ProgressOverlay({ visible, size }: { visible: boolean; size: number }): React.JSX.Element {
  return (
    <CircularProgress
      aria-hidden
      size={size}
      thickness={4.5}
      sx={{
        position: "absolute",
        inset: 0,
        m: "auto",
        opacity: visible ? 1 : 0,
        transform: visible ? "scale(1)" : "scale(0.82)",
        transition: "opacity 160ms ease, transform 160ms ease",
        pointerEvents: "none",
      }}
    />
  );
}

export interface NetworkButtonProps extends Omit<ButtonProps, "onClick" | "action"> {
  networkAction: () => Promise<void> | void;
  children: ReactNode;
}

export function NetworkButton({ networkAction, children, disabled, sx, ...props }: NetworkButtonProps): React.JSX.Element {
  const state = useNetworkActionState();
  return (
    <Button
      {...props}
      aria-busy={state.pending || undefined}
      disabled={disabled || state.pending}
      onClick={(): void => {
        void state.run(networkAction);
      }}
      sx={{
        position: "relative",
        transition: "opacity 120ms ease, background-color 120ms ease, color 120ms ease",
        ...(state.pending && { opacity: 0.58 }),
        ...(state.progress && { "& .MuiButton-startIcon": { opacity: 0 } }),
        ...sx,
      }}
    >
      <Box component="span" sx={{ display: "inline-flex", alignItems: "center", opacity: state.progress ? 0 : 1 }}>
        {children}
      </Box>
      <ProgressOverlay visible={state.progress} size={18} />
    </Button>
  );
}

export interface NetworkIconButtonProps extends Omit<IconButtonProps, "onClick" | "action"> {
  networkAction: () => Promise<void> | void;
}

export function NetworkIconButton({ networkAction, children, disabled, sx, ...props }: NetworkIconButtonProps): React.JSX.Element {
  const state = useNetworkActionState();
  return (
    <IconButton
      {...props}
      aria-busy={state.pending || undefined}
      disabled={disabled || state.pending}
      onClick={(_event: MouseEvent<HTMLButtonElement>): void => {
        void state.run(networkAction);
      }}
      sx={{
        position: "relative",
        transition: "opacity 120ms ease, background-color 120ms ease, color 120ms ease",
        ...(state.pending && { opacity: 0.52 }),
        ...sx,
      }}
    >
      <Box component="span" sx={{ display: "inline-flex", opacity: state.progress ? 0 : 1 }}>
        {children}
      </Box>
      <ProgressOverlay visible={state.progress} size={18} />
    </IconButton>
  );
}
