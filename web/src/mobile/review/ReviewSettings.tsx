import {
  Box,
  Divider,
  FormControlLabel,
  MenuItem,
  Select,
  Stack,
  Switch,
  Typography,
} from "@mui/material";
import {
  REVIEW_CODE_FONT_SIZES,
  REVIEW_CONTEXT_LINES,
} from "./reviewSettingsModel";
import {
  updateReviewSettings,
  useReviewLanguageCapabilities,
  useReviewSettings,
} from "./reviewSettings";

function SettingToggle({
  label,
  description,
  checked,
  onChange,
  disabled = false,
}: {
  label: string;
  description: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}): React.JSX.Element {
  return (
    <FormControlLabel
      labelPlacement="start"
      control={
        <Switch
          checked={checked}
          disabled={disabled}
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

export function ReviewSettingsContent({
  language,
}: {
  language?: import("./codeApi").CodeLanguageCapabilities | undefined;
} = {}): React.JSX.Element {
  const settings = useReviewSettings();
  const publishedLanguage = useReviewLanguageCapabilities();
  const capabilities = language ?? publishedLanguage;
  return (
    <Stack spacing={3}>
      <Stack spacing={2}>
        <Typography variant="overline" color="text.secondary">
          Code display
        </Typography>
        <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={2}>
          <Box>
            <Typography variant="body2" fontWeight={650}>
              Code font size
            </Typography>
            <Typography variant="caption" color="text.secondary">
              Only code and diff text
            </Typography>
          </Box>
          <Select
            size="small"
            value={settings.codeFontSize}
            onChange={(event): void =>
              updateReviewSettings({ codeFontSize: Number(event.target.value) })}
            sx={{ minWidth: 92 }}
          >
            {REVIEW_CODE_FONT_SIZES.map((value) => (
              <MenuItem key={value} value={value}>
                {value}px
              </MenuItem>
            ))}
          </Select>
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
          disabled={!capabilities?.diagnostics}
        />
        <SettingToggle
          label="Inlay hints"
          description="Show inferred types and parameter names"
          checked={settings.inlayHints}
          onChange={(inlayHints): void => updateReviewSettings({ inlayHints })}
          disabled={!capabilities?.inlayHints}
        />
        <SettingToggle
          label="Semantic highlighting"
          description="Overlay LSP semantic tokens on syntax colors"
          checked={settings.semanticHighlighting}
          onChange={(semanticHighlighting): void =>
            updateReviewSettings({ semanticHighlighting })}
          disabled={!capabilities?.semanticTokens}
        />
        {capabilities?.state !== "ready" && (
          <Typography variant="caption" color="text.secondary">
            Language intelligence is {capabilities?.state ?? "unavailable"} for this
            worktree. Syntax highlighting remains available locally.
          </Typography>
        )}
      </Stack>
    </Stack>
  );
}
