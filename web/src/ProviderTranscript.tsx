import {
  Box,
  CircularProgress,
  keyframes,
  Stack,
  Typography,
  useTheme,
} from "@mui/material";
import { alpha } from "@mui/material/styles";
import { LightbulbOutlined, TerminalRounded } from "@mui/icons-material";
import type {
  ProviderUiManifest,
  TranscriptPresentationContract,
} from "../../packages/provider-ui-sdk/src/index.ts";
import { Markdown } from "./Markdown";
import {
  providerPresentationEntry,
  useProviderCatalog,
} from "./providerCatalog";
import {
  ProviderMark,
  readableProviderMarkColor,
} from "./ProviderSurface";

type ThoughtPresentation = TranscriptPresentationContract["thought"];

const DEFAULT_THOUGHT_PRESENTATION: ThoughtPresentation = {
  variant: "timeline",
  density: "comfortable",
  current_surface: "plain",
};

const thoughtPulse = keyframes`
  0%, 100% { opacity: 1; }
  50% { opacity: 0.52; }
`;
const thoughtShimmer = keyframes`
  from { background-position: 100% 0; }
  to { background-position: 0% 0; }
`;
const terminalCaret = keyframes`
  0%, 50% { opacity: 1; }
  51%, 100% { opacity: 0; }
`;
const SIGNAL_THOUGHT_MARK_SIZE = 14;

function WorkcellGlyph({ size = 16 }: { size?: number }): React.JSX.Element {
  return (
    <LightbulbOutlined
      aria-hidden
      sx={{
        fontSize: size,
        display: "block",
        flexShrink: 0,
      }}
    />
  );
}

function TerminalGlyph({ current }: { current: boolean }): React.JSX.Element {
  return (
    <Box
      component="span"
      aria-hidden
      sx={{
        display: "inline-flex",
        alignItems: "center",
        justifyContent: "center",
        width: 15,
        height: 15,
        lineHeight: 1,
        flexShrink: 0,
      }}
    >
      {current
        ? <CircularProgress size={13} thickness={4.5} color="inherit" />
        : <TerminalRounded sx={{ fontSize: 16, display: "block" }} />}
    </Box>
  );
}

function VariantGlyph({
  presentation,
  manifest,
  current,
  header = false,
}: {
  presentation: ThoughtPresentation;
  manifest?: ProviderUiManifest | undefined;
  current: boolean;
  header?: boolean;
}): React.JSX.Element {
  switch (presentation.variant) {
    case "timeline":
      return (
        <Box
          component="span"
          sx={{
            width: header ? 7 : 5,
            height: header ? 7 : 5,
            display: "block",
            borderRadius: "50%",
            bgcolor: current ? "currentColor" : "text.disabled",
            animation: current
              ? `${thoughtPulse} 1.4s ease-in-out infinite`
              : "none",
            "@media (prefers-reduced-motion: reduce)": {
              animation: "none",
            },
          }}
        />
      );
    case "workcell":
      return header ? <WorkcellGlyph size={14} /> : (
        <LightbulbOutlined
          sx={{
            fontSize: 14,
            animation: current
              ? `${thoughtPulse} 1.5s ease-in-out infinite`
              : "none",
            "@media (prefers-reduced-motion: reduce)": {
              animation: "none",
            },
          }}
        />
      );
    case "signal":
      return manifest
        ? (
          <Box
            component="span"
            sx={{
              width: SIGNAL_THOUGHT_MARK_SIZE,
              height: SIGNAL_THOUGHT_MARK_SIZE,
              display: "inline-flex",
              alignItems: "center",
              justifyContent: "center",
              opacity: current ? 1 : 0.78,
              animation: current
                ? `${thoughtPulse} 1.8s ease-in-out infinite`
                : "none",
              "@media (prefers-reduced-motion: reduce)": {
                animation: "none",
              },
            }}
          >
            <ProviderMark
              manifest={manifest}
              size={SIGNAL_THOUGHT_MARK_SIZE}
            />
          </Box>
        )
        : (
          <Box
            component="span"
            sx={{
              width: 7,
              height: 7,
              display: "block",
              borderRadius: "50%",
              bgcolor: "currentColor",
            }}
          />
        );
    case "terminal":
      return <TerminalGlyph current={current} />;
  }
}

function StreamingThoughtCaret(): React.JSX.Element {
  return (
    <Box
      component="span"
      aria-hidden
      sx={{
        display: "inline-block",
        width: "0.55em",
        height: "1em",
        ml: 0.25,
        verticalAlign: "text-bottom",
        bgcolor: "currentColor",
        animation: `${terminalCaret} 1s steps(2, jump-none) infinite`,
        "@media (prefers-reduced-motion: reduce)": {
          animation: "none",
        },
      }}
    />
  );
}

function markerGeometry(variant: ThoughtPresentation["variant"]): {
  size: number;
  gap: number;
  paddingLeft: number;
} {
  switch (variant) {
    case "timeline":
      return { size: 5, gap: 9, paddingLeft: 2 };
    case "workcell":
      return { size: 15, gap: 4, paddingLeft: 3 };
    case "signal":
      return {
        size: SIGNAL_THOUGHT_MARK_SIZE,
        gap: 6,
        paddingLeft: 0,
      };
    case "terminal":
      return { size: 15, gap: 5, paddingLeft: 2 };
  }
}

export function ProviderThoughtSteps({
  sections,
  streaming,
  provider,
  providerVersion,
  providerDigest,
}: {
  sections: string[];
  streaming: boolean;
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
}): React.JSX.Element {
  const theme = useTheme();
  const { catalog } = useProviderCatalog();
  const entry = providerPresentationEntry(
    catalog?.providers ?? [],
    provider,
    providerVersion,
    providerDigest,
  );
  const manifest = entry?.manifest;
  const presentation = manifest?.host.schema_version === 2
    ? manifest.host.transcript.thought
    : DEFAULT_THOUGHT_PRESENTATION;
  const visible = sections.filter((section) => section.trim() !== "");
  const geometry = markerGeometry(presentation.variant);
  const accent = manifest
    ? readableProviderMarkColor(manifest.display.accent, theme)
    : theme.palette.primary.main;
  const secondary = manifest
    ? readableProviderMarkColor(manifest.display.secondary_accent, theme)
    : accent;
  const muted = theme.palette.text.secondary;
  const compact = presentation.density === "compact";
  const currentSurface = presentation.current_surface === "soft";
  const signalHeader = presentation.variant === "signal";
  const dualAccent = presentation.variant === "workcell" ||
    presentation.variant === "signal";

  return (
    <Stack
      spacing={0}
      data-provider-thought-variant={presentation.variant}
      data-provider-thought-density={presentation.density}
      sx={{ flex: 1, minWidth: 0 }}
      aria-label="Thinking steps"
    >
      {
        /* A live thought already has a current-step marker. Showing the
          Provider activity header above it duplicates both icon and status;
          reserve the header for the brief state before the first step lands. */
      }
      {streaming && visible.length === 0 && presentation.active_label && (
        <Box
          data-provider-thought-header={presentation.variant}
          sx={{
            display: "grid",
            gridTemplateColumns: signalHeader
              ? `${geometry.size}px minmax(0, 1fr)`
              : "auto minmax(0, 1fr)",
            alignItems: "center",
            columnGap: signalHeader ? `${geometry.gap}px` : 0.75,
            minHeight: compact ? 16 : 18,
            mb: compact ? 0.125 : 0.25,
            pl: signalHeader ? `${geometry.paddingLeft}px` : 0,
            pr: 0,
            py: 0,
            borderRadius: 0,
            bgcolor: "transparent",
            color: accent,
          }}
          aria-label={presentation.active_label}
        >
          <VariantGlyph
            presentation={presentation}
            manifest={manifest}
            current
            header
          />
          <Typography
            aria-hidden
            variant="caption"
            sx={{
              fontWeight: 500,
              letterSpacing: presentation.variant === "terminal"
                ? "0.035em"
                : "0.01em",
              backgroundImage: dualAccent
                ? `linear-gradient(100deg, ${muted} 0%, ${muted} 34%, ${accent} 46%, ${secondary} 54%, ${muted} 66%, ${muted} 100%)`
                : `linear-gradient(100deg, ${muted} 0%, ${muted} 36%, ${accent} 50%, ${muted} 64%, ${muted} 100%)`,
              backgroundSize: "220% 100%",
              backgroundRepeat: "no-repeat",
              WebkitBackgroundClip: "text",
              backgroundClip: "text",
              color: "transparent",
              animation: `${thoughtShimmer} 3.1s linear infinite`,
              "@media (prefers-reduced-motion: reduce)": {
                animation: "none",
                backgroundImage: "none",
                color: "text.secondary",
              },
            }}
          >
            {presentation.active_label}
          </Typography>
        </Box>
      )}
      {visible.map((section, index) => {
        const current = streaming && index === visible.length - 1;
        const hasNext = index < visible.length - 1;
        return (
          <Box
            key={index}
            data-thought-step-current={current ? "true" : undefined}
            sx={{
              position: "relative",
              display: "grid",
              gridTemplateColumns: `${geometry.size}px minmax(0, 1fr)`,
              columnGap: `${geometry.gap}px`,
              pl: `${geometry.paddingLeft}px`,
              pr: current && currentSurface ? 1 : 0,
              py: current && currentSurface ? (compact ? 0.375 : 0.5) : 0,
              mb: hasNext
                ? compact
                  ? presentation.variant === "workcell" ? 0.125 : 0.25
                  : presentation.variant === "timeline"
                  ? 0.75
                  : presentation.variant === "workcell"
                  ? 0.25
                  : 0.5
                : 0,
              borderRadius: current && currentSurface ? 1.25 : 0,
              bgcolor: current && currentSurface
                ? presentation.variant === "workcell" ? "action.hover" : alpha(
                  accent,
                  theme.palette.mode === "dark" ? 0.13 : 0.07,
                )
                : "transparent",
            }}
          >
            <Box
              aria-hidden="true"
              data-thought-step-indicator-lane
              sx={{
                position: "relative",
                alignSelf: "stretch",
                minHeight: "1lh",
              }}
            >
              <Box
                data-thought-step-indicator
                sx={{
                  position: "absolute",
                  left: "50%",
                  top: `calc(0.5lh - ${geometry.size / 2}px)`,
                  transform: "translateX(-50%)",
                  width: geometry.size,
                  height: geometry.size,
                  display: "grid",
                  placeItems: "center",
                  color: current
                    ? accent
                    : presentation.variant === "signal"
                    ? alpha(accent, 0.72)
                    : "text.disabled",
                }}
              >
                <VariantGlyph
                  presentation={presentation}
                  manifest={manifest}
                  current={current}
                />
              </Box>
              {hasNext && (
                <Box
                  aria-hidden="true"
                  data-thought-step-connector
                  sx={{
                    position: "absolute",
                    left: "50%",
                    top: `calc(0.5lh + ${geometry.size / 2}px)`,
                    bottom: -2,
                    width: "1px",
                    transform: "translateX(-50%)",
                    bgcolor: "divider",
                  }}
                />
              )}
            </Box>
            <Box
              data-thought-step-content
              sx={{
                minWidth: 0,
                opacity: current || !streaming
                  ? 1
                  : presentation.variant === "workcell"
                  ? 0.55
                  : presentation.variant === "signal"
                  ? 0.82
                  : 0.68,
                fontStyle: "normal",
                color: current ? "text.primary" : "text.secondary",
                ...(current && {
                  backgroundImage: dualAccent
                    ? `linear-gradient(100deg, ${muted} 0%, ${muted} 34%, ${accent} 46%, ${secondary} 54%, ${muted} 66%, ${muted} 100%)`
                    : `linear-gradient(100deg, ${muted} 0%, ${muted} 34%, ${accent} 50%, ${muted} 66%, ${muted} 100%)`,
                  backgroundSize: "240% 100%",
                  backgroundRepeat: "no-repeat",
                  WebkitBackgroundClip: "text",
                  backgroundClip: "text",
                  color: "transparent",
                  animation: `${thoughtShimmer} 3.1s linear infinite`,
                  "& p, & p *": { color: "inherit" },
                  "@media (prefers-reduced-motion: reduce)": {
                    animation: "none",
                    backgroundImage: "none",
                    color: "text.primary",
                    WebkitTextFillColor: "currentColor",
                  },
                }),
                "& p": {
                  m: 0,
                  fontStyle: "normal",
                  fontWeight: current ? 600 : 500,
                  lineHeight: 1.45,
                },
              }}
            >
              <Markdown text={section} />
              {current &&
                (presentation.variant === "timeline" ||
                  presentation.variant === "terminal") &&
                <StreamingThoughtCaret />}
            </Box>
          </Box>
        );
      })}
    </Stack>
  );
}
