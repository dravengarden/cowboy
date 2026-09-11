import type {
  EffectCapability,
  MessageEmission,
  ProviderUiManifest,
  ReducerRule,
  SurfaceSlot,
  ValueType,
} from "@cowboy/provider-ui";

/** Author-time linking only. Downloaded JSON still needs the independent SDK
 * validator, signature verification and core capability admission. No runtime
 * host, registration, resources, React or executable Plugin code is exported.
 */
export type ProviderUiContract = Pick<ProviderUiManifest, "logic" | "ui">;
type Scalar<T extends ValueType> = {
  string: string;
  bool: boolean;
  integer: number;
}[T];
type TypedStateField = {
  [V in ValueType]: { id: string; value_type: V; initial: Scalar<V> };
}[ValueType];
type States<C extends ProviderUiContract> = C["logic"]["state"][number];
type Messages<C extends ProviderUiContract> = C["logic"]["messages"][number];
type Effects<C extends ProviderUiContract> = C["logic"]["effects"][number];

export type ProviderStateOf<C extends ProviderUiContract> = {
  [F in States<C> as F["id"]]: Scalar<F["value_type"]>;
};
type Payload<C extends ProviderUiContract, M extends Messages<C>["id"]> = {
  [K in keyof Extract<Messages<C>, { id: M }>["payload"]]: Scalar<
    Extract<Messages<C>, { id: M }>["payload"][K]
  >;
};
export type ProviderMessageOf<C extends ProviderUiContract> = {
  [M in Messages<C>["id"]]: { message: M; payload: Payload<C, M> };
}[Messages<C>["id"]];

type KeysOfType<T, V> = {
  [K in keyof T]: T[K] extends V ? K : never;
}[keyof T];
type Assignment<C extends ProviderUiContract, M extends Messages<C>["id"]> = {
  [F in keyof ProviderStateOf<C>]: {
    field: F;
    value:
      | { source: "literal"; value: ProviderStateOf<C>[F] }
      | {
        source: "state";
        field: KeysOfType<ProviderStateOf<C>, ProviderStateOf<C>[F]>;
      }
      | {
        source: "message";
        field: KeysOfType<Payload<C, M>, ProviderStateOf<C>[F]>;
      };
  };
}[keyof ProviderStateOf<C>];
type CheckedReducer<C extends ProviderUiContract, R extends ReducerRule> =
  R extends unknown ? R["message"] extends Messages<C>["id"] ? {
        message: R["message"];
        assignments: Assignment<C, R["message"]>[];
        effect?: Effects<C>["id"];
      }
    : never
    : never;

type CheckedEmission<C extends ProviderUiContract, E extends MessageEmission> =
  {
    [M in Extract<Messages<C>["id"], E["message"]>]:
      Exclude<keyof E["payload"], keyof Payload<C, M>> extends never
        ? { message: M; payload: Payload<C, M> }
        : never;
  }[Extract<Messages<C>["id"], E["message"]>];
type SlotCapabilities<S extends SurfaceSlot> =
  | "open_external_documentation"
  | (S extends "setup"
    ? "begin_service_authentication" | "logout_service_authentication"
    : S extends "empty" ? "install_on_machine"
    : S extends "settings" ? "upgrade_on_machine" | "request_uninstall_plan"
    : never);
type ReducerEffects<R extends ReducerRule, M extends string> = R extends unknown
  ? M extends R["message"] ? R extends { effect?: infer E } ? Extract<E, string>
    : never
  : never
  : never;
type MessageCapabilities<C extends ProviderUiContract, M extends string> =
  Extract<
    Effects<C>,
    {
      id: ReducerEffects<
        C["logic"]["reducers"][number],
        M
      >;
    }
  >["capability"];

/** Walk the literal UI tree, including nested activity labels/conditions.
 * Checking actual emission keys also rejects excess keys on empty payloads;
 * TypeScript's ordinary structural `{}` alone cannot express that boundary.
 */
type CheckedUi<C extends ProviderUiContract, N, S extends SurfaceSlot> =
  N extends { component: "button"; emit: infer E extends MessageEmission }
    ? Exclude<MessageCapabilities<C, E["message"]>, SlotCapabilities<S>> extends
      never ?
        & { emit: CheckedEmission<C, E> }
        & { [K in Exclude<keyof N, "emit">]: CheckedUi<C, N[K], S> }
    : never
    : N extends { source: "state"; field: infer F }
      ? F extends keyof ProviderStateOf<C> ? N : never
    : N extends { op: "state_equals"; field: infer F }
      ? F extends keyof ProviderStateOf<C>
        ? { op: "state_equals"; field: F; value: ProviderStateOf<C>[F] }
      : never
    : N extends { asset: infer A }
      ? A extends C["ui"]["assets"][number]["id"]
        ? { [K in keyof N]: CheckedUi<C, N[K], S> }
      : never
    : N extends object ? { [K in keyof N]: CheckedUi<C, N[K], S> }
    : N;

type Linked<C extends ProviderUiContract> = string extends
  | States<C>["id"]
  | Messages<C>["id"]
  | Effects<C>["id"] ? never
  : {
    logic: {
      state: TypedStateField[];
      reducers: {
        [K in keyof C["logic"]["reducers"]]: C["logic"]["reducers"][K] extends
          ReducerRule ? CheckedReducer<C, C["logic"]["reducers"][K]>
          : C["logic"]["reducers"][K];
      };
      effects: {
        [K in keyof C["logic"]["effects"]]: C["logic"]["effects"][K] extends
          { capability: EffectCapability } ? {
            success_message: Messages<C>["id"];
            failure_message: Messages<C>["id"];
          }
          : C["logic"]["effects"][K];
      };
    };
    ui: {
      surfaces: {
        [S in SurfaceSlot]: CheckedUi<C, C["ui"]["surfaces"][S], S>;
      };
    };
  };

/** Infer declarations first; references cannot widen the schema to make an
 * invalid assignment/message legal. Produces the existing wire shape without
 * conversion. Integers, resource bounds, duplicate IDs and downloaded input
 * remain runtime-validation responsibilities. Use literals, not widened IR.
 */
export function defineProviderUiContract<const C extends ProviderUiContract>(
  contract: C & Linked<NoInfer<C>>,
): C {
  return contract;
}
