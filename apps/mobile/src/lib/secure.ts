// Small values that must stay on this device: the Better Auth session cookie, the device id and BYO AI keys
// (ARCHITECTURE §12.2: `expo-secure-store`, key `ai.<profileId>`). iOS Keychain / Android Keystore.
import * as SecureStore from "expo-secure-store";

const OPTIONS: SecureStore.SecureStoreOptions = {
  keychainAccessible: SecureStore.AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY,
};

/** Synchronous access for Better Auth's Expo plugin (`storage.getItem` must be sync). */
export const secureSync = {
  getItem: (key: string): string | null => SecureStore.getItem(key, OPTIONS),
  setItem: (key: string, value: string): void => SecureStore.setItem(key, value, OPTIONS),
};

export function secureGet(key: string): Promise<string | null> {
  return SecureStore.getItemAsync(key, OPTIONS);
}

export function secureSet(key: string, value: string): Promise<void> {
  return SecureStore.setItemAsync(key, value, OPTIONS);
}

export function secureDelete(key: string): Promise<void> {
  return SecureStore.deleteItemAsync(key, OPTIONS);
}

/** Where this platform keeps secrets, for settings copy. */
export const SECURE_STORE_NAME = "this device's secure storage";
