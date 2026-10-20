// Small values that must stay on this device: the Better Auth session cookie, the device id and BYO AI keys
// (ARCHITECTURE §12.2: `expo-secure-store`, key `ai.<profileId>`). iOS Keychain / Android Keystore.
//
// Every call is guarded: when the store cannot be used (a simulator build without a keychain entitlement, a
// locked keystore) the failure is recorded once and the root layout shows `StorageFailureScreen` instead of a
// spinner that never ends. Nothing here reads the store on import; callers read a value when they need it.
import * as SecureStore from "expo-secure-store";
import { useSyncExternalStore } from "react";
import { SECURE_STORAGE_ERROR, type StorageFailure, storageFailure } from "../vm/storage";

const OPTIONS: SecureStore.SecureStoreOptions = {
  keychainAccessible: SecureStore.AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY,
};

let failure: StorageFailure | null = null;
const listeners = new Set<() => void>();

function report(err: unknown): void {
  if (failure) return;
  failure = storageFailure(err);
  for (const l of [...listeners]) l();
}

/** Clears a recorded failure (the "Try again" button); the next call reports again if it still fails. */
export function resetSecureFailure(): void {
  failure = null;
  for (const l of [...listeners]) l();
}

/** The recorded secure-storage failure, if any. */
export function useSecureFailure(): StorageFailure | null {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => failure,
    () => failure,
  );
}

/** Synchronous access for Better Auth's Expo plugin (`storage.getItem` must be sync). */
export const secureSync = {
  getItem: (key: string): string | null => {
    try {
      return SecureStore.getItem(key, OPTIONS);
    } catch (err) {
      report(err);
      return null;
    }
  },
  setItem: (key: string, value: string): void => {
    try {
      SecureStore.setItem(key, value, OPTIONS);
    } catch (err) {
      report(err);
    }
  },
};

async function guarded<T>(run: () => Promise<T>): Promise<T> {
  try {
    return await run();
  } catch (err) {
    report(err);
    throw new Error(SECURE_STORAGE_ERROR);
  }
}

export function secureGet(key: string): Promise<string | null> {
  return guarded(() => SecureStore.getItemAsync(key, OPTIONS));
}

export function secureSet(key: string, value: string): Promise<void> {
  return guarded(() => SecureStore.setItemAsync(key, value, OPTIONS));
}

export function secureDelete(key: string): Promise<void> {
  return guarded(() => SecureStore.deleteItemAsync(key, OPTIONS));
}

/** Where this platform keeps secrets, for settings copy. */
export const SECURE_STORE_NAME = "this device's secure storage";
