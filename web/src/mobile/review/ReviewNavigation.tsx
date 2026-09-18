import { lazy, Suspense, useLayoutEffect, useRef, useState } from "react";
import { Alert, Box, Button, Stack, Typography } from "@mui/material";
import { useStore } from "@cowboy/state-store";
import type { CodeBufferCleanup } from "../../codeBuffers/cleanup.ts";
import type { OwnedNavigation } from "../../codeBuffers/navigation.ts";
import type { NavigationKind } from "../../codeBuffers/navigationProtocol.ts";
import { productCodeBuffers } from "../../codeBuffers/product.ts";
import type { Point } from "../../codeBuffers/protocol.ts";
import { openAppSettings } from "../../appSettings.ts";
import {
  createReviewDestination,
  type ReviewDestination,
} from "./ownedReviewDestination.ts";
import type { OwnedReviewIntelligence } from "./useOwnedReviewBuffer.ts";
import { useReviewSettings } from "./reviewSettings.ts";

const CodeViewer = lazy(() => import("./CodeViewer.tsx"));
const kinds = [
  ["definition", "Definition"],
  ["declaration", "Declaration"],
  ["typeDefinition", "Type"],
  ["implementation", "Implementations"],
  ["references", "References"],
] as const satisfies readonly (readonly [NavigationKind, string])[];

/** Explicit acquisition UI. Mount/hover never probes five effectful queries. */
export function ReviewNavigation(
  { intelligence, point, source = productCodeBuffers.cleanup }: {
    intelligence: OwnedReviewIntelligence;
    point: Point;
    source?: CodeBufferCleanup;
  },
) {
  // Identity replacement removes the whole old consumer before paint. Never
  // reinterpret an operation for a different point or equal-text ABA capture.
  const [original] = useState({
    identity: intelligence.identity,
    point,
    source,
  });
  if (
    original.identity !== intelligence.identity ||
    original.point.row !== point.row ||
    original.point.column !== point.column || original.source !== source
  ) return null;
  return (
    <NavigationView intelligence={intelligence} point={point} source={source} />
  );
}

function NavigationView({ intelligence, point, source }: {
  intelligence: OwnedReviewIntelligence;
  point: Point;
  source: CodeBufferCleanup;
}) {
  const context = useStore(source);
  const settings = useReviewSettings();
  const observer = useRef<AbortController | undefined>(undefined);
  const attempted = useRef(false);
  const working = useRef(false);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const [operation, setOperation] = useState<OwnedNavigation>();
  const [page, setPage] = useState(0);
  const selected = useRef<ReviewDestination | undefined>(undefined);
  const [destination, setDestination] = useState<{
    reader: ReviewDestination;
    path: string;
  }>();
  const [, refresh] = useState(0);
  useLayoutEffect(() => {
    const lifetime = new AbortController();
    observer.current = lifetime;
    return () => {
      lifetime.abort();
      void selected.current?.close();
    };
  }, []);

  const run = (action: (signal: AbortSignal) => Promise<unknown>) => {
    const signal = observer.current?.signal;
    if (!signal || signal.aborted || context.contextLost || working.current) {
      return;
    }
    working.current = true;
    setBusy(true);
    setFailed(false);
    void action(signal).catch(() => {
      if (!signal.aborted) setFailed(true);
    }).finally(() => {
      working.current = false;
      if (!signal.aborted) {
        setBusy(false);
        refresh((value) => value + 1);
      }
    });
  };
  if (context.contextLost || operation?.view().contextLost) {
    return (
      <Alert severity="info">
        Original access ended. Target text and actions are unavailable.
      </Alert>
    );
  }
  const view = operation?.view();
  const targets = operation?.targets() ?? [];
  const start = Math.min(page, Math.max(0, Math.ceil(targets.length / 5) - 1)) *
    5;
  const targetView = destination?.reader.view();
  return (
    <Stack spacing={1} data-review-navigation>
      {!operation && (
        <Stack direction="row" useFlexGap flexWrap="wrap" gap={1}>
          {kinds.map(([kind, label]) => (
            <Button
              key={kind}
              size="small"
              disabled={busy || attempted.current || !intelligence.identity}
              onClick={() => {
                if (attempted.current) return;
                attempted.current = true;
                run(async (signal) => {
                  const next = await intelligence.prepareNavigation(
                    point,
                    kind,
                    signal,
                  );
                  if (!signal.aborted) setOperation(next);
                });
              }}
            >
              Prepare {label}
            </Button>
          ))}
        </Stack>
      )}
      {view && (
        <>
          <Typography variant="caption" role="status">
            Navigation · {view.busy
              ? "working"
              : !view.fresh
              ? "outcome unconfirmed"
              : view.observation.state}
            {view.observation.pending ? " · pending" : ""}
          </Typography>
          <Stack direction="row" useFlexGap flexWrap="wrap" gap={1}>
            <Button
              disabled={busy || !view.canExecute}
              onClick={() => run((signal) => operation!.execute(signal))}
            >
              Acquire targets
            </Button>
            <Button
              disabled={busy || !view.canInspect}
              onClick={() => run((signal) => operation!.observe(signal))}
            >
              Check navigation
            </Button>
            <Button
              disabled={busy || !view.canRelease}
              onClick={() => run((signal) => operation!.release(signal))}
            >
              Release navigation
            </Button>
          </Stack>
          <Typography variant="caption" color="text.secondary">
            Acquire runs this query once. Release drops this navigation's
            ownership, not open target views or earlier language-server effects.
          </Typography>
          {view.observation.state === "retained" &&
            targets.slice(start, start + 5).map((target, index) => (
              <Button
                key={start + index}
                data-review-target-choice
                size="small"
                sx={{
                  textTransform: "none",
                  justifyContent: "flex-start",
                  overflowWrap: "anywhere",
                }}
                disabled={busy || !!destination ||
                  !view.canPrepareDestination ||
                  !!operation!.destination(target)}
                onClick={() =>
                  run(async () => {
                    const reader = createReviewDestination(operation!, target);
                    selected.current = reader;
                    setDestination({ reader, path: target.location.path });
                    await reader.start();
                  })}
              >
                Read {target.location.path}:{target.location.start.row + 1}
              </Button>
            ))}
          {view.observation.state === "retained" && targets.length > 5 && (
            <Stack direction="row" spacing={1} alignItems="center">
              <Button
                disabled={start === 0}
                onClick={() =>
                  setPage(start / 5 - 1)}
              >
                Previous targets
              </Button>
              <Typography variant="caption">
                {start + 1}–{Math.min(start + 5, targets.length)} of{" "}
                {targets.length}
              </Typography>
              <Button
                disabled={start + 5 >= targets.length}
                onClick={() =>
                  setPage(start / 5 + 1)}
              >
                Next targets
              </Button>
            </Stack>
          )}
        </>
      )}
      {failed && (
        <Alert severity="info">
          This action is unavailable. Nothing will be retried automatically.
          Check the original operation or review cleanup in Settings → About.
        </Alert>
      )}
      {destination && targetView && (
        <Stack spacing={1} data-review-destination={targetView.status}>
          <Stack
            direction="row"
            useFlexGap
            flexWrap="wrap"
            gap={1}
            alignItems="center"
          >
            <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>
              {destination.path}
            </Typography>
            <Button
              onClick={() => {
                void destination.reader.close();
                selected.current = undefined;
                setDestination(undefined);
              }}
            >
              Close target
            </Button>
            <Button
              disabled={busy || !targetView.canInspect}
              onClick={() =>
                run(() =>
                  destination.reader.inspect()
                )}
            >
              Check target
            </Button>
            {targetView.canOpen && (
              <Button
                disabled={busy}
                onClick={() => run(() => destination.reader.open())}
              >
                Open target
              </Button>
            )}
            {targetView.canRead && (
              <Button
                disabled={busy}
                onClick={() => run(() => destination.reader.read())}
              >
                Read target text
              </Button>
            )}
          </Stack>
          {targetView.displayed
            ? (
              <>
                <Typography variant="caption" color="text.secondary">
                  Verified navigation snapshot · read-only. This is not a live
                  disk view; no diagnostics or positional reads are inferred
                  from it.
                </Typography>
                <Box
                  sx={{
                    height: 360,
                    minHeight: 0,
                    overflow: "auto",
                    touchAction: settings.softWrap
                      ? "pan-y pinch-zoom"
                      : "pan-x pan-y pinch-zoom",
                  }}
                >
                  <Suspense fallback={<Typography>Loading reader…</Typography>}>
                    <CodeViewer
                      text={targetView.displayed.content.text}
                      kind="source"
                      path={destination.path}
                      softWrap={settings.softWrap}
                      fontSize={settings.codeFontSize}
                      revealLine={targetView.displayed.range.start.row + 1}
                      // Empty/EOF locations reveal a line, not a fabricated
                      // neighbouring character from the legacy pulse clamp.
                      revealRange={targetView.displayed.range.start.row ===
                            targetView.displayed.range.end.row &&
                          targetView.displayed.range.start.column ===
                            targetView.displayed.range.end.column
                        ? undefined
                        : targetView.displayed.range}
                      revealRequestId={targetView.displayed.range.id}
                      diagnostics={false}
                      inlayHints={false}
                      semanticHighlighting={false}
                      scrollRestoreKey="owned-navigation-target"
                      onScrollTopChange={() => undefined}
                    />
                  </Suspense>
                </Box>
              </>
            )
            : (
              <Typography variant="body2" role="status">
                {busy
                  ? "Reading original target…"
                  : targetView.status === "mismatch" ||
                      targetView.status === "stale"
                  ? "Target text changed. No text or old position was displayed."
                  : "No verified complete target text. Check the original target; it will not be reopened automatically."}
              </Typography>
            )}
        </Stack>
      )}
      {attempted.current && (
        <Button
          size="small"
          onClick={() => openAppSettings({ tab: "info", section: "code" })}
        >
          Review cleanup
        </Button>
      )}
    </Stack>
  );
}
