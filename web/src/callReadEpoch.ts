/** Bound anchored Call reads to one attempt per ledger observation. */
export class CallReadEpoch {
  private epoch = "";
  private attempted = new Set<string>();

  claim(sessionId: string, observedAt: number | null, callId: string): boolean {
    const epoch = `${sessionId}:${String(observedAt)}`;
    if (epoch !== this.epoch) {
      this.epoch = epoch;
      this.attempted.clear();
    }
    if (this.attempted.has(callId)) return false;
    this.attempted.add(callId);
    return true;
  }
}
