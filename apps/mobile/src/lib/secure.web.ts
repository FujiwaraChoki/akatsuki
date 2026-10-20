// Web preview of the mobile app (`expo export --platform web`): there is no keychain in a browser, so values live
// in memory for the tab's lifetime only. The product web client is apps/web; this build is for development.
const memory = new Map<string, string>();

export const secureSync = {
  getItem: (key: string): string | null => memory.get(key) ?? null,
  setItem: (key: string, value: string): void => {
    memory.set(key, value);
  },
};

export async function secureGet(key: string): Promise<string | null> {
  return memory.get(key) ?? null;
}

export async function secureSet(key: string, value: string): Promise<void> {
  memory.set(key, value);
}

export async function secureDelete(key: string): Promise<void> {
  memory.delete(key);
}

export const SECURE_STORE_NAME = "this browser tab's memory (not saved)";

/** The web preview keeps values in memory, which cannot fail. */
export function useSecureFailure(): null {
  return null;
}

export function resetSecureFailure(): void {}
