// Run in the pinned Nix shell and a private loopback network namespace.
// Arguments: pinned Firefox executable, one or more built fixture directories.
const [browser, ...args] = Deno.args;
const safety = args.includes("--safety");
const bundles = args.filter((argument) => argument !== "--safety");
if (
  !browser?.startsWith("/nix/store/") || !browser.endsWith("/bin/firefox") ||
  !bundles.length
) {
  throw new Error(
    "pass .#cowboy-idb-test-browser /bin/firefox and fixture directories",
  );
}
const delay = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));
const descriptor = {
  schema: "dravengarden.cowboy.product-sync-dataset/v1",
  dataset_id: `dataset-${"a".repeat(64)}`,
  user_id: "fixture-user",
  database_version: 2,
  outbox_contract: "atomic-delta-v1",
};
const session = {
  id: "fixture-session",
  provider: "codex",
  machine_id: "fixture-machine",
  cwd: "/synthetic",
  title: "Synthetic latency fixture",
  status: "running",
  origin: "web",
  created_at_ms: 0,
  updated_at_ms: 0,
};
const cases = bundles.flatMap((bundle) =>
  (safety ? ["changed-dataset", "missing-protocol"] : ["timing"])
    .map((scenario) => ({ bundle, scenario }))
);
for (const { bundle, scenario } of cases) {
  const temporary = await Deno.makeTempDir({ prefix: "cowboy-send-latency-" });
  let child: Deno.ChildProcess | undefined;
  let deadline: ReturnType<typeof setTimeout> | undefined;
  const sockets = new Set<WebSocket>();
  const report = Promise.withResolvers<unknown>();
  let discoveries = 0;
  let deliveries = 0;
  let seq = 0;
  let attempts = 0;
  let datasetId = descriptor.dataset_id;
  const seen = new Set<string>();
  const history: Record<string, unknown>[] = [];
  const script = await Deno.readTextFile(`${bundle}/fixture.js`);
  const server = Deno.serve(
    { hostname: "127.0.0.1", port: 0, onListen() {} },
    async (request) => {
      const url = new URL(request.url);
      if (url.pathname === "/fixture.js") {
        return new Response(script, {
          headers: { "Content-Type": "text/javascript" },
        });
      }
      if (url.pathname === "/report") {
        report.resolve(await request.json());
        return new Response("ok");
      }
      if (
        request.method === "POST" && url.pathname === "/fixture/switch-dataset"
      ) {
        datasetId = `dataset-${"b".repeat(64)}`;
        return new Response("ok");
      }
      if (
        request.method === "POST" && url.pathname === "/fixture/restore-dataset"
      ) {
        datasetId = descriptor.dataset_id;
        return new Response("ok");
      }
      if (url.pathname === "/api/sync/dataset") {
        discoveries++;
        await delay(120);
        return Response.json({ ...descriptor, dataset_id: datasetId });
      }
      if (url.pathname === "/ws") {
        attempts++;
        await delay(120);
        if (url.searchParams.get("dataset") !== datasetId) {
          return new Response("mismatch", { status: 409 });
        }
        const { socket, response } = Deno.upgradeWebSocket(
          request,
          scenario === "missing-protocol" ? {} : { protocol: "cowboy-sync-v1" },
        );
        sockets.add(socket);
        socket.onopen = () => {
          socket.send(
            JSON.stringify({ type: "sessions", sessions: [session] }),
          );
          socket.send(JSON.stringify({ type: "bootstrap_complete" }));
        };
        socket.onmessage = async (event) => {
          const message = JSON.parse(event.data);
          if (message.type !== "submit") {
            return;
          }
          if (seen.has(message.cmid)) return;
          seen.add(message.cmid);
          deliveries++;
          await delay(40);
          const echo = {
            session_id: session.id,
            seq: ++seq,
            cmid: message.cmid,
            kind: "update",
            update: {
              sessionUpdate: "user_message_chunk",
              content: { type: "text", text: message.text },
            },
          };
          const end = {
            session_id: session.id,
            seq: ++seq,
            kind: "turn_end",
            stop_reason: "EndTurn",
          };
          history.push(echo, end);
          socket.send(JSON.stringify({ type: "event", envelope: echo }));
          socket.send(JSON.stringify({ type: "event", envelope: end }));
        };
        socket.onclose = () => sockets.delete(socket);
        return response;
      }
      if (url.pathname.endsWith("/bootstrap")) {
        return Response.json({
          messages: [{
            type: "snapshot",
            session_id: session.id,
            events: history,
            reached_start: true,
          }],
        });
      }
      if (url.pathname === "/") {
        return new Response(
          `<!doctype html><div id="root"></div><script type="module">
import { run } from '/fixture.js';
let result; try { result = { ok: true, samples: await run() }; }
catch (error) { result = { ok: false, error: String(error) }; }
await fetch('/report', { method: 'POST', body: JSON.stringify(result) });
</script>`,
          { headers: { "Content-Type": "text/html" } },
        );
      }
      return new Response("fixture endpoint unavailable", { status: 404 });
    },
  );
  try {
    const profile = `${temporary}/profile`;
    await Deno.mkdir(profile);
    child = new Deno.Command(browser, {
      args: [
        "--headless",
        "--no-remote",
        "--new-instance",
        "--profile",
        profile,
        `http://127.0.0.1:${server.addr.port}/?scenario=${scenario}`,
      ],
      clearEnv: true,
      env: {
        MOZ_HEADLESS: "1",
        MOZ_CRASHREPORTER_DISABLE: "1",
        MOZ_NO_REMOTE: "1",
        XDG_RUNTIME_DIR: temporary,
        XDG_CACHE_HOME: `${temporary}/cache`,
        XDG_CONFIG_HOME: `${temporary}/config`,
        XDG_DATA_HOME: `${temporary}/data`,
      },
      stdout: "null",
      stderr: "null",
    }).spawn();
    deadline = setTimeout(
      () => report.reject(new Error("browser deadline exceeded")),
      45_000,
    );
    const result = await report.promise;
    const digest = Array.from(
      new Uint8Array(
        await crypto.subtle.digest("SHA-256", new TextEncoder().encode(script)),
      ),
    )
      .map((byte) => byte.toString(16).padStart(2, "0")).join("");
    console.log(
      JSON.stringify({
        bundle,
        digest,
        scenario,
        discoveries,
        attempts,
        deliveries,
        result,
      }),
    );
    if (
      !(result as { ok?: boolean }).ok || deliveries !== (safety ? 0 : 17) ||
      attempts < 1
    ) {
      throw new Error("send fixture failed");
    }
  } finally {
    clearTimeout(deadline);
    for (const socket of sockets) socket.close();
    try {
      child?.kill("SIGKILL");
    } catch { /* The browser may have exited. */ }
    await child?.status;
    await server.shutdown();
    await Deno.remove(temporary, { recursive: true });
  }
}
