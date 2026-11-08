import { ErrorOutline, InfoOutlined, PlayArrowOutlined, WarningAmber } from "@mui/icons-material";
import { Alert, Box, Button, List, ListItemButton, ListItemText, ListSubheader, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { desktopSize } from "../../surface/desktopSize";
import { notify } from "../../store";
import { DESKTOP_SURFACE_RADIUS } from "../DesktopEmbeddedControl";
import { LeaderKeycap } from "../commands/DesktopKeycap";
import { sendCheckRequest } from "../../mobile/review/checkRequest";
import { checkStatusSummary } from "../../mobile/review/checkAnnotations";
import { refreshReviewChecks } from "../../mobile/review/reviewChecksStore";
import { symbolKindLabel } from "../../mobile/review/outlineModel";
import { revealEditorLine } from "./reviewEditorSelection";
import { adjacentLine } from "./reviewDocumentKeys";
import { useInspectorData } from "./useInspectorData";
import type { ReviewWorkbench } from "./useReviewWorkbench";

export const REVIEW_INSPECTOR_REGION = "review.inspector";
export const REVIEW_RUN_CHECK_EVENT = "cowboy:review-run-check";

/** Focus the first symbol once the (possibly just shown) Inspector has it. */
export function focusOutline(list: HTMLElement | null, frames = 60): void {
  requestAnimationFrame(() => {
    const row = list?.querySelector<HTMLElement>("[data-review-inspector-section='outline'] [data-desktop-item]:not(.Mui-disabled)") ??
      list?.querySelector<HTMLElement>("[data-desktop-item]:not(.Mui-disabled)");
    if (row && row.offsetParent !== null) row.focus();
    else if (frames > 0) focusOutline(list, frames - 1);
  });
}

const rowSx = {
  py: 0,
  px: 1,
  minHeight: desktopSize(26),
  borderRadius: `${String(DESKTOP_SURFACE_RADIUS)}px`,
} as const;
const SEVERITY_ICON = {
  error: <ErrorOutline sx={{ fontSize: "0.95rem", color: "error.main" }} />,
  warning: <WarningAmber sx={{ fontSize: "0.95rem", color: "warning.main" }} />,
  note: <InfoOutlined sx={{ fontSize: "0.95rem", color: "info.main" }} />,
} as const;

function Section({ id, title, count, children }: {
  id: string;
  title: string;
  count?: number | undefined;
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <li data-review-inspector-section={id}>
      <ul style={{ padding: 0 }}>
        <ListSubheader sx={{ lineHeight: desktopSize(26), fontSize: "0.68rem", bgcolor: "background.paper" }}>
          {title}{count === undefined ? "" : ` · ${String(count)}`}
        </ListSubheader>
        {children}
      </ul>
    </li>
  );
}

function Empty({ children }: { children: React.ReactNode }): React.JSX.Element {
  return <Typography sx={{ px: 2, py: 0.5, fontSize: "0.72rem", color: "text.secondary" }}>{children}</Typography>;
}

/** Run a declared check of the Session's own worktree; an uncertain reply
 *  keeps its operation ID so a retry asks about the same request. */
function useRunCheck(sessionId: string): (check: string, epoch: string) => void {
  const operations = useRef(new Map<string, { operation: string; epoch: string }>());
  return (check, epoch) => {
    const key = `${sessionId}:${check}`;
    const request = operations.current.get(key) ?? { operation: crypto.randomUUID(), epoch };
    operations.current.set(key, request);
    void sendCheckRequest(sessionId, check, request).then((result) => {
      if (result.forget) operations.current.delete(key);
      if (result.message) notify(result.message, "warning");
      refreshReviewChecks(sessionId);
    });
  };
}

function InspectorDetails({ review, placement }: {
  review: ReviewWorkbench;
  placement: ReturnType<typeof useInspectorData>["placement"];
}): React.JSX.Element {
  const ready = review.load?.state === "ready" ? review.load.value : undefined;
  return (
    <Section id="details" title="Details">
      {review.document
        ? (
          <Box sx={{ px: 2, fontSize: "0.7rem", color: "text.secondary", display: "grid", gridTemplateColumns: "auto 1fr", columnGap: 1, rowGap: 0.25 }} data-review-details>
            <span>Path</span><Box component="span" sx={{ wordBreak: "break-all", color: "text.primary" }}>{review.document.path}</Box>
            <span>Kind</span><span>{{ diff: "Changes", source: "File", commit: "Commit", pr: "Pull request" }[review.document.kind]}</span>
            {review.document.kind === "diff" && <><span>Against</span><span>{review.document.comparison ?? review.document.scope}</span></>}
            {ready?.added !== undefined && <><span>Lines</span><span>+{ready.added} −{ready.removed ?? 0}</span></>}
            {ready?.revision && <><span>Revision</span><Box component="span" sx={{ fontFamily: "monospace" }}>{ready.revision.slice(0, 12)}</Box></>}
            {review.openReviewed !== undefined && <><span>Reviewed</span><span>{review.openReviewed ? "yes" : "no"}</span></>}
            <span>Placement</span><span>{{ source: "file lines", verified: "verified against the file", unverified: "not verified", none: "patch only" }[placement]}</span>
          </Box>
        )
        : <Empty>No file open.</Empty>}
    </Section>
  );
}

/**
 * The Inspector (docs/desktop-code-review.md "Inspector"): the open file's
 * outline, hunks, problems and facts. `Enter` on a row moves the Document's
 * cursor there; `]d`/`[d` step the placed problems; `gO` arrives here.
 */
export function ReviewInspector({ review, onShow }: {
  review: ReviewWorkbench;
  /** Make the Inspector visible (a narrow Review keeps it folded). */
  onShow: () => void;
}): React.JSX.Element {
  const data = useInspectorData(review);
  const list = useRef<HTMLUListElement>(null);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = globalThis.setInterval(() => setNow(Date.now()), 30_000);
    return () => globalThis.clearInterval(timer);
  }, []);
  const own = review.target?.own === true;
  const runCheck = useRunCheck(review.target?.sessionId ?? "");
  const runnable = own && data.checks?.allowed === true && data.checks.runnable === true && !!data.checks.epoch;
  const jump = (row: number | undefined): void => {
    const view = review.editor.current;
    if (!view || row === undefined) return;
    revealEditorLine(view, row);
    view.focus();
  };
  const latest = useRef(data);
  latest.current = data;
  const latestShow = useRef(onShow);
  latestShow.current = onShow;
  useEffect(() => {
    review.inspectorAction.current = (action, view) => {
      if (action.kind === "outline") {
        latestShow.current();
        focusOutline(list.current);
        return;
      }
      if (action.kind !== "problem") return;
      const rows = latest.current.problems.flatMap((problem) => problem.row === undefined ? [] : [problem.row]);
      const cursor = view.state.doc.lineAt(view.state.selection.main.head).number;
      const row = adjacentLine([...new Set(rows)].sort((a, b) => a - b), cursor, action.offset);
      if (row === undefined) notify(rows.length === 0 ? "No problems placed in this document" : "No more problems", "info");
      else revealEditorLine(view, row);
    };
    return () => {
      review.inspectorAction.current = undefined;
    };
  }, [review.inspectorAction]);
  useEffect(() => {
    const element = list.current?.closest<HTMLElement>(`[data-desktop-region="${REVIEW_INSPECTOR_REGION}"]`);
    const run = (): void => {
      const checks = latest.current.checks;
      const first = checks?.declared[0];
      if (!runnable || !checks?.epoch || !first) {
        notify(own ? "No project check can run here" : "Checks run only in the Session's own worktree", "info");
        return;
      }
      if (checks.declared.length === 1) runCheck(first.id, checks.epoch);
      else list.current?.querySelector<HTMLElement>("[data-review-check-action]")?.focus();
    };
    element?.addEventListener(REVIEW_RUN_CHECK_EVENT, run);
    return () => element?.removeEventListener(REVIEW_RUN_CHECK_EVENT, run);
  });
  const status = review.document && data.checks ? checkStatusSummary(data.checks, review.document.path, now) : undefined;
  return (
    <List ref={list} dense disablePadding data-review-inspector sx={{ height: "100%", overflowY: "auto", px: 0.5, pb: 2 }}>
      <Section id="outline" title="Outline" count={Array.isArray(data.outline) ? data.outline.length : undefined}>
        {!review.document && <Empty>Open a file to see its symbols.</Empty>}
        {data.outline === "unavailable" && <Empty>No outline for this file.</Empty>}
        {Array.isArray(data.outline) && data.outline.map((symbol) => (
          <ListItemButton
            key={`${symbol.name}:${String(symbol.line)}:${String(symbol.depth)}`}
            data-desktop-item={`symbol:${String(symbol.line)}:${symbol.name}`}
            disabled={symbol.row === undefined}
            onClick={() => jump(symbol.row)}
            sx={{ ...rowSx, pl: 1 + symbol.depth * 1.5 }}
          >
            <ListItemText
              primary={symbol.name}
              secondary={symbolKindLabel(symbol.kind)}
              slotProps={{ primary: { noWrap: true, sx: { fontSize: "0.76rem" } }, secondary: { sx: { fontSize: "0.64rem" } } }}
              sx={{ my: 0, display: "flex", alignItems: "baseline", gap: 0.75 }}
            />
          </ListItemButton>
        ))}
      </Section>
      {data.hunks.length > 0 && (
        <Section id="hunks" title="Hunks" count={data.hunks.length}>
          {data.hunks.map((hunk) => (
            <ListItemButton key={hunk.row} data-desktop-item={`hunk:${String(hunk.row)}`} onClick={() => jump(hunk.row + 1)} sx={rowSx}>
              <ListItemText
                primary={hunk.header.replace(/^(@@[^@]*@@).*/, "$1")}
                slotProps={{ primary: { noWrap: true, sx: { fontSize: "0.72rem", fontFamily: "monospace" } } }}
                sx={{ my: 0 }}
              />
              <Typography component="span" sx={{ fontFamily: "monospace", fontSize: "0.68rem", flexShrink: 0 }}>
                <Box component="span" sx={{ color: "success.main" }}>+{hunk.added}</Box>{" "}
                <Box component="span" sx={{ color: "error.main" }}>−{hunk.removed}</Box>
              </Typography>
            </ListItemButton>
          ))}
        </Section>
      )}
      <Section id="problems" title="Problems" count={data.problems.length}>
        {status && <Typography sx={{ px: 2, py: 0.25, fontSize: "0.68rem", color: "text.secondary", whiteSpace: "pre-line" }}>{status.summary}</Typography>}
        {data.placement === "unverified" && data.problems.length > 0 && (
          <Alert severity="info" sx={{ mx: 1, py: 0, fontSize: "0.7rem" }}>
            This diff is not verified against the file, so findings are listed but not placed. Open the file to step through them.
          </Alert>
        )}
        {data.problems.length === 0 && <Empty>{data.checks?.runs.length ? "No findings in this file." : "No check has run."}</Empty>}
        {data.problems.map((problem) => (
          <ListItemButton key={problem.key} data-desktop-item={problem.key} data-review-problem={problem.line} disabled={problem.row === undefined} onClick={() => jump(problem.row)} sx={{ ...rowSx, gap: 0.75, alignItems: "flex-start", py: 0.25 }}>
            {SEVERITY_ICON[problem.severity]}
            <ListItemText
              primary={problem.message}
              secondary={`${problem.source} · line ${String(problem.line)}`}
              slotProps={{ primary: { sx: { fontSize: "0.74rem" } }, secondary: { sx: { fontSize: "0.64rem" } } }}
              sx={{ my: 0 }}
            />
          </ListItemButton>
        ))}
        {runnable && data.checks && (
          <Box sx={{ display: "flex", flexWrap: "wrap", gap: 0.5, px: 1, pt: 0.5, alignItems: "center" }}>
            {data.checks.declared.map((check) => (
              <Button
                key={check.id}
                size="small"
                data-desktop-item={`check:${check.id}`}
                data-review-check-action={check.id}
                startIcon={<PlayArrowOutlined />}
                disabled={data.checks!.runs.some((run) => run.check === check.id && run.state === "running")}
                onClick={() => runCheck(check.id, data.checks!.epoch!)}
                sx={{ textTransform: "none", fontSize: "0.72rem" }}
              >
                {check.title ?? check.id}
              </Button>
            ))}
            <LeaderKeycap leaderKey="VK" />
          </Box>
        )}
      </Section>
      <InspectorDetails review={review} placement={data.placement} />
    </List>
  );
}
