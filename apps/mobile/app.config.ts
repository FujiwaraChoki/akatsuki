// Expo app config (ARCHITECTURE §6.4). Bundle id / package `vc.akatsuki.app`, deep-link scheme `akatsuki`
// (Better Auth Expo plugin and the magic-link callback `akatsuki://auth/callback`).
//
// Public endpoints come from EXPO_PUBLIC_* variables at build time (ARCHITECTURE §18, "Clients"): set them in the
// shell for `expo start` / `expo export`, or as EAS environment variables for cloud builds. Never put secrets here:
// everything in this file and in EXPO_PUBLIC_* ends up inside the app binary.
import type { ConfigContext, ExpoConfig } from "expo/config";

const IOS_BUNDLE_ID = "vc.akatsuki.app";
const ANDROID_PACKAGE = "vc.akatsuki.app";

// Theme colours for native chrome (splash, notification accent). Mirrors packages/tokens/tokens.json; the JS
// side reads @akatsuki/tokens directly. A test (test/config.test.ts) keeps these in sync with the tokens.
const LIGHT_BACKGROUND = "#FFFFFF";
const DARK_BACKGROUND = "#0A0A0A";
const ACCENT = "#C1272D";

export default ({ config }: ConfigContext): ExpoConfig => ({
  ...config,
  name: "Akatsuki",
  slug: "akatsuki",
  owner: process.env.EXPO_OWNER || undefined,
  version: "0.1.0",
  scheme: "akatsuki",
  orientation: "default",
  userInterfaceStyle: "automatic",
  icon: "./assets/icon.png",
  platforms: ["ios", "android", "web"],
  ios: {
    bundleIdentifier: IOS_BUNDLE_ID,
    supportsTablet: true,
    buildNumber: "1",
    config: { usesNonExemptEncryption: false },
    infoPlist: {
      // Mail links open in the system browser; the app itself never loads remote pages.
      LSApplicationQueriesSchemes: ["mailto", "https", "http"],
      // A self-hosted model on the user's network (Ollama, LM Studio) is reached over plain http (D-334).
      NSAppTransportSecurity: { NSAllowsLocalNetworking: true },
      NSLocalNetworkUsageDescription:
        "Akatsuki connects to an AI model on your local network only if you add one in Settings.",
    },
  },
  android: {
    package: ANDROID_PACKAGE,
    versionCode: 1,
    // The logo is a red dot (D-430): the dot is the foreground, so the ground is the light background.
    adaptiveIcon: {
      foregroundImage: "./assets/adaptive-icon.png",
      backgroundColor: LIGHT_BACKGROUND,
    },
    // Nothing beyond what the modules declare (INTERNET, notifications). No contacts, no storage.
    blockedPermissions: [
      "android.permission.READ_EXTERNAL_STORAGE",
      "android.permission.WRITE_EXTERNAL_STORAGE",
      "android.permission.SYSTEM_ALERT_WINDOW",
    ],
    intentFilters: [
      {
        action: "VIEW",
        category: ["BROWSABLE", "DEFAULT"],
        data: [{ scheme: "akatsuki" }],
      },
    ],
  },
  web: {
    bundler: "metro",
    output: "single",
    favicon: "./assets/favicon.png",
  },
  plugins: [
    "expo-router",
    // Android blocks cleartext http by default; the app enforces https everywhere except a local-network AI
    // endpoint the user typed in (baseUrlProblem in packages/ai), D-334.
    ["expo-build-properties", { android: { usesCleartextTraffic: true } }],
    "expo-secure-store",
    "expo-sqlite",
    "expo-web-browser",
    "expo-localization",
    // Dictation (§12.1, D-461): the microphone, asked for on the first Record in the composer. No background
    // recording or playback.
    [
      "expo-audio",
      {
        microphonePermission:
          "Akatsuki records your voice only while you dictate a message, then turns it into text.",
        enableBackgroundRecording: false,
        enableBackgroundPlayback: false,
      },
    ],
    // Attachments (§10.4): the photo library for photos and videos; no camera (the microphone is expo-audio's).
    [
      "expo-image-picker",
      {
        photosPermission: "Akatsuki attaches the photos and videos you choose to your message.",
        cameraPermission: false,
        microphonePermission: false,
      },
    ],
    [
      "expo-notifications",
      {
        icon: "./assets/notification-icon.png",
        color: ACCENT,
      },
    ],
    [
      "expo-splash-screen",
      {
        image: "./assets/splash-icon.png",
        imageWidth: 160,
        resizeMode: "contain",
        backgroundColor: LIGHT_BACKGROUND,
        dark: { image: "./assets/splash-icon-dark.png", backgroundColor: DARK_BACKGROUND },
      },
    ],
  ],
  experiments: {
    typedRoutes: false,
  },
  extra: {
    // The EAS project id is created by `eas init` with the owner's Expo account (not done yet). Push tokens need
    // it (expo-notifications `getExpoPushTokenAsync({ projectId })`); until it exists push registration is off.
    eas: process.env.EAS_PROJECT_ID ? { projectId: process.env.EAS_PROJECT_ID } : undefined,
    router: {},
  },
});

export const NATIVE_THEME = { LIGHT_BACKGROUND, DARK_BACKGROUND, ACCENT, IOS_BUNDLE_ID, ANDROID_PACKAGE };
