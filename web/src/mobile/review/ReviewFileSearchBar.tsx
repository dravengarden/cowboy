import { Close, Search } from "@mui/icons-material";
import {
  Box,
  CircularProgress,
  IconButton,
  InputAdornment,
  InputBase,
  type InputBaseProps,
  ToggleButton,
} from "@mui/material";
import type { ReviewSearchMode } from "./ReviewSearchResults";
import { useSurfaceProfile } from "../../surface/SurfaceProfile";

/** The composed search field owns one focus boundary. */
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
  onKeyDown: InputBaseProps["onKeyDown"];
  onMode: () => void;
  onDocuments?: (() => void) | undefined;
}): React.JSX.Element {
  const mobile = useSurfaceProfile().kind !== "desktop";
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
          borderRadius: mobile ? "999px" : "8px",
          "&:has(input:focus-visible)": { outline: "1px solid", outlineColor: "primary.main" },
        }}
      >
        <InputBase
          value={query}
          onChange={(event) => onQuery(event.target.value)}
          onKeyDown={onKeyDown}
          placeholder={label}
          disabled={disabled}
          inputProps={{
            "aria-label": label,
            enterKeyHint: "go",
            autoCapitalize: "off",
            autoCorrect: "off",
            spellCheck: false,
          }}
          startAdornment={
            <InputAdornment position="start" sx={{ mr: 0.75, color: "text.secondary" }}>
              {searching ? (
                <CircularProgress size="1.25rem" aria-label="Searching" />
              ) : (
                <Search sx={{ fontSize: "1.25rem" }} />
              )}
            </InputAdornment>
          }
          endAdornment={
            query ? (
              <InputAdornment position="end" sx={{ ml: 0 }}>
                <IconButton
                  aria-label="Clear file search"
                  onClick={() => onQuery("")}
                  sx={{
                    minWidth: 48,
                    minHeight: 48,
                    p: 0,
                    "& .MuiSvgIcon-root": { fontSize: "1.25rem" },
                  }}
                >
                  <Close />
                </IconButton>
              </InputAdornment>
            ) : undefined
          }
          sx={{
            flex: "1 1 0",
            minWidth: 0,
            minHeight: 48,
            pl: 1,
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
          sx={{ display: "flex", flexShrink: 0, gap: 0.25, px: 0.25, borderRadius: "inherit" }}
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
    <ToggleButton
      data-review-text-search={kind === "text" ? true : undefined}
      data-review-changed-documents={kind === "documents" ? true : undefined}
      aria-label={label}
      value={kind}
      selected={selected}
      onChange={onClick}
      color="primary"
      sx={{
        minWidth: 48,
        minHeight: 48,
        px: 0.75,
        py: 0.5,
        border: 0,
        borderRadius: "inherit",
        fontSize: "0.8125rem",
        textTransform: "none",
        fontWeight: selected ? 600 : 400,
      }}
    >
      {children}
    </ToggleButton>
  );
}
