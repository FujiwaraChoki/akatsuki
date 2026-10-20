// Web preview: no push notifications.
export type PushPermission = "granted" | "denied" | "undetermined" | "unavailable";

export function installNotificationHandler(): void {}
export function easProjectId(): string | null {
  return null;
}
export async function pushPermission(): Promise<PushPermission> {
  return "unavailable";
}
export async function obtainPushToken(): Promise<{ token: string | null; permission: PushPermission }> {
  return { token: null, permission: "unavailable" };
}
export function addResponseListener(_onOpen: (data: unknown) => void): () => void {
  return () => {};
}
