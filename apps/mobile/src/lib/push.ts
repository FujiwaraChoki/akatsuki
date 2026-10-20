// Push notifications (ARCHITECTURE §4.5 `internal.notify.newMail`, §14.2, D-351). Convex sends through Expo's push
// service; this module asks for permission and obtains the Expo push token that `devices.register` stores.
// Nothing here decides what a notification contains: `prefs.notifications` does, on the server.
import Constants from "expo-constants";
import * as Device from "expo-device";
import * as Notifications from "expo-notifications";
import { Platform } from "react-native";

export type PushPermission = "granted" | "denied" | "undetermined" | "unavailable";

let handlerInstalled = false;

/** Foreground presentation: a banner, no sound, no badge (the app shows its own counts). */
export function installNotificationHandler(): void {
  if (handlerInstalled) return;
  handlerInstalled = true;
  Notifications.setNotificationHandler({
    handleNotification: async () => ({
      shouldShowBanner: true,
      shouldShowList: true,
      shouldPlaySound: false,
      shouldSetBadge: false,
    }),
  });
}

export function easProjectId(): string | null {
  const extra = Constants.expoConfig?.extra as { eas?: { projectId?: string } } | undefined;
  return extra?.eas?.projectId ?? Constants.easConfig?.projectId ?? null;
}

export async function pushPermission(): Promise<PushPermission> {
  if (!Device.isDevice) return "unavailable";
  const { status } = await Notifications.getPermissionsAsync();
  return status === "granted" ? "granted" : status === "denied" ? "denied" : "undetermined";
}

/**
 * Asks for permission (once) and returns the Expo push token, or null when push is unavailable (simulator, no
 * EAS project id in this build, permission denied).
 */
export async function obtainPushToken(): Promise<{ token: string | null; permission: PushPermission }> {
  if (!Device.isDevice) return { token: null, permission: "unavailable" };
  const projectId = easProjectId();
  if (Platform.OS === "android") {
    await Notifications.setNotificationChannelAsync("mail", {
      name: "New mail",
      importance: Notifications.AndroidImportance.DEFAULT,
      showBadge: false,
    });
  }
  let { status } = await Notifications.getPermissionsAsync();
  if (status !== "granted") status = (await Notifications.requestPermissionsAsync()).status;
  if (status !== "granted")
    return { token: null, permission: status === "denied" ? "denied" : "undetermined" };
  if (!projectId) return { token: null, permission: "granted" };
  const token = await Notifications.getExpoPushTokenAsync({ projectId });
  return { token: token.data, permission: "granted" };
}

export function addResponseListener(onOpen: (data: unknown) => void): () => void {
  const sub = Notifications.addNotificationResponseReceivedListener((response) => {
    onOpen(response.notification.request.content.data);
  });
  return () => sub.remove();
}
