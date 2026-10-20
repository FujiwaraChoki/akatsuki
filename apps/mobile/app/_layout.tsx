// Root: fonts, gesture handler, safe areas, Convex + Better Auth (§6.4). Screens under (app) need a session.
import { ConvexBetterAuthProvider } from "@convex-dev/better-auth/react";
import { useFonts } from "expo-font";
import { Stack } from "expo-router";
import * as SplashScreen from "expo-splash-screen";
import { StatusBar } from "expo-status-bar";
import { type ComponentProps, useEffect } from "react";
import { View } from "react-native";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { ToastHost } from "../src/components/Toasts";
import { Label, Txt } from "../src/components/ui";
import { ENV } from "../src/env";
import { authClient } from "../src/lib/auth";
import { convex } from "../src/lib/clients";
import { FONT_SOURCES } from "../src/theme/fonts";
import { ThemeProvider, useTheme } from "../src/theme/ThemeProvider";

SplashScreen.preventAutoHideAsync().catch(() => {});

// The provider's AuthClient type does not accept the Expo plugin's client type, although it is one (same package
// pair as convex/: @convex-dev/better-auth 0.12.5, better-auth 1.6.33).
type BetterAuthClientProp = ComponentProps<typeof ConvexBetterAuthProvider>["authClient"];

function NotConfigured({ missing }: { missing: string[] }) {
  const t = useTheme();
  return (
    <View
      style={{
        flex: 1,
        backgroundColor: t.colors.background,
        padding: t.space[3],
        justifyContent: "center",
        gap: t.space[2],
      }}
    >
      <Txt variant="heading">Akatsuki is not configured</Txt>
      <Txt variant="body" tone="secondary">
        This build has no server addresses. Set these variables and rebuild:
      </Txt>
      {missing.map((m) => (
        <Label key={m}>{m}</Label>
      ))}
    </View>
  );
}

function AppStack() {
  const t = useTheme();
  return (
    <>
      <StatusBar style={t.mode === "dark" ? "light" : "dark"} />
      <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: t.colors.background } }}>
        <Stack.Screen name="index" />
        <Stack.Screen name="sign-in" options={{ animation: "fade" }} />
        <Stack.Screen name="auth/callback" options={{ animation: "fade" }} />
        <Stack.Screen name="(app)" options={{ animation: "fade" }} />
      </Stack>
    </>
  );
}

export default function RootLayout() {
  const [fontsLoaded, fontError] = useFonts(FONT_SOURCES);
  const ready = fontsLoaded || fontError !== null;
  useEffect(() => {
    if (ready) SplashScreen.hideAsync().catch(() => {});
  }, [ready]);
  if (!ready) return null;
  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <SafeAreaProvider>
        <ThemeProvider>
          {ENV.env && convex && authClient ? (
            <ConvexBetterAuthProvider
              client={convex}
              authClient={authClient as unknown as BetterAuthClientProp}
            >
              <AppStack />
              <ToastHost />
            </ConvexBetterAuthProvider>
          ) : (
            <NotConfigured missing={ENV.missing} />
          )}
        </ThemeProvider>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}
