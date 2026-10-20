// Haptic feedback for gestures and actions. Failures (web, devices without a Taptic Engine) are ignored.
import * as Haptics from "expo-haptics";

export function tick(): void {
  Haptics.selectionAsync().catch(() => {});
}

export function impact(): void {
  Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light).catch(() => {});
}

export function success(): void {
  Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success).catch(() => {});
}

export function warning(): void {
  Haptics.notificationAsync(Haptics.NotificationFeedbackType.Warning).catch(() => {});
}
