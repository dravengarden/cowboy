import { assertEquals } from "jsr:@std/assert";

const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);

Deno.test("session settings exposes confirmed compact and clear actions", () => {
  assertEquals(
    composerSource.includes(
      'aria-label="compact conversation from session settings"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      'aria-label="clear conversation from session settings"',
    ),
    true,
  );
  assertEquals(
    composerSource.includes("onSessionAction={setCmdConfirm}"),
    true,
  );
  assertEquals(composerSource.includes("event.currentTarget.blur();"), true);
  assertEquals(
    composerSource.includes("disabled={dead || compacting}"),
    true,
  );
  assertEquals(
    composerSource.includes('color={action.destructive ? "error" : "primary"}'),
    true,
  );
});

Deno.test("session actions default collapsed and expand reload, compact, and clear rows", () => {
  assertEquals(
    composerSource.includes(
      "const [sessionActionsExpanded, setSessionActionsExpanded] = useState(false);",
    ),
    true,
  );
  assertEquals(
    composerSource.includes("setSessionActionsExpanded(false);"),
    true,
  );
  assertEquals(
    /aria-label=\{actionsExpanded\s*\?\s*"Collapse session actions"\s*:\s*"Expand session actions"\}/
      .test(composerSource),
    true,
  );
  assertEquals(
    composerSource.includes(
      "<Collapse id={actionsPanelId} in={actionsExpanded} unmountOnExit>",
    ),
    true,
  );

  const disclosureStart = composerSource.indexOf(
    "aria-label={actionsExpanded",
  );
  const disclosureEnd = composerSource.indexOf("</Collapse>", disclosureStart);
  const disclosureSource = composerSource.slice(disclosureStart, disclosureEnd);
  assertEquals(disclosureStart >= 0 && disclosureEnd > disclosureStart, true);
  assertEquals(disclosureSource.includes('direction="row"'), false);
  assertEquals(disclosureSource.match(/\bfullWidth\b/g)?.length, 3);
});
