import {
  checkComposition,
  type CheckedStructure,
  CompositionCheckError,
} from "./composition-check.ts";
import { decodeComposition } from "./composition.generated.ts";
import { baseInput, linkVectors } from "./composition.fixtures.ts";

for (const vector of linkVectors()) {
  Deno.test(`composition links: ${vector.name}`, async () => {
    try {
      const report = await checkComposition(vector.raw);
      if (vector.error) throw new Error(`expected ${vector.error}`);
      if (
        report.authorized !== false || report.status !== "structurally_valid"
      ) {
        throw new Error("a diagnostic cannot grant execution");
      }
    } catch (error) {
      if (
        !(error instanceof CompositionCheckError) || error.code !== vector.error
      ) {
        throw error;
      }
    }
  });
}

Deno.test("link report identity ignores declaration and JSON property order", async () => {
  const raw = baseInput();
  const original = await checkComposition(raw);
  const proposal = decodeComposition(raw);
  const reversed = await checkComposition(JSON.stringify({
    bindings: [...proposal.bindings].reverse(),
    nodes: [...proposal.nodes].reverse(),
    scopes: [...proposal.scopes].reverse(),
    format: proposal.format,
  }));
  if (JSON.stringify(original) !== JSON.stringify(reversed)) {
    throw new Error("declaration order changed a link report");
  }
  const reverseKeys = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(reverseKeys);
    if (value !== null && typeof value === "object") {
      return Object.fromEntries(
        Object.entries(value).reverse().map(
          ([key, item]) => [key, reverseKeys(item)],
        ),
      );
    }
    return value;
  };
  const reordered = await checkComposition(
    JSON.stringify(reverseKeys(proposal)),
  );
  if (JSON.stringify(original) !== JSON.stringify(reordered)) {
    throw new Error("nested property order changed a link report");
  }
  if (original.dependency_order.join() !== "victoria,client") {
    throw new Error("provider must precede consumer");
  }
  const changed = await checkComposition(
    baseInput([{ path: "/bindings/0/revision", value: "2" }]),
  );
  if (original.proposal_digest === changed.proposal_digest) {
    throw new Error("binding revision must change identity");
  }
});

Deno.test("checked reports are frozen diagnostics, not decodable grants", async () => {
  const report = await checkComposition(baseInput());
  for (
    const value of [
      report,
      report.sites,
      report.sites[0],
      report.sites[0].site,
      report.dependency_order,
      report.remote_bindings[0],
      report.remote_bindings[0].consumer,
    ]
  ) {
    if (!Object.isFrozen(value)) throw new Error("mutable checked projection");
  }
  try {
    await checkComposition(JSON.stringify(report));
    throw new Error("accepted a serialized checked report as input");
  } catch (error) {
    if (
      !(error instanceof CompositionCheckError) ||
      error.code !== "invalid_contract"
    ) throw error;
  }
  // Type-only assertions are deliberately not executed against frozen values.
  const types = (
    report: CheckedStructure,
    proposal: ReturnType<typeof decodeComposition>,
  ): void => {
    // @ts-expect-error A decoded proposal has not passed graph linking.
    const checked: CheckedStructure = proposal;
    // @ts-expect-error A copied wire projection lacks the private check brand.
    const fabricated: CheckedStructure = {
      status: "structurally_valid",
      authorized: false,
      contract_fingerprint: report.contract_fingerprint,
      proposal_digest: "x",
      dependency_order: [],
      reverse_dependency_order: [],
      sites: [],
      remote_bindings: [],
    };
    // @ts-expect-error Checked reports cannot grant authority.
    report.authorized = true;
    // @ts-expect-error The graph projection is recursively readonly.
    report.remote_bindings[0].consumer.node = proposal.nodes[0].id;
    void checked;
    void fabricated;
  };
  void types;
});

Deno.test("link errors contain closed codes only", async () => {
  try {
    await checkComposition('{"secret":"synthetic-private-token"}');
    throw new Error("invalid input accepted");
  } catch (error) {
    if (
      !(error instanceof CompositionCheckError) ||
      error.message !== "invalid_contract"
    ) throw error;
    if (Object.hasOwn(error, "cause")) {
      throw new Error("native exception leaked");
    }
  }
});
