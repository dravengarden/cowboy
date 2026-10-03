import { Button, type ButtonProps } from "@mui/material";
import { useEffect, useRef, useState } from "react";

/** Keep ordinary redirects visible while the browser waits for the server. */
export function ExternalSignInButton({
  busy = false,
  onRedirectBusy,
  onRedirectError,
  children,
  ...props
}: ButtonProps & {
  busy?: boolean;
  onRedirectBusy?: ((busy: boolean) => void) | undefined;
  onRedirectError?: ((error: string | null) => void) | undefined;
}): React.JSX.Element {
  const [redirecting, setRedirecting] = useState(false);
  const pending = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const callbacks = useRef({ onRedirectBusy, onRedirectError });
  callbacks.current = { onRedirectBusy, onRedirectError };
  useEffect(() => {
    const reset = (): void => {
      if (!pending.current) return;
      pending.current = false;
      clearTimeout(timer.current);
      setRedirecting(false);
      callbacks.current.onRedirectBusy?.(false);
    };
    // Safari can restore the same React tree, including its busy state, from
    // the back/forward cache after cancelled or completed authentication.
    globalThis.addEventListener("pageshow", reset);
    return () => {
      globalThis.removeEventListener("pageshow", reset);
      clearTimeout(timer.current);
    };
  }, []);
  const loading = busy || redirecting;
  return (
    <Button
      {...props}
      loading={loading}
      loadingPosition="start"
      aria-busy={loading}
      onClick={(event) => {
        if (pending.current || busy) {
          event.preventDefault();
          return;
        }
        props.onClick?.(event);
        if (
          !props.href || event.defaultPrevented || event.metaKey ||
          event.ctrlKey || event.shiftKey || event.altKey
        ) return;
        pending.current = true;
        setRedirecting(true);
        callbacks.current.onRedirectError?.(null);
        callbacks.current.onRedirectBusy?.(true);
        timer.current = setTimeout(() => {
          pending.current = false;
          setRedirecting(false);
          callbacks.current.onRedirectBusy?.(false);
          callbacks.current.onRedirectError?.(
            "Sign-in is taking longer than expected. If this page hasn't changed, check your connection and try again.",
          );
        }, 20_000);
      }}
    >
      <span role="status" aria-live="polite">
        {redirecting ? "Opening sign-in…" : children}
      </span>
    </Button>
  );
}
