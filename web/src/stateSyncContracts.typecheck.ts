// Production tsc includes these negative contracts; Deno's no-check test runner
// is not evidence of compile-time safety. Never executed or bundled.
import { createOwnedResourceScope } from "@cowboy/state-store/scope";
import { replicatedStore } from "@cowboy/state-sync";

export function verifySyncLifecycleTypes(): void {
  const scope = createOwnedResourceScope();
  // @ts-expect-error cleanup has no arbitrary result/data payload
  scope.defer(() => 42);
  // @ts-expect-error a task's result stays typed across admission
  const wrong: Promise<number> = scope.run(() => Promise.resolve("value"));
  void wrong;
  // @ts-expect-error lifecycle is an observation, not a writable authority
  scope.snapshot().phase = "active";
  const store = replicatedStore({
    initial: 0,
    clientId: "typed",
    send: () => {},
    mutators: {
      add: (value: number, args: { amount: number }) => value + args.amount,
    },
  });
  // @ts-expect-error mutation name and argument schema stay correlated
  store.mutateDurably("add", { amount: "two" });
  // @ts-expect-error consumers cannot revive a sealed instance
  store.lifecycle.phase = "active";
}
