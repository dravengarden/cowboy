import {
  Autocomplete,
  Checkbox,
  Chip,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { isManagedChild } from "./managedCalls";
import { useStoreSelector } from "./store";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { sessionAccountIdentity } from "./accountIdentity";

/** One selectable row per real session. Names are presentation, ids are the
 * references; the Service rechecks ownership on save, send and history reads. */
export function SessionContextPicker(
  { value, onChange, disabled = false, exclude }: {
    value: string[];
    onChange: (value: string[]) => void;
    disabled?: boolean;
    exclude?: string | undefined;
  },
): React.JSX.Element {
  const sessions = useStoreSelector((state) => state.sessions);
  const { data: accounts } = useSubscriptionAccounts();
  const choices = sessions.filter((session) =>
    !session.system && !session.closing && !isManagedChild(session) &&
    session.id !== exclude
  );
  const ids = [...new Set([...value, ...choices.map((session) => session.id)])];
  const label = (id: string) =>
    choices.find((session) => session.id === id)?.title || id;
  const identityLabel = (id: string) => {
    const session = choices.find((candidate) => candidate.id === id);
    return session
      ? `${label(id)} · ${sessionAccountIdentity(accounts, session).short}`
      : label(id);
  };
  return (
    <Autocomplete
      multiple
      disableCloseOnSelect
      options={ids}
      value={value}
      disabled={disabled}
      getOptionLabel={identityLabel}
      getOptionDisabled={(id) =>
        !choices.some((session) => session.id === id) ||
        (!value.includes(id) && value.length >= 32)}
      onChange={(_, ids) => onChange(ids)}
      renderValue={(ids, getItemProps) =>
        ids.map((id, index) => (
          <Chip
            {...getItemProps({ index })}
            key={id}
            size="small"
            label={identityLabel(id)}
            sx={{ maxWidth: "100%" }}
          />
        ))}
      renderOption={(props, id, state) => {
        const session = choices.find((choice) => choice.id === id);
        return (
          <li {...props} key={id}>
            <Checkbox checked={state.selected} tabIndex={-1} size="small" />
            <Stack sx={{ minWidth: 0 }}>
              <Typography sx={{ overflowWrap: "anywhere" }}>
                {label(id)}
              </Typography>
              <Typography variant="caption" color="text.secondary">
                {session
                  ? `${session.provider} · ${
                    sessionAccountIdentity(accounts, session).short
                  } · ${session.workspace_name || session.machine_id}`
                  : "No longer available"}
              </Typography>
            </Stack>
          </li>
        );
      }}
      renderInput={(params) => (
        <TextField
          {...params}
          label="Context from sessions"
          placeholder={value.length
            ? "Add another session"
            : "Search sessions · optional"}
          helperText="Optional · up to 32 sessions, read on demand."
        />
      )}
      sx={{ minWidth: 0, "& .MuiAutocomplete-inputRoot": { flexWrap: "wrap" } }}
    />
  );
}
