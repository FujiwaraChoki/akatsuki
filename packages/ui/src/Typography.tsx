// Brand typography: Instrument Serif headings (single weight; emphasis by size and colour) and uppercase,
// tracked labels (0.12em in app chrome, 0.3em on marketing).
import type { HTMLAttributes } from "react";
import { cx } from "./config.tsx";

export interface HeadingProps extends HTMLAttributes<HTMLHeadingElement> {
  level?: 1 | 2 | 3;
  /** `subject`: an opened thread's subject; `heading`: app headings; `empty`: empty states; `display`: hero. */
  variant?: "subject" | "heading" | "empty" | "display";
  /** Accent colour (display text, empty states). */
  accent?: boolean;
}

export function Heading({ level = 2, variant = "heading", accent, className, ...rest }: HeadingProps) {
  const Tag = `h${level}` as const;
  return (
    <Tag
      className={cx("ak-heading", `ak-heading--${variant}`, accent && "ak-heading--accent", className)}
      {...rest}
    />
  );
}

export interface EyebrowProps extends HTMLAttributes<HTMLSpanElement> {
  /** `app` (0.12em, Geist) or `brand` (0.3em, Instrument Serif). */
  variant?: "app" | "brand";
}

/** An uppercase, tracked label ("SPLITS", "ACCOUNTS"). */
export function Eyebrow({ variant = "app", className, ...rest }: EyebrowProps) {
  return <span className={cx("ak-eyebrow", `ak-eyebrow--${variant}`, className)} {...rest} />;
}
