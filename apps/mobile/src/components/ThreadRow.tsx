// One conversation in the list, with swipe actions: right = archive (or restore), left = snooze, long left =
// trash. The drag runs on the UI thread (Reanimated worklets); the action and haptics run on the JS thread once a
// threshold is crossed or the finger lifts. The thresholds and the action choice come from vm/swipe.ts.
import { memo, useCallback, useEffect, useState } from "react";
import { type LayoutChangeEvent, Pressable, StyleSheet, View } from "react-native";
import { GestureDetector, usePanGesture } from "react-native-gesture-handler";
import Animated, { interpolate, useAnimatedStyle, useSharedValue, withTiming } from "react-native-reanimated";
import { scheduleOnRN } from "react-native-worklets";
import * as haptics from "../lib/haptics";
import { useTheme } from "../theme/ThemeProvider";
import { SWIPE_LONG, SWIPE_SHORT, type SwipeAction, type SwipeSides, swipeResult } from "../vm/swipe";
import type { ThreadRowModel } from "../vm/threadList";
import { Txt } from "./ui";

export interface ThreadRowProps {
  model: ThreadRowModel;
  sides: SwipeSides;
  selected: boolean;
  onOpen: (id: string) => void;
  onSwipe: (id: string, action: SwipeAction) => void;
  onLongPress?: (id: string) => void;
}

function ThreadRowImpl({ model, sides, selected, onOpen, onSwipe, onLongPress }: ThreadRowProps) {
  const t = useTheme();
  const [width, setWidth] = useState(0);
  const x = useSharedValue(0);
  const stage = useSharedValue(0);
  const hasLeading = sides.leading !== null;
  const hasShort = sides.trailing[0] !== null;
  const hasLong = sides.trailing[1] !== null;

  const release = useCallback(
    (dx: number) => {
      const action = swipeResult(dx, width, sides);
      if (action) onSwipe(model.id, action);
      // Rows that stay in this view (or a failed action) slide back.
      if (!action?.removes) x.value = withTiming(0, { duration: 200 });
    },
    [width, sides, onSwipe, model.id, x],
  );

  // FlashList recycles row components: a reused row starts centred.
  // biome-ignore lint/correctness/useExhaustiveDependencies: model.id is the trigger (a recycled row shows another thread)
  useEffect(() => {
    x.value = 0;
    stage.value = 0;
  }, [model.id, x, stage]);

  const pan = usePanGesture({
    activeOffsetX: [-14, 14],
    failOffsetY: [-12, 12],
    onUpdate: (e) => {
      "worklet";
      const dx = e.translationX;
      x.value = dx > 0 && !hasLeading ? 0 : dx < 0 && !hasShort ? 0 : dx;
      const f = width > 0 ? Math.abs(x.value) / width : 0;
      const next =
        x.value > 0 ? (f >= SWIPE_SHORT ? 1 : 0) : f >= SWIPE_LONG && hasLong ? 2 : f >= SWIPE_SHORT ? 1 : 0;
      if (next !== stage.value) {
        stage.value = next;
        scheduleOnRN(haptics.tick);
      }
    },
    onDeactivate: () => {
      "worklet";
      const dx = x.value;
      const armed = stage.value > 0;
      stage.value = 0;
      if (armed) {
        x.value = withTiming(dx > 0 ? width : -width, { duration: 160 });
        scheduleOnRN(release, dx);
      } else {
        x.value = withTiming(0, { duration: 160 });
      }
    },
  });

  const rowStyle = useAnimatedStyle(() => ({ transform: [{ translateX: x.value }] }));
  const leadStyle = useAnimatedStyle(() => ({
    opacity: interpolate(x.value, [0, width * SWIPE_SHORT], [0.35, 1], "clamp"),
  }));
  const trailStyle = useAnimatedStyle(() => ({
    opacity: interpolate(-x.value, [0, width * SWIPE_SHORT], [0.35, 1], "clamp"),
  }));
  const longStyle = useAnimatedStyle(() => ({ opacity: -x.value >= width * SWIPE_LONG ? 1 : 0 }));

  // A swiped-away row snaps back if the action did not remove it (e.g. "Move to Inbox" in All mail).
  const onLayout = useCallback(
    (e: LayoutChangeEvent) => {
      setWidth(e.nativeEvent.layout.width);
      x.value = 0;
    },
    [x],
  );

  const actions = [sides.leading, ...sides.trailing].filter(Boolean).map((a) => ({
    name: a!.id,
    label: a!.label,
  }));

  return (
    <View onLayout={onLayout} style={{ backgroundColor: t.colors.background }}>
      <View
        style={[StyleSheet.absoluteFill, { flexDirection: "row" }]}
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
      >
        <Animated.View
          style={[
            { flex: 1, backgroundColor: t.colors.accent, justifyContent: "center", paddingLeft: t.space[2] },
            leadStyle,
          ]}
        >
          {sides.leading ? (
            <Txt variant="label" tone="onAccent">
              {sides.leading.label}
            </Txt>
          ) : null}
        </Animated.View>
        <Animated.View
          style={[
            {
              flex: 1,
              backgroundColor: t.colors.foreground,
              alignItems: "flex-end",
              justifyContent: "center",
              paddingRight: t.space[2],
            },
            trailStyle,
          ]}
        >
          <View>
            {sides.trailing[0] ? (
              <Txt variant="label" style={{ color: t.colors.background }}>
                {sides.trailing[0].label}
              </Txt>
            ) : null}
            {sides.trailing[1] ? (
              <Animated.View
                style={[StyleSheet.absoluteFill, { backgroundColor: t.colors.foreground }, longStyle]}
              >
                <Txt variant="label" style={{ color: t.colors.background }}>
                  {sides.trailing[1].label}
                </Txt>
              </Animated.View>
            ) : null}
          </View>
        </Animated.View>
      </View>
      <GestureDetector gesture={pan}>
        <Animated.View style={[{ backgroundColor: t.colors.background }, rowStyle]}>
          <Pressable
            onPress={() => onOpen(model.id)}
            onLongPress={onLongPress ? () => onLongPress(model.id) : undefined}
            accessibilityRole="button"
            accessibilityLabel={model.accessibilityLabel}
            accessibilityHint="Opens the conversation"
            accessibilityActions={actions}
            onAccessibilityAction={(e) => {
              const a = [sides.leading, ...sides.trailing].find((s) => s?.id === e.nativeEvent.actionName);
              if (a) onSwipe(model.id, a);
            }}
            style={({ pressed }) => ({
              flexDirection: "row",
              paddingVertical: t.space["1_5"],
              paddingRight: t.space[2],
              paddingLeft: t.space["1_5"],
              gap: t.space[1],
              backgroundColor: pressed || selected ? t.colors.selected : t.colors.background,
              borderBottomWidth: StyleSheet.hairlineWidth,
              borderBottomColor: t.colors.borderSubtle,
            })}
          >
            <View style={{ width: 8, paddingTop: 7, alignItems: "center" }}>
              {model.unread ? (
                <View style={{ width: 7, height: 7, borderRadius: 4, backgroundColor: t.colors.accent }} />
              ) : null}
            </View>
            <View style={{ flex: 1, gap: 2, opacity: model.pending ? 0.85 : 1 }}>
              <View style={{ flexDirection: "row", alignItems: "baseline", gap: t.space[1] }}>
                <Txt variant={model.unread ? "bodyStrong" : "body"} numberOfLines={1} style={{ flex: 1 }}>
                  {model.sender}
                </Txt>
                {model.starred ? (
                  <Txt variant="caption" tone="accent" accessibilityElementsHidden>
                    ★
                  </Txt>
                ) : null}
                <Txt variant="caption" tone={model.unread ? "text" : "muted"}>
                  {model.date}
                </Txt>
              </View>
              <Txt
                variant={model.unread ? "bodyStrong" : "body"}
                tone={model.unread ? "text" : "secondary"}
                numberOfLines={1}
              >
                {model.subject}
              </Txt>
              <Txt variant="meta" tone="muted" numberOfLines={2}>
                {model.badge ? `${model.badge} · ` : ""}
                {model.hasDraft ? "Draft · " : ""}
                {model.needsReply ? "Needs reply · " : ""}
                {model.hasAttachments ? "Attachment · " : ""}
                {model.snippet}
              </Txt>
            </View>
          </Pressable>
        </Animated.View>
      </GestureDetector>
    </View>
  );
}

export const ThreadRow = memo(ThreadRowImpl);
