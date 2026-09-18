import { useEffect, useRef, useState } from "react";
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
import { ConfirmSheet } from "./Sheet.tsx";
import { productCodeBuffers } from "./codeBuffers/product.ts";
import type {
  CodeBufferNavigations,
  NavigationHandle,
  NavigationRow,
  NavigationStatus,
} from "./codeBuffers/navigationProjection.ts";

const labels = {
  working: "Waiting for the original operation",
  context_lost: "Original access ended",
  release_uncertain: "Release unconfirmed — checks only",
  unavailable: "Current status unavailable",
  unknown: "Acquisition outcome unknown — checks only",
  pending: "Original operation pending",
  prepared: "Prepared · no targets acquired",
  retained: "Targets retained · release still required",
} satisfies Record<NavigationStatus, string>;

/** Passive original-group recovery. No Execute, destination import or polling. */
export function CodeBufferNavigationPanel(
  { source = productCodeBuffers.navigations }: {
    source?: CodeBufferNavigations;
  } = {},
) {
  const [original] = useState(source);
  return original === source ? <NavigationPanel source={source} /> : null;
}
function NavigationPanel({ source }: { source: CodeBufferNavigations }) {
  const snapshot = useStore(source);
  const [selected, setSelected] = useState<NavigationRow>();
  const [page, setPage] = useState(0);
  const [failed, setFailed] = useState(false);
  const lifetime = useRef<AbortController | undefined>(undefined);
  const busy = useRef(new Set<NavigationHandle>());
  useEffect(() => {
    const observer = new AbortController();
    lifetime.current = observer;
    return () => observer.abort();
  }, []);
  const start =
    Math.min(page, Math.max(0, Math.ceil(snapshot.rows.length / 5) - 1)) * 5;
  const current = snapshot.rows.find((row) => row.handle === selected?.handle);
  const canRelease = !!current?.canRelease && !snapshot.contextLost;
  const run = (row: NavigationRow, release: boolean) => {
    const signal = lifetime.current?.signal;
    if (!signal || signal.aborted || busy.current.has(row.handle)) return;
    busy.current.add(row.handle);
    setFailed(false);
    void (release
      ? source.release(row.handle, signal)
      : source.inspect(row.handle, signal)).catch(() => {
        if (!signal.aborted) setFailed(true);
      }).finally(() => busy.current.delete(row.handle));
  };
  if (!snapshot.rows.length) return null;
  return (
    <>
      <Accordion disableGutters data-code-navigation="panel">
        <AccordionSummary expandIcon={<ExpandMore />}>
          <Typography variant="body2">
            Code navigation · {snapshot.rows.length} retained
          </Typography>
        </AccordionSummary>
        <AccordionDetails>
          <Stack spacing={1.5}>
            <Typography variant="caption">
              This page only. Check queries the original group. Release drops
              its ownership, not open target views or prior language-server
              effects.
            </Typography>
            {snapshot.contextLost
              ? (
                <Alert severity="warning">
                  Original access ended. Details and actions are unavailable;
                  release is not confirmed.
                </Alert>
              )
              : (
                <>
                  {failed && (
                    <Alert severity="info">
                      Action unavailable. Nothing will be retried automatically.
                    </Alert>
                  )}
                  {snapshot.rows.slice(start, start + 5).map((row) => (
                    <Stack
                      key={row.ordinal}
                      spacing={0.5}
                      data-code-navigation="row"
                    >
                      <Typography
                        variant="body2"
                        sx={{ overflowWrap: "anywhere" }}
                      >
                        Navigation {row.ordinal} · {row.target?.path}
                      </Typography>
                      <Typography variant="caption" role="status">
                        {labels[row.status]}
                      </Typography>
                      <Stack direction="row" spacing={1}>
                        <Button
                          disabled={!row.canInspect}
                          onClick={() => run(row, false)}
                        >
                          Check navigation status
                        </Button>
                        <Button
                          disabled={!row.canRelease}
                          onClick={() => setSelected(row)}
                        >
                          Release navigation…
                        </Button>
                      </Stack>
                    </Stack>
                  ))}
                  {snapshot.rows.length > 5 && (
                    <Stack direction="row" spacing={1}>
                      <Button
                        disabled={start === 0}
                        onClick={() =>
                          setPage(start / 5 - 1)}
                      >
                        Previous
                      </Button>
                      <Typography variant="caption">
                        {start + 1}–{Math.min(start + 5, snapshot.rows.length)}
                        {" "}
                        of {snapshot.rows.length}
                      </Typography>
                      <Button
                        disabled={start + 5 >= snapshot.rows.length}
                        onClick={() =>
                          setPage(start / 5 + 1)}
                      >
                        Next
                      </Button>
                    </Stack>
                  )}
                </>
              )}
          </Stack>
        </AccordionDetails>
      </Accordion>
      <ConfirmSheet
        open={!!selected && !snapshot.contextLost}
        onClose={() => setSelected(undefined)}
        title="Release this navigation?"
        actions={
          <>
            <Button onClick={() => setSelected(undefined)}>Cancel</Button>
            <Button
              disabled={!canRelease}
              onClick={() => {
                if (!current || !canRelease) return;
                setSelected(undefined);
                run(current, true);
              }}
            >
              Release navigation
            </Button>
          </>
        }
      >
        <Typography>
          Release only this original navigation group. Independent target
          buffers still need their own cleanup. This is not rollback or proof
          that native buffers physically closed.
        </Typography>
      </ConfirmSheet>
    </>
  );
}
