import {
  type BoolExpression,
  type EffectCapability,
  type EffectSchema,
  evaluateExpression,
  initialProviderState,
  type ProviderHostContext,
  type ProviderState,
  type ProviderUiManifest,
  type SurfaceSlot,
  transitionProvider,
  type UiNode,
  validateProviderUiManifest,
} from "@cowboy/provider-ui";
import { createOwnedResourceScope } from "@cowboy/state-store/scope";

type ButtonNode = Extract<UiNode, { component: "button" }>;
/** Fences local observations only, never cancels an admitted remote operation. */
export interface ProviderUiObservation {
  readonly active: boolean;
}
export type ProviderUiEffectHandler = (
  effect: EffectSchema,
  observation: ProviderUiObservation,
) => Promise<void>;
export interface ProviderUiContext {
  readonly host: ProviderHostContext;
  readonly onEffect?: ProviderUiEffectHandler;
  readonly blockedCapabilities?: ReadonlySet<EffectCapability> | undefined;
}
export interface ProviderUiSnapshot {
  readonly state: ProviderState;
  readonly busyEffect: string | null;
  readonly problem: "unsupported_effect_profile" | null;
}
interface Action {
  readonly visibility: readonly BoolExpression[];
  readonly effect: EffectSchema | undefined;
  readonly supported: boolean;
}

/** Resolve ALL reducers, including a pure reducer preceding the effect rule.
 * The independent IR validator forbids two effect-bearing rules per message.
 */
function messageEffect(
  manifest: ProviderUiManifest,
  message: string,
): EffectSchema | undefined {
  const effectId = manifest.logic.reducers.find((rule) =>
    rule.message === message && rule.effect !== undefined
  )?.effect;
  return manifest.logic.effects.find((effect) => effect.id === effectId);
}

function supportedEffect(
  manifest: ProviderUiManifest,
  effect: EffectSchema,
): boolean {
  const success = manifest.logic.messages.find((message) =>
    message.id === effect.success_message
  );
  const failure = manifest.logic.messages.find((message) =>
    message.id === effect.failure_message
  );
  // This core executor supplies no Plugin-authored host arguments. Completion
  // acknowledges only the host callback, not remote business-state convergence.
  // Do not silently ignore request fields or another completion-triggered effect.
  return Object.keys(effect.request).length === 0 && success !== undefined &&
    Object.keys(success.payload).length === 0 && failure !== undefined &&
    Object.keys(failure.payload).length === 1 &&
    failure.payload.detail === "string" &&
    messageEffect(manifest, effect.success_message) === undefined &&
    messageEffect(manifest, effect.failure_message) === undefined;
}

function freezeData<T>(value: T): T {
  if (value !== null && typeof value === "object") {
    for (const child of Object.values(value)) freezeData(child);
    Object.freeze(value);
  }
  return value;
}

/** Core-local owner of one immutable UI/slot/target binding. Construction has
 * no ambient I/O. A mounted host commits context updates; Plugin data cannot
 * supply callbacks or use this owner as installation/authentication authority.
 */
export function createProviderUiOwner(
  input: ProviderUiManifest,
  slot: SurfaceSlot,
) {
  const manifest = structuredClone(input);
  validateProviderUiManifest(manifest);
  freezeData(manifest);
  const scope = createOwnedResourceScope();
  const observation: ProviderUiObservation = Object.freeze({
    get active() {
      return scope.active;
    },
  });
  const listeners = new Set<() => void>();
  const actions = new Map<ButtonNode, Action>();
  let context: ProviderUiContext | undefined;
  function visit(node: UiNode, visibility: readonly BoolExpression[]): void {
    if (node.component === "stack") {
      const conditions = node.visible_when
        ? [...visibility, node.visible_when]
        : visibility;
      for (const child of node.children) visit(child, conditions);
    } else if (node.component === "button") {
      const effect = messageEffect(manifest, node.emit.message);
      actions.set(node, {
        visibility,
        effect,
        supported: !effect || supportedEffect(manifest, effect),
      });
    }
  }
  visit(manifest.ui.surfaces[slot], []);
  let current: ProviderUiSnapshot = Object.freeze({
    state: Object.freeze(initialProviderState(manifest)),
    busyEffect: null,
    problem: [...actions.values()].some((action) => !action.supported)
      ? "unsupported_effect_profile"
      : null,
  });
  function publish(state: ProviderState, busyEffect: string | null): void {
    if (!scope.active) return;
    current = Object.freeze({
      state: Object.freeze(state),
      busyEffect,
      problem: current.problem,
    });
    // Snapshot intentionally: subscriptions created by a listener belong to the
    // next notification and must not make this dispatch unbounded.
    // oxlint-disable-next-line unicorn/no-useless-spread
    for (const listener of [...listeners]) {
      if (!scope.active) break;
      if (!listeners.has(listener)) continue;
      // Observers cannot interrupt an admitted host request or its settlement.
      try {
        listener();
      } catch { /* Observation is not execution authority. */ }
    }
  }
  const buttonState = (node: ButtonNode) => {
    const action = actions.get(node);
    const blocked = !action || !action.supported ||
      (action.effect !== undefined &&
        context?.blockedCapabilities?.has(action.effect.capability) === true);
    return {
      effect: action?.effect,
      blocked,
      busy: action?.effect !== undefined &&
        current.busyEffect === action.effect.id,
      disabled: blocked || !scope.active || !context ||
        current.busyEffect !== null ||
        (action?.effect !== undefined && !context.onEffect) ||
        !action?.visibility.every((condition) =>
          evaluateExpression(condition, current.state, context!.host)
        ) ||
        !evaluateExpression(node.enabled_when, current.state, context.host),
    };
  };
  return {
    manifest,
    slot,
    snapshot: (): ProviderUiSnapshot => current,
    lifecycle: scope.snapshot,
    subscribe(listener: () => void): () => void {
      if (scope.active) listeners.add(listener);
      return () => listeners.delete(listener);
    },
    /** Called only by committed core UI, never during a speculative render. */
    updateContext(next: ProviderUiContext): void {
      if (!scope.active) return;
      context = {
        host: Object.freeze({ ...next.host }),
        ...(next.onEffect ? { onEffect: next.onEffect } : {}),
        blockedCapabilities: new Set(next.blockedCapabilities),
      };
      publish(current.state, current.busyEffect);
    },
    buttonState,
    emit(node: ButtonNode): Promise<void> {
      if (buttonState(node).disabled) return Promise.resolve();
      const next = transitionProvider(manifest, current.state, node.emit);
      if (!next.effect) {
        publish(next.state, null);
        return Promise.resolve();
      }
      const effect = next.effect;
      const onEffect = context!.onEffect!;
      // run reserves a task lease before publish can reenter. publish reserves
      // the busy state before observers or the host callback can click again.
      return scope.run(async () => {
        publish(next.state, effect.id);
        try {
          await onEffect(effect, observation);
        } catch (cause) {
          if (scope.active) {
            publish(
              transitionProvider(manifest, current.state, {
                message: effect.failure_message,
                // Backend detail belongs to core management, not Plugin state.
                payload: { detail: "Provider operation failed" },
              }).state,
              null,
            );
          }
          throw cause;
        }
        if (scope.active) {
          publish(
            transitionProvider(manifest, current.state, {
              message: effect.success_message,
              payload: {},
            }).state,
            null,
          );
        }
      });
    },
    /** Seals immediately and drains submitted requests. Never aborts, retries,
     * undoes or claims to reconcile a remote effect after the view is gone.
     */
    dispose(): Promise<void> {
      listeners.clear();
      context = undefined;
      return scope.dispose();
    },
  };
}
export type ProviderUiOwner = ReturnType<typeof createProviderUiOwner>;
