const tests = [];
const measurements = {};
const rejectionEvidence = {};
const encoder = new TextEncoder();
const decoder = new TextDecoder();
const bytes = (s) => Array.from(encoder.encode(s));
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const assert = (condition, message) => {
  if (!condition) throw new Error(message);
};
let worker;
let next = 0;
const pending = new Map();
function rpc(op, args = {}) {
  return new Promise((resolve, reject) => {
    const id = ++next;
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`RPC timed out: ${op}`));
    }, 10000);
    pending.set(id, { resolve, reject, timer });
    worker.postMessage({ id, op, args });
  });
}
async function api(path, body) {
  const response = await fetch(
    `/fixture/${path}`,
    body === undefined ? {} : {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    },
  );
  assert(response.ok, `fixture HTTP ${path}: ${response.status}`);
  return response.json();
}
async function waitFor(predicate, label, timeout = 5000) {
  const deadline = performance.now() + timeout;
  do {
    const result = await predicate();
    if (result) return result;
    await sleep(30);
  } while (performance.now() < deadline);
  throw new Error(
    `Timed out: ${label}; client=${
      JSON.stringify(await rpc("stats"))
    }; server=${JSON.stringify(await api("stats"))}`,
  );
}
async function pass(name) {
  tests.push(name);
  document.querySelector("#status").textContent = tests.join("\n");
  await api("progress", { phase: name, passed: tests.length });
}
async function receive(expected, timeout = 5000) {
  return await waitFor(
    async () => {
      const values = await rpc("inbox");
      if (!values.length) return false;
      assert(
        values.length === 1,
        `Unexpected duplicate payload: ${values.length}`,
      );
      assert(
        decoder.decode(new Uint8Array(values[0])) === expected,
        "Payload changed during encrypted round trip",
      );
      return true;
    },
    `receive ${expected.slice(0, 40)}`,
    timeout,
  );
}
async function roundTrip(label, timeout = 5000) {
  await rpc("send", { payload: bytes(label) });
  await receive(label, timeout);
}

try {
  assert(
    isSecureContext && location.protocol === "https:",
    "TLS secure context required",
  );
  const ready = new Promise((resolve, reject) => {
    worker = new Worker("./worker.js", { type: "module" });
    worker.onerror = (event) => reject(new Error(event.message));
    worker.onmessage = ({ data }) => {
      if (data.ready) return resolve(data);
      const call = pending.get(data.id);
      if (!call) return;
      clearTimeout(call.timer);
      pending.delete(data.id);
      data.ok ? call.resolve(data.result) : call.reject(new Error(data.error));
    };
  });
  const startup = await ready;
  assert(startup.worker, "WireGuard is not running in a worker");
  measurements.wasmWorkerStartupMs = startup.startupMs;
  const config = await api("config");
  const url = `${location.origin.replace("https:", "wss:")}/wireguard`;
  const identity = await rpc("create", { key: config.server_public });
  await api("peer", { public_key: identity.publicKey });
  await rpc("connect", { url });
  const first = performance.now();
  await roundTrip("WGPROBE_PAYLOAD_SECRET_initial");
  measurements.firstHandshakeAndRoundTripMs = performance.now() - first;
  assert(
    (await rpc("stats")).handshakes_received > 0,
    "No validated handshake response",
  );
  await pass(
    "WASM worker uses browser CSPRNG and completes WireGuard handshake over verified WSS",
  );
  await pass(
    "Valid IPv4/UDP payload round trip through browser and native Rust engines",
  );

  await api("push", bytes("server-initiated-message"));
  await receive("server-initiated-message");
  await pass("Server-initiated encrypted payload reaches the browser");

  let before = (await api("stats")).peer;
  await rpc("replay");
  await waitFor(
    async () => (await api("stats")).peer.rejected > before.rejected,
    "server rejects replay",
  );
  assert(
    (await api("stats")).peer.accepted === before.accepted,
    "Server delivered replay",
  );
  assert(!(await rpc("inbox")).length, "Replay generated application response");
  await roundTrip("after-client-replay");
  await pass(
    "Replayed client ciphertext rejected; subsequent fresh traffic succeeds",
  );

  const clientBefore = await rpc("stats");
  await api("inject-last", {});
  await waitFor(
    async () => (await rpc("stats")).rejected > clientBefore.rejected,
    "browser rejects replay",
  );
  assert(!(await rpc("inbox")).length, "Browser delivered replay");
  rejectionEvidence.serverReplay = await rpc("stats");
  await roundTrip("after-server-replay");
  await pass(
    "Replayed server ciphertext rejected in WASM; subsequent fresh traffic succeeds",
  );

  before = (await api("stats")).peer;
  await rpc("corrupt", { payload: bytes("tampered-tag-probe") });
  await waitFor(
    async () => (await api("stats")).peer.rejected > before.rejected,
    "AEAD rejects tampered tag",
  );
  assert(
    (await api("stats")).peer.accepted === before.accepted,
    "Tampered packet delivered",
  );
  await rpc("repair");
  await receive("tampered-tag-probe");
  await pass(
    "Modified authentication tag rejected; original packet with same counter still accepted",
  );

  before = (await api("stats")).peer;
  await rpc("send", { payload: bytes("forged-source"), forged: true });
  await waitFor(
    async () =>
      (await api("stats")).peer.address_rejected > before.address_rejected,
    "inner source allowlist",
  );
  assert(
    (await api("stats")).peer.accepted === before.accepted,
    "Forged source accepted",
  );
  await roundTrip("after-forged-source");
  await pass(
    "Authenticated packet with unauthorized inner source rejected by peer address policy",
  );

  await rpc("pause");
  await sleep(250);
  await rpc("resume", { url });
  await roundTrip("reconnected-with-same-device-key");
  await pass(
    "WSS reconnect and explicit resume reset establish a fresh working WireGuard session",
  );

  await api("peer", { public_key: null });
  const revokedBefore = (await api("stats")).revoked_packets;
  await rpc("send", { payload: bytes("must-not-arrive-after-revocation") });
  await waitFor(
    async () => (await api("stats")).revoked_packets > revokedBefore,
    "revoked traffic reached server and was rejected",
  );
  assert(
    !(await rpc("inbox")).length,
    "Revoked peer received application response",
  );
  await api("peer", { public_key: identity.publicKey });
  await rpc("reset");
  await roundTrip("explicit-reauthorization-restores-access");
  await pass(
    "Removing peer invalidates established traffic; explicit reauthorization recovers",
  );

  const wrongKey = new Array(32).fill(0);
  wrongKey[0] = 9;
  await api("peer", { public_key: wrongKey });
  await rpc("reset");
  await rpc("send", { payload: bytes("wrong-device-public-key") });
  await waitFor(
    async () => (await api("stats")).peer.rejected > 0,
    "wrong registered device key",
  );
  assert(
    (await api("stats")).peer.accepted === 0,
    "Unregistered device accepted",
  );
  await api("peer", { public_key: identity.publicKey });
  await rpc("reset");
  await roundTrip("correct-device-public-key");
  await pass("Server enforces configured device public key");

  const wrongIdentity = await rpc("create", { key: wrongKey });
  await api("peer", { public_key: wrongIdentity.publicKey });
  await rpc("connect", { url });
  await rpc("send", { payload: bytes("wrong-server-public-key") });
  await waitFor(
    async () => (await api("stats")).peer.rejected > 0,
    "wrong pinned server key",
  );
  assert(
    (await rpc("stats")).handshakes_received === 0,
    "Wrong server pin established session",
  );
  assert(!(await rpc("inbox")).length, "Wrong server pin delivered payload");
  rejectionEvidence.wrongServerPin = (await api("stats")).peer;
  const correctIdentity = await rpc("create", { key: config.server_public });
  assert(
    JSON.stringify(correctIdentity.publicKey) !==
      JSON.stringify(identity.publicKey),
    "Fresh browser identity reused a key",
  );
  await api("peer", { public_key: correctIdentity.publicKey });
  await rpc("connect", { url });
  await roundTrip("correct-server-pin");
  await pass(
    "Browser enforces server public-key pin; correct pin restores communication",
  );

  await rpc("reset");
  await rpc("dropHandshake");
  const retryStarted = performance.now();
  await roundTrip("handshake-retry-after-loss", 9000);
  measurements.droppedHandshakeRecoveryMs = performance.now() - retryStarted;
  assert(
    (await rpc("stats")).dropped === 1,
    "Handshake was not actually dropped",
  );
  await pass(
    "Real WireGuard timer retransmits a deliberately dropped handshake",
  );

  const rtt = [];
  for (let i = 0; i < 20; i++) {
    const start = performance.now();
    await roundTrip(`latency-${i}`);
    rtt.push(performance.now() - start);
  }
  rtt.sort((a, b) => a - b);
  measurements.fixtureRoundTripMedianMs = rtt[10];
  const bulkStart = performance.now();
  const payload = "x".repeat(1000);
  for (let i = 0; i < 64; i++) await roundTrip(`${i}:${payload}`);
  measurements.sequentialPayloadBytes = Array.from(
    { length: 64 },
    (_, i) => encoder.encode(`${i}:${payload}`).length,
  ).reduce((a, b) => a + b, 0);
  measurements.sequentialPayloadRoundTripsMs = performance.now() - bulkStart;
  await pass(
    "Sixty-four 1 KB payloads survive encrypted transport without corruption",
  );

  const initial = await rpc("stats");
  const rekeyStarted = performance.now();
  await api("progress", {
    phase: "Waiting for real default 120-second WireGuard rekey timer",
    passed: tests.length,
  });
  while (performance.now() - rekeyStarted < 145000) {
    await roundTrip(`rekey-${Math.floor(performance.now())}`);
    const current = await rpc("stats");
    if (
      current.handshakes_received > initial.handshakes_received &&
      current.lastReceiver !== initial.lastReceiver
    ) break;
    await sleep(1000);
  }
  const rekeyed = await rpc("stats");
  assert(
    rekeyed.handshakes_received > initial.handshakes_received,
    "Default rekey did not complete",
  );
  assert(
    rekeyed.lastReceiver !== initial.lastReceiver,
    "Traffic did not switch to new session index",
  );
  measurements.rekeyObservationMs = performance.now() - rekeyStarted;
  await roundTrip("WGPROBE_PAYLOAD_SECRET_after-rekey");
  await pass(
    "Unmodified default rekey timer establishes a new session while payloads keep flowing",
  );
  assert(
    !(await rpc("stats")).leak,
    "Application marker visible in WireGuard carrier packets",
  );
  await pass(
    "WSS binary messages carry WireGuard ciphertext without application plaintext marker",
  );

  await api("report", {
    ok: true,
    tests,
    measurements,
    rejectionEvidence,
    client: await rpc("stats"),
    server: await api("stats"),
    userAgent: navigator.userAgent,
  });
} catch (error) {
  await api("report", {
    ok: false,
    tests,
    measurements,
    rejectionEvidence,
    error: String(error),
    stack: error.stack,
  });
} finally {
  worker?.terminate();
}
