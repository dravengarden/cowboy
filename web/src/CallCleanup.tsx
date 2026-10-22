import { Button, Checkbox, FormControlLabel, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { ConfirmSheet } from "./Sheet";
import { callActive, callModelLabel, callTitle, dismissCalls, type ManagedCallSummary, startedLabel } from "./managedCalls";
import { recordUndo } from "./undoActions";

/** Selection is frozen when opening: newly finished calls are never swept up. */
export function CallCleanup({ parent, calls, onChange }: {
  parent: string;
  calls: readonly ManagedCallSummary[];
  onChange: () => void;
}): React.JSX.Element {
  const [candidates, setCandidates] = useState<ManagedCallSummary[] | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const settled = calls.filter((call) => !callActive(call.state));
  const clear = async (ids: string[]): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      const changed = await dismissCalls(parent, ids, true);
      if (changed.length > 0) {
        recordUndo(
          changed.length === 1 ? "1 call cleared" : `${String(changed.length)} calls cleared`,
          [{ kind: "restore_calls", parent, ids: changed }],
        );
      }
      setCandidates(null);
      onChange();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not clear calls");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Stack spacing={0.5}>
      <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap">
        <Button disabled={busy || settled.length === 0} onClick={() => {
          setCandidates(settled);
          setSelected(settled.map((call) => call.call_id));
          setError(null);
        }}>Clear ended calls…</Button>
      </Stack>
      {error && <Typography variant="caption" color="error" role="alert">{error}</Typography>}
      <ConfirmSheet open={candidates !== null} onClose={() => { if (!busy) setCandidates(null); }}
        title="Clear calls from this list?"
        actions={<Stack direction="row" spacing={1}>
          <Button disabled={busy} onClick={() => setCandidates(null)}>Cancel</Button>
          <Button color="error" variant="contained" disabled={busy || selected.length === 0}
            onClick={() => void clear(selected)}>Clear {selected.length}</Button>
        </Stack>}
      >
        <Stack spacing={1}>
          <Typography variant="body2">Only selected ended calls leave this list. Results and call receipts are retained; code and the parent session stay intact.</Typography>
          <FormControlLabel label="Select all ended calls shown"
            control={<Checkbox disabled={busy} checked={selected.length === candidates?.length}
              indeterminate={selected.length > 0 && selected.length !== candidates?.length}
              onChange={(_, checked) => setSelected(checked ? candidates!.map((call) => call.call_id) : [])} />} />
          <Stack sx={{ maxHeight: "40vh", overflowY: "auto" }}>
            {candidates?.map((call) => <FormControlLabel key={call.call_id}
              label={<Stack sx={{ py: 0.5 }}>
                <Typography variant="body2">{callTitle(call)} · {call.state}</Typography>
                <Typography variant="caption" color="text.secondary">{callModelLabel(call)} · {startedLabel(call.created_at_ms)}</Typography>
              </Stack>}
              control={<Checkbox disabled={busy} checked={selected.includes(call.call_id)}
                onChange={(_, checked) => setSelected((ids) => checked ? [...ids, call.call_id] : ids.filter((id) => id !== call.call_id))} />} />)}
          </Stack>
          {error && <Typography color="error" role="alert">{error}</Typography>}
        </Stack>
      </ConfirmSheet>
    </Stack>
  );
}
