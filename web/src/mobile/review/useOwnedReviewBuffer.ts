import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  captureContent,
  type CapturedContent,
} from "../../codeBuffers/content.ts";
import { productCodeBuffers } from "../../codeBuffers/product.ts";
import {
  BufferClientError,
  type DocumentSymbol,
} from "../../codeBuffers/protocol.ts";
import type {
  CodeDocumentSymbol,
  CodeHover,
  CodeLanguage,
  CodeOutline,
} from "./codeApi.ts";
import {
  createReviewBuffer,
  type ReviewBuffer,
  type ReviewBufferSource,
} from "./ownedReviewBuffer.ts";

export type ReviewCodeStatus =
  | "checking"
  | "ready"
  | "mismatch"
  | "unavailable"
  | "incomplete"
  | "synchronization";
type Capture = {
  reader: ReviewBuffer;
  content: CapturedContent;
  signal: AbortSignal;
};
type Evidence = {
  capture: Capture;
  status: ReviewCodeStatus;
  language?: CodeLanguage;
};

function symbols(values: readonly DocumentSymbol[]): CodeDocumentSymbol[] {
  return values.map((value) => ({
    ...value,
    children: symbols(value.children),
  }));
}

/** The same LF text is handed to both CodeMirror and the content capture. */
export function reviewDisplayText(text: string): string {
  return text.replace(/\r\n?/g, "\n");
}

export function useOwnedReviewBuffer(
  sessionId: string,
  path: string,
  enabled: boolean,
  completeText: string | undefined,
  source: ReviewBufferSource = productCodeBuffers,
) {
  const [member, setMember] = useState<
    { sessionId: string; path: string; reader: ReviewBuffer }
  >();
  const [capture, setCapture] = useState<Capture>();
  const [captureFailed, setCaptureFailed] = useState<string>();
  const [evidence, setEvidence] = useState<Evidence>();
  const contentObserver = useRef<AbortController | undefined>(undefined);
  useEffect(() => {
    if (!enabled) return;
    const reader = createReviewBuffer(source, { sessionId, path });
    setMember({ sessionId, path, reader });
    return () => {
      void reader.close();
    };
  }, [enabled, sessionId, path, source]);
  const reader =
    enabled && member?.sessionId === sessionId && member.path === path
      ? member.reader
      : undefined;
  // End old text/Apply authority at commit, before paint or a subsequent click.
  // Capturing/hashing new text remains post-paint work, not a layout measurement.
  useLayoutEffect(() => () => contentObserver.current?.abort(), [
    reader,
    completeText,
  ]);
  useLayoutEffect(() => () => {
    void reader?.close();
  }, [reader]);
  useEffect(() => {
    setCapture(undefined);
    setCaptureFailed(undefined);
    if (!reader || completeText === undefined) return;
    const observer = new AbortController();
    contentObserver.current = observer;
    void captureContent(completeText).then((content) => {
      if (!observer.signal.aborted) {
        setCapture({ reader, content, signal: observer.signal });
      }
    }).catch(() => {
      if (!observer.signal.aborted) setCaptureFailed(completeText);
    });
    return () => observer.abort();
  }, [reader, completeText]);

  // Render-time equality suppresses old results even before effect cleanup.
  const current =
    capture?.reader === reader && capture?.content.text === completeText &&
      !capture?.signal.aborted
      ? capture
      : undefined;
  const report = useCallback(
    (selected: Capture, status: ReviewCodeStatus, language?: CodeLanguage) => {
      if (!selected.signal.aborted) {
        setEvidence({
          capture: selected,
          status,
          ...(language ? { language } : {}),
        });
      }
    },
    [],
  );
  const language = useCallback(
    async (selected: Capture, reconcile: boolean) => {
      report(selected, "checking");
      try {
        const result = await selected.reader.read(
          selected.content,
          { kind: "language" },
          selected.signal,
          reconcile,
        );
        const observed = result.result.result;
        if (observed.kind === "mismatch") {
          report(selected, "mismatch");
          return;
        }
        const value = observed.observation;
        report(selected, "ready", {
          apiVersion: 1,
          path,
          // Original open's lower bound only. Equality came from readContent.
          version: [...result.openedVersion],
          diagnostics: value.diagnostics.map(({ source, ...value }) => ({
            ...value,
            ...(source === null ? {} : { source }),
          })),
          inlayHints: value.inlayHints.map(({ kind, ...value }) => ({
            ...value,
            ...(kind === null ? {} : { kind }),
          })),
          semanticTokens: [...value.semanticTokens],
        });
      } catch {
        report(selected, "unavailable");
      }
    },
    [path, report],
  );
  useEffect(() => {
    if (current) void language(current, false);
  }, [current, language]);
  const matching = evidence?.capture === current ? evidence : undefined;

  return useMemo(() => ({
    status: (completeText === undefined
      ? "incomplete"
      : captureFailed === completeText
      ? "unavailable"
      : matching?.status ?? "checking") as ReviewCodeStatus,
    language: matching?.language,
    // This identity changes for every displayed snapshot, including equal-text
    // ABA after a loading interval. Outline must end its previous observer too.
    identity: current,
    async hover(
      row: number,
      column: number,
      observer: AbortSignal,
    ): Promise<CodeHover> {
      if (!current) {
        throw new BufferClientError("state");
      }
      const signal = AbortSignal.any([current.signal, observer]);
      const result = await current.reader.read(current.content, {
        kind: "hover",
        position: { row, column },
      }, signal);
      const value = result.result.result;
      if (value.kind === "mismatch") {
        report(current, "mismatch");
        throw new BufferClientError("state");
      }
      return {
        apiVersion: 1,
        path,
        contents: value.contents.map(({ language, ...block }) => ({
          ...block,
          ...(language === null ? {} : { language }),
        })),
      };
    },
    async outline(observer: AbortSignal): Promise<CodeOutline> {
      if (!current) {
        throw new BufferClientError("state");
      }
      const result = await current.reader.read(current.content, {
        kind: "symbols",
      }, AbortSignal.any([current.signal, observer]));
      const value = result.result.result;
      if (value.kind === "mismatch") {
        report(current, "mismatch");
        throw new BufferClientError("state");
      }
      return {
        apiVersion: 1,
        path,
        symbols: symbols(value.observation.symbols),
      };
    },
    check() {
      if (current) {
        void language(current, true);
      }
    },
    prepareRefresh() {
      if (!current) {
        return;
      }
      report(current, "checking");
      void current.reader.prepareRefresh(current.content, current.signal).then(
        () => {
          report(current, "synchronization");
        },
      ).catch(() =>
        report(current, "unavailable")
      );
    },
  }), [completeText, captureFailed, matching, current, path, report, language]);
}

export type OwnedReviewIntelligence = ReturnType<typeof useOwnedReviewBuffer>;
