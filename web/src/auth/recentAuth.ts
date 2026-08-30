import { isRecentProductAuthRequired, type ProductMe } from "./authApi";

export interface RecentProductAuthOptions {
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
  }
  await reauthenticate(options);
  return await operation();
}
