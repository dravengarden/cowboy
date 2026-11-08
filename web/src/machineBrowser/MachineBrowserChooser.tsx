/** The Machine choice for an account's first browser. An account that was
 * never signed in has no placement and no Machine holding its credentials,
 * so the Controller cannot pick one; the owner chooses among the connected
 * Machines that can run a browser now and the open is pinned there. See
 * docs/machine-browser.md, "Viewer (core web)". */
import { Button, MenuItem, Stack, TextField, Typography } from "@mui/material";
import { useState } from "react";
import { useStoreSelector } from "../store";
import type { BrowserRead } from "./api";
import { machineChoices } from "./entryRules";

export function MachineBrowserChooser({
  read,
  busy,
  onOpen,
}: {
  read: BrowserRead | undefined;
  busy: boolean;
  onOpen: (machine: string) => void;
}): React.JSX.Element {
  const machines = useStoreSelector((state) => state.machines);
  const choices = machineChoices(read, machines);
  const [picked, setPicked] = useState("");
  // A choice that is no longer offered is never opened.
  const machine = choices.some((choice) => choice.id === picked) ? picked : (choices[0]?.id ?? "");
  if (choices.length === 0) {
    return (
      <Typography variant="body2" role="status">
        No connected Machine can run a browser right now. Connect one, then refresh.
      </Typography>
    );
  }
  return (
    <Stack
      direction="row"
      spacing={1}
      alignItems="center"
      flexWrap="wrap"
      useFlexGap
      data-machine-browser-chooser
    >
      <TextField
        select
        size="small"
        label="Machine"
        value={machine}
        disabled={busy}
        onChange={(event) => setPicked(event.target.value)}
        helperText="This account has no Machine yet; its browser stays on the one you choose."
        sx={{ minWidth: 200 }}
      >
        {choices.map((choice) => (
          <MenuItem key={choice.id} value={choice.id}>
            {choice.label}
          </MenuItem>
        ))}
      </TextField>
      <Button variant="contained" disabled={busy || !machine} onClick={() => onOpen(machine)}>
        Open browser
      </Button>
    </Stack>
  );
}
