// A destructive confirmation: the system alert on iOS and Android, `window.confirm` in the web preview (where
// react-native-web's Alert does nothing).
import { Alert, Platform } from "react-native";

export function confirmAction(opts: {
  title: string;
  body: string;
  confirm: string;
  cancel?: string;
  destructive?: boolean;
  onConfirm: () => void;
}): void {
  if (Platform.OS === "web") {
    if (typeof window !== "undefined" && window.confirm(`${opts.title}\n\n${opts.body}`)) opts.onConfirm();
    return;
  }
  Alert.alert(opts.title, opts.body, [
    { text: opts.cancel ?? "Cancel", style: "cancel" },
    { text: opts.confirm, style: opts.destructive ? "destructive" : "default", onPress: opts.onConfirm },
  ]);
}
