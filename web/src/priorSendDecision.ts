export interface PriorSendDecision {
  readonly id: number;
  readonly sessionId: string;
  readonly ids: readonly string[];
  readonly resolve: () => void;
  readonly reject: (error: Error) => void;
}
const listeners = new Set<() => void>();
let requests: PriorSendDecision[] = [];
let nextRequestId = 0;
export const currentPriorSendDecision = (): PriorSendDecision | null =>
  requests[0] ?? null;
export function subscribePriorSendDecision(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export function requestPriorSendDecision(
  sessionId: string,
  ids: readonly string[],
): Promise<void> {
  if (listeners.size === 0) {
    return Promise.reject(
      new Error("Review the earlier message before sending."),
    );
  }
  return new Promise((resolve, reject) => {
    requests = [...requests, {
      id: ++nextRequestId,
      sessionId,
      ids,
      resolve,
      reject,
    }];
    listeners.forEach((listener) => listener());
  });
}

export function cancelPriorSendDecisions(): void {
  for (const request of requests) finishPriorSendDecision(request, false);
}
export function finishPriorSendDecision(
  request: PriorSendDecision,
  proceed: boolean,
): void {
  if (!requests.includes(request)) return;
  requests = requests.filter((item) => item !== request);
  if (proceed) request.resolve();
  else {request.reject(
      new DOMException("New message kept in composer", "AbortError"),
    );}
  listeners.forEach((listener) => listener());
}
