// The bottom toolbar of the thread and of a multi-select (spec §4.4): a hairline top on the page colour, actions
// as a 20pt icon over an 11pt label in `tertiary`, and at most one ink pill (Reply).
import type { ReactNode } from "react";
import { Pressable, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useTheme } from "../theme/ThemeProvider";
import { Icon, type IconName } from "./Icon";
import { Txt } from "./ui";

export function Toolbar({ children, label }: { children: ReactNode; label?: string }) {
  const t = useTheme();
  const insets = useSafeAreaInsets();
  return (
    <View
      accessibilityRole="toolbar"
      accessibilityLabel={label}
      style={{
        flexDirection: "row",
        alignItems: "center",
        justifyContent: "space-around",
        minHeight: 56,
        paddingBottom: insets.bottom,
        borderTopWidth: 1,
        borderTopColor: t.colors.borderSubtle,
        backgroundColor: t.colors.background,
      }}
    >
      {children}
    </View>
  );
}

export function ToolbarButton({
  icon,
  label,
  onPress,
  hint,
  destructive,
}: {
  icon: IconName;
  label: string;
  onPress: () => void;
  hint?: string;
  destructive?: boolean;
}) {
  const t = useTheme();
  const c = destructive ? t.colors.accentText : t.colors.tertiary;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint={hint}
      onPress={onPress}
      hitSlop={6}
      style={({ pressed }) => ({
        minWidth: 64,
        minHeight: 48,
        alignItems: "center",
        justifyContent: "center",
        gap: 2,
        opacity: pressed ? 0.55 : 1,
      })}
    >
      <Icon name={icon} color={destructive ? c : t.colors.text} />
      <Txt variant="caption" style={{ color: c, fontSize: 11, lineHeight: 14 }}>
        {label}
      </Txt>
    </Pressable>
  );
}

/** The one ink pill in a toolbar (Reply). */
export function ToolbarPill({
  icon,
  label,
  onPress,
}: {
  icon?: IconName;
  label: string;
  onPress: () => void;
}) {
  const t = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      style={({ pressed }) => ({
        flexDirection: "row",
        alignItems: "center",
        gap: 6,
        height: 36,
        paddingHorizontal: t.space[2],
        borderRadius: t.radius.full,
        backgroundColor: t.colors.foreground,
        opacity: pressed ? 0.75 : 1,
      })}
    >
      {icon ? <Icon name={icon} size={18} color={t.colors.background} /> : null}
      <Txt variant="bodyStrong" tone="inverse">
        {label}
      </Txt>
    </Pressable>
  );
}
