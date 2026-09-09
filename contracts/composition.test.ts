import {
  CONTRACT_FINGERPRINT,
  decodeComposition,
  MAX_BYTES,
  type ScopeId,
} from "./composition.generated.ts";
import { strictJson } from "./strict-json.ts";

const fixture = JSON.parse(
  await Deno.readTextFile(
    new URL("../tests/fixtures/composition-v1.json", import.meta.url),
  ),
);
for (const vector of fixture.vectors) {
  Deno.test(`composition codec: ${vector.name}`, () => {
    const proposal = structuredClone(fixture.base);
    for (const edit of vector.edits ?? []) {
      const path = edit.path.slice(1).split("/");
      const key = path.pop();
      let parent = proposal;
      for (const part of path) parent = parent[part];
      if (edit.remove) delete parent[key];
      else parent[key] = edit.value;
    }
    let error: string | undefined;
    try {
      decodeComposition(vector.raw ?? JSON.stringify(proposal));
    } catch (caught) {
      error = (caught as Error).message;
    }
    if (
      (error === undefined) !== vector.decode ||
      !vector.decode && error !== vector.error
    ) {
      throw new Error(`codec mismatch: ${vector.name}: ${error}`);
    }
  });
}

Deno.test("generated identities cannot be substituted at compile time", () => {
  const proposal = decodeComposition(JSON.stringify(fixture.base));
  const acceptScope = (_id: ScopeId): void => {};
  acceptScope(proposal.nodes[0].scope);
  // @ts-expect-error NodeId and ScopeId are distinct, even though both are strings.
  acceptScope(proposal.nodes[0].id);
  // @ts-expect-error A raw string has not passed the codec.
  acceptScope("service");
  if (!/^sha256:[a-f0-9]{64}$/.test(CONTRACT_FINGERPRINT)) {
    throw new Error("fingerprint");
  }
});

Deno.test("strict JSON rejects byte/depth budgets and handles prototype keys as data", () => {
  for (const input of [" ".repeat(MAX_BYTES + 1), '"\ud800"', "1e400"]) {
    let failed = false;
    try {
      decodeComposition(input);
    } catch {
      failed = true;
    }
    if (!failed) throw new Error("unbounded or invalid JSON accepted");
  }
  const object = strictJson('{"__proto__":{"owned":true}}', MAX_BYTES, 32);
  if (
    Object.getPrototypeOf(object) !== null ||
    !Object.hasOwn(object as object, "__proto__")
  ) {
    throw new Error("prototype pollution");
  }
  if (strictJson('"\\ud83d\\ude80"', MAX_BYTES, 32) !== "🚀") {
    throw new Error("valid Unicode rejected");
  }
});
