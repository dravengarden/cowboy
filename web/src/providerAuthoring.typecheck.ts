import {
  defineProviderUiContract,
  type ProviderMessageOf,
  type ProviderStateOf,
  type ProviderUiContract,
} from "@cowboy/provider-authoring";
import { providerUiContractFixture as fixture } from "./providerUiContract.fixture";

/** Never executed. The strict production tsc gate checks negative contracts. */
export function checkProviderAuthoringTypes(
  widened: ProviderUiContract,
  ambiguousMessage: "start" | "docs",
): void {
  defineProviderUiContract(fixture);
  // @ts-expect-error A widened/decoded wire type is not a literal authoring proof.
  defineProviderUiContract(widened);
  const state: ProviderStateOf<typeof fixture> = {
    busy: false,
    count: 2,
    detail: "",
  };
  // @ts-expect-error State IDs retain their declared scalar type.
  state.busy = "true";
  const message: ProviderMessageOf<typeof fixture> = {
    message: "start",
    payload: { count: 2 },
  };
  if (message.message === "start") {
    const count: number = message.payload.count;
    void count;
    // @ts-expect-error The message discriminant selects its exact payload.
    void message.payload.detail;
  }
  const badInitial = {
    ...fixture,
    logic: {
      ...fixture.logic,
      state: [
        { ...fixture.logic.state[0], initial: "false" },
        fixture.logic.state[1],
        fixture.logic.state[2],
      ],
    },
  };
  // @ts-expect-error Initial values and field declarations cannot disagree.
  defineProviderUiContract(badInitial);
  const badSource = {
    ...fixture,
    logic: {
      ...fixture.logic,
      reducers: [{
        message: "start" as const,
        assignments: [{
          field: "busy" as const,
          value: { source: "message" as const, field: "count" as const },
        }],
      }],
    },
  };
  // @ts-expect-error An integer payload cannot be assigned to a boolean state.
  defineProviderUiContract(badSource);
  const badLiteral = {
    ...fixture,
    logic: {
      ...fixture.logic,
      reducers: [{
        message: "done" as const,
        assignments: [{
          field: "busy" as const,
          value: { source: "literal" as const, value: 1 },
        }],
      }],
    },
  };
  // @ts-expect-error Literal assignments retain the target type.
  defineProviderUiContract(badLiteral);
  const badState = {
    ...fixture,
    logic: {
      ...fixture.logic,
      reducers: [{
        message: "done" as const,
        assignments: [{
          field: "busy" as const,
          value: { source: "state" as const, field: "count" as const },
        }],
      }],
    },
  };
  // @ts-expect-error State-to-state assignments cannot change scalar type.
  defineProviderUiContract(badState);
  const badTarget = {
    ...fixture,
    logic: {
      ...fixture.logic,
      reducers: [{
        message: "done" as const,
        assignments: [{
          field: "typo" as const,
          value: { source: "literal" as const, value: false },
        }],
      }],
    },
  };
  // @ts-expect-error Reducer targets must exist.
  defineProviderUiContract(badTarget);
  const badEffect = {
    ...fixture,
    logic: {
      ...fixture.logic,
      reducers: [{
        message: "start" as const,
        assignments: [],
        effect: "typo",
      }],
    },
  };
  // @ts-expect-error Effects must be declared.
  defineProviderUiContract(badEffect);
  const badMessage = {
    ...fixture,
    logic: {
      ...fixture.logic,
      reducers: [{ message: "typo", assignments: [] }],
    },
  };
  // @ts-expect-error Reducers must consume a declared message.
  defineProviderUiContract(badMessage);
  const badCompletion = {
    ...fixture,
    logic: {
      ...fixture.logic,
      effects: [{ ...fixture.logic.effects[0], success_message: "typo" }],
    },
  };
  // @ts-expect-error Completion messages must exist.
  defineProviderUiContract(badCompletion);
  const button = fixture.ui.surfaces.empty.children[0];
  const badPayload = {
    ...fixture,
    ui: {
      ...fixture.ui,
      surfaces: {
        ...fixture.ui.surfaces,
        empty: {
          ...button,
          emit: { message: "start" as const, payload: { count: "one" } },
        },
      },
    },
  };
  // @ts-expect-error Payload value is linked to the message declaration.
  defineProviderUiContract(badPayload);
  const missingPayload = {
    ...fixture,
    ui: {
      ...fixture.ui,
      surfaces: {
        ...fixture.ui.surfaces,
        empty: { ...button, emit: { message: "start" as const, payload: {} } },
      },
    },
  };
  // @ts-expect-error Required payload fields cannot be omitted.
  defineProviderUiContract(missingPayload);
  const ambiguousPayload = {
    ...fixture,
    ui: {
      ...fixture.ui,
      surfaces: {
        ...fixture.ui.surfaces,
        empty: { ...button, emit: { message: ambiguousMessage, payload: {} } },
      },
    },
  };
  // @ts-expect-error A union discriminant cannot hide a required payload field.
  defineProviderUiContract(ambiguousPayload);
  const extraPayload = {
    ...fixture,
    ui: {
      ...fixture.ui,
      surfaces: {
        ...fixture.ui.surfaces,
        empty: {
          ...button,
          emit: { message: "docs" as const, payload: { extra: true } },
        },
      },
    },
  };
  // @ts-expect-error Empty payload schemas reject extra keys, including variables.
  defineProviderUiContract(extraPayload);
  const badCondition = {
    ...fixture,
    ui: {
      ...fixture.ui,
      surfaces: {
        ...fixture.ui.surfaces,
        empty: {
          ...button,
          enabled_when: {
            op: "state_equals" as const,
            field: "count" as const,
            value: false,
          },
        },
      },
    },
  };
  // @ts-expect-error State equality uses the field's scalar type.
  defineProviderUiContract(badCondition);
  const badLabel = {
    ...fixture,
    ui: {
      ...fixture.ui,
      surfaces: {
        ...fixture.ui.surfaces,
        empty: {
          ...button,
          label: { source: "state" as const, field: "typo" },
        },
      },
    },
  };
  // @ts-expect-error UI state references cannot dangle.
  defineProviderUiContract(badLabel);
  const wrongSlot = {
    ...fixture,
    ui: {
      ...fixture.ui,
      surfaces: { ...fixture.ui.surfaces, setup: fixture.ui.surfaces.empty },
    },
  };
  // @ts-expect-error Install effects cannot escape into Service authentication UI.
  defineProviderUiContract(wrongSlot);
  const optionalEffect: {
    message: "start" | "docs";
    assignments: [];
    effect?: "install";
  } = { message: "start", assignments: [], effect: "install" };
  const optionalEscape = {
    ...wrongSlot,
    logic: { ...fixture.logic, reducers: [optionalEffect] },
  };
  // @ts-expect-error Optional effects or union message names still carry their possible capability.
  defineProviderUiContract(optionalEscape);
  const badAsset = {
    ...fixture,
    ui: {
      ...fixture.ui,
      surfaces: {
        ...fixture.ui.surfaces,
        card: {
          component: "asset" as const,
          asset: "typo",
          size: "sm" as const,
        },
      },
    },
  };
  // @ts-expect-error Asset IDs must resolve.
  defineProviderUiContract(badAsset);
}
