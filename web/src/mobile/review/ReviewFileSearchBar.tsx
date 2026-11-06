import { Close, Search } from "@mui/icons-material";
import {
  Box,
  ButtonBase,
  CircularProgress,
  IconButton,
  InputAdornment,
  TextField,
  type TextFieldProps,
} from "@mui/material";
import type { ReviewSearchMode } from "./ReviewSearchResults";

/** One quiet search surface; filter paint is smaller than its touch target. */
export function ReviewFileSearchBar({
  query,
  mode,
  searching,
  disabled,
  keyboardReserve,
  showDocuments,
  onQuery,
  onKeyDown,
  onMode,
  onDocuments,
}: {
  query: string;
  mode: ReviewSearchMode;
  searching: boolean;
  disabled: boolean;
  keyboardReserve: string;
  showDocuments: boolean;
  onQuery: (query: string) => void;
  onKeyDown: TextFieldProps["onKeyDown"];
  onMode: () => void;
  onDocuments?: (() => void) | undefined;
}): React.JSX.Element {
  const label = mode === "text" ? "Find text" : "Find a file";
  return (
    <Box
      data-review-find-bar="true"
      sx={{
        flex: "0 0 auto",
        px: 0.75,
        pt: 0.5,
        // Retain the navigator's existing keyboard and safe-area owner.
        pb: `max(4px, calc(var(--kb-inset, 0px) + 4px - ${keyboardReserve}))`,
      }}
    >
      <Box
        data-review-search-surface
        sx={{
          display: "flex",
          alignItems: "center",
          minWidth: 0,
          bgcolor: "action.hover",
          borderRadius: "8px",
          "&:has(input:focus-visible)": { outline: "1px solid", outlineColor: "primary.main" },
        }}
      >
        <TextField
          value={query}
          onChange={(event) => onQuery(event.target.value)}
          onKeyDown={onKeyDown}
          placeholder={label}
          size="small"
          disabled={disabled}
          slotProps={{
            htmlInput: {
              "aria-label": label,
              enterKeyHint: "go",
              autoCapitalize: "off",
              autoCorrect: "off",
              spellCheck: false,
            },
            input: {
              startAdornment: (
                <InputAdornment position="start" sx={{ mr: 0.75, color: "text.secondary" }}>
                  {searching ? (
                    <CircularProgress size="1.25rem" aria-label="Searching" />
                  ) : (
                    <Search sx={{ fontSize: "1.25rem" }} />
                  )}
                </InputAdornment>
              ),
              endAdornment: query ? (
                <InputAdornment position="end" sx={{ ml: 0 }}>
                  <IconButton
                    aria-label="Clear file search"
                    onClick={() => onQuery("")}
                    sx={{
                      width: 44,
                      height: 44,
                      p: 0,
                      "& .MuiSvgIcon-root": { fontSize: "1.25rem" },
                    }}
                  >
                    <Close />
                  </IconButton>
                </InputAdornment>
              ) : undefined,
            },
          }}
          sx={{
            flex: "1 1 0",
            minWidth: 0,
            "& .MuiOutlinedInput-root": { minHeight: 44, pl: 1, pr: 0, borderRadius: "8px" },
            "& .MuiOutlinedInput-notchedOutline": { border: 0 },
            "& input": {
              minWidth: 0,
              py: 1,
              fontSize: "0.875rem",
              "&::placeholder": { color: "text.secondary", opacity: 0.65 },
            },
          }}
        />
        <Box
          role="group"
          aria-label="File search options"
          sx={{ display: "flex", flexShrink: 0, gap: 0.25, px: 0.25 }}
        >
          <SearchOption
            kind="text"
            label="Search file contents"
            selected={mode === "text"}
            onClick={onMode}
          >
            Text
          </SearchOption>
          {onDocuments && (
            <SearchOption
              kind="documents"
              label={showDocuments ? "Browse all files" : "Changed documents"}
              selected={showDocuments}
              onClick={onDocuments}
            >
              Changed
            </SearchOption>
          )}
        </Box>
      </Box>
    </Box>
  );
}

function SearchOption({
  kind,
  label,
  selected,
  onClick,
  children,
}: {
  kind: "text" | "documents";
  label: string;
  selected: boolean;
  onClick: () => void;
  children: string;
}): React.JSX.Element {
  return (
    <ButtonBase
      data-review-text-search={kind === "text" ? true : undefined}
      data-review-changed-documents={kind === "documents" ? true : undefined}
      aria-label={label}
      aria-pressed={selected}
      onClick={onClick}
      disableRipple
      sx={{
        minWidth: 44,
        minHeight: 44,
        borderRadius: "8px",
        fontSize: "0.8125rem",
        color: selected ? "primary.main" : "text.secondary",
        "&:focus-visible [data-review-search-option]": {
          outline: "2px solid",
          outlineColor: "primary.main",
        },
      }}
    >
      <Box
        component="span"
        data-review-search-option
        sx={{
          minHeight: "1.75rem",
          display: "flex",
          alignItems: "center",
          px: 0.75,
          py: 0.5,
          borderRadius: "6px",
          bgcolor: selected ? "action.selected" : "transparent",
          fontWeight: selected ? 600 : 400,
        }}
      >
        {children}
      </Box>
    </ButtonBase>
  );
}
