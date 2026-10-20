import { getRandomBytes } from "expo-crypto";
import { uuidv4, uuidv7 } from "../vm/ids.ts";

/** A new UUIDv7 for `clientOpId`, `clientDraftId` and `clientSendId` (§1.2). */
export function newClientId(): string {
  return uuidv7(Date.now(), getRandomBytes);
}

/** A new UUIDv4 (deviceId, AI profile ids). */
export function newUuidV4(): string {
  return uuidv4(getRandomBytes);
}
