// The `table` node (docs/widgets.md §5.2, §7.2, §7.5): a real table while it
// fits the width it is given, one card per row otherwise. The transcript
// preview draws at most `previewRows` rows; the inspector pages the rest.
// `sort: true` sorts on the client by header (or a picker in card layout).

import { useContext, useState } from "react";
import {
  Box,
  MenuItem,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TablePagination,
  TableRow,
  TableSortLabel,
  TextField,
  Typography,
  useTheme,
} from "@mui/material";
import { WIDGET_LIMITS, type Cell, type Node } from "../../../contracts/widget-v1.ts";
import { cellParts, columnOf, toneColor, type CellParts } from "./widgetFormat";
import { WidgetLinkText } from "./WidgetLinkText";
import { useElementWidth } from "./useElementWidth";
import { moreLabel, useWidgetMode, WidgetSortContext } from "./widgetMode";
import { nextSort, sortedRowIndexes, type SortState } from "./widgetTableModel";

type TableNode = Extract<Node, { type: "table" }>;
type ColumnView = ReturnType<typeof columnOf>;

/** Below this width a table with more than three columns becomes cards. */
const CARD_BREAK = 560;

function CellText({ parts }: { parts: CellParts }): React.JSX.Element {
  const theme = useTheme();
  const color = toneColor(theme, parts.tone);
  return (
    <Box
      component="span"
      sx={{ ...(color ? { color, fontWeight: 600 } : {}), overflowWrap: "anywhere" }}
    >
      {parts.url ? <WidgetLinkText url={parts.url}>{parts.text}</WidgetLinkText> : parts.text}
    </Box>
  );
}

function align(column: ColumnView): "left" | "center" | "right" {
  return column.align === "end" ? "right" : column.align === "center" ? "center" : "left";
}

/** `min` keeps a column as narrow as its widest unbroken value; `max` gives it room. */
function widthSx(column: ColumnView): Record<string, string> {
  if (column.width === "min") return { width: "1%", whiteSpace: "nowrap" };
  if (column.width === "max") return { minWidth: "12em" };
  return {};
}

function CardRows({
  columns,
  rows,
}: {
  columns: ColumnView[];
  rows: readonly (readonly Cell[])[];
}): React.JSX.Element {
  return (
    <Box sx={{ display: "grid", gap: 1 }}>
      {rows.map((row, r) => (
        <Box key={r} sx={{ border: 1, borderColor: "divider", borderRadius: 1, px: 1.5, py: 1 }}>
          <Typography variant="body2" sx={{ fontWeight: 600, mb: 0.5 }}>
            <CellText parts={cellParts(row[0] ?? null, columns[0]?.format)} />
          </Typography>
          <Box
            component="dl"
            sx={{
              m: 0,
              display: "grid",
              gridTemplateColumns: "minmax(6em, max-content) 1fr",
              columnGap: 1.5,
              rowGap: 0.25,
            }}
          >
            {columns.slice(1).map((column, c) => (
              <Box key={c} sx={{ display: "contents" }}>
                <Typography component="dt" variant="caption" color="text.secondary">
                  {column.label}
                </Typography>
                <Typography component="dd" variant="body2" sx={{ m: 0 }}>
                  <CellText parts={cellParts(row[c + 1] ?? null, column.format)} />
                </Typography>
              </Box>
            ))}
          </Box>
        </Box>
      ))}
    </Box>
  );
}

/** Card layout has no headers; a native picker keeps sort reachable on touch. */
function CardSortPicker({
  columns,
  sort,
  onSort,
}: {
  columns: ColumnView[];
  sort: SortState;
  onSort: (sort: SortState) => void;
}): React.JSX.Element {
  const value = sort ? `${String(sort.column)}:${sort.direction}` : "";
  return (
    <TextField
      select
      size="small"
      label="Sort by"
      value={value}
      onChange={(event): void => {
        const [column, direction] = event.target.value.split(":");
        onSort(
          column === undefined || column === ""
            ? null
            : { column: Number(column), direction: direction === "desc" ? "desc" : "asc" },
        );
      }}
      sx={{ mb: 1, minWidth: 180 }}
      slotProps={{ select: { displayEmpty: true }, inputLabel: { shrink: true } }}
    >
      <MenuItem value="">Original order</MenuItem>
      {columns.flatMap((column, c) => [
        <MenuItem key={`${String(c)}:asc`} value={`${String(c)}:asc`}>
          {`${column.label} ↑`}
        </MenuItem>,
        <MenuItem key={`${String(c)}:desc`} value={`${String(c)}:desc`}>
          {`${column.label} ↓`}
        </MenuItem>,
      ])}
    </TextField>
  );
}

function GridRows({
  columns,
  rows,
  sortable,
  sort,
  onSort,
  sticky,
}: {
  columns: ColumnView[];
  rows: readonly (readonly Cell[])[];
  sortable: boolean;
  sort: SortState;
  onSort: (sort: SortState) => void;
  sticky: boolean;
}): React.JSX.Element {
  return (
    <Table
      size="small"
      stickyHeader={sticky}
      sx={{ tableLayout: "auto", "& td, & th": { px: 1, overflowWrap: "anywhere" } }}
    >
      <TableHead>
        <TableRow>
          {columns.map((column, c) => {
            const active = sort?.column === c;
            return (
              <TableCell
                key={c}
                align={align(column)}
                aria-sort={
                  active ? (sort.direction === "asc" ? "ascending" : "descending") : undefined
                }
                sx={{ fontWeight: 600, color: "text.secondary", ...widthSx(column) }}
              >
                {sortable ? (
                  <TableSortLabel
                    active={active}
                    direction={active ? sort.direction : "asc"}
                    onClick={(): void => onSort(nextSort(sort, c))}
                  >
                    {column.label}
                  </TableSortLabel>
                ) : (
                  column.label
                )}
              </TableCell>
            );
          })}
        </TableRow>
      </TableHead>
      <TableBody>
        {rows.map((row, r) => (
          <TableRow key={r}>
            {columns.map((column, c) => (
              <TableCell
                key={c}
                align={align(column)}
                sx={{
                  fontVariantNumeric: "tabular-nums",
                  ...(column.width === "min" ? { whiteSpace: "nowrap" } : {}),
                }}
              >
                <CellText parts={cellParts(row[c] ?? null, column.format)} />
              </TableCell>
            ))}
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

export function WidgetTable({ node }: { node: TableNode }): React.JSX.Element {
  const [ref, width] = useElementWidth();
  const mode = useWidgetMode();
  const shared = useContext(WidgetSortContext);
  const [localSort, setLocalSort] = useState<SortState>(null);
  const sort = shared ? shared.get(node) : localSort;
  const setSort = shared ? (next: SortState): void => shared.set(node, next) : setLocalSort;
  const [page, setPage] = useState(0);
  const columns = node.columns.map(columnOf);
  const sortable = node.sort === true;
  const order = sortedRowIndexes(
    node.rows,
    sortable ? sort : null,
    sort ? columns[sort.column]?.format : undefined,
  );
  const perPage = WIDGET_LIMITS.inspectorPageRows;
  const paged = mode === "inspector" && order.length > perPage;
  const start = paged ? page * perPage : 0;
  const end = mode === "inspector" ? (paged ? start + perPage : order.length) : WIDGET_LIMITS.previewRows;
  const rows = order.slice(start, end).map((index) => node.rows[index]!);
  const hidden = mode === "preview" ? order.length - rows.length : 0;
  const cards =
    node.mobile === "cards" ||
    (node.mobile !== "fit" && columns.length > 3 && width > 0 && width < CARD_BREAK);
  const changeSort = (next: SortState): void => {
    setSort(next);
    setPage(0);
  };
  const caption = [node.caption, hidden > 0 ? moreLabel(hidden, "row") : null].filter(Boolean);
  return (
    <Box ref={ref} data-widget-table={cards ? "cards" : "table"} sx={{ minWidth: 0 }}>
      {cards ? (
        <>
          {sortable ? <CardSortPicker columns={columns} sort={sort} onSort={changeSort} /> : null}
          <CardRows columns={columns} rows={rows} />
        </>
      ) : (
        <GridRows
          columns={columns}
          rows={rows}
          sortable={sortable}
          sort={sort}
          onSort={changeSort}
          sticky={mode === "inspector"}
        />
      )}
      {paged ? (
        <TablePagination
          component="div"
          count={order.length}
          page={page}
          onPageChange={(_, next): void => setPage(next)}
          rowsPerPage={perPage}
          rowsPerPageOptions={[]}
        />
      ) : null}
      {caption.length > 0 ? (
        <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.5 }}>
          {caption.join(" · ")}
        </Typography>
      ) : null}
    </Box>
  );
}
