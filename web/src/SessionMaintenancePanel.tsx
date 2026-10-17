import { useEffect, useRef, useState } from "react";
import { orderedItems } from "./aboutPreferences";
import { Alert, Box, Divider, FormControlLabel, Stack, Switch, Typography } from "@mui/material";
import { expectHttpOk } from "./httpResponse";
import { lowPriorityInterval } from "./network/bandwidth";
import { machineResourcesCaption, type MachineResourceSummary } from "./machineResources";
import { MachineResourceOverview } from "./MachineResourceOverview";

export interface MaintenancePolicy {
  memory_reclaim: boolean;
  reclaim_on_capacity: boolean;
  auto_update_idle_sessions: boolean;
}

export interface MaintenanceMachine {
  machine: MachineResourceSummary;
  can_configure: boolean;
  policy: MaintenancePolicy;
  sessions: number;
  working: number;
  warm: number;
  sleeping: number;
  starting: number;
  updating: number;
  waiting: { id: string; title: string; reason: string; paused: boolean }[];
}

const controls: [keyof MaintenancePolicy, string][] = [
  ["memory_reclaim", "Sleep idle sessions at low memory"],
  ["reclaim_on_capacity", "Free idle workers for queued messages"],
  ["auto_update_idle_sessions", "Update idle Providers automatically"],
];

export function SessionMaintenancePanel({ order = [] }: { order?: string[] } = {}): React.JSX.Element {
  const [rows, setRows] = useState<MaintenanceMachine[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState<string>();
  const [now, setNow] = useState(Date.now);
  const revision = useRef(0);
  useEffect(() => {
    const controller = new AbortController();
    let loading = false;
    const load = async (): Promise<void> => {
      if (loading) return;
      loading = true;
      setNow(Date.now());
      const observed = revision.current;
      try {
        const response = await fetch("/api/session-maintenance", { signal: controller.signal });
        await expectHttpOk(response, "Could not load automatic maintenance");
        const next = await response.json() as MaintenanceMachine[];
        if (!controller.signal.aborted && observed === revision.current) { setRows(next); setLoaded(true); setNow(Date.now()); setError(undefined); }
      } catch (cause) {
        if (!controller.signal.aborted) setError(String(cause));
      } finally { loading = false; }
    };
    void load();
    const stop = lowPriorityInterval(() => { void load(); }, 15_000);
    return () => { controller.abort(); stop(); };
  }, []);

  const update = async (row: MaintenanceMachine, key: keyof MaintenancePolicy, value: boolean): Promise<void> => {
    setPending(row.machine.id);
    revision.current += 1;
    try {
      const response = await fetch(`/api/machines/${encodeURIComponent(row.machine.id)}/maintenance`, {
        method: "PUT", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ [key]: value }),
      });
      await expectHttpOk(response, "Could not save automatic maintenance");
      const policy = await response.json() as MaintenancePolicy;
      revision.current += 1;
      setRows((previous) => previous.map((item) => item.machine.id === row.machine.id ? { ...item, policy } : item));
      setError(undefined);
    } catch (cause) { setError(String(cause)); }
    finally { setPending(undefined); }
  };

  return (
    <Stack spacing={1.5} data-session-maintenance>
      <Typography variant="caption" color="text.secondary">
        Session history has no worker limit. Workers start only when needed.
        Sleeping sessions adopt updates without starting a worker.
      </Typography>
      {error && <Alert severity="error">{error}</Alert>}
      {!rows.length && !error && <Typography variant="body2">{loaded ? "No Machines enrolled." : "Loading…"}</Typography>}
      <Box sx={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(min(100%, 320px), 1fr))", gap: 1.5, alignItems: "start" }}>
      {orderedItems(rows, order, (row) => row.machine.id).map((row) => (
        <Stack key={row.machine.id} spacing={1.25} data-machine-resources={row.machine.id}
          sx={{ border: 1, borderColor: "divider", borderRadius: 1.5, p: 1.5, minWidth: 0 }}>
          <Box>
            <Typography variant="body2" fontWeight={650} sx={{ overflowWrap: "anywhere" }}>{row.machine.display_name}</Typography>
            <Typography variant="caption" color="text.secondary">{machineResourcesCaption(row.machine, now)}</Typography>
            {row.machine.resources && (now - row.machine.resources.observed_at_ms > 60_000 || !row.machine.connected) && <Typography variant="caption" color="text.secondary" display="block">Last reported resources · not a live reading</Typography>}
          </Box>
          <MachineResourceOverview row={row} />
          {row.waiting.length > 0 && <Box sx={{ borderLeft: 2, borderColor: "warning.main", pl: 1 }}>
            <Typography variant="caption" fontWeight={650}>{row.waiting.length} queued sessions</Typography>
            {row.waiting.map((item) => (
              <Typography key={item.id} variant="caption" color="text.secondary" display="block" sx={{ overflowWrap: "anywhere" }}>
                {item.title}: {item.paused ? "Queue paused" : item.reason}.
              </Typography>
            ))}
            <Typography variant="caption" color="text.secondary">Pause or cancel in the session queue.</Typography>
          </Box>}
          <Divider />
          <Box>
          <Typography variant="caption" fontWeight={650} color="text.secondary">Automatic maintenance</Typography>
          {controls.map(([key, label]) => (
            <FormControlLabel key={key} label={label}
              slotProps={{ typography: { variant: "body2" } }}
              sx={{ m: 0, display: "flex", justifyContent: "space-between", minHeight: 44, gap: 1 }} labelPlacement="start"
              control={<Switch size="small" checked={row.policy[key]} disabled={!row.can_configure || pending !== undefined}
                onChange={(_, checked) => { void update(row, key, checked); }} />} />
          ))}
          </Box>
        </Stack>
      ))}
      </Box>
    </Stack>
  );
}
