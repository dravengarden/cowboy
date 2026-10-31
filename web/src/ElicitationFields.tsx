import { useCallback, useEffect, useId, useState } from "react";
import {
  alpha,
  Box,
  Button,
  Checkbox,
  FormControlLabel,
  Radio,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import OpenInNew from "@mui/icons-material/OpenInNew";
import { approvalProseSx } from "./approvalCardStyle";
import {
  autoResolutionSecondsLeft,
  type Elicitation,
  ELICITATION_ACCEPT,
  ELICITATION_DECLINE,
  elicitationAnswer,
  type ElicitationChoice,
  elicitationComplete,
  type ElicitationField,
  formatCountdown,
  type ElicitationValue,
  type ElicitationValues,
  initialElicitationValues,
} from "./elicitation";

export interface ElicitationChoiceButton {
  /** The option id this button answers with; the form's is computed. */
  id: string;
  label: string;
  reject: boolean;
  disabled: boolean;
  run: () => void;
}

/** Form state for one elicitation, and the Submit/Decline (or Done/Decline)
 *  choices each approval surface draws with its own buttons. Surfaces are
 *  reused across requests, so the answers reset when the request changes. */
export function useElicitationForm(
  requestId: string,
  elicitation: Elicitation | null,
  respond: (optionId: string) => void,
): {
  values: ElicitationValues;
  setValue: (id: string, value: ElicitationValue) => void;
  choices: ElicitationChoiceButton[];
} {
  const fields = elicitation?.mode === "form" ? elicitation.fields : [];
  const [state, setState] = useState<{ requestId: string; values: ElicitationValues }>(
    () => ({ requestId, values: initialElicitationValues(fields) }),
  );
  const values = state.requestId === requestId
    ? state.values
    : initialElicitationValues(fields);
  const setValue = useCallback((id: string, value: ElicitationValue): void => {
    setState((previous) => ({
      requestId,
      values: {
        ...(previous.requestId === requestId
          ? previous.values
          : initialElicitationValues(fields)),
        [id]: value,
      },
    }));
  }, [fields, requestId]);
  if (!elicitation) return { values, setValue, choices: [] };
  const complete = elicitationComplete(fields, values);
  const choices: ElicitationChoiceButton[] = [
    {
      id: ELICITATION_ACCEPT,
      label: elicitation.mode === "url" ? "Done" : "Submit",
      reject: false,
      disabled: !complete,
      run: () => {
        if (!complete) return;
        respond(
          elicitation.mode === "url"
            ? ELICITATION_ACCEPT
            : elicitationAnswer(fields, values),
        );
      },
    },
    {
      id: ELICITATION_DECLINE,
      label: "Decline",
      reject: true,
      disabled: false,
      run: () => respond(ELICITATION_DECLINE),
    },
  ];
  return { values, setValue, choices };
}

/** The answer an approval surface marks busy: a form answer carries content
 *  after the `accept` option id. */
export function elicitationChoiceOf(optionId: string | null): string | null {
  if (optionId === null) return null;
  return optionId.startsWith(`${ELICITATION_ACCEPT}:`) ? ELICITATION_ACCEPT : optionId;
}

function FieldHeading({ field }: { field: ElicitationField }): React.JSX.Element {
  return (
    <Box sx={{ mb: 0.75 }}>
      <Typography variant="body2" sx={{ fontWeight: 650 }}>
        {field.title}
        {field.required && (
          <Box component="span" aria-hidden sx={{ color: "error.main", ml: 0.25 }}>*</Box>
        )}
      </Typography>
      {field.description && (
        <Typography variant="caption" sx={{ color: "text.secondary", display: "block" }}>
          {field.description}
        </Typography>
      )}
    </Box>
  );
}

/** When this client first saw each auto-resolving request. The worker sends
 *  no timestamp, so a reload restarts the count from the reload; the agent's
 *  own timer is authoritative and its resolution removes the form. */
const firstSeenAt = new Map<string, number>();
const FIRST_SEEN_LIMIT = 256;

function firstSeen(key: string, now: number): number {
  const known = firstSeenAt.get(key);
  if (known !== undefined) return known;
  if (firstSeenAt.size >= FIRST_SEEN_LIMIT) {
    const oldest = firstSeenAt.keys().next().value;
    if (oldest !== undefined) firstSeenAt.delete(oldest);
  }
  firstSeenAt.set(key, now);
  return now;
}

/** "Continues without your answer in 42s": the agent answers on its own. */
function AutoResolutionCountdown(
  { seenKey, autoResolutionMs }: { seenKey: string; autoResolutionMs: number },
): React.JSX.Element {
  const [now, setNow] = useState(() => Date.now());
  const since = firstSeen(seenKey, now);
  const left = autoResolutionSecondsLeft(autoResolutionMs, since, now);
  const due = left === 0;
  useEffect(() => {
    if (due) return undefined;
    const timer = globalThis.setInterval(() => setNow(Date.now()), 1000);
    return (): void => globalThis.clearInterval(timer);
  }, [due]);
  return (
    <Typography
      variant="caption"
      role="timer"
      data-elicitation-countdown={left}
      sx={{ color: left <= 10 ? "warning.main" : "text.secondary", display: "block" }}
    >
      {due
        ? "Continuing without your answer…"
        : `Continues without your answer in ${formatCountdown(left)}`}
    </Typography>
  );
}

/** An option's plain-text preview, collapsed until asked for. */
function ChoicePreview({ preview }: { preview: string }): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <Box sx={{ pl: 4, pb: 0.5 }} data-elicitation-preview={open ? "open" : "closed"}>
      <Button
        size="small"
        aria-expanded={open}
        aria-controls={id}
        onClick={(): void => setOpen((value) => !value)}
        sx={{ textTransform: "none", minWidth: 0, px: 0.5, py: 0, fontSize: "0.75rem" }}
      >
        {open ? "Hide preview" : "Show preview"}
      </Button>
      {open && (
        <Box
          id={id}
          component="pre"
          sx={{
            m: 0,
            mt: 0.25,
            px: 1,
            py: 0.75,
            maxHeight: 220,
            overflow: "auto",
            borderRadius: 1,
            border: 1,
            borderColor: "divider",
            bgcolor: "action.hover",
            fontFamily: "monospace",
            fontSize: "0.75rem",
            lineHeight: 1.45,
            whiteSpace: "pre",
          }}
        >
          {preview}
        </Box>
      )}
    </Box>
  );
}

// One line of choice title; the control box shares it so they align exactly.
const CHOICE_LINE = "1.375rem";

/** A selectable row: the whole tile is the hit target, the selected one is
 *  tinted, and the radio/checkbox sits on the title's first line. */
function ChoiceTile(
  { title, description, selected, disabled, control }: {
    title: string;
    description: string | null;
    selected: boolean;
    disabled: boolean;
    control: React.JSX.Element;
  },
): React.JSX.Element {
  return (
    <FormControlLabel
      disabled={disabled}
      data-elicitation-choice
      data-selected={selected ? "true" : undefined}
      control={control}
      label={
        <Box sx={{ minWidth: 0 }}>
          <Typography
            variant="body2"
            sx={{ lineHeight: CHOICE_LINE, fontWeight: selected ? 600 : 400 }}
          >
            {title}
          </Typography>
          {description && (
            <Typography
              variant="caption"
              sx={{ color: "text.secondary", display: "block", lineHeight: 1.4 }}
            >
              {description}
            </Typography>
          )}
        </Box>
      }
      sx={(t) => ({
        display: "flex",
        alignItems: "flex-start",
        gap: 1,
        m: 0,
        px: 1.25,
        py: 1,
        borderRadius: 1.5,
        border: 1,
        borderColor: selected ? alpha(t.palette.primary.main, 0.55) : "divider",
        bgcolor: selected
          ? alpha(t.palette.primary.main, t.palette.mode === "dark" ? 0.18 : 0.08)
          : alpha(t.palette.background.paper, 0.6),
        transition: "background-color 120ms ease, border-color 120ms ease",
        "@media (hover: hover)": {
          "&:hover": disabled ? {} : { borderColor: alpha(t.palette.primary.main, 0.4) },
        },
        "& .MuiFormControlLabel-label": { flex: 1, minWidth: 0 },
        "& .MuiRadio-root, & .MuiCheckbox-root": {
          p: 0,
          height: CHOICE_LINE,
          "& .MuiSvgIcon-root": { fontSize: "1.25rem" },
        },
      })}
    />
  );
}

/** One option, and its preview outside the label so opening the preview
 *  never picks the option. */
function ChoiceRow(
  { choice, children }: { choice: ElicitationChoice; children: React.ReactNode },
): React.JSX.Element {
  if (!choice.preview) return <>{children}</>;
  return (
    <Box>
      {children}
      <ChoicePreview preview={choice.preview} />
    </Box>
  );
}

function FieldInput({
  field,
  value,
  disabled,
  onChange,
}: {
  field: ElicitationField;
  value: ElicitationValue | undefined;
  disabled: boolean;
  onChange: (value: ElicitationValue) => void;
}): React.JSX.Element {
  switch (field.kind) {
    case "single":
      return (
        <Stack role="radiogroup" aria-label={field.title} spacing={0.75}>
          {field.choices.map((choice) => (
            <ChoiceRow key={choice.value} choice={choice}>
              <ChoiceTile
                title={choice.title}
                description={choice.description}
                selected={value === choice.value}
                disabled={disabled}
                control={
                  <Radio
                    checked={value === choice.value}
                    onChange={() => onChange(choice.value)}
                  />
                }
              />
            </ChoiceRow>
          ))}
        </Stack>
      );
    case "multi": {
      const picked = Array.isArray(value) ? value : [];
      return (
        <Stack role="group" aria-label={field.title} spacing={0.75}>
          {field.choices.map((choice) => (
            <ChoiceRow key={choice.value} choice={choice}>
              <ChoiceTile
                title={choice.title}
                description={choice.description}
                selected={picked.includes(choice.value)}
                disabled={disabled}
                control={
                  <Checkbox
                    checked={picked.includes(choice.value)}
                    onChange={(event) =>
                      onChange(
                        event.target.checked
                          ? [...picked, choice.value]
                          : picked.filter((item) => item !== choice.value),
                      )}
                  />
                }
              />
            </ChoiceRow>
          ))}
        </Stack>
      );
    }
    case "boolean":
      return (
        <FormControlLabel
          disabled={disabled}
          control={
            <Checkbox
              size="small"
              checked={value === true}
              onChange={(event) => onChange(event.target.checked)}
            />
          }
          label={<Typography variant="body2">Yes</Typography>}
        />
      );
    default:
      return (
        <TextField
          fullWidth
          size="small"
          disabled={disabled}
          value={typeof value === "string" ? value : ""}
          onChange={(event) => onChange(event.target.value)}
          type={field.secret
            ? "password"
            : field.kind === "number" || field.kind === "integer"
            ? "number"
            : "text"}
          multiline={field.kind === "text" && !field.secret}
          maxRows={6}
          autoComplete="off"
          slotProps={{ htmlInput: { "aria-label": field.title } }}
        />
      );
  }
}

/** The body of an elicitation: the agent's message and either the form
 *  fields or the link to open. The surface draws the choice buttons. */
export function ElicitationFields({
  elicitation,
  values,
  setValue,
  disabled,
  seenKey,
}: {
  elicitation: Elicitation;
  values: ElicitationValues;
  setValue: (id: string, value: ElicitationValue) => void;
  disabled: boolean;
  /** Identifies this request across remounts (session, seq and request id);
   *  the auto-resolution countdown starts when a key is first drawn. */
  seenKey?: string;
}): React.JSX.Element {
  return (
    <Stack
      spacing={1.25}
      data-elicitation={elicitation.mode}
      sx={{ maxHeight: "min(50vh, 420px)", overflow: "auto", pr: 0.5 }}
    >
      <Typography
        variant="body2"
        sx={{ ...approvalProseSx, whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}
      >
        {elicitation.message}
      </Typography>
      {elicitation.mode === "form" && elicitation.autoResolutionMs !== null &&
        seenKey !== undefined && (
        <AutoResolutionCountdown
          seenKey={seenKey}
          autoResolutionMs={elicitation.autoResolutionMs}
        />
      )}
      {elicitation.mode === "url"
        ? (
          <Box>
            <Button
              size="small"
              variant="outlined"
              endIcon={<OpenInNew fontSize="small" />}
              href={elicitation.url}
              target="_blank"
              rel="noopener noreferrer"
              sx={{ textTransform: "none", maxWidth: "100%" }}
            >
              <Box
                component="span"
                sx={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}
              >
                {new URL(elicitation.url).host}
              </Box>
            </Button>
            <Typography
              variant="caption"
              sx={{ color: "text.secondary", display: "block", mt: 0.5 }}
            >
              Open the link, finish there, then choose Done.
            </Typography>
          </Box>
        )
        : elicitation.fields.map((field) => (
          <Box key={field.id}>
            <FieldHeading field={field} />
            <FieldInput
              field={field}
              value={values[field.id]}
              disabled={disabled}
              onChange={(value) => setValue(field.id, value)}
            />
          </Box>
        ))}
    </Stack>
  );
}
