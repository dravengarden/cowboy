// The `mcp_app` node (docs/widget-apps-mcp.md §8): a signed MCP Apps view
// the Controller published after the agent called its tool. It never starts
// by itself: the row shows the binding, what the owner granted it and Run.
// Run asks the Controller for a frame lease, takes one of the page's runtime
// slots and the single full-app ownership, and stops out of view. Where
// views are not admitted the tool's text result stands in.

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
import { openExternalUrl } from "../../openExternal";
import { mcpAppsEnabled } from "./mcpAppsEnabled";
import { claimFullApp, type FullAppClaim } from "./fullAppOwner";
import {
  closeMcpAppFrame,
  openMcpAppFrame,
  startMcpApp,
  type McpAppFrame,
  type McpAppRun,
} from "./mcpAppHost";
import McpCallConfirm, { type PendingCall } from "./McpCallConfirm";

type McpAppData = Extract<ParsedNode, { type: "mcp_app" }>;
type Send = (text: string, code: string, kind: "mcp_app", lease: string) => Promise<boolean>;

const MAX_HEIGHT = 720;
const OUT_OF_VIEW_GRACE_MS = 1500;

function parse(text: string | null): unknown {
  if (text === null) return null;
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return null;
  }
}

function capabilityLine(frame: McpAppFrame | null): string {
  if (!frame) return "";
  const parts: string[] = [];
  const tools = frame.capabilities.call_tools ?? [];
  if (tools.length > 0) parts.push(`calls ${tools.join(", ")}`);
  if (frame.capabilities.message) parts.push("may ask to send a message");
  if (frame.capabilities.open_links) parts.push("may open links");
  return parts.length > 0 ? `Granted: ${parts.join("; ")}.` : "Granted: display only.";
}

function RunningView({
  node,
  session,
  frame,
  sendRef,
  onStopped,
  confirmCall,
  claim,
}: {
  node: McpAppData;
  session: string;
  frame: McpAppFrame;
  claim: FullAppClaim;
  sendRef: { current: Send | undefined };
  onStopped: (reason: string) => void;
  confirmCall: (call: PendingCall["call"]) => Promise<boolean>;
}): React.JSX.Element {
  const container = useRef<HTMLDivElement>(null);
  const run = useRef<McpAppRun | null>(null);
  const theme = useTheme();

  useEffect(() => {
    const release = reserveExternalRuntime();
    if (!release) {
      claim.release();
      closeMcpAppFrame(session, frame.lease);
      onStopped("Too many apps are running on this page. Close one, then run this again.");
      return;
    }
    let done = false;
    const finish = (reason: string): void => {
      if (done) return;
      done = true;
      claim.release();
      release();
      onStopped(reason);
    };
    if (!container.current) {
      finish("closed");
      return;
    }
    const result = parse(node.result);
    const started = startMcpApp(
      container.current,
      {
        session,
        frame,
        version: "phase-c",
        hostContext: {
          theme: theme.palette.mode,
          locale: navigator.language,
          platform: "web",
          containerDimensions: {
            width: container.current.clientWidth,
            maxHeight: MAX_HEIGHT,
          },
        },
        toolInput: (parse(node.input) as Record<string, unknown> | null) ?? {},
        toolResult: result ?? {
          content: [{ type: "text", text: node.fallback }],
          _meta: { "cowboy/truncated": true },
        },
      },
      {
        onHeight: (height) => {
          started.frame.style.height = `${Math.min(MAX_HEIGHT, Math.max(80, height))}px`;
        },
        confirmCall,
        message: (text, lease) =>
          sendRef.current?.(text, node.sha256, "mcp_app", lease) ?? Promise.resolve(false),
        openLink: (url) => openExternalUrl(url),
        onStopped: finish,
      },
    );
    run.current = started;

    // Off screen, or on a hidden page, the view is stopped after a grace.
    let hiddenSince: ReturnType<typeof setTimeout> | null = null;
    let inView = true;
    const review = (): void => {
      const visible = inView && document.visibilityState === "visible";
      if (visible && hiddenSince) {
        clearTimeout(hiddenSince);
        hiddenSince = null;
      } else if (!visible && !hiddenSince) {
        hiddenSince = setTimeout(
          () => run.current?.stop("This view was stopped while it was out of view."),
          OUT_OF_VIEW_GRACE_MS,
        );
      }
    };
    const observer = new IntersectionObserver((records) => {
      inView = records.some((record) => record.isIntersecting);
      review();
    });
    observer.observe(container.current);
    document.addEventListener("visibilitychange", review);
    return () => {
      observer.disconnect();
      document.removeEventListener("visibilitychange", review);
      if (hiddenSince) clearTimeout(hiddenSince);
      run.current?.stop();
      finish("closed");
    };
    // Keyed by lease in the parent: each run starts once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    run.current?.hostContextChanged({ theme: theme.palette.mode });
  }, [theme.palette.mode]);

  return <Box ref={container} data-mcp-app-frame="" sx={{ borderRadius: 1, overflow: "hidden" }} />;
}

export default function WidgetMcpApp({ node }: { node: McpAppData }): React.JSX.Element {
  const verified = useWidgetVerified();
  const interaction = useWidgetInteraction();
  const target =
    interaction && !interaction.closed
      ? { sessionId: interaction.sessionId, widgetId: interaction.widgetId, nodeId: node.id }
      : null;
  const send = useAppSend(node, target);
  const sendRef = useRef<Send | undefined>(send.onSendSettled);
  sendRef.current = send.onSendSettled;
  const [state, setState] = useState<
    "idle" | "opening" | { frame: McpAppFrame; claim: FullAppClaim; generation: number } | { stopped: string }
  >("idle");
  const [call, setCall] = useState<PendingCall | null>(null);
  // Each Run is one generation: a stop, a later Run or unmounting retires it,
  // and anything that answers for a retired generation is dropped.
  const generation = useRef(0);
  useEffect(
    () => () => {
      generation.current += 1;
    },
    [],
  );
  // Only a published row, verified by the Controller, may run a view.
  if (!verified || !target)
    return <Fallback node={node} note={interaction?.closed ? "This view was closed." : null} />;
  if (!mcpAppsEnabled()) {
    return <Fallback node={node} note={`“${node.title}” is an MCP app view: not on this browser.`} />;
  }
  const stop = (reason: string): void => {
    generation.current += 1;
    send.revoke();
    setCall((pending) => {
      pending?.answer(false);
      return null;
    });
    setState({ stopped: reason });
  };
  const run = async (): Promise<void> => {
    const mine = (generation.current += 1);
    // Ownership is taken when Run is tapped, so a slow earlier Run cannot
    // displace a later app when its lease arrives.
    const claim = claimFullApp(() => {
      if (generation.current === mine) stop("Another full app started.");
    });
    setState("opening");
    const opened = await openMcpAppFrame(target.sessionId, target.widgetId, node.id);
    if (generation.current !== mine || !claim.owns()) {
      if (!("refused" in opened)) closeMcpAppFrame(target.sessionId, opened.lease);
      claim.release();
      return;
    }
    if ("refused" in opened) {
      claim.release();
      setState({ stopped: opened.refused });
      return;
    }
    setState({ frame: opened, claim, generation: mine });
  };
  const confirmCall = (pending: PendingCall["call"]): Promise<boolean> =>
    new Promise((resolve) => {
      setCall((current) => {
        current?.answer(false);
        return { call: pending, answer: resolve };
      });
    });
  const active = typeof state === "object" && "frame" in state ? state : null;
  const running = active?.frame ?? null;
  return (
    <Box data-widget-mcp-app={node.binding} sx={{ display: "grid", gap: 1, minWidth: 0 }}>
      <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
        <Box sx={{ flex: 1, minWidth: 0 }}>
          <Typography variant="body2">{node.title}</Typography>
          <Typography variant="caption" color="text.secondary">
            {running
              ? `Signed view from ${node.binding}, without network. ${capabilityLine(running)}`
              : `Signed view from ${node.binding}.`}
          </Typography>
        </Box>
        {running ? (
          <Button
            size="small"
            data-desktop-transcript-stop="action"
            startIcon={<StopRounded />}
            onClick={(): void => stop("Stopped.")}
            sx={{ textTransform: "none" }}
          >
            Stop
          </Button>
        ) : (
          <Button
            size="small"
            variant="outlined"
            data-desktop-transcript-stop="action"
            disabled={state === "opening"}
            startIcon={<PlayArrowRounded />}
            onClick={(): void => void run()}
            sx={{ textTransform: "none" }}
          >
            Run
          </Button>
        )}
      </Box>
      {typeof state === "object" &&
      "stopped" in state &&
      state.stopped !== "Stopped." &&
      state.stopped !== "closed" ? (
        <Alert severity="warning" variant="outlined">
          {state.stopped}
        </Alert>
      ) : null}
      {active ? (
        <RunningView
          key={active.frame.lease}
          node={node}
          session={target.sessionId}
          frame={active.frame}
          claim={active.claim}
          sendRef={sendRef}
          onStopped={(reason): void => {
            if (generation.current === active.generation) stop(reason);
          }}
          confirmCall={confirmCall}
        />
      ) : (
        <Fallback node={node} note={null} />
      )}
      {call ? (
        <McpCallConfirm
          pending={call}
          binding={node.binding}
          onClose={(): void => setCall(null)}
        />
      ) : null}
      {send.confirm}
    </Box>
  );
}
