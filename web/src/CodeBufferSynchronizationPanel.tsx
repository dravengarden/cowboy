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
  SynchronizationAction,
  SynchronizationConfirmation,
} from "./codeBuffers/synchronization.ts";
import type {
  CodeBufferSynchronizations,
  SynchronizationHandle,
  SynchronizationRow,
  SynchronizationStatus,
} from "./codeBuffers/synchronizationProjection.ts";

const labels = {
  working: "Waiting for the original operation",
  context_lost: "Original access ended",
  retirement_uncertain: "Retirement not confirmed — status checks only",
  unavailable: "Current status unavailable — check the original operation",
  unknown: "Outcome unknown — do not repeat or undo the refresh",
  pending: "Operation pending — no new action was queued",
  prepared: "Prepared only — native text has not been refreshed",
  applied: "Refresh confirmed — retire this operation before continuing",
  changed: "Refresh refused — text or ownership changed",
  source: "Refresh refused — disk source unavailable or unsuitable",
  shared: "Refresh refused — another owner shares the native buffer",
} satisfies Record<SynchronizationStatus, string>;
const PAGE_SIZE = 5;
interface Selection {
  readonly row: SynchronizationRow;
  readonly action: SynchronizationAction;
  readonly token: SynchronizationConfirmation;
}

/** Existing core continuations only; opening Settings never prepares a refresh. */
export function CodeBufferSynchronizationPanel({
  source = productCodeBuffers.synchronizations,
}: { source?: CodeBufferSynchronizations } = {}): React.JSX.Element | null {
  const [original] = useState(source);
  if (original !== source) return null;
  return <SynchronizationPanel source={original} />;
}

function SynchronizationPanel(
  { source }: { source: CodeBufferSynchronizations },
) {
  const snapshot = useStore(source);
  const id = useId();
  const [requestedPage, setPage] = useState(0);
  const [selected, setSelected] = useState<Selection | null>(null);
  const [message, setMessage] = useState(false);
  const lifetime = useRef<AbortController | null>(null);
  const inFlight = useRef(new Set<SynchronizationHandle>());
  useEffect(() => {
    const observer = new AbortController();
    lifetime.current = observer;
    return () => {
      observer.abort(); // detach only; core owns admitted work
      lifetime.current = null;
    };
  }, [source]);
  const page = Math.min(
    requestedPage,
    Math.max(0, Math.ceil(snapshot.rows.length / PAGE_SIZE) - 1),
  );
  const start = page * PAGE_SIZE;
  const confirmable = !!selected && !snapshot.contextLost &&
    source.isCurrent(selected.row.handle, selected.token);

  function preview(row: SynchronizationRow, action: SynchronizationAction) {
    try {
      setSelected({ row, action, token: source.preview(row.handle, action) });
      setMessage(false);
    } catch {
      setMessage(true);
    }
  }
  function run(row: SynchronizationRow, token?: SynchronizationConfirmation) {
    const observer = lifetime.current?.signal;
    if (!observer || observer.aborted || inFlight.current.has(row.handle)) {
      return;
    }
    const claimed = inFlight.current;
    claimed.add(row.handle);
    setMessage(false);
    const work = token
      ? source.confirm(row.handle, token, observer)
      : source.inspect(row.handle, observer);
    void work.catch(() => {
      if (!observer.aborted) setMessage(true);
    }).finally(() => claimed.delete(row.handle));
  }

  if (!snapshot.rows.length) return null;
  return (
    <>
      <Accordion disableGutters data-code-sync="panel">
        <AccordionSummary
          expandIcon={<ExpandMore />}
          id={`${id}-summary`}
          aria-controls={`${id}-details`}
        >
          <Typography variant="body2">
            Code synchronization · {snapshot.rows.length} retained
          </Typography>
        </AccordionSummary>
        <AccordionDetails id={`${id}-details`}>
          <Stack spacing={1.5}>
            <Typography variant="caption" color="text.secondary">
              This page only. These original operations survive closing a view,
              not reloading the page. There is no automatic retry, undo, or
              recovery on a new login.
            </Typography>
            {snapshot.contextLost && (
              <Alert severity="warning">
                Original access ended. Details and actions are hidden. Missing
                access is not proof that a refresh or retirement completed.
              </Alert>
            )}
            {message && !snapshot.contextLost && (
              <Alert severity="info">
                The action is unavailable. Review its status; nothing will be
                retried automatically.
              </Alert>
            )}
            {snapshot.rows.slice(start, start + PAGE_SIZE).map((row) => (
              <Stack
                key={row.ordinal}
                spacing={0.5}
                data-code-sync="row"
                sx={{ minWidth: 0, overflowWrap: "anywhere" }}
              >
                <Typography variant="body2">
                  Synchronization {row.ordinal}
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
                {row.content && (
                  <Typography variant="caption" color="text.secondary">
                    Captured text: {row.content.utf8Bytes} UTF-8 bytes · SHA-256
                    {" "}
                    {row.content.sha256}
                  </Typography>
                )}
                {!snapshot.contextLost && (
                  <Stack direction="row" useFlexGap flexWrap="wrap" spacing={1}>
                    <Button
                      disabled={!row.canInspect}
                      onClick={() =>
                        run(row)}
                    >
                      Check synchronization
                    </Button>
                    {row.canApply && (
                      <Button onClick={() => preview(row, "apply")}>
                        Review refresh…
                      </Button>
                    )}
                    {row.canRetire && (
                      <Button onClick={() => preview(row, "retire")}>
                        Retire operation…
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
        title={selected?.action === "apply"
          ? "Refresh native code buffer?"
          : "Retire synchronization?"}
        actions={
          <>
            <Button onClick={() => setSelected(null)}>Cancel</Button>
            <Button
              variant="contained"
              disabled={!confirmable}
              onClick={() => {
                if (
                  !selected ||
                  !source.isCurrent(selected.row.handle, selected.token)
                ) return;
                const original = selected;
                setSelected(null);
                run(original.row, original.token);
              }}
            >
              {selected?.action === "apply"
                ? "Refresh native buffer"
                : "Retire synchronization"}
            </Button>
          </>
        }
      >
        {selected && !snapshot.contextLost && (
          <Stack spacing={1} sx={{ overflowWrap: "anywhere" }}>
            <Typography>{selected.row.target?.path}</Typography>
            <Typography variant="body2">
              Session {selected.row.target?.sessionId}
            </Typography>
            <Typography variant="body2">
              Captured text: {selected.row.content?.utf8Bytes}{" "}
              UTF-8 bytes · SHA-256 {selected.row.content?.sha256}
            </Typography>
            <Typography variant="body2">
              {selected.action === "apply"
                ? "Refresh only this original native buffer from disk, and only if the disk matches this captured text. Dirty, changed or shared buffers are refused. This does not write the file or grant later edits. It cannot automatically undo an applied refresh."
                : "Retire only this original prepared or completed operation. This does not undo a refresh, delete a file or release its buffer. Pending and unknown effects cannot be discarded."}
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
