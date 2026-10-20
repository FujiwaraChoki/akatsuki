// Primitives styled only from tokens (via the theme): text styles, buttons, fields, chips, rows.
import type { ReactNode } from "react";
import {
  ActivityIndicator,
  Pressable,
  type PressableProps,
  type StyleProp,
  StyleSheet,
  Text,
  TextInput,
  type TextInputProps,
  type TextProps,
  type TextStyle,
  View,
  type ViewStyle,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { useTheme } from "../theme/ThemeProvider";
import type { Theme } from "../theme/theme";

type Variant = keyof Theme["text"];
type Tone = "text" | "secondary" | "muted" | "accent" | "onAccent" | "error";

export function Txt({
  variant = "body",
  tone = "text",
  style,
  ...rest
}: TextProps & { variant?: Variant; tone?: Tone; style?: StyleProp<TextStyle> }) {
  const t = useTheme();
  const color =
    tone === "accent" || tone === "error"
      ? t.colors.accentText
      : tone === "onAccent"
        ? t.colors.onAccent
        : t.colors[tone];
  return <Text {...rest} maxFontSizeMultiplier={1.6} style={[t.text[variant], { color }, style]} />;
}

/** Uppercase tracked label (0.12em in app chrome). */
export function Label({ children, style }: { children: ReactNode; style?: StyleProp<TextStyle> }) {
  return (
    <Txt variant="label" tone="muted" style={style} accessibilityRole="header">
      {children}
    </Txt>
  );
}

export function Screen({
  children,
  edges = ["top", "left", "right"],
}: {
  children: ReactNode;
  edges?: ("top" | "bottom" | "left" | "right")[];
}) {
  const t = useTheme();
  return (
    <SafeAreaView edges={edges} style={{ flex: 1, backgroundColor: t.colors.background }}>
      {children}
    </SafeAreaView>
  );
}

export type ButtonVariant = "primary" | "secondary" | "ghost";

export function Button({
  title,
  onPress,
  variant = "secondary",
  disabled,
  busy,
  hint,
  style,
  accessibilityLabel,
  compact,
}: {
  title: string;
  onPress?: () => void;
  variant?: ButtonVariant;
  disabled?: boolean;
  busy?: boolean;
  /** Keyboard hint shown after the title on wide screens, e.g. "⌘↵". */
  hint?: string | null;
  style?: StyleProp<ViewStyle>;
  accessibilityLabel?: string;
  compact?: boolean;
}) {
  const t = useTheme();
  const primary = variant === "primary";
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel ?? title}
      accessibilityState={{ disabled: !!disabled, busy: !!busy }}
      disabled={disabled || busy}
      onPress={onPress}
      hitSlop={6}
      style={({ pressed, focused }: { pressed: boolean; focused?: boolean }) => [
        {
          minHeight: compact ? 36 : 44,
          paddingHorizontal: compact ? t.space["1_5"] : t.space[2],
          // Pills, like web and the landing site (one radius token for buttons on every client, D-409).
          borderRadius: t.radius.full,
          alignItems: "center",
          justifyContent: "center",
          flexDirection: "row",
          gap: t.space[1],
          // A disabled primary is an outline with quiet text, not a dimmed deep crimson that reads as broken.
          opacity: disabled && !primary ? 0.45 : 1,
          backgroundColor: primary
            ? disabled
              ? "transparent"
              : t.colors.accent
            : variant === "secondary"
              ? t.colors.surface
              : "transparent",
          borderWidth: variant === "secondary" ? StyleSheet.hairlineWidth : primary && disabled ? 1 : 0,
          borderColor: t.colors.border,
        },
        primary && !disabled ? { boxShadow: `0 0 ${t.space[3]}px ${t.colors.accentGlow}` } : null,
        pressed ? { opacity: 0.75 } : null,
        focused ? { borderWidth: 2, borderColor: t.colors.accentMid } : null,
        style,
      ]}
    >
      {busy ? <ActivityIndicator color={primary ? t.colors.onAccent : t.colors.muted} /> : null}
      <Txt
        variant="bodyStrong"
        tone={primary ? (disabled ? "muted" : "onAccent") : variant === "ghost" ? "accent" : "text"}
      >
        {title}
      </Txt>
      {hint ? (
        <Txt variant="caption" tone={primary ? "onAccent" : "muted"} style={{ opacity: 0.8 }}>
          {hint}
        </Txt>
      ) : null}
    </Pressable>
  );
}

export function Field({
  label,
  error,
  style,
  ...rest
}: TextInputProps & { label?: string; error?: string | null }) {
  const t = useTheme();
  return (
    <View style={{ gap: t.space["0_5"] }}>
      {label ? <Label>{label}</Label> : null}
      <TextInput
        placeholderTextColor={t.colors.muted}
        selectionColor={t.colors.accent}
        accessibilityLabel={label}
        {...rest}
        style={[
          t.text.reading,
          {
            color: t.colors.text,
            minHeight: 44,
            paddingHorizontal: t.space["1_5"],
            paddingVertical: t.space[1],
            borderRadius: t.radius.md,
            borderWidth: StyleSheet.hairlineWidth,
            borderColor: error ? t.colors.accent : t.colors.border,
            backgroundColor: t.colors.background,
          },
          style,
        ]}
      />
      {error ? (
        <Txt variant="meta" tone="error" accessibilityLiveRegion="polite">
          {error}
        </Txt>
      ) : null}
    </View>
  );
}

export function Chip({
  label,
  onPress,
  selected,
  onRemove,
}: {
  label: string;
  onPress?: () => void;
  selected?: boolean;
  onRemove?: () => void;
}) {
  const t = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={onRemove ? `Remove ${label}` : label}
      accessibilityState={{ selected: !!selected }}
      onPress={onRemove ?? onPress}
      style={({ pressed }) => ({
        flexDirection: "row",
        alignItems: "center",
        gap: t.space["0_5"],
        paddingHorizontal: t.space["1_5"],
        minHeight: 32,
        borderRadius: t.radius.full,
        backgroundColor: selected ? t.colors.accentTint : t.colors.surface,
        borderWidth: StyleSheet.hairlineWidth,
        borderColor: selected ? t.colors.accentMid : t.colors.borderSubtle,
        opacity: pressed ? 0.7 : 1,
      })}
    >
      <Txt variant="meta" tone={selected ? "accent" : "text"}>
        {label}
      </Txt>
      {onRemove ? (
        <Txt variant="meta" tone="muted" accessibilityElementsHidden>
          ×
        </Txt>
      ) : null}
    </Pressable>
  );
}

export function Row({
  title,
  detail,
  right,
  onPress,
  selected,
  ...rest
}: { title: string; detail?: string | null; right?: ReactNode; selected?: boolean } & Pick<
  PressableProps,
  "onPress" | "accessibilityHint"
>) {
  const t = useTheme();
  return (
    <Pressable
      accessibilityRole={onPress ? "button" : undefined}
      accessibilityState={{ selected: !!selected }}
      onPress={onPress}
      disabled={!onPress}
      {...rest}
      style={({ pressed }) => ({
        flexDirection: "row",
        alignItems: "center",
        gap: t.space["1_5"],
        paddingHorizontal: t.space[2],
        paddingVertical: t.space["1_5"],
        minHeight: 52,
        backgroundColor: pressed || selected ? t.colors.selected : "transparent",
      })}
    >
      <View style={{ flex: 1, gap: 2 }}>
        <Txt variant="body">{title}</Txt>
        {detail ? (
          <Txt variant="meta" tone="muted">
            {detail}
          </Txt>
        ) : null}
      </View>
      {right}
    </Pressable>
  );
}

export function Divider() {
  const t = useTheme();
  return <View style={{ height: StyleSheet.hairlineWidth, backgroundColor: t.colors.borderSubtle }} />;
}

export function Section({
  title,
  children,
  footer,
}: {
  title: string;
  children: ReactNode;
  footer?: string | null;
}) {
  const t = useTheme();
  return (
    <View style={{ marginTop: t.space[3] }}>
      <Label style={{ paddingHorizontal: t.space[2], marginBottom: t.space[1] }}>{title}</Label>
      <View
        style={{
          borderTopWidth: StyleSheet.hairlineWidth,
          borderBottomWidth: StyleSheet.hairlineWidth,
          borderColor: t.colors.borderSubtle,
        }}
      >
        {children}
      </View>
      {footer ? (
        <Txt variant="meta" tone="muted" style={{ paddingHorizontal: t.space[2], marginTop: t.space[1] }}>
          {footer}
        </Txt>
      ) : null}
    </View>
  );
}

export function Spinner() {
  const t = useTheme();
  return <ActivityIndicator color={t.colors.accent} accessibilityLabel="Loading" />;
}

/** A radio-style choice row. */
export function Choice({
  title,
  detail,
  selected,
  onPress,
}: {
  title: string;
  detail?: string;
  selected: boolean;
  onPress: () => void;
}) {
  const t = useTheme();
  return (
    <Row
      title={title}
      detail={detail}
      onPress={onPress}
      right={
        <View
          accessibilityElementsHidden
          style={{
            width: 20,
            height: 20,
            borderRadius: 10,
            borderWidth: 2,
            borderColor: selected ? t.colors.accent : t.colors.borderHover,
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          {selected ? (
            <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: t.colors.accent }} />
          ) : null}
        </View>
      }
      selected={false}
    />
  );
}
