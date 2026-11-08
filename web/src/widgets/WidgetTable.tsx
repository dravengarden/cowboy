// The `table` node (docs/widgets.md §5.2, §7.2): a real table while it fits
// the width it is given, one card per row otherwise. The transcript preview
// draws at most `previewRows` rows.

import {
  Box,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  Typography,
  useTheme,
} from "@mui/material";
import { WIDGET_LIMITS, type Node } from "../../../contracts/widget-v1.ts";
import { cellParts, columnOf, toneColor, type CellParts } from "./widgetFormat";
import { WidgetLinkText } from "./WidgetLinkText";
import { useElementWidth } from "./useElementWidth";

type TableNode = Extract<Node, { type: "table" }>;

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

export function WidgetTable({ node }: { node: TableNode }): React.JSX.Element {
  const [ref, width] = useElementWidth();
  const columns = node.columns.map(columnOf);
  const rows = node.rows.slice(0, WIDGET_LIMITS.previewRows);
  const hidden = node.rows.length - rows.length;
  const cards =
    node.mobile === "cards" ||
    (node.mobile !== "fit" && columns.length > 3 && width > 0 && width < CARD_BREAK);
  return (
    <Box ref={ref} data-widget-table={cards ? "cards" : "table"} sx={{ minWidth: 0 }}>
      {cards ? (
        <Box sx={{ display: "grid", gap: 1 }}>
          {rows.map((row, r) => (
            <Box
              key={r}
              sx={{ border: 1, borderColor: "divider", borderRadius: 1, px: 1.5, py: 1 }}
            >
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
      ) : (
        <Table
          size="small"
          sx={{ tableLayout: "auto", "& td, & th": { px: 1, overflowWrap: "anywhere" } }}
        >
          <TableHead>
            <TableRow>
              {columns.map((column, c) => (
                <TableCell
                  key={c}
                  align={
                    column.align === "end" ? "right" : column.align === "center" ? "center" : "left"
                  }
                  sx={{ fontWeight: 600, color: "text.secondary" }}
                >
                  {column.label}
                </TableCell>
              ))}
            </TableRow>
          </TableHead>
          <TableBody>
            {rows.map((row, r) => (
              <TableRow key={r}>
                {columns.map((column, c) => (
                  <TableCell
                    key={c}
                    align={
                      column.align === "end"
                        ? "right"
                        : column.align === "center"
                          ? "center"
                          : "left"
                    }
                    sx={{ fontVariantNumeric: "tabular-nums" }}
                  >
                    <CellText parts={cellParts(row[c] ?? null, column.format)} />
                  </TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
      {node.caption || hidden > 0 ? (
        <Typography variant="caption" color="text.secondary" sx={{ display: "block", mt: 0.5 }}>
          {[
            node.caption,
            hidden > 0 ? `${hidden} more row${hidden === 1 ? "" : "s"} not shown` : null,
          ]
            .filter(Boolean)
            .join(" · ")}
        </Typography>
      ) : null}
    </Box>
  );
}
