async function request(path: string, init?: RequestInit): Promise<Response> {
  const response = await fetch(path, {
    cache: "no-store",
    credentials: "same-origin",
    ...init,
  });
  if (!response.ok) {
    await response.body?.cancel();
    throw new Error(
      "The operation could not be completed. Refresh installation status before trying again.",
    );
  }
  return response;
}

export async function extensionManagementJson<T>(
  path: string,
  init?: RequestInit,
): Promise<T> {
  return (await request(path, init)).json();
}

/** Installation and removal acknowledge success with HTTP 204. */
export async function changeExtensionInstallation(
  path: string,
  init: RequestInit,
): Promise<void> {
  const response = await request(path, init);
  await response.body?.cancel();
}
