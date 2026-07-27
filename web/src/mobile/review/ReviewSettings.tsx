import {
  Box,
  Divider,
  FormControlLabel,
  MenuItem,
  Select,
  Stack,
  Switch,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
} from "@mui/material";
import { SettingsSheet } from "../../_shell";
import {
  REVIEW_CODE_FONT_SIZES,
  REVIEW_CONTEXT_LINES,
} from "./reviewSettingsModel";
import {
  updateReviewSettings,
  useReviewSettings,
} from "./reviewSettings";

function SettingToggle({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}): React.JSX.Element {
  return (
    <FormControlLabel
      labelPlacement="start"
      control={
        <Switch
          checked={checked}
          onChange={(_, next): void => onChange(next)}
        />
      }
      label={
        <Box sx={{ minWidth: 0 }}>
          <Typography variant="body2" fontWeight={650}>{label}</Typography>
          <Typography variant="caption" color="text.secondary">
            {description}
          </Typography>
        </Box>
      }
      sx={{
        width: "100%",
        justifyContent: "space-between",
        alignItems: "center",
        gap: 2,
        m: 0,
      }}
    />
  );
}

export function ReviewSettings(): React.JSX.Element {
  const settings = useReviewSettings();
  return (
    <SettingsSheet title="Code Review settings" cover>
      <Stack spacing={2}>
        <Typography variant="overline" color="text.secondary">
          Code display
        </Typography>
        <Stack spacing={1}>
          <Box>
            <Typography variant="body2" fontWeight={650}>
              Code font size
            </Typography>
            <Typography variant="caption" color="text.secondary">
              Only code and diff text
            </Typography>
          </Box>
          <ToggleButtonGroup
            exclusive
            fullWidth
            size="small"
            value={settings.codeFontSize}
            onChange={(_, value: number | null): void => {
              if (value !== null) updateReviewSettings({ codeFontSize: value });
            }}
          >
            {REVIEW_CODE_FONT_SIZES.map((value) => (
              <ToggleButton key={value} value={value}>
                {value}px
              </ToggleButton>
            ))}
          </ToggleButtonGroup>
        </Stack>
        <SettingToggle
          label="Soft wrap"
          description="Wrap long code lines to the phone width"
          checked={settings.softWrap}
          onChange={(softWrap): void => updateReviewSettings({ softWrap })}
        />
      </Stack>

      <Divider />
      <Stack spacing={2}>
        <Typography variant="overline" color="text.secondary">
          Diff
        </Typography>
        <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={2}>
          <Box>
            <Typography variant="body2" fontWeight={650}>Context</Typography>
            <Typography variant="caption" color="text.secondary">
              Unchanged lines around each hunk
            </Typography>
          </Box>
          <Select
            size="small"
            value={settings.contextLines}
            onChange={(event): void =>
              updateReviewSettings({ contextLines: Number(event.target.value) })}
            sx={{ minWidth: 92 }}
          >
            {REVIEW_CONTEXT_LINES.map((value) => (
              <MenuItem key={value} value={value}>
                {value === -1 ? "All" : value}
              </MenuItem>
            ))}
          </Select>
        </Stack>
        <SettingToggle
          label="Whitespace changes"
          description="Include whitespace-only edits in diffs"
          checked={settings.showWhitespaceChanges}
          onChange={(showWhitespaceChanges): void =>
            updateReviewSettings({ showWhitespaceChanges })}
        />
      </Stack>

      <Divider />
      <Stack spacing={2}>
        <Typography variant="overline" color="text.secondary">
          Language intelligence
        </Typography>
        <SettingToggle
          label="Diagnostics"
          description="Show LSP errors and warnings"
          checked={settings.diagnostics}
          onChange={(diagnostics): void => updateReviewSettings({ diagnostics })}
        />
        <SettingToggle
          label="Inlay hints"
          description="Show inferred types and parameter names"
          checked={settings.inlayHints}
          onChange={(inlayHints): void => updateReviewSettings({ inlayHints })}
        />
        <SettingToggle
          label="Semantic highlighting"
          description="Overlay LSP semantic tokens on syntax colors"
          checked={settings.semanticHighlighting}
          onChange={(semanticHighlighting): void =>
            updateReviewSettings({ semanticHighlighting })}
        />
      </Stack>
    </SettingsSheet>
  );
}
