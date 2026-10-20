// Design tokens mapped to React Native styles (BRIEF "Design theme"). Every colour, size, radius and font comes
// from @akatsuki/tokens; nothing here hard-codes the palette. Pure (no React Native import) so it is unit-tested.
import { accent, color, fg, rgba, type ThemeMode, text as textTokens, tokens } from "@akatsuki/tokens";

/** Font family names registered with expo-font (one family per weight: Android has no weight synthesis). */
export const FONT_FAMILIES = {
  serif: "InstrumentSerif",
  serifItalic: "InstrumentSerif-Italic",
  sans400: "Geist",
  sans500: "Geist-Medium",
  sans600: "Geist-SemiBold",
  sans700: "Geist-Bold",
  mono400: "GeistMono",
  mono500: "GeistMono-Medium",
} as const;

/** Files of each family, relative to @akatsuki/tokens/fonts (names from tokens.font.*.files). */
export const FONT_FILES: Record<(typeof FONT_FAMILIES)[keyof typeof FONT_FAMILIES], string> = {
  InstrumentSerif: tokens.font.serif.files["400"],
  "InstrumentSerif-Italic": tokens.font.serif.files["400italic"],
  Geist: tokens.font.sans.files["400"],
  "Geist-Medium": tokens.font.sans.files["500"],
  "Geist-SemiBold": tokens.font.sans.files["600"],
  "Geist-Bold": tokens.font.sans.files["700"],
  GeistMono: tokens.font.mono.files["400"],
  "GeistMono-Medium": tokens.font.mono.files["500"],
};

export function fontFamily(role: "serif" | "sans" | "mono", weight: number): string {
  if (role === "serif") return FONT_FAMILIES.serif; // single weight 400; emphasis by size and colour
  if (role === "mono") return weight >= 500 ? FONT_FAMILIES.mono500 : FONT_FAMILIES.mono400;
  if (weight >= 700) return FONT_FAMILIES.sans700;
  if (weight >= 600) return FONT_FAMILIES.sans600;
  if (weight >= 500) return FONT_FAMILIES.sans500;
  return FONT_FAMILIES.sans400;
}

type Rgb = readonly [number, number, number];

/** `a` mixed with `b` (`t` = share of `b`), as an rgb() string. Used for the contrast-lifted accent (D-301). */
export function mix(a: Rgb, b: Rgb, t: number): string {
  const c = a.map((v, i) => Math.round(v * (1 - t) + b[i]! * t));
  return `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
}

export interface Colors {
  background: string;
  foreground: string;
  text: string;
  secondary: string;
  muted: string;
  borderHover: string;
  border: string;
  borderSubtle: string;
  accent: string;
  accentCore: string;
  accentMid: string;
  accentGlow: string;
  accentTint: string;
  accentWash: string;
  /**
   * Accent for text and thin strokes. Light: the accent itself (#C1272D on white is about 5.5:1). Dark: the deep
   * crimson mixed 50/50 with the foreground, as on the landing site (D-301), because #800000 on #0A0A0A is 1.8:1.
   */
  accentText: string;
  /** Accent text on the inverted toast surface. */
  accentOnInverse: string;
  /** Text on a solid accent fill: the light end of the theme in both modes (dark accent is #800000). */
  onAccent: string;
  /** Slightly lifted surface (sheets, cards). */
  surface: string;
  /** Pressed / selected row. */
  selected: string;
  /** The light page behind sender-styled HTML email. */
  paper: string;
  paperText: string;
  /** Dims the screen behind sheets and menus. */
  scrim: string;
  /** Translucent page behind the command palette. */
  veil: string;
}

export function makeColors(mode: ThemeMode): Colors {
  const c = color[mode];
  return {
    background: c.background,
    foreground: c.foreground,
    text: fg(mode, "text"),
    secondary: fg(mode, "secondary"),
    muted: fg(mode, "muted"),
    borderHover: fg(mode, "borderHover"),
    border: fg(mode, "border"),
    borderSubtle: fg(mode, "borderSubtle"),
    accent: c.accent,
    accentCore: accent(mode, "core"),
    accentMid: accent(mode, "mid"),
    accentGlow: accent(mode, "glow"),
    accentTint: accent(mode, "tint"),
    accentWash: accent(mode, "wash"),
    accentText: c.accentText, // the shared token (D-399), same lift as D-301
    accentOnInverse:
      mode === "dark" ? color.dark.accent : mix(color.light.rgb.accent, color.light.rgb.background, 0.5),
    onAccent: mode === "dark" ? color.dark.foreground : color.light.background,
    surface: mode === "dark" ? fg("dark", "borderSubtle") : accent("light", "wash"),
    selected: accent(mode, "tint"),
    paper: color.light.background,
    paperText: color.light.foreground,
    scrim: mode === "dark" ? rgba("dark", "background", 0.7) : rgba("light", "foreground", 0.35),
    veil: rgba(mode, "background", 0.86),
  };
}

export interface TextStyleSpec {
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  letterSpacing: number;
  textTransform?: "uppercase";
}

type AppStyle = keyof typeof textTokens.app;

/** A token text style as an RN style: `tracking` (em) becomes letterSpacing in points. */
export function textStyle(name: AppStyle, scale = 1): TextStyleSpec {
  const t = textTokens.app[name] as {
    font: "serif" | "sans" | "mono";
    size: number;
    lineHeight: number;
    tracking: number;
    weight: number;
    uppercase?: boolean;
  };
  const size = Math.round(t.size * scale * 10) / 10;
  return {
    fontFamily: fontFamily(t.font, t.weight),
    fontSize: size,
    lineHeight: Math.round(size * t.lineHeight),
    letterSpacing: Math.round(t.tracking * size * 100) / 100,
    ...(t.uppercase ? { textTransform: "uppercase" as const } : {}),
  };
}

/**
 * Mobile type scale. Touch screens read larger than desktop chrome, so the app styles are scaled by 1.15 for body
 * text (13 → 15) while headings keep their token sizes.
 */
export const MOBILE_SCALE = 1.15;

export interface Theme {
  mode: ThemeMode;
  colors: Colors;
  space: typeof tokens.space;
  radius: typeof tokens.radius;
  motion: typeof tokens.motion;
  layout: typeof tokens.layout;
  effect: typeof tokens.effect;
  text: {
    caption: TextStyleSpec;
    meta: TextStyleSpec;
    body: TextStyleSpec;
    bodyStrong: TextStyleSpec;
    reading: TextStyleSpec;
    code: TextStyleSpec;
    label: TextStyleSpec;
    subject: TextStyleSpec;
    heading: TextStyleSpec;
    empty: TextStyleSpec;
  };
}

export function makeTheme(mode: ThemeMode): Theme {
  return {
    mode,
    colors: makeColors(mode),
    space: tokens.space,
    radius: tokens.radius,
    motion: tokens.motion,
    layout: tokens.layout,
    effect: tokens.effect,
    text: {
      caption: textStyle("caption", MOBILE_SCALE),
      meta: textStyle("meta", MOBILE_SCALE),
      body: textStyle("body", MOBILE_SCALE),
      bodyStrong: textStyle("bodyStrong", MOBILE_SCALE),
      reading: textStyle("reading"),
      code: textStyle("code", MOBILE_SCALE),
      label: textStyle("label"),
      subject: textStyle("subject"),
      heading: textStyle("heading"),
      empty: textStyle("empty"),
    },
  };
}

/** The effective mode for `prefs.theme` and the OS scheme. */
export function resolveMode(
  pref: "system" | "light" | "dark" | undefined,
  system: "light" | "dark" | null | undefined,
): ThemeMode {
  if (pref === "light" || pref === "dark") return pref;
  return system === "dark" ? "dark" : "light";
}
