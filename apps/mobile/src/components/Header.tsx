import type { ReactNode } from "react";
import { Pressable, View } from "react-native";
import { useTheme } from "../theme/ThemeProvider";
import { Icon, type IconName } from "./Icon";
import { Txt } from "./ui";

/**
 * Screen header (spec §4.1): 44pt on the page colour. `variant="page"` (default) sets the title in Instrument
 * Serif 28 (settings and other pages, one serif per screen); `variant="bar"` sets it in 17/600 sans (the inbox
 * switcher, compose). A back button is a chevron.
 */
export function Header({
  title,
  onBack,
  backLabel = "Back",
  onTitlePress,
  titleHint = "Choose a mailbox",
  right,
  subtitle,
  variant = "page",
  center,
}: {
  title: string;
  onBack?: () => void;
  backLabel?: string;
  onTitlePress?: () => void;
  titleHint?: string;
  right?: ReactNode;
  subtitle?: string | null;
  variant?: "page" | "bar";
  /** Centre the title (compose). */
  center?: boolean;
}) {
  const t = useTheme();
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        minHeight: 44,
        paddingHorizontal: t.space[1],
        paddingVertical: variant === "page" ? t.space[1] : 0,
        gap: t.space["0_5"],
      }}
    >
      {onBack ? <HeaderButton label={backLabel} icon="chevron-left" onPress={onBack} /> : null}
      <Pressable
        style={{
          flex: 1,
          paddingHorizontal: onBack ? 0 : t.space[1],
          alignItems: center ? "center" : "flex-start",
        }}
        disabled={!onTitlePress}
        onPress={onTitlePress}
        accessibilityRole={onTitlePress ? "button" : "header"}
        accessibilityHint={onTitlePress ? titleHint : undefined}
      >
        <View style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
          <Txt variant={variant === "page" ? "heading" : "title"} numberOfLines={1}>
            {title}
          </Txt>
          {onTitlePress ? <Icon name="chevron-down" size={16} color={t.colors.muted} /> : null}
        </View>
        {subtitle ? (
          <Txt variant="meta" tone="muted" numberOfLines={1}>
            {subtitle}
          </Txt>
        ) : null}
      </Pressable>
      {right}
    </View>
  );
}

/** A 44×44 header action: an icon (with the label for assistive tech) or a quiet word. */
export function HeaderButton({
  label,
  onPress,
  accent,
  hint,
  icon,
  filled,
}: {
  label: string;
  onPress: () => void;
  /** The screen's main word action (ink, medium). */
  accent?: boolean;
  hint?: string;
  icon?: IconName;
  /** A filled icon (a starred star). */
  filled?: boolean;
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
        paddingHorizontal: icon ? 0 : t.space[1],
        justifyContent: "center",
        alignItems: "center",
        opacity: pressed ? 0.55 : 1,
      })}
    >
      {icon ? (
        <Icon name={icon} color={t.colors.text} filled={filled} />
      ) : (
        <Txt variant={accent ? "bodyStrong" : "body"} tone={accent ? "text" : "tertiary"}>
          {label}
        </Txt>
      )}
    </Pressable>
  );
}
