// Active reminders (`mail.listTimers({kind:"reminder"})`), soonest first.

import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import { useMutation, useQuery } from "convex/react";
import { router } from "expo-router";
import { FlatList, View } from "react-native";
import { EmptyState } from "../../src/components/EmptyState";
import { Header, HeaderButton } from "../../src/components/Header";
import { openThread } from "../../src/components/ThreadList";
import { Row, Screen, Spinner } from "../../src/components/ui";
import { errorText } from "../../src/lib/errors";
import { useShortcuts } from "../../src/state/keyboard";
import { useMail } from "../../src/state/mail";
import { showToast } from "../../src/state/toasts";
import { describeTime } from "../../src/vm/snooze";

function ReminderRow({
  timerId,
  threadId,
  fireAt,
  note,
}: {
  timerId: string;
  threadId: string;
  fireAt: number;
  note?: string;
}) {
  const mail = useMail();
  const thread = useQuery(api.mail.getThread, { threadId: threadId as Id<"threads"> });
  const cancel = useMutation(api.mail.cancelReminder);
  return (
    <Row
      title={thread?.thread.subject || "(no subject)"}
      detail={[describeTime(fireAt, Date.now(), mail.timeZone), note].filter(Boolean).join(" · ")}
      onPress={() => openThread(threadId)}
      right={
        <HeaderButton
          label="Cancel"
          onPress={() =>
            cancel({ timerId: timerId as Id<"timers"> }).catch((err) =>
              showToast({ id: `rem:${timerId}`, text: errorText(err), tone: "error" }),
            )
          }
        />
      }
    />
  );
}

export default function Reminders() {
  const mail = useMail();
  const timers = useQuery(api.mail.listTimers, mail.entitled ? { kind: "reminder" } : "skip");
  useShortcuts("list", { "nav.back": () => router.back() });
  return (
    <Screen>
      <Header title="Reminders" onBack={() => router.back()} />
      {timers === undefined ? (
        <View style={{ padding: 32, alignItems: "center" }}>
          <Spinner />
        </View>
      ) : (
        <FlatList
          data={[...timers].sort((a, b) => a.fireAt - b.fireAt)}
          keyExtractor={(x) => x._id}
          renderItem={({ item }) => (
            <ReminderRow timerId={item._id} threadId={item.threadId} fireAt={item.fireAt} note={item.note} />
          )}
          ListEmptyComponent={
            <EmptyState
              title="No reminders"
              body="Press Remind me on a conversation to bring it back if nobody replies."
            />
          }
        />
      )}
    </Screen>
  );
}
