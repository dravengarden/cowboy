import { useEffect, useRef } from "react";
import { Box, Button, Chip, Paper, Stack, Typography } from "@mui/material";
import {
  CheckCircle,
  Construction,
  ErrorOutline,
  Psychology,
  RadioButtonUnchecked,
} from "@mui/icons-material";
import { derive, type RenderItem } from "./derive";
import type { Envelope } from "./protocol";
import { send } from "./store";

function toolColor(status: string): "default" | "success" | "error" | "warning" {
  if (status === "completed") return "success";
  if (status === "failed") return "error";
  if (status === "in_progress") return "warning";
  return "default";
}

function ToolRow({ item }: { item: Extract<RenderItem, { kind: "tool" }> }): React.JSX.Element {
  return (
    <Chip
      icon={<Construction fontSize="small" />}
      label={`${item.title} · ${item.status}`}
      size="small"
      color={toolColor(item.status)}
      variant="outlined"
      sx={{ alignSelf: "flex-start", maxWidth: "100%" }}
    />
  );
}

function MessageBubble({
  role,
  text,
}: {
  role: "assistant" | "user";
  text: string;
}): React.JSX.Element {
  const mine = role === "user";
  return (
    <Paper
      variant="outlined"
      sx={{
        p: 1.25,
        alignSelf: mine ? "flex-end" : "flex-start",
        maxWidth: "92%",
        bgcolor: mine ? "primary.main" : "background.paper",
        color: mine ? "primary.contrastText" : "text.primary",
        whiteSpace: "pre-wrap",
        wordBreak: "break-word",
      }}
    >
      <Typography variant="body2" component="div">
        {text}
      </Typography>
    </Paper>
  );
}

function PermissionCard({
  sessionId,
  item,
}: {
  sessionId: string;
  item: Extract<RenderItem, { kind: "permission" }>;
}): React.JSX.Element {
  return (
    <Paper variant="outlined" sx={{ p: 1.5, borderColor: "warning.main", alignSelf: "stretch" }}>
      <Typography variant="subtitle2" gutterBottom>
        {item.title}
      </Typography>
      {item.resolved ? (
        <Typography variant="caption" color="text.secondary">
          Resolved{item.chosen ? `: ${item.chosen}` : ""}
        </Typography>
      ) : (
        <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
          {item.options.map((opt) => (
            <Button
              key={opt.optionId}
              size="small"
              variant={opt.kind.startsWith("allow") ? "contained" : "outlined"}
              color={opt.kind.startsWith("reject") ? "error" : "primary"}
              onClick={(): void =>
                send({
                  type: "permission",
                  session_id: sessionId,
                  request_id: item.requestId,
                  option_id: opt.optionId,
                })
              }
            >
              {opt.name}
            </Button>
          ))}
        </Stack>
      )}
    </Paper>
  );
}

export function Transcript({
  sessionId,
  timeline,
}: {
  sessionId: string;
  timeline: Envelope[];
}): React.JSX.Element {
  const items = derive(timeline);
  const endRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const stick = useRef(true);

  useEffect(() => {
    if (stick.current) endRef.current?.scrollIntoView({ block: "end" });
  });

  function onScroll(): void {
    const el = containerRef.current;
    if (!el) return;
    stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  }

  return (
    <Box
      ref={containerRef}
      onScroll={onScroll}
      sx={{ flex: 1, overflowY: "auto", p: { xs: 1, sm: 2 } }}
    >
      <Stack spacing={1.25}>
        {items.map((item, i) => {
          switch (item.kind) {
            case "message":
              return <MessageBubble key={i} role={item.role} text={item.text} />;
            case "thought":
              return (
                <Stack
                  key={i}
                  direction="row"
                  spacing={1}
                  sx={{ color: "text.secondary", alignSelf: "flex-start", maxWidth: "92%" }}
                >
                  <Psychology fontSize="small" />
                  <Typography variant="body2" sx={{ whiteSpace: "pre-wrap", fontStyle: "italic" }}>
                    {item.text}
                  </Typography>
                </Stack>
              );
            case "tool":
              return <ToolRow key={i} item={item} />;
            case "plan":
              return (
                <Paper key={i} variant="outlined" sx={{ p: 1.25, alignSelf: "stretch" }}>
                  <Typography variant="overline">Plan</Typography>
                  <Stack spacing={0.5} sx={{ mt: 0.5 }}>
                    {item.entries.map((e, j) => (
                      <Stack key={j} direction="row" spacing={1} alignItems="center">
                        {e.status === "completed" ? (
                          <CheckCircle fontSize="small" color="success" />
                        ) : (
                          <RadioButtonUnchecked fontSize="small" color="disabled" />
                        )}
                        <Typography variant="body2">{e.content}</Typography>
                      </Stack>
                    ))}
                  </Stack>
                </Paper>
              );
            case "permission":
              return <PermissionCard key={i} sessionId={sessionId} item={item} />;
            case "lifecycle":
              return (
                <Stack
                  key={i}
                  direction="row"
                  spacing={1}
                  alignItems="center"
                  sx={{ color: item.status === "crashed" ? "error.main" : "text.secondary" }}
                >
                  <ErrorOutline fontSize="small" />
                  <Typography variant="caption">
                    {item.status}
                    {item.detail ? `: ${item.detail}` : ""}
                  </Typography>
                </Stack>
              );
          }
        })}
      </Stack>
      <div ref={endRef} />
    </Box>
  );
}
