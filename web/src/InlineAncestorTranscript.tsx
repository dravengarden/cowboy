import { Box, Button, CircularProgress, Stack, Typography } from "@mui/material";
import HistoryRounded from "@mui/icons-material/HistoryRounded";
import { useMemo, useState } from "react";
import { ContinuityDivider } from "./ContinuityDivider";
import { derive } from "./derive";
import { InlineHandoffStatusStrip } from "./InlineHandoffStatusStrip";
import { providerPresentation } from "./providerPresentation";
import { ReadOnlyAncestorItemView } from "./ReadOnlyAncestorItemView";
import { SessionContinuedFrom } from "./SessionContextCue";
import { useStoreSelector } from "./store";
import { ToolDetailsBrowser } from "./ToolDetailsBrowser";
import { toolRuns } from "./tools/runs";
import { shouldPaintTranscriptLifecycle } from "./transcriptLoadingPresentation";
import { type LoadedAncestorSegment, useSessionLineage } from "./useSessionLineage";
import { glyphSize } from "./surface/desktopSize";

function AncestorSegmentBlock({
  segment,
  childProvider,
  desktop,
  selectedToolKey,
  onSelectTool,
}: {
  segment: LoadedAncestorSegment;
  childProvider?: string | undefined;
  desktop: boolean;
  selectedToolKey: string | null;
  onSelectTool: (segmentId: string, itemKey: string) => void;
}): React.JSX.Element {
  const items = useMemo(
    () =>
      derive(segment.events).filter(
        (item) =>
          item.kind !== "forked" &&
          !(item.kind === "lifecycle" && !shouldPaintTranscriptLifecycle(item.status)),
      ),
    [segment.events],
  );
  const prefix = `anc:${segment.hop.depth}:${segment.hop.session_id}:`;

  return (
    <Box data-ancestor-segment={segment.hop.session_id} sx={{ display: "flex", flexDirection: "column" }}>
      {segment.error && (
        <Typography variant="caption" color="error" sx={{ textAlign: "center", py: 0.75 }}>
          {segment.error}
        </Typography>
      )}
      {items.map((item) => (
        <Box
          key={`${prefix}${item.key}`}
          data-ancestor-item={`${prefix}${item.key}`}
          sx={{ py: 0.625, display: "flex", flexDirection: "column", opacity: 0.92 }}
        >
          <ReadOnlyAncestorItemView
            item={item}
            originSessionId={segment.hop.session_id}
            provider={segment.hop.provider}
            desktop={desktop}
            selectedToolKey={
              selectedToolKey === `${prefix}${item.key}` ? item.key : null
            }
            onOpenTool={(key): void => onSelectTool(segment.hop.session_id, key)}
          />
        </Box>
      ))}
      <ContinuityDivider hop={segment.hop} childProvider={childProvider} />
    </Box>
  );
}

/** Renders inherited read-only ancestor conversation segments inline above a
 *  forked or replaced session's own messages, with a local read-only tool
 *  details drawer and on-demand pagination. Ordinary sessions render nothing. */
export function InlineAncestorTranscript({
  sessionId,
  desktop,
}: {
  sessionId: string;
  desktop: boolean;
}): React.JSX.Element | null {
  const session = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === sessionId),
  );
  const hasLineage = Boolean(session?.forked_from || session?.continued_from);
  const { lineage, segments, loadingLineage, hasEarlier, loadingEarlier, loadEarlier } =
    useSessionLineage(sessionId, hasLineage);
  const [selectedTool, setSelectedTool] = useState<{ segmentId: string; itemKey: string } | null>(
    null,
  );

  const selectedSegment = selectedTool
    ? segments.find((seg) => seg.hop.session_id === selectedTool.segmentId)
    : undefined;
  const selectedItems = useMemo(
    () => (selectedSegment ? derive(selectedSegment.events) : []),
    [selectedSegment],
  );
  const selectedRuns = useMemo(() => toolRuns(selectedItems), [selectedItems]);
  const selectedToolKey = selectedTool && selectedSegment
    ? `anc:${selectedSegment.hop.depth}:${selectedSegment.hop.session_id}:${selectedTool.itemKey}`
    : null;

  if (!hasLineage) return null;

  // Fallback to compact provenance card while lineage hasn't returned hops
  // (e.g. legacy session without durable lineage edges or mock harness).
  if (!loadingLineage && (!lineage || lineage.hops.length === 0)) {
    return <SessionContinuedFrom sessionId={sessionId} />;
  }

  return (
    <Box
      data-inline-ancestor-transcript={sessionId}
      sx={{ display: "flex", flexDirection: "column", width: "100%", minWidth: 0 }}
    >
      {hasEarlier && (
        <Box sx={{ display: "flex", justifyContent: "center", pt: 1, pb: 0.75 }}>
          <Button
            size="small"
            variant="outlined"
            disabled={loadingEarlier}
            startIcon={
              loadingEarlier
                ? <CircularProgress size={13} color="inherit" />
                : <HistoryRounded sx={{ fontSize: glyphSize(15) }} />
            }
            onClick={loadEarlier}
            sx={{ borderRadius: 999, textTransform: "none", px: 1.5, minHeight: 32 }}
          >
            {loadingEarlier ? "Loading earlier messages…" : "Load earlier messages"}
          </Button>
        </Box>
      )}
      {loadingLineage && segments.length === 0 && (
        <Stack direction="row" spacing={1} alignItems="center" justifyContent="center" sx={{ py: 1.5, color: "text.secondary" }}>
          <CircularProgress size={14} color="inherit" />
          <Typography variant="caption">Loading previous conversation…</Typography>
        </Stack>
      )}
      {segments.map((segment, index) => (
        <AncestorSegmentBlock
          key={`${segment.hop.depth}:${segment.hop.session_id}:${segment.hop.edge_version}`}
          segment={segment}
          childProvider={
            index === segments.length - 1
              ? session?.provider
              : segments[index + 1]?.hop.provider
          }
          desktop={desktop}
          selectedToolKey={selectedToolKey}
          onSelectTool={(segmentId, itemKey): void => setSelectedTool({ segmentId, itemKey })}
        />
      ))}
      {selectedTool && selectedSegment && (
        <ToolDetailsBrowser
          items={selectedItems}
          runs={selectedRuns}
          selectedKey={selectedTool.itemKey}
          provider={selectedSegment.hop.provider}
          desktop={desktop}
          historyComplete={selectedSegment.reachedStart}
          onSelect={(key): void => setSelectedTool({ segmentId: selectedSegment.hop.session_id, itemKey: key })}
          onClose={(): void => setSelectedTool(null)}
          onLocate={(key): void => {
            setSelectedTool(null);
            const target = document.querySelector(`[data-ancestor-item="anc:${selectedSegment.hop.depth}:${selectedSegment.hop.session_id}:${key}"]`);
            target?.scrollIntoView({ behavior: "smooth", block: "center" });
          }}
        />
      )}
    </Box>
  );
}

/** Zero-blank empty/preparing presentation for forked & replaced sessions:
 *  renders the inherited conversation inline ABOVE a sleek in-flow status strip. */
export function LineageEmptyContinuation({
  sessionId,
  kind,
  location,
  provider,
  providerVersion,
  providerDigest,
  sleeping = false,
  crashDetail = null,
  desktop,
}: {
  sessionId: string;
  kind: "preparing" | "blocked" | "ready";
  location: string;
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
  sleeping?: boolean | undefined;
  crashDetail?: string | null | undefined;
  desktop: boolean;
}): React.JSX.Element {
  const agent = providerPresentation(provider, providerVersion, providerDigest).agent;
  return (
    <Box
      data-lineage-empty-continuation={kind}
      data-conversation-empty-state={kind}
      sx={{
        display: "flex",
        flexDirection: "column",
        width: "100%",
        minWidth: 0,
        pb: 1.5,
      }}
    >
      <InlineAncestorTranscript sessionId={sessionId} desktop={desktop} />
      <InlineHandoffStatusStrip
        sessionId={sessionId}
        kind={kind}
        location={location}
        agent={agent}
        sleeping={sleeping}
        crashDetail={crashDetail}
      />
    </Box>
  );
}
