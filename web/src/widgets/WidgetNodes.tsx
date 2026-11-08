// Renders validated widget nodes with product MUI components
// (docs/widgets.md §5.2, §7). Content is data: nothing here evaluates it.
// Inline-carrier constraints (§6.4): images show their `alt`, inputs render
// inert with their labels.

import {
  Alert,
  Box,
  Button,
  CircularProgress,
  Divider,
  LinearProgress,
  Paper,
  Typography,
  useTheme,
} from "@mui/material";
import CheckRounded from "@mui/icons-material/CheckRounded";
import CloseRounded from "@mui/icons-material/CloseRounded";
import ImageOutlined from "@mui/icons-material/ImageOutlined";
import InsertDriveFileOutlined from "@mui/icons-material/InsertDriveFileOutlined";
import RadioButtonUncheckedRounded from "@mui/icons-material/RadioButtonUncheckedRounded";
import RemoveRounded from "@mui/icons-material/RemoveRounded";
import type { ListItem, Step, Tone } from "../../../contracts/widget-v1.ts";
import { Markdown } from "../Markdown";
import { StatusChip } from "../StatusChip";
import { cellParts, toneColor } from "./widgetFormat";
import type { ParsedNode } from "./widgetDocument";
import { WidgetLinkText } from "./WidgetLinkText";
import { WidgetTable } from "./WidgetTable";

const GAP = { xs: 0.5, sm: 1, md: 1.5, lg: 2.5 } as const;

function Children({ nodes }: { nodes: ParsedNode[] }): React.JSX.Element {
  return (
    <>
      {nodes.map((child, index) => (
        <WidgetNode key={child.id ?? index} node={child} />
      ))}
    </>
  );
}

function ListItems({ items, ordered }: { items: ListItem[]; ordered: boolean }): React.JSX.Element {
  const theme = useTheme();
  return (
    <Box component={ordered ? "ol" : "ul"} sx={{ m: 0, pl: 3, "& li + li": { mt: 0.25 } }}>
      {items.map((item, index) => (
        <li key={index}>
          <Box component="span" sx={{ color: toneColor(theme, item.tone) }}>
            {item.to?.kind === "url" ? (
              <WidgetLinkText url={item.to.url}>{item.text}</WidgetLinkText>
            ) : (
              item.text
            )}
          </Box>
          {item.children?.length ? <ListItems items={item.children} ordered={ordered} /> : null}
        </li>
      ))}
    </Box>
  );
}

function StepIcon({ state }: { state: Step["state"] }): React.JSX.Element {
  const sx = { fontSize: "1.1em", flexShrink: 0, mt: "0.15em" } as const;
  switch (state) {
    case "done":
      return <CheckRounded color="success" sx={sx} />;
    case "failed":
      return <CloseRounded color="error" sx={sx} />;
    case "active":
      return <CircularProgress size="0.95em" thickness={6} sx={{ ...sx, mx: "0.075em" }} />;
    case "skipped":
      return <RemoveRounded color="disabled" sx={sx} />;
    default:
      return <RadioButtonUncheckedRounded color="disabled" sx={sx} />;
  }
}

function ImagePlaceholder({
  alt,
  caption,
}: {
  alt: string;
  caption?: string | undefined;
}): React.JSX.Element {
  return (
    <Box
      role="img"
      aria-label={alt}
      data-widget-image-placeholder=""
      sx={{
        display: "flex",
        gap: 1,
        alignItems: "center",
        border: 1,
        borderColor: "divider",
        borderRadius: 1,
        px: 1.5,
        py: 1,
        color: "text.secondary",
      }}
    >
      <ImageOutlined fontSize="small" />
      <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>
        {alt}
        {caption ? ` — ${caption}` : ""}
      </Typography>
    </Box>
  );
}

type Of<T extends ParsedNode["type"]> = Extract<ParsedNode, { type: T }>;

function Card({ node }: { node: Of<"card"> }): React.JSX.Element {
  return (
    <Paper variant="outlined" sx={{ p: 1.5, display: "grid", gap: 1, minWidth: 0 }}>
      {node.title || node.status ? (
        <Box sx={{ display: "flex", gap: 1, alignItems: "center", flexWrap: "wrap" }}>
          {node.title ? (
            <Typography variant="subtitle2" sx={{ fontWeight: 600, flex: "1 1 auto", minWidth: 0 }}>
              {node.title}
            </Typography>
          ) : null}
          {node.status ? <WidgetStatus label={node.status.label} tone={node.status.tone} /> : null}
        </Box>
      ) : null}
      {node.subtitle ? (
        <Typography variant="body2" color="text.secondary">
          {node.subtitle}
        </Typography>
      ) : null}
      <Children nodes={node.children} />
      {node.actions?.length ? (
        <Box sx={{ display: "flex", gap: 1, flexWrap: "wrap" }}>
          {node.actions.map((action) => (
            <WidgetNode key={action.id} node={action} />
          ))}
        </Box>
      ) : null}
    </Paper>
  );
}

function WidgetStatus({
  label,
  tone,
}: {
  label: string;
  tone?: Tone | undefined;
}): React.JSX.Element {
  const theme = useTheme();
  const color = toneColor(theme, tone);
  return <StatusChip label={label} sx={color ? { color, borderColor: color } : {}} />;
}

function StackNode({ node }: { node: Of<"stack"> }): React.JSX.Element {
  return (
    <Box
      data-widget-stack={node.direction ?? "column"}
      sx={{
        display: "flex",
        flexDirection: node.direction === "row" ? "row" : "column",
        flexWrap: node.direction === "row" ? "wrap" : "nowrap",
        gap: GAP[node.gap ?? "sm"],
        alignItems:
          node.align === "center"
            ? "center"
            : node.align === "end"
              ? "flex-end"
              : node.align === "stretch"
                ? "stretch"
                : node.direction === "row"
                  ? "flex-start"
                  : "stretch",
        minWidth: 0,
        // A row keeps every child at least 120 CSS px and wraps instead
        // of overflowing (§7.2).
        ...(node.direction === "row"
          ? { "& > *": { flex: "1 1 120px", minWidth: "min(120px, 100%)" } }
          : {}),
      }}
    >
      <Children nodes={node.children} />
    </Box>
  );
}

function SectionNode({ node }: { node: Of<"section"> }): React.JSX.Element {
  return (
    <Box
      component="details"
      open={!node.collapsed}
      sx={{ "& > summary": { cursor: "pointer", fontWeight: 600, minHeight: 32, py: 0.5 } }}
    >
      <Typography component="summary" variant="subtitle2">
        {node.title}
      </Typography>
      <Box sx={{ display: "grid", gap: 1, pt: 0.5 }}>
        <Children nodes={node.children} />
      </Box>
    </Box>
  );
}

function TextNode({ node }: { node: Of<"text"> }): React.JSX.Element {
  const theme = useTheme();
  const color = toneColor(theme, node.tone);
  const variant =
    node.variant === "title"
      ? "subtitle1"
      : node.variant === "subtitle"
        ? "subtitle2"
        : node.variant === "caption"
          ? "caption"
          : "body2";
  return (
    <Typography
      component="div"
      variant={variant}
      sx={{
        ...(color ? { color } : {}),
        ...(node.variant === "title" ? { fontWeight: 600 } : {}),
        "& p": { m: 0 },
      }}
    >
      <Markdown text={node.text} />
    </Typography>
  );
}

function KvNode({ node }: { node: Of<"kv"> }): React.JSX.Element {
  const theme = useTheme();
  return (
    <Box
      component="dl"
      data-widget-kv=""
      sx={{
        m: 0,
        display: "grid",
        gridTemplateColumns: "repeat(auto-fit, minmax(min(260px, 100%), 1fr))",
        gap: 1,
      }}
    >
      {node.items.map((item, index) => {
        const parts = cellParts(
          { v: item.v, ...(item.tone ? { tone: item.tone } : {}) },
          undefined,
        );
        const color = toneColor(theme, parts.tone);
        return (
          <Box key={index} sx={{ minWidth: 0 }}>
            <Typography component="dt" variant="caption" color="text.secondary">
              {item.k}
            </Typography>
            <Typography
              component="dd"
              variant="body2"
              sx={{
                m: 0,
                fontWeight: 600,
                overflowWrap: "anywhere",
                ...(color ? { color } : {}),
              }}
            >
              {item.to?.kind === "url" ? (
                <WidgetLinkText url={item.to.url}>{parts.text}</WidgetLinkText>
              ) : (
                parts.text
              )}
            </Typography>
          </Box>
        );
      })}
    </Box>
  );
}

function FileNode({ node }: { node: Of<"file"> }): React.JSX.Element {
  return (
    <Box
      sx={{
        display: "inline-flex",
        gap: 0.5,
        alignItems: "center",
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
        fontSize: "0.85em",
        border: 1,
        borderColor: "divider",
        borderRadius: 1,
        px: 0.75,
        py: 0.25,
        maxWidth: "100%",
        overflowWrap: "anywhere",
      }}
    >
      <InsertDriveFileOutlined sx={{ fontSize: "1.1em" }} />
      {node.path}
      {node.line ? `:${node.line}` : ""}
      {node.summary ? (
        <Box
          component="span"
          sx={{ color: "text.secondary", fontFamily: "inherit" }}
        >{` — ${node.summary}`}</Box>
      ) : null}
    </Box>
  );
}

function ProgressNode({ node }: { node: Of<"progress"> }): React.JSX.Element {
  const theme = useTheme();
  return (
    <Box sx={{ display: "grid", gap: 0.5 }}>
      {node.label || node.value !== undefined ? (
        <Box sx={{ display: "flex", justifyContent: "space-between", gap: 1 }}>
          <Typography variant="body2">{node.label}</Typography>
          {node.value !== undefined ? (
            <Typography
              variant="body2"
              color="text.secondary"
              sx={{ fontVariantNumeric: "tabular-nums" }}
            >{`${Math.round(node.value * 100)}%`}</Typography>
          ) : null}
        </Box>
      ) : null}
      {node.variant === "circular" ? (
        <CircularProgress
          variant={node.value === undefined ? "indeterminate" : "determinate"}
          {...(node.value !== undefined ? { value: node.value * 100 } : {})}
          size={28}
        />
      ) : (
        <LinearProgress
          variant={node.value === undefined ? "indeterminate" : "determinate"}
          {...(node.value !== undefined ? { value: node.value * 100 } : {})}
          sx={{
            height: 6,
            borderRadius: 3,
            "& .MuiLinearProgress-bar": {
              transition: "none",
              ...(toneColor(theme, node.tone)
                ? { backgroundColor: toneColor(theme, node.tone) }
                : {}),
            },
          }}
        />
      )}
      {node.detail ? (
        <Typography variant="caption" color="text.secondary">
          {node.detail}
        </Typography>
      ) : null}
    </Box>
  );
}

function StepsNode({ node }: { node: Of<"steps"> }): React.JSX.Element {
  return (
    <Box component="ol" sx={{ m: 0, p: 0, listStyle: "none", display: "grid", gap: 0.5 }}>
      {node.steps.map((step, index) => (
        <Box
          component="li"
          key={index}
          data-step-state={step.state}
          sx={{ display: "flex", gap: 1, alignItems: "flex-start" }}
        >
          <StepIcon state={step.state} />
          <Box sx={{ minWidth: 0 }}>
            <Typography
              variant="body2"
              sx={{
                ...(step.state === "skipped" ? { color: "text.disabled" } : {}),
                ...(step.state === "active" ? { fontWeight: 600 } : {}),
              }}
            >
              {step.label}
            </Typography>
            {step.detail ? (
              <Typography variant="caption" color="text.secondary">
                {step.detail}
              </Typography>
            ) : null}
          </Box>
        </Box>
      ))}
    </Box>
  );
}

function AlertNode({ node }: { node: Of<"alert"> }): React.JSX.Element {
  return (
    <Alert severity={node.severity} variant="outlined" sx={{ py: 0.25, "& p": { m: 0 } }}>
      {node.title ? (
        <Typography variant="subtitle2" sx={{ fontWeight: 600 }}>
          {node.title}
        </Typography>
      ) : null}
      <Markdown text={node.text} />
    </Alert>
  );
}

function GalleryNode({ node }: { node: Of<"gallery"> }): React.JSX.Element {
  return (
    <Box
      sx={{
        display: "grid",
        gridTemplateColumns: "repeat(auto-fill, minmax(min(200px, 100%), 1fr))",
        gap: 1,
      }}
    >
      {node.images.map((image, index) => (
        <ImagePlaceholder key={index} alt={image.alt} caption={image.caption} />
      ))}
    </Box>
  );
}

function ButtonNode({ node }: { node: Of<"button"> }): React.JSX.Element {
  return (
    <Box>
      <Button
        size="small"
        disabled
        variant={
          node.variant === "primary" ? "contained" : node.variant === "text" ? "text" : "outlined"
        }
        color={node.variant === "destructive" ? "error" : "primary"}
        title="Read-only widget: reply in the composer"
        sx={{ textTransform: "none" }}
      >
        {node.label}
      </Button>
    </Box>
  );
}

function ChoiceNode({ node }: { node: Of<"choice"> }): React.JSX.Element {
  return (
    <Box sx={{ display: "grid", gap: 0.5 }}>
      <Typography variant="body2" sx={{ fontWeight: 600 }}>
        {node.label}
      </Typography>
      <Box component="ul" sx={{ m: 0, pl: 3 }}>
        {node.options.map((option) => (
          <li key={option.value}>
            <Typography variant="body2" component="span">
              {option.label}
            </Typography>
            {option.detail ? (
              <Typography
                variant="caption"
                color="text.secondary"
              >{` — ${option.detail}`}</Typography>
            ) : null}
          </li>
        ))}
      </Box>
    </Box>
  );
}

type Leaf = Of<
  "progress" | "steps" | "alert" | "image" | "gallery" | "button" | "choice" | "unsupported"
>;

/** Status, media and input nodes. */
function WidgetLeaf({ node }: { node: Leaf }): React.JSX.Element {
  switch (node.type) {
    case "progress":
      return <ProgressNode node={node} />;
    case "steps":
      return <StepsNode node={node} />;
    case "alert":
      return <AlertNode node={node} />;
    case "image":
      return <ImagePlaceholder alt={node.alt} caption={node.caption} />;
    case "gallery":
      return <GalleryNode node={node} />;
    case "button":
      return <ButtonNode node={node} />;
    case "choice":
      return <ChoiceNode node={node} />;
    case "unsupported":
      return node.fallback ? (
        <Markdown text={node.fallback} />
      ) : (
        <Typography
          variant="caption"
          color="text.secondary"
          data-widget-unsupported={node.original}
        >
          {`Unsupported widget component: ${node.original}`}
        </Typography>
      );
  }
}

export function WidgetNode({ node }: { node: ParsedNode }): React.JSX.Element | null {
  switch (node.type) {
    case "stack":
      return <StackNode node={node} />;
    case "card":
      return <Card node={node} />;
    case "section":
      return <SectionNode node={node} />;
    case "divider":
      return <Divider />;
    case "text":
      return <TextNode node={node} />;
    case "kv":
      return <KvNode node={node} />;
    case "list":
      return (
        <Typography component="div" variant="body2">
          <ListItems items={node.items} ordered={node.ordered ?? false} />
        </Typography>
      );
    case "table":
      return <WidgetTable node={node} />;
    case "link":
      return (
        <Typography variant="body2">
          {node.to.kind === "url" ? (
            <WidgetLinkText url={node.to.url}>{node.label}</WidgetLinkText>
          ) : (
            node.label
          )}
        </Typography>
      );
    case "file":
      return <FileNode node={node} />;
    case "status":
      return (
        <Box>
          <WidgetStatus label={node.label} tone={node.tone} />
        </Box>
      );
    default:
      return <WidgetLeaf node={node} />;
  }
}
