import { type ComponentProps, useMemo } from "react";
import {
  checkDiagnosticsForPath,
  withCheckDiagnostics,
} from "./checkAnnotations";
import CodeViewer from "./CodeViewer";
import { useReviewChecks } from "./reviewChecksStore";

/** The session's code viewer with project check findings annotated on
 *  source files, beside the language engine's own diagnostics. */
export default function CheckedCodeViewer(
  { sessionId, ...props }: ComponentProps<typeof CodeViewer> & {
    sessionId: string;
  },
): React.JSX.Element {
  const checks = useReviewChecks(sessionId);
  const { kind, path, languageData } = props;
  const annotated = useMemo(
    () =>
      kind === "source"
        ? withCheckDiagnostics(
          languageData,
          path,
          checkDiagnosticsForPath(checks, path),
        )
        : languageData,
    [checks, kind, languageData, path],
  );
  return <CodeViewer {...props} languageData={annotated} />;
}
