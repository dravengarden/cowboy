import { Box, ButtonBase, Collapse, Dialog, DialogContent, DialogTitle, IconButton, Stack, Typography } from "@mui/material";
import CallSplitOutlined from "@mui/icons-material/CallSplitOutlined";
import CloseRounded from "@mui/icons-material/CloseRounded";
import ExpandMore from "@mui/icons-material/ExpandMore";
import { useEffect, useMemo, useState } from "react";
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

/** Shows inherited ancestor Explore question pages strictly `< edge.before_seq`
 *  above the current branch's own Explore list. */
export function LineageExploreQuestions({
  sessionId,
  desktop,
}: {
  sessionId: string;
  desktop: boolean;
}): React.JSX.Element | null {
  const hasLineage = useStoreSelector((state) => {
    const s = state.sessions.find((entry) => entry.id === sessionId);
    return Boolean(s?.forked_from || s?.continued_from);
  });
  const { lineage } = useSessionLineage(sessionId, hasLineage);
  const hop = lineage?.hops[0];
  const [expanded, setExpanded] = useState(false);
  const [pages, setPages] = useState<QuestionSummary[]>([]);
  const [selected, setSelected] = useState<{ title: string; events: Envelope[] } | null>(null);

  useEffect(() => {
    if (!hop || hop.before_seq === null || hop.before_seq === 0) {
      setPages([]);
      return undefined;
    }
    const controller = new AbortController();
    void fetch(
      `/api/sessions/${encodeURIComponent(sessionId)}/lineage/${encodeURIComponent(hop.session_id)}/question-pages?limit=64`,
      { credentials: "same-origin", signal: controller.signal },
    )
      .then((res) => (res.ok ? (res.json() as Promise<{ pages: QuestionSummary[] }>) : null))
      .then((data) => {
        if (data && Array.isArray(data.pages)) setPages(data.pages);
      })
      .catch(() => {});
    return () => controller.abort();
  }, [hop?.session_id, hop?.before_seq, sessionId]);

  const selectedItems = useMemo(
    () => (selected ? derive(selected.events).filter((item) => item.kind !== "forked") : []),
    [selected],
  );

  if (!hop || pages.length === 0) return null;

  const openPage = (page: QuestionSummary): void => {
    void fetch(
      `/api/sessions/${encodeURIComponent(sessionId)}/lineage/${encodeURIComponent(hop.session_id)}/question-pages/${String(page.id)}`,
      { credentials: "same-origin" },
    )
      .then((res) => (res.ok ? (res.json() as Promise<{ events: Envelope[] }>) : null))
      .then((data) => {
        if (data && Array.isArray(data.events)) {
          setSelected({ title: page.title, events: data.events });
        }
      })
      .catch(() => {});
  };

  return (
    <Box data-lineage-explore-questions sx={{ mb: 1, borderBottom: 1, borderColor: "divider", pb: 0.75 }}>
      <ButtonBase
        onClick={(): void => setExpanded((value) => !value)}
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
            Earlier from {hop.title || hop.session_id} ({pages.length})
          </Typography>
        </Stack>
        <ExpandMore sx={{ fontSize: glyphSize(16), transform: expanded ? "rotate(180deg)" : undefined }} />
      </ButtonBase>
      <Collapse in={expanded}>
        <Stack spacing={0.25} sx={{ pt: 0.5 }}>
          {pages.map((page) => (
            <ButtonBase
              key={`${hop.session_id}:${page.id}`}
              onClick={(): void => openPage(page)}
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
      {selected && (
        <Dialog open onClose={(): void => setSelected(null)} maxWidth="md" fullWidth>
          <DialogTitle sx={{ display: "flex", alignItems: "center", justifyContent: "space-between", py: 1.25 }}>
            <Typography variant="subtitle2" noWrap sx={{ fontWeight: 600 }}>
              {selected.title}
            </Typography>
            <IconButton size="small" aria-label="Close" onClick={(): void => setSelected(null)}>
              <CloseRounded fontSize="small" />
            </IconButton>
          </DialogTitle>
          <DialogContent dividers>
            <Stack spacing={1}>
              {selectedItems.map((item) => (
                <ReadOnlyAncestorItemView
                  key={item.key}
                  item={item}
                  originSessionId={hop.session_id}
                  provider={hop.provider}
                  desktop={desktop}
                  selectedToolKey={null}
                  onOpenTool={(): void => {}}
                />
              ))}
            </Stack>
          </DialogContent>
        </Dialog>
      )}
    </Box>
  );
}
