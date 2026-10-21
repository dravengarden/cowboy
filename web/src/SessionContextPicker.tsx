import { useRef, useState } from "react";
import {
  Autocomplete,
  Box,
  Button,
  Checkbox,
  Chip,
  IconButton,
  InputAdornment,
  MenuList,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import {
  CheckBoxOutlineBlank,
  CheckBoxOutlined,
  ChevronRight,
  Close,
  Search,
} from "@mui/icons-material";
import { isManagedChild } from "./managedCalls";
import { useStoreSelector } from "./store";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { sessionAccountIdentity } from "./accountIdentity";
import { useSurfaceProfile } from "./surface/SurfaceProfile";
import { Sheet } from "./Sheet";
import { ReliableListItemButton } from "./ReliableListItemButton";
import { useDialogFocus } from "./useDialogInputFocus";
import { isImeKeyEvent } from "./imeKey";

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
  const desktop = useSurfaceProfile().kind === "desktop";
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const choices = sessions.filter((session) =>
    !session.system && !session.closing && !isManagedChild(session) &&
    session.id !== exclude
  );
  const byId = new Map(choices.map((session) => [session.id, session]));
  // Keep rows stationary as the user selects them; only missing selections
  // need extra rows. Selection order remains in value, not the browsing list.
  const ids = [...new Set([...choices.map((session) => session.id), ...value])];
  const label = (id: string) => byId.get(id)?.title || id;
  const identityLabel = (id: string) => {
    const session = byId.get(id);
    return session
      ? `${label(id)} · ${sessionAccountIdentity(accounts, session).short}`
      : label(id);
  };
  const detail = (id: string) => {
    const session = byId.get(id);
    return session
      ? `${session.provider} · ${
        sessionAccountIdentity(accounts, session).short
      } · ${session.workspace_name || session.machine_id}`
      : "No longer available · remove from selection";
  };
  // A missing source must remain removable, even though it cannot be added.
  const unavailable = (id: string) =>
    !value.includes(id) &&
    (!byId.has(id) || value.length >= 32);
  if (!desktop) {
    const close = () => {
      setOpen(false);
      trigger.current?.focus({ preventScroll: true });
    };
    return (
      <Stack
        spacing={1}
        data-context-picker
        onKeyDown={(event) => {
          if (!open) return;
          // Includes the portalled footer. Escape must never reach the parent
          // Settings sheet's window listener and dismiss both layers.
          if (isImeKeyEvent(event.nativeEvent)) event.stopPropagation();
          else if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            close();
          }
        }}
      >
        <Button
          ref={trigger}
          variant="outlined"
          disableFocusRipple
          disabled={disabled}
          aria-haspopup="dialog"
          aria-expanded={open}
          endIcon={<ChevronRight />}
          onClick={() => setOpen(true)}
          sx={{
            minHeight: 48,
            justifyContent: "space-between",
            textTransform: "none",
            color: "text.primary",
            borderColor: "divider",
            "&.Mui-focusVisible": {
              outline: "2px solid",
              outlineColor: "primary.main",
              outlineOffset: 2,
            },
          }}
        >
          Context from sessions{value.length ? ` · ${value.length}` : ""}
        </Button>
        {value.length > 0 && (
          <Stack direction="row" useFlexGap flexWrap="wrap" gap={0.75}>
            {value.map((id) => (
              <Chip
                key={id}
                size="small"
                label={identityLabel(id)}
                disabled={disabled}
                onDelete={() =>
                  onChange(value.filter((value) => value !== id))}
                sx={{ maxWidth: "100%" }}
              />
            ))}
          </Stack>
        )}
        {open && (
          <Sheet
            open
            forceSheet
            portal
            fill
            title="Context from sessions"
            onClose={close}
            actions={
              <Button fullWidth variant="contained" onClick={close}>
                Done
              </Button>
            }
          >
            <MobileSessionChoices
              ids={ids}
              value={value}
              label={label}
              detail={detail}
              unavailable={unavailable}
              disabled={disabled}
              onChange={onChange}
              onClose={close}
            />
          </Sheet>
        )}
      </Stack>
    );
  }
  return (
    <Autocomplete
      data-context-picker
      multiple
      disableCloseOnSelect
      options={ids}
      value={value}
      disabled={disabled}
      getOptionLabel={identityLabel}
      getOptionDisabled={unavailable}
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
        return (
          <li {...props} key={id}>
            <Checkbox checked={state.selected} tabIndex={-1} size="small" />
            <Stack sx={{ minWidth: 0 }}>
              <Typography sx={{ overflowWrap: "anywhere" }}>
                {label(id)}
              </Typography>
              <Typography variant="caption" color="text.secondary">
                {detail(id)}
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
          helperText={value.length >= 32
            ? "32 sessions selected · limit reached"
            : undefined}
        />
      )}
      sx={{ minWidth: 0, "& .MuiAutocomplete-inputRoot": { flexWrap: "wrap" } }}
    />
  );
}

/** Touch starts with a browsable list, not a focused input. The portalled
 * sheet owns the single scroll area and keyboard inset; search stays at its
 * top and Done stays above the keyboard. Closing retains the form selection,
 * just like the Desktop multiselect; only the owning form saves it. */
function MobileSessionChoices({
  ids,
  value,
  label,
  detail,
  unavailable,
  disabled,
  onChange,
  onClose,
}: {
  ids: string[];
  value: string[];
  label: (id: string) => string;
  detail: (id: string) => string;
  unavailable: (id: string) => boolean;
  disabled: boolean;
  onChange: (value: string[]) => void;
  onClose: () => void;
}): React.JSX.Element {
  const [query, setQuery] = useState("");
  const panel = useRef<HTMLDivElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLUListElement>(null);
  useDialogFocus(() => panel.current, false);
  const needle = query.trim().toLocaleLowerCase();
  const matches = ids.filter((id) =>
    `${label(id)} ${detail(id)}`.toLocaleLowerCase().includes(needle)
  );
  const tabStop = matches.find((id) => value.includes(id)) ??
    matches.find((id) => !unavailable(id));
  return (
    <Box
      ref={panel}
      tabIndex={-1}
      data-context-choices
      sx={{ outline: "none", pb: 1 }}
      onKeyDown={(event) => {
        if (isImeKeyEvent(event.nativeEvent)) {
          event.stopPropagation();
        } else if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          if (document.activeElement === search.current) panel.current?.focus();
          else onClose();
        }
      }}
    >
      <Box
        sx={{
          position: "sticky",
          top: 0,
          zIndex: 1,
          bgcolor: "background.paper",
          pt: 1,
        }}
      >
        <TextField
          inputRef={search}
          fullWidth
          value={query}
          placeholder="Search sessions"
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (isImeKeyEvent(event.nativeEvent)) return;
            if (event.key === "Enter" || event.key === "ArrowDown") {
              event.preventDefault();
              list.current?.querySelector<HTMLElement>(
                '[role="option"]:not([aria-disabled="true"])',
              )?.focus();
            }
          }}
          slotProps={{
            htmlInput: {
              "aria-label": "Search context sessions",
              enterKeyHint: "search",
            },
            input: {
              sx: { fontSize: "max(16px, 1rem)" },
              startAdornment: (
                <InputAdornment position="start">
                  <Search fontSize="small" />
                </InputAdornment>
              ),
              endAdornment: query && (
                <InputAdornment position="end">
                  <IconButton
                    aria-label="Clear search"
                    onPointerDown={(event) => event.preventDefault()}
                    onClick={() => setQuery("")}
                  >
                    <Close fontSize="small" />
                  </IconButton>
                </InputAdornment>
              ),
            },
          }}
        />
        <Stack
          direction="row"
          alignItems="center"
          justifyContent="space-between"
          sx={{ minHeight: 44 }}
        >
          <Typography variant="caption" color="text.secondary" role="status">
            {value.length
              ? `${value.length} of 32 selected`
              : "Choose sessions to read when needed"}
          </Typography>
          {value.length > 0 && (
            <Button
              size="small"
              disabled={disabled}
              onPointerDown={(event) => event.preventDefault()}
              onClick={() => onChange([])}
            >
              Clear
            </Button>
          )}
        </Stack>
      </Box>
      <MenuList
        ref={list}
        role="listbox"
        aria-label="Context sessions"
        aria-multiselectable
        disablePadding
      >
        {matches.map((id) => {
          const selected = value.includes(id);
          return (
            <ReliableListItemButton
              key={id}
              role="option"
              tabIndex={id === tabStop ? 0 : -1}
              aria-selected={selected}
              selected={selected}
              disabled={disabled || unavailable(id)}
              onPointerDown={(event) => {
                // Keep search and its keyboard stationary while picking more
                // than one result. The footer owns explicit keyboard dismissal.
                if (document.activeElement === search.current) {
                  event.preventDefault();
                }
              }}
              onActivate={() => {
                if (disabled || unavailable(id)) return;
                onChange(
                  selected
                    ? value.filter((value) => value !== id)
                    : [...value, id],
                );
              }}
              sx={{
                minHeight: 56,
                borderRadius: 1.5,
                px: 1,
                gap: 1.5,
                my: 0.25,
              }}
            >
              {selected
                ? <CheckBoxOutlined color="primary" />
                : <CheckBoxOutlineBlank color="action" />}
              <Stack sx={{ minWidth: 0, overflowWrap: "anywhere" }}>
                <Typography variant="body2">{label(id)}</Typography>
                <Typography variant="caption" color="text.secondary">
                  {detail(id)}
                </Typography>
              </Stack>
            </ReliableListItemButton>
          );
        })}
      </MenuList>
      {!matches.length && (
        <Typography color="text.secondary" sx={{ py: 3 }} role="status">
          {needle ? "No matching sessions" : "No other sessions yet"}
        </Typography>
      )}
    </Box>
  );
}
