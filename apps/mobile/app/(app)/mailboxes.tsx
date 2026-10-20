// Mailbox switcher (sheet): system views, reminders, user labels, settings.
import { formatKeys, getShortcut, keysFor } from "@akatsuki/shortcuts";
import { router } from "expo-router";
import { ScrollView } from "react-native";
import { Row, Section, Txt } from "../../src/components/ui";
import { useKeyboard, useShortcuts } from "../../src/state/keyboard";
import { useMail } from "../../src/state/mail";
import { useTheme } from "../../src/theme/ThemeProvider";
import { MAILBOXES } from "../../src/vm/threadList";

export default function Mailboxes() {
  const t = useTheme();
  const mail = useMail();
  const keyboard = useKeyboard();
  const hint = (id: string) => {
    const k = keysFor(getShortcut(id), keyboard.platform, keyboard.keymap)[0];
    return k ? (
      <Txt variant="code" tone="muted">
        {formatKeys(k, keyboard.platform)}
      </Txt>
    ) : null;
  };
  useShortcuts("palette", { "palette.close": () => router.back() });
  const userLabels = mail.labels
    .filter((l) => l.labelKey.startsWith("l:"))
    .sort((a, b) => a.name.localeCompare(b.name));
  const multi = mail.accounts.length > 1;

  return (
    <ScrollView
      style={{ flex: 1, backgroundColor: t.colors.background }}
      contentContainerStyle={{ paddingBottom: t.space[4] }}
    >
      <Section title="Mailboxes">
        {MAILBOXES.map((m) => (
          <Row
            key={m.id}
            title={m.title}
            right={hint(m.shortcut)}
            onPress={() => {
              router.back();
              router.navigate({ pathname: "/inbox", params: { mailbox: m.id } });
            }}
          />
        ))}
        <Row
          title="Reminders"
          right={hint("go.reminders")}
          onPress={() => {
            router.back();
            router.push("/reminders");
          }}
        />
      </Section>
      {userLabels.length > 0 ? (
        <Section title="Labels">
          {userLabels.map((l) => (
            <Row
              key={l._id}
              title={l.name}
              detail={multi ? (mail.accountById.get(l.accountId)?.email ?? null) : null}
              onPress={() => {
                router.back();
                router.navigate({ pathname: "/inbox", params: { labelKey: l.labelKey, title: l.name } });
              }}
            />
          ))}
        </Section>
      ) : null}
      <Section title="Akatsuki">
        <Row
          title="Settings"
          right={hint("settings.open")}
          onPress={() => {
            router.back();
            router.push("/settings");
          }}
        />
        <Row
          title="Keyboard shortcuts"
          right={hint("help.shortcuts")}
          onPress={() => {
            router.back();
            router.push("/settings/shortcuts");
          }}
        />
      </Section>
    </ScrollView>
  );
}
