// The `dom_app` node (docs/widget-apps-dom.md §5, §6): a full-DOM app that
// never starts by itself. It shows its title and Run; tapping Run reserves
// one of the page's runtime slots (shared with phase A apps) and the single
// full-app ownership (a later Run stops an earlier one), then starts it in a
// sandboxed frame. Out of view or on a hidden page it is stopped after a short
// grace. Where full apps are not admitted, the agent's fallback stands in. A
// `send` becomes the same confirmation as phase A apps; a stopped frame's
// unconfirmed requests are dropped with it.

import { useEffect, useRef, useState } from "react";
import { Alert, Box, Button, Typography } from "@mui/material";
import { useTheme } from "@mui/material/styles";
import PlayArrowRounded from "@mui/icons-material/PlayArrowRounded";
import StopRounded from "@mui/icons-material/StopRounded";
import type { ParsedNode } from "../widgetDocument";
import { useWidgetVerified } from "../widgetMode";
import { useWidgetInteraction } from "../widgetInteractions";
import { Fallback, useAppSend } from "../WidgetApp";
import { reserveExternalRuntime } from "../apps/appRuntimes";
import { domAppsEnabled } from "./domAppsEnabled";
import { startDomApp, type DomApp } from "./domAppHost";

type DomAppData = Extract<ParsedNode, { type: "dom_app" }>;
type Send = ((text: string, code: string, kind?: "app" | "dom_app") => void) | undefined;

const HEIGHT = { xs: 160, sm: 320, md: 480, lg: 720 } as const;
const OUT_OF_VIEW_GRACE_MS = 1500;

// One full-DOM app per page (§5). Ownership is taken when Run is tapped, so
// an earlier start still loading its images cannot displace a later one.
let owner = 0;
let running: DomApp | null = null;

function RunningApp({
  node,
  sendRef,
  onStopped,
}: {
  node: DomAppData;
  sendRef: { current: Send };
  onStopped: (reason: string) => void;
}): React.JSX.Element {
  const container = useRef<HTMLDivElement>(null);
  const app = useRef<DomApp | null>(null);
  const propsRef = useRef<unknown>(node.props ?? {});
  propsRef.current = node.props ?? {};
  const theme = useTheme();
  const [booted, setBooted] = useState(false);
  const max = HEIGHT[node.height ?? "md"];

  useEffect(() => {
    const token = (owner += 1);
    running?.stop("Another full app started.");
    const release = reserveExternalRuntime();
    if (!release) {
      onStopped("Too many apps are running on this page. Close one, then run this again.");
      return;
    }
    let done = false;
    const finish = (reason: string): void => {
      if (done) return;
      done = true;
      release();
      onStopped(reason);
    };
    void import("../apps/appImages")
      .then(({ loadAppImageBytes }) => loadAppImageBytes(node.images))
      .then((images) => {
        if (done || token !== owner || !container.current) {
          if (!done && token !== owner) finish("Another full app started.");
          return;
        }
        const started = startDomApp(
          container.current,
          {
            code: node.code,
            props: propsRef.current,
            images,
            context: { theme: theme.palette.mode, locale: navigator.language, platform: "web" },
          },
          {
            onBooted: () => setBooted(true),
            onHeight: (height) => {
              started.frame.style.height = `${Math.min(max, Math.max(80, height))}px`;
            },
            onSend: (text) => sendRef.current?.(text, node.code, "dom_app"),
            onStopped: (reason) => {
              if (running === started) running = null;
              finish(reason);
            },
          },
        );
        running = started;
        app.current = started;
      });

    // Off screen, or on a hidden page, the app is stopped after a grace.
    let hiddenSince: ReturnType<typeof setTimeout> | null = null;
    let inView = true;
    const review = (): void => {
      const visible = inView && document.visibilityState === "visible";
      if (visible && hiddenSince) {
        clearTimeout(hiddenSince);
        hiddenSince = null;
      } else if (!visible && !hiddenSince) {
        hiddenSince = setTimeout(
          () => app.current?.stop("This app was stopped while it was out of view."),
          OUT_OF_VIEW_GRACE_MS,
        );
      }
    };
    const observer = new IntersectionObserver((records) => {
      inView = records.some((record) => record.isIntersecting);
      review();
    });
    if (container.current) observer.observe(container.current);
    document.addEventListener("visibilitychange", review);
    return () => {
      observer.disconnect();
      document.removeEventListener("visibilitychange", review);
      if (hiddenSince) clearTimeout(hiddenSince);
      app.current?.stop();
      if (token === owner && !app.current) owner += 1;
      finish("closed");
    };
    // Keyed by identity and code in the parent: each run starts once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const propsJson = JSON.stringify(node.props ?? {});
  useEffect(() => {
    app.current?.props(JSON.parse(propsJson) as unknown);
  }, [propsJson]);

  return (
    <Box sx={{ display: "grid", gap: 0.5 }}>
      {booted ? null : (
        <Typography variant="caption" color="text.secondary">
          Starting “{node.title}”…
        </Typography>
      )}
      <Box ref={container} data-dom-app-frame="" sx={{ borderRadius: 1, overflow: "hidden" }} />
    </Box>
  );
}

export default function WidgetDomApp({ node }: { node: DomAppData }): React.JSX.Element {
  const verified = useWidgetVerified();
  const interaction = useWidgetInteraction();
  const target =
    interaction && !interaction.closed
      ? { sessionId: interaction.sessionId, widgetId: interaction.widgetId, nodeId: node.id }
      : null;
  const send = useAppSend(node, target);
  // Always the current identity's sender, whatever run is live.
  const sendRef = useRef<Send>(send.onSend);
  sendRef.current = send.onSend;
  const [state, setState] = useState<"idle" | "running" | { stopped: string }>("idle");
  if (!verified) return <Fallback node={node} note={null} />;
  if (interaction?.closed)
    return <Fallback node={node} note={`“${node.title}” was closed by the agent.`} />;
  if (!domAppsEnabled()) {
    return (
      <Fallback node={node} note={`“${node.title}” is a full app: open it in Chrome on a Mac.`} />
    );
  }
  // A new identity or version is a new run.
  const identity = `${target?.sessionId}:${target?.widgetId}:${node.id}:${node.code}`;
  const stop = (reason: string): void => {
    send.revoke();
    setState({ stopped: reason });
  };
  return (
    <Box data-widget-dom-app={node.title} sx={{ display: "grid", gap: 1, minWidth: 0 }}>
      {state === "running" ? (
        <>
          <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
            <Typography variant="caption" color="text.secondary" sx={{ flex: 1 }}>
              Full app: runs its own code, without network. Cowboy can stop it.
            </Typography>
            <Button
              size="small"
              startIcon={<StopRounded />}
              onClick={(): void => stop("Stopped.")}
              sx={{ textTransform: "none" }}
            >
              Stop
            </Button>
          </Box>
          <RunningApp key={identity} node={node} sendRef={sendRef} onStopped={stop} />
        </>
      ) : (
        <>
          <Typography variant="body2">{node.title}</Typography>
          {typeof state === "object" && state.stopped !== "Stopped." && state.stopped !== "closed" ? (
            <Alert severity="warning" variant="outlined">
              {state.stopped}
            </Alert>
          ) : null}
          <Button
            size="small"
            variant="outlined"
            startIcon={<PlayArrowRounded />}
            onClick={(): void => setState("running")}
            sx={{ textTransform: "none", justifySelf: "start" }}
          >
            Run “{node.title}”
          </Button>
        </>
      )}
      {send.confirm}
    </Box>
  );
}
