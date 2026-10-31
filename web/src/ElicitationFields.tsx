import { useCallback, useState } from "react";
import {
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
import {
  type Elicitation,
  ELICITATION_ACCEPT,
  ELICITATION_DECLINE,
  elicitationAnswer,
  elicitationComplete,
  type ElicitationField,
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
    <Box sx={{ mb: 0.25 }}>
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

function ChoiceLabel(
  { title, description }: { title: string; description: string | null },
): React.JSX.Element {
  return (
    <Box sx={{ py: 0.25 }}>
      <Typography variant="body2">{title}</Typography>
      {description && (
        <Typography variant="caption" sx={{ color: "text.secondary", display: "block" }}>
          {description}
        </Typography>
      )}
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
        <Box role="radiogroup" aria-label={field.title}>
          {field.choices.map((choice) => (
            <FormControlLabel
              key={choice.value}
              disabled={disabled}
              sx={{ display: "flex", alignItems: "flex-start", mr: 0 }}
              control={
                <Radio
                  size="small"
                  checked={value === choice.value}
                  onChange={() => onChange(choice.value)}
                  sx={{ py: 0.5 }}
                />
              }
              label={<ChoiceLabel title={choice.title} description={choice.description} />}
            />
          ))}
        </Box>
      );
    case "multi": {
      const picked = Array.isArray(value) ? value : [];
      return (
        <Box role="group" aria-label={field.title}>
          {field.choices.map((choice) => (
            <FormControlLabel
              key={choice.value}
              disabled={disabled}
              sx={{ display: "flex", alignItems: "flex-start", mr: 0 }}
              control={
                <Checkbox
                  size="small"
                  checked={picked.includes(choice.value)}
                  onChange={(event) =>
                    onChange(
                      event.target.checked
                        ? [...picked, choice.value]
                        : picked.filter((item) => item !== choice.value),
                    )}
                  sx={{ py: 0.5 }}
                />
              }
              label={<ChoiceLabel title={choice.title} description={choice.description} />}
            />
          ))}
        </Box>
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
}: {
  elicitation: Elicitation;
  values: ElicitationValues;
  setValue: (id: string, value: ElicitationValue) => void;
  disabled: boolean;
}): React.JSX.Element {
  return (
    <Stack
      spacing={1.25}
      data-elicitation={elicitation.mode}
      sx={{ maxHeight: "min(50vh, 420px)", overflow: "auto", pr: 0.5 }}
    >
      <Typography variant="body2" sx={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>
        {elicitation.message}
      </Typography>
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
