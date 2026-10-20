import type { ReactNode } from "react";
import { Pressable, View } from "react-native";
import { useTheme } from "../theme/ThemeProvider";
import { Txt } from "./ui";

/** Screen header: Instrument Serif title, optional back button and trailing actions. */
export function Header({
  title,
  onBack,
  onTitlePress,
  right,
  subtitle,
}: {
  title: string;
  onBack?: () => void;
  onTitlePress?: () => void;
  right?: ReactNode;
  subtitle?: string | null;
}) {
  const t = useTheme();
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        paddingHorizontal: t.space[2],
        paddingTop: t.space[1],
        paddingBottom: t.space[1],
        gap: t.space[1],
      }}
    >
      {onBack ? <HeaderButton label="Back" onPress={onBack} /> : null}
      <Pressable
        style={{ flex: 1 }}
        disabled={!onTitlePress}
        onPress={onTitlePress}
        accessibilityRole={onTitlePress ? "button" : "header"}
        accessibilityHint={onTitlePress ? "Choose a mailbox" : undefined}
      >
        <Txt variant="heading" numberOfLines={1}>
          {title}
        </Txt>
        {subtitle ? (
          <Txt variant="label" tone="muted" numberOfLines={1}>
            {subtitle}
          </Txt>
        ) : null}
      </Pressable>
      {right}
    </View>
  );
}

export function HeaderButton({
  label,
  onPress,
  accent,
  hint,
}: {
  label: string;
  onPress: () => void;
  accent?: boolean;
  hint?: string;
}) {
  const t = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint={hint}
      onPress={onPress}
      hitSlop={8}
      style={({ pressed }) => ({
        minHeight: 44,
        minWidth: 44,
        paddingHorizontal: t.space[1],
        justifyContent: "center",
        alignItems: "center",
        opacity: pressed ? 0.6 : 1,
      })}
    >
      <Txt variant="label" tone={accent ? "accent" : "secondary"}>
        {label}
      </Txt>
    </Pressable>
  );
}
