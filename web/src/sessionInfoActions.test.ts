import { assertEquals } from "jsr:@std/assert";

const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);

Deno.test("session settings exposes confirmed compact and clear actions", () => {
  assertEquals(
    composerSource.includes('aria-label="compact conversation from session settings"'),
    true,
  );
  assertEquals(
    composerSource.includes('aria-label="clear conversation from session settings"'),
    true,
  );
  assertEquals(
    composerSource.includes("onSessionAction={setCmdConfirm}"),
    true,
  );
  assertEquals(
    composerSource.includes('disabled={dead || compacting}'),
    true,
  );
  assertEquals(
    composerSource.includes('color={action.destructive ? "error" : "primary"}'),
    true,
  );
});
