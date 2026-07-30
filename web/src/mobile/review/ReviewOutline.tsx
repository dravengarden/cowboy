import { Clear, Search } from "@mui/icons-material";
import {
  Alert,
  Box,
  CircularProgress,
  IconButton,
  InputAdornment,
  List,
  ListItemButton,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { useEffect, useMemo, useRef, useState } from "react";
import { navigationHaptic } from "../../haptic";
import { Sheet } from "../../Sheet";
import { type CodeDocumentSymbol, fetchCodeOutline } from "./codeApi";
import {
  activeOutlineRow,
  filterOutline,
  flattenOutline,
  symbolKindLabel,
} from "./outlineModel";

const VISIBLE_SYMBOL_CAP = 800;

export function ReviewOutline({
  open,
  onClose,
  sessionId,
  path,
  currentLine,
  onSelect,
}: {
  open: boolean;
  onClose: () => void;
  sessionId: string;
  path: string;
  currentLine?: number | undefined;
  onSelect: (line: number) => void;
}): React.JSX.Element {
  const [symbols, setSymbols] = useState<CodeDocumentSymbol[]>([]);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const activeRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return undefined;
    const controller = new AbortController();
    setLoading(true);
    setError(false);
    setQuery("");
    void fetchCodeOutline(sessionId, path, controller.signal)
      .then((value) => setSymbols(value.symbols))
      .catch(() => {
        if (!controller.signal.aborted) setError(true);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [open, path, sessionId]);

  const rows = useMemo(() => flattenOutline(symbols), [symbols]);
  const active = useMemo(
    () => activeOutlineRow(rows, currentLine),
    [currentLine, rows],
  );
  const filtered = useMemo(
    () => filterOutline(rows, query),
    [query, rows],
  );
  const visible = filtered.slice(0, VISIBLE_SYMBOL_CAP);

  useEffect(() => {
    if (!open || loading || query || !activeRef.current) return;
    activeRef.current.scrollIntoView({ block: "nearest" });
  }, [active, loading, open, query]);

  const fileName = path.split("/").at(-1) ?? path;
  return (
    <Sheet
      open={open}
      onClose={onClose}
      title={`Outline · ${fileName}`}
      forceSheet
      cover
    >
      <Stack spacing={1.25} sx={{ minHeight: "55vh", pb: 2 }}>
        <TextField
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Filter symbols"
          size="small"
          fullWidth
          slotProps={{
            input: {
              startAdornment: (
                <InputAdornment position="start">
                  <Search fontSize="small" />
                </InputAdornment>
              ),
              endAdornment: query
                ? (
                  <InputAdornment position="end">
                    <IconButton
                      size="small"
                      aria-label="Clear outline filter"
                      onClick={() => setQuery("")}
                    >
                      <Clear fontSize="small" />
                    </IconButton>
                  </InputAdornment>
                )
                : undefined,
            },
          }}
          sx={{
            position: "sticky",
            top: 0,
            zIndex: 2,
            bgcolor: "background.paper",
            borderRadius: 1,
          }}
        />
        {loading
          ? (
            <Stack alignItems="center" justifyContent="center" sx={{ py: 8 }}>
              <CircularProgress size={26} />
            </Stack>
          )
          : error
          ? (
            <Alert severity="info">
              Zed could not produce an outline for this file yet.
            </Alert>
          )
          : visible.length === 0
          ? (
            <Typography
              color="text.secondary"
              sx={{ py: 6, textAlign: "center" }}
            >
              {query ? "No matching symbols" : "No symbols in this file"}
            </Typography>
          )
          : (
            <>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ px: 0.5 }}
              >
                {query
                  ? `${filtered.length} matches`
                  : `${rows.length} symbols · tap to jump`}
              </Typography>
              <List disablePadding>
                {visible.map((row, index) => {
                  const selected = !query && row === active;
                  return (
                    <ListItemButton
                      key={`${row.symbol.selectionStart.row}:${row.symbol.selectionStart.column}:${row.symbol.name}:${index}`}
                      ref={selected ? activeRef : undefined}
                      selected={selected}
                      onClick={() => {
                        navigationHaptic();
                        onSelect(row.symbol.selectionStart.row + 1);
                        onClose();
                      }}
                      sx={{
                        minHeight: 48,
                        pl: 1 + Math.min(row.depth, 4) * 2,
                        pr: 1,
                        borderRadius: 1.5,
                        mb: 0.25,
                        scrollMarginTop: 64,
                        "&::before": row.depth
                          ? {
                            content: '""',
                            alignSelf: "stretch",
                            borderLeft: 1,
                            borderColor: "divider",
                            mr: 1.25,
                          }
                          : undefined,
                      }}
                    >
                      <Box sx={{ minWidth: 0, flex: 1 }}>
                        <Typography
                          variant="body2"
                          fontFamily="var(--cowboy-font-mono)"
                          noWrap
                          fontWeight={selected ? 700 : 500}
                        >
                          {row.symbol.name}
                        </Typography>
                        <Typography variant="caption" color="text.secondary">
                          {symbolKindLabel(row.symbol.kind)} · line{" "}
                          {row.symbol.selectionStart.row + 1}
                        </Typography>
                      </Box>
                    </ListItemButton>
                  );
                })}
              </List>
              {filtered.length > VISIBLE_SYMBOL_CAP && (
                <Typography
                  variant="caption"
                  color="text.secondary"
                  sx={{ px: 1 }}
                >
                  Refine the filter to search the remaining symbols.
                </Typography>
              )}
            </>
          )}
      </Stack>
    </Sheet>
  );
}
