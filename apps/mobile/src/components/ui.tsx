// Primitives styled only from tokens (via the theme), per the First Light spec (§2, §4): ink on paper, hairlines
// for structure, red only where it means something. Buttons are flat pills: `primary` is ink, `brand` is the
// accent fill (sign-in only), `secondary` a hairline pill, `ghost` quiet text, `destructive` accent-text.
import { Children, Fragment, isValidElement, type ReactNode, useState } from "react";
import {
  ActivityIndicator,
  Platform,
  Pressable,
  type PressableProps,
  type StyleProp,
  StyleSheet,
  Switch,
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

/** Borderless inputs (compose fields, search, palette) draw no browser focus outline in the web preview. */
export const NO_OUTLINE: TextStyle | null =
  Platform.OS === "web" ? ({ outlineStyle: "none", outlineWidth: 0 } as unknown as TextStyle) : null;

/** Keyboard hints next to actions only where a keyboard is likely: iPad (spec §4.6), and wide web previews. */
export function showKeyHints(width?: number): boolean {
  if (Platform.OS === "ios") return Platform.isPad;
  if (Platform.OS === "web") return (width ?? 0) >= 768;
  return false;
}

type Variant = keyof Theme["text"];
type Tone = "text" | "secondary" | "tertiary" | "muted" | "accent" | "onAccent" | "error" | "inverse";

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
        : tone === "inverse"
          ? t.colors.background
          : t.colors[tone];
  return <Text {...rest} maxFontSizeMultiplier={1.6} style={[t.text[variant], { color }, style]} />;
}

/** Sentence-case group or field label (`text.app.section`). */
export function Label({ children, style }: { children: ReactNode; style?: StyleProp<TextStyle> }) {
  return (
    <Txt variant="label" tone="muted" style={style} accessibilityRole="header">
      {children}
    </Txt>
  );
}

/** Mono label in sentence case (spec §2.1): only the thread crumb, AI provenance and chips. */
export function Crumb({ children, style }: { children: ReactNode; style?: StyleProp<TextStyle> }) {
  return (
    <Txt variant="crumb" tone="muted" style={style}>
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

export type ButtonVariant = "primary" | "brand" | "secondary" | "ghost" | "destructive";

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
  icon,
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
  /** An icon before the title. */
  icon?: ReactNode;
}) {
  const t = useTheme();
  const filled = variant === "primary" || variant === "brand";
  const fill = variant === "brand" ? t.colors.accent : t.colors.foreground;
  const textColor = filled
    ? disabled
      ? t.colors.muted
      : variant === "brand"
        ? t.colors.onAccent
        : t.colors.background
    : variant === "destructive"
      ? t.colors.accentText
      : variant === "ghost"
        ? t.colors.tertiary
        : t.colors.text;
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
          minHeight: compact ? 36 : 48,
          paddingHorizontal: compact ? t.space["1_5"] : t.space[2],
          borderRadius: t.radius.full,
          alignItems: "center",
          justifyContent: "center",
          flexDirection: "row",
          gap: t.space[1],
          opacity: disabled && !filled ? 0.45 : 1,
          // A disabled filled button is a quiet grey pill, not a dimmed colour that reads as broken.
          backgroundColor: filled ? (disabled ? t.colors.border : fill) : "transparent",
          borderWidth: variant === "secondary" ? 1 : 0,
          borderColor: t.colors.border,
        },
        pressed ? { opacity: 0.7 } : null,
        // Focus is a 2px ink ring (spec §2.8).
        focused ? { outlineWidth: 2, outlineColor: t.colors.foreground, outlineOffset: 2 } : null,
        style,
      ]}
    >
      {busy ? <ActivityIndicator color={textColor} /> : icon}
      <Text
        maxFontSizeMultiplier={1.6}
        style={[t.text.bodyStrong, { color: textColor }, compact ? { fontSize: 14 } : null]}
      >
        {title}
      </Text>
      {hint ? (
        <Txt variant="caption" style={{ color: textColor, opacity: 0.7 }}>
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
  onFocus,
  onBlur,
  ...rest
}: TextInputProps & { label?: string; error?: string | null }) {
  const t = useTheme();
  const [focused, setFocused] = useState(false);
  return (
    <View style={{ gap: t.space["0_5"] }}>
      {label ? <Label>{label}</Label> : null}
      <TextInput
        placeholderTextColor={t.colors.muted}
        selectionColor={t.colors.dawn}
        cursorColor={t.colors.dawn}
        accessibilityLabel={label}
        onFocus={(e) => {
          setFocused(true);
          onFocus?.(e);
        }}
        onBlur={(e) => {
          setFocused(false);
          onBlur?.(e);
        }}
        {...rest}
        style={[
          t.text.reading,
          {
            color: t.colors.text,
            minHeight: 48,
            paddingHorizontal: t.space["1_5"],
            paddingVertical: t.space[1],
            borderRadius: t.radius.md,
            borderWidth: 1,
            borderColor: error ? t.colors.accentText : focused ? t.colors.foreground : t.colors.border,
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
  mono,
}: {
  label: string;
  onPress?: () => void;
  selected?: boolean;
  onRemove?: () => void;
  /** Mono text (search operators). */
  mono?: boolean;
}) {
  const t = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={onRemove ? `Remove ${label}` : label}
      accessibilityState={{ selected: !!selected }}
      onPress={onRemove ?? onPress}
      style={({ pressed }) => ({
        alignSelf: "flex-start",
        flexDirection: "row",
        alignItems: "center",
        gap: t.space["0_5"],
        paddingHorizontal: t.space["1_5"],
        minHeight: 32,
        borderRadius: t.radius.full,
        backgroundColor: selected || pressed ? t.colors.surfaceStrong : t.colors.background,
        borderWidth: 1,
        borderColor: selected ? t.colors.border : t.colors.borderSubtle,
      })}
    >
      <Txt
        variant={mono ? "code" : "meta"}
        tone={selected ? "text" : "secondary"}
        style={mono ? { fontSize: 13 } : null}
      >
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
  icon,
  ...rest
}: { title: string; detail?: string | null; right?: ReactNode; selected?: boolean; icon?: ReactNode } & Pick<
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
        minHeight: 48,
        backgroundColor: pressed || selected ? t.colors.surfaceStrong : "transparent",
      })}
    >
      {icon}
      <View style={{ flex: 1, gap: 2 }}>
        <Txt variant="body" style={{ fontSize: 16, lineHeight: 22 }}>
          {title}
        </Txt>
        {detail ? (
          <Txt variant="meta" tone="tertiary">
            {detail}
          </Txt>
        ) : null}
      </View>
      {right}
    </Pressable>
  );
}

export function Divider({ inset = 0 }: { inset?: number }) {
  const t = useTheme();
  return (
    <View
      style={{ height: StyleSheet.hairlineWidth, marginLeft: inset, backgroundColor: t.colors.borderSubtle }}
    />
  );
}

/**
 * A settings group (spec §4.9): a sentence-case header, the rows on a `surface` plane with radius lg, hairlines
 * between rows inset 16, and an optional footer. One level, no card inside a card.
 */
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
  const items = Children.toArray(children).filter((c) => isValidElement(c));
  return (
    <View style={{ marginTop: t.space[3], marginHorizontal: t.space[2] }}>
      <Label style={{ marginBottom: t.space[1], marginLeft: t.space["0_5"] }}>{title}</Label>
      {items.length > 0 ? (
        <View style={{ backgroundColor: t.colors.surface, borderRadius: t.radius.lg, overflow: "hidden" }}>
          {items.map((c, i) => (
            <Fragment key={(isValidElement(c) && c.key) || i}>
              {i > 0 ? <Divider inset={t.space[2]} /> : null}
              {c}
            </Fragment>
          ))}
        </View>
      ) : null}
      {footer ? (
        <Txt variant="meta" tone="muted" style={{ marginTop: t.space[1], marginHorizontal: t.space["0_5"] }}>
          {footer}
        </Txt>
      ) : null}
    </View>
  );
}

export function Spinner() {
  const t = useTheme();
  return <ActivityIndicator color={t.colors.muted} accessibilityLabel="Loading" />;
}

/** A radio-style choice row: an ink ring, an ink dot when chosen (no accent, spec §4.9). */
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
            borderWidth: 1.5,
            borderColor: selected ? t.colors.foreground : t.colors.border,
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          {selected ? (
            <View style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: t.colors.foreground }} />
          ) : null}
        </View>
      }
      selected={false}
    />
  );
}

/** An ink switch (spec §4.9): track `fg` when on, `lineStrong` when off; the thumb is the page colour. */
export function Toggle({
  value,
  onValueChange,
  disabled,
  accessibilityLabel,
}: {
  value: boolean;
  onValueChange: (v: boolean) => void;
  disabled?: boolean;
  accessibilityLabel: string;
}) {
  const t = useTheme();
  return (
    <Switch
      accessibilityLabel={accessibilityLabel}
      value={value}
      disabled={disabled}
      onValueChange={onValueChange}
      trackColor={{ true: t.colors.foreground, false: t.colors.border }}
      thumbColor={t.colors.background}
      ios_backgroundColor={t.colors.border}
      {...({ activeThumbColor: t.colors.background } as object)}
    />
  );
}
