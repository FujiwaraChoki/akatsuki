// The shared stroke icons (design spec §2.11): 24-unit geometry drawn at 20pt with a 1.75 stroke on mobile, in the
// colour of the text around them. Icons sit next to a word or on a 44pt button with an accessibility label.
import Svg, { Circle, Ellipse, Line, Path, Polygon, Polyline, Rect } from "react-native-svg";
import { useTheme } from "../theme/ThemeProvider";
import { ICON_DATA, type IconName } from "./_generated/iconData";

const TAGS = {
  path: Path,
  rect: Rect,
  circle: Circle,
  line: Line,
  polyline: Polyline,
  polygon: Polygon,
  ellipse: Ellipse,
};

export function Icon({
  name,
  size = 20,
  color,
  filled = false,
  strokeWidth = 1.75,
}: {
  name: IconName;
  size?: number;
  color?: string;
  /** Fill the shape too (a starred star). */
  filled?: boolean;
  strokeWidth?: number;
}) {
  const t = useTheme();
  const c = color ?? t.colors.text;
  return (
    <Svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke={c}
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      pointerEvents="none"
    >
      {ICON_DATA[name].map(([tag, attrs], i) => {
        const El = TAGS[tag as keyof typeof TAGS];
        if (!El) return null;
        // biome-ignore lint/suspicious/noArrayIndexKey: static geometry, never reordered
        return <El key={i} {...(attrs as object)} {...(filled ? { fill: c } : {})} />;
      })}
    </Svg>
  );
}

export type { IconName };
