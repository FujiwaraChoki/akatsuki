// Snooze / remind / send-later picker: presets first (one tap), then a custom day and half-hour slot. Times are
// computed in the user's zone by vm/snooze.ts.
import { useMemo, useState } from "react";
import { Pressable, ScrollView, View } from "react-native";
import { useTheme } from "../theme/ThemeProvider";
import { customTime, type DayOption, dayOptions, slotLabel, TIME_SLOTS, type TimePreset } from "../vm/snooze";
import { wallTime } from "../vm/zoned";
import { Button, Chip, Label, Txt } from "./ui";

export function TimePicker({
  presets,
  timeZone,
  confirmLabel,
  onPick,
}: {
  presets: TimePreset[];
  timeZone: string;
  confirmLabel: string;
  onPick: (at: number) => void;
}) {
  const t = useTheme();
  const now = Date.now();
  const days = useMemo(() => dayOptions(now, timeZone), [now, timeZone]);
  const [custom, setCustom] = useState(false);
  const [day, setDay] = useState<DayOption>(days[1] ?? days[0]!);
  const [slot, setSlot] = useState(() => {
    const w = wallTime(now, timeZone);
    return Math.min(47, Math.ceil((w.hour * 60 + w.minute + 60) / 30)) * 30;
  });
  const choice = customTime(day, slot, now, timeZone);

  return (
    <View style={{ gap: t.space[1] }}>
      {presets.map((p, i) => (
        <Pressable
          key={p.id}
          accessibilityRole="button"
          accessibilityLabel={`${p.label}, ${p.hint}`}
          onPress={() => onPick(p.at)}
          style={({ pressed }) => ({
            flexDirection: "row",
            alignItems: "center",
            minHeight: 52,
            paddingHorizontal: t.space[2],
            borderRadius: t.radius.md,
            backgroundColor: pressed ? t.colors.selected : "transparent",
          })}
        >
          <Txt variant="caption" tone="muted" style={{ width: 20 }}>
            {i + 1}
          </Txt>
          <Txt variant="body" style={{ flex: 1 }}>
            {p.label}
          </Txt>
          <Txt variant="meta" tone="muted">
            {p.hint}
          </Txt>
        </Pressable>
      ))}
      {!custom ? (
        <Button title="Pick a date and time" variant="ghost" onPress={() => setCustom(true)} />
      ) : (
        <View style={{ gap: t.space[1], paddingTop: t.space[1] }}>
          <Label style={{ paddingHorizontal: t.space[2] }}>Day</Label>
          <ScrollView
            horizontal
            showsHorizontalScrollIndicator={false}
            contentContainerStyle={{ gap: t.space[1], paddingHorizontal: t.space[2] }}
          >
            {days.map((d) => (
              <Chip
                key={d.offset}
                label={d.label}
                selected={d.offset === day.offset}
                onPress={() => setDay(d)}
              />
            ))}
          </ScrollView>
          <Label style={{ paddingHorizontal: t.space[2] }}>Time</Label>
          <ScrollView
            horizontal
            showsHorizontalScrollIndicator={false}
            contentContainerStyle={{ gap: t.space[1], paddingHorizontal: t.space[2] }}
          >
            {TIME_SLOTS.map((m) => (
              <Chip key={m} label={slotLabel(m)} selected={m === slot} onPress={() => setSlot(m)} />
            ))}
          </ScrollView>
          {!choice.ok ? (
            <Txt variant="meta" tone="error" style={{ paddingHorizontal: t.space[2] }}>
              {choice.error}
            </Txt>
          ) : null}
          <View style={{ paddingHorizontal: t.space[2] }}>
            <Button
              title={confirmLabel}
              variant="primary"
              disabled={!choice.ok}
              onPress={() => {
                if (choice.ok) onPick(choice.at);
              }}
            />
          </View>
        </View>
      )}
    </View>
  );
}
