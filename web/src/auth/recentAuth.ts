import { isRecentProductAuthRequired, type ProductMe } from "./authApi";

export async function retryWithRecentProductAuth<T>(
  operation: () => Promise<T>,
  reauthenticate: () => Promise<ProductMe>,
): Promise<T> {
  try {
    return await operation();
  } catch (reason) {
    if (!isRecentProductAuthRequired(reason)) throw reason;
  }
  await reauthenticate();
  return await operation();
}
