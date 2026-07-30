import {
  AccountTreeOutlined,
  AdjustOutlined,
  CodeOutlined,
  Clear,
  DataArrayOutlined,
  DataObjectOutlined,
  DiamondOutlined,
  Search,
} from "@mui/icons-material";
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
import { alpha, type Theme } from "@mui/material/styles";
import { useEffect, useMemo, useRef, useState } from "react";
import { navigationHaptic } from "../../haptic";
import { Sheet } from "../../Sheet";
import { type CodeDocumentSymbol, fetchCodeOutline } from "./codeApi";
import {
  activeOutlineRow,
  filterOutline,
  flattenOutline,
  type OutlineSymbolCategory,
  outlineSymbolCategory,
  symbolKindLabel,
} from "./outlineModel";

const VISIBLE_SYMBOL_CAP = 800;

function categoryColor(
  theme: Theme,
  category: OutlineSymbolCategory,
): string {
  if (category === "module") return theme.palette.info.main;
  if (category === "type") return theme.palette.secondary.main;
  if (category === "function" || category === "method") {
    return theme.palette.success.main;
  }
  if (category === "field") return theme.palette.warning.main;
  if (category === "constant") return theme.palette.info.dark;
  if (category === "object") return theme.palette.text.secondary;
  return theme.palette.text.secondary;
}

function SymbolCategoryIcon({
  category,
}: {
  category: OutlineSymbolCategory;
}): React.JSX.Element {
  if (category === "module") return <AccountTreeOutlined />;
  if (category === "type") return <DataObjectOutlined />;
  if (category === "function") {
    return (
      <Box
        component="span"
        sx={{
          fontFamily: "var(--cowboy-font-mono)",
          fontSize: 17,
          fontWeight: 750,
          lineHeight: 1,
        }}
      >
        λ
      </Box>
    );
  }
  if (category === "method") return <CodeOutlined />;
  if (category === "field") return <DataArrayOutlined />;
  if (category === "constant") return <DiamondOutlined />;
  if (category === "object") return <AccountTreeOutlined />;
  return <AdjustOutlined />;
}

function HighlightedSymbolName({
  name,
  query,
}: {
  name: string;
  query: string;
}): React.JSX.Element {
  const terms = query.trim().split(/\s+/u).filter(Boolean);
  if (terms.length === 0) return <>{name}</>;
  const escaped = terms.map((term) =>
    term.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")
  );
  const pattern = new RegExp(`(${escaped.join("|")})`, "giu");
  const normalized = new Set(terms.map((term) => term.toLocaleLowerCase()));
  return (
    <>
      {name.split(pattern).map((part, index) =>
        normalized.has(part.toLocaleLowerCase())
          ? (
            <Box
              // The same fragment may occur more than once in one symbol.
              key={`${part}:${index}`}
              component="mark"
              sx={{
                color: "inherit",
                bgcolor: (theme) => alpha(theme.palette.warning.main, 0.24),
                borderRadius: 0.5,
                px: 0.125,
              }}
            >
              {part}
            </Box>
          )
          : part
      )}
    </>
  );
}

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
                  const category = outlineSymbolCategory(row.symbol.kind);
                  return (
                    <ListItemButton
                      key={`${row.symbol.selectionStart.row}:${row.symbol.selectionStart.column}:${row.symbol.name}:${index}`}
                      ref={selected ? activeRef : undefined}
                      selected={selected}
                      aria-current={selected ? "location" : undefined}
                      onClick={() => {
                        navigationHaptic();
                        onSelect(row.symbol.selectionStart.row + 1);
                        onClose();
                      }}
                      sx={{
                        position: "relative",
                        minHeight: row.depth === 0 ? 48 : 44,
                        py: 0.625,
                        pl: 0.75 + Math.min(row.depth, 4) * 1.75,
                        pr: 1,
                        borderRadius: 1.5,
                        mb: 0.25,
                        mt: row.depth === 0 && index > 0 ? 0.5 : 0,
                        scrollMarginTop: 64,
                        bgcolor: selected
                          ? (theme) => alpha(theme.palette.primary.main, 0.1)
                          : "transparent",
                        "&.Mui-selected": {
                          bgcolor: (theme) =>
                            alpha(theme.palette.primary.main, 0.1),
                        },
                        "&.Mui-selected:hover": {
                          bgcolor: (theme) =>
                            alpha(theme.palette.primary.main, 0.14),
                        },
                        "&::before": row.depth
                          ? {
                            content: '""',
                            alignSelf: "stretch",
                            borderLeft: 1,
                            borderColor: (theme) =>
                              alpha(theme.palette.text.secondary, 0.18),
                            mr: 0.75,
                          }
                          : undefined,
                        "&::after": selected
                          ? {
                            content: '""',
                            position: "absolute",
                            left: 0,
                            top: 7,
                            bottom: 7,
                            width: 3,
                            borderRadius: "0 3px 3px 0",
                            bgcolor: "primary.main",
                          }
                          : undefined,
                      }}
                    >
                      <Box
                        aria-hidden
                        sx={{
                          width: 26,
                          height: 26,
                          mr: 1,
                          flexShrink: 0,
                          display: "grid",
                          placeItems: "center",
                          borderRadius: 1,
                          color: (theme) => categoryColor(theme, category),
                          bgcolor: (theme) =>
                            alpha(categoryColor(theme, category), 0.12),
                          "& svg": { fontSize: 16 },
                        }}
                      >
                        <SymbolCategoryIcon category={category} />
                      </Box>
                      <Box sx={{ minWidth: 0, flex: 1 }}>
                        <Typography
                          variant="body2"
                          fontFamily="var(--cowboy-font-mono)"
                          noWrap
                          fontWeight={selected ? 700 : 500}
                        >
                          <HighlightedSymbolName
                            name={row.symbol.name}
                            query={query}
                          />
                        </Typography>
                        <Stack
                          direction="row"
                          alignItems="baseline"
                          spacing={0.75}
                        >
                          <Typography
                            variant="caption"
                            fontWeight={700}
                            sx={{
                              color: (theme) =>
                                categoryColor(theme, category),
                            }}
                          >
                            {symbolKindLabel(row.symbol.kind)}
                          </Typography>
                          <Typography
                            variant="caption"
                            color="text.secondary"
                          >
                            line {row.symbol.selectionStart.row + 1}
                          </Typography>
                        </Stack>
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
