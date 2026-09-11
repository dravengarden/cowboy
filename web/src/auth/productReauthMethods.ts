import {
  type AuthHostPlugin,
  PASSWORD_LOGIN_METHOD,
  type ProductOidcProvider,
} from "./authApi";

const PROVIDER_PREFIX = "provider:";

export interface ProductAccountVerificationMethod {
  id: string;
  label: string;
  authMethod: string;
}

export interface ProductPrimaryReauthMethods {
  methods: ProductAccountVerificationMethod[];
  legacySession: boolean;
  unavailableMethod?: string;
}

export function providerVerificationMethodId(providerId: string): string {
  return `${PROVIDER_PREFIX}${providerId}`;
}

/** Only external identities may select their presentation through a Plugin. */
export function loginMethodLabel(
  id: string,
  hostPlugins: readonly AuthHostPlugin[] | undefined,
  providers: readonly ProductOidcProvider[],
): string {
  if (id === PASSWORD_LOGIN_METHOD) return "Password";
  const hostLabel = hostPlugins?.find((plugin) => plugin.id === id)?.label
    ?.trim();
  if (hostLabel) return hostLabel;
  return providers.find((provider) => provider.id === id)?.display_name ?? id;
}

export function productAccountVerificationMethods(
  orderedMethodIds: string[],
  passwordEnabled: boolean,
  providers: ProductOidcProvider[],
  hostPlugins: readonly AuthHostPlugin[] = [],
): ProductAccountVerificationMethod[] {
  return orderedMethodIds.flatMap((id) => {
    if (id === PASSWORD_LOGIN_METHOD) {
      return passwordEnabled
        ? [{
          id: PASSWORD_LOGIN_METHOD,
          label: loginMethodLabel(id, hostPlugins, providers),
          authMethod: PASSWORD_LOGIN_METHOD,
        }]
        : [];
    }
    const provider = providers.find((candidate) => candidate.id === id);
    return provider
      ? [{
        id: providerVerificationMethodId(provider.id),
        label: loginMethodLabel(id, hostPlugins, providers),
        authMethod: provider.id,
      }]
      : [];
  });
}

export function resolvePrimaryReauthMethods(
  primaryAuthMethod: string | null | undefined,
  accountMethods: ProductAccountVerificationMethod[],
): ProductPrimaryReauthMethods {
  if (primaryAuthMethod == null) {
    return { methods: accountMethods, legacySession: true };
  }
  const method = accountMethods.find((candidate) =>
    candidate.authMethod === primaryAuthMethod
  );
  if (method) return { methods: [method], legacySession: false };
  return {
    methods: [],
    legacySession: false,
    unavailableMethod: primaryAuthMethod,
  };
}
