import type { Keymap, Platform } from "@akatsuki/shortcuts";
import type { ReactNode } from "react";
import { UiConfigProvider } from "./config.tsx";
import { ToastProvider } from "./Toast.tsx";

export interface AkatsukiUiProviderProps {
  /** Defaults to this browser's platform. */
  platform?: Platform;
  /** `prefs.keymap`. */
  keymap?: Keymap;
  /** Most toasts visible at once. */
  maxToasts?: number;
  /** One info toast at a time, errors (max two) above it (SPEC §3.9). */
  singleInfoToast?: boolean;
  children: ReactNode;
}

/** Wrap the app once: shortcut hints follow platform and keymap, and `useToast()` works. */
export function AkatsukiUiProvider({
  platform,
  keymap,
  maxToasts,
  singleInfoToast,
  children,
}: AkatsukiUiProviderProps) {
  return (
    <UiConfigProvider platform={platform} keymap={keymap}>
      <ToastProvider max={maxToasts} singleInfo={singleInfoToast}>
        {children}
      </ToastProvider>
    </UiConfigProvider>
  );
}
