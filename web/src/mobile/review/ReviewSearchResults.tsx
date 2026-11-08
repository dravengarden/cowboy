import { DescriptionOutlined } from "@mui/icons-material";
import {
  Alert,
  Box,
  CircularProgress,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Typography,
} from "@mui/material";
import { useEffect, useRef, useState } from "react";
import {
  type CodeContentMatch,
  type CodeContentSearchResults,
  fetchCodeContentSearch,
  fetchCodeSearch,
} from "../../review/codeApi";

export type ReviewSearchMode = "files" | "text";

export interface ReviewSearchState {
  files: string[];
  matches: CodeContentMatch[];
  stopped: CodeContentSearchResults["stopped"];
  searching: boolean;
  failed: boolean;
}

const EMPTY: ReviewSearchState = {
  files: [],
  matches: [],
  stopped: null,
  searching: false,
  failed: false,
};

/** Debounced file-name or content search; a newer query aborts the older one. */
export function useReviewSearch(
  sessionId: string | undefined,
  query: string,
  revision: number,
  mode: ReviewSearchMode,
): ReviewSearchState {
  const [state, setState] = useState<ReviewSearchState>(EMPTY);
  const active = useRef<AbortController | null>(null);
  useEffect(() => {
    active.current?.abort();
    const trimmed = query.trim();
    if (!sessionId || !trimmed) {
      active.current = null;
      setState(EMPTY);
      return undefined;
    }
    const controller = new AbortController();
    active.current = controller;
    setState((current) => ({ ...current, searching: true, failed: false }));
    const timer = globalThis.setTimeout(() => {
      const request = mode === "text"
        ? fetchCodeContentSearch(sessionId, trimmed, controller.signal).then(
          (result) => ({
            ...EMPTY,
            matches: result.matches,
            stopped: result.stopped,
          }),
        )
        : fetchCodeSearch(sessionId, trimmed, controller.signal).then((
          result,
        ) => ({ ...EMPTY, files: result.files }));
      void request
        .then((next) => {
          if (!controller.signal.aborted) setState(next);
        })
        .catch((error) => {
          if (
            !controller.signal.aborted &&
            !(error instanceof DOMException && error.name === "AbortError")
          ) {
            setState({ ...EMPTY, failed: true });
          }
        })
        .finally(() => {
          if (active.current === controller) active.current = null;
        });
    }, mode === "text" ? 300 : 180);
    return () => {
      globalThis.clearTimeout(timer);
      controller.abort();
    };
  }, [mode, query, revision, sessionId]);
  useEffect(() => () => active.current?.abort(), []);
  return state;
}

/** The first result Enter opens. */
export function firstReviewSearchResult(
  state: ReviewSearchState,
  mode: ReviewSearchMode,
): { path: string; line?: number } | undefined {
  if (mode === "text") {
    const match = state.matches[0];
    return match ? { path: match.path, line: match.line } : undefined;
  }
  const path = state.files[0];
  return path ? { path } : undefined;
}

const STOPPED: Record<NonNullable<ReviewSearchState["stopped"]>, string> = {
  limit: "Showing the first 100 matches. Refine the search to see more.",
  files: "Stopped after scanning the file limit; some files were not searched.",
  bytes: "Stopped after scanning the size limit; some files were not searched.",
  deadline: "Stopped at the time limit; some files were not searched.",
};

function splitPath(path: string): [string, string | undefined] {
  const split = path.lastIndexOf("/");
  return split >= 0
    ? [path.slice(split + 1), path.slice(0, split)]
    : [path, undefined];
}

const mono = {
  noWrap: true,
  fontFamily: "var(--cowboy-font-mono)",
  fontSize: "0.875rem",
};

export function ReviewSearchResults(
  { state, mode, onOpen }: {
    state: ReviewSearchState;
    mode: ReviewSearchMode;
    onOpen: (path: string, line?: number) => void;
  },
): React.JSX.Element {
  const count = mode === "text" ? state.matches.length : state.files.length;
  if (state.searching && count === 0) {
    return (
      <Box sx={{ display: "grid", placeItems: "center", pt: 6 }}>
        <CircularProgress size={24} />
      </Box>
    );
  }
  if (state.failed) {
    return (
      <Alert severity="error">
        {mode === "text"
          ? "Text search is unavailable"
          : "File search is unavailable"}
      </Alert>
    );
  }
  if (count === 0) {
    return (
      <Typography color="text.secondary" textAlign="center" sx={{ pt: 6 }}>
        {mode === "text"
          ? state.stopped ? STOPPED[state.stopped] : "No matching text"
          : "No matching files"}
      </Typography>
    );
  }
  if (mode === "files") {
    return (
      <List disablePadding>
        {state.files.map((path) => {
          const [name, directory] = splitPath(path);
          return (
            <ListItemButton
              key={path}
              onClick={() => onOpen(path)}
              sx={{ minHeight: 48, borderRadius: 2 }}
            >
              <ListItemIcon sx={{ minWidth: 34, color: "text.secondary" }}>
                <DescriptionOutlined fontSize="small" />
              </ListItemIcon>
              <ListItemText
                primary={name}
                secondary={directory}
                primaryTypographyProps={mono}
                secondaryTypographyProps={{ noWrap: true }}
              />
            </ListItemButton>
          );
        })}
      </List>
    );
  }
  return (
    <>
      {state.stopped && (
        <Typography
          variant="caption"
          color="text.secondary"
          component="div"
          sx={{ px: 1.5, pb: 0.5 }}
        >
          {STOPPED[state.stopped]}
        </Typography>
      )}
      <List disablePadding data-review-text-results>
        {state.matches.map((match) => (
          <ListItemButton
            key={`${match.path}:${match.line}`}
            onClick={() => onOpen(match.path, match.line)}
            sx={{ minHeight: 48, borderRadius: 2 }}
          >
            <ListItemText
              primary={match.text.trim() || " "}
              secondary={`${match.path}:${match.line}`}
              primaryTypographyProps={mono}
              secondaryTypographyProps={{ noWrap: true }}
            />
          </ListItemButton>
        ))}
      </List>
    </>
  );
}
