// The two brand marks (design spec §2.4, §2.10): the red dot, and the horizon, a 1px dawn line with a band of
// light standing on it, used only on sign-in and at inbox zero. Static on mobile (no animation).
import { type StyleProp, Text, type TextStyle, View, type ViewStyle } from "react-native";
import Svg, { Defs, LinearGradient, RadialGradient, Rect, Stop } from "react-native-svg";
import { useTheme } from "../theme/ThemeProvider";

export function Dot({ size = 12, style }: { size?: number; style?: StyleProp<ViewStyle> }) {
  const t = useTheme();
  return (
    <View
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[{ width: size, height: size, borderRadius: size / 2, backgroundColor: t.colors.dawn }, style]}
    />
  );
}

/**
 * A serif headline whose final period is drawn as a dawn dot (0.16em, on the baseline). The accessible name keeps
 * the period ("Mail at the speed of thought.").
 */
export function DotHeadline({ text, style }: { text: string; style?: StyleProp<TextStyle> }) {
  const t = useTheme();
  const words = text.replace(/\.$/, "");
  const size = t.text.display.fontSize;
  const dot = Math.max(5, Math.round(size * 0.16));
  return (
    <Text
      accessibilityRole="header"
      accessibilityLabel={`${words}.`}
      maxFontSizeMultiplier={1.4}
      style={[t.text.display, { color: t.colors.text }, style]}
    >
      {words}
      <Text accessibilityElementsHidden>{" "}</Text>
      <View
        style={{
          width: dot,
          height: dot,
          borderRadius: dot / 2,
          backgroundColor: t.colors.dawn,
          marginLeft: Math.round(size * 0.03),
          transform: [{ translateX: -Math.round(size * 0.12) }, { translateY: -Math.round(size * 0.04) }],
        }}
      />
    </Text>
  );
}

/** The horizon: a full-bleed line and the band of light standing on it, positioned by the caller (absolute). */
export function Horizon({ style }: { style?: StyleProp<ViewStyle> }) {
  const t = useTheme();
  const dark = t.mode === "dark";
  const dawn = t.colors.dawn;
  return (
    <View
      pointerEvents="none"
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[{ position: "absolute", left: 0, right: 0, height: 65 }, style]}
    >
      <Svg width="100%" height={65} preserveAspectRatio="none" viewBox="0 0 100 65">
        <Defs>
          <RadialGradient id="band" cx="50" cy="64" rx="38" ry="64" gradientUnits="userSpaceOnUse">
            <Stop offset="0" stopColor={dawn} stopOpacity={dark ? 0.3 : 0.07} />
            <Stop offset="1" stopColor={dawn} stopOpacity={0} />
          </RadialGradient>
          <LinearGradient id="line" x1="0" y1="0" x2="1" y2="0">
            <Stop offset="0.04" stopColor={dawn} stopOpacity={0} />
            <Stop offset="0.2" stopColor={dawn} stopOpacity={0.35} />
            <Stop offset="0.38" stopColor={dawn} stopOpacity={1} />
            <Stop offset="0.62" stopColor={dawn} stopOpacity={1} />
            <Stop offset="0.8" stopColor={dawn} stopOpacity={0.35} />
            <Stop offset="0.96" stopColor={dawn} stopOpacity={0} />
          </LinearGradient>
        </Defs>
        <Rect x="0" y="0" width="100" height="64" fill="url(#band)" />
        <Rect x="0" y="64" width="100" height="1" fill="url(#line)" opacity={dark ? 0.95 : 0.8} />
      </Svg>
    </View>
  );
}
