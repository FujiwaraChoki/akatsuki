// One conversation in the list, with swipe actions: right = archive (or restore), left = snooze, long left =
// trash. A long press starts multi-select; while selecting, taps toggle the row and swipes are off. The drag runs on the UI thread (Reanimated worklets); the action and haptics run on the JS thread once a
// threshold is crossed or the finger lifts. The thresholds and the action choice come from vm/swipe.ts.
import { memo, useCallback, useEffect, useState } from "react";
import { type LayoutChangeEvent, Pressable, StyleSheet, View } from "react-native";
import { GestureDetector, usePanGesture } from "react-native-gesture-handler";
import Animated, {
  interpolate,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withSpring,
  withTiming,
} from "react-native-reanimated";
import { scheduleOnRN } from "react-native-worklets";
import * as haptics from "../lib/haptics";
import { useTheme } from "../theme/ThemeProvider";
import {
  SWIPE_LONG,
  SWIPE_SHORT,
  type SwipeAction,
  type SwipeActionId,
  type SwipeSides,
  swipeResult,
} from "../vm/swipe";
import type { ThreadRowModel } from "../vm/threadList";
import { Icon, type IconName } from "./Icon";
import { Txt } from "./ui";

const SWIPE_ICON: Record<SwipeActionId, IconName> = {
  archive: "archive",
  moveToInbox: "inbox",
  untrash: "undo",
  notSpam: "inbox",
  snooze: "clock",
  trash: "trash",
};

/** Height of a row (spec §2.2 `layout.rowHeightTouch`). */
export const ROW_HEIGHT = 64;

export interface ThreadRowProps {
  model: ThreadRowModel;
  sides: SwipeSides;
  selected: boolean;
  onOpen: (id: string) => void;
  onSwipe: (id: string, action: SwipeAction) => void;
  onLongPress?: (id: string) => void;
  /** The list is in selection mode. */
  selecting?: boolean;
  /** This row is in the selection. */
  checked?: boolean;
  onToggle?: (id: string) => void;
}

function ThreadRowImpl({
  model,
  sides,
  selected,
  onOpen,
  onSwipe,
  onLongPress,
  selecting = false,
  checked = false,
  onToggle,
}: ThreadRowProps) {
  const t = useTheme();
  const reduced = useReducedMotion();
  const [width, setWidth] = useState(0);
  const x = useSharedValue(0);
  const stage = useSharedValue(0);
  const hasLeading = sides.leading !== null;
  const hasShort = sides.trailing[0] !== null;
  const hasLong = sides.trailing[1] !== null;

  const release = useCallback(
    (dx: number) => {
      const action = swipeResult(dx, width, sides);
      if (action) {
        haptics.impact();
        onSwipe(model.id, action);
      }
      // Rows that stay in this view (or a failed action) spring back.
      if (!action?.removes) x.value = reduced ? 0 : withSpring(0, { damping: 20, stiffness: 300 });
    },
    [width, sides, onSwipe, model.id, x, reduced],
  );

  // FlashList recycles row components: a reused row starts centred.
  // biome-ignore lint/correctness/useExhaustiveDependencies: model.id is the trigger (a recycled row shows another thread)
  useEffect(() => {
    x.value = 0;
    stage.value = 0;
  }, [model.id, x, stage]);

  const pan = usePanGesture({
    enabled: !selecting,
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
        // Reduced motion: no slide-out; the row simply goes (spec §4.3).
        x.value = reduced
          ? dx > 0
            ? width
            : -width
          : withTiming(dx > 0 ? width : -width, { duration: 160 });
        scheduleOnRN(release, dx);
      } else {
        x.value = reduced ? 0 : withSpring(0, { damping: 20, stiffness: 300 });
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
    <View
      onLayout={onLayout}
      style={{
        backgroundColor: t.colors.background,
        borderBottomWidth: StyleSheet.hairlineWidth,
        borderBottomColor: t.colors.borderSubtle,
        marginLeft: 0,
      }}
    >
      <View
        style={[StyleSheet.absoluteFill, { flexDirection: "row" }]}
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
      >
        {/* Right: ink (archive, restore). Left: neutral (snooze); a long left swipe turns accent (Trash). */}
        <Animated.View
          style={[
            {
              flex: 1,
              backgroundColor: t.colors.foreground,
              flexDirection: "row",
              alignItems: "center",
              gap: t.space[1],
              paddingLeft: 20,
            },
            leadStyle,
          ]}
        >
          {sides.leading ? (
            <>
              <Icon name={SWIPE_ICON[sides.leading.id]} color={t.colors.background} />
              <Txt variant="bodyStrong" tone="inverse">
                {sides.leading.label}
              </Txt>
            </>
          ) : null}
        </Animated.View>
        <Animated.View
          style={[
            {
              flex: 1,
              backgroundColor: t.colors.surfaceStrong,
              flexDirection: "row",
              alignItems: "center",
              justifyContent: "flex-end",
              gap: t.space[1],
              paddingRight: 20,
            },
            trailStyle,
          ]}
        >
          {sides.trailing[0] ? (
            <>
              <Txt variant="bodyStrong">{sides.trailing[0].label}</Txt>
              <Icon name={SWIPE_ICON[sides.trailing[0].id]} />
            </>
          ) : null}
          {sides.trailing[1] ? (
            <Animated.View
              style={[
                StyleSheet.absoluteFill,
                {
                  backgroundColor: t.colors.accent,
                  flexDirection: "row",
                  alignItems: "center",
                  justifyContent: "flex-end",
                  gap: t.space[1],
                  paddingRight: 20,
                },
                longStyle,
              ]}
            >
              <Txt variant="bodyStrong" tone="onAccent">
                {sides.trailing[1].label}
              </Txt>
              <Icon name={SWIPE_ICON[sides.trailing[1].id]} color={t.colors.onAccent} />
            </Animated.View>
          ) : null}
        </Animated.View>
      </View>
      <GestureDetector gesture={pan}>
        <Animated.View style={[{ backgroundColor: t.colors.background }, rowStyle]}>
          <Pressable
            onPress={() => (selecting && onToggle ? onToggle(model.id) : onOpen(model.id))}
            onLongPress={onLongPress ? () => onLongPress(model.id) : undefined}
            delayLongPress={350}
            accessibilityRole={selecting ? "checkbox" : "button"}
            accessibilityState={selecting ? { checked } : undefined}
            accessibilityLabel={model.accessibilityLabel}
            accessibilityHint={selecting ? "Adds or removes it from the selection" : "Opens the conversation"}
            accessibilityActions={
              selecting ? [] : [...actions, ...(onLongPress ? [{ name: "select", label: "Select" }] : [])]
            }
            onAccessibilityAction={(e) => {
              if (e.nativeEvent.actionName === "select") return onLongPress?.(model.id);
              const a = [sides.leading, ...sides.trailing].find((s) => s?.id === e.nativeEvent.actionName);
              if (a) onSwipe(model.id, a);
            }}
            style={({ pressed }) => ({
              height: ROW_HEIGHT,
              flexDirection: "row",
              alignItems: "center",
              paddingRight: t.space[2],
              paddingLeft: 10,
              backgroundColor: checked || pressed || selected ? t.colors.surfaceStrong : t.colors.background,
            })}
          >
            {/* The keyboard cursor (iPad): a 2pt dawn bar on the left edge (spec §4.2). */}
            {selected ? (
              <View
                style={{
                  position: "absolute",
                  left: 0,
                  top: 0,
                  bottom: 0,
                  width: 2,
                  backgroundColor: t.colors.dawn,
                }}
              />
            ) : null}
            {selecting ? (
              <View style={{ width: 30, alignItems: "flex-start" }} accessibilityElementsHidden>
                <View
                  style={{
                    width: 20,
                    height: 20,
                    borderRadius: 10,
                    borderWidth: 1.5,
                    borderColor: checked ? t.colors.foreground : t.colors.border,
                    backgroundColor: checked ? t.colors.foreground : "transparent",
                    alignItems: "center",
                    justifyContent: "center",
                  }}
                >
                  {checked ? (
                    <Icon name="check" size={14} color={t.colors.background} strokeWidth={2.25} />
                  ) : null}
                </View>
              </View>
            ) : (
              <View style={{ width: 14, alignItems: "flex-start", alignSelf: "flex-start", paddingTop: 20 }}>
                {model.unread ? (
                  <View
                    style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: t.colors.foreground }}
                  />
                ) : null}
              </View>
            )}
            <View style={{ flex: 1, gap: 2, opacity: model.pending ? 0.8 : 1 }}>
              <View style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
                <Txt
                  variant={model.unread ? "unread" : "body"}
                  tone={model.unread ? "text" : "secondary"}
                  numberOfLines={1}
                  style={{ flexShrink: 1 }}
                >
                  {model.sender}
                </Txt>
                {model.hasDraft ? <RowChip text="Draft" /> : null}
                {model.needsReply ? <RowChip text="Needs reply" /> : null}
                <View style={{ flex: 1 }} />
                {model.hasAttachments ? <Icon name="paperclip" size={13} color={t.colors.muted} /> : null}
                <Txt variant="code" tone="muted" style={{ fontVariant: ["tabular-nums"] }}>
                  {model.date}
                </Txt>
                {model.starred ? <Icon name="star" size={12} color={t.colors.foreground} filled /> : null}
              </View>
              <Txt variant="body" tone={model.unread ? "text" : "secondary"} numberOfLines={1}>
                {model.badge ? `${model.badge} · ` : ""}
                {model.subject}
                {model.snippet ? (
                  <Txt variant="body" tone="muted">
                    {`  ${model.snippet}`}
                  </Txt>
                ) : null}
              </Txt>
            </View>
          </Pressable>
        </Animated.View>
      </GestureDetector>
    </View>
  );
}

/** Draft, Needs reply: mono sentence-case chips on line 1 (spec §4.2). */
function RowChip({ text }: { text: string }) {
  const t = useTheme();
  return (
    <View
      style={{
        height: 16,
        paddingHorizontal: 4,
        borderRadius: t.radius.sm,
        borderWidth: 1,
        borderColor: t.colors.borderSubtle,
        justifyContent: "center",
      }}
    >
      <Txt variant="crumb" tone="tertiary" style={{ fontSize: 9, lineHeight: 12 }}>
        {text}
      </Txt>
    </View>
  );
}

export const ThreadRow = memo(ThreadRowImpl);
