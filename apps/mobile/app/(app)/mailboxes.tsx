// Mailbox switcher (sheet): system views, reminders, user labels, settings.
import { formatKeys, getShortcut, keysFor } from "@akatsuki/shortcuts";
import { router } from "expo-router";
import { ScrollView, useWindowDimensions } from "react-native";
import { Icon, type IconName } from "../../src/components/Icon";
import { Row, Section, showKeyHints, Txt } from "../../src/components/ui";
import { useKeyboard, useShortcuts } from "../../src/state/keyboard";
import { useMail } from "../../src/state/mail";
import { useTheme } from "../../src/theme/ThemeProvider";
import { MAILBOXES, type MailboxId } from "../../src/vm/threadList";

const MAILBOX_ICON: Record<MailboxId, IconName> = {
  inbox: "inbox",
  starred: "star",
  snoozed: "clock",
  sent: "send",
  drafts: "file",
  all: "all",
  spam: "octagon",
  trash: "trash",
};

export default function Mailboxes() {
  const t = useTheme();
  const { width } = useWindowDimensions();
  const mail = useMail();
  const keyboard = useKeyboard();
  const hint = (id: string) => {
    if (!showKeyHints(width)) return null;
    const k = keysFor(getShortcut(id), keyboard.platform, keyboard.keymap)[0];
    return k ? (
      <Txt variant="code" tone="muted">
        {formatKeys(k, keyboard.platform)}
      </Txt>
    ) : null;
  };
  useShortcuts("palette", { "palette.close": () => router.back() });
  const userLabels = mail.labels
    .filter((l) => l.labelKey.startsWith("l:") && !l.role && !l.hidden && !l.name.startsWith("Akatsuki/"))
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
            icon={<Icon name={MAILBOX_ICON[m.id]} color={t.colors.muted} />}
            title={m.title}
            right={hint(m.shortcut)}
            onPress={() => {
              router.back();
              router.navigate({ pathname: "/inbox", params: { mailbox: m.id } });
            }}
          />
        ))}
        <Row
          icon={<Icon name="bell" color={t.colors.muted} />}
          title="Reminders"
          right={hint("go.reminders")}
          onPress={() => {
            router.back();
            router.push("/reminders");
          }}
        />
        <Row
          icon={<Icon name="send" color={t.colors.muted} />}
          title="Scheduled"
          onPress={() => {
            router.back();
            router.push("/scheduled");
          }}
        />
      </Section>
      {userLabels.length > 0 ? (
        <Section title="Labels">
          {userLabels.map((l) => (
            <Row
              key={l._id}
              icon={<Icon name="tag" color={t.colors.muted} />}
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
