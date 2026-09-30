import {
  AuthApiError,
  isRecentProductAuthRequired,
  type ProductMe,
} from "./authApi";

export interface RecentProductAuthOptions {
  purpose?: "recent" | "primary" | "passkey";
  resumeLabel?: string;
  resumeWithUserGesture?: boolean;
}

export async function retryWithRecentProductAuth<T>(
  operation: () => Promise<T>,
  reauthenticate: (options?: RecentProductAuthOptions) => Promise<ProductMe>,
  options: RecentProductAuthOptions = {},
): Promise<T> {
  try {
    return await operation();
  } catch (reason) {
    if (!isRecentProductAuthRequired(reason)) throw reason;
    // A session deadline requires its specific ceremony, even when the caller
    // only requested a recent step-up. A Passkey cannot renew primary login.
    if (reason instanceof AuthApiError && reason.reauthKind) {
      options = { ...options, purpose: reason.reauthKind };
    }
  }
  await reauthenticate(options);
  return await operation();
}
