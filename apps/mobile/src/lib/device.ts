// This device's identity (ARCHITECTURE §1.2: `deviceId` is a UUIDv4 minted on first run and kept in SecureStore)
// and its `devices.register` payload.
import * as Application from "expo-application";
import * as Device from "expo-device";
import { Platform } from "react-native";
import { newUuidV4 } from "./ids";
import { secureGet, secureSet } from "./secure";

const DEVICE_KEY = "device.id";
let cached: string | null = null;

export async function getDeviceId(): Promise<string> {
  if (cached) return cached;
  const stored = await secureGet(DEVICE_KEY);
  if (stored && stored.length >= 8) {
    cached = stored;
    return stored;
  }
  const id = newUuidV4();
  await secureSet(DEVICE_KEY, id);
  cached = id;
  return id;
}

export function deviceName(): string {
  const model =
    Device.modelName ??
    (Platform.OS === "ios" ? "iPhone" : Platform.OS === "android" ? "Android" : "Browser");
  return `Akatsuki on ${model}`.slice(0, 120);
}

export function appVersion(): string {
  const v = Application.nativeApplicationVersion ?? "0.1.0";
  const b = Application.nativeBuildVersion;
  return b ? `${v} (${b})` : v;
}
