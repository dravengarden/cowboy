import { type ComponentProps, useMemo } from "react";
import { checkDiagnosticsForPath, withCheckDiagnostics } from "../../review/checkAnnotations";
import CodeViewer from "./CodeViewer";
import { diffCheckAnnotations } from "../../review/diffCheckAnnotations";
import { useReviewChecks } from "../../review/reviewChecksStore";

/** The session's code viewer with project check findings annotated on
 *  source files, beside the language engine's own diagnostics. */
export default function CheckedCodeViewer({
  sessionId,
  ...props
}: ComponentProps<typeof CodeViewer> & {
  sessionId: string;
}): React.JSX.Element {
  const checks = useReviewChecks(sessionId);
  const { kind, path, languageData, text, mapDiffPoint } = props;
  const annotated = useMemo(
    () =>
      kind === "source"
        ? withCheckDiagnostics(languageData, path, checkDiagnosticsForPath(checks, path))
        : withCheckDiagnostics(
            undefined,
            path,
            mapDiffPoint
              ? diffCheckAnnotations(text, checkDiagnosticsForPath(checks, path), mapDiffPoint)
              : [],
          ),
    [checks, kind, languageData, path, text, mapDiffPoint],
  );
  return <CodeViewer {...props} languageData={annotated} />;
}
