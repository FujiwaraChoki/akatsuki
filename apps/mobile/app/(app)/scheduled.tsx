// Scheduled messages (§4.2 `sends.list` / `sends.reschedule` / `sends.cancel` / `sends.resolveUnknown`, §10.2,
// §5.6; web D-283 parity): later sends with Change time and Cancel, failed sends, and sends whose outcome is
// unknown. The message text stays in the provider's Drafts folder; nothing here reads it.

import { api } from "@akatsuki/convex";
import { useMutation, useQuery } from "convex/react";
import { router } from "expo-router";
import { useState } from "react";
import { Pressable, ScrollView, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { EmptyState } from "../../src/components/EmptyState";
import { Header } from "../../src/components/Header";
import { SheetModal } from "../../src/components/SheetModal";
import { TimePicker } from "../../src/components/TimePicker";
import { Button, Screen, Spinner, Txt } from "../../src/components/ui";
import { errorText } from "../../src/lib/errors";
import { useShortcuts } from "../../src/state/keyboard";
import { useMail } from "../../src/state/mail";
import { showToast } from "../../src/state/toasts";
import { useTheme } from "../../src/theme/ThemeProvider";
import {
  SCHEDULED_DONE,
  SCHEDULED_STATES,
  type ScheduledAction,
  scheduledRows,
} from "../../src/vm/scheduled";
import { describeTime, sendLaterPresets } from "../../src/vm/snooze";

export default function Scheduled() {
  const t = useTheme();
  const insets = useSafeAreaInsets();
  const mail = useMail();
  const sends = useQuery(api.sends.list, mail.entitled ? { states: [...SCHEDULED_STATES] } : "skip");
  const cancel = useMutation(api.sends.cancel);
  const reschedule = useMutation(api.sends.reschedule);
  const resolve = useMutation(api.sends.resolveUnknown);
  const [changing, setChanging] = useState<string | null>(null);
  useShortcuts("list", { "nav.back": () => router.back() });

  const rows = scheduledRows(sends ?? [], (at) => describeTime(at, Date.now(), mail.timeZone));

  const act = (clientSendId: string, action: ScheduledAction, sendAt?: number) => {
    const p =
      action === "cancel"
        ? cancel({ clientSendId }).then((r) => {
            if (r.result === "too_late") throw new Error("too_late");
          })
        : action === "reschedule"
          ? reschedule({ clientSendId, sendAt: sendAt! })
          : resolve({ clientSendId, resolution: action === "was_sent" ? "was_sent" : "discard" });
    p.then(() => showToast({ id: `sched:${clientSendId}`, text: SCHEDULED_DONE[action] })).catch((err) =>
      showToast({
        id: `sched:${clientSendId}`,
        text:
          err instanceof Error && err.message === "too_late"
            ? "Too late: it is already being sent."
            : errorText(err),
        tone: "error",
      }),
    );
  };

  return (
    <Screen>
      <Header title="Scheduled" onBack={() => router.back()} />
      {sends === undefined ? (
        <View style={{ padding: t.space[4], alignItems: "center" }}>
          <Spinner />
        </View>
      ) : rows.length === 0 ? (
        <EmptyState
          title="Nothing scheduled"
          body="Use Send later in the composer to send a message at a set time."
        />
      ) : (
        <ScrollView contentContainerStyle={{ paddingBottom: t.space[8] }}>
          {rows.map((r) => (
            <View
              key={r.id}
              style={{
                paddingHorizontal: t.space[2],
                paddingVertical: t.space["1_5"],
                gap: t.space["0_5"],
                borderBottomWidth: 1,
                borderBottomColor: t.colors.borderSubtle,
              }}
            >
              <Txt variant="bodyStrong" numberOfLines={1}>
                {r.subject}
              </Txt>
              <Txt variant="meta" tone="muted" numberOfLines={1}>
                To {r.to}
              </Txt>
              <Txt variant="meta" tone={r.tone === "error" ? "error" : "secondary"}>
                {r.status}
              </Txt>
              {r.actions.length > 0 ? (
                <View style={{ flexDirection: "row", gap: t.space[1], marginTop: t.space["0_5"] }}>
                  {r.actions.map((a) => (
                    <Button
                      key={a.id}
                      title={a.label}
                      compact
                      variant={a.id === "cancel" || a.id === "not_sent" ? "ghost" : "secondary"}
                      onPress={() =>
                        a.id === "reschedule" ? setChanging(r.clientSendId) : act(r.clientSendId, a.id)
                      }
                    />
                  ))}
                </View>
              ) : null}
            </View>
          ))}
        </ScrollView>
      )}
      <SheetModal visible={changing !== null} transparent onRequestClose={() => setChanging(null)}>
        <Pressable
          accessibilityLabel="Close"
          style={{ flex: 1, backgroundColor: t.colors.scrim }}
          onPress={() => setChanging(null)}
        />
        <View
          style={{
            backgroundColor: t.colors.background,
            borderTopLeftRadius: t.radius.xl,
            borderTopRightRadius: t.radius.xl,
            paddingTop: t.space[2],
            paddingBottom: insets.bottom + t.space[2],
          }}
        >
          <Txt variant="title" style={{ paddingHorizontal: t.space[2], paddingBottom: t.space[1] }}>
            Send at
          </Txt>
          <TimePicker
            presets={sendLaterPresets(Date.now(), mail.timeZone)}
            timeZone={mail.timeZone}
            confirmLabel="Change time"
            onPick={(at) => {
              const id = changing;
              setChanging(null);
              if (id) act(id, "reschedule", at);
            }}
          />
        </View>
      </SheetModal>
    </Screen>
  );
}
