// Instrument Serif (brand, headings, opened-thread subjects, empty states) and Geist / Geist Mono (dense UI), loaded
// with expo-font from the OFL font files in packages/tokens/fonts. Static requires so Metro bundles them as assets.
import { FONT_FAMILIES } from "./theme";

export const FONT_SOURCES = {
  [FONT_FAMILIES.serif]: require("@akatsuki/tokens/fonts/InstrumentSerif-Regular.ttf"),
  [FONT_FAMILIES.serifItalic]: require("@akatsuki/tokens/fonts/InstrumentSerif-Italic.ttf"),
  [FONT_FAMILIES.sans400]: require("@akatsuki/tokens/fonts/Geist-Regular.ttf"),
  [FONT_FAMILIES.sans500]: require("@akatsuki/tokens/fonts/Geist-Medium.ttf"),
  [FONT_FAMILIES.sans600]: require("@akatsuki/tokens/fonts/Geist-SemiBold.ttf"),
  [FONT_FAMILIES.sans700]: require("@akatsuki/tokens/fonts/Geist-Bold.ttf"),
  [FONT_FAMILIES.mono400]: require("@akatsuki/tokens/fonts/GeistMono-Regular.ttf"),
  [FONT_FAMILIES.mono500]: require("@akatsuki/tokens/fonts/GeistMono-Medium.ttf"),
} as const;
