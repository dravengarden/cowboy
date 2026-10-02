// Run in the pinned Nix shell and a private loopback network namespace.
// Arguments: pinned Firefox executable, one or more built fixture directories.
const [browser, ...args] = Deno.args;
const chromium = browser?.endsWith("/bin/chromium");
const queued = args.includes("--queued");
const metadata = args.includes("--metadata");
const local = args.includes("--local") || metadata || queued;
const safety = args.includes("--safety");
const recovery = args.includes("--transcript-recovery");
const slowMobile = args.includes("--slow-mobile");
const draftSend = args.includes("--draft-send");
const bundles = args.filter((argument) =>
  argument !== "--queued" && argument !== "--metadata" &&
  argument !== "--local" &&
  argument !== "--safety" && argument !== "--transcript-recovery" &&
  argument !== "--slow-mobile" &&
  argument !== "--draft-send"
);
if (
  !browser?.startsWith("/nix/store/") ||
  (!browser.endsWith("/bin/firefox") && !chromium) ||
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
  (slowMobile
    ? ["slow-mobile", "lost-send"]
    : draftSend
    ? ["draft-send"]
    : metadata
    ? ["metadata"]
    : queued
    ? ["queued"]
    : local
    ? ["local"]
    : recovery
    ? ["transcript-recovery"]
    : safety
    ? ["changed-dataset", "missing-protocol"]
    : ["timing"])
    .map((scenario) => ({ bundle, scenario }))
);
for (const { bundle, scenario } of cases) {
  const temporary = await Deno.makeTempDir({ prefix: "cowboy-send-latency-" });
  let child: Deno.ChildProcess | undefined;
  let deadline: ReturnType<typeof setTimeout> | undefined;
  const sockets = new Set<WebSocket>();
  const report = Promise.withResolvers<unknown>();
  let bootstraps = 0;
  let discoveries = 0;
  let deliveries = 0;
  let metadataMutations = 0;
  let seq = 0;
  let attempts = 0;
  let lostSend = false;
  let recoveredTransport = false;
  let datasetId = descriptor.dataset_id;
  const seen = new Set<string>();
  const history: Record<string, unknown>[] = [];
  const image = { type: "image", mimeType: "image/png", data: "c2hvdA==" };
  const draft = {
    id: "fixture-draft",
    text: "image draft caption",
    cmid: "draft-creation",
    content: [image, { type: "text", text: "image draft caption" }, image],
  };
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
      if (local && /^\/[a-zA-Z0-9_-]+\.js$/.test(url.pathname)) {
        try {
          return new Response(
            await Deno.readTextFile(`${bundle}${url.pathname}`),
            {
              headers: { "Content-Type": "text/javascript" },
            },
          );
        } catch {
          return new Response("missing fixture chunk", { status: 404 });
        }
      }
      if (url.pathname === "/fixture/metrics") {
        return Response.json({ deliveries, metadataMutations });
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
            JSON.stringify({
              type: "sessions",
              sessions: metadata
                ? [session, {
                  ...session,
                  id: "fixture-other",
                  title: "Other fixture",
                }]
                : [{
                  ...session,
                  status: queued ? "starting" : session.status,
                }],
            }),
          );
          socket.send(JSON.stringify({ type: "bootstrap_complete" }));
        };
        socket.onmessage = async (event) => {
          const message = JSON.parse(event.data);
          if (message.type === "connection_probe") {
            if (scenario === "lost-send" && lostSend && deliveries === 0) {
              recoveredTransport = true;
              socket.close();
              return;
            }
            socket.send(
              JSON.stringify({
                type: "connection_probe",
                nonce: message.nonce,
              }),
            );
            return;
          }
          if (draftSend && message.type === "activate_draft") {
            if (
              typeof message.cmid !== "string" || message.cmid === draft.cmid
            ) {
              report.resolve({
                ok: false,
                error: "draft send omitted its operation identity",
              });
              return;
            }
            deliveries++;
            // The source disappears before its echo. It must not remove the
            // image preview, nor leave Sending after the full echo arrives.
            socket.send(
              JSON.stringify({
                type: "sync_patch",
                state: `queue:${session.id}`,
                version: 2,
                value: { queue: [], drafts: [] },
                confirmed: [],
              }),
            );
            await delay(100);
            for (const [index, content] of draft.content.entries()) {
              const echo = {
                session_id: session.id,
                seq: ++seq,
                kind: "update",
                ...(index === 0 ? { cmid: message.cmid } : {}),
                update: { sessionUpdate: "user_message_chunk", content },
              };
              history.push(echo);
              socket.send(JSON.stringify({ type: "event", envelope: echo }));
              await delay(30);
            }
            return;
          }
          if (message.type === "sync") metadataMutations++;
          if (message.type !== "submit") {
            return;
          }
          if (scenario === "lost-send" && !recoveredTransport) {
            lostSend = true;
            return;
          }
          if (seen.has(message.cmid)) return;
          seen.add(message.cmid);
          deliveries++;
          if (local) {
            socket.send(JSON.stringify({
              type: "sync_patch",
              state: `queue:${session.id}`,
              version: deliveries,
              value: { queue: [], drafts: [] },
              confirmed: [message.cmid],
            }));
          }
          await delay(scenario === "slow-mobile" ? 33_000 : local ? 800 : 40);
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
        if (scenario === "slow-mobile") {
          bootstraps++;
          await delay(12_000);
        }
        if (recovery) {
          bootstraps++;
          await delay(150);
          if (bootstraps <= 2) {
            return new Response("temporary failure", { status: 503 });
          }
          if (bootstraps === 3) return Response.json({ messages: [] });
          history.splice(0, history.length, {
            session_id: session.id,
            seq: 1,
            kind: "update",
            update: {
              sessionUpdate: "agent_message_chunk",
              content: { type: "text", text: "fresh complete answer" },
            },
          });
        }
        return Response.json({
          messages: [
            {
              type: "snapshot",
              session_id: session.id,
              events: history,
              reached_start: true,
            },
            ...(draftSend
              ? [{
                type: "sync_patch",
                state: `queue:${session.id}`,
                version: 1,
                value: { queue: [], drafts: deliveries ? [] : [draft] },
                confirmed: [draft.cmid],
              }]
              : []),
          ],
        });
      }
      if (url.pathname === "/") {
        return new Response(
          `<!doctype html><div id="root"></div><script type="module">
let result; try { const { run } = await import('/fixture.js'); result = { ok: true, samples: await run() }; }
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
      args: chromium
        ? [
          "--headless",
          "--no-sandbox",
          "--disable-gpu",
          "--no-first-run",
          "--no-default-browser-check",
          "--disable-background-networking",
          "--disable-component-update",
          "--remote-debugging-port=0",
          `--user-data-dir=${profile}`,
          `http://127.0.0.1:${server.addr.port}/?scenario=${scenario}`,
        ]
        : [
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
    void child.status.then((status) => {
      if (status.code !== 0) {
        report.reject(
          new Error(
            `browser exited: ${status.code} ${
              JSON.stringify({ discoveries, attempts, deliveries })
            }`,
          ),
        );
      }
    });
    deadline = setTimeout(
      () =>
        report.reject(
          new Error(
            `browser deadline exceeded: ${
              JSON.stringify({ discoveries, attempts, deliveries })
            }`,
          ),
        ),
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
        bootstraps,
        attempts,
        deliveries,
        result,
      }),
    );
    if (
      !(result as { ok?: boolean }).ok ||
      deliveries !==
        (metadata
          ? 0
          : local || draftSend || slowMobile
          ? 1
          : safety || recovery
          ? 0
          : 17) ||
      attempts < 1
    ) {
      throw new Error("send fixture failed");
    }
    if (scenario === "lost-send" && attempts !== 2) {
      throw new Error(
        "lost send must recover through exactly one replacement socket",
      );
    }
  } finally {
    clearTimeout(deadline);
    for (const socket of sockets) socket.close();
    if (chromium && child) {
      try {
        const [port, path] =
          (await Deno.readTextFile(`${temporary}/profile/DevToolsActivePort`))
            .trim().split("\n");
        const debuggerSocket = new WebSocket(`ws://127.0.0.1:${port}${path}`);
        await new Promise<void>((resolve) => {
          const timeout = setTimeout(() => {
            debuggerSocket.close();
            resolve();
          }, 2000);
          debuggerSocket.onopen = () =>
            debuggerSocket.send(
              JSON.stringify({ id: 1, method: "Browser.close" }),
            );
          debuggerSocket.onclose = debuggerSocket.onerror = () => {
            clearTimeout(timeout);
            resolve();
          };
        });
        await Promise.race([child.status, delay(2000)]);
      } catch { /* An already-exited browser needs no graceful close. */ }
    }
    try {
      child?.kill("SIGKILL");
    } catch { /* The browser may have exited. */ }
    await child?.status;
    await server.shutdown();
    await Deno.remove(temporary, { recursive: true });
  }
}
