// Sign-out (§16.1): forget this device on the server (its push token goes with it, D-403), revoke the session,
// then wipe bodies, the AI keys on this device, the outbox and list caches.
import { api } from "@akatsuki/convex";
import { clearAccessToken } from "@akatsuki/wire";
import { signOutSession } from "./auth";
import { clearMemoryBodies } from "./bodies";
import { convex } from "./clients";
import { wipeLocalData } from "./db";
import { getDeviceId } from "./device";
import { secureDelete, secureGet, secureSet } from "./secure";

const AI_KEYS_INDEX = "ai.keys";

async function keyIndex(): Promise<Set<string>> {
  try {
    const raw = await secureGet(AI_KEYS_INDEX);
    return new Set<string>(raw ? (JSON.parse(raw) as string[]) : []);
  } catch {
    return new Set();
  }
}

/** Remembers which `ai.<profileId>` entries exist so sign-out can delete them (SecureStore cannot list keys). */
export async function recordAiKey(name: string, present: boolean): Promise<void> {
  const set = await keyIndex();
  if (present) set.add(name);
  else set.delete(name);
  await secureSet(AI_KEYS_INDEX, JSON.stringify([...set]));
}

export async function signOutEverywhereOnDevice(): Promise<void> {
  // While the session still works: remove this device's row so the next person to sign in on this phone does
  // not receive the previous user's new-mail pushes. Bounded so an offline sign-out is not held up.
  await forgetThisDevice();
  try {
    await signOutSession();
  } catch {
    // Offline: the local wipe still happens; the session expires on the server.
  }
  clearAccessToken();
  convex?.clearAuth();
  clearMemoryBodies();
  await wipeLocalData().catch(() => {});
  for (const name of await keyIndex()) await secureDelete(name).catch(() => {});
  await secureDelete(AI_KEYS_INDEX).catch(() => {});
}

async function forgetThisDevice(): Promise<void> {
  // Offline, the mutation would wait in the client's queue and could run under the next user's session.
  if (!convex?.connectionState().isWebSocketConnected) return;
  try {
    const deviceId = await getDeviceId();
    await Promise.race([
      convex.mutation(api.devices.remove, { deviceId }),
      new Promise((resolve) => setTimeout(resolve, 4000)),
    ]);
  } catch {
    // not_found (never registered) or offline. devices.register on the next sign-in takes the push token over
    // from this row anyway (D-403).
  }
}
