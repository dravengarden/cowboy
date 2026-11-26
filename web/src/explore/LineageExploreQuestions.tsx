import {
  Box,
  Button,
  ButtonBase,
  Collapse,
  Dialog,
  DialogContent,
  DialogTitle,
  IconButton,
  Stack,
  Typography,
} from "@mui/material";
import CallSplitOutlined from "@mui/icons-material/CallSplitOutlined";
import CloseRounded from "@mui/icons-material/CloseRounded";
import ExpandMore from "@mui/icons-material/ExpandMore";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { LineageHop } from "../ancestorHistory";
import { derive } from "../derive";
import type { Envelope } from "../protocol";
import { ReadOnlyAncestorItemView } from "../ReadOnlyAncestorItemView";
import { useStoreSelector } from "../store";
import { glyphSize } from "../surface/desktopSize";
import { useSessionLineage } from "../useSessionLineage";

interface QuestionSummary {
  id: number;
  title: string;
  ordinal: number;
}

interface HopPagesState {
  pages: QuestionSummary[];
  nextBeforeSeq: number | null;
  loading: boolean;
}

interface QuestionPagesResponse {
  pages: QuestionSummary[];
  next_before_seq?: number | null;
}

function LineageExploreHopSection({
  hop,
  state,
  expanded,
  onToggle,
  onLoadEarlier,
  onOpenPage,
}: {
  hop: LineageHop;
  state: HopPagesState;
  expanded: boolean;
  onToggle: () => void;
  onLoadEarlier: () => void;
  onOpenPage: (page: QuestionSummary) => void;
}): React.JSX.Element {
  return (
    <Box data-lineage-explore-hop={hop.session_id}>
      <ButtonBase
        onClick={onToggle}
        sx={{
          width: "100%",
          minHeight: 36,
          px: 1,
          borderRadius: 1.5,
          justifyContent: "space-between",
          color: "text.secondary",
          "&:hover": { bgcolor: "action.hover" },
        }}
      >
        <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
          <CallSplitOutlined sx={{ fontSize: glyphSize(15), color: "primary.main", flexShrink: 0 }} />
          <Typography variant="caption" noWrap sx={{ fontWeight: 600 }}>
            Earlier from {hop.title || hop.session_id} ({state.pages.length}
            {state.nextBeforeSeq !== null ? "+" : ""})
          </Typography>
        </Stack>
        <ExpandMore sx={{ fontSize: glyphSize(16), transform: expanded ? "rotate(180deg)" : undefined }} />
      </ButtonBase>
      <Collapse in={expanded}>
        <Stack spacing={0.25} sx={{ pt: 0.5 }}>
          {state.nextBeforeSeq !== null && (
            <Button
              size="small"
              variant="text"
              disabled={state.loading}
              onClick={onLoadEarlier}
              sx={{ alignSelf: "flex-start", textTransform: "none", fontSize: "0.72rem", px: 1 }}
            >
              {state.loading ? "Loading earlier questions…" : "Load earlier questions"}
            </Button>
          )}
          {state.pages.map((page) => (
            <ButtonBase
              key={`${hop.session_id}:${page.id}`}
              onClick={(): void => onOpenPage(page)}
              sx={{
                width: "100%",
                minHeight: 34,
                px: 1.25,
                py: 0.4,
                borderRadius: 1.25,
                justifyContent: "flex-start",
                textAlign: "left",
                "&:hover": { bgcolor: "action.hover" },
              }}
            >
              <Typography variant="caption" color="text.secondary" sx={{ mr: 0.75, flexShrink: 0 }}>
                #{page.ordinal}
              </Typography>
              <Typography variant="body2" noWrap sx={{ fontSize: "0.8rem" }}>
                {page.title}
              </Typography>
            </ButtonBase>
          ))}
        </Stack>
      </Collapse>
    </Box>
  );
}

function LineageExplorePageDialog({
  selected,
  desktop,
  onClose,
}: {
  selected: { hop: LineageHop; title: string; events: Envelope[] };
  desktop: boolean;
  onClose: () => void;
}): React.JSX.Element {
  const items = useMemo(
    () => derive(selected.events).filter((item) => item.kind !== "forked"),
    [selected.events],
  );
  return (
    <Dialog open onClose={onClose} maxWidth="md" fullWidth>
      <DialogTitle sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", py: 1.25 }}>
        <Typography variant="subtitle2" noWrap sx={{ fontWeight: 600 }}>
          {selected.title}
        </Typography>
        <IconButton size="small" aria-label="Close" onClick={onClose}>
          <CloseRounded fontSize="small" />
        </IconButton>
      </DialogTitle>
      <DialogContent dividers>
        <Stack spacing={1}>
          {items.map((item) => (
            <ReadOnlyAncestorItemView
              key={item.key}
              item={item}
              originSessionId={selected.hop.session_id}
              provider={selected.hop.provider}
              desktop={desktop}
              selectedToolKey={null}
              onOpenTool={(): void => {}}
            />
          ))}
        </Stack>
      </DialogContent>
    </Dialog>
  );
}

async function fetchHopQuestions(
  sessionId: string,
  hopId: string,
  beforeSeq?: number,
  signal?: AbortSignal,
): Promise<QuestionPagesResponse | null> {
  try {
    const query = beforeSeq !== undefined ? `limit=64&before_seq=${String(beforeSeq)}` : "limit=64";
    const res = await fetch(
      `/api/sessions/${encodeURIComponent(sessionId)}/lineage/${encodeURIComponent(hopId)}/question-pages?${query}`,
      { credentials: "same-origin", signal: signal ?? null },
    );
    return res.ok ? ((await res.json()) as QuestionPagesResponse) : null;
  } catch {
    return null;
  }
}

/** Shows inherited ancestor Explore question pages across ALL reachable lineage
 *  hops strictly `< edge.before_seq`, with per-hop pagination. */
export function LineageExploreQuestions({
  sessionId,
  desktop,
  defaultExpanded = false,
}: {
  sessionId: string;
  desktop: boolean;
  defaultExpanded?: boolean;
}): React.JSX.Element | null {
  const hasLineage = useStoreSelector((state) => {
    const s = state.sessions.find((entry) => entry.id === sessionId);
    return Boolean(s?.forked_from || s?.continued_from);
  });
  const { lineage } = useSessionLineage(sessionId, hasLineage);
  const eligibleHops = useMemo(
    () =>
      (lineage?.hops ?? []).filter(
        (hop) =>
          hop.before_seq !== null &&
          hop.before_seq > 0 &&
          (hop.history_state === "live" || hop.history_state === "archived"),
      ),
    [lineage?.hops],
  );
  const hopSignature = useMemo(
    () => eligibleHops.map((h) => `${h.session_id}:${String(h.edge_version)}:${String(h.before_seq)}`).join("|"),
    [eligibleHops],
  );
  const [byHop, setByHop] = useState<Record<string, HopPagesState>>({});
  const [expandedByHop, setExpandedByHop] = useState<Record<string, boolean>>({});
  const [selected, setSelected] = useState<{ hop: LineageHop; title: string; events: Envelope[] } | null>(null);

  useEffect(() => {
    if (eligibleHops.length === 0) {
      setByHop({});
      return undefined;
    }
    const controller = new AbortController();
    void Promise.all(
      eligibleHops.map(async (hop) => {
        const data = await fetchHopQuestions(sessionId, hop.session_id, undefined, controller.signal);
        if (!data) return null;
        return {
          hopId: hop.session_id,
          pages: Array.isArray(data.pages) ? data.pages : [],
          nextBeforeSeq: typeof data.next_before_seq === "number" ? data.next_before_seq : null,
        };
      }),
    ).then((results) => {
      if (controller.signal.aborted) return;
      const next: Record<string, HopPagesState> = {};
      for (const entry of results) {
        if (entry) next[entry.hopId] = { pages: entry.pages, nextBeforeSeq: entry.nextBeforeSeq, loading: false };
      }
      setByHop(next);
    });
    return () => controller.abort();
  }, [hopSignature, sessionId]);

  const loadEarlierForHop = useCallback(
    (hop: LineageHop): void => {
      const current = byHop[hop.session_id];
      if (!current || current.loading || current.nextBeforeSeq === null) return;
      const cursor = current.nextBeforeSeq;
      setByHop((prev) => ({ ...prev, [hop.session_id]: { ...current, loading: true } }));
      void fetchHopQuestions(sessionId, hop.session_id, cursor).then((data) => {
          setByHop((prev) => {
            const existing = prev[hop.session_id];
            if (!existing) return prev;
            if (!data || !Array.isArray(data.pages)) {
              return { ...prev, [hop.session_id]: { ...existing, loading: false } };
            }
            const seen = new Set(existing.pages.map((p) => p.id));
            return {
              ...prev,
              [hop.session_id]: {
                pages: [...data.pages.filter((p) => !seen.has(p.id)), ...existing.pages],
                nextBeforeSeq: typeof data.next_before_seq === "number" ? data.next_before_seq : null,
                loading: false,
              },
            };
          });
        })
        .catch(() => {
          setByHop((prev) => {
            const existing = prev[hop.session_id];
            return existing ? { ...prev, [hop.session_id]: { ...existing, loading: false } } : prev;
          });
        });
    },
    [byHop, sessionId],
  );

  const openPage = useCallback(
    (hop: LineageHop, page: QuestionSummary): void => {
      void fetch(
        `/api/sessions/${encodeURIComponent(sessionId)}/lineage/${encodeURIComponent(hop.session_id)}/question-pages/${String(page.id)}`,
        { credentials: "same-origin" },
      )
        .then((res) => (res.ok ? (res.json() as Promise<{ events: Envelope[] }>) : null))
        .then((data) => {
          if (data && Array.isArray(data.events)) setSelected({ hop, title: page.title, events: data.events });
        })
        .catch(() => {});
    },
    [sessionId],
  );

  const populatedHops = eligibleHops.filter((hop) => (byHop[hop.session_id]?.pages.length ?? 0) > 0);
  if (populatedHops.length === 0) return null;

  return (
    <Box data-lineage-explore-questions sx={{ mb: 1, borderBottom: 1, borderColor: "divider", pb: 0.75 }}>
      <Stack spacing={0.5}>
        {populatedHops.map((hop, idx) => {
          const expanded = expandedByHop[hop.session_id] ?? (defaultExpanded && idx === 0);
          return (
            <LineageExploreHopSection
              key={`${hop.session_id}:${hop.edge_version}`}
              hop={hop}
              state={byHop[hop.session_id]!}
              expanded={expanded}
              onToggle={(): void => setExpandedByHop((prev) => ({ ...prev, [hop.session_id]: !expanded }))}
              onLoadEarlier={(): void => loadEarlierForHop(hop)}
              onOpenPage={(page): void => openPage(hop, page)}
            />
          );
        })}
      </Stack>
      {selected && (
        <LineageExplorePageDialog
          selected={selected}
          desktop={desktop}
          onClose={(): void => setSelected(null)}
        />
      )}
    </Box>
  );
}
