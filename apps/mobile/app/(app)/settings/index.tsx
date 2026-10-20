// Settings: account, plan status (no purchase, §13.1), mailboxes, notifications (§14.2), appearance, sending,
// privacy, AI providers, devices, shortcuts, account deletion.

import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import { useMutation, useQuery } from "convex/react";
import * as Linking from "expo-linking";
import { router } from "expo-router";
import * as WebBrowser from "expo-web-browser";
import { useEffect, useState } from "react";
import { Alert, Modal, ScrollView, Switch, View } from "react-native";
import { Header, HeaderButton } from "../../../src/components/Header";
import { Button, Chip, Choice, Field, Row, Screen, Section, Txt } from "../../../src/components/ui";
import { authClient } from "../../../src/lib/auth";
import { api as apiClient, worker } from "../../../src/lib/clients";
import { appVersion, deviceName, getDeviceId } from "../../../src/lib/device";
import { errorText } from "../../../src/lib/errors";
import { easProjectId, obtainPushToken, type PushPermission, pushPermission } from "../../../src/lib/push";
import { signOutEverywhereOnDevice } from "../../../src/lib/session";
import { useShortcuts } from "../../../src/state/keyboard";
import { useMail } from "../../../src/state/mail";
import { showToast } from "../../../src/state/toasts";
import { useTheme } from "../../../src/theme/ThemeProvider";
import { defaultProfile, egressView, settingsOrEmpty } from "../../../src/vm/aiSettings";
import {
  FULL_MODE_WARNING,
  NOTIFICATION_MODES,
  type NotificationMode,
  needsWarning,
  permissionText,
} from "../../../src/vm/notifications";
import { planStatus } from "../../../src/vm/plan";

const CONNECT_RETURN = "akatsuki://connected";
const UNDO_CHOICES = [0, 5, 10, 20, 30];

function ImapForm({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const t = useTheme();
  const [f, setF] = useState({
    email: "",
    username: "",
    password: "",
    imapHost: "",
    imapPort: "993",
    smtpHost: "",
    smtpPort: "465",
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = (k: keyof typeof f) => (v: string) => setF((x) => ({ ...x, [k]: v }));
  const submit = async () => {
    if (!worker) return;
    setBusy(true);
    setError(null);
    try {
      const imapPort = Number(f.imapPort);
      const smtpPort = Number(f.smtpPort);
      await worker.connectImap({
        email: f.email.trim().toLowerCase(),
        username: f.username.trim() || f.email.trim(),
        password: f.password,
        imap: { host: f.imapHost.trim(), port: imapPort, tls: imapPort === 143 ? "starttls" : "implicit" },
        smtp: { host: f.smtpHost.trim(), port: smtpPort, tls: smtpPort === 587 ? "starttls" : "implicit" },
      });
      setF((x) => ({ ...x, password: "" }));
      onClose();
      showToast({ id: "imap", text: "Mailbox connected. The first sync is running." });
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal visible={visible} animationType="slide" presentationStyle="pageSheet" onRequestClose={onClose}>
      <ScrollView
        style={{ backgroundColor: t.colors.background }}
        contentContainerStyle={{ padding: t.space[2], gap: t.space[2] }}
        keyboardShouldPersistTaps="handled"
      >
        <Txt variant="heading">Connect IMAP</Txt>
        <Txt variant="meta" tone="muted">
          Your password goes to the Akatsuki sync server once over TLS, is encrypted there, and never comes
          back to any device.
        </Txt>
        <Field
          label="Email"
          value={f.email}
          onChangeText={set("email")}
          autoCapitalize="none"
          keyboardType="email-address"
        />
        <Field
          label="Username"
          value={f.username}
          onChangeText={set("username")}
          autoCapitalize="none"
          placeholder="Usually the email address"
        />
        <Field
          label="Password"
          value={f.password}
          onChangeText={set("password")}
          secureTextEntry
          autoCapitalize="none"
          textContentType="password"
        />
        <Field
          label="IMAP server"
          value={f.imapHost}
          onChangeText={set("imapHost")}
          autoCapitalize="none"
          placeholder="imap.example.com"
        />
        <Field
          label="IMAP port"
          value={f.imapPort}
          onChangeText={set("imapPort")}
          keyboardType="number-pad"
        />
        <Field
          label="SMTP server"
          value={f.smtpHost}
          onChangeText={set("smtpHost")}
          autoCapitalize="none"
          placeholder="smtp.example.com"
        />
        <Field
          label="SMTP port"
          value={f.smtpPort}
          onChangeText={set("smtpPort")}
          keyboardType="number-pad"
          error={error}
        />
        <Button
          title="Connect"
          variant="primary"
          busy={busy}
          onPress={submit}
          disabled={!f.email || !f.password || !f.imapHost || !f.smtpHost}
        />
        <Button title="Cancel" variant="ghost" onPress={onClose} />
      </ScrollView>
    </Modal>
  );
}

export default function Settings() {
  const t = useTheme();
  const mail = useMail();
  const updatePrefs = useMutation(api.me.updatePrefs);
  const register = useMutation(api.devices.register);
  const removeAccount = useMutation(api.accounts.remove);
  const removeDevice = useMutation(api.devices.remove);
  const deleteAccount = useMutation(api.me.deleteAccount);
  const devices = useQuery(api.devices.list, {});
  const ai = settingsOrEmpty(useQuery(api.ai.getSettings, {}));
  const [deviceId, setDeviceId] = useState<string | null>(null);
  const [permission, setPermission] = useState<PushPermission>("undetermined");
  const [imap, setImap] = useState(false);
  const [connecting, setConnecting] = useState<string | null>(null);
  const prefs = mail.prefs;

  useEffect(() => {
    getDeviceId().then(setDeviceId);
    pushPermission()
      .then(setPermission)
      .catch(() => setPermission("unavailable"));
  }, []);

  useShortcuts("list", { "nav.back": () => router.back() });

  const patch = (p: Parameters<typeof updatePrefs>[0]["patch"]) =>
    updatePrefs({ patch: p }).catch((err) => showToast({ id: "prefs", text: errorText(err), tone: "error" }));

  const setNotifications = async (mode: NotificationMode) => {
    const apply = async () => {
      await patch({ notifications: mode });
      if (mode === "off" || !deviceId) return;
      const { token, permission: p } = await obtainPushToken().catch(() => ({
        token: null,
        permission: "unavailable" as const,
      }));
      setPermission(p);
      if (token)
        await register({
          deviceId,
          kind: "mobile",
          name: deviceName(),
          appVersion: appVersion(),
          expoPushToken: token,
        });
    };
    if (needsWarning(prefs?.notifications ?? "private", mode))
      Alert.alert(FULL_MODE_WARNING.title, FULL_MODE_WARNING.body, [
        { text: FULL_MODE_WARNING.cancel, style: "cancel" },
        { text: FULL_MODE_WARNING.confirm, onPress: () => void apply() },
      ]);
    else await apply();
  };

  const connect = async (provider: "google" | "microsoft", accountId?: string) => {
    if (!worker) return;
    setConnecting(provider);
    try {
      const { authorizeUrl } = await worker.connectStart(provider, {
        returnTo: CONNECT_RETURN,
        ...(accountId ? { accountId: accountId as Id<"mailAccounts"> } : {}),
      });
      const result = await WebBrowser.openAuthSessionAsync(authorizeUrl, CONNECT_RETURN);
      if (result.type === "success") {
        const url = new URL(result.url);
        const grant = url.searchParams.get("grant");
        if (url.searchParams.get("status") === "pending" && grant) {
          // The worker creates the account only for the user who started the flow (D-395).
          const done = await worker.connectComplete({ grant });
          showToast({
            id: "connect",
            text: done.reconnected ? "Mailbox reconnected." : "Mailbox connected. The first sync is running.",
          });
        } else
          showToast({
            id: "connect",
            text: errorText({
              code: url.searchParams.get("code") ?? "internal",
              message: "",
              retryable: false,
            }),
            tone: "error",
          });
      }
    } catch (err) {
      showToast({ id: "connect", text: errorText(err), tone: "error" });
    } finally {
      setConnecting(null);
    }
  };

  const confirmRemoveAccount = (accountId: string, email: string) =>
    Alert.alert(
      `Remove ${email}?`,
      "Akatsuki stops syncing it and deletes its data here. Mail stays at your provider.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Remove",
          style: "destructive",
          onPress: () =>
            removeAccount({ accountId: accountId as Id<"mailAccounts"> }).catch((err) =>
              showToast({ id: "rm", text: errorText(err), tone: "error" }),
            ),
        },
      ],
    );

  const signOut = () =>
    Alert.alert(
      "Sign out?",
      "Cached mail, drafts saved on this phone and AI keys on this phone are removed.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Sign out",
          style: "destructive",
          onPress: () => signOutEverywhereOnDevice().then(() => router.replace("/sign-in")),
        },
      ],
    );

  const confirmDelete = () =>
    Alert.alert(
      "Delete your Akatsuki account?",
      "This removes your connected mailboxes from Akatsuki, your settings and everything Akatsuki stores for you. Mail at your providers is not touched. This cannot be undone.",
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Delete account",
          style: "destructive",
          onPress: async () => {
            try {
              await deleteAccount({ confirm: "DELETE" });
              await apiClient?.cancel({ immediately: true }).catch(() => {});
              await (authClient as unknown as { deleteUser?: () => Promise<unknown> } | null)
                ?.deleteUser?.()
                .catch(() => {});
              await signOutEverywhereOnDevice();
              router.replace("/sign-in");
            } catch (err) {
              showToast({ id: "delete", text: errorText(err), tone: "error" });
            }
          },
        },
      ],
    );

  const plan = planStatus(mail.me?.entitlement, mail.me?.billing, Date.now(), mail.timeZone);
  const aiDefault = defaultProfile(ai);
  const notifMode = prefs?.notifications ?? "private";
  const permText = permissionText(permission, easProjectId() !== null);

  return (
    <Screen>
      <Header title="Settings" onBack={() => router.back()} />
      <ScrollView contentContainerStyle={{ paddingBottom: t.space[8] }}>
        <Section title="Account">
          <Row
            title={mail.me?.name ?? mail.me?.email ?? "Signed in"}
            detail={mail.me?.name ? mail.me.email : null}
          />
          <Row title="Sign out" onPress={signOut} />
        </Section>

        <Section title="Plan">
          <Row title={plan.title} detail={plan.detail} />
        </Section>

        <Section title="Mailboxes">
          {mail.accounts.map((a) => (
            <Row
              key={a._id}
              title={a.email}
              detail={[
                a.provider === "gmail" ? "Gmail" : a.provider === "graph" ? "Outlook" : "IMAP",
                a.status.replace("_", " "),
              ].join(" · ")}
              right={
                <View style={{ flexDirection: "row" }}>
                  {a.status === "reauth_required" && a.provider !== "imap" ? (
                    <HeaderButton
                      label="Reconnect"
                      accent
                      onPress={() => connect(a.provider === "gmail" ? "google" : "microsoft", a._id)}
                    />
                  ) : null}
                  <HeaderButton label="Remove" onPress={() => confirmRemoveAccount(a._id, a.email)} />
                </View>
              }
            />
          ))}
          {mail.entitled ? (
            <View style={{ padding: t.space[2], gap: t.space[1] }}>
              <Button
                title="Connect Gmail"
                busy={connecting === "google"}
                onPress={() => connect("google")}
              />
              <Button
                title="Connect Outlook"
                busy={connecting === "microsoft"}
                onPress={() => connect("microsoft")}
              />
              <Button title="Connect IMAP" onPress={() => setImap(true)} />
            </View>
          ) : (
            <Row title="Connecting mailboxes needs an active plan" />
          )}
        </Section>

        <Section title="Notifications" footer={permText}>
          {NOTIFICATION_MODES.map((m) => (
            <Choice
              key={m.mode}
              title={m.title}
              detail={m.detail}
              selected={notifMode === m.mode}
              onPress={() => void setNotifications(m.mode)}
            />
          ))}
        </Section>

        <Section title="Appearance">
          {(["system", "light", "dark"] as const).map((th) => (
            <Choice
              key={th}
              title={th === "system" ? "Match the system" : th === "light" ? "Light" : "Dark"}
              selected={(prefs?.theme ?? "system") === th}
              onPress={() => patch({ theme: th })}
            />
          ))}
        </Section>

        <Section title="Keyboard" footer="For iPad and tablets with a hardware keyboard.">
          <Choice
            title="Superhuman keys"
            selected={(prefs?.keymap ?? "superhuman") === "superhuman"}
            onPress={() => patch({ keymap: "superhuman" })}
          />
          <Choice
            title="Gmail keys"
            selected={prefs?.keymap === "gmail"}
            onPress={() => patch({ keymap: "gmail" })}
          />
          <Row title="Keyboard shortcuts" onPress={() => router.push("/settings/shortcuts")} />
        </Section>

        <Section title="Undo send" footer="How long Undo stays available after you press Send.">
          <View style={{ flexDirection: "row", flexWrap: "wrap", gap: t.space[1], padding: t.space[2] }}>
            {UNDO_CHOICES.map((s) => (
              <Chip
                key={s}
                label={s === 0 ? "Off" : `${s} s`}
                selected={(prefs?.undoSendSeconds ?? 10) === s}
                onPress={() => patch({ undoSendSeconds: s })}
              />
            ))}
          </View>
        </Section>

        <Section title="Privacy">
          <Row
            title="Load remote images"
            detail="Images load through the Akatsuki image proxy, so senders do not see your address or location."
            right={
              <Switch
                accessibilityLabel="Load remote images"
                value={prefs?.remoteImages === "proxy"}
                onValueChange={(v) => {
                  void patch({ remoteImages: v ? "proxy" : "block" });
                }}
                trackColor={{ true: t.colors.accent, false: t.colors.border }}
              />
            }
          />
          <Row
            title="Keep messages on this phone"
            detail="Opened and prefetched messages are cached here (up to 300 MB) for speed and offline reading."
            right={
              <Switch
                accessibilityLabel="Keep messages on this phone"
                value={prefs?.cacheBodiesOnDevice ?? true}
                onValueChange={(v) => {
                  void patch({ cacheBodiesOnDevice: v });
                }}
                trackColor={{ true: t.colors.accent, false: t.colors.border }}
              />
            }
          />
        </Section>

        <Section title="AI">
          <Row
            title={aiDefault ? aiDefault.label : "AI is off"}
            detail={
              aiDefault
                ? egressView(aiDefault).text
                : "Choose Akatsuki AI, your own key or a model on your network."
            }
            onPress={() => router.push("/settings/ai")}
          />
        </Section>

        <Section title="Devices">
          {(devices ?? []).map((d) => (
            <Row
              key={d._id}
              title={d.name}
              detail={d.deviceId === deviceId ? "This device" : `${d.kind} · ${d.appVersion}`}
              right={
                d.deviceId === deviceId ? null : (
                  <HeaderButton
                    label="Remove"
                    onPress={() =>
                      removeDevice({ deviceId: d.deviceId }).catch((err) =>
                        showToast({ id: "dev", text: errorText(err), tone: "error" }),
                      )
                    }
                  />
                )
              }
            />
          ))}
        </Section>

        <Section title="About" footer="Akatsuki is open source, free for personal use.">
          <Row title="Version" detail={appVersion()} />
          <Row title="Terms" onPress={() => Linking.openURL("https://akatsuki.vc/terms")} />
          <Row title="Privacy" onPress={() => Linking.openURL("https://akatsuki.vc/privacy")} />
        </Section>

        <Section title="Danger zone">
          <Row title="Delete account" onPress={confirmDelete} />
        </Section>
      </ScrollView>
      <ImapForm visible={imap} onClose={() => setImap(false)} />
    </Screen>
  );
}
