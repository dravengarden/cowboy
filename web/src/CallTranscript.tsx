import { useEffect } from "react";
import { Box } from "@mui/material";
import { Transcript } from "./Transcript";
import type { Envelope } from "./protocol";
import { peekSession, useStoreSelector } from "./store";

const EMPTY: Envelope[] = [];

/**
 * A managed call's child conversation shown inside the Calls modal. It renders
 * the same Transcript as the opened session, kept loaded and live while shown
 * (`peekSession`), without opening the child or reviving its agent.
 */
export function CallTranscript({
  sessionId,
  provider,
  bottomInset,
  desktop = false,
}: {
  sessionId: string;
  provider: string;
  /** Desktop reads like a document: short content starts at the top. */
  desktop?: boolean;
  /** Clearance for a floating footer the newest message passes under. */
  bottomInset?: string;
}): React.JSX.Element {
  useEffect(() => peekSession(sessionId), [sessionId]);
  const timeline = useStoreSelector((snapshot) =>
    snapshot.timelines.get(sessionId)
  );
  const hydrated = useStoreSelector((snapshot) =>
    snapshot.hydrated.has(sessionId)
  );
  const connected = useStoreSelector((snapshot) => snapshot.connected);
  const session = useStoreSelector((snapshot) =>
    snapshot.sessions.find((candidate) => candidate.id === sessionId)
  );
  return (
    <Box
      data-call-transcript={sessionId}
      sx={{
        flex: 1,
        minHeight: 0,
        display: "flex",
        flexDirection: "column",
        position: "relative",
      }}
    >
      <Transcript
        sessionId={sessionId}
        timeline={timeline ?? EMPTY}
        status={session?.status ?? "running"}
        provider={session?.provider ?? provider}
        providerVersion={session?.provider_version}
        providerDigest={session?.provider_generation_digest}
        cwd={session?.cwd ?? ""}
        loading={!hydrated}
        connected={connected}
        topInset="0px"
        bottomInset={bottomInset}
        shortContentAtTop={desktop}
        statusBar={false}
      />
    </Box>
  );
}
