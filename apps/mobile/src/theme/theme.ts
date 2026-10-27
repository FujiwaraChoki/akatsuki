// Design tokens mapped to React Native styles (BRIEF "Design theme"). Every colour, size, radius and font comes
// from @akatsuki/tokens; nothing here hard-codes the palette. Pure (no React Native import) so it is unit-tested.
import { accent, color, rgba, type ThemeMode, text as textTokens, tokens } from "@akatsuki/tokens";

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
  /** Ink text steps (design spec §2.3): text 1.0, secondary 0.8, tertiary 0.7, muted 0.6. */
  text: string;
  secondary: string;
  tertiary: string;
  muted: string;
  /** Strokes: hover/active borders (`stroke`), input rest borders (`lineStrong`), hairlines (`line`). */
  borderHover: string;
  border: string;
  borderSubtle: string;
  /** The accent fill: three brand buttons and the long-swipe Trash layer only. */
  accent: string;
  /** Accent for text: errors and destructive labels (dark: lifted to 5.6:1, D-301). */
  accentText: string;
  /** The red of marks in both themes: the brand dot, the cursor bar, carets, the horizon, recording (D-431). */
  dawn: string;
  /** Text on an accent fill (D-431). */
  onAccent: string;
  /** Neutral planes (fg over bg, opaque): `surface` 4% (settings groups, keycaps, fields), `surfaceStrong` 7%. */
  surface: string;
  surfaceStrong: string;
  /** Pressed / selected row: `surfaceStrong`. */
  selected: string;
  /** The wash under the horizon line (sign-in, inbox zero) and nothing else. */
  accentWash: string;
  /** The light page behind sender-styled HTML email. */
  paper: string;
  paperText: string;
  /** Dims the screen behind bottom sheets and menus (no blur). */
  scrim: string;
  /** The lighter scrim behind the command palette. */
  scrimLight: string;
}

type Rgb3 = readonly [number, number, number];
const hex = (c: Rgb3) =>
  `#${c
    .map((v) => v.toString(16).padStart(2, "0"))
    .join("")
    .toUpperCase()}`;
/** fg over bg at `alpha`, as an opaque colour (layers never stack into darker greys). */
export function blend(mode: ThemeMode, alpha: number): string {
  const { foreground: f, background: b } = color[mode].rgb;
  return hex(f.map((v, i) => Math.round(v * alpha + b[i]! * (1 - alpha))) as unknown as Rgb3);
}

export function makeColors(mode: ThemeMode): Colors {
  const c = color[mode];
  const o = tokens.opacity.foreground;
  return {
    background: c.background,
    foreground: c.foreground,
    text: c.foreground,
    secondary: blend(mode, o.secondary),
    tertiary: blend(mode, o.tertiary),
    muted: blend(mode, o.muted),
    borderHover: blend(mode, o.borderHover),
    border: blend(mode, o.border),
    borderSubtle: blend(mode, o.borderSubtle),
    accent: c.accent,
    accentText: c.accentText,
    dawn: c.dawn,
    onAccent: c.onAccent,
    surface: blend(mode, o.surface),
    surfaceStrong: blend(mode, o.surfaceStrong),
    selected: blend(mode, o.surfaceStrong),
    accentWash: accent(mode, "wash"),
    paper: color.light.background,
    paperText: color.light.foreground,
    scrim: mode === "dark" ? rgba("dark", "background", 0.6) : rgba("light", "foreground", 0.18),
    scrimLight: mode === "dark" ? rgba("dark", "background", 0.5) : rgba("light", "foreground", 0.06),
  };
}

export interface TextStyleSpec {
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
  letterSpacing: number;
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
  };
  const size = Math.round(t.size * scale * 10) / 10;
  return {
    fontFamily: fontFamily(t.font, t.weight),
    fontSize: size,
    lineHeight: Math.round(size * t.lineHeight),
    letterSpacing: Math.round(t.tracking * size * 100) / 100,
  };
}

/** A token style at an explicit mobile size (touch screens read larger than desktop chrome; spec §4). */
function sized(
  name: AppStyle,
  size: number,
  over: Partial<{ weight: number; lineHeight: number }> = {},
): TextStyleSpec {
  const t = textTokens.app[name] as {
    font: "serif" | "sans" | "mono";
    lineHeight: number;
    tracking: number;
    weight: number;
  };
  const lh = over.lineHeight ?? t.lineHeight;
  return {
    fontFamily: fontFamily(t.font, over.weight ?? t.weight),
    fontSize: size,
    lineHeight: Math.round(size * lh),
    letterSpacing: Math.round(t.tracking * size * 100) / 100,
  };
}

export interface Theme {
  mode: ThemeMode;
  colors: Colors;
  /** Level 1 elevation (spec §2.7): popovers, the palette, toasts. Everything else is flat. */
  shadow: { pop: string };
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
    /** The unread sender on a row: the one 600 in the product (spec §2.1). */
    unread: TextStyleSpec;
    /** Header titles ("Inbox ⌄", "New message"). */
    title: TextStyleSpec;
    reading: TextStyleSpec;
    code: TextStyleSpec;
    /** Sentence-case group header (`text.app.section`). */
    label: TextStyleSpec;
    /** Mono label, sentence case: only the thread crumb, AI provenance and chips (`text.app.label`). */
    crumb: TextStyleSpec;
    subject: TextStyleSpec;
    heading: TextStyleSpec;
    empty: TextStyleSpec;
    /** The sign-in headline (`text.brand.title` at the mobile size). */
    display: TextStyleSpec;
  };
}

export function makeTheme(mode: ThemeMode): Theme {
  return {
    mode,
    colors: makeColors(mode),
    shadow: {
      pop:
        mode === "dark"
          ? `0 16px 48px -16px ${rgba("dark", "background", tokens.effect.popOpacityDark)}`
          : `0 16px 48px -16px ${rgba("light", "foreground", tokens.effect.popOpacityLight)}, 0 2px 6px -2px ${rgba("light", "foreground", 0.06)}`,
    },
    space: tokens.space,
    radius: tokens.radius,
    motion: tokens.motion,
    layout: tokens.layout,
    effect: tokens.effect,
    text: {
      caption: sized("caption", 12),
      meta: sized("meta", 13),
      body: sized("body", 15, { lineHeight: 1.4 }),
      bodyStrong: sized("bodyStrong", 15, { lineHeight: 1.4 }),
      unread: sized("bodyStrong", 15, { weight: 600, lineHeight: 1.4 }),
      title: sized("bodyStrong", 17, { weight: 600, lineHeight: 1.3 }),
      reading: sized("reading", 16, { lineHeight: 1.55 }),
      code: sized("code", 12),
      label: sized("section", 13),
      crumb: textStyle("label"),
      subject: textStyle("subject"),
      heading: textStyle("heading"),
      empty: textStyle("empty"),
      display: { ...textStyle("heading"), fontSize: 40, lineHeight: 40, letterSpacing: -0.8 },
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
