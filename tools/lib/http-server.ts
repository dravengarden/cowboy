// A loopback fixture server for the browser conformance harnesses: one request
// handler, and WebSockets the handler accepts from inside that handler. It
// keeps the harnesses' per-connection closures over Bun.serve, whose WebSocket
// callbacks are otherwise registered once for the whole server.

/** The server side of an accepted WebSocket. */
export class FixtureSocket {
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void | Promise<void>) | null = null;
  onclose: (() => void) | null = null;
  #peer: Bun.ServerWebSocket<FixtureSocket> | undefined;
  #closed = false;

  get readyState(): number {
    if (this.#closed) return WebSocket.CLOSED;
    return this.#peer === undefined ? WebSocket.CONNECTING : WebSocket.OPEN;
  }

  send(data: string): void {
    this.#peer?.send(data);
  }

  close(code?: number, reason?: string): void {
    this.#peer?.close(code, reason);
  }

  /** @internal */
  opened(peer: Bun.ServerWebSocket<FixtureSocket>): void {
    this.#peer = peer;
    this.onopen?.();
  }

  /** @internal */
  ended(): void {
    this.#closed = true;
    this.onclose?.();
  }
}

/** Returned by `accept`; the handler returns it to finish the upgrade. */
const UPGRADED = new Response(null);

export interface FixtureServer {
  readonly port: number;
  /** Stop listening and close every open connection. */
  shutdown(): Promise<void>;
}

export interface FixtureRequest {
  readonly request: Request;
  /** Accept this request as a WebSocket, echoing `protocol` when given. */
  accept(options?: { protocol?: string }): {
    socket: FixtureSocket;
    response: Response;
  };
}

/** Serve `handler` on an unused loopback port. */
export function serveFixture(
  handler: (context: FixtureRequest) => Response | Promise<Response>,
): FixtureServer {
  const server = Bun.serve<FixtureSocket>({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(request, owner) {
      const response = await handler({
        request,
        accept(options = {}) {
          const socket = new FixtureSocket();
          const upgraded = owner.upgrade(request, {
            data: socket,
            ...(options.protocol === undefined ? {} : {
              headers: { "Sec-WebSocket-Protocol": options.protocol },
            }),
          });
          if (!upgraded) throw new Error("request is not a WebSocket upgrade");
          return { socket, response: UPGRADED };
        },
      });
      // Bun completes an upgrade by returning nothing from fetch.
      return response === UPGRADED ? undefined : response;
    },
    websocket: {
      open: (peer) => peer.data.opened(peer),
      message: (peer, message) =>
        void peer.data.onmessage?.({
          data: typeof message === "string"
            ? message
            : new TextDecoder().decode(message),
        }),
      close: (peer) => peer.data.ended(),
    },
  });
  return {
    port: server.port ?? 0,
    shutdown: () => server.stop(true),
  };
}
