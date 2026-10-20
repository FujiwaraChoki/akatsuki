// The signed-in shell: mail context, theme from prefs, hardware keyboard, global shortcuts, toasts for op
// failures, outbox replay (§16.1), device registration with the push token (§4.2 `devices.register`).
import { api } from "@akatsuki/convex";
import { useConvexAuth, useMutation } from "convex/react";
import { Redirect, router, Stack } from "expo-router";
import { useEffect, useRef, useState } from "react";
import { Platform, View } from "react-native";
import { FailureWatcher } from "../../src/components/Toasts";
import { Button, Spinner, Txt } from "../../src/components/ui";
import { authClient } from "../../src/lib/auth";
import { convex } from "../../src/lib/clients";
import { appVersion, deviceName, getDeviceId } from "../../src/lib/device";
import {
  addResponseListener,
  installNotificationHandler,
  obtainPushToken,
  pushPermission,
} from "../../src/lib/push";
import { signOutEverywhereOnDevice } from "../../src/lib/session";
import { outbox, useThreadActions } from "../../src/state/actions";
import { KeyboardProvider, useGlobalShortcuts, useKeyboard } from "../../src/state/keyboard";
import { deviceTimeZone, MailProvider, useMail } from "../../src/state/mail";
import { ThemeProvider, useTheme } from "../../src/theme/ThemeProvider";
import { notificationTarget } from "../../src/vm/notifications";
import { mailboxForShortcut } from "../../src/vm/threadList";

function GlobalKeys() {
  const { undoLast } = useThreadActions();
  const keyboard = useKeyboard();
  const go = (shortcut: string) => {
    const m = mailboxForShortcut(shortcut);
    if (m) router.navigate({ pathname: "/inbox", params: { mailbox: m.id } });
  };
  useGlobalShortcuts({
    "palette.open": () =>
      router.push({ pathname: "/palette", params: { scopes: keyboard.activeScopes().join(",") } }),
    "help.shortcuts": () => router.push("/settings/shortcuts"),
    "compose.new": () => router.push("/compose"),
    "search.open": () => router.push("/search"),
    "settings.open": () => router.push("/settings"),
    undo: () => {
      undoLast();
    },
    "go.inbox": () => go("go.inbox"),
    "go.starred": () => go("go.starred"),
    "go.sent": () => go("go.sent"),
    "go.drafts": () => go("go.drafts"),
    "go.snoozed": () => go("go.snoozed"),
    "go.all": () => go("go.all"),
    "go.spam": () => go("go.spam"),
    "go.trash": () => go("go.trash"),
    "go.reminders": () => router.push("/reminders"),
    "go.label": () => router.push({ pathname: "/palette", params: { mode: "labels" } }),
    // Palette-only commands (no keys): labels are managed on their settings page.
    "labels.create": () => router.push("/settings/labels"),
    "labels.rename": () => router.push("/settings/labels"),
    "labels.remove": () => router.push("/settings/labels"),
    "sends.scheduled": () => router.push("/scheduled"),
    "compose.continueDraft": () => go("go.drafts"),
    // The search screen registers its own handlers for these; from elsewhere they open it.
    "search.ai": () => router.push("/search"),
    "search.fullMailbox": () => router.push("/search"),
  });
  return null;
}

/** One-time work after sign-in: create missing user rows, register this device, replay the outbox. */
function SessionEffects() {
  const mail = useMail();
  const bootstrap = useMutation(api.me.bootstrap);
  const register = useMutation(api.devices.register);
  const did = useRef({ bootstrap: false, register: false, replay: false });

  useEffect(() => {
    if (mail.me === null && !did.current.bootstrap) {
      did.current.bootstrap = true;
      bootstrap({ timezone: deviceTimeZone() }).catch(() => {
        did.current.bootstrap = false;
      });
    }
  }, [mail.me, bootstrap]);

  useEffect(() => {
    if (!mail.me || did.current.register) return;
    did.current.register = true;
    (async () => {
      const deviceId = await getDeviceId();
      let expoPushToken: string | undefined;
      if (mail.prefs?.notifications !== "off" && (await pushPermission()) === "granted") {
        expoPushToken = (await obtainPushToken().catch(() => ({ token: null }))).token ?? undefined;
      }
      await register({
        deviceId,
        kind: "mobile",
        name: deviceName(),
        appVersion: appVersion(),
        ...(expoPushToken ? { expoPushToken } : {}),
      });
    })().catch(() => {
      did.current.register = false;
    });
  }, [mail.me, mail.prefs?.notifications, register]);

  useEffect(() => {
    if (!mail.entitled || did.current.replay || !convex) return;
    did.current.replay = true;
    const c = convex;
    outbox
      .replay(async (e) => {
        if (e.fn === "mail.apply") await c.mutation(api.mail.apply, e.args as never);
        else if (e.fn === "mail.snooze") await c.mutation(api.mail.snooze, e.args as never);
        else if (e.fn === "mail.unsnooze") await c.mutation(api.mail.unsnooze, e.args as never);
        else if (e.fn === "mail.undo") await c.mutation(api.mail.undo, e.args as never);
        else if (e.fn === "mail.setReminder") await c.mutation(api.mail.setReminder, e.args as never);
        return "done";
      }, Date.now())
      .catch(() => {});
  }, [mail.entitled]);

  useEffect(() => {
    if (Platform.OS === "web") return;
    installNotificationHandler();
    return addResponseListener((data) => router.push(notificationTarget(data)));
  }, []);

  return null;
}

function Shell() {
  const mail = useMail();
  return (
    <ThemeProvider pref={mail.prefs?.theme}>
      <KeyboardProvider keymap={mail.prefs?.keymap ?? "superhuman"}>
        <SessionEffects />
        <GlobalKeys />
        <FailureWatcher />
        <ShellStack />
      </KeyboardProvider>
    </ThemeProvider>
  );
}

function ShellStack() {
  const t = useTheme();
  return (
    <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: t.colors.background } }}>
      <Stack.Screen name="inbox" />
      <Stack.Screen name="thread/[threadId]" />
      <Stack.Screen name="search" options={{ animation: "fade" }} />
      <Stack.Screen name="reminders" />
      <Stack.Screen name="settings/index" />
      <Stack.Screen name="settings/ai" />
      <Stack.Screen name="settings/shortcuts" />
      <Stack.Screen name="settings/labels" />
      <Stack.Screen name="settings/snippets" />
      <Stack.Screen name="scheduled" />
      <Stack.Screen name="compose" options={{ presentation: "modal", gestureEnabled: true }} />
      <Stack.Screen
        name="snooze"
        options={{ presentation: "formSheet", sheetAllowedDetents: [0.6, 1], sheetGrabberVisible: true }}
      />
      <Stack.Screen name="palette" options={{ presentation: "transparentModal", animation: "fade" }} />
      <Stack.Screen
        name="mailboxes"
        options={{ presentation: "formSheet", sheetAllowedDetents: [0.7, 1], sheetGrabberVisible: true }}
      />
    </Stack>
  );
}

/** While the session or the Convex token is loading; after 12 s it says so and offers a way out. */
function Connecting() {
  const t = useTheme();
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    const id = setTimeout(() => setSlow(true), 12_000);
    return () => clearTimeout(id);
  }, []);
  return (
    <View
      style={{
        flex: 1,
        alignItems: "center",
        justifyContent: "center",
        gap: t.space[2],
        padding: t.space[3],
        backgroundColor: t.colors.background,
      }}
    >
      <Spinner />
      {slow ? (
        <>
          <Txt
            variant="body"
            tone="secondary"
            style={{ textAlign: "center" }}
            accessibilityLiveRegion="polite"
          >
            Still connecting to Akatsuki. Check your connection.
          </Txt>
          <Button
            title="Sign out"
            variant="ghost"
            onPress={() =>
              signOutEverywhereOnDevice()
                .catch(() => {})
                .then(() => router.replace("/sign-in"))
            }
          />
        </>
      ) : null}
    </View>
  );
}

export default function AppLayout() {
  const session = authClient!.useSession();
  const convexAuth = useConvexAuth();
  if (session.isPending) return <Connecting />;
  if (!session.data) return <Redirect href="/sign-in" />;
  // Convex must hold this session's token before any query runs; right after sign-in it can lag a moment, and
  // queries sent without it fail with `unauthenticated`.
  if (convexAuth.isLoading || !convexAuth.isAuthenticated) return <Connecting />;
  return (
    <MailProvider>
      <Shell />
    </MailProvider>
  );
}
