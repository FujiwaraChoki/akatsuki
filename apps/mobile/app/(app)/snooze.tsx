// Snooze picker (§10 `mail.snooze`) and "Remind me if no reply" (`mail.setReminder`), as a sheet.
import { router, useLocalSearchParams } from "expo-router";
import { useState } from "react";
import { ScrollView, View } from "react-native";
import { TimePicker } from "../../src/components/TimePicker";
import { Txt } from "../../src/components/ui";
import { useThreadActions } from "../../src/state/actions";
import { useShortcuts } from "../../src/state/keyboard";
import { useMail } from "../../src/state/mail";
import { finishPickerTargets, pickerTargets } from "../../src/state/targets";
import { useTheme } from "../../src/theme/ThemeProvider";
import { snoozePresets } from "../../src/vm/snooze";

export default function SnoozeSheet() {
  const t = useTheme();
  const mail = useMail();
  const actions = useThreadActions();
  const { threadId, accountId, mode, from, bulk } = useLocalSearchParams<{
    threadId: string;
    accountId: string;
    mode?: string;
    from?: string;
    bulk?: string;
  }>();
  // A multi-select (state/targets.ts): snooze only; reminders are per conversation.
  const [targets] = useState(() => (bulk === "1" ? pickerTargets() : []));
  const remind = mode === "remind" && targets.length === 0;
  const presets = snoozePresets(Date.now(), mail.timeZone).filter((p) => !remind || p.id !== "later_today");
  const thread = { _id: threadId, accountId };

  const pick = (at: number) => {
    if (remind) {
      actions.remind(thread, at);
      router.back();
    } else if (targets.length > 0) {
      actions.snooze(targets, at);
      finishPickerTargets();
      router.back();
    } else {
      actions.snooze([thread], at);
      // From a conversation, leave the sheet and the conversation: the thread has left the inbox.
      if (from === "thread") router.dismiss(2);
      else router.back();
    }
  };

  useShortcuts("palette", { "palette.close": () => router.back() });

  return (
    <ScrollView
      style={{ flex: 1, backgroundColor: t.colors.background }}
      contentContainerStyle={{ paddingVertical: t.space[2] }}
    >
      <View style={{ paddingHorizontal: t.space[2], paddingBottom: t.space[1] }}>
        <Txt variant="title">
          {remind
            ? "Remind me"
            : targets.length > 1
              ? `Snooze ${targets.length} conversations until`
              : "Snooze until"}
        </Txt>
        {remind ? (
          <Txt variant="meta" tone="muted">
            The conversation comes back to the inbox at that time unless someone replies first.
          </Txt>
        ) : null}
      </View>
      <TimePicker
        presets={presets}
        timeZone={mail.timeZone}
        confirmLabel={remind ? "Set reminder" : "Snooze"}
        onPick={pick}
      />
    </ScrollView>
  );
}
