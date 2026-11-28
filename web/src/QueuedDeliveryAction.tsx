import { Send } from "@mui/icons-material";
import { IconButton, Tooltip } from "@mui/material";
import { useState } from "react";
import type { DeliveryTarget } from "./currentTurnDelivery";
import { DeliveryPicker } from "./DeliveryPicker";
import { NetworkIconButton } from "./NetworkActionFeedback";
import type { Status } from "./protocol";
import { isCurrentContextReady } from "./sessionContext";
import { useAnnouncedSessionContext } from "./SessionContextCue";
import {
  cancelTurn,
  currentTurnPrompt,
  notify,
  type QueuedMessage,
  removeQueued,
  requestSendQueued,
  setQueueEditing,
  useConnected,
  useStoreSelector,
  wrapUpQueued,
} from "./store";
import { getQueueEditHoldId } from "./queueEditingHold";
import { isOwnerAuthoredRow } from "./wrapUpDelivery";

export function QueuedDeliveryAction({
  desktop,
  dispatchable,
  sessionId,
  message,
  status,
}: {
  desktop: boolean;
  dispatchable: boolean;
  sessionId: string;
  message: QueuedMessage;
  status: Status;
}): React.JSX.Element {
  const [deliveryAnchor, setDeliveryAnchor] = useState<HTMLElement | null>(null);
  const queuedAhead = useStoreSelector((snapshot) =>
    Math.max(
      0,
      (snapshot.queues.get(sessionId) ?? []).findIndex((row) => row.id === message.id),
    ),
  );
  const queuePaused = useStoreSelector(
    (snapshot) => snapshot.sessions.find((session) => session.id === sessionId)?.paused ?? false,
  );
  const sessionProvider = useStoreSelector(
    (snapshot) => snapshot.sessions.find((session) => session.id === sessionId)?.provider,
  );
  const disallowAntigravitySteer = sessionProvider === "antigravity";
  const queueSteerCapable = useStoreSelector(
    (snapshot) => {
      const session = snapshot.sessions.find((candidate) => candidate.id === sessionId);
      return session?.provider !== "antigravity" && session?.supports_current_turn === true;
    },
  );
  const queueWrapUpCapable = useStoreSelector(
    (snapshot) =>
      snapshot.sessions.find((candidate) => candidate.id === sessionId)?.wrap_up_steering === true,
  );
  const sessionContext = useAnnouncedSessionContext(sessionId);
  const currentContextReady = isCurrentContextReady(sessionContext);
  const ownerAuthored = isOwnerAuthoredRow(message);
  const queueConnected = useConnected();
  const wrapUpRow = (target: DeliveryTarget | undefined): void => {
    setDeliveryAnchor(null);
    void wrapUpQueued(sessionId, message.id, target)
      .then((result) => {
        if (result.hinted) notify("Asked agent to wrap up — runs next", "info");
      })
      .catch((error: Error) => notify(error.message));
  };
  const steerQueued = (target: DeliveryTarget | undefined, nextCheckpoint = false): void => {
    setDeliveryAnchor(null);
    const prevHold = getQueueEditHoldId(sessionId);
    // Hold auto-drain while steering so the queued original cannot race and double-send.
    if (prevHold === null) setQueueEditing(sessionId, message.id);
    // Current-turn delivery persists the message before removing the queued original.
    void currentTurnPrompt(sessionId, message.text, message.attachments, target, nextCheckpoint)
      .then(() => removeQueued(sessionId, message.id))
      .catch((error: Error) => notify(error.message))
      .finally(() => {
        if (prevHold === null && getQueueEditHoldId(sessionId) === message.id) {
          setQueueEditing(sessionId, null);
        }
      });
  };

  const primary = dispatchable ? (
    <Tooltip title="Send now">
      <NetworkIconButton
        data-desktop-item-action="default"
        size="small"
        color="primary"
        aria-label="send now"
        reliableTouch={!desktop}
        networkAction={() => requestSendQueued(sessionId, message.id)}
      >
        <Send fontSize="small" />
      </NetworkIconButton>
    </Tooltip>
  ) : (
    <Tooltip
      title={queueWrapUpCapable && !queueSteerCapable
        ? "Send: Ask agent to wrap up, or run it first"
        : "Send: Steer into the current turn, or run it first"}
    >
      <IconButton
        data-desktop-item-action="default"
        size="small"
        color="primary"
        aria-label="send now"
        aria-haspopup="menu"
        aria-expanded={deliveryAnchor !== null}
        onClick={(event): void => setDeliveryAnchor(event.currentTarget)}
      >
        <Send fontSize="small" />
      </IconButton>
    </Tooltip>
  );

  return (
    <>
      {primary}
      {deliveryAnchor && (
        <DeliveryPicker
          sessionId={sessionId}
          anchor={deliveryAnchor}
          busy={status === "busy"}
          steerReady={queueConnected && status === "busy" && queueSteerCapable}
          wrapUpReady={queueConnected && status === "busy" && !queueSteerCapable && queueWrapUpCapable && !queuePaused && currentContextReady}
          contextReady={currentContextReady}
          ownerAuthored={ownerAuthored}
          disallowSteer={disallowAntigravitySteer}
          paused={queuePaused}
          queueCount={queuedAhead}
          queued
          onClose={(): void => setDeliveryAnchor(null)}
          onNextTurn={(): void => setDeliveryAnchor(null)}
          onQueueFront={(): void => {
            setDeliveryAnchor(null);
            void requestSendQueued(sessionId, message.id).catch((error: Error) =>
              notify(error.message),
            );
          }}
          onSteer={steerQueued}
          onWrapUp={wrapUpRow}
          onSendAndPause={(): void => setDeliveryAnchor(null)}
          onStop={(): void => {
            setDeliveryAnchor(null);
            cancelTurn(sessionId);
          }}
        />
      )}
    </>
  );
}
