import { useEffect, useId, useRef, useState } from "react";
import { useStore } from "@cowboy/state-store";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Button,
  Stack,
  Typography,
} from "@mui/material";
import { ExpandMore } from "@mui/icons-material";
import { ConfirmSheet } from "./Sheet";
import { productCodeBuffers } from "./codeBuffers/product.ts";
import type {
  CleanupHandle,
  CleanupRow,
  CleanupStatus,
  CodeBufferCleanup,
} from "./codeBuffers/cleanup.ts";

const labels = {
  working: "Waiting for the original operation",
  context_lost: "Original access ended",
  release_uncertain: "Release not confirmed — status checks only",
  unavailable: "Current status unavailable",
  unknown: "Outcome unknown",
  pending: "Operation pending — cleanup was not queued",
  synchronization: "Resolve and retire the original synchronization first",
  navigation: "Inspect and release the original navigation first",
  destination: "Inspect the original navigation to resolve target handoff",
  needs_cleanup: "Cleanup still required",
} satisfies Record<CleanupStatus, string>;
const PAGE_SIZE = 5;

/** A view of retained core owners. Mounting, expanding and paging are local. */
export function CodeBufferCleanupPanel({
  source = productCodeBuffers.cleanup,
}: {
  source?: CodeBufferCleanup;
} = {}): React.JSX.Element | null {
  // A caller replacing the core source must explicitly remount this entry.
  const [mountedSource] = useState(source);
  if (source !== mountedSource) return null;
  return <CleanupPanel source={mountedSource} />;
}

function CleanupPanel({ source }: { source: CodeBufferCleanup }) {
  const snapshot = useStore(source);
  const id = useId();
  const [requestedPage, setPage] = useState(0);
  const [selected, setSelected] = useState<CleanupRow | null>(null);
  const [message, setMessage] = useState(false);
  const lifetime = useRef<AbortController | null>(null);
  const inFlight = useRef(new Set<CleanupHandle>());
  useEffect(() => {
    const observer = new AbortController();
    lifetime.current = observer;
    return () => {
      observer.abort(); // detach this view, never cancel the core continuation
      lifetime.current = null;
    };
  }, [source]);
  const page = Math.min(
    requestedPage,
    Math.max(0, Math.ceil(snapshot.rows.length / PAGE_SIZE) - 1),
  );
  const start = page * PAGE_SIZE;
  const current = snapshot.rows.find((row) => row.handle === selected?.handle);
  const confirmable = !!current?.canContinue && !snapshot.contextLost;

  function run(row: CleanupRow, cleanup: boolean) {
    const observer = lifetime.current?.signal;
    if (!observer || observer.aborted || inFlight.current.has(row.handle)) {
      return;
    }
    const claimed = inFlight.current;
    claimed.add(row.handle);
    setMessage(false);
    const work = cleanup
      ? source.continueCleanup(row.handle)
      : source.inspect(row.handle, observer);
    void work.catch(() => {
      if (!observer.aborted) setMessage(true);
    }).finally(() => {
      claimed.delete(row.handle);
    });
  }

  if (!snapshot.active && !snapshot.rows.length) return null;
  return (
    <>
      <Accordion disableGutters data-code-cleanup="panel">
        <AccordionSummary
          expandIcon={<ExpandMore />}
          id={`${id}-summary`}
          aria-controls={`${id}-details`}
        >
          <Typography variant="body2">
            Code resources · {snapshot.rows.length} awaiting cleanup
          </Typography>
        </AccordionSummary>
        <AccordionDetails id={`${id}-details`}>
          <Stack spacing={1.5}>
            <Typography variant="caption" color="text.secondary">
              This page only · {snapshot.active}{" "}
              active. Active views are not closed here. Reloading does not
              recover or release older resources.
            </Typography>
            {snapshot.contextLost
              ? (
                <Alert severity="warning">
                  Original access ended. Details and actions are unavailable;
                  these resources have not been confirmed released. A new login
                  cannot take over this page's owners.
                </Alert>
              )
              : (
                <Typography variant="caption" color="text.secondary">
                  Check status only queries the original resource. Continue
                  cleanup performs one bounded pass; it never reopens a file,
                  undoes edits, or repeats an uncertain release.
                </Typography>
              )}
            {message && !snapshot.contextLost && (
              <Alert severity="info">
                The action is unavailable. Review the current status; nothing
                will be retried automatically.
              </Alert>
            )}
            {snapshot.rows.slice(start, start + PAGE_SIZE).map((row) => (
              <Stack
                key={row.ordinal}
                spacing={0.5}
                data-code-cleanup="row"
                sx={{ minWidth: 0, overflowWrap: "anywhere" }}
              >
                <Typography variant="body2">
                  Buffer {row.ordinal}
                  {row.target && ` · ${row.target.path}`}
                </Typography>
                <Typography variant="caption" role="status">
                  {labels[row.status]}
                </Typography>
                {row.target && (
                  <Typography variant="caption" color="text.secondary">
                    Session {row.target.sessionId}
                  </Typography>
                )}
                {!snapshot.contextLost && (
                  <Stack direction="row" useFlexGap flexWrap="wrap" spacing={1}>
                    <Button
                      disabled={!row.canInspect}
                      onClick={() =>
                        run(row, false)}
                    >
                      Check status
                    </Button>
                    {row.canContinue && (
                      <Button onClick={() => setSelected(row)}>
                        Continue cleanup…
                      </Button>
                    )}
                  </Stack>
                )}
              </Stack>
            ))}
            {snapshot.rows.length > PAGE_SIZE && (
              <Stack direction="row" alignItems="center" spacing={1}>
                <Button
                  disabled={page === 0}
                  onClick={() =>
                    setPage(page - 1)}
                >
                  Previous
                </Button>
                <Typography variant="caption">
                  {start + 1}–{Math.min(
                    start + PAGE_SIZE,
                    snapshot.rows.length,
                  )} of {snapshot.rows.length}
                </Typography>
                <Button
                  disabled={start + PAGE_SIZE >= snapshot.rows.length}
                  onClick={() =>
                    setPage(page + 1)}
                >
                  Next
                </Button>
              </Stack>
            )}
          </Stack>
        </AccordionDetails>
      </Accordion>
      <ConfirmSheet
        open={selected !== null && !snapshot.contextLost}
        onClose={() => setSelected(null)}
        title="Continue buffer cleanup?"
        actions={
          <>
            <Button onClick={() => setSelected(null)}>Cancel</Button>
            <Button
              variant="contained"
              disabled={!confirmable}
              onClick={() => {
                if (!selected || !confirmable) return;
                const original = selected;
                setSelected(null);
                run(original, true);
              }}
            >
              Continue cleanup
            </Button>
          </>
        }
      >
        {!snapshot.contextLost && selected && (
          <Stack spacing={1} sx={{ overflowWrap: "anywhere" }}>
            <Typography>
              Buffer {selected.ordinal} · {selected.target?.path}
            </Typography>
            <Typography variant="body2">
              Session {selected.target?.sessionId}
            </Typography>
            <Typography variant="body2">
              Check this original owner's outcome and release it only when its
              state permits. Pending or unknown results remain visible. This
              does not delete files, close sessions, or undo earlier work.
            </Typography>
            {!confirmable && (
              <Alert severity="info">
                The state changed. Close this confirmation and review the
                current evidence.
              </Alert>
            )}
          </Stack>
        )}
      </ConfirmSheet>
    </>
  );
}
