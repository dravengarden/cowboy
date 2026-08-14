import { useState } from "react";
import { Box, Button, Stack, Typography, useTheme } from "@mui/material";
import { ExpandLess, ExpandMore } from "@mui/icons-material";
import type { ContentChunk, PromptOrigin } from "./derive";
import { Markdown } from "./Markdown";
import { runtimePromptPresentation } from "./promptOrigin";
import {
  agentOriginDisplayName,
  agentOriginSourceLabel,
  cowboyOriginCaption,
  originProviderId,
} from "./promptOriginPresentation";
import { ProviderIcon } from "./ProviderIcon";
import { providerVisual } from "./providerVisual";

function messageText(chunks: ContentChunk[]): string {
  return chunks
    .filter((chunk): chunk is Extract<ContentChunk, { type: "text" }> =>
      chunk.type === "text"
    )
    .map((chunk) => chunk.text)
    .join("");
}

function CowboyOriginNote({
  origin,
  text,
}: {
  origin: PromptOrigin;
  text: string;
}): React.JSX.Element {
  return (
    <Box
      data-prompt-origin-actor="cowboy"
      data-prompt-origin-source={origin.source}
      sx={{
        alignSelf: "flex-end",
        maxWidth: { xs: "92%", sm: "80%" },
        display: "flex",
        flexDirection: "column",
        alignItems: "flex-end",
        gap: 0.5,
        py: 0.5,
      }}
    >
      <Typography variant="caption" color="text.secondary" sx={{ fontWeight: 600 }}>
        {cowboyOriginCaption(origin)}
      </Typography>
      <Box
        sx={{
          width: "100%",
          px: 1.25,
          py: 0.75,
          borderRadius: 1.5,
          border: 1,
          borderColor: "divider",
          bgcolor: "action.hover",
          color: "text.secondary",
          fontSize: 13,
        }}
      >
        <Markdown text={text} />
      </Box>
    </Box>
  );
}

function AgentOriginNote({
  origin,
  text,
  provider,
}: {
  origin: PromptOrigin;
  text: string;
  provider: string;
}): React.JSX.Element {
  const theme = useTheme();
  const [detailsOpen, setDetailsOpen] = useState(false);
  const providerId = originProviderId(origin, provider);
  const visual = providerVisual(providerId, theme.palette.mode);
  const presented = runtimePromptPresentation(text, origin);
  const name = agentOriginDisplayName(providerId);
  const source = agentOriginSourceLabel(origin);
  const hasDetails = Boolean(presented.raw && presented.raw !== presented.title);
  const mark = <ProviderIcon provider={providerId} sx={{ fontSize: 18 }} />;

  return (
    <Box
      data-prompt-origin-actor="agent"
      data-prompt-origin-source={origin.source}
      data-prompt-origin-provider={providerId}
      aria-label={`${name} ${source} update`}
      sx={{
        alignSelf: "stretch",
        py: 0.25,
      }}
    >
      <Box
        sx={{
          px: 1.25,
          py: 0.875,
          borderRadius: 1,
          border: 1,
          borderColor: "divider",
          bgcolor: "action.hover",
          color: "text.primary",
        }}
      >
        <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
          <Box
            aria-hidden
            sx={{
              width: 18,
              height: 18,
              flexShrink: 0,
              color: visual.primary,
              display: "grid",
              placeItems: "center",
              "& .MuiSvgIcon-root": { fontSize: 18 },
            }}
          >
            {mark ?? (
              <Typography component="span" sx={{ fontSize: 11, fontWeight: 700, lineHeight: 1 }}>
                {name.slice(0, 1)}
              </Typography>
            )}
          </Box>
          <Typography
            variant="caption"
            sx={{ fontWeight: 700, color: "text.primary", letterSpacing: "0.01em" }}
          >
            {name}
          </Typography>
          <Typography variant="caption" sx={{ color: "text.disabled" }}>
            {source}
          </Typography>
        </Stack>
        <Stack
          direction="row"
          spacing={1}
          alignItems="center"
          sx={{ mt: 0.5, minWidth: 0 }}
        >
          <Typography
            variant="body2"
            sx={{ flex: 1, minWidth: 0, fontWeight: 600, lineHeight: "inherit" }}
          >
            {presented.title || text}
          </Typography>
          {hasDetails && (
            <Button
              size="small"
              disableRipple
              aria-expanded={detailsOpen}
              onClick={(): void => setDetailsOpen((open) => !open)}
              endIcon={detailsOpen ? <ExpandLess /> : <ExpandMore />}
              sx={{
                flexShrink: 0,
                minWidth: 0,
                px: 0.5,
                color: "text.secondary",
                textTransform: "none",
                fontWeight: 600,
                "& .MuiButton-endIcon": { ml: 0.25 },
                "& .MuiButton-endIcon > svg": { fontSize: "1.15rem" },
                "&:hover": { bgcolor: "transparent", color: "text.primary" },
              }}
            >
              {detailsOpen ? "Hide" : "Details"}
            </Button>
          )}
        </Stack>
        {hasDetails && detailsOpen && (
          <Box
            component="pre"
            sx={{
              m: 0,
              mt: 0.75,
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
              fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
              fontSize: "0.78em",
              lineHeight: 1.45,
              color: "text.secondary",
            }}
          >
            {presented.raw}
          </Box>
        )}
      </Box>
    </Box>
  );
}

export function PromptOriginNote({
  origin,
  chunks,
  provider,
}: {
  origin: PromptOrigin;
  chunks: ContentChunk[];
  provider: string;
}): React.JSX.Element {
  const text = messageText(chunks);
  if (origin.actor === "agent") {
    return <AgentOriginNote origin={origin} text={text} provider={provider} />;
  }
  return <CowboyOriginNote origin={origin} text={text} />;
}
