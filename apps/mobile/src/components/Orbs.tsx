// Crimson dawn orbs (BRIEF "Atmosphere"): soft radial gradients that float and morph slowly behind onboarding,
// empty states and the palette. Motion stops when the OS asks for reduced motion.
import { useEffect } from "react";
import { StyleSheet, View } from "react-native";
import Animated, {
  Easing,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withRepeat,
  withTiming,
} from "react-native-reanimated";
import { useTheme } from "../theme/ThemeProvider";

interface OrbSpec {
  size: number;
  top: `${number}%`;
  left: `${number}%`;
  core: "accentCore" | "accentMid" | "accentGlow";
  floatSeconds: number;
  morphSeconds: number;
  dx: number;
  dy: number;
}

function Orb({ spec, still }: { spec: OrbSpec; still: boolean }) {
  const t = useTheme();
  const p = useSharedValue(0);
  const m = useSharedValue(0);
  useEffect(() => {
    if (still) return;
    p.value = withRepeat(
      withTiming(1, { duration: spec.floatSeconds * 1000, easing: Easing.inOut(Easing.sin) }),
      -1,
      true,
    );
    m.value = withRepeat(
      withTiming(1, { duration: spec.morphSeconds * 1000, easing: Easing.inOut(Easing.quad) }),
      -1,
      true,
    );
  }, [still, spec.floatSeconds, spec.morphSeconds, p, m]);
  const style = useAnimatedStyle(() => ({
    transform: [
      { translateX: p.value * spec.dx },
      { translateY: p.value * spec.dy },
      { scaleX: 1 + m.value * 0.12 },
      { scaleY: 1 - m.value * 0.08 },
    ],
  }));
  const color = t.colors[spec.core];
  return (
    <Animated.View
      style={[
        {
          position: "absolute",
          top: spec.top,
          left: spec.left,
          width: spec.size,
          height: spec.size,
          borderRadius: spec.size / 2,
          backgroundImage: `radial-gradient(circle at center, ${color} 0%, ${t.colors.accentWash} 45%, transparent 70%)`,
          opacity: t.motion.orb.opacity * 2.4,
        },
        style,
      ]}
    />
  );
}

/** Background orbs; place inside a positioned container. */
export function Orbs({ intensity = 1 }: { intensity?: number }) {
  const t = useTheme();
  const reduced = useReducedMotion();
  const [f0, f1, f2] = t.motion.orb.floatSeconds;
  const [m0, m1, m2] = t.motion.orb.morphSeconds;
  const specs: OrbSpec[] = [
    {
      size: 360 * intensity,
      top: "-12%",
      left: "-30%",
      core: "accentCore",
      floatSeconds: f0,
      morphSeconds: m0,
      dx: 40,
      dy: 30,
    },
    {
      size: 300 * intensity,
      top: "35%",
      left: "45%",
      core: "accentMid",
      floatSeconds: f1,
      morphSeconds: m1,
      dx: -50,
      dy: 20,
    },
    {
      size: 240 * intensity,
      top: "70%",
      left: "-10%",
      core: "accentGlow",
      floatSeconds: f2,
      morphSeconds: m2,
      dx: 30,
      dy: -40,
    },
  ];
  return (
    <View
      pointerEvents="none"
      // Clipped: the orbs reach past the edges, which would scroll the page sideways on the web target.
      style={[StyleSheet.absoluteFill, { overflow: "hidden" }]}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      {specs.map((s) => (
        <Orb key={s.top} spec={s} still={reduced} />
      ))}
    </View>
  );
}
