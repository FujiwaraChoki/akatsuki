// Open Graph artwork (§13), rendered at build time with next/og. Light theme, like the owner's
// todo app. Neutrals are the light-theme mixes of #171717 into #FFFFFF.
import { readFile } from "node:fs/promises";
import path from "node:path";
import { repoRoot } from "./repo";

const C = {
  bg: "#FFFFFF",
  fg: "#171717",
  dawn: "#C1272D",
  fg90: "#2E2E2E",
  fg80: "#454545",
  fg70: "#5D5D5D",
  fg55: "#6A6A6A",
  fg40: "#A2A2A2",
  line: "#E8E8E8",
  lineStrong: "#D1D1D1",
  surface: "#F9F9F9",
  surface2: "#F2F2F2",
};

export async function ogFonts() {
  // The brand TTFs live in the token package; satori needs TTF, not woff2.
  const dir = path.join(repoRoot(), "packages/tokens/fonts");
  const f = (n: string) => readFile(path.join(dir, n));
  const [serif, serifIt, sans, sansMed, mono] = await Promise.all([
    f("InstrumentSerif-Regular.ttf"),
    f("InstrumentSerif-Italic.ttf"),
    f("Geist-Regular.ttf"),
    f("Geist-Medium.ttf"),
    f("GeistMono-Medium.ttf"),
  ]);
  return [
    { name: "Instrument Serif", data: serif, weight: 400 as const, style: "normal" as const },
    { name: "Instrument Serif", data: serifIt, weight: 400 as const, style: "italic" as const },
    { name: "Geist", data: sans, weight: 400 as const, style: "normal" as const },
    { name: "Geist", data: sansMed, weight: 500 as const, style: "normal" as const },
    { name: "Geist Mono", data: mono, weight: 500 as const, style: "normal" as const },
  ];
}

/** 1200×630, light (§13, round 5): the headline in Instrument Serif, and the logo, a red dot,
 * rising out of a horizon on the right. The headline's period is the same red. */
export function OgArtwork() {
  const D = 232;
  return (
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        position: "relative",
        backgroundColor: C.bg,
        fontFamily: "Geist",
        overflow: "hidden",
      }}
    >
      {/* the low wash on the horizon, behind the sun */}
      <div
        style={{
          position: "absolute",
          left: 560,
          top: 250,
          width: 640,
          height: 280,
          display: "flex",
          backgroundImage:
            "radial-gradient(50% 100% at 50% 100%, rgba(193,39,45,0.13), rgba(193,39,45,0.04) 45%, rgba(193,39,45,0) 100%)",
        }}
      />
      {/* the sun: the logo's dot, a third of it still below the line */}
      <div
        style={{
          position: "absolute",
          left: 880 - D / 2,
          top: 530 - D * 0.7,
          width: D,
          height: D,
          borderRadius: D / 2,
          backgroundColor: C.dawn,
          display: "flex",
        }}
      />
      {/* ground, so the disc is cut by the horizon */}
      <div
        style={{
          position: "absolute",
          left: 0,
          right: 0,
          top: 530,
          bottom: 0,
          backgroundColor: C.bg,
          display: "flex",
        }}
      />
      <div
        style={{
          position: "absolute",
          left: 0,
          right: 0,
          top: 530,
          height: 1,
          display: "flex",
          backgroundImage:
            "linear-gradient(90deg, rgba(193,39,45,0) 0%, rgba(193,39,45,0.35) 30%, rgba(193,39,45,1) 62%, rgba(193,39,45,1) 84%, rgba(193,39,45,0.35) 96%, rgba(193,39,45,0.2) 100%)",
        }}
      />

      <div
        style={{
          position: "absolute",
          left: 80,
          top: 112,
          width: 720,
          display: "flex",
          flexDirection: "column",
        }}
      >
        <div
          style={{
            fontFamily: "Instrument Serif",
            fontSize: 112,
            lineHeight: 0.94,
            letterSpacing: "-0.025em",
            color: C.fg,
          }}
        >
          Clear your inbox
        </div>
        <div
          style={{
            display: "flex",
            fontFamily: "Instrument Serif",
            fontSize: 112,
            lineHeight: 0.94,
            letterSpacing: "-0.025em",
            color: C.fg,
            marginTop: 6,
          }}
        >
          <span style={{ color: C.fg55 }}>by</span>
          <span style={{ fontStyle: "italic", marginLeft: 26 }}>first light</span>
          <span style={{ color: C.dawn, marginLeft: -2 }}>.</span>
        </div>
        <div
          style={{
            marginTop: 44,
            fontFamily: "Geist",
            fontSize: 24,
            lineHeight: 1.45,
            color: C.fg70,
            width: 470,
          }}
        >
          The email client you drive from the keyboard. Gmail, Outlook and IMAP.
        </div>
      </div>
      <div
        style={{
          position: "absolute",
          left: 80,
          top: 560,
          display: "flex",
          fontFamily: "Geist Mono",
          fontWeight: 500,
          fontSize: 15,
          letterSpacing: "0.16em",
          color: C.fg55,
        }}
      >
        AKATSUKI.VC · PRE-ALPHA
      </div>
    </div>
  );
}

/** The logo: a red dot. `ground` paints a background (Apple touch icons cannot be transparent). */
export function IconArtwork({ size, ground }: { size: number; ground?: string }) {
  const d = ground ? size * 0.5 : size * 0.86;
  return (
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: ground ?? "transparent",
      }}
    >
      <div style={{ width: d, height: d, borderRadius: d / 2, backgroundColor: C.dawn, display: "flex" }} />
    </div>
  );
}
