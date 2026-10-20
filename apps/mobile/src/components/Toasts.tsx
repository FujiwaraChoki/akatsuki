// Toast host (bottom of the screen) and the op-failure watcher (§5.7): every failed or unknown op from
// `ops.failures` is shown once, keyed by opId, with Retry (`ops.retry`) and Dismiss (`ops.ackFailure`).
import { api } from "@akatsuki/convex";
import { useMutation, useQuery } from "convex/react";
import { useEffect, useRef } from "react";
import { Pressable, View } from "react-native";
import Animated, { FadeIn, FadeInDown, FadeOut, useReducedMotion } from "react-native-reanimated";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { describeOpFailure, errorText } from "../lib/errors";
import { newClientId } from "../lib/ids";
import { dismissToast, showToast, useToasts } from "../state/toasts";
import { useTheme } from "../theme/ThemeProvider";
import { Icon } from "./Icon";
import { Txt } from "./ui";

export function ToastHost() {
  const t = useTheme();
  const toasts = useToasts();
  const insets = useSafeAreaInsets();
  const reduced = useReducedMotion();
  if (toasts.length === 0) return null;
  const quick = t.motion.duration.quick;
  return (
    <View
      pointerEvents="box-none"
      style={{
        position: "absolute",
        left: 0,
        right: 0,
        // 12 above the home indicator, or above a screen's toolbar (about 56pt).
        bottom: insets.bottom + 64,
        alignItems: "center",
        gap: t.space[1],
        paddingHorizontal: t.space[2],
      }}
    >
      {toasts.map((toast) => (
        <Animated.View
          key={toast.id}
          entering={reduced ? FadeIn.duration(80) : FadeInDown.duration(quick)}
          exiting={FadeOut.duration(80)}
          accessibilityLiveRegion="polite"
          accessibilityRole="alert"
          style={{
            maxWidth: 560,
            width: "100%",
            flexDirection: "row",
            alignItems: "center",
            gap: t.space[1],
            paddingLeft: toast.tone === "error" ? t.space["1_5"] : t.space[2],
            paddingRight: 6,
            minHeight: 44,
            borderRadius: t.radius.full,
            backgroundColor: t.colors.foreground,
            boxShadow: t.shadow.pop,
          }}
        >
          {toast.tone === "error" ? <Icon name="alert-circle" size={16} color={t.colors.background} /> : null}
          <Txt variant="body" tone="inverse" style={{ flex: 1, fontSize: 14 }} numberOfLines={3}>
            {toast.text}
          </Txt>
          {(toast.actions ?? []).map((a) => (
            <Pressable
              key={a.label}
              accessibilityRole="button"
              onPress={a.run}
              hitSlop={8}
              style={{ paddingHorizontal: t.space[1], minHeight: 44, justifyContent: "center" }}
            >
              <Txt variant="bodyStrong" tone="inverse" style={{ fontSize: 14 }}>
                {a.label}
              </Txt>
            </Pressable>
          ))}
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Dismiss"
            onPress={() => dismissToast(toast.id)}
            hitSlop={8}
            style={{ paddingHorizontal: t.space[1], minHeight: 44, justifyContent: "center" }}
          >
            <Icon name="close" size={16} color={t.colors.background} />
          </Pressable>
        </Animated.View>
      ))}
    </View>
  );
}

const VERBS: Record<string, string> = {
  thread: "update",
  send: "send",
  delete_draft: "discard the draft",
  label_create: "create the label",
  label_rename: "rename the label",
  label_delete: "delete the label",
};

export function FailureWatcher() {
  const failures = useQuery(api.ops.failures, {});
  const ack = useMutation(api.ops.ackFailure);
  const retry = useMutation(api.ops.retry);
  const shown = useRef(new Set<string>());
  useEffect(() => {
    for (const f of failures ?? []) {
      if (shown.current.has(f._id)) continue;
      shown.current.add(f._id);
      const id = `op:${f._id}`;
      const text =
        f.state === "unknown"
          ? "We could not confirm this was sent. Check Sent, then send again or discard."
          : describeOpFailure({
              verb: VERBS[f.payloadKind] ?? "update",
              subject: f.subject,
              account: f.accountEmail,
              code: f.error.code,
            });
      const actions = [
        ...(f.error.retryable && f.payloadKind !== "send"
          ? [
              {
                label: "Retry",
                run: () => {
                  dismissToast(id);
                  retry({ opId: f._id, clientOpId: newClientId() }).catch((err) =>
                    showToast({ id: `${id}:retry`, text: errorText(err), tone: "error" }),
                  );
                },
              },
            ]
          : []),
        {
          label: "Dismiss",
          run: () => {
            dismissToast(id);
            ack({ opIds: [f._id] }).catch(() => {});
          },
        },
      ];
      showToast({ id, text, tone: "error", durationMs: 0, actions });
    }
  }, [failures, ack, retry]);
  return null;
}
